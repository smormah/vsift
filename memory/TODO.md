# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification) is not started, and under governance rule 10
it does not start on its own: the maintainer says when.** P13 finished on 2026-10-01 with
the 0.1.0 pre-release: `vsift-cli`, `@vsift/win32-x64`, `@vsift/darwin-arm64` and
`@vsift/linux-x64` on npm under `next` (`latest` is still the empty `0.0.0` placeholder),
and the GitHub pre-release `v0.1.0` with ten files. The ledger names the release commit
`011bc4d`; PR 12 (this change) holds the record, in `docs/planning/p13-distribution.md`.

**What the pre-release means:** anyone can install it (`npm install --global vsift-cli@next`
or the archives) and check it (`npm audit signatures` and `gh attestation verify` passed).
**What it does not mean:** a stable or supported release (nothing is announced; promotion
waits for P14). It was installed once from the real registry, on the maintainer's Windows
11 machine with npm: not a clean machine, not pnpm, Yarn or Bun, no agent has used it, and
Smart App Control is untried (L-098).

**The first publish took two tries.** The first approved run failed with `ENEEDAUTH` before
anything was published: the maintainer reported the npm-to-GitHub trusted-publisher
connections had never been completed (an earlier report said they were). The second run
published. npm gives no reason (L-100), so `release.md` 6.2 has a preflight.

**The P14 plan awaits the maintainer's confirmation (2026-10-02):** ADR 0024 (Proposed) and
`docs/planning/p14-qualification.md`; the eight decisions head the draft PR "P14 PR 0".

## What remains

1. **P14, the R0 qualification** (#17; not started). It holds: the release evidence ledger;
   fuzz, race, fault and soak runs; findings triage; the supported-profile matrix (L-035);
   user docs; a release candidate; R-01..R-14, SEC-01..SEC-25 and R-SEC03. It includes the
   named Codex and Claude Code run from a clean install on both transcript paths (decision
   H10) with a **cold-agent variant** (CLI on `PATH`, no skill, no docs; decided 2026-10-02),
   the clean-machine install with each package manager from the real registry, real-tool
   runs on hosted CI (L-042) and the stable-release decisions (version, `latest`, signing).
2. **What P14 needs from the maintainer:** to say "start"; the review tier's A-09 blurred
   re-run (L-095, #224, on the maintainer's accounts); SEC-T01's adversarial evidence or a
   decision (#188, L-068); the Smart App Control try-out on Windows 11 and a macOS 15 browser
   download (L-098); a decision on #204.
3. **Decided 2026-10-02 (maintainer):** (a) at the completion of R0 (after P14) we use the
   published CLI ourselves as a trial, with notes reviewed in batches; raised once at that
   point, and it installs nothing without the maintainer's word. (b) P14's clean-install run
   includes the cold-agent variant: an agent must be able to use the CLI from its own help
   and errors alone, with no skill and no MCP, which no trial has tested (every P12 trial
   loaded the skill); if it shows gaps, the cheap fix is a short "typical investigation"
   section in `vsift --help`.

## Open decisions (maintainer; decided ones are in ADRs 0021-0023)

- **#246:** staged publishing (a second approval on npmjs.com); deferred by the maintainer
  (2026-10-02) until after R1 or the public announcements; **#204:** Codex on Windows (L-076); the grader's `untrusted_listed` reading (F12-E01 only);
  #219.
- **P11 readings** (ADR 0021 notes): `KillMode=mixed` and resubmission (L-069); batch limits
  and exit 6 for a job-cancelled line (L-067); the engine's `tokio`; continuable failures,
  pruning, 192 KiB records, D2; `durable_worker` for ephemeral workspaces; links (L-062).
- **P13 readings** (each in its ADR 0023 note; none blocks anything): worker hosts render
  only their result (L-017); a failed smoke is `MISSING_CAPABILITY`; `407` by text (L-088);
  removal proves ownership (L-090); Windows `.tar.gz`; exits 126/127; `attest` without an
  approval; Release as a required check. **Also:** MSRV; an MCP adapter.

## Tracked issues

- **Close with this change:** #16 (the P13 packet issue, still open). **Open:** #17 (P14),
  #219, #224, #232 (a session root name with controls fails on Linux), #246; #170-#178
  (the L-numbers of the register); #159, #150 fixtures; #147; #128 flaky tests; #205, #206.

## Known issues and gates

- **Opt-in real-tool paths** (`--ignored`, never run by CI alone): the P07-P11 E2E tests, the
  `*_tools` engine tests, the Windows console-interrupt tests, the external-delivery
  simulation (L-042) and P13's real downloads (workflows `P13 managed smoke` and `P13
  managed power loss`, dispatch only).
- **npm:** `node --test npm/test/launcher.test.cjs` when `npm/` changes; a version bump also
  bumps `npm/vsift-cli/package.json` and its three optional dependencies.
- **Releasing again:** `docs/operations/release.md` section 6 (tag, dry run, preflight,
  dispatch with `dry_run` cleared, approve, verify). Only `plan`/`attest`/`publish` publish.
- **Campaigns:** never on disks that matter (L-056, L-057); bump the three `UBUNTU_IMAGE_*`
  together; trial records never hold the check code.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff,
  on both the supplied-transcript and local-ASR paths, in named Codex and Claude Code
  trials. P12 qualified both tiers; a run from a clean install is P14's (decision H10).
- **The skill** orchestrates the published CLI only: no processing logic, no tool
  grants. A new public command, flag, failure code or referenced field needs a skill
  update in the same change (`skill_contract` fails otherwise); its one input exception is
  the two `handoff check` forms. The check code lives only in its pixels, guard and grader.
- **New public items:** a public command, failure code, event kind or record type needs its
  `CommandName`, `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry, v1
  schemas and, for a command that completes, a renderer in `crates/vsift-cli/src/human/`
  with a snapshot. v1 changes are additive only since 0.1.0. A parser of untrusted input
  needs a seeded fuzz target. No npm package has scripts or names a person.
- **Commits:** session commits go through `CommitHooks`/`Commit`. A change to the
  commit path, or to request-record writes, reruns the crash campaign. Never enable
  `fault-injection`, `durability-campaign` or `install-test-hooks` in a release (the
  workflow lint fails a `release.yml` that selects any). R1 starts only after P14.
