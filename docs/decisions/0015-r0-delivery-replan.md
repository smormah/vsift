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
