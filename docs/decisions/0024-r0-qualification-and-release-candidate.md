# ADR 0024: R0 qualification and the release candidate

- Status: **Proposed** (2026-10-02). This is the P14 plan. The maintainer has not yet
  confirmed it: the lettered decisions below are recommendations until they do, and P14
  stays `planned` in the delivery ledger until then (governance rule 10). Like ADR 0021,
  ADR 0022 and ADR 0023, it becomes Accepted when P14 completes.
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
   Control, no Gatekeeper browser download.
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

## Decisions for the maintainer

Eight decisions, in plain English. Each has the options, a recommendation with the reason,
and what it costs you. The recommendation is what the rest of this ADR and the plan assume.

### A. Which version is R0, and how does a release become "stable"?

- **Options.** (1) `0.2.0` on npm's `latest` tag, promising the command-line grammar, exit
  codes and v1 JSON (additive-only since 0.1.0) and nothing else; 1.0.0 is chosen later. (2)
  `1.0.0` now, with the same promise under SemVer's "1". (3) Stay on 0.x pre-releases under
  `next` and never move `latest`.
- **Recommendation: 1.** Everything measured about accuracy comes from a synthetic corpus,
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
- **Recommendation: 1.** The clean install "from the real registry" needs a published
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
- **Recommendation: 1, with a trigger agreed now.** If the try-outs of decision H show that
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
- **Recommendation: Recommended.** It meets governance rule 11 on both transcript paths with
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
- **Recommendation: 1, with 4 as the fallback.** The worker host is in R0's scope and
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
- **Recommendation: 1.** A cell is supported when the published packages install from the
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
- **Recommendation: 1.** Allowed now: the existing pre-release wording, no "supported" and
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

- **Your hands.** Read your Smart App Control setting (a minute: Settings, Windows
  Security, App and browser control); a Smart App Control try-out needs a Windows 11 machine
  or virtual machine where it is On (we do not know what state a fresh machine will show,
  so the try-out records it); a browser-download try-out needs a Mac with macOS 15 (unknown
  whether you have one); starting each agent-trial batch; the A-09 blurred re-run (inside
  decision D); reviewing the SEC-T01 fixture (E); each publish session (B, A); one pass over
  the register entries the public claims lean on (about thirty, listed in the plan) and the
  readings in `memory/TODO.md`; merging the pull requests.
- **Options for what blocks the stable.** (1) The try-outs block the stable only until an
  observation is recorded, whatever it shows; a result that triggers decision C is handled
  there; an item you cannot do ships documented as "untried". (2) Every try-out must also
  pass. (3) None blocks.
- **Recommendation: 1.** A failed try-out changes what we say or sign, not whether the
  facts are known. Untried hardware is stated, never hidden.
- **Cost to you.** Roughly eight to twelve hours across the packet for the hands-on items,
  not counting pull-request review. A rough figure: it is not measured.

## Decision

Each item is the recommendation above as a rule. All stay proposals until the maintainer
confirms them.

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
  the lint's rule stands), and whether the Release workflow becomes a required check (PR 8).
- Which docs flip after the publish (the repository-only pages: PR 13) and which ship
  inside the artifacts and are right at the publish (package READMEs, the skill, release
  notes: PR 12), so `main` never claims a stable release before one exists.
