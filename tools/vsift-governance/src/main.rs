//! Validates the repository's machine-readable delivery controls.

#![forbid(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use serde::Deserialize;

mod npm_packages;
mod workflows;

const DEFAULT_LEDGER: &str = "docs/planning/delivery-ledger.json";
const OBJECTIVE: &str = "Deliver an agent-operated local video evidence tool with a production-quality single-host worker core.";
const EXPECTED_DECISIONS: usize = 13;
const EXPECTED_REQUIREMENTS: usize = 14;
const EXPECTED_PACKETS: usize = 15;
const EXPECTED_INVARIANTS: usize = 10;
const CORPUS_MANIFEST: &str = "fixtures/corpus/manifest.json";

/// Session handoff files and their maximum line counts.
///
/// These files are read at the start of every implementation session. When they
/// were allowed to grow as append-only logs they reached hundreds of lines, and an
/// agreement to keep them short did not survive because nothing enforced it
/// (ADR 0015). History belongs in git, the changelog and `docs/history/`.
const HANDOFF_FILE_LIMITS: [(&str, usize); 2] = [
    ("memory/TODO.md", 100),
    ("memory/project_current_status.md", 150),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeliveryLedger {
    schema_version: String,
    release: String,
    objective: String,
    source_documents: Vec<String>,
    scope: Scope,
    invariants: Vec<Invariant>,
    decisions: Vec<Decision>,
    requirements: Vec<Requirement>,
    packets: Vec<Packet>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    included: Vec<String>,
    deferred: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusManifest {
    schema_version: String,
    corpus_id: String,
    licence: String,
    timeline_unit: String,
    fixtures: Vec<CorpusFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusFixture {
    id: String,
    slug: String,
    purpose: String,
    duration_us: u64,
    resolution: Resolution,
    audio: FixtureAudio,
    events: Vec<FixtureEvent>,
    expected_terms: Vec<String>,
    tests: Vec<String>,
    #[serde(default)]
    variants: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resolution {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureAudio {
    mode: AudioMode,
    script: String,
    offset_us: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum AudioMode {
    None,
    CleanSynthetic,
    NoisySynthetic,
    OffsetSynthetic,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEvent {
    id: String,
    kind: EventKind,
    start_us: u64,
    end_us: u64,
    critical: bool,
    truth: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum EventKind {
    Stable,
    Change,
    Transient,
    Scroll,
    Speech,
    Malformed,
    Adversarial,
    /// How long an element drawn by the other events stays visible; it marks
    /// no change, so it is never critical (2026-09-28 truth amendment).
    Persistent,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invariant {
    id: String,
    statement: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    id: String,
    status: DecisionStatus,
    adr: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DecisionStatus {
    Accepted,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Requirement {
    id: String,
    packets: Vec<String>,
    tests: Vec<String>,
    fixtures: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Packet {
    id: String,
    status: PacketStatus,
    issue_url: String,
    depends_on: Vec<String>,
    requirements: Vec<String>,
    tests: Vec<String>,
    threats: Vec<String>,
    merge_commit: Option<String>,
    verification: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PacketStatus {
    Planned,
    InProgress,
    Complete,
}

#[derive(Debug)]
struct GovernanceError {
    messages: Vec<String>,
}

impl fmt::Display for GovernanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "delivery ledger validation failed:")?;
        for message in &self.messages {
            writeln!(formatter, "- {message}")?;
        }
        Ok(())
    }
}

impl Error for GovernanceError {}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("VSift delivery ledger is valid.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next();
    let ledger_path = arguments
        .next()
        .map_or_else(|| PathBuf::from(DEFAULT_LEDGER), PathBuf::from);

    if command.as_deref() != Some("check") || arguments.next().is_some() {
        return Err(Box::new(GovernanceError {
            messages: vec![String::from(
                "usage: cargo run -p vsift-governance -- check [ledger-path]",
            )],
        }));
    }

    let text = fs::read_to_string(&ledger_path)?;
    let ledger: DeliveryLedger = serde_json::from_str(&text)?;
    let repository_root = ledger_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    let corpus_text = fs::read_to_string(repository_root.join(CORPUS_MANIFEST))?;
    let corpus: CorpusManifest = serde_json::from_str(&corpus_text)?;
    let messages = validate(&ledger, &corpus, repository_root);

    if messages.is_empty() {
        Ok(())
    } else {
        Err(Box::new(GovernanceError { messages }))
    }
}

fn validate(
    ledger: &DeliveryLedger,
    corpus: &CorpusManifest,
    repository_root: &Path,
) -> Vec<String> {
    let mut messages = Vec::new();

    require_equal(&mut messages, "schema_version", &ledger.schema_version, "1");
    require_equal(&mut messages, "release", &ledger.release, "R0");
    require_equal(&mut messages, "objective", &ledger.objective, OBJECTIVE);
    require_count(
        &mut messages,
        "decisions",
        ledger.decisions.len(),
        EXPECTED_DECISIONS,
    );
    require_count(
        &mut messages,
        "requirements",
        ledger.requirements.len(),
        EXPECTED_REQUIREMENTS,
    );
    require_count(
        &mut messages,
        "packets",
        ledger.packets.len(),
        EXPECTED_PACKETS,
    );
    require_count(
        &mut messages,
        "invariants",
        ledger.invariants.len(),
        EXPECTED_INVARIANTS,
    );

    require_non_empty(&mut messages, "scope.included", &ledger.scope.included);
    require_non_empty(&mut messages, "scope.deferred", &ledger.scope.deferred);
    validate_documents(&mut messages, &ledger.source_documents, repository_root);
    validate_invariants(&mut messages, &ledger.invariants);
    validate_decisions(&mut messages, &ledger.decisions, repository_root);
    validate_requirements_and_packets(&mut messages, &ledger.requirements, &ledger.packets);
    validate_corpus(&mut messages, corpus);
    validate_handoff_files(&mut messages, repository_root);
    validate_fault_injection_features(&mut messages, repository_root);
    workflows::validate_workflows(&mut messages, repository_root);
    npm_packages::validate_npm_packages(&mut messages, repository_root);

    messages
}

/// Checks that no build a user receives can enable a development-only
/// feature ([`DEVELOPMENT_FEATURES`]).
///
/// The `fault-injection` feature lets `VSIFT_FAULT_POINT` stop the process at
/// a commit boundary (ADR 0020); `install-test-hooks` lets a managed download
/// reach a loopback test server (P13). Only the infrastructure crate may
/// define them, and they may be enabled only from development dependencies,
/// never by default, by a normal dependency or by a published crate's
/// features. The crate itself also refuses to compile them without debug
/// assertions.
fn validate_fault_injection_features(messages: &mut Vec<String>, root: &Path) {
    let mut manifests = vec![String::from("Cargo.toml"), String::from("fuzz/Cargo.toml")];
    for directory in ["crates", "tools"] {
        match fs::read_dir(root.join(directory)) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect();
                names.sort();
                manifests.extend(
                    names
                        .into_iter()
                        .map(|name| format!("{directory}/{name}/Cargo.toml"))
                        .filter(|path| root.join(path).is_file()),
                );
            }
            Err(error) => messages.push(format!("{directory} could not be listed: {error}")),
        }
    }
    for manifest in manifests {
        match fs::read_to_string(root.join(&manifest)) {
            Ok(text) => check_fault_injection_manifest(messages, &manifest, &text),
            Err(error) => messages.push(format!("{manifest} could not be read: {error}")),
        }
    }
}

/// The development-only features, and the one manifest allowed to define them.
///
/// `fault-injection` stops the process at a named commit boundary;
/// `durability-campaign` lets the crash campaign's negative control remove a
/// directory synchronisation (ADR 0020); `install-test-hooks` routes a
/// managed download to a local test server, bypassing the reviewed
/// catalogue's trust anchor, and fails a stage write on request (P13). All
/// three weaken a build on request.
const DEVELOPMENT_FEATURES: [&str; 3] = [
    "fault-injection",
    "durability-campaign",
    "install-test-hooks",
];
const FAULT_INJECTION_OWNER: &str = "crates/vsift-infrastructure/Cargo.toml";
/// The crash campaign tool, which is never published and may enable
/// `durability-campaign` through a non-default feature of its own.
const CAMPAIGN_TOOL: &str = "tools/p10-crash-campaign/Cargo.toml";
const CAMPAIGN_FEATURE: &str = "durability-campaign";

fn check_fault_injection_manifest(messages: &mut Vec<String>, manifest: &str, text: &str) {
    let unpublished = text.lines().any(|line| {
        line.split_once('=')
            .is_some_and(|(key, value)| key.trim() == "publish" && value.trim() == "false")
    });
    let mut section = "";
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            section = trimmed;
            continue;
        }
        for feature in DEVELOPMENT_FEATURES {
            if !trimmed.contains(feature) {
                continue;
            }
            let definition = manifest == FAULT_INJECTION_OWNER
                && section == "[features]"
                && trimmed.starts_with(&format!("{feature} ="));
            let development = section.ends_with("dev-dependencies]");
            // A test target of the owning crate that builds only when the
            // feature is on names it without enabling it.
            let gated_test = manifest == FAULT_INJECTION_OWNER
                && section == "[[test]]"
                && trimmed.starts_with("required-features");
            let campaign_tool = manifest == CAMPAIGN_TOOL
                && feature == CAMPAIGN_FEATURE
                && unpublished
                && section == "[features]"
                && !trimmed.starts_with("default");
            if !definition && !development && !campaign_tool && !gated_test {
                messages.push(format!(
                    "{manifest} line {} enables {feature} outside a development dependency; \
                     it must never reach a release build (ADR 0020)",
                    index + 1
                ));
            }
        }
    }
}

fn validate_handoff_files(messages: &mut Vec<String>, root: &Path) {
    for (relative_path, maximum_lines) in HANDOFF_FILE_LIMITS {
        match fs::read_to_string(root.join(relative_path)) {
            Ok(text) => check_handoff_length(messages, relative_path, &text, maximum_lines),
            Err(error) => messages.push(format!(
                "handoff file {relative_path} could not be read: {error}"
            )),
        }
    }
}

fn check_handoff_length(
    messages: &mut Vec<String>,
    relative_path: &str,
    text: &str,
    maximum_lines: usize,
) {
    let lines = text.lines().count();
    if lines > maximum_lines {
        messages.push(format!(
            "handoff file {relative_path} has {lines} lines, above its limit of \
             {maximum_lines}; rewrite it as current state and move history to git, \
             CHANGELOG.md or docs/history/"
        ));
    }
}

fn validate_corpus(messages: &mut Vec<String>, corpus: &CorpusManifest) {
    require_equal(
        messages,
        "corpus.schema_version",
        &corpus.schema_version,
        "1",
    );
    require_equal(
        messages,
        "corpus.corpus_id",
        &corpus.corpus_id,
        "vsift-r0-synthetic",
    );
    require_equal(
        messages,
        "corpus.licence",
        &corpus.licence,
        "MIT OR Apache-2.0",
    );
    require_equal(
        messages,
        "corpus.timeline_unit",
        &corpus.timeline_unit,
        "microseconds",
    );

    let expected: HashSet<String> = (1..=12).map(|index| format!("F{index:02}")).collect();
    let actual = unique_ids(
        messages,
        "fixture",
        corpus.fixtures.iter().map(|fixture| &fixture.id),
    );
    if actual != expected {
        messages.push(String::from("fixture IDs must be exactly F01 through F12"));
    }

    for fixture in &corpus.fixtures {
        if fixture.slug.trim().is_empty() || fixture.purpose.trim().is_empty() {
            messages.push(format!("{} requires a slug and purpose", fixture.id));
        }
        if fixture.duration_us == 0
            || fixture.resolution.width == 0
            || fixture.resolution.height == 0
        {
            messages.push(format!("{} has invalid duration or dimensions", fixture.id));
        }
        if fixture.audio.mode == AudioMode::None && !fixture.audio.script.is_empty() {
            messages.push(format!("{} has a script but declares no audio", fixture.id));
        }
        if fixture.audio.mode != AudioMode::None && fixture.audio.script.trim().is_empty() {
            messages.push(format!("{} declares audio without a script", fixture.id));
        }
        if (fixture.audio.mode == AudioMode::OffsetSynthetic) != fixture.audio.offset_us.is_some() {
            messages.push(format!(
                "{} has inconsistent audio offset metadata",
                fixture.id
            ));
        }
        require_non_empty(messages, &format!("{}.events", fixture.id), &fixture.events);
        require_non_empty(messages, &format!("{}.tests", fixture.id), &fixture.tests);

        if !fixture.events.iter().any(|event| event.critical) {
            messages.push(format!("{} has no critical ground-truth event", fixture.id));
        }
        for event in &fixture.events {
            validate_event(messages, fixture, event);
        }
        if fixture
            .expected_terms
            .iter()
            .any(|term| term.trim().is_empty())
        {
            messages.push(format!("{} contains an empty expected term", fixture.id));
        }
        if fixture
            .variants
            .iter()
            .any(|variant| variant.trim().is_empty())
        {
            messages.push(format!("{} contains an empty variant", fixture.id));
        }
    }
}

fn validate_event(messages: &mut Vec<String>, fixture: &CorpusFixture, event: &FixtureEvent) {
    if !event.id.starts_with(&format!("{}-E", fixture.id)) {
        messages.push(format!("{} event ID is outside its fixture", event.id));
    }
    if event.start_us >= event.end_us || event.end_us > fixture.duration_us {
        messages.push(format!("{} has an invalid time range", event.id));
    }
    if event.truth.trim().is_empty() {
        messages.push(format!("{} has empty ground truth", event.id));
    }
    match event.kind {
        // A persistent event only records how long a drawn element stays
        // visible; it marks no change, so it can never be a critical event.
        EventKind::Persistent if event.critical => {
            messages.push(format!(
                "{} is persistent and must not be critical",
                event.id
            ));
        }
        EventKind::Stable
        | EventKind::Change
        | EventKind::Transient
        | EventKind::Scroll
        | EventKind::Speech
        | EventKind::Malformed
        | EventKind::Adversarial
        | EventKind::Persistent => {}
    }
}

fn validate_documents(messages: &mut Vec<String>, documents: &[String], root: &Path) {
    require_non_empty(messages, "source_documents", documents);
    unique_ids(messages, "source document", documents.iter());
    for document in documents {
        if !root.join(document).is_file() {
            messages.push(format!("source document does not exist: {document}"));
        }
    }
}

fn validate_invariants(messages: &mut Vec<String>, invariants: &[Invariant]) {
    let expected: HashSet<String> = (1..=EXPECTED_INVARIANTS)
        .map(|index| format!("INV-{index:02}"))
        .collect();
    let actual: HashSet<String> = invariants.iter().map(|item| item.id.clone()).collect();

    if actual != expected {
        messages.push(String::from(
            "invariant IDs must be exactly INV-01 through INV-10",
        ));
    }
    for invariant in invariants {
        if invariant.statement.trim().is_empty() {
            messages.push(format!("{} has an empty statement", invariant.id));
        }
    }
}

fn validate_decisions(messages: &mut Vec<String>, decisions: &[Decision], root: &Path) {
    let expected: HashSet<String> = (1..=EXPECTED_DECISIONS)
        .map(|index| format!("DEC-{index:02}"))
        .collect();
    let actual: HashSet<String> = decisions.iter().map(|item| item.id.clone()).collect();

    if actual != expected {
        messages.push(String::from(
            "decision IDs must be exactly DEC-01 through DEC-13",
        ));
    }
    if decisions
        .iter()
        .any(|decision| decision.status != DecisionStatus::Accepted)
    {
        messages.push(String::from("every P00 decision must be accepted"));
    }
    for decision in decisions {
        if !root.join(&decision.adr).is_file() {
            messages.push(format!(
                "{} ADR does not exist: {}",
                decision.id, decision.adr
            ));
        }
    }
}

fn validate_requirements_and_packets(
    messages: &mut Vec<String>,
    requirements: &[Requirement],
    packets: &[Packet],
) {
    let requirement_ids = expected_ids("R", EXPECTED_REQUIREMENTS);
    let packet_ids: HashSet<String> = (0..EXPECTED_PACKETS)
        .map(|index| format!("P{index:02}"))
        .collect();
    let actual_requirements = unique_ids(
        messages,
        "requirement",
        requirements.iter().map(|item| &item.id),
    );
    let actual_packets = unique_ids(messages, "packet", packets.iter().map(|item| &item.id));

    if actual_requirements != requirement_ids {
        messages.push(String::from(
            "requirement IDs must be exactly R-01 through R-14",
        ));
    }
    if actual_packets != packet_ids {
        messages.push(String::from("packet IDs must be exactly P00 through P14"));
    }

    for (index, packet) in packets.iter().enumerate() {
        let expected = format!("P{index:02}");
        if packet.id != expected {
            messages.push(format!(
                "packet at ledger position {index} must be {expected}, found {}",
                packet.id
            ));
        }
    }

    validate_packet_status_sequence(messages, packets);

    let packet_order: HashMap<&str, usize> = packets
        .iter()
        .enumerate()
        .map(|(index, packet)| (packet.id.as_str(), index))
        .collect();

    validate_requirements(messages, requirements, &actual_packets);
    validate_packets(messages, packets, &actual_requirements, &packet_order);
    validate_reciprocal_traceability(messages, requirements, packets);
}

fn validate_requirements(
    messages: &mut Vec<String>,
    requirements: &[Requirement],
    packet_ids: &HashSet<String>,
) {
    for requirement in requirements {
        require_non_empty(
            messages,
            &format!("{}.packets", requirement.id),
            &requirement.packets,
        );
        require_non_empty(
            messages,
            &format!("{}.tests", requirement.id),
            &requirement.tests,
        );
        require_non_empty(
            messages,
            &format!("{}.fixtures", requirement.id),
            &requirement.fixtures,
        );
        for packet in &requirement.packets {
            if !packet_ids.contains(packet) {
                messages.push(format!(
                    "{} references unknown packet {packet}",
                    requirement.id
                ));
            }
        }
        for fixture in &requirement.fixtures {
            if !is_numbered_id(fixture, "F", 12) {
                messages.push(format!(
                    "{} references unknown fixture {fixture}",
                    requirement.id
                ));
            }
        }
    }
}

fn validate_packets(
    messages: &mut Vec<String>,
    packets: &[Packet],
    requirement_ids: &HashSet<String>,
    packet_order: &HashMap<&str, usize>,
) {
    for (index, packet) in packets.iter().enumerate() {
        if !packet
            .issue_url
            .starts_with("https://github.com/smormah/vsift/issues/")
        {
            messages.push(format!("{} has an invalid issue URL", packet.id));
        }
        require_non_empty(messages, &format!("{}.tests", packet.id), &packet.tests);
        if packet.id != "P00" {
            require_non_empty(
                messages,
                &format!("{}.requirements", packet.id),
                &packet.requirements,
            );
        }
        for requirement in &packet.requirements {
            if !requirement_ids.contains(requirement) {
                messages.push(format!(
                    "{} references unknown requirement {requirement}",
                    packet.id
                ));
            }
        }
        for dependency in &packet.depends_on {
            match packet_order.get(dependency.as_str()) {
                None => messages.push(format!(
                    "{} references unknown dependency {dependency}",
                    packet.id
                )),
                Some(dependency_index) if *dependency_index >= index => messages.push(format!(
                    "{} dependency {dependency} must appear earlier in the ledger",
                    packet.id
                )),
                Some(_) => {}
            }

            if matches!(
                packet.status,
                PacketStatus::Complete | PacketStatus::InProgress
            ) && packets
                .iter()
                .find(|item| &item.id == dependency)
                .is_some_and(|item| item.status != PacketStatus::Complete)
            {
                messages.push(format!(
                    "{} cannot be {:?} while dependency {dependency} is incomplete",
                    packet.id, packet.status
                ));
            }
        }
        validate_completion(messages, packet);
    }
}

fn validate_reciprocal_traceability(
    messages: &mut Vec<String>,
    requirements: &[Requirement],
    packets: &[Packet],
) {
    for requirement in requirements {
        for packet_id in &requirement.packets {
            if let Some(packet) = packets.iter().find(|item| &item.id == packet_id)
                && !packet.requirements.contains(&requirement.id)
            {
                messages.push(format!(
                    "{} and {} must reference each other",
                    requirement.id, packet.id
                ));
            }
        }
    }
    for packet in packets {
        for requirement_id in &packet.requirements {
            if let Some(requirement) = requirements.iter().find(|item| &item.id == requirement_id)
                && !requirement.packets.contains(&packet.id)
            {
                messages.push(format!(
                    "{} and {} must reference each other",
                    packet.id, requirement.id
                ));
            }
        }
    }
}

fn validate_packet_status_sequence(messages: &mut Vec<String>, packets: &[Packet]) {
    let active_count = packets
        .iter()
        .filter(|packet| packet.status == PacketStatus::InProgress)
        .count();
    if active_count > 1 {
        messages.push(String::from("at most one packet may be in progress"));
    }

    let mut encountered_incomplete = false;
    for packet in packets {
        match packet.status {
            PacketStatus::Complete if encountered_incomplete => messages.push(format!(
                "{} cannot be complete after an incomplete packet",
                packet.id
            )),
            PacketStatus::Complete => {}
            PacketStatus::InProgress | PacketStatus::Planned => encountered_incomplete = true,
        }
    }
}

fn validate_completion(messages: &mut Vec<String>, packet: &Packet) {
    match packet.status {
        PacketStatus::Complete => {
            let valid_commit = packet.merge_commit.as_deref().is_some_and(|commit| {
                commit.len() == 40 && commit.chars().all(|value| value.is_ascii_hexdigit())
            });
            if !valid_commit {
                messages.push(format!(
                    "{} is complete without a full merge commit",
                    packet.id
                ));
            }
            require_non_empty(
                messages,
                &format!("{}.verification", packet.id),
                &packet.verification,
            );
        }
        PacketStatus::Planned | PacketStatus::InProgress => {
            if packet.merge_commit.is_some() {
                messages.push(format!(
                    "{} has a merge commit but is not complete",
                    packet.id
                ));
            }
            if !packet.verification.is_empty() {
                messages.push(format!(
                    "{} has completion evidence but is not complete",
                    packet.id
                ));
            }
        }
    }

    if packet.threats.is_empty() && packet.id != "P00" {
        messages.push(format!("{} has no threat mapping", packet.id));
    }
}

fn expected_ids(prefix: &str, count: usize) -> HashSet<String> {
    (1..=count)
        .map(|index| format!("{prefix}-{index:02}"))
        .collect()
}

fn is_numbered_id(value: &str, prefix: &str, maximum: usize) -> bool {
    (1..=maximum).any(|index| value == format!("{prefix}{index:02}"))
}

fn unique_ids<'a>(
    messages: &mut Vec<String>,
    kind: &str,
    values: impl Iterator<Item = &'a String>,
) -> HashSet<String> {
    let mut result = HashSet::new();
    for value in values {
        if !result.insert(value.clone()) {
            messages.push(format!("duplicate {kind} ID: {value}"));
        }
    }
    result
}

fn require_non_empty<T>(messages: &mut Vec<String>, field: &str, values: &[T]) {
    if values.is_empty() {
        messages.push(format!("{field} must not be empty"));
    }
}

fn require_equal(messages: &mut Vec<String>, field: &str, actual: &str, expected: &str) {
    if actual != expected {
        messages.push(format!("{field} must be {expected:?}, found {actual:?}"));
    }
}

fn require_count(messages: &mut Vec<String>, field: &str, actual: usize, expected: usize) {
    if actual != expected {
        messages.push(format!(
            "{field} must contain {expected} items, found {actual}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CORPUS_MANIFEST, CorpusManifest, DEFAULT_LEDGER, DeliveryLedger, EventKind, PacketStatus,
        check_fault_injection_manifest, check_handoff_length, validate,
    };
    use std::{error::Error, fs, io, path::PathBuf};

    #[test]
    fn handoff_file_within_its_limit_is_accepted() {
        let mut messages = Vec::new();

        check_handoff_length(&mut messages, "memory/TODO.md", &"line\n".repeat(10), 10);

        assert!(messages.is_empty(), "{messages:#?}");
    }

    #[test]
    fn oversized_handoff_file_is_rejected() {
        let mut messages = Vec::new();

        check_handoff_length(&mut messages, "memory/TODO.md", &"line\n".repeat(11), 10);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("memory/TODO.md has 11 lines")),
            "{messages:#?}"
        );
    }

    #[test]
    fn fault_injection_is_defined_once_and_enabled_only_for_development() {
        let owner = "crates/vsift-infrastructure/Cargo.toml";
        let mut messages = Vec::new();
        check_fault_injection_manifest(
            &mut messages,
            owner,
            "[features]\n# fault-injection is development only\nfault-injection = []\n",
        );
        check_fault_injection_manifest(
            &mut messages,
            "crates/vsift-cli/Cargo.toml",
            "[dev-dependencies]\nvsift-infrastructure = { workspace = true, features = [\"fault-injection\"] }\n",
        );
        check_fault_injection_manifest(
            &mut messages,
            "crates/vsift/Cargo.toml",
            "[target.'cfg(unix)'.dev-dependencies]\nx = { features = [\"fault-injection\"] }\n",
        );
        assert!(messages.is_empty(), "{messages:#?}");

        for (manifest, text) in [
            (
                owner,
                "[features]\ndefault = [\"fault-injection\"]\nfault-injection = []\n",
            ),
            (
                "crates/vsift-cli/Cargo.toml",
                "[dependencies]\nvsift-infrastructure = { workspace = true, features = [\"fault-injection\"] }\n",
            ),
            (
                "crates/vsift/Cargo.toml",
                "[features]\nfault-injection = []\n",
            ),
            (
                "crates/vsift/Cargo.toml",
                "[features]\ntests = [\"vsift-infrastructure/fault-injection\"]\n",
            ),
        ] {
            let mut messages = Vec::new();
            check_fault_injection_manifest(&mut messages, manifest, text);
            assert_eq!(messages.len(), 1, "{manifest}: {text}");
        }
    }

    /// P13: the loopback publisher route and injected stage-write failure
    /// are refused everywhere but their definition and a development
    /// dependency, exactly like fault injection.
    #[test]
    fn install_test_hooks_are_enabled_only_for_development() {
        let owner = "crates/vsift-infrastructure/Cargo.toml";
        let mut messages = Vec::new();
        check_fault_injection_manifest(
            &mut messages,
            owner,
            "[features]\ninstall-test-hooks = []\n[[test]]\nname = \"p13_install_transaction\"\nrequired-features = [\"install-test-hooks\"]\n[dev-dependencies]\nvsift-infrastructure = { workspace = true, features = [\"install-test-hooks\"] }\n",
        );
        assert!(messages.is_empty(), "{messages:#?}");
        for (manifest, text) in [
            (
                owner,
                "[features]\ndefault = [\"install-test-hooks\"]\ninstall-test-hooks = []\n",
            ),
            (
                "crates/vsift-cli/Cargo.toml",
                "[dependencies]\nvsift-infrastructure = { workspace = true, features = [\"install-test-hooks\"] }\n",
            ),
            (
                "crates/vsift/Cargo.toml",
                "[features]\nhooks = [\"vsift-infrastructure/install-test-hooks\"]\n",
            ),
            (
                "crates/vsift-cli/Cargo.toml",
                "[[test]]\nrequired-features = [\"install-test-hooks\"]\n",
            ),
        ] {
            let mut messages = Vec::new();
            check_fault_injection_manifest(&mut messages, manifest, text);
            assert_eq!(messages.len(), 1, "{manifest}: {text}");
        }
    }

    #[test]
    fn the_campaign_feature_reaches_only_the_unpublished_campaign_tool() {
        let tool = "tools/p10-crash-campaign/Cargo.toml";
        let mut messages = Vec::new();
        check_fault_injection_manifest(
            &mut messages,
            "crates/vsift-infrastructure/Cargo.toml",
            "[features]\ndurability-campaign = []\n",
        );
        check_fault_injection_manifest(
            &mut messages,
            tool,
            "[package]\npublish = false\n[features]\ncampaign = [\"vsift-infrastructure/durability-campaign\"]\n",
        );
        check_fault_injection_manifest(
            &mut messages,
            "crates/vsift/Cargo.toml",
            "[dev-dependencies]\nx = { features = [\"durability-campaign\"] }\n",
        );
        assert!(messages.is_empty(), "{messages:#?}");

        for (manifest, text) in [
            (
                tool,
                "[package]\n[features]\ncampaign = [\"vsift-infrastructure/durability-campaign\"]\n",
            ),
            (
                tool,
                "[package]\npublish = false\n[features]\ndefault = [\"vsift-infrastructure/durability-campaign\"]\n",
            ),
            (
                tool,
                "[package]\npublish = false\n[dependencies]\nx = { features = [\"durability-campaign\"] }\n",
            ),
            (
                tool,
                "[package]\npublish = false\n[features]\ncampaign = [\"vsift-infrastructure/fault-injection\"]\n",
            ),
            (
                "crates/vsift-cli/Cargo.toml",
                "[package]\npublish = false\n[features]\ncampaign = [\"vsift-infrastructure/durability-campaign\"]\n",
            ),
        ] {
            let mut messages = Vec::new();
            check_fault_injection_manifest(&mut messages, manifest, text);
            assert_eq!(messages.len(), 1, "{manifest}: {text}");
        }
    }

    #[test]
    fn checked_in_governance_records_are_valid() -> Result<(), Box<dyn Error>> {
        let (ledger, corpus, root) = load_records()?;

        let messages = validate(&ledger, &corpus, &root);

        assert!(messages.is_empty(), "{messages:#?}");
        Ok(())
    }

    #[test]
    fn persistent_event_must_not_be_critical() -> Result<(), Box<dyn Error>> {
        let (ledger, mut corpus, root) = load_records()?;
        let event = corpus
            .fixtures
            .iter_mut()
            .flat_map(|fixture| fixture.events.iter_mut())
            .find(|event| event.kind == EventKind::Persistent)
            .ok_or_else(|| io::Error::other("checked-in corpus has no persistent event"))?;
        event.critical = true;

        let messages = validate(&ledger, &corpus, &root);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("is persistent and must not be critical")),
            "{messages:#?}"
        );
        Ok(())
    }

    #[test]
    fn complete_packet_requires_commit_and_verification() -> Result<(), Box<dyn Error>> {
        let (mut ledger, corpus, root) = load_records()?;
        let packet = ledger
            .packets
            .first_mut()
            .ok_or_else(|| io::Error::other("checked-in ledger has no packets"))?;
        packet.status = PacketStatus::Complete;
        packet.merge_commit = None;
        packet.verification.clear();

        let messages = validate(&ledger, &corpus, &root);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("full merge commit"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("verification"))
        );
        Ok(())
    }

    #[test]
    fn packet_cannot_depend_on_future_work() -> Result<(), Box<dyn Error>> {
        let (mut ledger, corpus, root) = load_records()?;
        let packet = ledger
            .packets
            .get_mut(1)
            .ok_or_else(|| io::Error::other("checked-in ledger has no P01 packet"))?;
        packet.depends_on = vec![String::from("P14")];

        let messages = validate(&ledger, &corpus, &root);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("must appear earlier"))
        );
        Ok(())
    }

    #[test]
    fn active_packet_requires_completed_predecessors() -> Result<(), Box<dyn Error>> {
        let (mut ledger, corpus, root) = load_records()?;
        let first = ledger
            .packets
            .first_mut()
            .ok_or_else(|| io::Error::other("checked-in ledger has no P00 packet"))?;
        first.status = PacketStatus::Planned;
        let second = ledger
            .packets
            .get_mut(1)
            .ok_or_else(|| io::Error::other("checked-in ledger has no P01 packet"))?;
        second.status = PacketStatus::InProgress;

        let messages = validate(&ledger, &corpus, &root);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("dependency P00 is incomplete"))
        );
        Ok(())
    }

    #[test]
    fn packet_and_requirement_traceability_must_be_reciprocal() -> Result<(), Box<dyn Error>> {
        let (mut ledger, corpus, root) = load_records()?;
        let requirement = ledger
            .requirements
            .first_mut()
            .ok_or_else(|| io::Error::other("checked-in ledger has no requirements"))?;
        requirement.packets.clear();

        let messages = validate(&ledger, &corpus, &root);

        assert!(
            messages
                .iter()
                .any(|message| { message.contains("P04 and R-01 must reference each other") })
        );
        Ok(())
    }

    fn load_records() -> Result<(DeliveryLedger, CorpusManifest, PathBuf), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ledger: DeliveryLedger =
            serde_json::from_str(&fs::read_to_string(root.join(DEFAULT_LEDGER))?)?;
        let corpus: CorpusManifest =
            serde_json::from_str(&fs::read_to_string(root.join(CORPUS_MANIFEST))?)?;
        Ok((ledger, corpus, root))
    }
}
