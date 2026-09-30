//! SEC-T02 over human-readable output (P13 PRs 2a and 2b; closes L-073).
//!
//! The adversarial sidecars `fixtures/corpus/transcripts/F12-adversarial.srt`
//! and `.vtt` (hidden-colour and class-hidden instructions, a forged
//! speaker, bidirectional overrides and zero-width characters, links, an
//! inert download-and-run line and a cue that imitates a `VSift` result) are
//! committed into a session and read back through the binary **without
//! `--json`**, as a person would. A `WebVTT` variant adds a voice name with
//! hidden characters, which reaches the terminal only as `display_label`.
//! Command lines with hostile arguments are rejected in human mode.
//!
//! For `transcript get`, `search`, `session status` and rejected command
//! lines, stdout and stderr must hold:
//!
//! - no ESC, CSI, OSC, C0 control other than the line break, DEL or C1
//!   control, and so no `ESC ] 8 ;` terminal link;
//! - no raw hidden character: each is written as `<U+XXXX>`;
//! - evidence only on quoted lines (`  | `), so a cue that imitates a
//!   result or a command can never stand on a line of its own;
//! - no line longer than a diagnostic (4,096 bytes).
//!
//! P13 PR 2b adds the commands it renders. `candidates`, the frame
//! commands, `crop`, `audio` and the `job` commands run here without
//! `--json` under a session root whose name holds hidden characters (and,
//! off Windows, an OSC-8 link, ANSI colour, line break and C1 control), and
//! must fail inertly. Their results carry no evidence text; the one
//! untrusted text they can carry, a delivered path under such a root, is
//! re-run through the binary in `evidence_cli_contract`
//! (`sec_t02_hostile_session_root_paths_stay_inert_in_human_output`, which
//! can seed evidence without media tools), and for every frame command in
//! the renderers' unit tests (`human::tests`). The worker hosts' human runs,
//! with hostile request text, are in `job_run_cli_contract` and
//! `job_batch_cli_contract`.
//!
//! The golden snapshots under `tests/human_output/` (expiry times replaced
//! by `<TIME>`) are for readability review only: human text is not a
//! contract. Set `VSIFT_UPDATE_HUMAN_SNAPSHOTS=1` to rewrite them. The
//! property test that the `TerminalText` builder never writes a control or
//! hidden character, whatever it is given, is a unit test of the builder
//! (`human::text`), which is private to the crate.

use std::{
    env,
    error::Error,
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, build_imported_revision,
};
use vsift_contract::is_hidden_character;
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot, read_supplied_transcript};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-sec-t02-human-";
const SRT: &str = "fixtures/corpus/transcripts/F12-adversarial.srt";
const VTT: &str = "fixtures/corpus/transcripts/F12-adversarial.vtt";
/// F12's duration in the frozen manifest.
const F12_DURATION_US: u64 = 12_000_000;
const SESSION: &str = "ses_5ec7020000000000000000000000f12a";
/// The longest line a human output may hold, line break excluded.
const MAX_LINE_BYTES: usize = 4_095;
/// The quote prefix of every line of evidence text.
const QUOTE: &str = "  | ";

/// A marker in argument text: a right-to-left override, a zero-width space,
/// an ANSI escape, an OSC-8 link and a line break around ASCII words.
const SENTINEL: &str = "QXSENTINEL\u{202E}ZWREVERSED\u{200B}JOINED\u{1b}[31mESCAPED\u{1b}]8;;https://example.invalid\u{7}LINK\nINJECTED";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf, &'static str);

/// A session-root folder name that holds a right-to-left override and a
/// zero-width space, and where the platform allows them in a name an OSC-8
/// link, an ANSI colour, a line break and a C1 control (P13 PR 2b).
#[cfg(windows)]
const HOSTILE_SESSIONS: &str = "private\u{202E}snoisses\u{200B} sessions";
#[cfg(not(windows))]
const HOSTILE_SESSIONS: &str = "private\u{202E}snoisses\u{200B}\u{1b}]8;;https:example.invalid\u{7}x\u{1b}[31m\nForged: line\u{85} sessions";

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_sessions("private sessions")
    }

    /// An owned folder whose session root is its child `sessions`.
    fn with_sessions(sessions: &'static str) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path, sessions))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn sessions(&self) -> PathBuf {
        self.path(self.1)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.path(name);
        fs::write(&path, bytes)?;
        Ok(path)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// `vsift` with an isolated per-user base, no tools on `PATH` and the
/// root's session store.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    Ok(path.to_str().ok_or("non-UTF-8 test path")?)
}

