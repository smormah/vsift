//! T-04 local-ASR qualification (P07 increment 3c): accuracy, timing and
//! memory of each reviewed model profile through `VSift`'s real chain.
//!
//! The scoring rules (`asr_scoring`) and their frozen expectations are tested
//! on every run. The measurement itself is opt-in, because it needs the
//! reviewed whisper.cpp build, the pinned models and `FFmpeg`/`FFprobe` on
//! `PATH`, and takes minutes:
//!
//! ```console
//! VSIFT_TEST_WHISPER_CLI=<absolute whisper-cli path>
//! VSIFT_TEST_WHISPER_MODEL=<absolute ggml-base.bin path>
//! VSIFT_TEST_WHISPER_MODEL_Q5_1=<absolute ggml-base-q5_1.bin path>   # optional
//! cargo test --release -p vsift-infrastructure --locked --test p07_asr_qualification -- --ignored --nocapture
//! ```
//!
//! For every profile, on 4 recognizer threads (the D6 measurement point):
//!
//! - accuracy: each committed speech clip (`*-speech.*`) is transcribed by
//!   `transcribe_range` exactly as a retranscription does, and scored against
//!   its frozen script and spoken critical terms;
//! - model load time: whisper.cpp's own `load time` for the F01 utterance,
//!   median of three runs;
//! - real-time factor: a clip of about three minutes, concatenated here from
//!   the committed utterances (never committed), transcribed three times;
//!   the median elapsed time divided by the clip's duration;
//! - peak memory of `whisper-cli` during those runs: on Windows the
//!   `PeakWorkingSet64` of every `whisper-cli` process, sampled every 100 ms
//!   by `PowerShell` `Get-Process`; on Linux `VmHWM` from `/proc/<pid>/status`
//!   sampled every 100 ms. Other platforms report it as not measured.
//!
//! The accuracy gates decided for the base profile (maintainer, 2026-09-25) are
//! enforced: pooled word error rate of the clips without added noise at most
//! 10%, and every spoken critical term found, in every clip including the noisy
//! F08, except the reviewed known misses. F08's word error rate is reported as
//! a known limitation and not gated until a noisy-speech fixture set exists
//! (issue #150). The resource gates (real-time factor at most 0.5,
//! peak memory at most 400 MiB) are evaluated and reported, not enforced,
//! because they measure the host. The quantized profile is reported only. The
//! report is written to `.vsift/e2e-runs/p07-asr-qualification-<run>/`.

