//! Providers under folders deeper than Windows' `MAX_PATH` (#469, L-187).
//!
//! Windows refuses to start a process whose working directory is longer than
//! 258 characters (`CreateProcessW` fails with `ERROR_DIRECTORY`, whatever
//! `LongPathsEnabled` says), while `FFmpeg` 6 and later open input files of
//! any length. A provider that ran in the folder of the file it reads could
//! therefore not start at all once a long per-user folder or session root
//! pushed that folder past the limit, and the run was reported as a missing
//! capability. Providers now run in their own executable's folder.
//!
//! The first two tests run everywhere and need no media tool: they start the
//! standard-library stand-in `vsift-smoke-fixture`, which answers any media
//! argument list by exiting with status 64, so the run must come back as the
//! provider's own refusal and never as a process that could not start. The
//! last two are opt-in and run the real tools under such a folder:
//!
//! ```console
//! cargo test -p vsift-infrastructure --locked --test long_provider_paths -- --ignored --nocapture
//! ```
//!
//! The first needs `ffmpeg` and `ffprobe` on `PATH`; the second also needs
//! `VSIFT_TEST_WHISPER_CLI` and `VSIFT_TEST_WHISPER_MODEL`, as P07's tests do.
//!
//! Every long folder is built from made-up segments inside the test's own
//! temporary folder, and no path is printed.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, LocalAsrVerification,
    LocalAsrVerifier, MediaToolVerification, MediaToolVerifier, SpeechRecognizer,
};
use vsift_domain::{DurabilityRequirement, OperationId, SessionId};
use vsift_infrastructure::{
    ExecutableResolver, FfmpegMedia, FilesystemSessionStore, FixtureAsrVerifier,
    FixtureMediaToolVerifier, HostIsolation, MediaError, MediaProviderConformance,
    ProcessCancellation, ProcessWorkingDirectory, SourceSnapshot, TrustedExecutable, WhisperCli,
    WhisperError, WhisperSpeechRecognizer, reviewed_compatibility_policy,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

/// The stand-in provider; its own name gives it no behaviour, so it exits 64.
const STAND_IN: &str = env!("CARGO_BIN_EXE_vsift-smoke-fixture");
const OWNED_PREFIX: &str = "vsift-long-provider-paths-";
/// Comfortably past `MAX_PATH` (260), and past the 258 characters a process's
/// working directory may have, once the store's own folders are added.
const LONG_FOLDER_CHARACTERS: usize = 300;
/// A made-up folder name; nothing about the machine is in it.
const SEGMENT: &str = "a-made-up-folder-name-that-only-makes-the-path-long";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(fs::canonicalize(&path)?))
    }

    /// A new folder inside this root whose plain path (without the `\\?\`
    /// prefix `canonicalize` gives on Windows) has at least `characters`
    /// characters.
    fn long_folder(&self, characters: usize) -> Built<PathBuf> {
        let mut folder = self.0.clone();
        let mut index = 0_u32;
        while plain_length(&folder) < characters {
            folder.push(format!("{SEGMENT}-{index}"));
            index += 1;
        }
        fs::create_dir_all(&folder)?;
        Ok(folder)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// The length Windows counts against `MAX_PATH`: the path without the
/// extended-length prefix.
fn plain_length(path: &Path) -> usize {
    plain(path).as_os_str().to_string_lossy().chars().count()
}

/// `path` without the extended-length prefix, as a per-user folder taken
/// from the environment is given.
fn plain(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// A session store at `root` with one initialised session holding F01.
async fn staged_f01(root: &Path) -> Built<(FilesystemSessionStore, SourceSnapshot)> {
    let store = FilesystemSessionStore::provision_default(root)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let store = FilesystemSessionStore::open_existing(root)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &repository("fixtures/corpus/generated/F01.mp4"),
    )?;
    Ok((store, snapshot))
}

/// #469, the session-root case: a source copy whose folder is longer than a
/// working directory may be is still handed to a provider that starts.
#[tokio::test]
async fn a_provider_starts_for_a_source_deeper_than_max_path() -> TestResult {
    let root = OwnedRoot::new()?;
    let session_root = root.long_folder(LONG_FOLDER_CHARACTERS)?.join("sessions");
    let (store, snapshot) = staged_f01(&session_root).await?;
    let copy_folder = snapshot
        .provider_path()
        .parent()
        .map(Path::to_path_buf)
        .ok_or("the copy has no folder")?;
    assert!(plain_length(&copy_folder) > LONG_FOLDER_CHARACTERS);
    let stand_in = TrustedExecutable::explicit(STAND_IN)?;
    let media = FfmpegMedia::new(
        MediaProviderConformance::r0(stand_in.clone(), stand_in),
        HostIsolation::ProcessOnly,
        &store,
    );

    let probed = media.probe(&snapshot, ProcessCancellation::new()).await;

    assert!(
        matches!(probed, Err(MediaError::ProviderRejected)),
        "the stand-in must have run and refused the media, not failed to start: {probed:?}"
    );
    drop(snapshot);
    drop(store);
    Ok(())
}

/// #469, the recognizer case: a chunk folder longer than a working
/// directory may be still runs whisper.cpp.
#[tokio::test]
async fn a_recognizer_starts_for_a_chunk_folder_deeper_than_max_path() -> TestResult {
    let root = OwnedRoot::new()?;
    let chunks = root.long_folder(LONG_FOLDER_CHARACTERS)?;
    let model = root.0.join("model.bin");
    fs::write(&model, b"not a model; the stand-in never reads it")?;
    let whisper = WhisperCli::new(
        TrustedExecutable::explicit(STAND_IN)?,
        &model,
        NonZeroU16::MIN,
        HostIsolation::ProcessOnly,
    )?;
    let workspace = ProcessWorkingDirectory::new(&chunks)?;

    let recognized = whisper
        .transcribe_chunk(&workspace, 0, &[0_i16; 1600], ProcessCancellation::new())
        .await;

    assert!(
        matches!(recognized, Err(WhisperError::ProviderFailed)),
        "the stand-in must have run and exited unsuccessfully, not failed to start: {recognized:?}"
    );
    Ok(())
}

fn tools_on_path() -> Built<MediaProviderConformance> {
    let resolver = ExecutableResolver::from_current_path();
    Ok(MediaProviderConformance::r0(
        resolver.resolve(OsStr::new("ffmpeg"))?,
        resolver.resolve(OsStr::new("ffprobe"))?,
    ))
}

fn required_path(name: &str) -> Built<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is not set").into())
}

