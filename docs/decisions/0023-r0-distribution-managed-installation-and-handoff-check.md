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

*Amendment, 2026-09-30 (maintainer, later the same day): the launcher package is
`vsift-cli`.* npm refused the unscoped `vsift` at publish ("Package name too similar to
existing packages sift, tsify", E403 on the maintainer's placeholder). The maintainer holds
`vsift-cli` with a placeholder `0.0.0` published on 2026-09-30. The installed command stays
`vsift` (the package's `bin` key), and the platform packages stay in the `@vsift` scope. Read
"the unscoped `vsift` package" and "the `vsift` package" in this ADR as `vsift-cli`; the
reasons and the lesson (a not-found lookup is not availability) are in the ADR 0009 note of
the same date. This supersedes the unscoped launcher name of this decision.

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

*Note, 2026-09-30 (maintainer), with the amendment of decision A:* npm refused the `vsift`
placeholder, which was never published. The placeholder exception applies to `vsift-cli`:
the maintainer published `vsift-cli@0.0.0` (README and `package.json` only), which stays the
`latest` dist-tag until a stable release, while the 0.x pre-release goes under `next` and is
installed as `vsift-cli@next`. Nothing else is published before P13 completes.
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
  class to `free`; until the maintainer says so they stay `never` (*decided in PR 6:*
  `free`, see its note);
- a typed dependency selector for `setup remove` and `setup rollback`, which parse it
  as a string today (*decided in PR 6:* the component identifiers of the JSON results
  and a canonical `--version` key, see its note).

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

## Implementation note, 2026-09-30 (P13 PR 8, `release.yml` and the governance workflow lint)

Delivered from section 1: the release workflow up to packaging, the reproducibility
check, the commit suffix of `--version` and the governance workflow lint. The
attestation and publish jobs, the protected `release` environment and the `dry_run`
input are PR 10's; the npm packages are PR 9's. The runbook is
[`docs/operations/release.md`](../operations/release.md).

- **`release.yml`** runs on pull requests and pushes to `main` that touch an archive
  input (`crates/`, `skills/vsift/`, the licences, the manifests, the toolchain, the
  packaging tool, the workflow) and on manual dispatch; never on a tag or a release. It
  has `permissions: {}` and `contents: read` per job, no `id-token`, no secret and no
  cache (a cache written by one run could feed a later release build). Until PR 10
  every run is the dry run of section 1: the archives and `SHA256SUMS` stay the run's
  artifacts for 7 days, unsigned and unattested.
- **Build.** `cargo build --release --locked -p vsift-cli --bin vsift --target
  <target>` on `windows-2025` (static C runtime), `macos-15` and `ubuntu-22.04`; no
  feature is ever selected. Each target is built twice on its runner, with the release
  output removed in between, and the executables must be identical. Windows needs
  `-C link-arg=/Brepro`: measured on 2026-09-30, two static-CRT builds of one commit
  differed in the PE time stamp, the debug directory's time stamps and the PDB identity
  without it, and were identical with it (7,884,800 bytes).
- **`--version` names the commit.** A build with `VSIFT_SOURCE_COMMIT` set prints
  `vsift <version> (<first 12 digits of the commit>)`, the form `rustc --version`
  uses; the workflow sets it from the checked-out commit and requires that line. A
  build without it prints `vsift <version>` as before; a value that is not a full
  40-digit lowercase SHA fails the build (`crates/vsift-cli/build.rs`,
  `src/build_identity.rs`).
- **Notices and SBOM.** cargo-about 0.9.2 (`--offline`, the licences of `deny.toml`,
  build and development dependencies excluded: they are not in the binary) writes
  `THIRD-PARTY-NOTICES` from `tools/vsift-release/notices/`; cargo-cyclonedx 0.5.9
  writes a CycloneDX 1.5 SBOM of `vsift-cli`'s graph for the target (the tool's default,
  which keeps build-time dependencies), with `SOURCE_DATE_EPOCH` set to the commit time. Both tools are installed
  with `cargo install --locked` at an exact version.
- **Archives.** `tools/vsift-release` (never shipped; no new dependency) packages
  `vsift-<version>-<target>.tar.gz` holding `vsift` or `vsift.exe`, `LICENSE`,
  `LICENSE-APACHE`, `LICENSE-MIT`, `THIRD-PARTY-NOTICES`, `vsift.cdx.json` and
  `skills/vsift/` byte for byte (decision H7). It refuses an executable of another name,
  format or architecture, so no test binary can be packaged; the archive is
  deterministic (path order, the commit time, owner 0, fixed modes, a gzip header
  without name or time). The workflow packages twice and compares, reads each archive
  back against its inputs, and checks `SHA256SUMS` with `sha256sum --check --strict`.
- **Governance workflow lint (R-SEC01).** `vsift-governance` now parses every workflow
  (new development-tool dependency `yaml-rust2` 0.13, MIT OR Apache-2.0, pure Rust,
  with `arraydeque` and `hashlink`; never in a shipped crate) and fails, as this section
  specifies, an action not pinned to a full commit SHA, `pull_request_target`, missing or
  non-read-only top-level `permissions` (and `read-all`/`write-all` anywhere), and
  `id-token: write` outside the release workflow's `attest` and `publish` jobs. Two
  rules are added: a `run` script may interpolate only `runner`, `matrix`, `strategy`,
  `job` and nine fixed `github` fields, everything else goes through `env`; and in
  `release.yml` the build command above is the only `cargo build`, no cargo command but
  a pinned `cargo install` selects features, packages or profiles, and no key or value
  names a development feature (`fault-injection`, `durability-campaign`,
  `install-test-hooks`), the smoke-test stand-in, the crash-campaign tool, a
  `CARGO_PROFILE_` override or `debug-assertions`. The lint reads the YAML tree, so a
  flow mapping or an alias cannot hide a grant, and it refuses merge keys. No existing
  workflow violated a rule.
- **For the maintainer's review:** `.tar.gz` for Windows too (no `zip` dependency;
  Windows 11 opens it); the twelve-digit commit form; the two added lint rules; the
  path filter on the pull-request trigger; the SBOM's `bom-ref` values carrying the
  runner's checkout path ([L-089](../planning/known-limits.md#l-089)).