mod asr_scoring;

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use asr_scoring::{
    Edits, KNOWN_BASE_MISSES, SpeechExpectation, contains_term, is_known_miss, normalise, score,
    speech_expectations, word_edits,
};
use serde_json::{Value, json};
use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, RecognizerIdentity,
    SpeechRecognizer, TranscribeRangeRequest, transcribe_range, whole_file_source_segment,
};
use vsift_domain::{
    AsrModelProfile, ChunkPlan, DurabilityRequirement, MediaSelection, OperationId, SessionId,
};
use vsift_infrastructure::{
    ExecutableResolver, FfmpegMedia, FfmpegSpeechAudio, FilesystemSessionStore, HostIsolation,
    MediaProviderConformance, ProcessCancellation, ProcessWorkingDirectory, SourceSnapshot,
    TrustedExecutable, WhisperCli, WhisperSpeechRecognizer,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-p07-asr-qualification-";
/// The D6 measurement point.
const THREADS: u16 = 4;
/// Target length of the concatenated timing clip.
const TIMING_CLIP_TARGET_US: u64 = 180_000_000;
const UTTERANCE_GAP_US: u64 = 300_000;
const TIMING_RUNS: usize = 3;
const LOAD_TIME_RUNS: usize = 3;
#[cfg(any(windows, target_os = "linux"))]
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
/// D6 gates for the base profile, in basis points where they are rates.
const CLEAN_WER_GATE_BP: usize = 1_000;
/// Why F08's word error rate is reported but not gated.
const NOISY_WER_NOT_GATED: &str = "known limitation: one 13-word noisy clip is too small a basis for a word error rate gate; noisy speech is gated on critical terms only until the noisy-speech fixture set of issue #150 exists (maintainer decision, 2026-09-25)";
const RTF_GATE_MILLI: u128 = 500;
const PEAK_MEMORY_GATE_BYTES: u64 = 400 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn load_json(relative: &str) -> Built<Value> {
    Ok(serde_json::from_slice(&fs::read(repository(relative))?)?)
}

fn expectations() -> Built<Vec<SpeechExpectation>> {
    Ok(speech_expectations(&load_json(
        "fixtures/corpus/manifest.json",
    )?)?)
}

fn words(text: &str) -> Vec<String> {
    normalise(text)
}

// ---------------------------------------------------------------------------
// Scoring rules, always run.
// ---------------------------------------------------------------------------

#[test]
fn normalisation_follows_the_reviewed_rules() {
    for (text, expected) in [
        ("Error E-409!", &["error", "e409"][..]),
        ("AB-731", &["ab731"][..]),
        (
            "This total should be 125.00.",
            &["this", "total", "should", "be", "125"][..],
        ),
        ("127.50 and 2.5", &["127.5", "and", "2.5"][..]),
        ("the build is 2,048", &["the", "build", "is", "2048"][..]),
        (
            "rises to twelve, says three",
            &["rises", "to", "12", "says", "3"][..],
        ),
        ("at 10:32 while", &["at", "10", "32", "while"][..]),
        ("\"Q\" to \"Failed\"", &["q", "to", "failed"][..]),
        (
            "El identificador es AB-731.",
            &["el", "identificador", "es", "ab731"][..],
        ),
        ("SAFE-12", &["safe12"][..]),
    ] {
        assert_eq!(words(text), expected, "{text}");
    }
}

#[test]
fn word_edits_count_substitutions_deletions_and_insertions() {
    let reference = words("the queue is empty");
    assert_eq!(word_edits(&reference, &reference).errors(), 0);
    let edits = word_edits(&reference, &words("the q is empty now"));
    assert_eq!(
        (edits.substitutions, edits.deletions, edits.insertions),
        (1, 0, 1)
    );
    let edits = word_edits(&reference, &words("queue is"));
    assert_eq!(
        (edits.substitutions, edits.deletions, edits.insertions),
        (0, 2, 0)
    );
    assert_eq!(edits.wer_basis_points(), 5_000);
    let pooled = edits.plus(Edits {
        reference_words: 6,
        ..Edits::default()
    });
    assert_eq!(pooled.wer_basis_points(), 2_000);
}

#[test]
fn critical_terms_match_across_word_boundaries_but_not_inside_numbers() {
    assert!(contains_term(
        &words("the code is safe 12"),
        &words("SAFE-12")
    ));
    assert!(contains_term(&words("at 10, 32 while"), &words("10:32")));
    assert!(contains_term(&words("code E409 after"), &words("E-409")));
    assert!(!contains_term(&words("invoice 407"), &words("4407")));
    assert!(!contains_term(&words("from q to failed"), &words("queued")));
    assert!(!contains_term(&words(""), &words("queue")));
}

/// Expectations come from the frozen manifest alone: spoken terms are
/// critical, visual-only terms are not, and every reviewed known miss is a
/// real critical term of its fixture.
#[test]
fn expectations_derive_only_from_the_frozen_scripts() -> TestResult {
    let expectations = expectations()?;
    let ids: Vec<&str> = expectations
        .iter()
        .map(|expectation| expectation.fixture.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "F01", "F02", "F03", "F04", "F05", "F06", "F07", "F08", "F09", "F12"
        ]
    );
    let find = |id: &str| {
        expectations
            .iter()
            .find(|expectation| expectation.fixture == id)
            .ok_or_else(|| format!("{id} missing"))
    };
    assert_eq!(find("F03")?.visual_only_terms, ["G18"]);
    assert_eq!(
        find("F03")?.critical_terms,
        ["125.00", "recalculation", "127.50"]
    );
    assert_eq!(find("F08")?.critical_terms.len(), 4);
    assert!(find("F08")?.noisy());
    for (fixture, known) in KNOWN_BASE_MISSES {
        let covering: Vec<&String> = find(fixture)?
            .critical_terms
            .iter()
            .filter(|term| is_known_miss(fixture, term))
            .collect();
        assert_eq!(covering.len(), 1, "{fixture} {known}");
    }
    assert!(is_known_miss("F05", "invoice 4407"));
    assert!(!is_known_miss("F05", "success banner"));
    assert!(!is_known_miss("F04", "invoice 4407"));
    for expectation in &expectations {
        let perfect = score(expectation, &expectation.script);
        assert_eq!(perfect.edits.errors(), 0, "{}", expectation.fixture);
        assert!(perfect.missed.is_empty(), "{}", expectation.fixture);
    }
    Ok(())
}