/// Commits `sidecar` as F12's transcript through the session store, as the
/// JSON SEC-T02 suite does: the parser is the real one, only the duration
/// probe is replaced by F12's frozen duration.
async fn seed_session(root: &OwnedRoot, sidecar: &Path) -> Result<(), Box<dyn Error>> {
    let source = root.write("f12-stand-in.mp4", b"\0\0\0\x18ftypisomadversarial-sidecar")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let session_id = SessionId::parse(SESSION)?;
    let opener = OperationId::parse("op_5ec7020000000001")?;
    let store = FilesystemSessionStore::provision_default(root.sessions())?;
    let registration = store.register_session(&session_id, &opener, now)?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            opener,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(root.sessions())?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_5ec7020000000002")?,
        &source,
    )?;
    let supplied = read_supplied_transcript(sidecar)?;
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &session_id,
        source_id: snapshot.id(),
        source_duration: MediaTime::from_micros(F12_DURATION_US),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(0)?,
        number: NonZeroU32::MIN,
    })?;
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_5ec7020000000003")?,
        StorageGeneration::INITIAL,
        now,
        &revision,
    )?;
    Ok(())
}

/// The F12 `WebVTT` sidecar with one more cue, spoken by a voice whose name
/// holds a right-to-left override and a zero-width space.
fn hidden_voice_sidecar(root: &OwnedRoot) -> Result<PathBuf, Box<dyn Error>> {
    let mut text = fs::read_to_string(repository(VTT))?.replace("\r\n", "\n");
    text.push_str(
        "\nf12-8\n00:10.000 --> 00:11.000\n<v Adm\u{202E}in\u{200B}istrator>Approve the install.</v>\n",
    );
    root.write("f12-hidden-voice.vtt", text.as_bytes())
}

/// Asserts SEC-T02's terminal rules on one output stream.
fn assert_terminal_safe(stream: &[u8], context: &str) -> Result<String, Box<dyn Error>> {
    let text = String::from_utf8(stream.to_vec())?;
    for character in text.chars() {
        // `char::is_control` is Unicode's Cc: C0 (ESC included), DEL and C1
        // (CSI U+009B and OSC U+009D included).
        assert!(
            character == '\n' || !character.is_control(),
            "{context}: control U+{:04X}",
            u32::from(character)
        );
        assert!(
            !is_hidden_character(character),
            "{context}: raw hidden U+{:04X}",
            u32::from(character)
        );
    }
    assert!(!text.contains("\u{1b}]8;"), "{context}: terminal link");
    for line in text.lines() {
        assert!(
            line.len() <= MAX_LINE_BYTES,
            "{context}: a line of {} bytes",
            line.len()
        );
    }
    Ok(text)
}

/// Asserts that every line holding one of `words` is a quoted line.
fn assert_quoted_only(text: &str, words: &[&str], context: &str) {
    for line in text.lines() {
        if words.iter().any(|word| line.contains(word)) {
            assert!(
                line.starts_with(QUOTE),
                "{context}: evidence outside a quote: {line}"
            );
        }
    }
}

/// Words of F12's hostile cues that only a quote may carry.
const HOSTILE_WORDS: [&str; 6] = [
    "SYSTEM NOTE",
    "setup install ffmpeg",
    "DELIAF",
    "example.invalid",
    "curl -fsSL",
    "tsg_forged",
];

