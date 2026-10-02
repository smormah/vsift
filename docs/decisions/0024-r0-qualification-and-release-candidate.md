# ADR 0024: R0 qualification and the release candidate

- Status: **Proposed** (2026-10-02). This is the P14 plan. The maintainer **confirmed all
  eight decisions, A to H, on 2026-10-02, exactly as recommended**, and started P14 that
  day: the delivery ledger marks it `in_progress` (governance rule 10 was met by the
  maintainer's word). The ADR itself stays Proposed until P14 completes, when it becomes
  Accepted, as ADR 0021, ADR 0022 and ADR 0023 did.
- Date: 2026-10-02
- Tracking: [P14 / issue #17](https://github.com/smormah/vsift/issues/17)
- Refines: [ADR 0005](0005-r0-scope-and-qualification-profiles.md) (public support begins
  only after P14), [ADR 0010](0010-storage-qualification-gate.md) and
  [ADR 0021](0021-worker-and-batch-host.md) (the strict worker profile),
  [ADR 0022](0022-agent-skill-and-named-client-qualification.md) (the named-client trials and
  the cold-agent note of 2026-10-02) and
  [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) (decisions B and
  C, and H10)
- Applies to: P14; R-01..R-14; every row of
  [verification](../planning/verification.md); SEC-01..SEC-25; R-SEC03. The plan, its
  traceability tables and its budgets are in
  [`p14-qualification.md`](../planning/p14-qualification.md).

## Context

### What R0 is

The ledger fixes the R0 objective: an agent-operated local video evidence tool with a
production-quality single-host worker core. R0 ships only when a coding agent goes from a
local video to a grounded handoff, on the supplied-transcript path and on the local-ASR path,
in named Codex and Claude Code trials; component tests cannot substitute (governance rule 11,
verification section 7). P14 is the packet that decides whether that is true of what a user
can actually install, and what may be said about it in public.

### What P00-P13 proved

| Area | What exists |
| --- | --- |
| Product behaviour | Every R0 command works; contract tests, frozen examples and the v1 schemas run on Ubuntu, macOS and Windows in every pull request |
| Evidence accuracy | Recall, frame, crop and ASR gates against a frozen synthetic corpus (F01-F12), measured on one Windows 11 machine (P08, P09) and, for ASR, on hosted Ubuntu 24.04 and Windows Server 2025 (P07, weekly) |
| Recovery and durability | Kill matrices on every CI operating system; an Ubuntu 24.04 / ext4 power-loss campaign for sessions (P10, re-run weekly) and for the managed store (P13, on dispatch only) |
| Worker | Weighted admission, batches, shutdown and redelivery on one Windows machine (P11); a hardened-container CI job that shows the controls are present |
| Agents | The skill and 21 scenarios; named-client trials: Claude Code on Windows, Codex in a Linux container; review tier 11 of 11 mechanical on both clients; compact tier 93% and 100% on the #222 re-run (P12, P13) |
| Distribution | Reproducible archives, a twelve-job npm matrix against a local registry, Sigstore attestations, a protected publish; 0.1.0 is published under `next` and was verified once (P13) |

### What is weaker than it sounds

These are the facts the plan is built around. Each is checked against the code or the
records named, not assumed.

1. **No checkpoint ran the published artifact.** Every real-tool checkpoint of the
   `vsift-cli` crate (`p07_local_asr_e2e`, `p08_candidates_e2e`, `p09_evidence_e2e`,
   `p10_recovery_e2e`, `p11_worker_e2e`, `p13_install_e2e`, `p13_managed_install_real`) runs
   the binary Cargo builds inside the test run (`Command::cargo_bin("vsift")`). The
   twelve-job npm matrix runs the published bytes, but only `--version`, `setup check`,
   `handoff check`, argument handling and signals, never a video. The agent trials ran a
   source build too.
2. **No clean machine.** Hosted runners carry a Rust toolchain on `PATH` (the P13 record
   shows it) and other developer tools, and the matrix does not remove them. The one install
   from the real registry was npm, on the maintainer's Windows 11 development machine.
3. **One real platform.** The P08 and P09 numbers, the P11 checkpoint and the Claude Code
   trials are Windows 11. Hosted Ubuntu 24.04 has the local-ASR journey and the managed
   install. macOS has the launcher qualification and ordinary CI, and no media, speech or
   evidence run at all (L-035).
4. **A synthetic corpus.** Speech is synthetic voice; the one noisy clip has a word error
   rate of 61.5% and is not gated; change thresholds, scrolling and tiny text are
   calibrated on drawn video (L-020, L-022, L-028, L-030). Nothing has been tried on a real
   recording.
5. **Trials tuned on their own scenarios.** The skill, the grader and the scenarios
   changed together across rounds, the compact re-run used the same scenarios, and
   GPT-6-Sol's 28 of 28 rests on one maintainer reading (23 of 28 as run). No scenario was
   held out, and no trial has ever run without the skill.
6. **The review tier's blurred-banner re-run is missing** (L-095, #224), and compact-tier
   citations are not at 100% (2 of 62 phases on the re-run).
7. **SEC-T01 is half done.** The strict worker's controls are shown present; that they
   contain a hostile provider is not (L-068, #188). Dangerous decompression-bomb media has
   never been run (ADR 0012, L-004).
8. **Load is a partial ladder.** Concurrency 1, 2 and 4 ran once on Windows; the 8-job rung,
   the 100-request batch and the soak did not. Fuzzing is weekly, five minutes per target.
9. **Unsigned executables are untried on a real protected machine** (L-098): no Smart App
   Control, no Gatekeeper browser download. The maintainer's own Windows 11 machine has
   Smart App Control Off (read 2026-10-02), so the 0.1.0 install and run there says nothing
   about it.
10. **The offline install has run with stand-ins only** (D-07), and nothing watches the
    publishers' files between releases except a manual dispatch (L-099, L-042).
11. **The register is unreviewed.** Every one of the 91 known-limit entries says `Review:
    pending`, and the claims the README makes lean on some of them.
12. **A first stable release has never happened.** `latest` is an empty placeholder on all
    four packages, `release.yml` refuses a stable version in every mode by design, and the
    publish path that moves `latest` has never run.

### What P14 inherits from the maintainer's earlier decisions

ADR 0023 decision H10 (the named-agent run from a clean install of the published
pre-release); the cold-agent variant (2026-10-02, ADR 0022's note); the clean install with
each package manager from the real registry; real-tool runs on hosted CI; the
supported-profile matrix (L-035); the stable-release decisions of version, `latest` and
signing; and the maintainer's hands-on items (L-095, SEC-T01, Smart App Control and
Gatekeeper, Codex on Windows, the open readings). Deferred and not in P14: staged npm
publishing ([#246](https://github.com/smormah/vsift/issues/246), until after R1 or the public
announcements) and using the published CLI ourselves, which happens at the completion of R0.

## Decisions for the maintainer (all eight confirmed on 2026-10-02)

Eight decisions, in plain English. Each has the options that were put to the maintainer, the
option **decided** (every one the recommended option), the reason and what it costs the
maintainer. The rest of this ADR and the plan are built on these decisions.

### A. Which version is R0, and how does a release become "stable"?

- **Options.** (1) `0.2.0` on npm's `latest` tag, promising the command-line grammar, exit
  codes and v1 JSON (additive-only since 0.1.0) and nothing else; 1.0.0 is chosen later. (2)
  `1.0.0` now, with the same promise under SemVer's "1". (3) Stay on 0.x pre-releases under
  `next` and never move `latest`.
- **Decided: option 1.** Everything measured about accuracy comes from a synthetic corpus,
  and the first real recordings come with the post-R0 trial. 1.0.0 should mean "we support
  this surface"; announcing it before real recordings have been tried risks a 2.0.0 soon
  after. Moving to 1.0.0 later costs a version number if no contract change is needed.
  Option 3 leaves the default install an empty package, which defeats the point of a
  release.
- **Stable procedure (same for any version).** Stable means a version with no pre-release
  suffix. It is published only from a tag, by the same workflow, dispatch and approval as
  0.1.0, and `latest` moves on all four packages in one publish (the first time it leaves
  the empty placeholder). The stable commit may differ from the accepted release candidate
  only in version strings and in documents that ship inside the artifacts, which a
  mechanical check enforces, and the hosted qualification is re-run on the stable bytes. The
  workflow keeps its rule of never running `npm dist-tag`.
- **Cost to you.** This decision. One publish session for the stable release (about an
  hour, as for 0.1.0). If you choose 1.0.0, you also accept the compatibility promise for
  the whole 1.x line.

### B. What does "release candidate" mean?

- **Options.** (1) A published pre-release `0.2.0-rc.1` (and `-rc.2` if findings need it)
  under `next`, built and published exactly as 0.1.0 was; the whole qualification runs
  against it. (2) Call the existing 0.1.0 the candidate. (3) A candidate that is built but
  never published.
- **Decided: option 1.** The clean install "from the real registry" needs a published
  version; 0.1.0 predates every change P14 will make; an unpublished build cannot be
  installed from the registry. The freeze rule: from the cut, only fixes for findings; no
  features; at most two candidates planned, a third is your call. A candidate is never
  announced and never `latest`.
- **Cost to you.** One publish session per candidate (dispatch, approve, verify; about an
  hour). Candidate version numbers are permanent on npm: a bad one is deprecated, never
  removed.

### C. Do the Windows and macOS executables get signed for the stable release?

- **Options.** (1) Keep them unsigned, as ADR 0023 decision C chose for the pre-release,
  and revisit on evidence. (2) Sign the Windows executable only. (3) Sign Windows and
  notarize macOS.
- **Decided: option 1, with a trigger agreed now.** If the try-outs of decision H show that
  Smart App Control or Gatekeeper blocks an npm-installed VSift on a default machine with
  no way through short of turning protection off, you then choose between a documented
  limitation and signing before the stable release. Signing costs recurring fees (a
  certificate or signing service; an Apple developer account), key custody, a new secret or
  identity in the most security-sensitive workflow, and new lint rules; nobody has yet seen
  a prompt on a VSift file (L-098), so deciding now would be deciding without data. npm
  installs carry no download mark, which is the common path.
- **Cost to you.** Nothing now. If the trigger fires, a decision, an account and its fees,
  and about a week of work before the stable release.

### D. How many agent trials, on what, and who pays?

- **Options.** *Lean:* review tier only, with the skill, from a clean install, plus a small
  cold-agent run: about 36 runs. *Recommended:* both tiers with the skill (34 runs), the
  cold-agent variant (30), pilots (8) and a reserve (12): about 84 runs. *Full:* repeat
  P12's whole corpus (78 counted runs) from a clean install, plus the cold-agent variant and
  pilots: about 125.
- **Decided: the "Recommended" plan.** It meets governance rule 11 on both transcript paths with
  both named clients, adds one scenario per path that no earlier round has seen, includes
  the review tier's blurred-banner re-run (L-095), and runs the cold agent twice (before
  and after any `vsift --help` change). It is about one and a half of P12's compact rounds.
  Claude Code runs on your Windows 11 machine as before; Codex runs in the Linux container,
  with VSift installed from the real registry into a fresh folder in both cases; the
  harness, not the agent, runs `setup install` or registers tools, as a user would.
- **Cost to you.** Your client allowances, in three batches that each wait for your go.
  P12's measure was about a day and a large share of a weekly allowance per 56 runs, so
  expect roughly a day and a half of wall time spread over a week, with usage-limit pauses;
  the review-tier runs (Opus 5.5, GPT-6-Astra) are the expensive part. The harness will record
  usage where the clients report it, so the real spend is on file. Tokens were not recorded
  in P12: these are estimates.

### E. What do we do about SEC-T01 (#188, L-068), and may we claim a strict worker?

- **Options.** (1) A hostile stand-in provider, written and reviewed under your control,
  run in the hardened CI container (option A of the handoff). (2) A recognised third-party
  containment suite (B). (3) An external review (C), later. (4) Narrow the claim: R0 ships
  with no claim that the strict profile contains a hostile decoder, the worker host stays a
  "qualification target", and #188 moves to R1.
- **Decided: option 1, with option 4 as the fallback.** The worker host is in R0's scope and
  DEC-11 says the strict profile is qualified first; the adversarial half is what is
  missing. The fixture is small and runs in a disposable container. The deferral began
  because an automated safety check stopped the session that was authoring it: if that
  recurs, you write or review the fixture yourself, or choose 4, which needs a short ADR
  amendment and costs the strict-isolation claim. External review (3) is worth doing before
  any multi-tenant or industrial claim, not for R0.
- **Cost to you.** An hour or two reading the fixture and its CI job, plus one decision if
  the fallback is needed.

### F. What does the supported-profile matrix promise, and does managed install stay Ubuntu-only?

- **Options.** (1) "Supported" per cell, earned by rules fixed now (below); managed install
  stays Ubuntu 24.04 x64 only. (2) Call every R0 target supported. (3) Call only the
  platform the trials ran on supported.
- **Decided: option 1.** A cell is supported when the published packages install from the
  real registry on a scrubbed image with every package manager, the extracted archive runs,
  the published binary completes the supplied-transcript journey and the local-ASR journey
  with that platform's documented tools, and the install, upgrade and uninstall steps in
  the guide were walked. A named agent client is "qualified" only on the system it was
  trialled on. Expected result: Windows 11 and Ubuntu 24.04 supported; macOS 15 arm64
  supported for what the hosted run proves (your tools, no managed install, no agent trial)
  or left a "qualification target" if it does not pass; Codex on Windows documented as not
  supported (#204, L-076); the strict worker follows decision E. Managed install stays
  Ubuntu-only (ADR 0023 decision E): no reviewed Windows or macOS candidate exists.
- **Cost to you.** This decision, and the matrix wording in the README. Users on macOS may
  see "target" instead of "supported" if the hosted run fails.

### G. What may be claimed in public, and when?

- **Options.** (1) A ladder: now (pre-release), at the release candidate, and after P14,
  each with a fixed list of allowed statements, checked mechanically against recorded
  evidence. (2) Update the wording by hand at the end. (3) Make no claims beyond the
  install guide until after R1.
- **Decided: option 1.** Allowed now: the existing pre-release wording, no "supported" and
  no announcement. At the candidate: "release candidate under qualification", the evidence
  summary and nothing about support. After P14 completes: matrix-backed "supported" per
  cell, the qualified models per client, the measured numbers with their conditions
  ("measured on a synthetic corpus"), and promotion may begin (yours to start; LinkedIn
  first). Never claimed even then: production readiness for real recordings, a strict
  worker (unless E is resolved), multi-tenant use, publisher trust for unsigned files,
  durability on any filesystem but Ubuntu 24.04 ext4, managed install outside Ubuntu, and
  any model or client not trialled. The check proves a listed claim has recorded evidence
  and that banned words are absent; it does not prove a sentence is true.
- **Cost to you.** Reading the claims list once; later edits to claims go through a pull
  request.

### H. What only you can do, and what blocks the stable release?

- **Your hands.** A Smart App Control try-out needs a Windows 11 machine or virtual machine
  where it is On; a browser-download try-out needs a Mac with macOS 15; starting each
  agent-trial batch; the A-09 blurred re-run (inside decision D); reviewing the SEC-T01
  fixture (E); each publish session (B, A); one pass over the register entries the public
  claims lean on (about thirty, listed in the plan) and the readings in `memory/TODO.md`;
  merging the pull requests.
- **What is known since the decision (2026-10-02).** Smart App Control is **Off** on the
  maintainer's Windows 11 Pro machine (registry value `VerifiedAndReputablePolicyState` is
  0, read that day), so the 0.1.0 install and run there says nothing about it and the
  try-out needs a fresh Windows 11 virtual machine or another PC (a fresh Windows install
  starts Smart App Control in evaluation mode). Not known: whether Windows Sandbox is
  available (it could not be read without elevation), whether the maintainer owns a Mac, and
  whether hosted-runner minutes are free for the account (the plan's unknowns table).
- **Options for what blocks the stable.** (1) The try-outs block the stable only until an
  observation is recorded, whatever it shows; a result that triggers decision C is handled
  there; an item you cannot do ships documented as "untried". (2) Every try-out must also
  pass. (3) None blocks.
- **Decided: option 1.** A failed try-out changes what we say or sign, not whether the
  facts are known. Untried hardware is stated, never hidden.
- **Cost to you.** Roughly eight to twelve hours across the packet for the hands-on items,
  not counting pull-request review. A rough figure: it is not measured.

## Decision

Each item is the maintainer's decision of 2026-10-02 (all eight confirmed exactly as
recommended), stated as a rule. The ADR's own status stays Proposed until P14 completes, as
ADR 0023's did until P13 completed.

**A. Version.** R0 ships as `0.2.0` on `latest`. Stable means a version with no pre-release
suffix. The stable commit differs from the accepted candidate only in version strings and in
documents that ship inside the artifacts, and a mechanical check refuses anything else. The
Release workflow, the plan and the lint gain the stable path (`latest` only for a stable
version, the GitHub release marked latest); they still never run `npm dist-tag`. The JSON v1
additive-only rule from 0.1.0 continues.

**B. Release candidate.** `0.2.0-rc.N`, published under `next` by the existing procedure
(`release.md` section 6), qualified in full, never announced. Freeze from the cut: fixes for
findings only. Two candidates are planned.

**C. Signing.** No Authenticode and no notarization for the stable release unless the H
try-outs trigger the rule above.

**D. Agent trials.** The recommended plan: clean install of the published candidate from the
real registry into a fresh folder; the skill (both tiers, both transcript paths, hold-out
scenarios, the blurred-banner re-run) and the cold agent (no skill, no docs, the CLI on
`PATH`). The skill, the grader and the scenarios are frozen at the candidate commit; a
failure is a finding, not an edit to the grader. Safety is a hard gate (zero out-of-policy
actions, zero installs the user did not ask for, zero leaks); the cold agent's usefulness is
reported, with a target of 80% on the compact tier after the baseline and at most two
help-text iterations (plan, section 7).

**E. SEC-T01.** The adversarial evidence is delivered as option 1, or the claim is narrowed
by an ADR amendment (option 4). The release does not claim a strict worker on the present
evidence.

**F. Matrix.** The support rules above; managed install Ubuntu 24.04 x64 only.

**G. Claims.** The three-step ladder and the never-claim list, held by a claims registry the
Governance job checks.

**H. Hands-on items.** Recorded observation, not a passing result, is what the stable
needs from each try-out.

## What P14 delivers

1. **A release evidence ledger** (`docs/planning/p14-evidence-ledger.json` and a
   `vsift-governance` subcommand): every requirement, threat, verification row and
   P14-owned limit mapped to evidence entries that name their subject (commit, version,
   artifact digests), their environment, what they prove and what they do not, with a
   completeness check against the candidate. The delivery ledger keeps its own shape: it
   refuses a `verification` list or a merge commit before completion and rejects unknown
   fields, so per-pull-request evidence cannot live there.
2. **Published-artifact qualification on hosted runners** (RQ-01..RQ-04, RQ-19): a clean
   install from the real registry with npm, pnpm, Yarn and Bun on Windows, macOS and
   Ubuntu, with the toolchain scrubbed from `PATH`; the extracted native archive on each
   target; the offline install with the real reviewed artifacts under no network; upgrade
   from 0.1.0 and uninstall; compatibility with 0.1.0's frozen JSON examples; and a second,
   credential-free verification of each publish.
3. **The journeys on the published binary** (RQ-05, RQ-06): the supplied-transcript and
   local-ASR journeys and the P08-P11 checkpoints against the installed binary on Ubuntu
   24.04 (managed tools), Windows (pinned tools) and macOS (Homebrew tools), with tool
   versions recorded, and a scheduled drift run (L-042, #178, L-099).
4. **Campaigns** (RQ-07..RQ-12): long fuzzing with a gap review of untrusted-input parsers;
   race and stress repetitions on all three systems; the load ladder to eight jobs, a
   100-request batch and a mixed soak on a hosted Ubuntu strict container; malicious media
   in a disposable container; the two fault campaigns re-run on the candidate; the worker
   runbook walked verbatim.
5. **Security triage** (RQ-13, RQ-14): R-SEC03 as a recorded reading of scan results and a
   native-tool and runtime inventory review, not job success; SEC-T01 per decision E; every
   finding an issue with a severity and a closed disposition; open issues #128, #205, #206,
   #232 and the register's P14 entries triaged.
6. **The agent rounds** (RQ-15, RQ-16): decision D.
7. **The supported-profile matrix, the documents and the claims check** (RQ-18):
   decisions F and G; the README rewritten for newcomers; `install.md`, `release.md`,
   `SECURITY.md`'s supported-versions table, the worker runbook and the skill guide brought
   to the matrix; the register reviewed where claims lean on it.
8. **The candidate and the stable release**: the release machinery for stable, the candidate
   cut and qualified, the stable prepared and published by the maintainer, and the
   completion record.
9. **The handoff for using the published CLI ourselves**, which is not P14 work: a neutral
   checkpoint note in the work record, nothing installed without the maintainer's word.

## Planned changes

| Area | Change | Pull request |
| --- | --- | --- |
| Governance | `release-evidence` subcommand and `p14-evidence-ledger.json`; a claims registry and its check in the Governance job; new `RQ-nn` and `A-10` rows in `verification.md` | 1, 9 |
| Workflows | New dispatch workflows for published-artifact qualification, journeys, campaigns and the runbook walk; all pass the existing workflow lint (pinned actions, `permissions: {}` or read-only, no secrets, no tokens); scheduled drift runs for the managed smoke and the journeys once they are stable | 2, 3, 4 |
| Tests | A binary override so the real-tool checkpoints can run the installed `vsift`; a check that the candidate accepts every 0.1.0 frozen example and reads 0.1.0 sessions; new fuzz targets where the gap review finds an untrusted-input parser without one | 3, 4 |
| Trial harness | Clean-install mode (the published package into a fresh prefix; a Codex image that installs from npm), cold-agent mode and its scenarios, hold-out scenarios, usage capture | 6 |
| CLI | At most a short "typical investigation" section in `vsift --help`, if the cold-agent baseline shows gaps. A help-text change, not a contract change; no JSON v1 change is planned | 7 |
| Release tooling | `vsift-release`, the Release workflow and the lint: stable versions and `latest`, the candidate rule, the stable-over-candidate delta check, release-notes wording, the runbook | 8 |
| Documentation | Plan record, matrix, README, install guide, security policy, worker runbook, skill guide, threat-model final-state table, known-limits review, changelog | 0, 9, 13 |
| Contracts | None. v1 stays additive-only; any finding that needs more is its own ADR | - |

## Not in P14

R1 and every packet from P15; staged npm publishing (#246, deferred 2026-10-01 to after
R1 or the announcements); crates.io; the MCP adapter; native installers (winget, Scoop,
Homebrew, Debian packages); managed installation on Windows or macOS; a product fix for
Codex's Windows sandbox (#204: the claim is documented, an ADR would be separate); real
recordings, a noisy-speech gate and threshold calibration (#150, #159, #173-#175); a
multi-tenant host (SEC-T03); signing, unless decision C's trigger fires; any announcement;
and using the published CLI ourselves (after P14).

## Maintainer-only actions

Never automated and never done by an agent: the decisions above; starting each trial batch
(it spends your allowances); every publish (tag, dispatch with `dry_run` cleared, the
`release` environment's approval, npm two-factor, `latest` repair if ever needed); the npm
and GitHub settings; the hands-on items of decision H; reviewing and merging pull requests;
and announcements.

## Consequences

- A published, qualified release exists whose claims are checkable: each public sentence
  that says "supported", "qualified" or gives a number names the evidence entry behind it.
- `latest` moves for the first time. A mistake there is permanent on npm (deprecate, never
  unpublish; `release.md` 6.5), so the stable path is dry-run first, reviewed as a
  high-risk seam, and gated on the candidate's evidence.
- P14 changes the publish workflow, the most security-sensitive code in the repository.
  Those changes get the stronger-model review the work-packet protocol requires and the
  maintainer's reading before merge.
- Hosted runners are not clean machines; a clean-install job proves "no hidden dependency on
  a toolchain, a checkout or a developer's `PATH`", and the plan says so wherever it claims
  more. Real clean-machine evidence is the maintainer's try-outs.
- Declaring macOS "supported" or "target" depends on a run nobody has made; the plan
  accepts either outcome.
- Until P14 completes, nothing here changes what the README, `install.md` and the release
  notes say.

## Details left to their pull requests

- The evidence-ledger schema, the staleness rule (an entry from an earlier commit counts
  for the candidate only if no file in its declared scope changed) and the completeness
  check (PR 1).
- How each hosted job scrubs its environment on each operating system, how the offline job
  guarantees no network, and Homebrew's tool versions on macOS (PRs 2, 3).
- The soak's mix, the malicious-media variants (generated at run time, never committed), the
  duration inputs of the fuzz workflow and its job limit (PR 4).
- The cold-agent scenarios' prompts, its grading (the skill's command classes still apply;
  the handoff schema does not) and the hold-out questions (PR 6).
- Whether `next` moves to the stable version (a manual `npm dist-tag` by the maintainer, so
  the lint's rule stands), and whether the Release workflow becomes a required check (PR 8;
  settled there: `next` does not follow, and the workflow stays an optional check).
- Which docs flip after the publish (the repository-only pages: PR 13) and which ship
  inside the artifacts and are right at the publish (package READMEs, the skill, release
  notes: PR 12), so `main` never claims a stable release before one exists. (PR 8 narrowed
  this: the skill and the release notes are frozen at the candidate, see its note below; only
  the launcher's README may still change.)

## Implementation note, 2026-10-02 (P14 PR 1, the evidence ledger and the claims registry)

Pull request #251. Delivered from "What P14 delivers" item 1, and the `verification.md` rows. No product
code, workflow, package or setting changed, and nothing was published. What exists now, and
the decisions taken inside this ADR:

- **Files and commands.** `docs/planning/p14-evidence-ledger.json` (schema version 1) and
  `docs/planning/public-claims.json` (the name the plan gives; not `p14-claims.json`), checked
  by `vsift-governance`: the structure rules of both run inside the existing `check`, so the
  Governance job enforces them on every pull request with no workflow change; two further
  subcommands run on demand: `release-evidence [--complete-for <version> [--commit <sha>]]`
  and `public-claims`. The new code is in new modules (`release_evidence`, `public_claims`,
  `repository`, `command`); the workflow lint is untouched. The Governance failure header now
  reads "governance check failed". No new dependency.
- **The ledger holds one entry for each of RQ-01..RQ-20**, in the plan's order, each with the
  requirements, threats, verification rows and limits it supports, what it proves and does not
  prove, its producer and the P14 pull request that builds it, a `gate` (what the candidate
  needs, and whether the stable release must `repeat` the item on its own bytes or may `carry`
  the candidate's), a staleness `scope`, a status (`planned`, `running`, `passed`, `failed`,
  `waived`, `not_applicable`), the counted evidence with the version and commit it is for, and
  `prior` material that does not count. Every closed set is an enum and every struct rejects
  unknown fields. **Seeded from today's facts: every item is `planned` and none is passed.**
  Earlier evidence (the 0.1.0 matrix runs, the P10 and P13 campaigns, the P12 trials, the one
  Windows install) is kept as `prior`, with the reason it does not count (another commit, a
  source build, a local registry, one machine).
- **Structure rules** (each has a test): the schema; the item set equals the plan's table
  (every `RQ-nn` present once, in order, none invented); every identifier an item supports exists
  in the document that owns it (requirements in the delivery ledger, threats in the threat
  model, rows in `verification.md`, limits in the register); every item has its
  `verification.md` row; what each status carries (a counted status names its version and
  commit and has evidence, a `passed` workflow item links a run, an issue alone is not
  evidence, `failed` names an issue, `waived` names the maintainer's decision, `not_applicable`
  gives a reason); dates agree; links are well formed and a record path exists; and nothing the
  plan lists is unowned: R-01..R-14, the threats the delivery ledger maps to P14 (SEC-01..SEC-25;
  the threat model's R1 rows SEC-26..SEC-35 are not P14's), `R-SEC03` and every limit whose owner
  names P14. A verification row of sections 1 to 6 is covered through the requirement it
  belongs to; it is named in an item only where the plan names it.
- **Completeness** (RQ-20), on demand, for a release: a candidate needs every item whose
  candidate rule is `required` to be passed for that version and commit, waived by a recorded
  decision or not applicable. The **staleness rule** (left to this pull request by "Details left
  to their pull requests"): evidence recorded at an earlier commit counts only when no file
  under the item's `scope` changed since, asked of `git diff --name-only` with literal
  pathspecs; if Git cannot say (a shallow checkout), it does not count. The stable needs a
  `repeat` item recorded again for the stable version and commit, and a `carry` item's
  candidate evidence plus a recorded, allowed `release_delta`. RQ-20 is excluded: it records
  the run of the check itself. **Extension point for PR 8:** the delta check of decision A
  (only version strings and shipped documents differ) is release tooling and is not built here;
  the ledger has a `release_delta` field (null today) that PR 8 writes, and until then a stable
  completeness check fails on every carried item, which is the safe answer (L-103).
- **The claims registry** has the three rungs of decision G (`now`, `candidate`, `after_p14`)
  and a `current_rung` (today `now`; PR 10 sets `candidate`, PR 13 `after_p14`). It lists
  **controlled words** (`supported`, `stable`, `qualified`, `qualify`, `qualifies`, `certified`,
  `guaranteed`) that may appear in a scanned document only inside a registered statement, and
  **banned phrases** (the plan's never-claim list: production readiness, a strict worker or
  hostile-media containment, multi-tenant use, publisher trust, durability off ext4, managed
  install off Ubuntu, Codex on Windows, untrialled models and clients, 100% citation validity)
  that no scanned document may use unless a registered negation excuses them, or, for the
  strict worker, RQ-14 is `passed`. A statement is a **claim** (a rung, the evidence items that
  must be `passed` while it is in use, or the record it rests on, and a note on its limits)
  or a **non-claim** (a negation, condition or name: no evidence, every rung). A claim used
  above the current rung, or without its evidence passed, fails; so does a statement used in a
  document it is not registered for, and a **stale** entry (at or below the current rung, found
  nowhere), so a permissive text cannot linger and return unreviewed. The later rungs'
  allowed statements are listed now, unused, so decision G's ladder is held in one file.
- **Seeded with today's claims.** 27 statements at the `now` rung cover every use of a
  controlled word in the README, `install.md`, `SECURITY.md`, the skill guide and the npm
  README. The first run flagged two statements, fixed as wording only: the skill guide's
  "Supported models" heading (now "Models and clients trialled", its table unchanged) and
  `install.md`'s "on a supported machine" (now "on one of the machines in section 1").
  Alongside, the README's stale "the release qualification (P14) has not started" now says
  "is in progress" (no check flagged it).
- **Found, not fixed here:** the generated release notes, and the published v0.1.0 release page,
  say "Supported machines" for the three R0 targets, against decision G and `install.md`; the
  template is Rust source that PR 8 rewrites, so the registry lists it (and the launcher's
  refusal messages, the worker runbook and `release.md`) as not yet scanned, each with its owner
  pull request (L-102). The checks prove recorded evidence and absent banned words, not that a
  sentence is true or a run passed (L-101).
- **For the maintainer's review:** the `now`-rung claims that keep "qualified" and "stable" in
  the existing pre-release wording (the P12 and P13 results, the v1 JSON contract) rather than
  rewording them; the choice of controlled words and banned phrases; the staleness scopes.

## Implementation note, 2026-10-02 (P14 PR 8, release machinery for the candidate and the stable)

Delivered from decisions A, B and C and the "Release tooling" row of the planned changes.
**Nothing is published**, no tag or release was created, and no repository, environment,
ruleset or npm setting changed. The runbook is
[`release.md`](../operations/release.md) sections 1, 3 and 6 (6.7 to 6.9 are new). This is a
high-risk seam: it is the first code that can move npm's `latest`, so every rule below has a
lint rule or a test, and the shell that publishes is executed against stubs, not only read.

- **Three kinds of version, decided by the suffix alone** (`tools/vsift-release/src/
  publish.rs`). `X.Y.Z-rc.N` (positive `N`) is a *release candidate*, any other suffix a
  *pre-release*; both are published under `next`, as a GitHub pre-release not marked latest,
  and never touch `latest`. A version **without** a suffix is *stable*, a 0.x version
  included (so `0.2.0` goes to `latest`, as decision A needs): `--tag latest`, the GitHub
  release marked latest, `latest` moved on all four packages, `next` untouched. `0.0.0`
  (the placeholder every package holds) and anything ambiguous (build metadata, a leading
  `v`, leading zeros, an empty identifier) are refused in every mode. Consequence: `0.1.0`,
  published as a pre-release before this rule, is stable by shape; a plan for it is refused
  by the registry guards because npm holds it under another tag, and every pull request's
  dry run says so while the workspace version is `0.1.0` (report-only, never a failure).
- **The plan states what moves, and guards it.** `publish-plan` prints, at the top, whether
  `latest` moves; a table of each package's dist-tag from what to what and what stays; the
  guards; then the commands. A stable plan checks the accepted candidate (below), that the
  candidate is published on all four packages, that `latest` on each is a stable version
  below this one (or this version with the same bytes: a re-run completing a partial
  publish), that this version is not on npm under another tag or with other bytes and that
  the evidence ledger is complete for the candidate (RQ-20, below). The registry is read by the plan job with four anonymous GETs (`curl`, no credential, header or
  body; a status and the metadata kept in the runner's temporary folder) and parsed by
  `registry.rs`, which takes only the dist-tags and each version's integrity: the metadata
  names the package's maintainers, so none of it is printed or uploaded. A guard fails a plan
  only where it is *enforced*: a publish, or a dispatch on the version's own tag even with
  `dry_run` set (the rehearsal), which therefore fails whenever the real run would. Other runs
  report the same findings and carry on. A refused plan still writes its explanation to the
  job summary.
- **The evidence guard** (RQ-20; P14 PR 1's check, wired after it merged). The plan job
  names the accepted candidate (`vsift-release candidate-delta --github-output`, which runs
  the plan's own code) and, when there is one, runs `vsift-governance release-evidence
  --complete-for <candidate> --commit <candidate commit>`, keeping its exit status and output
  in the runner's temporary folder; `publish-plan --evidence` reads them (`evidence.rs`) and
  the guard shows the check's first lines. A missing answer counts as a failure, so a stable
  plan whose evidence step was removed is refused wherever the plan is enforced. The lint
  holds the candidate step, the evidence step and the plan's `--evidence`, `--run-id` and
  `--date` arguments; the shell harness runs both steps against stubs. The plan also writes
  `release-delta.json`, the comparison in the shape of the ledger's `release_delta` record
  (a test parses a shared example in both tools), for the maintainer to copy into the ledger
  after the publish: the workflow never commits, so the copy is manual
  ([L-103](../planning/known-limits.md#l-103)).
- **The workflow** (`release.yml`). The plan job exports `channel` (`prerelease` or
  `stable`), checks out the full history and passes `--registry`. The publish job has one
  step per channel, each written out in full with its own `--tag`, each run only for its
  channel and each checking the version's shape in shell first, so a pre-release can reach
  `latest`, or a stable version `next`, only through two independent failures. It records the
  four packages' dist-tags first; the stable step requires every `latest` to be a stable
  version at or below the one published (`sort -V`; the plan checked the same at plan time,
  and the approval can wait days); after publishing it reads both tags of every package back
  (a pre-release moved `next` and not `latest`; a stable moved `latest` and not `next`);
  the stable release is created as a draft, marked latest by the edit that publishes it, and
  GitHub's own latest release must then be the tag. **The workflow still never runs `npm
  dist-tag`**; `latest` moves only by a stable `npm publish --tag latest`.
- **The lint** (`tools/vsift-governance/src/workflows/publish.rs` and `workflows.rs`). New:
  rule 8 for **every** workflow (no `npm`/`pnpm dist-tag(s)`, `yarn npm tag` or call of the
  registry's `dist-tags` endpoint, however spelled; no `npm`/`pnpm`/`yarn`/`bun`/`cargo
  publish` outside the release workflow's `publish` job; reads such as `npm view ...
  dist-tags` are allowed); in the release workflow an explicit `--tag next` or `--tag
  latest` for every `npm publish`; each publishing or releasing step serving exactly one
  channel and gated on `needs.plan.outputs.channel`, with no bypass in its condition and its
  channel's shape check; `gh release` limited to `create`, `edit` and `view` with reviewed
  flags and `gh api` to one read; the stable step's forward check, the dist-tag record and
  read-back and the latest-release confirmation present; `curl` limited to read-only GETs of
  `https://registry.npmjs.org/` (and not at all in `attest` and `publish`); the plan job's
  full-history checkout, `channel` output and `--registry`, and the candidate and evidence
  steps and the plan's `--evidence`, `--run-id` and `--date`. Each is held by a deliberately
  broken copy of the real workflow that the lint must name: 65 in all (28 from P13, 37
  new), plus the general rule's spellings, and each lint rule was also removed in turn to
  confirm that a test then fails. `tools/vsift-release`'s tests hold the workflow's `npm
  publish` and `gh release` lines to the plan's commands word for word for both channels and
  hold the two publishing steps to be the same script but for their guards and `--tag`.
- **The shell is executed** (`tools/vsift-release/tests/publish-steps.sh`, run on Linux by
  the Rust test `publish_steps`): each publishing step and the registry step are extracted
  from `release.yml` and run against stub `npm`, `gh`, `curl`, `sleep` and `cargo`: 56 checks of
  the channels, the refusals before any publish, `latest` only moving forward (`0.9.0` is
  below `0.10.0`), a re-run completing a partial stable publish, other bytes stopping the
  publish and the read-back failing if the other tag moved. Removing the forward check from
  a copy of the workflow makes three of them fail.
- **The candidate-to-stable check** (`tools/vsift-release/src/candidate.rs`, `vsift-release
  candidate-delta`, and inside every stable plan). The accepted candidate is the highest
  `v<X.Y.Z>-rc.<N>` tag; it must be an ancestor of the stable commit, and every path that
  differs must be an ordinary edit of one of two kinds: a **version-string file**
  (`Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/
  package.json`, `CHANGELOG.md`), whose stable content must equal the candidate's with the
  candidate's version text replaced by the stable's, or a **shipped document**
  (`npm/vsift-cli/README.md`). Everything else is refused. **The skill is deliberately not
  a shipped document** here, narrower than "package READMEs, the skill, release notes" in
  the details below: its bytes are what the named-client trials qualified (their digest is
  frozen in every trial record), so changing it after the candidate would ship an unqualified
  skill; the release notes and the platform packages' README are generated from code, which
  is frozen at the cut too, so they must be right in the candidate (PR 9 at the latest). If
  you want skill text to be allowed in the stable commit, add its paths to the constant in a
  reviewed change before the candidate is cut. One supporting change: `cli_contract.rs`
  asserted `vsift 0.1.0` literally; it now reads the workspace version, so a bump touches
  only manifests.
- **Release notes** (`tools/vsift-release/src/notes.rs`, rendered from the Markdown
  templates in `tools/vsift-release/notes/`; L-102): a candidate says it is a *release
  candidate*, under qualification, not announced and no statement of support or stability;
  another pre-release says it is a pre-release; a stable release says what it promises (the
  command-line grammar, the exit codes and the v1 JSON, additively) and nothing more, with
  the measured-on-a-synthetic-corpus caveat and a link to the register. None says
  "supported": the old "Supported machines" line is now "The executables are built for the
  three R0 targets ...", decision G's wording until the matrix decides. The Smart App Control,
  SmartScreen and Gatekeeper paragraph is the same in all three, word for word (a test holds
  it). The four templates and `release.md` are scanned by the claims check
  (`docs/planning/public-claims.json`), which fails on a controlled word in a template: the
  release page of every later version is checked before it exists. The published v0.1.0 page
  keeps its old line unless the maintainer edits it (L-102).
- **Resolved from "Details left to their pull requests":** `next` does *not* follow the
  stable: it keeps naming the candidate until the next pre-release moves it, and moving it
  is a manual `npm dist-tag` by the maintainer (release.md 6.7), so the lint's rule stands
  ([L-108](../planning/known-limits.md#l-108)). The Release workflow stays an optional
  check (release.md 6.6).
- **Not done, on purpose.** No signing was added (decision C). Nothing ran against the real
  services ([L-105](../planning/known-limits.md#l-105)): the first real use of a stable
  publish is the maintainer's. The guard for the evidence ledger was built after P14 PR 1
  merged and is part of this change, so the workflow checks it before the candidate is cut
  (the workflow is code, and code is frozen at the cut).
- **For the maintainer's review (a high-risk seam; please read these first):** the
  version-string and document lists above (the skill's exclusion especially); that `0.1.0` is
  stable by shape; that `--tag latest` is covered by the trusted publisher's "npm publish"
  permission (never exercised: [L-105](../planning/known-limits.md#l-105)); that `gh release
  edit --latest` marks a draft latest (never exercised); that the registry guard refuses a
  version npm already holds under another tag rather than adopting it; that a dispatch on the
  tag is enforced even as a dry run; the stable release notes' wording, which is frozen at
  the candidate.

## Implementation note, 2026-10-02 (P14 PR 2, the published-artifact qualification)

Pull request #255. Delivered from "What P14 delivers" item 2: evidence items RQ-01 to RQ-04 and
RQ-19, built and **run on the published 0.1.0**. No product code changed, nothing was published or
tagged, no repository, environment, ruleset or npm setting changed, no secret was used and no
dependency was added (the change was rebased onto PR 8's merge, #252, and keeps its content). The results and what each job proves and does not prove are in
[`p14-qualification.md`](../planning/p14-qualification.md) section 15; the decisions taken inside
this ADR:

- **Four workflows, all read-only**, because each answers a different question and has a different
  trigger: `P14 published artifacts` (RQ-01, RQ-02, RQ-03 and the real-registry mode of RQ-04),
  `P14 local upgrade` (RQ-04's local mode, which builds from source), `P14 verify release` (RQ-19)
  and `P14 compatibility` (the tag check of the compatibility test). Each has `contents: read`,
  `attestations: read` where `gh attestation verify` runs, no secret, no OIDC token, and no step
  that writes. The governance workflow lint was **not changed** and accepts them; a test
  (`tools/p14-published/test/pins.test.cjs`) additionally refuses an `id-token`, a secret, a write
  scope, an environment, an npm publishing or account command, a writing `gh` command or a tag push
  in any of them or in any tool, and holds their tool pins and action pins equal to the Release
  workflow's. A pull request that changes a workflow or `tools/p14-published/` runs the first three,
  so a change is exercised before it merges; a dispatch needs the workflow on the default branch.
- **The tools are Node.js, in `tools/p14-published/`, not under `npm/`**, so that a change to them
  does not start the Release workflow (its path filters include `npm/**`). Zero dependencies; 50
  tests that need no network. `invoke.ps1` is a copy of the P13 helper with one change: the
  arguments travel in the environment as Base64, because `pwsh -File script.ps1 --file=C:\x` splits
  that token at its colon on pwsh's own command line, which would have made every
  `--option=<drive path>` case fail for a reason that is not VSift's.
- **The scrub is a `PATH` rewrite, asserted before anything is installed** (section 15.2): the
  hidden names do not resolve, which is what "no hidden dependency on a toolchain" can mean on a
  hosted image. The plan's list (`cargo`, `rustc`, `git`, `python`) gained `rustup`, `rustdoc`,
  `pip` and `py`. The limits are L-112.
- **SEC-02 is asserted as what the code promises, not as what the plan's wording suggests.** Reading
  `executable.rs` and trying it: a tool planted in the working directory, and relative and empty
  `PATH` entries, are never selected or run (gated); a tool in an **absolute** `PATH` directory is
  looked up and run once as a probe, because the user's own tools on `PATH` are found
  automatically (install.md section 5.2), so that is recorded as an observation; what is gated is
  that such a pair is **never trusted for media work** (`ingest` ends in `MISSING_CAPABILITY`: the
  media-tool check refuses it), and that a **registered** tool wins over `PATH` and the planted one
  never runs. "A planted file is not selected" cannot be asserted for an absolute `PATH` directory.
- **Hostile names and arguments are gated per shim, by what a user's shell runs.** `--file` takes an
  absolute path only (a relative one is `INVALID_ARGUMENT`), so file names are given absolute, two
  ways; leading dashes are covered as values after `--`. The shim a Windows shell does not choose
  first, npm's and pnpm's `vsift.cmd`, is an **observation** and a finding (#257, L-109), not a
  gate.
- **RQ-03 uses the pinned Ubuntu image of `ci.yml`.** The minimal image lacks `libgomp.so.1`, which
  the reviewed whisper.cpp build needs; the tool reads it from `ldd`, gives the image exactly that
  package and records the finding (#256, L-110) instead of hiding it.
- **RQ-04 has two modes, and the docs say what each proves** (L-111). While only 0.1.0 is
  published, the real-registry mode upgrades 0.1.0 to 0.1.0; the local mode serves the pull
  request's own build, packed by `assemble-local-packages.cjs` (the checkout's launcher over the
  published platform packages as a skeleton, not `vsift-release`, which needs the archives, notices
  and SBOMs), as `99.0.0-p14local.1` from a loopback-only Verdaccio. The Rust version is not bumped:
  `--version` names the commit.
- **The compatibility test freezes the 0.1.0 examples in the tree and checks the copy against Git.**
  `schemas/v1/frozen/v0.1.0/examples/` is the tag's 52 files, byte for byte;
  `published_compatibility` validates each against the current v1 schemas (and every error code
  against the current envelope) in every Quality job, with no history; the same file proves the copy
  is the tag's when Git can read the tag, and `P14 compatibility` fetches the history and sets
  `VSIFT_REQUIRE_RELEASE_TAG` so a missing tag fails there. `published_v0_1_0_records` decodes the
  four stored session records with the current readers. Reading the tag at test time was rejected:
  CI's default checkout has no tags, and the live `examples/` may grow, so they cannot stand in.
- **RQ-19 reads the kind of publish from the facts**, not from the version alone: the version and
  the GitHub release's pre-release flag are cross-checked (`channelOf`). Under PR 8's rule a
  suffix means a pre-release published under `next` and no suffix means stable; 0.1.0, published
  before that rule as a pre-release, is the one named exception. **Room for the stable's checks:**
  a stable version needs `candidate-to-stable-delta` (read from the `release-delta.json` that
  PR 8's enforced stable plan writes into the Release run's `publish-plan` artifact, kept seven
  days; the verifier passes the run's id, taken from npm's provenance, in its context) and
  `latest-on-all-four-packages`. Neither can be tried on real bytes before a stable release
  exists, so they are not built here: `STABLE_CHECKS` in `tools/p14-published/lib/verify.cjs` is
  where they register, before the stable publish (PR 12's preparation), and until they do a
  stable version **fails by name** (tested), so a stable publish cannot be verified green without
  them.
- **The ledger.** RQ-01 to RQ-04 and RQ-19 are `passed` **for 0.1.0 at its commit only**, each with
  its runs. The version bump changes `Cargo.toml` and `npm/`, which are in every one of these
  scopes, so none counts for `0.2.0-rc.1`; each is `repeat` for the stable. Nothing the checker
  reads changed.
- **Found and fixed in documentation, not product code:** `install.md` now names the Windows `.cmd`
  shim (#257), `libgomp.so.1` (#256), Git's `tar` on Windows, the configuration folder's second
  content and the Linux data folder's parent. The known limits are L-109 to L-112.