#[test]
fn known_misses_are_reported_separately_from_unexpected_ones() -> TestResult {
    let expectations = expectations()?;
    let f04 = expectations
        .iter()
        .find(|expectation| expectation.fixture == "F04")
        .ok_or("F04 missing")?;
    let scored = score(
        f04,
        "Scroll to order 1017. The status changes from Q to failed while the heading remains fixed.",
    );
    assert_eq!(scored.missed, ["queued", "header"]);
    assert_eq!(scored.known_misses, ["queued"]);
    assert_eq!(scored.unexpected_misses(), [&String::from("header")]);
    assert_eq!(scored.edits.substitutions, 2);
    Ok(())
}

// ---------------------------------------------------------------------------
// Opt-in measurement.
// ---------------------------------------------------------------------------

struct OwnedRoot(PathBuf);

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

fn required_path(name: &str) -> Built<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| format!("set {name} to an absolute path").into())
}

/// Tools and storage shared by every measurement.
struct Bench {
    root: OwnedRoot,
    store: FilesystemSessionStore,
    session: SessionId,
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
    whisper: TrustedExecutable,
    operations: std::cell::Cell<u64>,
}

impl Bench {
    async fn new() -> Built<Self> {
        let resolver = ExecutableResolver::from_current_path();
        let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"))?;
        let ffprobe = resolver.resolve(OsStr::new("ffprobe"))?;
        let whisper = TrustedExecutable::explicit(required_path("VSIFT_TEST_WHISPER_CLI")?)?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = OwnedRoot(
            env::temp_dir().join(format!("{OWNED_PREFIX}{}-{stamp}", std::process::id())),
        );
        fs::create_dir_all(root.0.join("work"))?;
        let store_root = root.0.join("sessions");
        let session = SessionId::parse("ses_0123456789abcdef")?;
        InitializeSessionStorage::new(FilesystemSessionStore::provision_default(&store_root)?)
            .execute(InitializeSessionStorageRequest::new(
                session.clone(),
                OperationId::parse("op_0123456789abcdef")?,
                DurabilityRequirement::Ephemeral,
            ))
            .await?;
        Ok(Self {
            store: FilesystemSessionStore::open_existing(&store_root)?,
            root,
            session,
            ffmpeg,
            ffprobe,
            whisper,
            operations: std::cell::Cell::new(1),
        })
    }

    fn recognizer(&self, model: &Path) -> Built<WhisperSpeechRecognizer> {
        Ok(WhisperSpeechRecognizer::new(
            WhisperCli::new(
                self.whisper.clone(),
                model,
                NonZeroU16::new(THREADS).ok_or("zero threads")?,
                HostIsolation::ProcessOnly,
            )?,
            ProcessWorkingDirectory::new(self.root.0.join("work"))?,
            ProcessCancellation::new(),
        ))
    }