## Implementation note, 2026-09-30 (P13 PR 6, managed lifecycle, steps 5 and 6)

`setup list`, `setup rollback`, `setup remove` and `setup repair` are implemented, with
bounded version cleanup and the stale-stage sweep. Kill and power-loss tests stay PR 7.
The rules live in `vsift-application/src/managed_lifecycle.rs` (pure policies over the
ports `ManagedStoreReader` and `ManagedStoreMaintenance`); the store's inspection, sweep
and deselection in `vsift-infrastructure/src/managed_store_lifecycle.rs`; the engine
operations in `vsift/src/lifecycle.rs`.

- **Grammar (the typed selector left to this pull request).** `setup list`; `setup repair`
  (its reserved `--profile` is dropped: the managed store is not per profile);
  `setup rollback <component> [--version <version>]`; `setup remove <component>
  [--version <version>]` or `setup remove --stale-stages`. `<component>` is one of the
  identifiers the JSON results use (`ffmpeg_ffprobe`, `whisper_cli`, `whisper_model`);
  `--version` must be a canonical managed key (`vsift_domain::ManagedVersionKey`: 1 to 64
  lowercase ASCII letters, digits, `.`, `_`, `-`, beginning and ending alphanumeric), so
  any other text is a parse failure and never reaches the store or any output.
- **Repair is read-only, as §3 step 5 and ADR 0007 say.** It diagnoses and emits a plan:
  each finding has a `kind`, a `fix` and the existing command that applies it (`setup
  rollback`, `setup rollback --version`, `setup remove --version`, `setup remove
  <component>` then a new plan and `setup install`, `setup remove --stale-stages`), or
  `manual` for content VSift cannot prove its own. It never creates the managed folder,
  takes no lock beyond a version's shared use lock while hashing it, never selects,
  removes or downloads, and a store that needs repair is a complete result (exit 0).
  Repair adds no second way to change the store: every fix is one of the commands above.
- **Skill classes (decided by the supervisor, 2026-09-30).** `setup list` and `setup
  repair` are `free`: both only read (list takes no guard, repair changes nothing, as a
  byte snapshot of the store in the tests shows). `setup rollback` and `setup remove` stay
  `never` with `setup install`: they change which tools every command uses. The skill tells
  the agent to relay repair's commands to the user, never to run them.
- **Selection history.** The selection pointer (now `VSIFT-MANAGED-POINTER-v2`; v1 is
  still read) records the version selected before and its manifest SHA-256 beside the
  selection, so selection and history change in one atomic rename. A rollback without
  `--version` selects that previous version only if it verifies and its manifest is the
  one recorded; with `--version` the version must verify against its own manifest. A
  version that does not verify, or has no manifest, is never selected. A second rollback
  returns, because the replaced version becomes the new previous one.
