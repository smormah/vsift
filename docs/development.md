# Development

## Prerequisites

- Git
- Rustup
- The Rust toolchain declared in `rust-toolchain.toml`

FFmpeg and Whisper are not required to compile or run unit tests. Integration tests that require specialist runtimes must detect and report their prerequisites explicitly.

## First build

```console
git clone https://github.com/smormah/vsift.git
cd vsift
cargo build --workspace --locked
cargo test --workspace --locked
```

Cargo automatically selects the repository toolchain through `rust-toolchain.toml`.

## Required checks

Run these before opening a pull request:

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo doc --workspace --no-deps --locked
```

CI runs the same checks on Windows, macOS, and Linux. If you changed anything under
`npm/`, also run `node --test npm/test/launcher.test.cjs` (see "npm packages" below).

If you changed a parser the fuzz targets use, or anything in `fuzz/`, also run the
fuzz harness checks (the workspace excludes `fuzz/`, so the commands above skip it):

```console
cargo fmt --manifest-path fuzz/Cargo.toml --check
cargo clippy --manifest-path fuzz/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path fuzz/Cargo.toml --locked
```

## Fuzzing

`fuzz/` holds `cargo-fuzz` targets for the parsers of untrusted input
([ADR 0016](decisions/0016-embeddable-engine-and-evidence-contract.md), decision 6):
`transcript_srt`, `transcript_webvtt`, `whisper_full_json`, `transcript_record`,
`ffprobe_metadata`, `transcript_cursor`, `visual_samples` (FFmpeg's `showinfo`
diagnostics of one visual window through `parse_visual_samples` and the window
analysis; the input is a frame count, a pixel seed and the diagnostics, and the target
writes the frames itself), `visual_index_record`, `search_query` (query normalisation
and matching over a query, a line feed and segment text), `frame_showinfo` (the
single-frame and first-audio-sample diagnostics readers), `frame_listing` (a frame
listing's diagnostics against a fixed 60 s window), `png_sequence` (the PNG sequence
walker; the input is an image count, a width and height, then the output),
`evidence_record` (the stored evidence record), `crop_rect` (an outer crop against
a 1440x900 frame, a line feed and an inner crop, parsed and composed), `mountinfo`
(a `/proc/self/mountinfo` table through the durable-profile check, which must give one
verdict for every device and keep it when the table is repeated) and `os_release`
(an `os-release` file through the same check's Ubuntu 24.04 test, whose verdict must
not change when a comment is appended or the file repeated). P11 PR 1 added
`job_request` (a worker request through `vsift_contract::decode_work_request`: an
accepted request keeps its bounds and step order and decodes to itself again, also
with whitespace appended), `job_batch_line` (one `job batch` line through
`decode_batch_line`), and, for issue #180, `job_record` (a stored `job.json` through
`decode_job_record`, round-tripped) and `chunk_checkpoint` (a stored chunk checkpoint
through `decode_chunk_checkpoint`, round-tripped). Their seeds are the frozen request
examples in `schemas/v1/examples/` and the example records in
`crates/vsift-infrastructure/tests/data/jobs/`, which that crate's
`job_record_examples` test pins to the encoder (regenerate with
`VSIFT_REGENERATE_JOB_EXAMPLES=1` and review the diff). P11 PR 2 added
`host_attestation` (the strict worker's kernel files: the same bytes through
`parse_proc_cgroup`, `parse_cpu_max`, `parse_cgroup_limit` and `parse_net_dev`; an
accepted limit reads back from its canonical form and adding an interface line decides
the network verdict as documented; seeds quote the attestation tests) and made
`mountinfo` also check that `classify_root_mount` accepts exactly the tables
`classify_mountinfo` does. P11 PR 3 added `request_record` (a stored worker request
record through `decode_request_record` for a fixed operation id, round-tripped, and
each recorded step and result through the contract's `decode_recorded`, which must
read back to exactly its bytes); its seeds are the example records in
`crates/vsift-infrastructure/tests/data/worker-requests/`, which the engine's
`request_record_examples` test pins to the store's encoder and the contract's
recorded form (regenerate with `VSIFT_REGENERATE_REQUEST_EXAMPLES=1`). P11 PR 4 added
`job_batch_file` (a whole `job batch` file through the reader `BatchLines`, under the
production limits and under small ones of 4 lines of 16 bytes: the count, every line
handed out, its number and whether it was over the bound must match an independent
split of the file at its line feeds, and each line is then decoded as `job batch`
decodes it); its seeds copy the frozen `job-batch.requests.jsonl` and
`job-batch.events.jsonl` in `schemas/v1/examples/`. P13 PR 5 added `handoff_check`
(a draft report through `vsift_contract::HandoffChecker::check_report`, the check of
`vsift handoff check`: an accepted draft of at most 64 KiB of UTF-8 must check the
same way twice, stay within its bounds, have a verdict equal to its errors and publish
only pointers of schema member names and indices and allowed values of the schema's
alphabet, so no draft text leaks); its seeds copy `skills/vsift/SKILL.md` and
`crates/vsift-contract/tests/data/handoff/draft-with-findings.md`. The two
diagnostics targets also require that indented copies of every line, as FFmpeg echoes
source metadata, never change a result; their seeds are the real FFmpeg 9.0 output in
`crates/vsift-infrastructure/tests/data/ffmpeg_diagnostics/`. It is a separate package with its
own lockfile. Each target body is a plain function in `fuzz/src/lib.rs`; the stable replay
tests above run it over every seed in `fuzz/seeds/<target>/`, and the libFuzzer entry
points in `fuzz/fuzz_targets/` (feature `libfuzzer`) run it under libFuzzer.

P14 PR 4's gap review added seven more targets, each through a published parser: `setup_plan`
(the saved setup plan `setup install` reads), `bundle_manifest` (a bundle manifest and the artifacts it
names, through `FilesystemSessionStore::validate_bundle` in a private folder; Unix only, the Windows replay
skips it), `tar_inventory`, `gzip_tar_inventory` and `xz_tar_inventory` (the managed archives'
inventories under a small and a production bound, checked against an independent statement of the archive
rules), `identifiers` (the id, key and label constructors against a grammar model) and `input_path` (the
relative-path and bundle-name grammar of a worker request against a model). A seed is a copy of a reviewed
fixture, an inline example, or derived from code and rebuilt by `VSIFT_REGENERATE_FUZZ_SEEDS=1` (the
replay tests refuse a seed whose origin is not listed in `fuzz/tests/replay.rs`).

The `Fuzz` workflow takes at most 14,400 s a target (`seconds`, default 300; the P14 campaign asks for
3,600) and keeps each target's log, corpus and a coverage-plateau line
(`tools/p14-campaigns/fuzz-summary.cjs`); a pull request that changes `fuzz/` or the workflow runs a
15-second smoke of every target. A long run is dispatched on a hosted runner, never locally:

```console
gh workflow run fuzz.yml --ref <branch> -f seconds=3600
```

libFuzzer needs a nightly toolchain, so the `Fuzz` workflow runs it weekly and on
manual dispatch, never as a per-PR check. To fuzz locally on Linux (x86-64, with a C++
compiler), use the workflow's pinned versions:

```console
rustup toolchain install nightly-2026-09-01 --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
mkdir -p fuzz/corpus/transcript_srt
cargo +nightly-2026-09-01 fuzz run --features libfuzzer transcript_srt \
  fuzz/corpus/transcript_srt fuzz/seeds/transcript_srt -- -max_total_time=300 -timeout=10