    /// Transcribes one clip through the real chain and returns its text and
    /// the elapsed recognition time.
    async fn transcribe(
        &self,
        clip: &Path,
        recognizer: &WhisperSpeechRecognizer,
        expected: &RecognizerIdentity,
    ) -> Built<(String, Duration, u64)> {
        let operation = self.operations.get() + 1;
        self.operations.set(operation);
        let snapshot = SourceSnapshot::stage(
            &self.store,
            &self.session,
            &OperationId::parse(format!("op_{operation:016x}"))?,
            clip,
        )?;
        let media = FfmpegMedia::new(
            MediaProviderConformance::r0(self.ffmpeg.clone(), self.ffprobe.clone()),
            HostIsolation::ProcessOnly,
            &self.store,
        );
        let description = media.probe(&snapshot, ProcessCancellation::new()).await?;
        let stream = description
            .speech_audio_stream()
            .ok_or("clip has no decodable audio")?;
        let source = whole_file_source_segment(snapshot.id(), description.duration)?;
        let audio = FfmpegSpeechAudio::new(
            &media,
            &snapshot,
            &description,
            MediaSelection {
                video: None,
                audio: Some(stream),
            },
            ProcessCancellation::new(),
        );
        let started = Instant::now();
        let transcription = transcribe_range(
            TranscribeRangeRequest {
                source_segment: &source,
                range: source.range(),
                plan: ChunkPlan::R0,
                audio_stream: stream,
                expected,
            },
            &audio,
            recognizer,
            &ProcessCancellation::new(),
        )
        .await?;
        let elapsed = started.elapsed();
        let text = transcription
            .segments
            .iter()
            .map(|merged| merged.segment.text().text().to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        Ok((text, elapsed, description.duration.as_micros()))
    }

    /// Builds the timing clip from the committed utterances, cycling through
    /// them with a short gap until it lasts about three minutes.
    fn timing_clip(&self, fixtures: &[String]) -> Built<(PathBuf, Vec<String>)> {
        let clip = self.root.0.join("timing-clip.mp4");
        let mut arguments: Vec<String> = vec!["-v".into(), "error".into(), "-n".into()];
        let mut filters = String::new();
        let mut order = Vec::new();
        let mut cursor_us = 0_u64;
        let mut index = 0_usize;
        while cursor_us < TIMING_CLIP_TARGET_US {
            let fixture = fixtures
                .get(index % fixtures.len().max(1))
                .ok_or("no utterances")?;
            let wav = repository(&format!(
                "fixtures/corpus/generated/speech/{fixture}-utterance.wav"
            ));
            cursor_us += wav_duration_us(&wav)? + UTTERANCE_GAP_US;
            arguments.extend(["-i".into(), wav.to_string_lossy().into_owned()]);
            write!(
                filters,
                "[{index}:a]aresample=16000,apad=pad_dur={:.3}[a{index}];",
                seconds(UTTERANCE_GAP_US)
            )?;
            order.push(fixture.clone());
            index += 1;
        }
        for input in 0..index {
            write!(filters, "[a{input}]")?;
        }
        write!(filters, "concat=n={index}:v=0:a=1[speech]")?;
        arguments.extend([
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("color=c=black:s=320x240:r=10:d={:.3}", seconds(cursor_us)),
            "-filter_complex".into(),
            filters,
            "-map".into(),
            format!("{index}:v"),
            "-map".into(),
            "[speech]".into(),
            "-c:v".into(),
            "libx264".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-c:a".into(),
            "aac".into(),
            "-shortest".into(),
            clip.to_string_lossy().into_owned(),
        ]);
        let status = Command::new(self.ffmpeg.path())
            .args(&arguments)
            .stdin(Stdio::null())
            .status()?;
        if !status.success() {
            return Err("ffmpeg could not build the timing clip".into());
        }
        Ok((clip, order))
    }
}

fn seconds(micros: u64) -> f64 {
    Duration::from_micros(micros).as_secs_f64()
}

/// Duration of a 16-bit PCM WAV from its header: data bytes over byte rate.
fn wav_duration_us(path: &Path) -> Built<u64> {
    let bytes = fs::read(path)?;
    let field = |at: usize| -> Built<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or("short WAV header")?
                .try_into()?,
        ))
    };
    let byte_rate = u64::from(field(28)?);
    let mut offset = 12_usize;
    while offset + 8 <= bytes.len() {
        let size = u64::from(field(offset + 4)?);
        if bytes.get(offset..offset + 4) == Some(b"data".as_slice()) {
            return Ok(size * 1_000_000 / byte_rate.max(1));
        }
        offset += 8 + usize::try_from(size)?;
    }
    Err("WAV has no data chunk".into())
}

