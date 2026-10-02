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
  Ubuntu 24.04 x86-64, through the manual workflow `P13 managed smoke` (job
  `managed-install`), in a fresh per-user base; it downloads the three pinned artifacts.
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
  is described in the [trial runbook](agents/trials.md).

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

It runs each publishing step of `release.yml` against stub `npm`, `gh` and `curl` (CI runs it
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
cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.0-rc.1 --commit <40 hex digits>
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
  `note`), `issues`, `decision`, `reason` and `date`. `release_delta` stays null until P14 PR 8's
  delta check records it.
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
