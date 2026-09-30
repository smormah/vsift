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
**The scope is pending the maintainer's choice.** The executable stays `vsift` (ADR
0001, ADR 0009).

*Correction, 2026-09-30 (maintainer):* the plan named the scope `@vsift` with an
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
out of a plain `npm install vsift`. crates.io needs every workspace crate published
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