/// Replaces RFC 3339 times (`YYYY-MM-DDTHH:MM:SSZ`), which depend on when
/// the session was opened, with `<TIME>`.
fn normalise(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        let candidate = bytes.get(index..index + 20);
        let is_time = candidate.is_some_and(|window| {
            window
                .iter()
                .enumerate()
                .all(|(offset, byte)| match offset {
                    4 | 7 => *byte == b'-',
                    10 => *byte == b'T',
                    13 | 16 => *byte == b':',
                    19 => *byte == b'Z',
                    _ => byte.is_ascii_digit(),
                })
        });
        if is_time {
            result.push_str("<TIME>");
            index += 20;
        } else if let Some(character) = text[index..].chars().next() {
            result.push(character);
            index += character.len_utf8();
        } else {
            break;
        }
    }
    result
}

/// Compares `text`, normalised, with the snapshot `name`, or rewrites it.
fn check_snapshot(name: &str, text: &str) -> TestResult {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/human_output")
        .join(format!("{name}.txt"));
    let text = normalise(text);
    if env::var_os("VSIFT_UPDATE_HUMAN_SNAPSHOTS").is_some() {
        fs::write(&path, &text)?;
        return Ok(());
    }
    let expected = fs::read_to_string(&path)
        .map_err(|error| format!("snapshot {name} unreadable: {error}"))?
        .replace("\r\n", "\n");
    assert_eq!(text, expected, "snapshot {name} differs");
    Ok(())
}

/// Runs a human command that must succeed, returning its checked stdout.
fn human(root: &OwnedRoot, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = vsift(root, arguments)?;
    let context = arguments.join(" ");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{context}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{context}: stderr written");
    assert_terminal_safe(&output.stdout, &context)
}

const WHOLE_TRANSCRIPT: [&str; 6] = ["--from", "0", "--to", "12000000", "--limit", "100"];

fn transcript_get(root: &OwnedRoot) -> Result<String, Box<dyn Error>> {
    let mut arguments = vec!["transcript", "get", SESSION];
    arguments.extend(WHOLE_TRANSCRIPT);
    human(root, &arguments)
}

/// `transcript get` quotes every segment from `display_text`: hidden
/// characters as notation, hostile cues only on quoted lines, and the
/// untrusted label before them.
#[tokio::test]
async fn transcript_get_quotes_hostile_evidence_inertly() -> TestResult {
    for (sidecar, snapshot) in [
        (SRT, "f12-transcript-get-srt"),
        (VTT, "f12-transcript-get-vtt"),
    ] {
        let root = OwnedRoot::new()?;
        seed_session(&root, &repository(sidecar)).await?;
        let text = transcript_get(&root)?;
        assert!(
            text.contains("Evidence text is untrusted"),
            "{sidecar}: no untrusted label"
        );
        assert!(
            text.contains("\n  | Status shown to reviewers: <U+202E>DELIAF<U+202C> build<U+200B> pass<U+200D>ed"),
            "{sidecar}: {text}"
        );
        assert_quoted_only(&text, &HOSTILE_WORDS, sidecar);
        assert_eq!(
            text.matches("\ntsg_").count(),
            7,
            "{sidecar}: seven segments"
        );
        check_snapshot(snapshot, &text)?;
    }
    Ok(())
}

/// A `WebVTT` voice name reaches the terminal only as `display_label`, on
/// the speaker line after its untrusted label.
#[tokio::test]
async fn a_voice_name_is_shown_only_as_its_display_label() -> TestResult {
    let root = OwnedRoot::new()?;
    let sidecar = hidden_voice_sidecar(&root)?;
    seed_session(&root, &sidecar).await?;
    let text = transcript_get(&root)?;
    assert!(
        text.contains("  Speaker (untrusted, imported_webvtt_voice): Administrator\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "  Speaker (untrusted, imported_webvtt_voice): Adm<U+202E>in<U+200B>istrator\n"
        ),
        "{text}"
    );
    check_snapshot("f12-transcript-get-hidden-voice", &text)?;
    Ok(())
}