```

The first corpus directory collects new inputs and, like `fuzz/artifacts/`, is ignored
by Git. A crash leaves its input in `fuzz/artifacts/<target>/`; reproduce it with
`cargo +nightly-2026-09-01 fuzz run --features libfuzzer <target> <crash-file>`. Every
crash becomes a permanent regression test at the parser's own layer, not in `fuzz/`.
Add a seed only by copying an existing reviewed fixture into `fuzz/seeds/<target>/` and
listing its origin in `fuzz/tests/replay.rs`; the replay tests fail on an unlisted or
drifted seed. The two visual targets' seeds are derived from the recorded visual
samples below; after re-recording them, rewrite the seeds with
`VSIFT_REGENERATE_FUZZ_SEEDS=1 cargo test --manifest-path fuzz/Cargo.toml --locked
the_visual_seeds` and review the diff. On Windows with MSVC, the same commands work when the directory holding
`clang_rt.asan_dynamic-x86_64.dll` (the MSVC `bin\HostX64\x64` directory) is on `PATH`.

## Architectural placement

Before adding code, identify its owner:

| Concern | Crate |
| --- | --- |
| Business concept or invariant | `vsift-domain` |
| Use-case orchestration or provider port | `vsift-application` |
| Filesystem, process, network, model, or storage implementation | `vsift-infrastructure` |
| Versioned JSON wire type, or mapping a domain/application value into one | `vsift-contract` |
| Composing use cases and adapters into an operation every host can call, or its typed result and error | `vsift` (engine) |
| Argument parsing, configuration precedence, human output, exit codes, or building the engine | `vsift-cli` |

A host depends on `vsift` and `vsift-contract` only. When a host needs a new capability,
add a typed engine operation rather than reaching into the application or
infrastructure crates. The `vsift` library API is 0.x and unstable; only the CLI and its
v1 JSON are stable public surfaces.

Do not create a general-purpose `utils` or `helpers` module. Name modules after the capability or concept they own.

## Testing

- Domain tests verify invariants without I/O.
- Application tests use small explicit fakes for ports.
- Infrastructure tests exercise real boundaries using isolated temporary directories and rights-safe fixtures.
  A test that needs a fake provider executable which the code under test runs with its
  own closed argument list uses the standard-library-only `vsift-smoke-fixture` binary
  of `vsift-infrastructure` (`testbin/`, found through `CARGO_BIN_EXE_vsift-smoke-fixture`),
  which takes its behaviour from the file name it is staged under. It is never shipped.
- Engine tests use `vsift` as a library, without the CLI, with an injected controlled
  clock, a sequential identifier source and temporary directories, so identities and
  expiry are exact. Tests that need real FFmpeg/FFprobe are `#[ignore]`d and opt-in.
- Contract tests serialize `vsift-contract` values and validate them against
  `schemas/v1` and its frozen examples, without running the CLI. The managed
  lifecycle examples (`setup-list.json`, `setup-rollback.json`, `setup-remove*.json`,
  `setup-repair.json`) are rewritten by `VSIFT_UPDATE_SCHEMA_EXAMPLES=1 cargo test -p
  vsift-contract --test setup_lifecycle_contract`; review the diff.
- CLI tests execute the compiled binary and verify public output and exit codes.
- JSON changes require compatibility-focused contract tests.

