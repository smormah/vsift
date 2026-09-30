# ADR 0023: R0 distribution, managed installation and handoff check

- Status: **Proposed** (2026-09-30). The maintainer started P13 on 2026-09-30 and
  accepted every recommendation of the P13 plan (decisions A-H below). The ADR is
  accepted when P13 completes with its evidence.
- Date: 2026-09-30
- Tracking: [P13 / issue #16](https://github.com/smormah/vsift/issues/16);
  [#213](https://github.com/smormah/vsift/issues/213) (`handoff check`)
- Refines: [ADR 0001](0001-rust-native-cli.md) (native binaries and npm),
  [ADR 0007](0007-managed-runtime-provisioning.md) (managed provisioning),
  [ADR 0008](0008-cli-and-json-contract.md) (namespace, human output, v1 contract),
  [ADR 0009](0009-package-identity-and-distribution.md) (names and publication),
  [ADR 0014](0014-progressive-dependency-setup.md) (setup journey) and
  [ADR 0022](0022-agent-skill-and-named-client-qualification.md) (the skill and its
  handoff)
- Applies to: P13; R-03, R-13, R-14; D-02..D-08, R-SEC01, R-SEC02, SEC-T02;
  SEC-12..SEC-15, SEC-22, SEC-23

## Context

P00-P12 are complete. VSift can be built only from source with Rust (L-036), managed
dependency installation has been parked since 2026-09-23 (L-037, ADR 0015), most
commands print indented JSON where ADR 0008 promises readable terminal text (L-017,
L-073), a command line that fails to parse gets no remediation (L-071), and P12's
compact-tier trials failed mostly on the handoff's format rather than its content
(#213, L-085). P13 owns all of this. It is the last packet before release
qualification (P14), so it also decides what is published, where, under which names
and with which trust signals.

What the code has today (checked on `main` at `c644645`):

- **Managed install primitives, no transaction.** `ManagedArtifactStore` (in
  `crates/vsift-infrastructure/src/managed_artifact_store.rs`) has the private marked
  root, the root-wide install guard, exact-byte import (also from an offline byte
  source), payload and runtime staging, `publish_and_select`, rollback selection and
  fenced removal. `managed_catalogue.rs` holds the one accepted catalogue revision,
  Ubuntu 24.04 x86-64, with its digest-bound compatibility policy. There is no smoke
  executor, no transaction composing download, stage, smoke and publish, no managed
  tier in provider lookup and no version cleanup.
- **Reserved commands.** `setup install` revalidates a saved plan and its digest,
  then answers `COMMAND_NOT_IMPLEMENTED` (`crates/vsift-cli/src/lib.rs`); `setup
  repair/list/remove/rollback` answer it at once.
- **Contract values.** The setup-check `lookup` enum is `explicit_path`,
  `configured_user_path` and `filtered_path`; the setup-check remediation's
  `managed_install` is the constant `unavailable_unqualified`; the setup-plan
  availability for the accepted target is `catalogue_accepted_install_pending`.
- **Handoff checks live in the grader.** `tools/vsift-agent-trials/src/handoff.rs`
  extracts and checks a handoff against `skills/vsift/handoff.schema.json`.
  `jsonschema` is a development dependency of every crate and a production one only
  of the trial harness.
- **Release plumbing.** Every workflow already pins its actions by commit SHA and
  declares `permissions`; nothing requests `id-token`. `vsift --version` prints the
  crate version only. The HTTPS transport uses `reqwest` with native TLS, so a Linux
  build links the system OpenSSL.

## Decision

### A. Names

The npm launcher is the unscoped `vsift` package. The per-platform packages live in
one npm scope owned by an organisation the maintainer creates: `@<scope>/win32-x64`,
`@<scope>/darwin-arm64` and `@<scope>/linux-x64` (written `@<scope>/…` in this ADR).
The executable stays `vsift` (ADR 0001, ADR 0009).

*Amendment, 2026-09-30 (maintainer): the scope is `@vsift`.* The maintainer owns the
npm organisation `vsift`, so the packages are `@vsift/win32-x64`, `@vsift/darwin-arm64`
and `@vsift/linux-x64`. Read `@<scope>/…` elsewhere in this ADR as `@vsift/…`. A brief
same-day choice of `@shongo` (#235) is withdrawn. The reasons are in the ADR 0009 note of
the same date.

*Correction, 2026-09-30 (maintainer), itself corrected by the amendment above (the
maintainer's first submission had created `vsift`; the message came from a repeated
submission):* the plan named the scope `@vsift` with an
organisation `vsift`, but npm refused that organisation name ("not available"): npm
organisation names share the user-name namespace, which an anonymous `npm view` of
package names cannot see. The candidates, in the maintainer's order of preference, are
`@vsift-cli` (packages `@vsift-cli/win32-x64`, `@vsift-cli/darwin-arm64`,
`@vsift-cli/linux-x64`), then `@vsifthq`, then `@vsiftdev`. The chosen scope is
recorded in an ADR 0009 note, which forbids a silent rename, and then here.

*Why:* the unscoped name is what people type; a scope keeps the platform packages
under one owner and makes a look-alike package visibly foreign. An anonymous
`npm view` (empty user and global configuration, no credentials) returned not-found
for `vsift` on 2026-09-30, rechecked by the maintainer the same day; that is an
observation, not a reservation (ADR 0009).

### B. What is published, and when

Nothing is published during P13's pull requests. At P13 completion, with the
maintainer's approval, one quiet 0.x pre-release is published to npm under the
dist-tag `next`, with its native archives on GitHub Releases. A stable release waits
for P14. No crate is published to crates.io in R0.

*Why:* a name is held only once the release publishes to it (ADR 0009), and the
pre-release proves the real publishing path before P14 depends on it; `next` keeps it
out of a plain `npm install vsift`.

*Amendment, 2026-09-30 (maintainer):* one exception. The maintainer personally publishes a
placeholder `vsift@0.0.0` (README and `package.json` only; no code, binaries or install
scripts) to hold the unscoped name during P13. This supersedes ADR 0009's no-placeholder
rule; the reasons are in ADR 0009's note of the same date. Nothing else is published
before completion. crates.io needs every workspace crate published
and an MSRV and semver policy (ADR 0016 decision 3), none of which R0 needs.

### C. Trust signals

Release artifacts carry Sigstore build-provenance attestations
(`actions/attest-build-provenance`) and npm provenance only. There is no Authenticode
signing and no Apple notarization in R0. `docs/operations/install.md` documents what
Windows SmartScreen and macOS Gatekeeper do with an unsigned binary downloaded
directly, and how to verify an archive against `SHA256SUMS` and its attestation.

*Why:* both signals are free, tied to the protected workflow and verifiable by
anyone (`gh attestation verify`, `npm audit signatures`). Code-signing certificates
and an Apple developer account are recurring costs and key-custody work that R0 does
not need; installing through npm avoids the browser quarantine that triggers
Gatekeeper. The threat model's rule stands: no claim of publisher trust for unsigned
artifacts.

### D. Platforms

The R0 targets are Windows 11 x64, macOS 15 arm64 and Ubuntu 24.04 x64 (ADR 0005's
desktop and worker targets). Windows is built with the MSVC toolchain and a static C
runtime; macOS on a macOS 15 arm64 runner; Linux as x64 glibc on Ubuntu 22.04, so the
binary also runs on newer glibc.

*Why:* these are the only targets with qualification evidence. Building Linux on the
older runner widens compatibility at no cost; a musl build is not needed for R0.

### E. Managed installation on Ubuntu 24.04 x64 only

Managed installation is qualified on Ubuntu 24.04 x86-64 only, the target of the one
accepted catalogue. Windows and macOS keep the typed manual and bring-your-own
guidance they have today (`unavailable_target`).

*Why:* ADR 0014 requires one qualified managed target and a manual path everywhere
else. The Windows FFmpeg candidate is a rotating daily build with unreconciled
notices (L-037), and there is no reviewed macOS candidate.

### F. Order of work

The P12 debt comes first, as separate pull requests on their own branch (#218-#221,
#224, L-085), because it touches the skill, the guard and the grader that `handoff
check` also changes. The compact tier is re-run once (#222) after `handoff check`
lands, so one run measures both.

### G. How a draft reaches `handoff check`

The command reads the draft from standard input, or from `--file <path>`. The skill
teaches exactly two literal forms: a quoted heredoc on POSIX shells
(`vsift handoff check --json <<'EOF'` ... `EOF`) and a single-quoted here-string piped
into it on PowerShell (`@'` ... `'@ | vsift handoff check --json`). This is the one
exception to the skill's rule that no command is piped or redirected.

*Why:* agents often may not write files, a draft does not fit safely in an argument,
and a quoted heredoc or single-quoted here-string passes the text without expansion.
Two fixed forms keep the exception small enough for the skill's guard and the grader
to recognise exactly.

### H. Technical defaults

1. **The validator answers, it does not fail.** `handoff check` exits 0 for any draft
   it could read, with `data.valid`; each problem carries a JSON pointer, a rule
   identifier and the allowed values where the rule has them, and never repeats text
   from the draft.
2. **Human output is the default.** Without `--json` or `--events`, every command
   prints readable terminal text. There is no TTY detection (output does not change
   when piped) and no colour in R0.
3. **`DOWNLOAD_FAILED`.** A new failure code for a managed download that fails, with a
   typed reason: `tls`, `redirect_policy`, `http_status`, `proxy_auth`, `offline` or
   `size`. It is in exit class 7.
4. **Pre-release in-place contract edits.** Before the first publication, a v1 value
   that has never shipped may be renamed or extended in place rather than versioned;
   each edit is listed below and in an ADR 0008 note.
5. **The launcher checks the binary digest** against `platform-digests.json` only if
   that adds less than 50 ms to a run; it always checks the version.
6. **Runtimes:** Node.js 22 or later and Bun 1.2 or later.
7. **The skill ships inside the npm package** and in every archive, byte-identical to
   `skills/vsift/`.
8. **R-13 joins P13** in the ledger and traceability, because `handoff check` serves
   the agent skill and handoff requirement.
9. **Power-loss qualification** of the managed store reuses the P10 crash campaign on
   a disposable VM. It claims fail-closed detection plus repair, not that every
   transaction survives power loss.
10. **No named-agent re-run from a clean install in P13.** That run is P14's release
    checkpoint.

## What P13 delivers

### 1. Native artifacts and the release pipeline

- `.github/workflows/release.yml` builds the three targets of decision D and packages
  each as an archive with the licences, `THIRD-PARTY-NOTICES` (from `cargo-about`),
  a CycloneDX SBOM for that target, and the skill. `SHA256SUMS` covers every archive.
- A reproducibility check builds each target twice on the same runner and compares
  the bytes. `vsift --version` gains the source commit as a suffix.
- An attest job creates Sigstore build-provenance attestations. A publish job runs in
  the protected GitHub environment `release`, whose reviewer is the maintainer, and
  publishes to npm through trusted publishing (OIDC). The workflow defaults to
  `dry_run`, which builds and checks everything but writes nothing outside the run:
  no attestation, release or package.
- **Governance workflow lint (R-SEC01):** the governance checker fails a workflow that
  uses an action not pinned to a full commit SHA, uses `pull_request_target`, lacks
  minimal `permissions`, or grants `id-token: write` outside the attest and publish
  jobs.

### 2. The npm launcher

- The `vsift` package is a plain CommonJS launcher, `bin/vsift.cjs`, using only `node:`
  built-ins that Node.js and Bun share. No package has an install script.
- The platform packages declare `os` and `cpu` and are the launcher's exact-version
  `optionalDependencies`. The launcher finds the matching one, checks its version and
  (decision H5) its digest, and runs the binary with an explicit executable and
  argument list, never a shell. It forwards signals and propagates the exit status. A
  missing, mismatched or unsupported platform gives a readable message naming the
  expected package and the supported targets.
- Qualification runs on a local Verdaccio registry with npm, pnpm, Yarn and Bun, both
  global and one-shot (`npx`, `pnpm dlx`, `yarn dlx`, `bunx`), with scripts disabled and
  with optional dependencies omitted, on every target.

### 3. Managed installation (Ubuntu 24.04 x64)

In the order recorded when P06 was parked (ADR 0015; the
[2026-09-09 to 23 delivery log](../history/2026-09-09-to-23-delivery-log.md)):

1. a production smoke executor on the existing `ProcessSupervisor`, enforcing the
   catalogue's digest-bound stream, generated-file and deadline limits;
2. proven failure cleanup before any activation;
3. a guarded per-component transaction: download from the publisher (or import with
   `--artifact-dir`), stage, smoke, then `publish_and_select`;
4. the managed tier in provider lookup, so precedence is per-call explicit path,
   configured path, managed version, then filtered `PATH` (ADR 0007's order);
5. `setup install`, `setup list`, `setup rollback`, `setup remove` and `setup repair`.
   `repair` is read-only: it diagnoses and emits a plan, so it adds no second way to
   change the store;
6. bounded cleanup that keeps the current and one previous version, and a sweep of
   stale stages left by an interrupted run;
7. kill and power-loss qualification (decision H9), D-02..D-08 and the P13 stage of
   the E2E spine.

*Implementation note, 2026-09-30 (P13 PR 3, steps 1 and 2):* the smoke executor and
failure cleanup exist as an internal capability; `setup install` still answers
`COMMAND_NOT_IMPLEMENTED` until PR 4, and nothing is published or selected.

- **Application ports** (`vsift-application`, `provisioning.rs`):
  `StagedManagedComponent` (a staged, unactivated component that can only say which
  component it is and be discarded) and `CompatibilitySmoke<C>`. The use case
  `smoke_before_activation` returns `Passed(candidates)`, still unactivated, or
  `Failed { failure, stages }`, having handed every candidate to cleanup: a failed
  smoke cannot say which of several candidates is at fault, and none may be
  published without a pass. A failure is a step (`layout`, `banner`,
  `media_fixture`, `speech_fixture`, `recheck`) and a closed reason
  (`missing_executable`, `wrong_architecture`, `not_executable`, `banner_mismatch`,
  `output_over_bound`, `deadline_exceeded`, `unexpected_extra_file`,
  `changed_content`, `provider_failed`, `fixture_mismatch`, `preparation`,
  `invalid_request`, `cancelled`). A disposal is `Discarded` or `Retained` with
  `ownership_unproved`, `unexpected_content` or `storage_failure`. None of these
  values is public contract yet; PR 4 decides what `setup install` reports.
- **Infrastructure** (`managed_smoke.rs`, `managed_artifact_store.rs`):
  `StagedManagedCandidate` owns one stage with its payload and runtime;
  `ReviewedUbuntuAction::stage_candidate` builds it from the accepted action and
  names its smoke roles (`ffmpeg`, `ffprobe`, `whisper-cli`, the model file).
  `StagedCompatibilitySmoke` runs, in order: the layout recheck (exact names, bytes,
  modes; each executable reviewed as executable and in the host's native format,
  read from its ELF, PE or Mach-O header); `ffmpeg -version` and `ffprobe -version`
  against the policy's banner prefixes and `whisper-cli --help` for a clean start,
  each through the `ProcessSupervisor` with the policy's stream bound and media
  deadline; the existing `FixtureMediaToolVerifier` within the media deadline; the
  existing `FixtureAsrVerifier` within the inference deadline, with the transcript
  file bounded by the policy (new `WhisperCli::with_output_limits`, which can only
  lower the R0 bounds); then removal of the smoke directory, which must be empty, and
  a second layout recheck, so a provider that wrote into its own installation fails.
  Executables run by explicit path from `runtime.pending`, with their working
  directory in a private `smoke.pending` directory in the same stage; there is no
  shell. A component that needs another uses the staged one, or an already selected
  provider passed as `SmokeCompanions` (not smoked again).
- **Cleanup** removes only positively identified content: it first proves the root
  and stage markers and each held directory's identity (otherwise nothing is touched,
  `ownership_unproved`); then removes only reviewed runtime names, an empty smoke
  directory, the selected payload names, the verified artifact and the stage marker,
  each a single-link regular file or the same held private directory. Anything else
  stops cleanup and is kept (`unexpected_content`).
- **Reading recorded here:** the policy's media deadline bounds the whole media
  fixture check, which is stricter than bounding each of its provider runs; the
  policy's stream bound applies to the banner runs, while the fixture operations keep
  their adapters' fixed bounds (the generated audio is bounded by the policy in the
  media verifier, and the transcript file now by the policy too).

*Implementation note, 2026-09-30 (P13 PR 4, steps 3 and 4):* `setup install` runs the
guarded transaction, and every command resolves tools through the managed tier. `setup
list/rollback/remove/repair`, bounded cleanup and the stale-stage sweep stay PR 6; kill
and power-loss tests PR 7.

- **Order.** `setup install --plan <file> --accept-plan <digest> [--artifact-dir
  <absolute folder>]` reads the strict saved plan, then (engine `install_setup`): refuses
  a relative artifact folder; answers `INVALID_ARGUMENT` with the manual path on a host
  whose target has no qualified catalogue, creating nothing; takes the root's install
  guard, which never waits (a held guard is `BUSY`, `retry_after_ms` 30,000; a root that
  cannot be created or proved private is `STORAGE_IO` with the manual path, D-09 and
  D-10); rebuilds the plan and requires it to equal the saved one and the digest to
  accept it; only then installs.
- **Transaction** (`vsift-application/src/install.rs`, `install_managed_components`, over
  the port `ManagedComponentInstaller`). Components go in plan order: media tools, then
  the whisper.cpp CLI, then the model. A component whose reviewed version is already
  selected is `already_current` and is not fetched. The others are fetched, staged with
  their runtime prepared, smoked with PR 3's `smoke_before_activation`, and published
  and selected one by one (`StagedManagedCandidate::publish_and_select`, which then
  removes the rest of the stage), so each activates atomically on its own. The CLI and
  its model are staged and smoked together when both are pending (neither can be smoked
  without the other: "the model is smoked with whisper"); any other component is smoked
  with the providers selected at that moment as companions, resolved in the lookup
  order below. The first failure stops the transaction; the components after it are
  `failed` with reason `blocked` and were never fetched, and a candidate staged with the
  failed one is discarded. A rerun of the same accepted command continues from the
  first component not yet current.
- **Decided here: a plan's intent and its observed state.** The digest binds the plan's
  intent: target, catalogue, every action's reviewed artifact, each dependency's and the
  model's disposition, and the observations of the tools outside VSift's managed store
  (explicit, configured and `PATH`) that decide which components are needed. Installing
  a managed component changes none of that. Beside it, `setup plan` shows the observed
  state, which is what commands would use now with the managed tier included:
  `readiness`, each dependency's `status`, the model's `status` (`managed_current` once
  the managed model is selected), each action's `state` (`pending` or `current`) and
  `install_needed` (false when every action is current). Acceptance compares the intent
  only (`SetupPlanResponse::require_same_plan`, and `install_setup` rebuilds the intent
  without opening the managed store), so the plan accepted before installing stays
  accepted after a partial or complete install and a rerun continues, while after a
  complete install `setup plan` reports every component current, readiness no longer
  blocked on them and nothing to install. Any change to the intent in the saved file is
  still refused. This replaces the first reading of this PR, a plan that did not see the
  managed tier at all, which was never released.
- **Network guard for tests (decided here).** A development build, which every test run
  without `--release` is, resolves no host name for a publisher download: the transfer
  client gets a resolver that refuses every name, so a reviewed publisher route fails
  as `offline` before any connection and nothing is staged. The loopback routes and
  test proxies are `127.0.0.1` literals, which are never resolved, so the D-03 and D-07
  tests are unaffected. The opt-in real-tool checkpoints run `--release`; a developer
  who wants a debug build to download sets `VSIFT_DEV_PUBLISHER_NETWORK=allow`. Release
  builds do not compile the guard. This makes the CI incident of this PR (a non-ignored
  test that accepted a real plan digest and started a real download) impossible
  rather than merely avoided.
- **Transport** (`publisher_artifact_transfer.rs`). One `GET` per attempt with no
  `Range` header; only a complete `200 OK` identity body is accepted, so `206` is
  refused; every byte goes through the exact size and SHA-256 check into a private
  stage that any failure, cancellation included, discards. `DOWNLOAD_FAILED` (exit 7)
  carries one reason: `tls` (a `native_tls::Error` in the error chain), `redirect_policy`
  (outside the reviewed route, another host, userinfo, more than three), `http_status`
  (any status but `200`, a `Content-Range`, a non-identity encoding), `proxy_auth` (a
  `407` response, or hyper-util's private tunnel error, recognised by its text because
  the type is not exported; the D-07 test fails if an update changes it), `offline`
  (connect, DNS, a dropped or stalled body, the transfer deadline) or `size` (a
  declared length or a body that differs from the review). Bytes of the right size and
  the wrong digest are `INTEGRITY_FAILURE`. The client keeps the system proxy settings
  and the neutral user agent `VSift/0.1 managed setup`; no error carries a URL, header or
  credential.
- **Offline import (D-07).** `--artifact-dir` reads, for each action, the file named by
  the last segment of its reviewed URL, as a regular file (no link followed), checks
  its size, then streams it through the same verifier into a fresh stage. The folder is
  the only user input; no URL, checksum or file name is read from the user. A missing
  or non-regular file is `INVALID_ARGUMENT` (`artifact_missing`,
  `artifact_not_regular_file`); a wrong size or digest `INTEGRITY_FAILURE`
  (`size_mismatch`, `digest_mismatch`).
- **What `setup install` reports of the smoke's typed failures (decided here).** A smoke
  that fails is `MISSING_CAPABILITY` (the reviewed tools do not work on this machine),
  except `cancelled` (`CANCELLED`) and `preparation` (`STORAGE_IO`, VSift's own
  directory); the data names the component, `step: smoke`, the smoke's `smoke_check`
  and its reason. A staged artifact whose contents or layout differ from the review, and
  a published version that already names other bytes, are `INTEGRITY_FAILURE`
  (`review_mismatch`); storage is `STORAGE_IO` (`storage`); a cancellation `CANCELLED`.
- **Result.** `data` lists every component with `status` (`activated`,
  `already_current`, `failed`), and for a failure its `step`, `reason`, `failure_code`
  and `smoke_check`, plus what cleanup did with its stage (`discarded`, or `retained`
  with its retention reason). A failed transaction is a failure result whose error is
  the first failure's code and fixed-prose remediation, with the same `data` beside it
  (the envelope's `with_failure_value`), so a caller always sees what is installed.
  `--events jsonl` adds `progress` events: `fetching_artifact` in bytes and
  `installing_components` in components. The command is long-running: Ctrl-C cancels it,
  discards the stage in progress and keeps what is already active.
- **Managed tier in lookup (step 4, ADR 0007's order).** Every command that runs a tool
  resolves it as: a per-call path (only `setup check` takes one), the configured path,
  the managed version `setup install` selected, then the filtered `PATH`; the model:
  configured, then managed. A managed version is opened only when its pointer names the
  manifest's SHA-256 and every file matches the manifest by size and SHA-256 (hashed
  once per open, after the shared use lock is held); one that does not is never run,
  and lookup falls through to `PATH`. Managed tools are thus identified by digest, which
  closes L-006 for them. The executable resolved from a version carries a
  `ManagedRuntimeHold` (the recognizer also its model's), so a job holds the version's
  shared use lock for as long as it holds the tool, and an update never removes a
  version in use. The engine takes the root from `EngineConfig::managed_root`
  (`ManagedRootLocation`, the platform default for the CLI).
- **Development-only test hook.** The transfer and transaction tests drive the real
  transport against local servers through the `install-test-hooks` feature of
  `vsift-infrastructure` (a loopback route on `127.0.0.1`, an explicit test proxy and an
  injected stage-write failure for the disk-full case). Like `fault-injection`, the crate
  refuses to compile it without debug assertions and the governance check refuses it
  anywhere but a development dependency.

### 4. Human-readable output

A renderer per command under `crates/vsift-cli/src/human/`, writing through one
`TerminalText` builder that escapes control characters and hidden characters with
`display_text`'s rule (`vsift_contract::is_hidden_character`, ADR 0008 note of
2026-09-29), so no evidence text reaches a terminal raw. The SEC-T02 suite is re-run
over human output. This closes L-017 and L-073 and the display part of L-016.

*Implementation note, 2026-09-30 (P13 PR 2a):* the builder and the first renderers are
in `crates/vsift-cli/src/human/`. `TerminalText` checks every character it is given:
a control character other than its own line breaks becomes U+FFFD, a hidden character
becomes `<U+XXXX>` (through `vsift_contract::terminal_safe_text`), identifiers are
never cut, a line of untrusted text longer than 4,000 bytes continues on the next
quoted line, and a result over the 1 MiB budget is refused whole. Untrusted text has
one entry, `push_untrusted`, which takes only a `DisplayText`: read from a result's
`display_text` or `display_label`, or rendered with the same rule from text that has no
display field (the parser's explanation, a probed version line). Renderers read a
result through typed views of its published JSON (the contract's data types keep their
fields private), and no view has a field for `text`, `original_text`, a speaker's
`label` or the query's terms, so quoting raw evidence cannot be written by mistake.
`setup check` renders from its typed report. Every failure renders on stderr as its
message and code, each remediation's summary (`Fix:`) and suggested command (`Run:`),
the affected identifiers and the retry hint; in human mode a rejected command line now
also shows PR 1's remediation, after the parser's explanation quoted line by line.
PR 2a covers setup (check, plan, configure, configure-model), `ingest`, the `session`
commands, `transcript get/retranscribe`, `search`, `bundle validate`, every failure and
rejected command lines; `candidates`, the frame commands, `crop`, `audio`, the `job`
commands and the worker hosts keep the indented JSON result until PR 2b, which also
decides the display of `files[].path` (L-016). `cli-v1.md` declares human text unstable
and not for parsing. SEC-T02's human-output rerun for these commands is
`crates/vsift-cli/tests/sec_t02_human_output.rs`, with a property test of the builder
and golden snapshots (for review, not a contract) in
`crates/vsift-cli/tests/human_output/`; L-017 and L-073 stay open for PR 2b's commands.

*Implementation note, 2026-09-30 (P13 PR 2b):* the remaining commands render through
the same builder: `candidates`, `frame get/neighbours/burst`, `crop`, `audio`, `job
status/resume/cancel` and the worker hosts `job run` and `job batch` (modules
`human/evidence.rs` and `human/job.rs`, typed views in `human/view.rs`). Every command
that completes now has a renderer; one without is a defect that fails `INTERNAL`, so
human mode never prints the JSON document. Decisions taken in this PR, for the
maintainer's review:

- **Paths (L-016's display).** The builder gains one entry, `push_path_line`: a
  delivered `files[].path` is written whole on a line of its own (after four spaces)
  under its item and `File (<media type>):` label, never cut, with the same character
  rules as any value; it reports whether the line is the path exactly. A path in the
  extended-length form (`\\?\`) is followed once by a note that some programs refuse
  it, that PowerShell's `Copy-Item -LiteralPath '<path>' <destination>` copies the file
  out, and that a `--session-root` of at most 125 characters gives plain paths. A path
  holding a control or hidden character (the session root is the user's choice), or
  longer than 4,000 bytes (replaced by a statement), is flagged once with `--json`
  named for the exact text. The JSON is unchanged. L-016 keeps only its residual.
- **Worker hosts keep JSON Lines for their stream.** Human mode renders the final
  response only, as it wrote only the final response before: `job run`'s job result
  (steps, typed outputs, uncovered ranges, failures, controls) and `job batch`'s
  summary. A `failed` or `cancelled` request or batch writes that text to stdout and
  its error to stderr in the failure form; exit statuses are unchanged. The
  `progress`, `lifecycle` and `result` events are not rendered: they are a supervisor's
  interface (ADR 0021: sequence numbers, never-dropped lifecycle and result events,
  64 KiB lines), consumed by machines, and a second, unstable human form of the stream
  would be a protocol without a user. `cli-v1.md` says so; L-017 is rewritten as that
  accepted residual (no progress in human mode).
- **SEC-T02 on every human output (closes L-073).** The results PR 2b renders carry no
  evidence text; their one untrusted text is a delivered path. The rerun covers it
  through the binary under a session root holding a right-to-left override and a
  zero-width space, plus off Windows an OSC-8 link, an ANSI colour, a line break and a
  C1 control (`evidence_cli_contract`, which seeds evidence without media tools, for
  `frame get`, `crop` and `audio`, and the Windows extended-length form); in the
  renderers' unit tests for every frame command with hostile, extended and over-long
  paths; in `sec_t02_human_output.rs` for every PR 2b command failing under that root;
  and in `job_run_cli_contract` and `job_batch_cli_contract` for the worker hosts with
  hostile request text. A second builder property test shows a path of any content is
  one safe line. Golden snapshots of every frozen example are in
  `crates/vsift-cli/tests/human_output/`.

### 5. Parse remediation (L-071)

A command line that fails to parse gets a typed remediation (what kind of mistake,
and which defined argument when the parser knows it) that never echoes the text the
user supplied.

*Implementation note, 2026-09-30 (P13 PR 1):* the parser's error kind maps to a closed
set of nine reasons: `unknown_argument`, `missing_required`, `invalid_value`,
`unexpected_value`, `argument_conflict`, `missing_subcommand`, `unknown_subcommand`,
`invalid_utf8` and `unclassified` (a kind VSift does not classify, so a parser upgrade
cannot add an unlisted value; no command line of today's grammar produces it). In
`--json` and `--events jsonl` modes the `parse` failure carries one remediation: the
summary `The command line was rejected (<reason>). <what is wrong>; read its help.`,
naming the deepest command reached and the blamed argument only when the parser's
report resolves to the grammar's own definitions; a PowerShell quoting note when that
command takes a comma-separated value (`crop --rect`); and the suggested `command`
`vsift <command words> --help`, `required_authority` `none`. `--json` together with
`--events` is `argument_conflict` too. The reason sits in the summary in the form the
search query's rejection already uses, so the envelope and schemas are unchanged and
no additive field (or ADR 0008 note under decision H4) was needed; a structured reason
field stays possible later as an additive change. Human mode keeps the parser's own
explanation on stderr, now through `vsift_contract::terminal_safe_text`, which
replaces controls and writes hidden characters as `<U+XXXX>` (`display_text`'s rule).
The contract is in `docs/contracts/cli-v1.md` ("Rejected command lines"), the frozen
example is `schemas/v1/examples/parse-failure.json`, and L-071 is closed.

### 6. `vsift handoff check` (#213)

- The schema and rule checks of the grader's handoff module move into production as
  `vsift-contract::handoff`; the grader keeps only its grading policy and calls them.
  `jsonschema` becomes a production dependency after the dependency review.
- Input per decision G, bounded to 64 KiB of UTF-8. Output per decision H1.
- `--session <id>` optionally resolves the draft's cited identities against that
  session's records, through the engine.
- The skill's command class is `free`.

### 7. Documentation

This ADR; notes in ADRs 0007, 0008, 0009, 0014 and 0022; `docs/contracts/cli-v1.md`;
verification, the threat model and known limits; `docs/operations/install.md` and
`docs/operations/release.md`; the qualification record
`docs/planning/p13-distribution.md`; and the P13 stage of the E2E spine.

## Planned contract changes

All within v1, before the first publication (decision H4):

| Change | Kind | Pull request |
| --- | --- | --- |
| Setup-check `lookup` gains `managed_version` | in-place enum extension | 4 |
| Setup-check remediation `managed_install` takes the plan's availability values instead of the constant `unavailable_unqualified` | in-place widening | 4 |
| Setup-plan availability `catalogue_accepted_install_pending` becomes `catalogue_accepted` | in-place rename | 4 |
| `DOWNLOAD_FAILED` with its reasons, exit 7; exit 7's category widens from storage and output I/O to include a managed download | new failure code | 4 |
| `setup-install`, `setup-list`, `setup-remove`, `setup-rollback` and `setup-repair` response schemas | new schemas | 4, 6 |
| The `handoff` namespace, `handoff check` and its response schema | new namespace and command | 5 |
| Readable terminal text as the default presentation | presentation (JSON unchanged) | 2a, 2b |
| Typed parse remediation | additive remediation | 1 |

Each change updates `CommandName`, `FailureCode::ALL`, the v1 schemas and examples,
`cli-v1.md` and the skill (the `skill_contract` guard fails otherwise). Once the first
artifact is published, v1 changes are additive only.

## Not in P13

P14's release qualification; managed installation on Windows and macOS; native
installers (winget, Scoop, Homebrew, Debian packages); crates.io publication; the MCP
adapter; SEC-T01's adversarial evidence (L-068); and the P12 debt #218-#224, fixed on
its own branch.

ADR 0001's "appropriate native installation methods" are, for R0, the archives on
GitHub Releases; native installers are later work.

## Maintainer-only actions

Never automated, and never done by an agent: the npm account and its two-factor
authentication; choosing the platform-package scope and creating its organisation;
configuring the trusted publishers;
the GitHub `release` environment and its reviewer; the tag ruleset; approving fork
pull-request workflows; the first publish (done personally with two-factor
authentication, or with a short-lived token stored only in the `release`
environment); and any announcement.

## Consequences

- R-14 becomes testable: an install without Rust through four package managers, and
  provenance anyone can verify. R-03's managed path exists on one target.
- The CLI surface grows by six commands and a namespace; the skill, its guard and the
  grader change with them. `handoff check`'s input exception must stay the only pipe
  or redirect the skill teaches.
- The Linux binary needs glibc 2.35 or later and OpenSSL 3 (`libssl.so.3`), because
  the transport uses the system TLS library; Ubuntu 22.04 and 24.04 have both.
- Unsigned Windows and macOS binaries trigger SmartScreen and Gatekeeper prompts when
  downloaded directly; users who want no prompt install through npm.
- A contract value that ships in the pre-release can no longer be renamed in place.
- The E2E spine's distribution checkpoint (an agent run from a clean install) moves
  to P14 (decision H10); P13's stage is mechanical: a clean install without Rust and
  one managed dependency install.

## Details left to their pull requests

These are implementation choices inside the decisions above, recorded here so that
the pull request that makes each one says so:

- whether `handoff check` reads the fenced block alone or the whole report around it
  (the grader checks both), and whether it reads a closed value in another letter case
  as the grader does (ADR 0022 decision 6) or refuses it and names the schema's
  spelling;
- how the contract crate embeds the skill-owned `handoff.schema.json` while the skill
  keeps owning it (ADR 0022 §6);
- whether the read-only `setup list` and `setup repair` move from the skill's `never`
  class to `free`; until the maintainer says so they stay `never`;
- a typed dependency selector for `setup remove` and `setup rollback`, which parse it
  as a string today.

## Implementation note, 2026-09-30 (P13 PR 5, `handoff check`, #213)

Delivered as section 6 says, with the details left to this pull request decided by
the supervisor on 2026-09-30:

- **The whole report is checked**, not only the block: the one `vsift-handoff` block
  (bounded, strict JSON), then the whole text for paths, links and raw hidden or
  control characters.
- **Letter case:** a closed value in another letter case is read as the schema's
  spelling and noted in `case_notes`, as the grader has done since P12 (ADR 0022
  decision 6); any other word is refused with the allowed values.
- **Where the check lives:** `vsift-contract::handoff` (`HandoffChecker`,
  `HandoffCheckData`): extraction, letter case, the schema, the handoff rules, the
  report-text rules and the session resolution. The trial grader calls it; the engine
  reads a session's identities for `--session` read-only (`Engine::
  handoff_session_records`, which never renews), and the CLI composes them.
- **The schema stays the skill's.** The contract embeds
  `skills/vsift/handoff.schema.json` with `include_str!`; a test requires the
  embedded copy to equal the file. Because it names a file outside the crate, the
  contract crate cannot be packaged for crates.io as it is; R0 publishes no crate
  (decision B), and known limit
  [L-086](../planning/known-limits.md#l-086) tracks the fix needed before any crates.io
  publication.
- **Schema validation without `jsonschema`** (maintainer decision of 2026-09-30,
  option 2 of three). Making `jsonschema` 0.56 a production dependency was measured
  first: it added 43 crates to the release binary's graph (164 to 207, including two
  regular-expression engines, a big-number stack, `ahash`, `parking_lot`, `zerocopy`,
  `uuid-simd` and a second `getrandom`) and grew the Windows release `vsift.exe` from
  6,554,112 to 12,078,080 bytes. Instead the contract has a validator of exactly the
  JSON Schema features the handoff schema uses; compiling the schema refuses any other
  keyword or form, so the skill cannot start using a feature the check would ignore.
  Its verdicts are held to `jsonschema` (a development dependency only) over the
  skill's examples, the REPORT skeleton, over 2,000 systematic mutations and 512
  generated drafts, with error locations compared pointer by pointer
  (`crates/vsift-contract/tests/handoff_differential.rs`). ECMA-262's `\s` is written
  out as the class `jsonschema` uses, so the two agree on every space character.
- **Dependency review: `regex`** (the schema's 17 patterns). Promoted from a
  transitive development dependency to a production dependency of `vsift-contract`,
  with `default-features = false, features = ["std"]` (no Unicode tables: the
  patterns use explicit ranges, and `\d`, `\w`, `\b` and `.` are refused). Licence
  MIT OR Apache-2.0; maintained by the Rust project (rust-lang/regex); already in the
  lockfile and reviewed by `cargo deny` with development dependencies included. It
  adds `regex`, `regex-automata` and `regex-syntax` to the release graph (164 to 167
  crates; without the `perf` feature `aho-corasick` and `memchr`'s use are not
  needed). With the whole check (validator, rules, embedded schema and `regex`), the
  Windows release `vsift.exe` measured 7,400,960 bytes, against 6,554,112 before
  (+846,848, 13%), and 12,078,080 with `jsonschema`.
- **Contract:** the `handoff` namespace (ADR 0008 note of 2026-09-30),
  `schemas/v1/handoff-check-data.schema.json`, `schemas/v1/examples/handoff-check.json`,
  39 closed rules, findings bounded to 100 per list with `truncated`, and a
  `session` object whose `gap` (`session_closed`, `session_expired`,
  `session_not_found`) replaces a failure.
- **Skill, guard and grader:** ADR 0022 note of 2026-09-30 ("`handoff check` in the
  skill"). The skill's forms carry no `--session`, so they stay exactly literal. Each
  form belongs to one shell: the quoted heredoc to POSIX shells (bash, sh, Git Bash on
  Windows) and the single-quoted here-string to PowerShell. The grader recognises each
  only in its own dialect (review of PR 5): in bash `@'...'@` is not a here-string, the
  draft's first apostrophe would end the quoting and the rest would run as commands, so
  that script is read as ordinary shell text and stays strict.
- **Fuzzing:** the `handoff_check` target (the 24th) runs the whole check over a
  draft and requires it to be deterministic, bounded, its verdict its errors, and
  every published pointer and allowed value within the grammar that keeps draft text
  out; its seeds copy `SKILL.md` and the frozen example's draft.
- R-13 is mapped to P13. The compact tier's re-run (#222) follows this pull request.