/// whisper.cpp's own model load time for one short utterance, in
/// milliseconds, from its `whisper_print_timings` line.
fn model_load_ms(whisper: &Path, model: &Path, work: &Path) -> Built<f64> {
    let output = Command::new(whisper)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(repository(
            "fixtures/corpus/generated/speech/F01-utterance.wav",
        ))
        .args([
            "-t",
            &THREADS.to_string(),
            "-l",
            "auto",
            "-bs",
            "5",
            "-bo",
            "5",
            "-ng",
        ])
        .current_dir(work)
        .stdin(Stdio::null())
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    stderr
        .lines()
        .find_map(|line| {
            let rest = line.split_once("load time =")?.1;
            rest.trim()
                .trim_end_matches("ms")
                .trim()
                .parse::<f64>()
                .ok()
        })
        .ok_or_else(|| "whisper-cli printed no load time".into())
}

fn median<T: Copy + PartialOrd>(mut values: Vec<T>) -> Option<T> {
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    values.get(values.len() / 2).copied()
}

/// Samples the peak memory of every `whisper-cli` process until stopped.
struct PeakMemorySampler {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<Option<u64>>>,
    method: &'static str,
}

impl PeakMemorySampler {
    fn start(work: &Path) -> Built<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&stop);
        let (worker, method) = platform_sampler(work, observed)?;
        Ok(Self {
            stop,
            worker,
            method,
        })
    }

    fn finish(mut self) -> Option<u64> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker
            .take()
            .and_then(|worker| worker.join().ok().flatten())
    }
}

type SamplerThread = Option<std::thread::JoinHandle<Option<u64>>>;

#[cfg(windows)]
fn platform_sampler(work: &Path, stop: Arc<AtomicBool>) -> Built<(SamplerThread, &'static str)> {
    use std::io::Write as _;

    // PowerShell reads the fixed script from standard input and runs until
    // the sentinel file is removed; nothing is interpolated into it. It is one
    // line because `-Command -` parses standard input line by line.
    const SCRIPT: &str = "$max = [int64]0; \
        while (Test-Path -LiteralPath $env:VSIFT_SAMPLER_SENTINEL) { \
        Get-Process -Name 'whisper-cli' -ErrorAction SilentlyContinue | ForEach-Object { \
        if ($_.PeakWorkingSet64 -gt $max) { $max = $_.PeakWorkingSet64 } }; \
        Start-Sleep -Milliseconds 100 }; Write-Output $max\n";
    let sentinel = work.join("memory-sampler.running");
    fs::write(&sentinel, b"")?;
    let mut child = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", "-"])
        .env("VSIFT_SAMPLER_SENTINEL", &sentinel)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("sampler has no standard input")?
        .write_all(SCRIPT.as_bytes())?;
    let worker = std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(SAMPLE_INTERVAL);
        }
        fs::remove_file(&sentinel).ok()?;
        let output = child.wait_with_output().ok()?;
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()
    });
    Ok((
        Some(worker),
        "PeakWorkingSet64 of whisper-cli processes, sampled every 100 ms by PowerShell Get-Process",
    ))
}