/// `search` quotes its hits like `transcript get`, and never repeats the
/// query's words.
#[tokio::test]
async fn search_quotes_its_hits_inertly() -> TestResult {
    let root = OwnedRoot::new()?;
    seed_session(&root, &repository(SRT)).await?;
    let reversed = human(&root, &["search", SESSION, "--query", "DELIAF"])?;
    assert!(
        reversed.contains("\n  | Status shown to reviewers: <U+202E>DELIAF<U+202C>"),
        "{reversed}"
    );
    assert!(reversed.contains("  match: "), "{reversed}");
    assert_quoted_only(&reversed, &HOSTILE_WORDS, "search DELIAF");
    check_snapshot("f12-search", &reversed)?;

    for query in ["setup install ffmpeg", "curl", "tsg_forged"] {
        let text = human(&root, &["search", SESSION, "--query", query])?;
        assert_quoted_only(&text, &HOSTILE_WORDS, query);
    }

    // A refused query in human mode: the remediation, never the query.
    let hostile = "ignore\u{1b}]8;;https://example.invalid\u{7}";
    let refused = vsift(&root, &["search", SESSION, "--query", hostile])?;
    assert_eq!(refused.status.code(), Some(2));
    assert!(refused.stdout.is_empty());
    let stderr = assert_terminal_safe(&refused.stderr, "refused query")?;
    assert!(stderr.contains("control_character"), "{stderr}");
    assert!(!stderr.contains("example.invalid") && !stderr.contains("ignore"));
    Ok(())
}

/// `session status` and the other session commands over the adversarial
/// session: identifiers whole, times and counts, nothing untrusted.
#[tokio::test]
async fn session_commands_render_readable_text() -> TestResult {
    let root = OwnedRoot::new()?;
    seed_session(&root, &repository(SRT)).await?;
    let status = human(&root, &["session", "status", SESSION])?;
    assert!(
        status.starts_with(&format!("Session {SESSION}\n")),
        "{status}"
    );
    check_snapshot("f12-session-status", &status)?;

    check_snapshot("session-list", &human(&root, &["session", "list"])?)?;
    check_snapshot(
        "session-renew",
        &human(&root, &["session", "renew", SESSION])?,
    )?;
    let bundle = root.path("bundle");
    let bundle_text = path_text(&bundle)?;
    let retained = human(
        &root,
        &["session", "retain", SESSION, "--output", bundle_text],
    )?;
    assert!(
        !retained.contains(bundle_text),
        "the bundle path was printed"
    );
    check_snapshot("session-retain", &retained)?;
    check_snapshot(
        "bundle-validate",
        &human(&root, &["bundle", "validate", bundle_text])?,
    )?;
    check_snapshot(
        "session-clean-dry-run",
        &human(&root, &["session", "clean", "--expired", "--dry-run"])?,
    )?;
    check_snapshot(
        "session-close",
        &human(&root, &["session", "close", SESSION])?,
    )?;
    Ok(())
}

/// Hostile arguments rejected in human mode: stdout is empty; stderr is
/// the fixed message, the parser's explanation quoted in display form, the
/// typed remediation and its help command.
#[test]
fn rejected_command_lines_with_hostile_arguments_are_inert() -> TestResult {
    let root = OwnedRoot::new()?;
    let flag = format!("--{SENTINEL}");
    let cases: [(&str, Vec<&str>); 4] = [
        (
            "unknown-flag",
            vec!["search", SESSION, "--query", "x", &flag],
        ),
        ("invalid-session", vec!["session", "status", SENTINEL]),
        (
            "invalid-number",
            vec!["transcript", "get", SESSION, "--limit", SENTINEL],
        ),
        ("unknown-command", vec![SENTINEL]),
    ];
    for (name, arguments) in cases {
        let output = vsift(&root, &arguments)?;
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert!(output.stdout.is_empty(), "{name}: stdout written");
        let stderr = assert_terminal_safe(&output.stderr, name)?;
        let lines: Vec<&str> = stderr.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some("Error: The command line arguments are invalid. (INVALID_ARGUMENT)"),
            "{name}: {stderr}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Fix: The command line was rejected (")),
            "{name}: {stderr}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Run: vsift ") && line.ends_with("--help")),
            "{name}: {stderr}"
        );
        // Wherever the parser quotes the argument, it is on quoted lines
        // with its hidden characters as notation.
        assert_quoted_only(&stderr, &["QXSENTINEL", "INJECTED"], name);
        if stderr.contains("QXSENTINEL") {
            assert!(
                stderr.contains("QXSENTINEL<U+202E>ZWREVERSED<U+200B>JOINED"),
                "{name}: {stderr}"
            );
        }
    }
    let output = vsift(&root, &["search", SESSION, "--query", "x", &flag])?;
    check_snapshot(
        "parse-failure-hostile-flag",
        &String::from_utf8(output.stderr)?,
    )?;
    Ok(())
}