- Commit-path fault points (ADR 0020): the session store names every commit boundary
  (`FaultPoint`, from `artifact-install` to `chain-checkpoint-write`). The store's
  unit tests, and builds with the development-only `fault-injection` feature of
  `vsift-infrastructure`, stop the process there when `VSIFT_FAULT_POINT=<name>[:<n>]`
  is set (exit status 91, a `VSIFT_FAULT_POINT_REACHED=<name>` line on standard error).
  The feature cannot be compiled without debug assertions and the governance check
  refuses it outside development dependencies; never enable it in a release build.
  Since P13 PR 7 the managed store has 22 more points (`FaultPoint::MANAGED`,
  `managed-directory-created` to `managed-stage-marker-removed`); their count spans one
  store handle, its clones and stages, so `<name>:<n>` is the `n`-th arrival in a whole
  command. The kill tests of the managed store live in
  `vsift-infrastructure/tests/p13_install_transaction/kill.rs`, a module of that test
  compiled when `fault-injection` is on too: `cargo test -p vsift-infrastructure
  --features fault-injection,install-test-hooks --test p13_install_transaction kill::`.
  On Linux they try every arrival of every point; elsewhere the first of each, unless
  `VSIFT_P13_KILL_EVERY_ARRIVAL=1` (about ten minutes on Windows).