#[cfg(target_os = "linux")]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Windows sampler can fail to start its PowerShell process"
)]
fn platform_sampler(_work: &Path, stop: Arc<AtomicBool>) -> Built<(SamplerThread, &'static str)> {
    let worker = std::thread::spawn(move || {
        let mut peak = 0_u64;
        while !stop.load(Ordering::SeqCst) {
            if let Ok(entries) = fs::read_dir("/proc") {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let is_whisper = fs::read_to_string(path.join("comm"))
                        .is_ok_and(|name| name.trim() == "whisper-cli");
                    if !is_whisper {
                        continue;
                    }
                    let high_water =
                        fs::read_to_string(path.join("status"))
                            .ok()
                            .and_then(|status| {
                                status.lines().find_map(|line| {
                                    line.strip_prefix("VmHWM:")?
                                        .trim()
                                        .trim_end_matches("kB")
                                        .trim()
                                        .parse::<u64>()
                                        .ok()
                                })
                            });
                    if let Some(kib) = high_water {
                        peak = peak.max(kib * 1024);
                    }
                }
            }
            std::thread::sleep(SAMPLE_INTERVAL);
        }
        Some(peak)
    });
    Ok((
        Some(worker),
        "VmHWM of whisper-cli processes from /proc/<pid>/status, sampled every 100 ms",
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Windows sampler can fail to start its PowerShell process"
)]
fn platform_sampler(_work: &Path, _stop: Arc<AtomicBool>) -> Built<(SamplerThread, &'static str)> {
    Ok((None, "not measured on this platform"))
}

/// The processor model as the operating system names it.
fn cpu_model() -> String {
    #[cfg(windows)]
    {
        Command::new("reg")
            .args([
                "query",
                r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
                "/v",
                "ProcessorNameString",
            ])
            .output()
            .ok()
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .find_map(|line| {
                        line.split_once("REG_SZ")
                            .map(|(_, name)| name.trim().to_owned())
                    })
            })
            .unwrap_or_else(|| String::from("unknown"))
    }
    #[cfg(target_os = "linux")]
    {
        fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|info| {
                info.lines().find_map(|line| {
                    line.strip_prefix("model name")
                        .and_then(|rest| rest.split_once(':'))
                        .map(|(_, name)| name.trim().to_owned())
                })
            })
            .unwrap_or_else(|| String::from("unknown"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        String::from("unknown")
    }
}

fn percent(basis_points: usize) -> String {
    format!("{}.{:02}%", basis_points / 100, basis_points % 100)
}

/// Accuracy of one profile over every speech clip.
async fn accuracy(
    bench: &Bench,
    recognizer: &WhisperSpeechRecognizer,
    identity: &RecognizerIdentity,
    expectations: &[SpeechExpectation],
    variants: &[Value],
) -> Built<(Vec<Value>, Edits, Edits, Vec<String>)> {
    let mut fixtures = Vec::new();
    let mut clean = Edits::default();
    let mut noisy = Edits::default();
    let mut unexpected = Vec::new();
    for expectation in expectations {
        let file = variants
            .iter()
            .find(|variant| variant["fixture"] == expectation.fixture.as_str())
            .and_then(|variant| variant["file"].as_str())
            .ok_or_else(|| format!("{} has no speech variant", expectation.fixture))?;
        let clip = repository(&format!("fixtures/corpus/generated/{file}"));
        let (text, elapsed, _) = bench.transcribe(&clip, recognizer, identity).await?;
        let scored = score(expectation, &text);
        if expectation.noisy() {
            noisy = noisy.plus(scored.edits);
        } else {
            clean = clean.plus(scored.edits);
        }
        for term in scored.unexpected_misses() {
            unexpected.push(format!("{} {term}", expectation.fixture));
        }
        println!(
            "  {} {:>7} WER, missed {:?} (known {:?}), {} ms",
            expectation.fixture,
            percent(scored.edits.wer_basis_points()),
            scored.missed,
            scored.known_misses,
            elapsed.as_millis()
        );
        fixtures.push(json!({
            "fixture": expectation.fixture,
            "mode": expectation.mode,
            "file": file,
            "reference_words": scored.edits.reference_words,
            "substitutions": scored.edits.substitutions,
            "deletions": scored.edits.deletions,
            "insertions": scored.edits.insertions,
            "wer_percent": percent(scored.edits.wer_basis_points()),
            "critical_terms": expectation.critical_terms,
            "visual_only_terms": expectation.visual_only_terms,
            "found": scored.found,
            "missed": scored.missed,
            "known_misses": scored.known_misses,
            "hypothesis": text,
            "elapsed_ms": elapsed.as_millis(),
        }));
    }
    Ok((fixtures, clean, noisy, unexpected))
}