/// #469, the per-user-folder case, with the real tools: the reviewed F01
/// checks (probe, frame, audio, the frame listing, an exact frame, a crop and
/// the visual samples) pass in a verification workspace deeper than
/// `MAX_PATH`, given by its plain path as a long `LOCALAPPDATA` gives it.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn the_media_tool_check_passes_deeper_than_max_path() -> TestResult {
    let root = OwnedRoot::new()?;
    let parent = plain(&root.long_folder(LONG_FOLDER_CHARACTERS)?);
    let verifier = FixtureMediaToolVerifier::new(
        tools_on_path()?,
        HostIsolation::ProcessOnly,
        parent,
        reviewed_compatibility_policy()?,
        ProcessCancellation::new(),
    );

    let verification = verifier.verify().await;

    println!("media-tool check deeper than MAX_PATH: {verification:?}");
    assert_eq!(verification, MediaToolVerification::Verified);
    Ok(())
}

/// #469 with the real recognizer: the reviewed speech fixture is decoded and
/// transcribed in a verification workspace deeper than `MAX_PATH`, so
/// whisper.cpp reads its chunk and writes its JSON there.
#[tokio::test]
#[ignore = "requires VSIFT_TEST_WHISPER_CLI, VSIFT_TEST_WHISPER_MODEL and FFmpeg/FFprobe on PATH"]
async fn the_speech_check_passes_deeper_than_max_path() -> TestResult {
    let root = OwnedRoot::new()?;
    let parent = plain(&root.long_folder(LONG_FOLDER_CHARACTERS)?);
    let whisper = WhisperCli::new(
        TrustedExecutable::explicit(required_path("VSIFT_TEST_WHISPER_CLI")?)?,
        &required_path("VSIFT_TEST_WHISPER_MODEL")?,
        NonZeroU16::new(4).ok_or("zero threads")?,
        HostIsolation::ProcessOnly,
    )?;
    let identity_folder = root.0.join("identity");
    fs::create_dir(&identity_folder)?;
    let expected = WhisperSpeechRecognizer::new(
        whisper.clone(),
        ProcessWorkingDirectory::new(&identity_folder)?,
        ProcessCancellation::new(),
    )
    .identity()
    .await?;
    let verifier = FixtureAsrVerifier::new(
        tools_on_path()?,
        HostIsolation::ProcessOnly,
        parent,
        whisper,
        expected,
        ProcessCancellation::new(),
    );

    let verification = verifier.verify().await;

    println!("speech check deeper than MAX_PATH: {verification:?}");
    assert_eq!(verification, LocalAsrVerification::Verified);
    Ok(())
}