- **Removal.** `setup remove <component>` removes the selection pointer first (commands
  stop using the component at once), then every version; `--version` refuses the selected
  version. A version a job holds (its shared use lock) is kept (`in_use`) and the result
  is `BUSY` (retry 30 s) with every item as data, like a failed install; a rerun finishes.
  *Decided here:* removal proves **ownership, not integrity**: every entry must be a name
  the manifest or the version's metadata gives and a single-link regular file, while a
  file's bytes or mode may differ, so a corrupted version can be removed. Before PR 6 the
  primitive also required the bytes, which left a corrupted version impossible to remove
  or reinstall (the reinstall's publication meets the same identity with other bytes). A
  link, a folder, an unknown name or an unreadable manifest still stops removal
  (`unexpected_content`, `STORAGE_IO`), and repair names it for the user (L-090).
- **Bounded cleanup and the sweep.** Once a `setup install` plan is accepted, under the
  guard and before anything is staged, the sweep removes the stages earlier runs
  abandoned (a killed run's, and those PR 3 and PR 4 cleanup kept); after the
  transaction, bounded cleanup keeps each component's selected and previous version
  (`RETAINED_MANAGED_VERSIONS` = 2) and every version a job holds, and leaves a component
  whose pointer cannot be read alone. The install data gains `cleanup` (stages removed
  and kept, each version handled): an additive field of a schema not yet published
  (decision H4). `setup remove --stale-stages` runs the sweep alone. A stage is created
  only by an installation, which holds the guard throughout, so under the guard no stage
  is live. The sweep removes a stage only when its marker is intact and every entry is
  the marker, the artifact or a flat payload, runtime or smoke folder of single-link
  regular files, or when it is empty or holds only a partial marker (a creation killed
  mid-marker); links and junctions are never followed, and the marker goes last.
- **Every platform.** The commands work everywhere: where managed installation is not
  available the folder is normally absent, so list and repair report
  `managed_folder: absent` / `nothing_installed` with `managed_install:
  unavailable_target`, rollback is `INVALID_ARGUMENT` (nothing installed) and remove and
  the sweep report nothing to remove. None of them creates the managed root.
- **Contract.** New data schemas `setup-list`, `setup-rollback`, `setup-remove` and
  `setup-repair` with frozen examples; refusals (`INVALID_ARGUMENT` for nothing
  installed, an unknown or no earlier version, or the selected version;
  `INTEGRITY_FAILURE` for a version that does not verify; `STORAGE_IO` for a store that
  cannot be proved VSift's own) carry fixed prose and a suggested `setup list` or `setup
  repair`. No new failure code. The commands are short: `--events jsonl` writes the
  terminal event alone, and an interruption ends them with the operating system's
  default, which the crash-consistent steps below allow.
- **Crash points for PR 7.** (1) Selection and rollback: after `<component>.pending` is
  written and before its rename over `<component>.current` (the selection is unchanged;
  the sweep or the next selection removes the pending file); after the rename (done).
  (2) Deselection: one unlink of the pointer. (3) Version removal: after the tombstone,
  after the payload files, after the use lock, after the manifest, after the tombstone,
  before the folder's removal (each rerun finishes; openers refuse a tombstoned version).
  (4) Component removal: between the pointer's unlink and each version's removal. (5)
  Sweep: after any stage file, after a folder, after the artifact, after the marker and
  before the stage folder's removal (the next sweep finishes; an empty or partial-marker
  stage is recognised). (6) Install: the sweep before staging and the cleanup after the
  transaction are the same steps. No step fsyncs its directory after a rename, so PR 7's
  power-loss claim is fail-closed detection (lookup verifies every file on each open)
  plus repair, as decision H9 says.

## Implementation note, 2026-09-30 (P13 PR 9, the npm packages and the Verdaccio matrix)

Delivered from section 2 and decisions A, H5, H6 and H7. Nothing is published: the
packages are built, checked and qualified inside the Release workflow's own run, against
a registry on the runner's loopback address. Attestation, npm provenance and the publish
job stay PR 10's. The launcher package is `vsift-cli` (the amendment of decision A); the command it installs is `vsift`, and the qualification installs `vsift-cli@next`. The runbooks are [`install.md`](../operations/install.md) (users) and
[`release.md`](../operations/release.md) section 5 (maintainer).

- **Four packages, assembled from the archives.** `tools/vsift-release npm` reads the
  three release archives back (each must be byte-identical to a fresh packaging of its
  own contents, so only canonical archives are used) and writes four package folders;
  `npm pack` makes the tarballs on Ubuntu, twice, and the two must be identical;
  `vsift-release npm-verify` then checks every tarball against a fresh assembly: only
  `package/` entries, exactly the assembled files with the same bytes, the executable
  bit exactly on the executable and the launcher script, and a manifest without
  lifecycle scripts or `gypfile` and no `binding.gyp`.
  - `vsift-cli` (the name since the amendment of decision A below): `bin/vsift.cjs` and `lib/launcher.cjs`, its `package.json` and `README.md` from `npm/vsift-cli/`, the
    three licence files and `skills/vsift/` from the archives (decision H7: the skill ships
    in the npm package, byte-identical to every archive's), and `platform-digests.json`.
    H7 supersedes the 2026-09-28 launcher note's "only the launcher and its `bin` entry".
  - `@vsift/win32-x64`, `@vsift/darwin-arm64`, `@vsift/linux-x64`: the target's
    executable, its `THIRD-PARTY-NOTICES`, the licence files, a README and a manifest
    written by the tool: name, version, description, `license`, `homepage`,
    `repository` (the form npm provenance requires), `os`, `cpu`, `files`,
    `preferUnplugged` (Yarn's Plug'n'Play keeps the executable on disk, where it can run)
    and `publishConfig.access: public` (a scope's first publish is otherwise
    restricted). No `libc` field: only glibc ships (launcher boundary note). The SBOM
    stays in the archives (for the maintainer's review).
  - **No person in any manifest.** The launcher's manifest is checked field by field
    against a closed list (`name`, `version`, `description`, `license`, `homepage`,
    `repository`, `bin`, `files`, `engines` with `node >=22`, and `optionalDependencies`
    naming each platform package at the launcher's exact version); the platform manifests
    come from a fixed template. No author, contributor, maintainer or address is written.
- **The digest comes from the build (decision H5).** `platform-digests.json`
  (`vsift-platform-digests/1`) records each executable's size and SHA-256, computed by the
  tool from the archives; nothing is maintained by hand. The launcher compares the size,
  then the SHA-256 of the whole file. Measured cost of the launcher's checks (version and
  digest, median of seven in one process): 38.7 ms for the 9,716,736-byte Windows release
  executable on a workstation without SHA extensions (Node.js 22.16). The matrix measures
  it on every hosted runner and fails above 50 ms, so the check stays within H5's budget
  or the run says otherwise. Measured on the hosted runners (Release run 36772356382):
  median 4.6 to 5.2 ms on macOS 15 arm64, 8.4 to 9.7 ms on Ubuntu 24.04 and 10.5 to
  10.8 ms on Windows Server 2025, under Node.js and Bun alike.
- **The launcher (`npm/vsift-cli/lib/launcher.cjs`, run by the three-line `bin/vsift.cjs`).** Plain CommonJS with `node:` built-ins
  only, about 250 lines. It maps `process.platform` and `process.arch` to the three
  targets, resolves `@vsift/<target>/package.json` with `require.resolve` from itself (so
  npm, pnpm's links, Yarn's Plug'n'Play and Bun resolve alike), requires the platform
  package's version to equal its own, checks the digest, and spawns the executable with
  `shell: false`, the arguments unchanged and the three standard streams inherited.
  *Decided here:* a launcher failure is one readable message on stderr and exit **127**
  when no usable platform package exists (an unsupported platform, optional dependencies
  omitted) or **126** when the package is refused or cannot start (another version, a
  digest or size mismatch, a damaged digest file, a start error, with the glibc 2.35 and
  OpenSSL 3 requirement named on Linux and Windows' 260-character path limit named when
  the path reaches it, [L-094](../planning/known-limits.md#l-094)): the shell's "not
  found" and "cannot execute".
  `vsift` itself never exits with either (`cli-v1.md`, exit taxonomy). Messages name the
  expected package, the version and the supported targets, never a stack trace.
- **Signals (decided here).** On Windows a console Ctrl-C or Ctrl-Break reaches every
  process of the console, so the launcher relays nothing and only stays alive (handlers
  for `SIGINT`, `SIGBREAK` and `SIGHUP`) until vsift has ended, then exits with its
  status. Elsewhere the launcher relays `SIGTERM` and `SIGHUP` once, and `SIGINT` unless
  one of its standard streams is a terminal: a terminal's Ctrl-C goes to the whole
  foreground process group, so vsift already has it, and relaying it would be vsift's
  second interruption, which escalates (ADR 0020 section 5). When vsift ends by a signal,
  the launcher removes its handlers and re-raises the same signal, so a shell sees 128
  plus its number. A signal sent to a whole process group that is not a terminal's (a
  supervisor, a tree kill) reaches vsift twice: [L-091](../planning/known-limits.md#l-091).
- **Governance.** `vsift-governance check` fails any `package.json` under `npm/` that
  declares `scripts`, `gypfile`, `author`, `contributors` or `maintainers`, any
  `binding.gyp` there, and a launcher manifest whose optional dependencies are not at its
  own exact version or that declares another kind of dependency; `npm-verify` holds the
  packed tarballs to the same rule.
- **The Verdaccio matrix (`release.yml`, jobs `npm-package` and `npm-qualify`).** The
  qualification runs in the Release workflow, so the tarballs it qualifies are the ones
  PR 10 will publish. Twelve jobs: `windows-2025`, `macos-15` and `ubuntu-24.04`, each
  with npm, pnpm, Yarn and Bun, on Node.js 22.23.3 and Bun 1.2.23 (the minimum runtimes of
  decision H6), the npm that Node.js ships, pnpm 12.8.1, Yarn 4.18.1 and Verdaccio 6.10.4,
  all pinned in the workflow. `npm/qualification/qualify.cjs` starts Verdaccio on
  `127.0.0.1:4873` with no uplink, no audit middleware and no web interface, creates a
  throwaway local user and writes its token only into a scratch npmrc scoped to that
  address, publishes the four tarballs with `npm publish --tag next --ignore-scripts`,
  removes every `npm_config_`, `YARN_`, `BUN_CONFIG_`, `PNPM_` and token variable from
  the environment of every package manager, and refuses any other registry. Checks per
  job: a global install (Yarn 4 has none, so a project install) with scripts disabled
  under a folder whose name has spaces, accents and CJK letters; `--version` prints the
  build's line naming the commit; `setup check --json`; `handoff check --file` with a
  path of spaces and Unicode through each installed command (`vsift.cmd` and `vsift.ps1`
  on Windows through PowerShell, the `bin` link elsewhere); exit statuses equal to
  vsift's own (2, 2 and 7); standard input; the cost of the launcher's checks; signals
  (POSIX: `SIGTERM` and `SIGINT` sent to the launcher alone end vsift and the launcher by
  that signal, with no process left holding the output pipes; Windows: a console
  Ctrl-Break ends both with `STATUS_CONTROL_C_EXIT` and leaves no orphan); a changed
  byte, a replaced executable, a mismatched platform-package version and (POSIX) a
  non-executable binary each refused readably, and the restored install running again;
  the one-shot runner (`npx`, `pnpm dlx`, `yarn dlx`, `bunx`, and `bunx --bun` on the Bun
  runtime); optional dependencies omitted (`--omit=optional`, `--no-optional`, Bun's
  `--omit optional`, and for Yarn, which cannot omit them, `supportedArchitectures` for
  another platform) giving exit 127 and a message naming the package without a stack
  trace; running with the registry stopped; and a clean uninstall (no launcher, platform
  package or command left; Bun 1.2 leaves the platform package, L-092). The launcher's own tests (`npm/test/launcher.test.cjs`, CI
  job `npm launcher` on three operating systems) cover selection, every refusal for every
  target, arguments with spaces, Unicode and shell metacharacters, streams, exit statuses,
  and signals with a stand-in executable (a targeted and a process-group interruption on
  POSIX, and on Windows a console Ctrl-Break that a launcher without its handler fails).
- **For the maintainer's review:** exits 126 and 127; the signal rules above; the SBOM
  left out of the platform packages; the Release workflow running twelve more jobs on
  every pull request that touches an archive or npm input; Yarn qualified through a
  project install and without the launcher-level signal checks (Plug'n'Play runs the
  launcher from a zip archive only `yarn` can open); only the minimum runtimes in the
  matrix ([L-092](../planning/known-limits.md#l-092)); the digest check's reach
  ([L-093](../planning/known-limits.md#l-093)).
- **Found while qualifying, for PR 10 and the release checklist:** Yarn 4.18 quarantines
  a version for a day after it is published (`npmMinimalAgeGate`, default one day), so a
  fresh `vsift-cli@next` cannot be installed with Yarn for its first day unless the user
  waits or preapproves `vsift-cli` and `@vsift/*` (`install.md`; the matrix sets the gate to
  zero because it installs seconds after publishing). On Windows an executable path of
  260 characters or more cannot be started (L-094); a deep pnpm global store under a long
  home folder reaches it. On Windows, Bun 1.2.23 fails a global install into a folder
  with non-ASCII letters ("InvalidWtf8") and leaves its `vsift.exe` shim after a global
  remove; `bunx --bun` there ran the command file without making it Node.js's main
  module, so the launcher is split: `bin/vsift.cjs` always calls `main()` of
  `lib/launcher.cjs` rather than testing `require.main` (L-092).