#[tokio::test]
#[ignore = "requires VSIFT_TEST_WHISPER_CLI, VSIFT_TEST_WHISPER_MODEL (and optionally VSIFT_TEST_WHISPER_MODEL_Q5_1) and FFmpeg/FFprobe on PATH; use --release"]
#[allow(
    clippy::too_many_lines,
    reason = "One qualification run per profile, reported as it runs"
)]
async fn local_asr_accuracy_timing_and_memory() -> TestResult {
    let started = Instant::now();
    let bench = Bench::new().await?;
    let expectations = expectations()?;
    let provenance = load_json("fixtures/corpus/generated/speech-provenance.json")?;
    let variants = provenance["assembly"]["variants"]
        .as_array()
        .ok_or("speech provenance has no variants")?
        .clone();
    let utterances: Vec<String> = expectations
        .iter()
        .map(|expectation| expectation.fixture.clone())
        .collect();
    let (timing_clip, timing_order) = bench.timing_clip(&utterances)?;
    let mut models = vec![required_path("VSIFT_TEST_WHISPER_MODEL")?];
    if let Ok(quantized) = required_path("VSIFT_TEST_WHISPER_MODEL_Q5_1") {
        models.push(quantized);
    }

    let mut profiles = Vec::new();
    let mut failures = Vec::new();
    let mut timing_duration_us = 0;
    for model in &models {
        let recognizer = bench.recognizer(model)?;
        let identity = recognizer.identity().await?;
        let profile = identity.model.profile();
        if profile == AsrModelProfile::Unreviewed {
            return Err("a model under test is not a reviewed pinned profile".into());
        }
        println!("profile {} on {THREADS} threads", profile.identifier());

        let (fixtures, clean, noisy, unexpected) =
            accuracy(&bench, &recognizer, &identity, &expectations, &variants).await?;

        let mut load_times = Vec::new();
        for _ in 0..LOAD_TIME_RUNS {
            load_times.push(model_load_ms(bench.whisper.path(), model, &bench.root.0)?);
        }
        let sampler = PeakMemorySampler::start(&bench.root.0)?;
        let method = sampler.method;
        let mut timings = Vec::new();
        for run in 0..TIMING_RUNS {
            let (_, elapsed, duration_us) = bench
                .transcribe(&timing_clip, &recognizer, &identity)
                .await?;
            timing_duration_us = duration_us;
            println!(
                "  timing run {}: {:.1} s for {:.1} s of audio",
                run + 1,
                elapsed.as_secs_f64(),
                seconds(duration_us)
            );
            timings.push(elapsed);
        }
        let peak = sampler.finish().filter(|bytes| *bytes > 0);
        let median_elapsed = median(timings.clone()).ok_or("no timing run")?;
        let rtf_milli = median_elapsed.as_micros() * 1_000 / u128::from(timing_duration_us.max(1));
        let load_ms = median(load_times.clone()).ok_or("no load time")?;

        let clean_ok = clean.wer_basis_points() <= CLEAN_WER_GATE_BP;
        let terms_ok = unexpected.is_empty();
        let rtf_ok = rtf_milli <= RTF_GATE_MILLI;
        let memory_ok = peak.map(|bytes| bytes <= PEAK_MEMORY_GATE_BYTES);
        if profile == AsrModelProfile::Base {
            if !clean_ok {
                failures.push(format!(
                    "base clean WER {} exceeds 10%",
                    percent(clean.wer_basis_points())
                ));
            }
            if !terms_ok {
                failures.push(format!("base missed critical terms {unexpected:?}"));
            }
        }
        println!(
            "  clean WER {}, F08 WER {}, unexpected misses {unexpected:?}, load {load_ms:.0} ms, RTF {}.{:03}, peak {} MiB",
            percent(clean.wer_basis_points()),
            percent(noisy.wer_basis_points()),
            rtf_milli / 1_000,
            rtf_milli % 1_000,
            peak.map_or_else(
                || String::from("not measured"),
                |bytes| (bytes / MIB).to_string()
            ),
        );
        profiles.push(json!({
            "profile": profile.identifier(),
            "model_sha256": identity.model.sha256().as_str(),
            "gates_enforced": profile == AsrModelProfile::Base,
            "accuracy": {
                "clean_pooled": {
                    "reference_words": clean.reference_words,
                    "errors": clean.errors(),
                    "wer_percent": percent(clean.wer_basis_points()),
                    "gate": "<= 10%",
                    "gated": true,
                    "met": clean_ok,
                },
                "f08_noisy": {
                    "reference_words": noisy.reference_words,
                    "errors": noisy.errors(),
                    "wer_percent": percent(noisy.wer_basis_points()),
                    "gated": false,
                    "reason": NOISY_WER_NOT_GATED,
                },
                "critical_terms": {
                    "unexpected_misses": unexpected,
                    "known_misses_list": KNOWN_BASE_MISSES
                        .iter()
                        .map(|(fixture, term)| format!("{fixture} {term}"))
                        .collect::<Vec<_>>(),
                    "gate": "every spoken critical term in every clip, noisy included, except the reviewed known misses",
                    "gated": true,
                    "met": terms_ok,
                },
                "fixtures": fixtures,
            },
            "model_load_ms": {
                "median": load_ms,
                "runs": load_times,
                "method": "whisper_print_timings load time of whisper-cli on the F01 utterance",
            },
            "real_time_factor": {
                "median": format!("{}.{:03}", rtf_milli / 1_000, rtf_milli % 1_000),
                "runs_ms": timings.iter().map(Duration::as_millis).collect::<Vec<_>>(),
                "clip_duration_ms": timing_duration_us / 1_000,
                "gate": "<= 0.5",
                "gated": false,
                "reason": "reported, not enforced: it measures the host",
                "met": rtf_ok,
            },
            "peak_memory": {
                "bytes": peak,
                "mib": peak.map(|bytes| bytes / MIB),
                "method": method,
                "gate": "<= 400 MiB",
                "gated": false,
                "reason": "reported, not enforced: it measures the host",
                "met": memory_ok,
            },
        }));
    }

    let report = json!({
        "schema_version": 1,
        "qualification": "P07 local ASR (T-04, maintainer decision D6)",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "cpu": cpu_model(),
        "logical_processors": std::thread::available_parallelism().map(std::num::NonZero::get).ok(),
        "recognizer_threads": THREADS,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "whisper_cli_sha256": vsift_infrastructure::identify_whisper_build(&bench.whisper)
            .await
            .map(vsift_infrastructure::WhisperBuildIdentity::sha256_hex)
            .ok(),
        "decoding_profile": "r0-v1",
        "timing_clip": {
            "built_from": "the committed speech utterances, cycled with 300 ms gaps, over a black video; built at run time and not committed",
            "utterances": timing_order,
            "duration_ms": timing_duration_us / 1_000,
        },
        "coverage_gaps": [
            "accented speech is not in the corpus (issue #150)",
            "crosstalk (overlapping speakers) is not in the corpus (issue #150)",
            "noisy speech is one 13-word clip (F08), so its word error rate is not gated (issue #150)",
            "every clip is synthetic (Kokoro) speech; no human recordings"
        ],
        "profiles": profiles,
        "failures": failures,
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository(".vsift/e2e-runs").join(format!(
        "p07-asr-qualification-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&run_dir)?;
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P07 ASR qualification report: {}", report_path.display());
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("base accuracy gates not met: {failures:?}").into())
    }
}
