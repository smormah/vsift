# ADR 0015: Re-plan R0 delivery around the working evidence pipeline

- Status: Accepted
- Date: 2026-09-23
- Tracking: [P06 / issue #9](https://github.com/smormah/vsift/issues/9) and
  [P13 / issue #16](https://github.com/smormah/vsift/issues/16)
- Refines: [ADR 0014](0014-progressive-dependency-setup.md); keeps
  [ADR 0007](0007-managed-runtime-provisioning.md) as the managed-installer design

## Context

P06 bundled two different things: getting a user's *existing* media and speech
tools recognised, selected and trusted, and a complete *managed installer* with
reviewed download catalogues, staging, activation, rollback, removal, cleanup
and power-loss qualification. By 2026-09-23 about 60 pull requests had landed
detection, bring-your-own (BYO) selection, read-only plans, the reviewed Ubuntu
catalogue, a compatibility policy and the installer's storage primitives. The
installer transaction and its public commands were still open, and the P06
retrospective sized the packet at six to eight ordinary packets.

The packets run in order and a packet cannot start until its predecessor is
complete. That made transcription, search, evidence retrieval and the agent
skill wait for the installer. Nothing user-visible can yet read a video's
content. The maintainer's own rule is that VSift must be seen working and used
before more time is spent elsewhere.

Delivery has also changed hands. The planning and governance machinery was
designed so small models could implement packets unattended. Work now moves to
an attended implementer, with the maintainer reviewing through the same
protected-branch checks.

## Decision

### Scope and sequence

1. **R0 scope is unchanged.** The detect → explicitly install → guide journey of
   ADR 0014 remains an R0 release requirement, and so does at least one
   qualified managed-install target.
2. **P06 completes on the "detect, select, verify, guide" half:**
   - Detection and precedence, persistent BYO executable/model selection,
     read-only plans and typed manual guidance. These have already landed.
   - A bounded compatibility check of the *selected* FFmpeg and FFprobe
     against the checked-in F01 fixture, run through the existing process
     supervisor under the limits fixed in the reviewed compatibility policy.
   - A model check that reports a known pinned digest, or an explicit
     "unverified model" state; it never infers readiness from file presence.
   - `setup check`/`setup plan` report what was actually verified instead of
     `executable_probe_only` where the check ran.
   - Evidence: D-01, the unavailable-target guidance part of D-07, D-09 and
     D-10, plus the P06 stage of the E2E spine for preinstalled, partial,
     off-PATH, denied, offline and unqualified setups.
   - Whisper functional verification arrives with P07's whisper.cpp adapter;
     `setup check` then reports it.
3. **Managed installation moves to P13 (distribution), still in R0.** This
   covers the accepted-plan transaction, direct download, staging, smoke test
   before activation, atomic activation, `setup install/repair/list/rollback/remove`,
   bounded version cleanup and interruption/power-loss qualification. D-02..D-08
   and threats SEC-12..SEC-15 and SEC-23 move with it. Installing VSift's own
   binaries and installing its dependencies are one distribution concern.
   `setup install` stays reserved and returns `COMMAND_NOT_IMPLEMENTED` until
   then. The merged catalogue, plan-acceptance and storage primitives stay in
   place; the resume order recorded at parking becomes P13's installer sequence.
4. **P07 is the next packet after P06 closes.** P07 starts with the embeddable
   engine boundary of [ADR 0016](0016-embeddable-engine-and-evidence-contract.md).

### Delivery process

5. **One pull request per coherent increment.** It carries code, tests, docs
   and handoff updates together. There are no separate "record evidence" pull
   requests. The pull request description holds the verification commands and
   results; CI runs are linked from the pull request itself.
6. **Packet completion is recorded once.** A small follow-up that sets the
   packet to `complete`, with the merge commit and verification summary the
   ledger requires, is allowed only when a whole packet finishes.
7. **Handoff files describe the current state, not history.** They are
   `memory/TODO.md` and `memory/project_current_status.md`. Each update rewrites
   them to stay true and short; the governance checker enforces size limits.
   History lives in git, the changelog, the qualification records and
   [the archived delivery log](../history/2026-09-09-to-23-delivery-log.md).
8. **Status is reported in plain English.** Every update says whether an
   *increment* or the *whole packet* is complete, and what remains.
9. **A failed required check is evidence, not noise.** Before re-running, link
   the failure to a tracked issue. Fixing a tracked intermittent failure takes
   priority over new feature work.

## Consequences

- The first user-visible evidence capability (transcripts, then search and
  frames) arrives several packets sooner. The managed installer arrives later
  in R0, but its merged foundations remain in place.
- P06 becomes closeable, and P13 grows to include dependency installation.
  Issue #9 keeps the verification scope; issue #16 gains the installer scope.
- Until P13, users on every target install FFmpeg, FFprobe and whisper.cpp
  themselves and register them with `setup configure`/`setup configure-model`.
  `setup check` and `setup plan` explain exactly what is missing.
- The reviewed Ubuntu catalogue has a 2028-08-01 stop-new-plans cutoff and
  depends on the publisher's retention of the month-end FFmpeg asset. P13 must
  revalidate or replace it through a reviewed catalogue revision.
- Pull request volume and session-start reading drop substantially. Reviewers
  read one PR per increment, and the size limits keep handoff files usable.

## 2026-09-23 implementation note: verification is an engine capability

Decision 2 said `setup check`/`setup plan` would report what was verified.
`setup check`'s v1 schema fixes `verification_scope` to `executable_probe_only`,
and ADR 0008 keeps that payload compatible, so that wording is corrected:

- Verification is an engine capability. The `MediaToolVerifier` port lives in
  the application layer; `FixtureMediaToolVerifier` runs the embedded F01 fixture
  through the real P04 probe, frame and audio operations and compares each
  result with F01's recorded truth. `verify_model_file` identifies a registered
  model against the reviewed pinned digest.
- Its consumer is an automatic preflight before the first media stage, which
  arrives with P07. No separate CLI command is added until an operator-facing
  consumer needs one, such as worker readiness (P11) or the installer (P13).
- `setup check` keeps its fast executable probe and its v1 contract unchanged.

## 2026-09-24 implementation note: the media-tool preflight is wired

- P07 wires the preflight named above. `preflight_media_tools` (application)
  reuses a recorded pass for an unchanged tool identity or runs the
  `MediaToolVerifier`; only passes are offered to the new
  `MediaToolVerificationCache` port, which fails closed and never returns an error.
- The engine runs it inside operations that execute FFmpeg/FFprobe on user media,
  before their first media stage and before the session root is touched: today
  `ingest --transcript`. Plain `ingest`, setup commands and `transcript get` do
  not run it. Local ASR, frames and audio will call the same hook.
- The record lives beside the `setup configure` selections in the private per-user
  directory (`media-tool-verification/verified-v1.json`), holding at most eight
  SHA-256 fingerprints with their verification times. A fingerprint binds both
  canonical executable paths and their on-disk identity, the reviewed policy and
  fixture digest, the adapter profile, host isolation, verifier authority, a
  verification-profile version and the VSift version. Executable contents are
  not hashed (cost on every operation, and shared libraries would be missed
  anyway); passes age out after seven days instead. Details and the failure
  result are in `docs/contracts/cli-v1.md`.
- A failure is `EngineError::MediaToolVerificationFailed` with the failed check
  and reason. It maps to existing codes (`MISSING_CAPABILITY` for an unusable
  tool; `DEADLINE_EXCEEDED`, `STORAGE_IO`, `CANCELLED` or `INTERNAL` where those
  are the truthful cause), so no new failure code or schema change was needed;
  the check and reason travel as typed identifiers in the fixed-prose remediation.
- Still no CLI command: `setup check` keeps `executable_probe_only`.
  `EnginePorts::with_media_tool_verifier` lets tests and other hosts replace the
  fixture verifier; such passes are recorded under a separate identity.

## 2026-09-25 implementation note: local-ASR verification before use

P07 increment 3b adds the whisper.cpp functional verification decision 2 promised,
as a second automatic preflight: before `transcript retranscribe` decodes the user's
audio, after the media-tool preflight, the selected recognizer and model transcribe
a reviewed speech clip built into VSift (F01) and must reproduce its words inside its
speech window. It shares the verification record (a separate fingerprint domain) and
the leftover-workspace sweep. `setup check` still reports `executable_probe_only`;
reporting local ASR there is increment 3c (decision D4). Details in ADR 0017.

## 2026-09-24 amendment: record readers and leftover verification workspaces

- Readers of the verification record take no lock and can catch a writer's
  rename: the file they opened is already unlinked (link count zero), or on
  Windows the name briefly refuses opens while the replaced file is pending
  deletion. Both were misread as an unsafe record (issue #136). They are now a
  transient replacement: the read is retried a bounded number of times and
  otherwise counts as "not verified". A record with more than one link is still
  unsafe. Flushing the new record before the rename is best effort, because a
  record lost or torn by a crash already reads as "not verified". Taking the
  write lock retries brief failures (an interrupted call, the lock file being
  created concurrently, a refused lock on a proven single-link regular file) a
  bounded number of times; a linked or non-regular lock file is refused at once
  and never reported as busy.
- A verification killed mid-run left its `vsift-tool-verification-<16 hex>`
  workspace in the state directory (issue #132). Each workspace now holds an
  exclusive lock on its `workspace.lock` for its whole life, and each preflight,
  holding the record lock without waiting, removes workspaces that are exactly
  so named, real directories, unchanged for at least an hour and not locked by a
  live verification (at most eight per preflight). Nothing else in the state
  directory is touched, links are never followed, and a workspace whose lock is
  held is kept whatever its age.