- Managed-install transport and transaction (P13 PR 4): the development-only
  `install-test-hooks` feature of `vsift-infrastructure` adds a loopback publisher
  route (a local test server on `127.0.0.1`, optionally through an explicit test proxy)
  and an injected stage-write failure. Like `fault-injection` it cannot be compiled
  without debug assertions and the governance check refuses it outside development
  dependencies. `tests/p13_install_transaction.rs` needs it (`required-features`): a
  workspace test run enables it through the engine crate's development dependency; alone,
  run `cargo test -p vsift-infrastructure --features install-test-hooks --test
  p13_install_transaction`. A release-mode run of the crate alone skips that test, so the
  opt-in real-tool tests keep `--release`, which they need anyway: a development build
  resolves no host name for a publisher download (the network guard), so no test run
  without `--release` can reach the internet through `setup install`; set
  `VSIFT_DEV_PUBLISHER_NETWORK=allow` to let a debug build download. The test's
  untrusted TLS identity is a
  throwaway self-signed key in `tests/fixtures/tls/` (README there). The real install
  (`VSIFT_P13_REAL_INSTALL=1`, `p13_managed_install_real` in `vsift-cli`) runs only on
  Ubuntu 24.04 x86-64, through the workflow `P13 managed smoke` (job `managed-install`;
  manual, and weekly against the published binary since P14 PR 3, see "Running a checkpoint
  against an installed binary" below), in a fresh per-user base; it downloads the three
  pinned artifacts.
  The P13 stage of the end-to-end spine (`VSIFT_P13_INSTALL_E2E=1`, `p13_install_e2e`
  in `vsift-cli`, P13 PR 7) runs the same way (job `install-e2e`): it kills the real
  install with `SIGKILL` twice, reruns it, runs the local-ASR journey on the managed
  tools and removes and reinstalls the model, downloading the artifacts up to four
  times.

- Durable-publication crash campaign (P10 PR 4, ADR 0020 section 7): the
  `vsift-crash-campaign` tool in `tools/p10-crash-campaign/` (a workload, a verifier,
  a dm-log-writes replay and a write-error assessor; `cargo test -p
  vsift-crash-campaign` runs its unit tests anywhere) and the `P10 durability campaign`
  workflow, which runs layers A (power loss at every flush), B (QEMU kills of the
  pinned Ubuntu 24.04 cloud image) and C (dm-flakey write errors) on hosted
  `ubuntu-24.04` runners, manually and weekly. Its negative control is built with
  `--profile campaign --features campaign` (the development-only
  `durability-campaign` feature of `vsift-infrastructure`, refused without debug
  assertions and, by the governance check, anywhere but development dependencies and
  that non-default feature) and selected with `VSIFT_CAMPAIGN_NEGATIVE_CONTROL=1`. The
  scripts create loop devices, device-mapper targets and virtual machines: run them
  only on disposable machines. Method and results: the
  [P10 durable-publication record](planning/p10-durable-publication.md). When Ubuntu
  retires the pinned cloud-image release, bump `UBUNTU_IMAGE_URL`,
  `UBUNTU_IMAGE_SHA256` and `UBUNTU_IMAGE_BYTES` together from the new release's
  `SHA256SUMS`.
  Since P13 PR 7 the same tool qualifies the managed store: `--store managed` on
  `workload` and `replay`, and `layer-a.sh`'s seventh argument `managed`, run the
  managed workload (stand-in versions, no tools) and hold every replayed flush to
  zero undone acknowledgements and no damage (ADR 0023 PR 7 note). The workload marks
  each command's start (`start-<seq>`) as well as its acknowledgement (`ack-<seq>`), so
  a point may hold the selection of the one command in flight there instead of the last
  acknowledged one (ADR 0023 PR 7 addendum, 2026-10-01). The manual workflow
  `P13 managed power loss` runs its positive run and negative control on hosted
  `ubuntu-24.04` runners; its negative control skips every managed folder flush and
  each runtime file's flush, and must lose acknowledgements.

- Agent-trial harness (P12 PR 2, ADR 0022 decision 7): `tools/vsift-agent-trials`
  (never published) prepares, runs, grades and records named-client trials of the
  skill; `cargo test -p vsift-agent-trials` runs its grader, stand-in-client and
  scenario tests anywhere and sends no prompt. `cargo run -p vsift-agent-trials --bin
  vsift-agent-trials -- check-scenarios` checks the scenario files against the corpus
  truth and the skill.
  Its command policy and budgets are parsed from `skills/vsift/references/`, so a
  skill change changes grading in the same commit. Operating it against real clients
  is described in the [trial runbook](agents/trials.md). On Windows its `campaign_script`
  tests run the real `campaigns/run-campaign.ps1` against a throwaway Git repository (a
  fake client stops the run at the version pin, so they spend nothing and call no model,
  npm or Docker); they need PowerShell 7 (`pwsh`) and Git on `PATH`, as the runbook does.

Tests must be deterministic and must not depend on internet access. Media fixtures must be generated by the project or have documented redistribution rights.

### Visual candidate recall (P08)

`crates/vsift-infrastructure/tests/data/visual_samples/F*.json` hold, for each visual
fixture, the time, 16x9 block means and hash of every sample FFmpeg decoded through
`FfmpegMedia::visual_samples`, with the FFmpeg version and date as provenance (about
100 KB in total; no pixels). The always-run `p08_candidate_recall` test replays them
through the real index extension and gates recall against the manifest truth, so it
needs no media tool. With FFmpeg and FFprobe on `PATH`:

```console
cargo test --release -p vsift-infrastructure --locked --test p08_candidates_fixtures -- --ignored --nocapture
```

runs the same gate on a live decode through `FfmpegVisualSampler`, prints the recall
report and the decode throughput, and reports how far a fresh decode drifts from the
recorded samples. To re-record them (after an FFmpeg upgrade that changes the report,
or a change to the sampling argv), run it with `VSIFT_RECORD_VISUAL_SAMPLES=1`, review
the diff and the report, then regenerate the fuzz seeds as above. Never edit the
manifest to make the gate pass.

The same gate through the `candidates` command, with the lead/lag, continuation,
stream and bundle, damaged-media, V-03 motion and S-11 stages, is the opt-in
checkpoint (see [the E2E spine](planning/e2e-test-spine.md)):

```console
cargo test --release -p vsift-cli --locked --test p08_candidates_e2e -- --ignored --nocapture
```

After an intended contract change, `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` rewrites the
frozen `candidates*.json(l)` examples (`vsift-contract`'s `candidates_contract`),
`bundle-visual-index-record.json` (`vsift-infrastructure`'s `visual_index_store`) and
`bundle-evidence-record.json` (`vsift-infrastructure`'s `evidence_store`); review the
diff before committing.

### Running a checkpoint against an installed binary (P14 PR 3)

By default every real-tool checkpoint (`p06_setup_e2e`, `p07_transcript_e2e`,
`p07_local_asr_e2e`, `p08_search_e2e`, `p08_candidates_e2e`, `p09_evidence_e2e`,
`p10_recovery_e2e`, `p11_worker_e2e`, `p13_install_e2e`, `p13_managed_install_real`) drives the
`vsift` that Cargo builds inside the test run. To drive the **published** binary instead, three
environment variables select and identify it; the one module that reads them is
`crates/vsift-cli/tests/published_binary/mod.rs`, and nothing sets them by default:

| Variable | Meaning |
| --- | --- |
| `VSIFT_E2E_BINARY` | Absolute path of the native `vsift` executable to run |
| `VSIFT_E2E_EXPECTED_VERSION` | The version it must print, for example `0.1.0` or `0.2.0-rc.1` |
| `VSIFT_E2E_EXPECTED_COMMIT` | The commit its tag points at (at least twelve lowercase hex digits; the binary prints twelve) |

The override is **refused, never ignored**, when the path is relative, names no file, or the
other two variables are missing or malformed, or when `<path> --version` does not print exactly
`vsift <version> (<commit>)` for that version and a commit the expected one starts with (a build
without `VSIFT_SOURCE_COMMIT` prints no commit and is refused). Without `VSIFT_E2E_BINARY`
nothing changes. Each checkpoint's `report.json` gains `binary_under_test` (`source`, the
`--version` line and the executable's SHA-256, never its path). `CARGO_BIN_EXE_vsift` cannot be
used for this: `assert_cmd` 2.2.2 reads it when a test runs, but `cargo test` sets it itself and
overwrites any value from outside.

Point it at the **native executable** the platform package ships, not at the `vsift` shim that
npm puts on `PATH`: the checkpoints run `vsift` with an empty `PATH` (no Node.js) and signal it
directly. For an npm install into a scratch folder on Windows, from a checkout of the release
tag (the tag `v0.1.0` predates the override, so use the current checkout's tests and the tag's
commit):

```powershell
npm install --prefix $env:TEMP\vsift-published vsift-cli@0.1.0 --ignore-scripts
$env:VSIFT_E2E_BINARY = "$env:TEMP\vsift-published\node_modules\@vsift\win32-x64\vsift.exe"
$env:VSIFT_E2E_EXPECTED_VERSION = "0.1.0"
$env:VSIFT_E2E_EXPECTED_COMMIT = (git rev-parse "v0.1.0^{commit}")   # after: git fetch --tags
cargo test --release -p vsift-cli --locked --test p07_transcript_e2e -- --ignored --nocapture
```

(`@vsift/darwin-arm64/vsift` and `@vsift/linux-x64/vsift` elsewhere.) Unset the three variables
afterwards. The helper's own tests (`e2e_binary_override`, no tools needed) run in every build.
A run on the maintainer's machine was made with a release build copied out of the tree and
named by the same variables (the mechanism is the same); the npm path itself runs on the hosted
runners, because nothing is installed on a development machine without its owner's word.

`p14_installed_binary_e2e` is the checkpoint written for an installed binary: hostile file
names (quotes, spaces, shell and glob characters, a leading dash, Unicode, and on Unix a newline,
a tab and a backslash) through the real tools, and a sentinel environment (variables that look
like secrets must not reach a tool child, which a recorder started by `vsift` proves). It has no
libtest harness and prints `skipped` unless `VSIFT_P14_INSTALLED_E2E=1`.

The hosted run is the workflow **P14 journeys** (`.github/workflows/p14-journeys.yml`, driven by
`tools/p14_journeys.py`, whose guardrails are `tools/test_p14_journeys.py`): on Ubuntu 24.04,
Windows and macOS 15 it installs `vsift-cli@<version>` from the real npm registry into a fresh
folder, stages the system's tools, and runs the checkpoints above with the override. It runs on
dispatch (`version` empty means the highest published), on a pull request that touches the
workflow or its tooling, and weekly. `P13 managed smoke` takes the same override through its
`published_version` input (or `highest`) and runs weekly too. Each run's job summary says which
tests ran, which did not and why; a failure is a finding to file before any rerun.

**Tests newer than the binary: a stage that needs a later version.** The tests of a run come from
the tag when the tag has the override and otherwise from the workflow's ref
([L-115](planning/known-limits.md#l-115)), so they move on while a published version stays put. A
published version cannot be held to a behaviour that was fixed after it: the stage
`p07_local_asr_cut_range` (#274) found that the published 0.1.0 fails a recognition range cut
mid-speech on all three systems, and would have failed every weekly run while 0.1.0 is the highest
published version. So a stage that asserts such a behaviour declares the **first version that has
it** (`published_binary::predates("0.2.0-rc.1")`) and reports itself `skipped`, with the reason and
`first_version`, only for a published binary older than that. A build from source and every
published version at or above the first one run the stage and must pass it. The driver lists a
skipped stage under "Stages that did not pass" with its reason, and **fails the checkpoint** for a
skip that names no first version or whose version is not strictly older: a skip is never a
pass and never a way to weaken a stage for a version that has the fix (`unjustified_skips`, tested in
`tools/test_p14_journeys.py`; the comparison is SemVer's, tested in `e2e_binary_override`). Add a
gate only for a behaviour fixed after the oldest version the weekly run still resolves, and say
which issue fixed it.

## Dependencies

Before adding a crate, review:

- whether the standard library already solves the problem clearly;
- maintenance activity and security history;
- transitive dependency cost;
- platform support and minimum Rust version;
- licence compatibility;
- whether it introduces native build requirements.

Commit `Cargo.lock` because VSift is an application. Dependency changes must pass `cargo deny check` in CI.

The fuzz harness's lockfile (`fuzz/Cargo.lock`) adds only `libfuzzer-sys` 0.4.13,
`arbitrary` 1.4.2 and `jobserver` 0.1.35 (a build helper of `cc`) to the workspace's
graph; its review, and the
open licence question for `libfuzzer-sys`'s declared NCSA term, are in the ADR 0016
note of 2026-09-24.

P02 uses `process-wrap` 10 for safe cross-platform access to Windows Job Objects and
Unix process groups. Its enabled features, MSRV, transitive footprint and isolation
limitations are recorded in [ADR 0003](decisions/0003-external-runtime-adapters.md).

P13 PR 5 made `regex` 1.13 (MIT OR Apache-2.0, the Rust project's) a production
dependency of `vsift-contract` for the handoff schema's patterns, without its Unicode
tables (`default-features = false, features = ["std"]`); it adds `regex`,
`regex-automata` and `regex-syntax` to the release graph. `jsonschema` stays a
development dependency (and the trial grader's): as a production one it added 43
crates and 5.5 MB to the release binary. The review is in the ADR 0023 note of
2026-09-30 ("P13 PR 5").

P13 PR 8 added `yaml-rust2` 0.13 (MIT OR Apache-2.0, pure Rust, with `arraydeque`
and `hashlink`) to the governance checker only, which parses `.github/workflows/*.yml`
for the workflow lint; no shipped crate depends on it. The review is in the ADR 0023
note of 2026-09-30 ("P13 PR 8").

## Release archives

`.github/workflows/release.yml` builds the native archives and `tools/vsift-release`
packages and checks them; only a dispatch of a release tag by the maintainer, approved in
the protected `release` environment, publishes (P13 PR 10 wired the attest and publish
jobs; they ran for the first time on 2026-10-01 and published 0.1.0;
`operations/release.md` section 6). What they hold, how
the build is kept reproducible and how to check a run's archives is in
[`operations/release.md`](operations/release.md). A release build selects no feature:
the governance check (`cargo run --locked -p vsift-governance -- check`) fails a
`release.yml` that does, and a workflow that breaks its lint (pinned actions, no
`pull_request_target`, read-only top-level `permissions`, `id-token` only for the
release attest and publish jobs, no untrusted `${{ }}` in a `run` script, and, in every
workflow, nothing that moves a dist-tag or publishes outside the `publish` job).

A version without a pre-release suffix is **stable** and moves npm's `latest`; a version
with one is published under `next` (P14 PR 8; `operations/release.md` sections 1 and 6.7). Changing
the release tool, the Release workflow or its lint is the highest-risk change in the repository, so
it comes with all of: a lint rule and a deliberately broken copy of the workflow that it must name
(`tools/vsift-governance/src/workflows/publish.rs`), the tool's tests, and a check of the shell. Run
the shell check by hand on Linux or in Git Bash:

```console
bash tools/vsift-release/tests/publish-steps.sh .github/workflows/release.yml
```

It runs each publishing step of `release.yml`, and the plan job's registry, candidate and evidence
steps, against stub `npm`, `gh`, `curl` and `cargo` (CI runs it
through `cargo test -p vsift-release` on Linux); no real service is touched. To see what a stable
version would be held to, run `cargo run --locked -p vsift-release -- candidate-delta` at its
commit in a clone that has the tags. Bumping the workspace version touches only the files of
`operations/release.md` 6.8, and a test that hard-codes the version defeats that check: read
`env!("CARGO_PKG_VERSION")` instead.

`vsift --version` prints `vsift <version> (<commit>)` when `VSIFT_SOURCE_COMMIT` holds
the full commit SHA at build time, as the release workflow sets it; a build without it
prints the version alone.

## npm packages

`npm/vsift-cli/` holds the npm launcher's sources: `bin/vsift.cjs`, which runs `lib/launcher.cjs` (plain CommonJS, `node:`
built-ins only, no dependencies), `package.json` and `README.md`. The platform packages
are generated from the release archives by `vsift-release npm`
([`operations/release.md`](operations/release.md) section 5). If you change the launcher,
run its tests with Node.js 22 or later (CI job `npm launcher` runs them on Windows, macOS
and Linux):

```console
node --test npm/test/launcher.test.cjs
```

They copy the Node.js executable into fake package layouts as a stand-in for `vsift`
(about 100 MB each, in the temporary folder) and, on Windows, send a console
Ctrl-Break with `tools/send-console-ctrl.ps1`. No package may have `scripts`, a
`gypfile`, a `binding.gyp` or a person in its manifest; the governance check fails
otherwise. Bumping the workspace version means changing `npm/vsift-cli/package.json`'s
`version` and its three optional dependencies with it, and the other files that hold the
version (`operations/release.md` section 6.3, step 1, lists them). The full qualification with npm,
pnpm, Yarn and Bun against a loopback Verdaccio runs in the Release workflow; to run it
yourself, see the same runbook section. It never publishes to a public registry, and
nothing in this repository may.

## Published-artifact qualification tools (P14)

`tools/p14-published/` holds plain Node.js tools (CommonJS, no dependency) that install and run
the **published** VSift on hosted runners: the real registry's packages with npm, pnpm, Yarn and
Bun, the release archives, the offline install with the real reviewed artifacts, the upgrade, and a
second verification of a publish. Four workflows run them: `P14 published artifacts`, `P14 local
upgrade`, `P14 verify release` and `P14 compatibility`
([`planning/p14-qualification.md`](planning/p14-qualification.md) section 15 says what each proves
and what it does not; the folder's `README.md` lists the scripts). They publish nothing, hold no
secret and need only the job's default read-only token; the governance workflow lint and
`test/pins.test.cjs` hold them to that.

```console
node --test "tools/p14-published/test/*.test.cjs"
```

The tests need no network and no published package. The tools themselves do: they read the public
registry and the GitHub release, so run them on a hosted runner (dispatch the workflow, or open a
pull request that changes the workflow or the tools). Only `verify-release.cjs` is safe elsewhere
(it reads, and downloads the release files into its work folder); the others install packages,
make sessions, run the published binary and delete folders in a throwaway user state, so do not run
them on a machine whose state you care about. Their pins (Node.js, Bun, pnpm, Yarn, Verdaccio, the actions,
the Ubuntu image) are the Release workflow's; change them there and in the P14 workflows together,
or the pins test fails. A pull request that changes `tools/p14-published/**` or one of the four
workflows starts the first three workflows; one that changes the schemas, the contract or the
record readers starts `P14 compatibility`, which fetches the history and requires the checked-in
copy of 0.1.0's JSON examples (`schemas/v1/frozen/v0.1.0/`) to be the tag's bytes. The tests behind
that workflow, `published_compatibility` and `published_v0_1_0_records`, also run in every Quality
job with the checked-in copy.

## Robustness campaigns (P14)

`tools/p14-campaigns/` holds Node.js tools (CommonJS, no dependency) for the campaigns that stress
the product on hosted runners ([`planning/p14-qualification.md`](planning/p14-qualification.md)
section 18 has the results and what each does and does not show). **They are for hosted runners and
disposable machines only.** They kill processes, fill disks, mount file systems, build containers,
create hostile files and install packages; never run one on a machine whose state you care about. What
is safe locally is the tooling's own tests, which need no network, no container and no published
package:

```console
node --test "tools/p14-campaigns/test/*.test.cjs"
```

| Campaign | Workflow (a pull request that changes it or its tools runs a small smoke) | Full run, from a hosted dispatch |
| --- | --- | --- |
| Fuzzing (RQ-07) | `Fuzz` | `gh workflow run fuzz.yml --ref <branch> -f seconds=3600` |
| Race and stress repetitions (RQ-08) | `P14 stress` | `gh workflow run p14-stress.yml --ref <branch>`; inputs `runs` (200), `long_runs` (1500, the supervisor and root-creation suites, plain and with every CPU busy) and `delivery_runs` (100) |
| Load ladder, 100-request batch, cancel, warm page and soak (RQ-09) | `P14 load` | `gh workflow run p14-load.yml --ref <branch>`; inputs `phases`, `ladder_requests` (24), `batch_requests` (100), `soak_requests` (1000) and `soak_minutes` (270) |
| Malicious media (RQ-10) | `P14 malicious media` | `gh workflow run p14-malicious-media.yml --ref <branch>` (the pull request's run is already the full set) |
| The worker runbook walk (RQ-12) | `P14 runbook walk` | `gh workflow run p14-runbook-walk.yml --ref <branch>` |
| The hosted part of the scan reading (RQ-13) | `P14 scan reading` | `gh workflow run p14-scan-reading.yml --ref <branch>`, then the read-only `gh api` calls listed in the reading |

Each workflow is read-only (`contents: read`), uses no secret, installs the **published** `vsift-cli`
from the real registry (an empty `version` means the one on `next`) and keeps its record for 90 days as an
artifact (`load-summary.md`, `hostile-summary.md`, `runbook-summary.md`, the per-job `summary.md` of the
stress run, the fuzz logs). A hosted job is limited to six hours, so the soak stops starting rounds after
`soak_minutes`. The load and walk jobs build the worker image from `tools/p14-campaigns/worker.Dockerfile`
around the published executable, make an ext4 volume in a file for the state folder and the bundle root (a
runner's own disk is mounted without write barriers and refuses `durable`), and install FFmpeg, whisper.cpp
and the model with the published binary's own `setup install`. A finding is a result: it gets an issue
before the run is repeated (governance rule 14), not a rerun. The governance workflow lint and
`tools/p14-published/test/pins.test.cjs` hold the workflows to the same pins and read-only scopes as the
others.

Three rules of the malicious-media campaign, all learned from one case that tested the wrong thing
([known limit L-134](planning/known-limits.md#l-134)):

- **A session root is a folder that does not exist yet.** VSift never adopts a folder it did not create
  and refuses one after a wait, before it looks at the source. A case that needs a filesystem of its own
  (`tmpfs` in `lib/hostile-cases.cjs`) gets it mounted for its containers, and its roots are folders inside
  the mount (`sessionRoot` in `hostile-media.cjs`), never the mount point.
- **A case that is about one answer pins it** (`answers`, by operation: the codes and, where a code alone
  does not say what happened, how the remediation begins). The plan's three codes are the rule for a
  hostile input in general; a pinned operation may give only its own answer, so a typed failure for another
  reason is a finding.
- **A filed finding is tracked for one operation and one outcome** (`TRACKED` in `hostile-media.cjs`).
  The same case answering anything else is new and fails the run. When a tracked case passes, the summary
  says to remove it from the list.

## Governance checks, release evidence and public claims

`cargo run --locked -p vsift-governance -- check` is the Governance job. Besides the delivery
ledger, the fixture corpus, the handoff files, the workflow lint and the npm package rules, it
checks the structure of the P14 release evidence ledger
(`docs/planning/p14-evidence-ledger.json`) and the public-claims registry
(`docs/planning/public-claims.json`) ([delivery governance](planning/delivery-governance.md)).
Run either on its own while you edit it, and the completeness check when a release needs it:

```console
cargo run --locked -p vsift-governance -- release-evidence
cargo run --locked -p vsift-governance -- public-claims
cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.0-rc.3 --commit <40 hex digits>
```

- **The ledger's shape** (schema version 1; the Rust types in
  `tools/vsift-governance/src/release_evidence/schema.rs` are the schema, and every struct
  rejects unknown fields): a header (`schema_version`, `packet`, `plan`, `release_delta`) and
  one item per plan row with `id`, `title`, `supports` (`requirements`, `threats`,
  `verification`, `limits`), `proves`, `does_not_prove`, `producer` (`kind`, `p14_pr`,
  `description`), `scope`, `gate` (`candidate`: `required` or `not_required`; `stable`:
  `repeat`, `carry` or `not_required`), `status` (`planned`, `running`, `passed`, `failed`,
  `waived`, `not_applicable`), `applies_to` (`version`, `commit`), `evidence` and `prior` (each a
  typed `reference` of `workflow_run`, `pull_request`, `issue` or `record`, with a `date` and a
  `note`), `issues`, `decision`, `reason` and `date`. `release_delta` stays null until
  the maintainer copies the stable plan's `release-delta.json` (P14 PR 8's delta check) into it
  after the publish.
- **Changing an evidence item's status** is a change to the ledger: set `status`, give the
  version and commit it is for (`applies_to`) and at least one typed link (a workflow run, a
  pull request, a repository record), move what no longer counts into `prior`, and add the
  `date`. The checker refuses a `passed` item without evidence, a `waived` one without the
  maintainer's decision, and so on. Keep each item's `scope` honest: it is the list of paths
  whose change makes older evidence stale.
- **Changing a public sentence** that uses `supported`, `stable`, `qualified` or kin changes
  the registry: the sentence must be inside a registered statement (a `claim` with its rung and
  the evidence it needs, or a `non_claim` such as a negation). A statement the documents no
  longer contain must be removed, or the check calls it stale.
- `--complete-for` needs the Git history between the evidence's commit and the target
  commit (a full checkout, not a shallow one); `--commit` defaults to `HEAD`. It is not part of
  the Governance job: it fails by design until every item the release needs is answered.
- The tool reads files and, for completeness only, runs `git diff` with explicit arguments and
  no shell. It fetches nothing and needs no credentials.

## The user guide and its checks

The R0 user guide is `docs/guide/` ([the plan it follows](planning/user-guide-spec.md)). Two
checks hold it to the code. Both are plain Node.js 22 (CommonJS, no dependencies, `node:test`)
in `tools/guide/`, and both run in the `Guide` workflow (`.github/workflows/guide.yml`), which
needs no secret and publishes nothing.

```console
node --test "tools/guide/test/*.test.cjs"
cargo build --locked -p vsift-cli
node tools/guide/generate-reference.cjs --binary target/debug/vsift --write
node tools/guide/generate-reference.cjs --binary target/debug/vsift --check
node tools/guide/check-examples.cjs --binary target/debug/vsift
```

- **The generated reference** (`docs/guide/reference/commands.md` from `vsift --help` and every
  sub-command's help, `docs/guide/reference/json.md` from `schemas/v1`). Never edit these two
  pages by hand: change the code or the schema, run `--write`, commit the result. `--check`
  fails if the committed pages differ from what the binary and the schemas give, and it also
  holds the hand-written pages to what they promise: the `<!-- guide-version: X -->` marker
  equals the binary's version, `troubleshooting.md` has one row per v1 failure code with the
  exit status the contract's taxonomy gives it, every relative link and anchor leads
  somewhere, and every page is in the `documents` of the public-claims registry.
- **The examples** (`check-examples.cjs`). A fenced `console` block that follows a
  `<!-- check -->` line is run against the real binary: each `$ vsift ...` line is a command,
  the lines beneath it are what the page shows, and the check fails if the real output is not
  what the page shows. Variants are `<!-- check: exit 3 -->` (the commands end with that
  status; the page is showing a failure) and `<!-- check: needs speech -->` (skipped unless a
  speech recogniser and model are installed). The commands run on the synthetic recordings of
  `fixtures/corpus/generated/` that the page names, copied into a sandbox of their own for each
  page, so a page never depends on another page's state and never touches your real VSift
  folders.
- **What is compared by kind, not by value.** Session, segment, evidence and candidate
  identifiers, digests, times of day, file sizes, durations, file paths, the commit in
  `vsift --version` and, for speech blocks, what the recogniser decides (the words, the spans,
  the confidence). A line of `...` stands for any number of lines, so a page may trim a long
  result, and a trimmed result says so. Everything else, including every word of every message,
  must match exactly. `lib/normalise.cjs` holds the list; adding to it needs a reason, because
  every mask makes the check weaker.
- **Placeholders.** `<session>`, `<segment>`, `<frame>`, `<crop>`, `<clip>` and the like stand
  for the first identifier of that kind an earlier command printed on the same page;
  `<candidate:3>` picks the third. A placeholder with no value yet fails the check rather than
  being guessed.
- **A version bump re-checks the guide.** The guide names a release (`0.2.0`), not a
  candidate: `0.2.0-rc.1`, `0.2.0-rc.2`, `0.2.0-rc.3` and `0.2.0` are the same release to these tools, so the stable commit
  needs no guide change, which matters because it may differ from its accepted candidate only in
  version-string files, shipped documents and the work record, and the generated reference pages,
  which name the release, are in none of them ([release process, 6.8](operations/release.md)).
  In the bump to a new release (before its first candidate is cut), run the examples, run
  `generate-reference.cjs --write`, and update the `guide-version` marker and the sentence
  beside it in `docs/guide/index.md`.
- **Tools and folders.** Everything runs in a throw-away folder (under `--work`, default the
  system temp folder) with its own `LOCALAPPDATA` and `XDG_DATA_HOME`, so your own VSift
  setup and sessions are never read or changed. `--install-managed` runs the binary's own
  `setup plan` and `setup install` into that folder, as the workflow does (Ubuntu 24.04 x64
  only; it downloads the three reviewed tools from their publishers, and a development build refuses to
  resolve publisher hosts unless `VSIFT_DEV_PUBLISHER_NETWORK=allow` is set, as the workflow does). Without it, FFmpeg and
  FFprobe must be on `PATH`. `--whisper <executable> --model <file>` registers your own
  whisper.cpp and model, and `--require-speech` turns a skipped speech block into a failure.
  On Windows keep `--work` short (for example `C:\vg`): the media tools fail on paths longer than
  Windows allows.

## Documentation

P03's cap-std/cap-fs-ext 4.0.3 storage dependencies and their review are in
the [storage feasibility record](planning/p03-storage-feasibility.md). Reproduce
the bounded probe with `cargo test --locked -p vsift-infrastructure --test
storage_feasibility -- --nocapture`. Report filesystem identity with the result;
these observations qualify API behavior, not durable publication. The production
adapter also uses the already-transitive Rustix 1.1 process feature on Unix to verify
root ownership, and Windows-only `windows-acl` 0.3.0 for read-only DACL inspection.
The latter is MIT licensed and unarchived but has low maintenance activity (last push
2023-06-09); its narrow use is isolated behind `cfg(windows)`, cargo-deny reports no
advisory, and replacement remains appropriate if a maintained safe DACL reader appears.
`deny.toml` explicitly includes development dependencies in licence and duplicate
checks; cargo-deny otherwise omits them from these checks by default. Keep the
explicit settings when evaluating future test-only platform wrappers.

Public behaviour belongs in the README or command documentation. Architectural decisions belong in `docs/decisions`. Security assumptions belong in `SECURITY.md` and the relevant design document.