/// `setup check`, `setup configure` and `setup configure-model` without
/// `--json`, with no tools on `PATH`.
#[test]
fn setup_commands_render_readable_text() -> TestResult {
    let root = OwnedRoot::new()?;
    let blocked = vsift(&root, &["setup", "check"])?;
    assert_eq!(blocked.status.code(), Some(2));
    let text = assert_terminal_safe(&blocked.stdout, "setup check")?;
    check_snapshot("setup-check-blocked", &text)?;

    let tool = root.write("stand-in-ffprobe", b"not a real tool")?;
    let configured = human(
        &root,
        &[
            "setup",
            "configure",
            "ffprobe",
            "--executable",
            path_text(&tool)?,
        ],
    )?;
    assert!(
        !configured.contains(path_text(&tool)?),
        "the path was printed"
    );
    assert!(configured.starts_with("Registered ffprobe"), "{configured}");
    let model = root.write("stand-in-model.bin", b"not a real model")?;
    let registered = human(
        &root,
        &["setup", "configure-model", "--file", path_text(&model)?],
    )?;
    assert!(
        registered.starts_with("Registered the local ASR model"),
        "{registered}"
    );
    Ok(())
}

/// P13 PR 2b's commands without `--json` over F12's adversarial session in
/// a session root whose name is hostile: with no media tools they fail, and
/// every failure is the fixed text on stderr (stdout empty), terminal-safe
/// and without a word of the root. The session's status still reads as
/// text. Their successful results, whose only untrusted text is a delivered
/// path under such a root, are re-run where evidence can be seeded:
/// `evidence_cli_contract` (`frame get`, `crop`, `audio`), and in the
/// builder's unit tests for every frame command; `candidates` and the `job`
/// commands carry no path and no evidence text.
#[tokio::test]
async fn part_two_commands_fail_inertly_under_a_hostile_session_root() -> TestResult {
    let root = OwnedRoot::with_sessions(HOSTILE_SESSIONS)?;
    seed_session(&root, &repository(SRT)).await?;
    let status = human(&root, &["session", "status", SESSION])?;
    assert!(
        status.starts_with(&format!("Session {SESSION}\n")),
        "{status}"
    );

    let unknown_evidence = "evd_ffffffffffffffffffffffffffffffff";
    let unknown_job = "job_ffffffffffffffffffffffffffffffff";
    let cases: [Vec<&str>; 9] = [
        vec!["candidates", SESSION, "--from", "0", "--to", "12000000"],
        vec!["frame", "get", SESSION, "--at", "0"],
        vec!["frame", "neighbours", SESSION, unknown_evidence],
        vec!["frame", "burst", SESSION, "--from", "0", "--to", "1000000"],
        vec!["crop", SESSION, unknown_evidence, "--rect", "0,0,8,8"],
        vec!["audio", SESSION, "--from", "0", "--to", "1000000"],
        vec!["job", "status", unknown_job],
        vec!["job", "resume", unknown_job],
        vec!["job", "cancel", unknown_job],
    ];
    for arguments in cases {
        let context = arguments.join(" ");
        let output = vsift(&root, &arguments)?;
        assert_ne!(output.status.code(), Some(0), "{context}");
        assert!(output.stdout.is_empty(), "{context}: stdout written");
        let stderr = assert_terminal_safe(&output.stderr, &context)?;
        assert!(stderr.starts_with("Error: "), "{context}: {stderr}");
        for fragment in ["snoisses", "example.invalid", "Forged"] {
            assert!(!stderr.contains(fragment), "{context}: {stderr}");
        }
    }
    Ok(())
}
