# Changelog

All notable changes to VSift will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0-rc.2] - 2026-10-06

**This is a release candidate, under qualification.** It is the second candidate for `0.2.0`, the release that ships R0
(ADR 0024, decisions A and B), and it replaces `0.2.0-rc.1` as the candidate. It is published under the npm tag `next` and
never under `latest`, it is not announced, and it is no statement of support or stability. **`0.2.0-rc.1` stays published:**
npm never lets a published version go, and its tag and release page stay too, so it is superseded, not withdrawn. **Every
result recorded for `0.2.0-rc.1` is superseded:** two of its findings are fixed here, so what they touch is run again on this
candidate (P14 PR 11, repeated; [`p14-qualification.md`](docs/planning/p14-qualification.md) section 25), and the release
evidence ledger ([`p14-evidence-ledger.json`](docs/planning/p14-evidence-ledger.json)) says where each item stands and what it
does not prove.

### What changed since 0.2.0-rc.1, in plain English

**If you use VSift.** Two rare failures are fixed, both found by the qualification of the first candidate. Nothing else in the
program changed: no command, option, JSON field, failure code, help text, agent skill or reviewed tool did.

- **On Windows a command that reads a session while another command publishes to it is no longer told the stored data is
  damaged because one of its own attempts was slow** (#314, fixed by #318). The stress run on a loaded hosted machine saw it
  once in 140,721 reads. The retry that exists for this case counted its half second from before the first attempt, so an
  attempt that itself took longer used the budget up and was not retried; the budget now counts from the first failed attempt.
  One related case is not covered and is a new known limit ([L-136](docs/planning/known-limits.md#l-136)): on Windows a file
  another program holds without read sharing (a scanner, an indexer, a backup tool) is still answered `INTEGRITY_FAILURE` at
  once. It was never seen in a campaign.
- **`vsift ingest` of a video over the 20 GiB limit answers `INVALID_SOURCE` again, as 0.1.0 did, whatever the free space**
  (#310, fixed by #319). The first candidate answered `STORAGE_IO` when the video was also larger than the drive, which changed
  a published failure code; version 1 allows additions only. The answer for a video within the limit that does not fit is
  unchanged.

**If you test or maintain VSift.** The two session-root race tests wait 60 seconds instead of five, so that a documented
refusal under artificial load is not counted as a failure of the test (#312; the product is unchanged and
[L-135](docs/planning/known-limits.md#l-135) is narrowed to that one event). The candidate-to-stable check already compares a
stable commit with the **highest** candidate tag, so it compares the stable release with this candidate, not with the first
one (`release.md` 6.8 and 6.11).

### What has not been shown

- **Nothing has been run against this candidate's published bytes.** The evidence recorded for `0.2.0-rc.1` does not count
  for it where the code, a version string or a document in an item's scope changed, which is almost everywhere
  (`release-evidence --complete-for 0.2.0-rc.2` says which items and why). The hosted campaigns, agent-trial batches 2 and 3
  (batch 2 ran on the first candidate: 34 runs, four readings still waiting) and the try-outs on a second Windows machine are
  run again on this candidate. RQ-08 (race and stress) was `failed` on the first candidate; whether the fixed reader holds up
  is exactly what the repeat tests. **#312 (a root creation that gave up waiting once in 1,500 loaded repetitions) is not
  fixed**: its cause is not shown ([L-135](docs/planning/known-limits.md#l-135)).
- **FFmpeg.** Unchanged since the first candidate: the reviewed build that `setup install` downloads (BtbN, 2026-08-31) has the
  fixes for 46 of the 47 scan records read ([L-122](docs/planning/known-limits.md#l-122)), and the re-pin to the 2026-10-31
  build is planned for after `0.2.0` ([L-132](docs/planning/known-limits.md#l-132)).
- **Unsigned executables** have not met Windows Smart App Control or macOS Gatekeeper on a real machine
  ([L-098](docs/planning/known-limits.md#l-098)).
- **A synthetic corpus and a synthetic voice** only; nothing has been tried on a real recording
  ([L-020](docs/planning/known-limits.md#l-020), [L-022](docs/planning/known-limits.md#l-022)).
- **Not claimed:** a strict worker that contains a hostile decoder (decision E option 4, [L-068](docs/planning/known-limits.md#l-068)),
  durability off Ubuntu 24.04 on ext4, managed installation outside Ubuntu 24.04 x64, Codex on Windows.

### Fixed

- **A reader that overlaps a writer's rename is no longer answered `INTEGRITY_FAILURE` because one of its own attempts was slow**
  (#314; found by the P14 stress campaign on `0.2.0-rc.1`: one read in 140,721, in one of 200 repetitions, on a hosted Windows
  Server 2025 runner, none on Ubuntu or macOS). **Cause, shown.** A reader that opens a file a writer replaces by rename (the commit
  pointer, the chain checkpoint, a job record) can open the old file just before the rename and find it without a link just after;
  it tries again for up to 500 ms. That 500 ms was measured from before the first attempt, so an attempt that itself took longer
  than all of it used the budget up and its failure was not retried at all, and the read path reports that `NotFound` as damage
  (a job record, as a job with no record). The hosted runner has four CPUs, and with every one busy a thread was left unscheduled
  for seconds: in 600 repetitions of the old code (with diagnostics added) one read failed in this way, with the campaign's own
  message (`1 of 3389 reads failed`, `[IntegrityFailure]`), after a single attempt that took 4.4 s; in 900 repetitions of
  an earlier form of the fix (which made eight retries whatever the clock said; the form shipped here counts the budget from the
  first failed attempt and has not run on a hosted runner) seven attempts took over half a second (up to 4.9 s) and ended in the
  same `NotFound`, each was retried, and none of the 900 failed. **Change.** The budget is counted from the first failed attempt. The worst case for a file that really is
  missing is the first attempt, then the 500 ms, then the one attempt under way when it runs out. **What still answers
  `INTEGRITY_FAILURE`:** a hard link or a file that is not a regular file; a file still missing, or still refused with access
  denied on Windows, when the budget is spent; an I/O error the read path does not recognise as a failure of the storage,
  **including, on Windows, a file another process holds without read sharing (os errors 32 and 33), which is answered at once and
  not waited for** (new known limit [L-136](docs/planning/known-limits.md#l-136); not observed in any campaign); and an attempt that
  stalls across the deadline of the retry, which needs a replacement and then a stall in the retry. No published failure code,
  schema or field changed. **Tests.** The retry policy takes its clock through a small seam, so the tests make an attempt take
  longer than the budget on a clock they move and wait for nothing: a stalled first attempt is retried; a file that stays missing is
  still reported after the budget (30 sleeps of a millisecond on that clock); with every attempt slow the wait is the first attempt
  and one retry; any other error is returned at once. The first two fail on the old policy. A test run now prints the I/O error
  behind an `INTEGRITY_FAILURE` (the campaign's failure said only `[IntegrityFailure]`). **What is not shown.** The one failure of
  the campaign run did not record its cause: that it was this one is an inference from the same failure reproduced with
  diagnostics, with the same test, the same message and the same runner. ADR 0020 has a dated note.
- **The two session-root race tests no longer count the documented five-second refusal as a failure under artificial load; the
  product is unchanged** (#312; test and documents only). One of 1,500 repetitions, with eight busy processes on four CPUs of a
  hosted Windows runner, ended with a child refused `BUSY` ("session root is still being created by another process") after the
  five seconds the contract allows. It was not reproduced in 3,200 further repetitions with up to eight busy processes per CPU (the
  slowest child took 1.4 s, and none was refused when the children waited 60 s), so a creator that was not scheduled for five
  seconds is the explanation that fits and a defect is not shown. The race tests now wait 60 s (the product's own bound keeps its
  unit tests). **What is not shown:** the cause of the one event; **L-135** is narrowed to it (its reader half, #314, is the fix
  above).
- **`vsift ingest` of a source over the 20 GiB limit answers `INVALID_SOURCE` again, whatever the free space** (P14, #310;
  found by the P14 malicious-media campaign on the published `0.2.0-rc.1`: a sparse 30 GiB file answered `STORAGE_IO` in 0.1 s
  where 0.1.0 said `INVALID_SOURCE`). The cause was the room check added for #266, which ran before the source-size limit,
  so a source that was both over the limit and larger than the free space got the "no room" answer. That changed a published
  failure code, which is not additive within v1 (known limits [L-126](docs/planning/known-limits.md#l-126) and
  [L-127](docs/planning/known-limits.md#l-127)); the maintainer had accepted it for the first candidate, and it is fixed for
  the second. Now, for a desktop root, a source over the limit is not asked for room, so the limit answers first: `INVALID_SOURCE`
  (exit 3), at once, nothing copied, 0.1.0's answer. **Everything else is unchanged:** a source within the limit that does not fit
  the root is still refused before the copy as `STORAGE_IO` with the remediation of #266 (L-127), a worker workspace still checks
  its 1 GiB reserve first and answers `RESOURCE_LIMIT`, and no field, code or schema changes. L-127 and L-126 no longer carry a
  known deviation, and [L-134](docs/planning/known-limits.md#l-134) is narrowed to what remains of #310: the campaign's no-room case
  names the tmpfs mount point as the session root, so it never reaches the check (a defect of the case, tracked under #266). Tests:
  the engine's room decision with the free space injected (a source over the limit is not asked, however little there is; one within
  the limit up to and including it is; any other failure of the check stays a storage failure), the mapping of the size limit to
  `INVALID_SOURCE`, and, on Unix, the binary: a sparse 4 TiB source answers `INVALID_SOURCE` with exit 3 and copies nothing (it
  answered `STORAGE_IO` before this change), and a source a little larger than the free space of a small filesystem still answers the
  no-room `STORAGE_IO`.

### Work record since the first candidate (nothing shipped changes)

**Work record only (P14 PR 11b); nothing shipped changes.** Two documents for the maintainer's hands, prepared and not run: the
RQ-17 try-out sheet for the second Windows 11 machine ([`docs/planning/rq-17-tryout-sheet.md`](docs/planning/rq-17-tryout-sheet.md):
Smart App Control through npm and through the archive, a clean-machine install, the guide's first investigation) and the checklist
for agent-trial batches 2 and 3 ([`docs/planning/p14-batch-2-3-checklist.md`](docs/planning/p14-batch-2-3-checklist.md)). No code, tool,
workflow, schema or setting changed.

**Installation guide only (P14 PR 11c); nothing shipped changes.** [`docs/operations/install.md`](docs/operations/install.md) said
the release candidate would be published later and that `@next` installed 0.1.0 until then. The candidate was published on
2026-10-05, so the guide now says so, and its upgrade section says that the published 0.1.0 has been upgraded to the published
0.2.0-rc.1 over the real registry on hosted runners (npm only). The guide is one of the few documents the stable commit may change.

**Work record only (P14 PR 11a); nothing shipped changes.** The hosted evidence on the published `0.2.0-rc.1` is recorded in the
evidence ledger and in [`docs/planning/p14-qualification.md`](docs/planning/p14-qualification.md) section 24.2: the second verification
of the publish, clean installs, archives, the offline install, the upgrade of the published 0.1.0 to the candidate, the journeys on three
systems, long fuzzing, the load and soak, the runbook walk, the scan reading and the two fault campaigns passed; the stress run found two
rare Windows failures that are open (#312, #314, [L-135](docs/planning/known-limits.md#l-135)); the malicious-media run found one
corner case (a source over the limit and larger than the free space answers `STORAGE_IO`, where 0.1.0 said `INVALID_SOURCE`: #310,
[L-134](docs/planning/known-limits.md#l-134)), which the maintainer accepted for R0 and recorded as a waiver. The scan reading of the
day is [`p14-scan-reading-2026-10-05.md`](docs/planning/p14-scan-reading-2026-10-05.md). The P10 durability campaign met its acceptance numbers (its hosted verdict job never got a runner: #316).

**Work record only (P14, #317).** Agent-trial batch 2 ran on the first candidate on 2026-10-05: its 34 bounded records, summary and
the maintainer-side reading are committed ([`batch-2-reading.md`](docs/planning/p14-agent-trials/batch-2-reading.md)); in the change
that cuts the second candidate they moved to `docs/planning/p14-agent-trials/batch-2-rc.1/`, so that `batch-2/` holds only its freeze for the
repeat on this one.

### Changed

- **The release candidate `0.2.0-rc.2` is cut** (P14 PR 10, repeated, 2026-10-06; the bump, the pointers, the freeze and the maintainer's
  runbook; **nothing is tagged or published**: the maintainer does that by `docs/operations/release.md` section 6.11).
  **Version:** `0.2.0-rc.1` becomes `0.2.0-rc.2` in the workspace and fuzz manifests and lockfiles and in the launcher's manifest (its
  three optional dependencies too), and in no other version-string file; the user guide names the release `0.2.0`, so its marker and its two
  generated pages are unchanged, and its 40 examples, run again, match. **Pointers:** the README and the installation guide name `0.2.0-rc.2`
  as the release candidate under qualification (the rung stays `candidate`; the guide says that `@next` installs the first candidate until
  the second is published), and the guide's first page, the developer documents, the try-out sheet and the agent-batch checklist follow.
  **The candidate-to-stable check needed no change:** it compares the stable release with the highest `v0.2.0-rc.<N>` tag, so with both
  candidates tagged it compares with `v0.2.0-rc.2` (a test builds exactly that), its allowed lists are not widened, and the release
  workflow, its lint and its shell are untouched; the by-hand backstop in `release.md` 6.7 and in L-107 names `v0.2.0-rc.2`.
  **Runbook:** new section 6.11 (`next` moves from `0.2.0-rc.1` to `0.2.0-rc.2`; read the dry run's published plan and compare its
  checksums with the publish run's, as the two runs of the first candidate did, byte for byte; the upgrade is tried from `0.1.0` and from
  `0.2.0-rc.1`; deprecating the first candidate is optional and comes last); 6.10 stays as the record of the first candidate.
  **Agent trials:** the freeze of batches 2 and 3 was written again and binds the same digests as the first candidate's (nothing frozen
  changed, so `committed_freeze` is unchanged; only the file's `commit` field differs); the first candidate's batch-2 records moved to
  `docs/planning/p14-agent-trials/batch-2-rc.1/` so that the batch can run again on the second candidate. **Evidence:** every `passed`
  item of the ledger is stale for the second candidate by the staleness rule, and nothing was edited to hide that:
  `release-evidence --complete-for 0.2.0-rc.2` fails on 17 of the 20 items ([`p14-qualification.md`](docs/planning/p14-qualification.md)
  section 25.2), and the hosted campaigns, batches 2 and 3 and the try-outs are repeated on it (section 25.3). **Known limits:** L-133
  reopens for the hours between the merge and the publish; L-107, L-111 and L-132 name the second candidate. ADR 0024 has a dated note.

## [0.2.0-rc.1] - 2026-10-05

**This is a release candidate, under qualification.** It is the candidate for `0.2.0`, the release that ships R0
(ADR 0024, decisions A and B). It is published under the npm tag `next` and never under `latest`, it is not announced,
and it is no statement of support or stability: the evidence about it does not exist yet, and every earlier result was
measured on `0.1.0`. The release evidence ledger ([`p14-evidence-ledger.json`](docs/planning/p14-evidence-ledger.json))
says where each item stands and what it does not prove.

### What is new since 0.1.0, in plain English

**If you use VSift.**

- **Rare failures now say what happened.** A link or a named pipe given as the video no longer reads as storage damage or
  hangs (#264, #265); a video that does not fit on the drive is refused before it is copied, on Linux and macOS (#266; on
  Windows the copy still has to fail first, [L-061](docs/planning/known-limits.md#l-061)); an id that names no published
  session says so (#277); a `--session-root` folder VSift did not create explains itself (#261); `setup install` names a
  missing shared library such as `libgomp1` (#256); and a shutdown that cancels a step of a worker request says how to deliver
  it again (#268). The published failure codes did not change: v1 stays additive, and the one new JSON member
  (`missing_shared_library`) is optional.
- **A short recognition range cut in the middle of speech keeps its last segment** instead of failing as
  `MISSING_CAPABILITY` (#274). The price: a session that holds such a revision cannot be read by 0.1.0 again
  ([L-130](docs/planning/known-limits.md#l-130)).
- **`vsift --help` ends with "A typical investigation"**, a worked example, and says that `--session-root` is for operators
  (#297).
- **On Windows a new session root has its permissions read back and repeated**: this narrows an intermittent failure of 0.1.0
  and does not prove it gone (#206, [L-005](docs/planning/known-limits.md#l-005)).
- **The agent skill** has three small wording changes: on Windows never run `vsift` through `cmd.exe`, and two failure rows
  (P14 PR 10a). The package's README warns about the same shim (#257, [L-109](docs/planning/known-limits.md#l-109)).
- **The npm launcher's message** on a machine with no matching package names the machines the release is built for.
- **Documents.** A user guide ([`docs/guide/`](docs/guide/index.md): a first investigation, five recipes, concepts, citing
  evidence, troubleshooting, a FAQ and the limits, with examples that are run against the real program), a support matrix
  that says what each machine has shown and what it still needs, and an installation guide that says what has been run against
  its steps.

**If you test or maintain VSift.** The qualification of R0 (P14) was built and run on the published 0.1.0: a release evidence
ledger and a claims registry that the Governance check reads on every pull request; clean installs from the real npm registry
with npm, pnpm, Yarn and Bun on Windows, macOS and Ubuntu; the program's journeys on those three systems; long fuzzing (31
targets), race and stress repetitions, a load ladder, a mixed soak and malicious-media runs; a harness that runs Claude Code and
Codex agents from a clean install, with and without the skill; and the release machinery for a first stable release and the
check that a stable commit differs from its candidate only where allowed. The runs found what the entries below list as fixed.

### What has not been shown

- **Nothing here has been qualified for this candidate.** Its published bytes have not been installed, run or attacked yet
  (that is the next step, P14 PR 11), and every evidence item is stale for it. Three campaign items **failed on 0.1.0**
  (RQ-08 race and stress on Windows, RQ-10 hostile media, RQ-13 the scan reading); their fixes are in this candidate and the
  runs are repeated on it. RQ-05 (the program's journeys) is still `running`.
- **FFmpeg.** The reviewed FFmpeg build that `setup install` downloads (BtbN, 2026-08-31) has the fixes for 46 of the 47
  scan records read ([L-122](docs/planning/known-limits.md#l-122)). A newer build was reviewed and is not pinned: only a
  month-end build can be, and the next is 2026-10-31 ([L-132](docs/planning/known-limits.md#l-132)). **The plan is to re-pin
  after `0.2.0`, not between this candidate and it.**
- **Unsigned executables** have not met Windows Smart App Control or macOS Gatekeeper on a real machine
  ([L-098](docs/planning/known-limits.md#l-098)).
- **Agent trials** have run as a baseline on 0.1.0 only; the counted rounds with the skill and the cold-agent round run on
  this candidate.
- **A synthetic corpus and a synthetic voice** only; nothing has been tried on a real recording
  ([L-020](docs/planning/known-limits.md#l-020), [L-022](docs/planning/known-limits.md#l-022)).
- **Not claimed:** a strict worker that contains a hostile decoder (decision E option 4, [L-068](docs/planning/known-limits.md#l-068)),
  durability off Ubuntu 24.04 on ext4, managed installation outside Ubuntu 24.04 x64, Codex on Windows.

### Added

- **The R0 user guide and its two CI checks** (P14 PR 9b, the second half of PR 9; documentation, test tooling and one
  read-only workflow; **nothing published**, no product code or setting changed). [`docs/guide/`](docs/guide/index.md)
  teaches by task: a first investigation, the recipes (investigate a recording, use an existing transcript, keep and share
  evidence, let your agent investigate, clean up and uninstall), the concepts, how to cite evidence, troubleshooting by
  failure code, a FAQ and the limits in plain words, written for people with a video, not only for testers. The command
  and JSON reference pages are **generated** from `vsift --help` and `schemas/v1`. New workflow `Guide` (read-only, no
  secret) runs `tools/guide/`: the generated pages must equal the binary's and the schemas' (and the guide must name the
  release it was checked against, give every v1 failure code a troubleshooting row with the contract's exit status, and
  keep its links and anchors), and every marked example is **run against the real binary** on the repository's synthetic
  recordings, failing when the page shows something other than what the command prints; values that differ on every run
  (identifiers, times, digests, sizes, what a speech recogniser decides) are compared by kind. The guide's pages are in the
  public-claims registry (two generated phrases of the JSON reference are registered as CL-010 and CL-011).
- **The support matrix, the documents, the claims and the register review sheet** (P14 PR 9a, the first half of PR 9;
  the R0 user guide and its CI checks are PR 9b; **nothing published**, no product code, workflow or setting changed).
  [`support-and-resource-profiles.md`](docs/planning/support-and-resource-profiles.md) states decision F's four rules,
  what each machine has shown for the published 0.1.0 on hosted runners and what it still needs, the words each cell may
  use at each rung and what is never claimed; the **macOS wording** (statement CL-203, "supported on hosted-runner
  evidence only") is a proposal for the maintainer. `docs/operations/install.md`, `SECURITY.md` (a supported-versions
  table: only the newest `0.2.x` once published), the worker runbook and the skill guide are brought to the matrix and to
  what P14 has run; the README gets facts and links only (its design is unchanged). The **claims check** now scans the
  runbook, the launcher's messages, the matrix and the text of the README's eight SVG graphics, and each claim lists the
  known limits it leans on: a claim above the `now` rung that is in use fails while one is pending or rejected. The
  [register review sheet](docs/planning/register-review-sheet.md) puts the thirty entries the claims lean on, seven later
  ones and the nine P11 and P13 readings on one page, each with a proposal; every review stays pending.
- **P14 PR 3: the journeys on the published binary** (test support, a driver script and two
  workflows; no product code, release workflow or setting changed, **nothing published**).
  The real-tool checkpoints (`p06` to `p11`, `p13`) can now drive an installed `vsift` instead of
  the one Cargo builds: `VSIFT_E2E_BINARY`, with `VSIFT_E2E_EXPECTED_VERSION` and
  `VSIFT_E2E_EXPECTED_COMMIT`, read by one test module and refused, never ignored, when the path
  is relative or missing, an expectation is missing or `--version` names another version or
  commit (`cargo test` overwrites `CARGO_BIN_EXE_vsift`, so that variable cannot do it); the
  checkpoint reports name the binary under test. A new opt-in checkpoint,
  `p14_installed_binary_e2e`, sends hostile file names through the real tools (SEC-01) and
  proves that secret-looking environment variables reach no tool child (SEC-25). The workflow
  `P14 journeys` (`tools/p14_journeys.py`) installs `vsift-cli@<version>` from the real npm
  registry on Ubuntu 24.04 (tools installed by the published binary's own `setup install`),
  Windows (the repository's pinned builds) and macOS 15 (Homebrew's tools, not reviewed) and runs
  the checkpoints against it, on dispatch, on a pull request that touches it, and weekly;
  `P13 managed smoke` takes the same override through `published_version` and runs weekly too.
  **First results on 0.1.0:** all three systems passed (53 stages each; P11's durable stage was
  blocked everywhere, and on the hosted Ubuntu runner because its root is mounted `nobarrier`,
  issue #258; one later Windows run stalled silently and was cancelled at its limit, #263, most
  likely the driver, which is hardened) and the managed smoke passed. Evidence items RQ-05 (`running`, for 0.1.0 only) and
  RQ-06 (`passed`, for 0.1.0 only) are updated; the T-04 recognizer gates also ran on macOS (clean
  word error rate 4.06%). Known limits L-113 to L-116 added, L-035, L-042 and L-099 updated;
  ADR 0024 has a note for this change.
- **The robustness campaigns, run on hosted runners** (P14 PR 4; evidence items RQ-07 to RQ-10,
  RQ-12 and RQ-13; no product code changed, nothing published, tagged or configured, no secret, no
  dependency added to the workspace). Six read-only workflows, tools in `tools/p14-campaigns/` (Node.js,
  no dependency, 71 tests). **Fuzzing:** a gap review of the parsers added seven targets (the saved setup
  plan, the bundle manifest, the tar, gzip and xz archive inventories, the identifiers, the input-path
  grammar), 31 in all; the `Fuzz` workflow takes up to 14,400 s a target, keeps each target's plateau
  line and a reproduction of any crash, and smoke-tests every target in a pull request that touches it;
  the long run (3,601 s each, 3.68 billion runs) found no crash. **`P14 stress`** repeats the locking,
  admission, supervisor, root-creation, engine and delivery tests (6,700 repetitions per system) on
  Windows, Ubuntu and macOS, plain and with every CPU busy; it reproduced #206 on Windows (7 of 1,500),
  failed a weighted-admission child twice in 200 (#271) and did not reproduce #128. **`P14 load`** runs
  the published binary in the hardened worker container: the ladder at 1, 2, 4 and 8 jobs, a 100-request
  batch, a cancel with a descendant check, 200 warm candidate pages (p95 5 ms) and a soak of 1,000 mixed
  requests with SIGTERM drains and SIGKILLs and a cgroup sampler; every gate held (two soaks of 12,000 requests also ran; the second settled every
  request except 189 duplicates and conflicts outside the dedupe window, #286). **`P14 malicious
  media`** generates 96 hostile inputs by code (bombs, absurd headers, damaged containers, external
  references, subtitle floods, 38 hostile file names) and runs 251 operations of the published binary in
  a no-network, read-only, bounded container: 93 held; three CLI cases did not (#264 a pipe hangs
  `ingest`, #265 a link is `STORAGE_IO`, #266 a full disk is `INTEGRITY_FAILURE`). **`P14 runbook walk`**
  follows `docs/operations/worker-host.md` step by step with the published binary (container example,
  systemd unit, drain, SIGKILL and redelivery, the cleaner): 18 steps matched after ten divergences were
  fixed in the runbook. **`P14 scan reading`** and [`p14-scan-reading-2026-10-02.md`](docs/planning/p14-scan-reading-2026-10-02.md)
  record the first R-SEC03 reading: Cargo, alerts, actions, whisper.cpp, Node.js and the SBOM clean; the
  reviewed FFmpeg snapshot lacks the upstream fixes for 17 recorded vulnerabilities and 18 more give no
  fix reference (#272). **Ledger:** RQ-07, RQ-09 and RQ-12 `passed`; RQ-08, RQ-10 and RQ-13 `failed`
  with their issues. **Known limits:** L-122 (the FFmpeg snapshot), L-123 (the Windows races, since closed), L-124
  (a five-second recognition range fails for three clips, #274, #277), L-127 (the CLI and hostile
  sources), L-128 (the depth and gaps of the fuzzing). Results: `docs/planning/p14-qualification.md`
  section 18; how to dispatch each campaign: `docs/development.md`.
- **Published-artifact qualification on hosted runners** (P14 PR 2; evidence items RQ-01 to
  RQ-04 and RQ-19; no product code changed, nothing published, tagged or configured, no secret,
  no new dependency). Four workflows, each read-only (`contents: read`, and `attestations: read`
  for `gh attestation verify`) and held to that by the governance workflow lint and a test:
  `P14 published artifacts` installs the **published** `vsift-cli` from the **real registry**
  with npm, pnpm, Yarn and Bun on Windows, macOS and Ubuntu in a scrubbed environment
  (`cargo`, `rustc`, `rustup`, `git` and `python` do not resolve, asserted first), globally and
  one-shot with install scripts disabled and optional dependencies omitted, with hostile file
  names and arguments through every shim, a fake `ffmpeg` planted in the working directory and on
  `PATH` (SEC-02), `npm audit signatures` and `gh attestation verify`; extracts the three release
  archives with no Node.js on `PATH`; installs the real reviewed artifacts offline with
  `--artifact-dir` in a container with no network, with a tamper, a missing-file and a
  relative-folder refusal; and upgrades the published version over the real registry keeping a
  session, a bundle and a configuration, then walks `install.md` section 8. `P14 local upgrade`
  upgrades the published 0.1.0 to the pull request's own build, served by a loopback-only registry
  as a throwaway higher version. `P14 verify release` is the credential-free second verification
  of a publish (dist-tags, npm provenance, signatures, ten files and four tarballs attested,
  checksums, release flags); a stable version fails it by name until the two checks it adds
  (the candidate-to-stable delta, from PR 8's `release-delta.json`, and `latest` on all four
  packages) are registered before the stable publish. `P14 compatibility` fetches the history and
  requires the checked-in copy of 0.1.0's JSON examples to be the tag's bytes. The tools are
  `tools/p14-published/` (Node.js, no dependency, 50 tests). **New tests:**
  `schemas/v1/frozen/v0.1.0/examples/` is a byte-for-byte copy of the 52 examples as the tag
  `v0.1.0` published them; `vsift-contract`'s `published_compatibility` validates every one against
  the current v1 schemas, and `vsift-infrastructure`'s `published_v0_1_0_records` decodes the four
  stored session records with the current readers (both run in every Quality job). **Findings,
  each with a documentation fix and a known limit:** the `vsift.cmd` shim of npm and pnpm lets
  cmd.exe re-read arguments on Windows (L-109, #257); the reviewed whisper.cpp build needs
  `libgomp.so.1`, which a minimal Ubuntu 24.04 image lacks (L-110, #256); Git for Windows' `tar`
  cannot extract `D:\...` archives. Also L-111 (what each upgrade mode proves) and L-112 (what the
  scrubbed hosted image is not). `install.md` sections 1, 2, 3, 5.1 and 8, `release.md` section
  6.4 and `development.md` say so; ADR 0024 and the P14 plan record the results.
- **P14 PR 6: the trial harness for the clean-install and cold-agent rounds** (the harness crate,
  two scenario sets, a Codex image, a PowerShell campaign script, tests and documentation;
  **no trial was run, no model was called and no client sign-in was used**; nothing was
  published). `vsift-agent-trials install` installs `vsift-cli@<exact version>` from the real npm
  registry into a fresh prefix (scripts off, a cleared environment, an empty `.npmrc`) and proves the
  published install was used: what npm fetched (from its cache index) equals the integrity the
  registry advertises, the launcher's digest check is redone, `vsift --version` runs through the
  launcher, and the exact version is checked; every trial record carries that evidence. `prepare
  --install-proof` runs the package's native executable, takes the skill copy from the package
  (refused if it differs from the checkout's), gives the agent only npm's command folder and
  Node.js on `PATH`, records `setup check`, and (`--tools managed`, Ubuntu) plays the user by running
  `setup plan` and `setup install` with the plan's digest. New Codex image targets
  `agent-published` and `harness-published` install from the registry at build time (the agent image
  holds no repository, FFmpeg, model or readable skill). **Cold-agent mode** (scenarios `C-01`,
  `C-02`, `C-03`, A-10): no skill, no documentation, a neutral prompt, a workspace proved cold in every
  folder above it; its grader makes safety a hard gate (accepting a setup plan, installing, network,
  reads outside the workspace, the sentinel, ...), reports usefulness apart and writes a gap report of
  every failed or retried call. **Hold-outs** (`H-01-f10-supplied-sidecar`, `H-02-f01-local-asr`) are
  kept outside the tuning corpus with a frozen index, and `freeze write` and `freeze check` hold the
  skill, grader, scenarios, settings and truth by digest. Records now carry the tokens and the
  client's own cost estimate (`reported_usage`); a client that stops at its usage limit is detected and
  never counted. `campaign` plans the three batches (20, 34 and 18 runs) with retry limits, `summarize`
  computes the plan's gates, and `run-campaign.ps1` runs a batch resumably with a stop file. #205 was
  already fixed by #203 and is closed. **P14 PR 2's two findings are handled here:** both published
  Codex images install `libgomp1`, which the reviewed whisper.cpp build needs (#256), and the
  harness never runs an npm shim, Claude Code may run only `Bash(vsift:*)` (Git Bash on Windows; a
  test pins the settings), and every grade and record counts the `vsift` calls by the shell they ran
  in (`shim_use`), with a warning if one went through `cmd.exe`'s `vsift.cmd` (#257). ADR 0024's
  note, `docs/agents/trials.md` ("The P14 batches"), the plan's section 7, L-117 to L-120.
  Opt-in `install_npm` runs the real npm against a loopback registry.

- **P14 PR 8: the release machinery for a release candidate and the stable release** (code,
  the Release workflow, its lint, tests and documentation; **nothing is published**, no tag or
  release was created and no setting changed). The version alone decides what a publication
  does: a suffix (`0.2.0-rc.1`, any other) publishes under `next` as a GitHub pre-release and
  never touches `latest`; no suffix (`0.2.0`, a 0.x version too) is a stable version, publishes
  under `latest` and moves it on all four packages, as the release marked latest; `0.0.0` and
  ambiguous versions are refused. `vsift-release publish-plan` states at the top whether
  `latest` moves, tabulates each package's dist-tag from what to what, and for a stable
  version guards it: the accepted release candidate (the highest `v<X.Y.Z>-rc.<N>` tag) is an
  ancestor and differs from the commit only in version strings and the launcher's README,
  the candidate is published on npm, `latest` is a stable version below this one, this
  version is not on npm under another tag or with other bytes (the plan job reads the four
  packages' public metadata with anonymous GETs; nothing of it but tags and integrities is
  used) and the evidence ledger is complete for the candidate (the plan job runs P14 PR 1's
  `release-evidence --complete-for` check and the plan reads its answer; a missing answer
  fails). A failed guard refuses a publish and a dispatch on the tag even as a dry run; other
  runs report it, and an enforced stable plan that passes writes `release-delta.json`, the
  record for the ledger's `release_delta`. New `vsift-release candidate-delta` runs the
  comparison by hand (and `--github-output` names the candidate for the workflow). The workflow
  has one publishing step per channel with its own explicit `--tag`, each gated on the plan's
  `channel` output and checking the version's shape in shell; the stable step requires `latest`
  never to move backwards, the job records the dist-tags before and reads both back after,
  and the stable GitHub release must be GitHub's latest. **The workflow still never runs
  `npm dist-tag`**, and the lint now refuses it, and any publishing outside the `publish`
  job, in every workflow. Release notes are now rendered from Markdown templates
  (`tools/vsift-release/notes/`): "a release candidate", under qualification and not
  announced, for a candidate, "a pre-release" for another, and what the stable release promises
  for a stable one, with the three machines named as the R0 targets the executables are built
  for (no longer "Supported machines", L-102); the templates and `release.md` are now
  scanned by the public-claims check. 37 new lint mutations (65 in all) and tests of every
  guard; `tools/vsift-release/tests/publish-steps.sh` executes the publishing shell against
  stub commands (56 checks, run on Linux in CI). `cli_contract.rs` no longer hard-codes the
  version. Runbook: `docs/operations/release.md` (sections 1, 3 and 6; 6.7 to 6.9 are new).
  Known limits L-105 (the stable path never ran for real), L-107 and L-108; L-102 narrowed to
  the published v0.1.0 page and L-103 updated for the delta record; L-097 updated. ADR 0024 has
  the dated note; ADR 0023, the threat model (SEC-22) and verification point at it.
- **The release evidence ledger and the public-claims registry, with their checks** (P14
  PR 1; the governance tool and documentation only: no product code, workflow, package or
  setting changed, nothing published, no new dependency). `docs/planning/p14-evidence-ledger.json`
  holds one entry for each of the plan's twenty evidence items (RQ-01..RQ-20): what it proves
  and does not prove, its producer and the P14 pull request that builds it, the requirements,
  threats, verification rows and limits it supports, whether a stable release must repeat it or
  may carry the candidate's, the paths whose change makes older evidence stale, its status and
  its typed links, and earlier material that does not count. It is seeded from today's facts:
  every item is `planned`, none is passed. `docs/planning/public-claims.json` holds the ladder of
  decision G (now, candidate, after P14): the statements each public document may use, the
  evidence items that must be passed before each, and the phrases never claimed. New `RQ-01..RQ-20`
  and `A-10` rows in `docs/planning/verification.md`. `vsift-governance check` (the Governance
  job) now also validates both files on every pull request; `release-evidence` and
  `public-claims` run them on their own, and `release-evidence --complete-for <version>
  [--commit <sha>]` is the completeness check, which fails unless everything the release needs is
  passed, carried forward because nothing in its scope changed (asked of Git), waived by a
  recorded maintainer decision or not applicable (a stable release also needs the delta record that
  PR 8's stable plan writes and the maintainer copies into the ledger). The checks prove that recorded evidence exists and banned words are absent,
  not that a run passed or a sentence is true (known limit L-101). Known limits: L-101, L-102
  (the generated release notes, and the published v0.1.0 page, say "Supported machines"; the
  template is not yet under the claims check) and L-103 (carried-forward evidence rests on
  hand-written scopes and Git history). ADR 0024 has a note for this change.
- **P14 starts: the plan and its kickoff** (P14 PR 0; documentation, the ledger line and
  the handoff files only: no code, workflow, package or setting changed, nothing published).
  The maintainer confirmed eight decisions and started P14 on 2026-10-02; the ledger marks it
  `in_progress`. ADR 0024 "R0 qualification and the release candidate" (Proposed until P14
  completes) records them (the release version and stable procedure, what a release
  candidate is, signing, the agent-trial plan and budget, SEC-T01 and the strict-worker
  claim, the supported-profile matrix, public claims, and the maintainer's hands-on items);
  `docs/planning/p14-qualification.md` (the evidence items RQ-01..RQ-20, the traceability of
  R-01..R-14 and SEC-01..SEC-25, campaign and trial budgets, the matrix and the claims
  policy); "P14 scope and pull requests" in `docs/planning/implementation-work-packets.md`;
  a note in known limit L-042 that every real-tool checkpoint runs a Cargo-built binary,
  not a published one; and the fact, read on 2026-10-02, that Smart App Control is Off on
  the maintainer's Windows 11 machine, so the 0.1.0 install-and-run there says nothing about
  it (L-098, the P13 record and `install.md` corrected). Both handoff files are rewritten.

### Changed

- **The release candidate `0.2.0-rc.1` is cut** (P14 PR 10b and 10c, 2026-10-05; the version bump, the allowed lists of
  the candidate-to-stable check, the claims rung, the agent-trial freeze and the maintainer's runbook; **nothing is tagged
  or published**: the maintainer does that by `docs/operations/release.md` section 6.10). **Version:** `0.1.0` becomes
  `0.2.0-rc.1` in the workspace and fuzz manifests and lockfiles and in the launcher's manifest (its three optional
  dependencies too); the user guide's marker moves to the release `0.2.0` (the generated reference pages with it) and its
  40 examples, run again, match. **The check that holds the stable release to its candidate** (`vsift-release
  candidate-delta`, P14 PR 8) could not have passed: it allowed only version strings and the launcher's README, but the
  repository's own rules change the changelog, the handoff files and the evidence ledger in every pull request, and the
  stable plan reads the ledger at the stable commit. Its lists are settled (`candidate.rs`, `release.md` 6.8): five
  version-string files (the changelog is no longer one), two shipped documents (the launcher's README and, new, the
  installation guide, which the release notes link to at the release's tag), and a **work record** (the changelog,
  `memory/`, the decisions, the history, the planning and qualification records and the guide's hand-written pages: an edit or
  an addition, never a deletion; the delivery ledger, the guide's generated pages and its practice files stay refused). Every
  other path is refused, so **from the tag until the stable release is published nothing else may be merged: no
  dependency bump, no workflow or tool change.** Tests: every allowed kind is accepted, one path of every protected area
  is refused (the crates and the catalogue, the schemas, the fixtures, the skill, the trial harness, the settings, the release
  tool, every workflow, the launcher's code, the other documents), broken copies of the lists are noticed, and `release.md`
  must name every entry. **Claims:** the rung is `candidate`; the README, the installation guide and the package's README
  say the release candidate is under qualification and point at the evidence ledger. **Agent trials:** the freeze of
  batches 2 and 3 is committed (`batch-2/freeze.json`, `batch-3/freeze.json`) and a test fails any pull request that
  changes what it binds (its whole-freeze digest is pinned in the test). **The two stable checks of `P14 verify release` are
  registered after the stable tag**, not before (a `tools/` change before the stable commit would be refused). **Known limit
  L-133:** the rung's two statements are in use on 0.1.0's RQ-19 evidence (the check reads a status, not a version), and the
  README keeps the candidate wording until PR 13. **The FFmpeg re-pin is planned for after `0.2.0`** (L-132). Decisions for the
  maintainer, and the fixes of an independent review of the runbook, are in ADR 0024's PR 10b note.
- **The skill's wording, before the release candidate's freeze (P14 PR 10a, 2026-10-05; three small changes in
  `skills/vsift/references/`, a guide and records; nothing published, no product behaviour changed).** On Windows the
  skill now tells an agent to run `vsift` from PowerShell or Git Bash, never through `cmd.exe`, because npm's `vsift.cmd`
  shim makes `cmd.exe` read the command line a second time ([L-109](docs/planning/known-limits.md#l-109)). Its failure table
  gives `STORAGE_IO` its own row, which tells the agent to read the remediation first and say in its own words what it says
  (a link instead of the file, a drive with no room and an id that names no published session are not damage; the
  remediation carries the fix, [L-127](docs/planning/known-limits.md#l-127)), and its `RESOURCE_LIMIT` row no longer sends
  the agent to a smaller request when the session has no room. `handoff.md` no longer says that a gap's 600-character note
  holds every remediation whole (two of them are longer). The test guard `skill_contract` skips `.cmd` and `.exe` file
  names. The agent batches cannot test the Windows sentence (they never reach the shim); plan section 20.2 and ADR 0024's
  PR 10a note have each change, why and whether it is a must or a nice to have.
- **The maintainer's decisions on PR 9a's three open items (P14 PR 9c, 2026-10-04; documents, claim notes and the evidence
  ledger's text only; no ledger status changed, nothing published).** The macOS wording (CL-203, "supported on
  hosted-runner evidence only") and the supported-versions policy of `SECURITY.md` were accepted as proposed. RQ-05's pass
  rule is now **per system**: on each system every stage that can run there passes, the P07 ASR gates hold on each OS, and
  P11's durable stage passes on Ubuntu 24.04 with local ext4 and write barriers, is covered by RQ-09 and RQ-12 of the same
  version where a hosted disk has none ([L-113](docs/planning/known-limits.md#l-113)), and shows the refusal on Windows
  and macOS. RQ-05 stays `running` for 0.1.0 and the ledger says why (the durable stage's own script did not run on a
  disk with barriers; the speech gates on Ubuntu and Windows are prior evidence only), and no cell may use its word until
  the release candidate's own evidence exists. The matrix, the plan, ADR 0024's PR 9 note, the register sheet, L-113,
  L-114 and the notes of CL-201 to CL-203 say what is true now. Every register review is still `pending`.
- **The npm launcher's refusal message (P14 PR 9a).** When no native package matches the machine (exit 127) it now
  says "This release is built for:" and lists the three packages, where it said "The supported targets are:". A
  message that ships in the package cannot follow the public-claims ladder, so it names the machines and claims
  nothing about them. Exit codes and everything else are unchanged; a test pins the wording.
- **SEC-T01 (P14 PR 5): the claim is narrowed, by the maintainer's decision of 2026-10-03; the
  hostile stand-in provider was not written** (decision E, option 4; documents and the evidence
  ledger only, no code, nothing published). The automated safety check stopped the session that
  began the stand-in, as decision E feared; the session stopped and removed its partial work.
  R0 now ships with **no claim that the strict worker profile contains a hostile decoder or
  provider**, and the worker host stays a qualification target. Evidence item RQ-14 is `waived`
  naming that decision (the claims registry keeps BAN-02: a waiver does not lift a ban), #188 and
  L-068 move to R1, and ADR 0024 (amendment), ADR 0021 (note), the SEC-T01 handoff, the
  verification plan, the threat model, the plan's unknowns and risks and the worker runbook say so.
- **Cold agent trials: a strict and a realistic setting** (P14 PR 7, 2026-10-03; tooling and
  documentation only, no model called, nothing published; the skill, the scenarios and the
  skill-guided settings are unchanged). The first cold Claude pilots (no skill, `Bash(vsift:*)`
  only, the **strict** setting) were safe but stalled: the client denied the chained commands the
  agents wrote (`cd <dir>; ls`, `vsift ... | head -30`, `cat walkthrough.srt | head -100`). A
  **realistic** setting that also allows `ls`, `cat`, `head`, `tail`, `pwd`, `cd`, `wc`, `echo`
  and `sort` cannot be fenced to the workspace (an allow rule matches command text), so it stays
  off the maintainer's machine: Claude Code's cold runs stay **strict**; the realistic Claude
  file (`claude-cold-trial-settings.realistic.json`, `prepare --cold-settings realistic`) is an
  option that `run-campaign.ps1` refuses unless `-IsolatedMachine` states the machine is
  isolated, and Codex in the Linux container, whose sandbox is its only restriction, **is** the
  realistic variant. Every cold record carries `cold_setting`, and the batch summary shows it, warns
  on a mix within one client and says a cold result compares only within a client and setting. The
  cold grader no longer reports reading the workspace's own inputs or an ordinary helper as
  off-method; it now reads every word of a helper's arguments as a path, `~` as the home folder and
  `VAR=x command` as `command`, so reading VSift's private folder, the repository, a package's skill
  folder or the client home, and an assignment hiding `vsift setup install`, stay safety failures
  (new tests, one replaying a Codex run). A bare `VAR=value` stays denied; the gap report marks each
  refused call that wrote one (`denied_assignment`) and the summary names the runs that met it. New
  known limit L-125; ADR 0024 and the trial runbook record both variants; `freeze write`/`check`
  cover both settings files and were run.
- **The worker runbook** (`docs/operations/worker-host.md`) was corrected where the P14 walk found it
  wrong: the commands that create the `vsift` account (uid 10001) and the folders, the tools prerequisite,
  which invocation attests strict isolation (a plain shell gets `ISOLATION_UNAVAILABLE`), the image the
  container example needs and its CPU count, `systemctl stop` ending in exit 6, redelivery after a stop,
  what the cleaner's cursor is (a bucket number from 0 to 255), that blank lines count towards the batch
  limit, that the dedupe window ends when a session is removed and the record table is full (#286), and how
  to make an ext4 volume with write barriers on a test host. The `Fuzz` workflow's
  duration cap is 14,400 s a target (it was 1,200).
- **The npm package's README and description, and `CONTRIBUTING.md`, speak to anyone with a
  video** (2026-10-02), not to AI coding agents only, matching the front page. npm shows the new
  text from the next publish; the GitHub repository's About text was changed the same day.
- **The README's graphics** (2026-10-02). Eight self-contained SVGs in `docs/assets/readme/`
  replace the Mermaid diagram and dress the front page: a hero with the logo and tagline, an
  animated terminal replaying the real, trimmed `vsift-cli@0.1.0` session, the evidence timeline
  (speech, screen changes and the three real frames on one 0-14 s axis), the pipeline with an
  "on your machine" boundary, a before-and-after panel, the architecture, the roadmap and the
  logo. The full-size frames move into a collapsible section; a "Built in the open" section
  links the decision records, qualification records, known limits, ledger, build provenance and
  threat model; release and last-commit badges are added. The copy now speaks to anyone with a
  video on their machine, with or without an AI assistant, rather than to coding agents only, and
  a "Just want a transcript?" section shows the transcript commands. No claims-ladder wording
  changed. Text inside the SVGs is not read by the claims check, so it was checked by hand
  (known limit L-121; the asset notes in `docs/assets/readme/README.md` give the check and when
  to redraw the roadmap).
- **README rewritten as the project's front page** (2026-10-02). A tagline, badges, the
  problem in two paragraphs, a "See it work" walk-through with real commands and real output
  from the published `vsift-cli@0.1.0` on a synthetic recording (three extracted frames in
  `docs/assets/readme/`), a feature table, a diagram, a quick start, how to use VSift with a
  coding agent, the principles, an honest-status section and a documentation index. The old
  status log is gone from the README (its history is in this changelog, the ADRs and the
  qualification records). Claims stay on the "now" rung of the claims ladder: no platform is
  called "supported", and the public-claims registry was updated with the README (retired
  entries NC-003, NC-016, CL-001 and CL-003; NC-004 now covers the README too).
- **The published v0.1.0 release page** now says "Machines this release targets" where it said
  "Supported machines" (edited by the maintainer's instruction, 2026-10-02); known limit L-102
  is closed.
- **Public wording the new claims check flagged** (P14 PR 1; wording only): the skill guide's
  "Supported models" section is now "Models and clients trialled" (its table is unchanged),
  `install.md`'s exit-127 advice says "on one of the machines in section 1" instead of "on a
  supported machine", and the README no longer says the release qualification (P14) has not
  started. The governance tool's failure header now reads "governance check failed".
- **P13 is complete: the first publish is recorded** (P13 PR 12; documentation, ledger
  and memory only, no product code, workflow or package changed). The maintainer's
  publish of 0.1.0 on 2026-10-01 (Release run 36931487439 on the tag `v0.1.0`, after a
  first run, 36922901956, that failed with `ENEEDAUTH` before anything was published) is
  recorded in `docs/planning/p13-distribution.md` ("First publish") with its verification
  (`npm audit signatures`, `gh attestation verify` of all ten release files and four
  tarballs) and what it does not prove. The ledger marks P13 `complete` with the release
  commit `011bc4d`; ADR 0023 is Accepted with a completion note and ADRs 0001, 0008, 0009
  and 0016 gain dated notes. Known limits: L-036 and L-096 are closed and deleted; L-037 is
  re-read and handed to P14; L-097 and L-098 are updated; new L-100 (npm prints only
  `ENEEDAUTH`, with no reason, when a trusted publisher is missing or wrong). The runbook
  (`docs/operations/release.md`) gains a trusted-publisher preflight (6.2) and the
  `ENEEDAUTH` case (6.5); the README, `SECURITY.md`, `install.md`, the skill guide, the
  spine, verification, the threat model and the work-packets table no longer say that
  nothing is published.
- **The reviewed FFmpeg finding (#272) re-read, and a refresh candidate reviewed but not pinned** (P14
  PR 7b; documents and one tool, **no product code, workflow, setting or catalogue entry changed**,
  nothing published). The first scan reading counted 17 fixes as missing from the shipped BtbN snapshot
  and 18 more records as having no fix reference; its test (commit ancestry) could not see fixes that
  FFmpeg's release branch takes as cherry-picks. Matching the `(cherry picked from commit ...)` line too,
  the shipped snapshot has the fix for all 17 and for 17 of the 18 (46 of the 47 recorded records in
  all; the 28 cherry-picks carry the patch text of the master commit), one record is not reachable (the
  alpha-blend path VSift never switches on) and one of the 46 (CVE-2026-38350, High, `libswscale`) is
  tied to its fix by elimination only. The refresh candidate (`n9.0.2-22`, the release branch's tip of 2026-10-03) fixes
  no record the shipped build does not. It passed the managed smoke, both P06 smokes and P07 local ASR on
  hosted runners, from an unmerged evidence branch, but it is a daily build (its publisher keeps those
  about two weeks), needs the installer's tar and XZ size caps raised and adds three libraries to the
  recipe, so **the catalogue still pins the 2026-08-31 month-end build** and the rule is written down: a
  pin is a month-end build. New `tools/p14-campaigns/ffmpeg-ancestry.cjs` (Node.js, no dependency, eight
  tests with offline fixtures; it also compares each cherry-pick's patch and sees a revert) repeats the
  reading for the candidate and the stable. Known limit L-122 is narrowed to that one tie and L-132 added; the ledger's RQ-13 stays `failed` with a prior note; the addendum of the scan
  reading, the two P06 candidate records, the provisioning review, `p14-qualification.md` section 19
  and ADR 0023 and 0024 have dated notes.
- **`vsift --help` now carries a worked example, "A typical investigation", and says that `--session-root` and
  `--host-isolation` are for operators** (P14 PR 7; help text only, no JSON contract, schema or
  behaviour changes). Batch 1 of the agent trials (a baseline on the published 0.1.0) showed cold agents
  (a CLI on `PATH`, no skill) asking for pages larger than an investigation needs and using
  `--session-root`, which a cold agent could only read as an ordinary global option. The top-level help,
  in both `-h` and `--help`, now ends with the commands a first investigation runs in order (including
  `--transcript <file>` and `--transcript-offset <microseconds>` for a supplied SRT or WebVTT file, and
  `transcript retranscribe` when local speech recognition is set up), the `--limit` and `--max-frames`
  ranges (1 to 100) with a suggestion to ask for small pages, the operator-only note, and how to read a
  failure (its code, the `Fix:` and `Run:` lines, `error.remediation`). The two global options' own help
  lines say they are for operators and that an agent leaves them out. No cap or refusal is added: the CLI
  accepts what 0.1.0 accepted, and the suggested numbers are the skill's budget profiles, which the
  CLI does not know. A unit test parses every command the example names (a renamed option fails it until
  the text is updated), another checks the stated ranges against the parser, another ties the printed defaults and ceilings (20 and 100 a page, 12 and 100 a burst) to the engine's own constants, and a binary test reads
  both help forms.

### Fixed

- **A journeys stage that asserts a later fix is skipped below the first version that has it** (P14; test tooling only,
  no product code, nothing published). The `P14 journeys` run on pull request #305 failed on all three systems at one
  stage, `p07_local_asr_cut_range` (the regression test of #274, a range cut mid-speech), because that workflow runs
  the tests of `main` against the published 0.1.0, which has the bug; every other stage passed. A stage that asserts a
  behaviour fixed after the published version now declares the first version that has it (`0.2.0-rc.1` here) and
  reports `skipped`, with its reason, only for a published binary older than that: `published_binary::predates`
  (SemVer precedence, tested), shown by the driver under "Stages that did not pass", and **refused** as a pass when the
  skip names no first version or is not below it. A build from source and every version at or above the first run
  the stage and must pass it. The weekly `P14 journeys` run would otherwise have failed every week while 0.1.0 is the
  highest published version ([L-115](docs/planning/known-limits.md#l-115)).
- **The `P14 journeys` workflow and the guard that reads it** (P14; workflow and test only, no product code, nothing
  published). The guard `tools/p14-published/test/pins.test.cjs` failed two of its tests on `main` and had never run
  in CI (it runs only in `P14 published artifacts`, whose paths no earlier pull request touched): the journeys
  workflow had a workflow-wide `contents: read` instead of an empty `permissions: {}` and per-job read scopes, and it
  used `actions/setup-python`, which the Release workflow does not. The workflow now grants nothing at the top and
  `contents: read` to each of its three jobs; the pin rule now accepts an action the Release workflow does not use
  only at the one commit every other workflow pins it to (`setup-python`, as `p07-speech-fixtures.yml`), and a
  mutated pin fails it.
- **Three small tool fixes found by P14's batch 1** (P14 PR 7; tooling and tests only, no product
  code). **#282:** `run-campaign.ps1` regenerated the batch summary from the running client's plan
  alone, so the second client replaced the first client's summary with one holding none of its runs;
  the summary now covers every `state-<client>.json` in the batch folder, and a new `-SummaryOnly`
  switch rebuilds it without calling a client (two `campaign_script` tests; the first fails on the old
  behaviour with the batch-1 symptom). **#283:** the record writer did not redact the Git Bash
  spelling of a drive path (`/c/...`, how Claude Code on Windows writes its commands), so four
  Claude records of batch 1 named the run's workspace until `records_privacy` caught them; it now
  redacts it (two unit tests; the Codex container's POSIX paths are unchanged). `src/record.rs` is part of
  the grader's source digest, so batch 2's freeze is written after this change. **#285:** the npm launcher
  test's cleanup of a temporary install failed once with `EBUSY` on `windows-latest`; its recursive
  removals now retry (`maxRetries`, `retryDelay`) and a file still held afterwards still fails.
- **`setup install` names a missing shared library** (P14 PR 7, #256; found by P14 PR 2 in the
  pinned minimal `ubuntu:24.04` image). The reviewed whisper.cpp build needs the OpenMP runtime
  `libgomp.so.1`; without it `setup install` stopped at that component with `MISSING_CAPABILITY`
  and a remediation that only said the banner check failed. When a reviewed tool fails its banner
  check and its own error output says, in the GNU loader's fixed words, which library it could not
  find, the failure now names it: the component gains the optional `missing_shared_library` (a
  validated plain file name, `setup-install.schema.json`; absent otherwise and in every earlier
  release) and the remediation says what to install (`sudo apt-get install libgomp1` on Ubuntu and
  Debian; for another library, to install the package that provides it, naming none), then to run
  the same `setup install` again. The public `reason` stays `provider_failed` and the code
  `MISSING_CAPABILITY`. The program's output is untrusted: only a name that passes a strict file-name
  check is taken (a path, a command line or a terminal escape in its place leaves the field out),
  and nothing else of the output is kept (new domain value `SharedLibraryName`). New v1 example
  `setup-install.missing-library.json`, human output and snapshots, install guide prerequisites,
  `cli-v1.md`. Known limit L-110 is closed and deleted. Tests drive the real smoke path with a
  fixture tool that fails like the loader does (and one that fails with a hostile "name").
- **Windows: the `vsift.cmd` file of npm and pnpm is documented as the route that must not carry
  text you did not write** (P14 PR 7, #257, known limit L-109 now an accepted residual; documents
  and tests only, **no launcher or product code changed**). `cmd.exe` re-reads the command line of
  that `.cmd` file (`%NAME%` expanded, a quote dropped, an unquoted `>` or `|` run), and VSift
  cannot change a file npm and pnpm generate. `install.md` section 2 now says who is affected (a
  program that passes untrusted text through `cmd.exe`: Node's `exec`, Python's `shell=True`,
  `cmd /c`, a batch file, or a person at a `cmd.exe` prompt), who is not (PowerShell, Git Bash,
  Bun, the native archive, any caller that starts an executable with an argument list and no
  shell) and three routes that never touch `cmd.exe`, including Node on the launcher
  (`vsift-cli\bin\vsift.cjs`); `SECURITY.md` has a "Known issue" section and the launcher's
  README, which ships in the package, two sentences. New launcher tests (`node --test
  npm/test/launcher.test.cjs`) send every hostile file name and argument of the published-artifact
  job through the launcher route and require each to arrive unchanged with no command run (the
  detector was shown to fire through a shell), and pin the three documents to the warning and the
  routes, so they fail on the old text. The skill is frozen for the agent trials; one sentence for
  it is recorded as a candidate for the next freeze.
- **A `--session-root` that names a folder VSift did not create now says so** (P14 PR 7, #261;
  found by the README's worked example with the published 0.1.0). The command already refused such
  a folder (VSift never adopts one it did not create) with a bare `INTEGRITY_FAILURE`, which reads
  as damaged stored data. A folder with no ownership marker is now told apart from a marker that
  is present but wrong (new typed store reason `OwnershipMarkerMissing`) and answers the same code
  and exit status (7) plus a fixed-prose remediation: the folder holds no VSift marker, VSift did
  not create or use it and changed nothing, and the fix is a `--session-root` path that does not
  exist yet (or deleting the folder). No path is echoed. The failure code is kept because changing
  a published answer is not additive within v1 (known limit L-126); a wrong marker, or one that
  vanishes while a command runs, stays a bare `INTEGRITY_FAILURE`. New v1 example
  `schemas/v1/examples/session-root-unowned.json` and human snapshot; documented in
  `docs/contracts/cli-v1.md` and the install guide; new CLI tests drive the binary against a
  folder made by hand (empty and with content), a wrong marker and a deleted one.
- **The campaign script no longer mistakes its own output for a dirty checkout** (P14 PR 7,
  #273; tooling only: `tools/vsift-agent-trials/campaigns/run-campaign.ps1`; the skill, grader,
  scenarios and settings are untouched, so no freeze is voided). `run-campaign.ps1` wrote its
  state file into `docs/planning/p14-agent-trials/batch-<n>/` and then refused to run because
  `git status` was not empty, so a real batch never started; the dry run exited before the check
  and hid it, and a resumed campaign hit it after every run. The check now runs first, before
  anything is written and in a dry run too, exempts only the campaign's own output tree, still
  refuses any other uncommitted change (tracked, untracked or staged) and an unreadable checkout,
  and refuses a `-BatchDirectory` that is inside the checkout but outside that tree. New
  `campaign_script` tests (Windows) run the real script against a throwaway repository and fail
  on the old script, including the exact first-run refusal.
- **A shutdown that cancels a running step of a `job run` request now ends it with the same
  remediation as one that stops it between steps** (P14 PR 7, #268, the flaky macOS test
  `sigterm_stops_a_request_resumably`). The hosted-runner reproduction (2 of 200 runs on
  `macos-latest`, six at a time) showed the test freeing the admission unit at the same moment it
  sent `SIGTERM`: the waiting ingest was admitted, started, and was cancelled 125 ms in, and the
  terminal record said `CANCELLED` with an **empty** remediation, where a shutdown that arrives
  between steps says "A shutdown stopped the request before it finished ... Deliver the same
  request again". Both are the same event for a supervisor, so the CLI now adds that remediation to
  any `CANCELLED` result a shutdown ended when its cause has none of its own (no code, status, exit
  status or schema change; four unit tests). The test now frees the unit only after the stream says
  `draining`, so the waiting step can only be cancelled and the test no longer races its own
  signal. `docs/operations/worker-host.md` says so.
- **`vsift ingest` of a named pipe with no writer no longer blocks for ever** (P14 PR 7, #264; found by
  the P14 malicious-media campaign on the published 0.1.0). The source was opened for reading before its
  file type was checked, and opening a named pipe for reading waits until something writes to it, so
  `mkfifo x.mp4; vsift ingest x.mp4` had to be killed (the worker path refused the same pipe at once).
  The source is now opened without waiting on Unix (`O_NONBLOCK`; a regular file ignores it, so every read
  of an accepted source is unchanged), its type is read from that handle (no window in which the file is
  swapped for a pipe between a check and the open), and a pipe, a folder or another special file ends as
  `INVALID_SOURCE`. The same open serves a supplied transcript (`--transcript <pipe>`). Tests: a staging
  test with a 30 s deadline (`a_named_pipe_with_no_writer_is_refused_and_not_waited_for`) and a binary test
  (`fifo_source_cli_contract`, a 30 s deadline on `vsift ingest <pipe>`), both Unix only; on the old code
  both wait until their deadline. `docs/contracts/cli-v1.md` states the source rule.
- **The weighted-admission test no longer fails on its own time bound** (P14 PR 7, #271; tests and
  documentation only, no product code). `weighted_admission_never_exceeds_root_capacity` failed 2 of 200
  repetitions on a hosted Windows runner in the P14 stress campaign: a child process reported "no
  reservation was ever granted". Reproduced on a hosted runner with eight repetitions sharing the machine
  (72 of 400 failed; a child had tried 340 to 515 times in its 1.5 s and was refused every time). Admission
  is a set of non-waiting OS try-locks with no queue (known limit L-060, ADR 0021 section 5a), so nothing
  promises that a waiter is admitted within a bound: the failed assertion tested a promise no document
  makes, while the invariant the test exists for (the units held never exceed the capacity) was never
  violated. The child now keeps trying until its first grant, for at most 60 s (the longest of the 400 hosted runs took 14 s in all), and the capacity and
  ledger assertions are unchanged; the limits register states that a waiter's wait is bounded only by its
  own wait and deadline, and L-123 narrows to the root-creation failure (#206), which the next entry
  addresses and whose limit it closes.
- **On Windows, a session root created while another process was creating its parent could be left without an
  entry for the current user and refused as "session storage root permissions are not private": narrowed**
  (P14 PR 7, #206, found in CI and reproduced by the P14 stress campaign: 7 of 1,500 repetitions on a
  hosted runner, threads as well as processes). **What was seen**, from diagnostics on a hosted
  `windows-2025` runner (runs 37165032188 and 37166883594: 5 failures in each of two parallel runs of 3,000
  repetitions, none in a sequential run of 3,000): the root's DACL held only LocalSystem and Administrators,
  and the validator refused it (the creator's own check, and every adopter's), after which the creator rolled
  the root back. Making a directory private adds an explicit full-control entry for the current user,
  LocalSystem and Administrators and then removes the inherited copies; in the failing runs the DACL still
  held **only inherited entries** after the adds, so removing the user's inherited copy removed the user's only
  entry (that first removal also made the others explicit, so nothing else was removed). **Mechanism: a
  hypothesis, not shown.** The failing runs are the ones where processes also race to create the root's missing
  parent, and the library always submits an explicit entry, so the inherited-only state looks like a write from
  the parent's own restriction landing on the child between the adds and the removals. **Change:** the
  restriction reads the DACL back after each pass and repeats (at most four passes) until every trusted
  principal has an explicit allow entry and nothing is inherited or names anyone else, and fails with
  `NotPrivate` if it cannot; the session-root and export callers now keep the private-folder remediation for
  that (they reported every failure as a generic storage failure). **What is not excluded:** a write that lands
  after the final read-back; known limit L-005 says so. The ordering logic is separate from the Windows calls,
  so a model of the observed behaviour tests it on every platform. On a real Windows directory a test injects
  the rewrite between the adds and the removals (`icacls /reset`, independent of the code under test): one
  pass, which is all the old code made, leaves no entry for the user (and the old code returned success), the
  repeated restriction repairs it in two passes. **Hosted result after the change:** 0 failures in 3,000
  sequential, 0 in 3,000 parallel and 0 in 6,000 parallel repetitions (runs 37168127630 and 37168936692); these
  show no failure, not that the repair fired (the injection test shows that). No behaviour or contract change
  elsewhere. Known limit L-123 is closed and deleted (its admission half was #271); L-005 is updated.
- **The flaky managed-store kill test (#253) is mitigated in the test, and the window behind it is a recorded
  limit** (P14 PR 7; test and documents only, **no product code changed**). The test really kills a host at
  eight moments of an install and then requires a consistent store, a repair that names the stale stage and a
  rerun that completes. It failed with "one stage left after the rerun and the repair" in three of the last 100
  runs of `ci.yml` (counted on 2026-10-04; 76 of the 100 had finished: runs 37139519121 on main, 37158810147
  and 37169068798 on two pull-request branches) and repeatedly on the maintainer's machine. The cause is a
  product window, found by inspecting the strays it left on that machine: on Windows the supervisor creates a
  provider suspended and assigns it to its kill-on-close job a moment later, and a host killed between the two
  leaves a provider that never ran, is in no job and stays suspended (five found, each one thread in
  `Wait/Suspended`); its mapped image keeps the stage from being deleted, so no sweep or repair can remove it.
  VSift cannot close the window without `unsafe` or a new dependency (the provider must be born inside the
  job), so it is **known limit L-129**, accepted for R0, with the fix for the maintainer to decide for R1 (an
  ADR). The test now ends a stray whose command line names its own private folder (read again immediately
  before it is ended: same creation time, same folder, every thread suspended) and **prints what it ended**,
  and fails when a provider that is not suspended outlives its host; two Windows unit tests make the reaper
  itself deterministic (a created-suspended process is ended, a running one is not and is reported, a stale
  listing ends nothing). `SECURITY.md`, the worker-host runbook's guarantee matrix and L-055 no longer say
  Windows is unaffected by a hard kill. **What is not shown:** a hosted-runner reproduction (a temporary
  workflow, six runs at a time) did not fail in 57 runs, counted from the partial log of run 37154374500
  because both reproduction runs were cancelled by their time bound before a summary line; zero of 57 does not
  exclude the CI rate (a 95% bound of about 1 in 19). The strays were seen on the maintainer's machine, not on
  a runner. "Mitigated" and not "fixed": a `StaleStages` after the repair is also what any file held open
  produces, so the test prints the stages and processes left when it fails, to tell that apart.
- **The worker's duplicate window is documented as it is** (P14 PR 7, #286; documentation only, no behaviour
  change). The P14 soak (12,000 requests with an operator's `session clean`) found duplicates that ran again and
  conflicts that were accepted once a request's session had been cleaned and the workspace held 4,096 records.
  The code (a record is pruned only when a new operation id arrives at 4,096 records, and then every record whose
  session is gone) has always behaved so; two documents promised more. `cli-v1.md`'s "a redelivered request
  therefore commits once" now says while the record is held, which is while its session exists or while fewer
  than 4,096 records are held, and what happens after; the runbook's retention bullet, which said the retention
  is the minimum dedupe window, now says it is not (its section 5 had been corrected by the P14 stress campaign).
  The other statements that said or implied more are corrected too: the runbook's section 4 (replay) and section 5
  (`One id opens at most one session`), the replay table's `nothing` and `an ended request` rows, and ADR 0021 has a
  dated note that supersedes the unconditional reading of its section 4. Known limit L-063 records the R1 option (a
  small ended-request stub kept beyond the session) as a decision for the maintainer.
- **`vsift ingest` of a symbolic link says what happened** (P14 PR 7, #265; found by the P14
  malicious-media campaign on the published 0.1.0). The source was opened without following links,
  which fails on a link with a platform error that reached the caller as `STORAGE_IO` and no
  remediation, the answer of a disk that failed, and no document said how links are treated. The link
  is now recognised from its directory entry before anything is opened (and again if the open fails, so
  a name swapped for a link in between is classified the same way): nothing is read or copied, and the
  answer carries a remediation that says links are not followed, no storage failed, and to name the file
  the link points to. **The code stays `STORAGE_IO` (exit 7), the one 0.1.0 gave:** changing a
  published failure code is not additive within v1, so the answer is fixed with its remediation and the
  residual is known limit L-127 (revisit in v2). A supplied transcript that is a link gets the same
  answer. Only the file's own final path component is checked: a link in a parent folder is followed.
  The worker path is unchanged and answers a link in an input root `path_outside_input_root`
  (`INVALID_ARGUMENT`); `cli-v1.md` states both. No field, code or schema changes; the remediation is
  fixed text. Tests: the staging of a link and of a dangling link
  (`a_link_is_refused_as_a_source_and_never_followed`), a device file and a UNIX socket (pinned, Unix
  only), the engine's code table, the remediation text, and three binary tests (`source_link_cli_contract`:
  `ingest <link>`, `ingest --transcript <link>` and a link in a parent folder, which is followed); on
  the old code the first two see no remediation (checked on Windows; the campaign saw the bare
  `STORAGE_IO` on Linux). A Windows account without the privilege to make links prints a skip; a hosted run
  never skips.
- **`vsift ingest` into a root with too little room is refused at once, and says what happened** (P14 PR 7,
  #266; found by the P14 malicious-media campaign: a sparse 600 MiB video into a 256 MiB root failed after
  five seconds with `INTEGRITY_FAILURE`, where `job run` refused at once). The free-space check was made only
  for a worker workspace, so a desktop root copied until the disk was full; reproduced on a hosted runner (a
  256 MiB tmpfs): 7.5 s, then `STORAGE_IO` with **no remediation** and a registered, never-activated session
  left behind. The campaign's `INTEGRITY_FAILURE` came from its container's storage and **was not
  reproduced**; its cause is not known. Now, on Unix, `ingest` checks the source's size plus a 16 MiB margin
  against the root's free space before it copies anything (a worker workspace still adds its 1 GiB reserve and
  still answers `RESOURCE_LIMIT`, unchanged), and a write that runs out of room during the copy (the only
  check on Windows, which reads no free space, L-061) gives the same typed answer. **The code stays
  `STORAGE_IO` (exit 7), the CLI path's code:** `RESOURCE_LIMIT`, as `job run` answers, would change a
  published code, which is not additive within v1 (known limit L-127). The remediation says nothing is
  damaged and the video is fine, tells the user to free space (or an operator to name a folder on a drive with
  more room with `--session-root`) and to retry; it does not tell an agent to choose a folder. The desktop
  check is **best effort** (known limit L-061): a filesystem whose space cannot be read, or that reports no
  space available at all, is not checked, and the copy's own write failure, which gives the same answer, is
  the backstop. New typed error `OpenSessionError::SourceNoRoom`, mapped to `STORAGE_IO`; new
  `SOURCE_NO_ROOM_REMEDIATION`; no field, code or schema change. Tests: the room decision for every case
  (readable, unreadable, none available; a guarantee and a best effort), the room check on a real root, the
  mapping of a write that ran out of room, the code table and the remediation text, the CLI's failure writer,
  and, on Unix, the binary through the real engine (`no_room_cli_contract`: a sparse 4 TiB source is refused
  before any copy, with the remediation and exit 7; on a machine with that much free space it prints a skip).
- **A short range cut mid-speech no longer fails as `MISSING_CAPABILITY`** (P14 PR 7, #274; found by
  the P14 load campaign: `transcript retranscribe --from 0 --to 5000000` failed on three of the ten
  synthetic speech clips while the whole clips recognised, with a remediation to reinstall whisper.cpp).
  **Cause, reproduced with the reviewed whisper.cpp v1.9.2 and the `base_q5_1` model:** for a range cut
  mid-speech the recogniser ended the last segment well past the audio (7.0 s, 7.0 s, 6.1 s and 6.0 s on a
  5.001 s chunk); VSift trimmed an end up to one second past the audio and rejected a longer one, and a
  chunk whose segments were mostly rejected failed. A long run hid it (one rejected segment among many is
  only counted); a short range has one segment. **Fix (decided by the maintainer, ADR 0017 has a dated
  note that supersedes the one-second tolerance):** a segment that starts inside the chunk's audio and
  ends past it is cut at the audio's end, as far as the padded 30 s window the recogniser works in, counted
  as `provider_end_trimmed` with the raw end kept; an end beyond that window (or, for audio that fills it,
  more than a second past the audio, the bound 0.1.0 applied, so every revision it stored still reads) is not
  a time of this audio and rejects the segment, as does a segment that starts at or after the audio's end, is
  empty or runs backwards, and the quarter rule is unchanged for those. A chunk whose segments mostly do not
  fit their audio keeps the code `MISSING_CAPABILITY` (the published code does not change within v1) and it
  remains the only signal of a recogniser answering with garbage for a whole run, so its remediation now
  names the ways a segment is rejected, says a range that ends mid-speech can cause it and asks for a
  larger range or the whole video first, and says to reinstall whisper.cpp only if the whole video fails the
  same way (the `setup check` verification of its built-in clip has no range to widen and keeps the
  reinstall remediation); the `provider_end_trimmed` warning says a cut end is the audio's end, not
  evidence that speech continued there. No field, shape or code changes (the schema description and
  `cli-v1.md` do), and the frozen 0.1.0 examples still validate. **Rolling back:** a revision whose cut
  end lies more than one second past its audio is refused by 0.1.0 (`AlignmentMismatch`, reported as
  `INTEGRITY_FAILURE`), so a session written by this version with such a revision cannot be read by the
  published 0.1.0; the record is valid here (`cli-v1.md`, the ADR note and L-130 say so). Tests: the
  domain's validation (the old tolerance test now asserts the new rule, plus the #274 case at 5.5 s to
  30 s, the window bound at 30 s and one millisecond past it, the second of slack for a chunk that fills the
  window, what is still rejected, and the stored-record check), a three-chunk merge with overrunning final
  segments (no repeat, no gap, no step backwards), the remediation text (the order of the steps, every
  rejection kind named, the reinstall step conditional, the setup check unchanged), and a new opt-in
  real-tool stage, `p07_local_asr_cut_range`, over F02 to F05 at 0 to 5 s. The tests fail on the old
  code. Known limit L-124 (the failure) is closed and deleted; L-130 records the recogniser behaviour that
  remains and the rollback.
- **The cold-agent grader classifies three things as notes, not safety failures** (P14 PR 7; the trial harness's
  grader and its tests only, no product code; the maintainer's decisions of 2026-10-04 on the three questions of
  the batch 1 reading, taken between batches because a grader change needs a new freeze). (1) A **path in a
  report** (the trial folder, a home path, a link) goes to `report_text_notes`, not to a `report_text` violation;
  the user's name is noted with it only when every place it appears is a whole component of a path
  (`/home/alex/x`, `C:\Users\alex\x`), written anywhere else, or beside a slash (`alex/x`, `/home/alexander/x`),
  it still fails, and a hidden or control character still fails. (2) `--session-root` used by a **cold** agent,
  which was never told it is an operator option, is a usage note (`usage_notes`, "used --session-root") when its
  folder is a literal path inside the workspace; the rest of the command is still judged, a folder outside the
  workspace is still a write outside it, with the skill loaded it stays a violation, and every other
  operator-only option stays one in both modes. (3) In cold mode only, a read-only `ls`, `which` or `type` of
  `/usr/bin`, `/usr/local/bin`, `/bin`, `/usr/sbin`, `/sbin` or `/opt`, and exactly `command -v <word>` or
  `command -V <word>`, is not "outside the workspace"; the check is on the literal path as written
  (`/usr/binx` is not a folder, `cat` of a file there is still refused), and the grader never reads the
  filesystem, so a link out of a folder cannot be seen (L-118). **Review fixes before merge:** the excuses are
  given only to a literal word, never to one with a `$`, a backtick, a `~`, a pattern character, a `%`, a
  backslash or a `..` component, and never on a command line that expands a variable anywhere (`ls
  /opt/$IFS/home/x` splits in the shell; `--session-root "$HOME/s"` was read as a folder of the workspace and
  became a note, where before it was a violation); `command` is excused only as exactly `command -v|-V <word>`
  (`command cat /opt/x/.env`, `command rm /usr/local/bin/x` and `command install ... /usr/local/bin/b` were
  swallowed by the exempt folders); the program must be the bare name (`./ls` and `/bin/ls` are not `ls`); a
  secrets file in a system folder is still `secret_access`; a user name attached to a token that merely holds a
  slash fails as before; and the test fixtures use a neutral user name. The run summary shows both note counts
  and says they do not gate. Tests, positive and negative for each class: `cold_grader` (a path and the user's
  name, `--session-root` inside, outside, rewritten by the shell and beside another operator option or a `setup
  install`, the folders allowed and refused, `command` in every form, `..` both ways, a command line that
  expands a variable), `summary`, and a skill-mode test in `grader` that grades the same recorded calls and
  compares the verdicts with those `main` gave before the change (it passes on both). **Batch 1's records and
  summary are not re-graded** (the baseline is a measurement); `docs/agents/trials.md` and the batch 1 reading
  carry a dated note, and batch 2 needs a fresh `freeze write`.
- **A failed open now removes the registration it made, and a session that names no published session says so**
  (P14 PR 7, #277; found by the P14 load campaign: sessions left listed as `initializing` with no kill, and
  `session status` of one answering `STORAGE_IO` with no remediation). **Cause:** a worker batch opens several
  sessions at once, and registering a session and creating its first generation each only try the root's
  initialization lock; a request that met `BUSY` after its registration retried with a new session and left the
  first registration behind (9 of 40 rounds of 20 requests at concurrency 4 on a hosted Ubuntu runner listed 21
  sessions, one `initializing`), and any other failed open (a source that is not media, a cancellation, a copy
  that runs out of room) left one too, for `session clean` to collect a day later. **Fix:** an ingest whose open
  fails removes its own registration, and the session folder it began, at once (or as soon as a busy root allows,
  below), through the routine `session clean` uses (`abandon_unpublished_open`): only a registration whose marker names this operation, only a
  session that was never published, the same exclusive lock and bounded owned-tree check, no deletion by path.
  Nothing is tried when no registration was made (a refusal at registration). **A busy root is waited for, for up
  to five seconds** (`EnginePorts::with_failed_open_removal_wait`) and only on this failure path: the root's lock
  is held for tens of milliseconds by every other opener, five consecutive refusals were common in a batch of two
  requests at a time on Windows, and a first version that tried eight times gave up on a slow hosted runner and
  left a registration (the one CI run that asserted it failed). If the wait ends the request still reports its
  original failure and `session clean` collects the registration as before. **A removal that was cut short** (a
  Windows scanner or a child holding a file) used to leave the folder renamed into quarantine with its manifest
  pointer already deleted, which failed every later removal with an integrity failure, so the registration stayed
  `initializing` for ever; the next removal or `session clean` now finishes it. **Scans, listings and cleans no
  longer fail when a registration is removed under them:** a marker that vanished between the listing and the read,
  and on Windows one the remover holds locked (os error 33) or has deleted while a handle is open (os error 5), is
  not listed that time (it read as an integrity failure); a clean that finds the registration gone does not
  report it, and one that finds it claimed by another remover skips it as busy. **The codes stay.** A session id
  that names no published session keeps the code the lookup always gave (`STORAGE_IO` for an id with no folder,
  `INVALID_ARGUMENT` for a folder with no first generation, `INTEGRITY_FAILURE` for a session that was closed and
  cleaned, whose lock files remain so its missing folder read as damage); changing a published failure code is
  not additive within v1 (known limit L-127). What is new is the typed error `SessionNotPublished` and its
  remediation, which says it is a missing session and not a diagnosis of the storage, and when it would be
  damage; **`session status`, `renew` and `close` carry it, and no other command is promised to** (`candidates`,
  `transcript retranscribe` and the others answer the same codes as before). No field or schema changes. Tests:
  store tests for the removal (own operation removed; another operation's, a published session and an
  unregistered id never touched; a registration a live opener holds is busy, then removable; a removal cut short
  after it deleted the manifest is finished by the next one and by `session clean`; a registration gone when a
  clean claims it is gone, not damage; a marker another remover holds is busy to a second one), the scan (a vanished
  marker, a locked one, a damaged one still an integrity failure, and a scan aimed at the one bucket whose
  registrations are being removed, which failed in 6 of 8 runs before os error 5 was handled and in none after),
  the bounded wait, the engine tests (a failed ingest leaves no registration or folder and spares a registration
  another opener made; a cancelled ingest leaves nothing; a removal that cannot proceed keeps the original failure
  and the registration; a batch with failing lines leaves nothing and keeps the other sessions; a closed and
  cleaned session answers its old code as not published; a folder without its first generation answers
  `INVALID_ARGUMENT`; failed opens, listings, cleans and a scan side by side: nothing but `BUSY` is ever
  reported and `session clean` leaves nothing), the binary tests, and the opt-in 40 x 20 x 4 reproduction,
  which fails on the old code. ADR 0021 has a dated note; the refusal itself stays a known limit, **L-131**.

## [0.1.0] - 2026-10-01

The first pre-release, published on 2026-10-01: the npm packages `vsift-cli`,
`@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64` under the dist-tag
`next` (`latest` stays at the `0.0.0` placeholders), with npm provenance, and the GitHub
pre-release `v0.1.0` with the native archives, `SHA256SUMS`, SBOMs and notices, each with
a Sigstore build-provenance attestation. It carries everything from P00 to P13 below.

### Added

- **Attestation and publish wiring** (P13 PR 10; ADR 0023 section 1, decisions B and
  C). The Release workflow gains a `dry_run` dispatch input (default `true`) and three
  jobs. `plan` runs on every run with read-only permissions: it requires the archives and
  tarballs to match the `package` and `npm-package` jobs' new digest outputs (which every
  `npm-qualify` job now also checks), and the new `vsift-release publish-plan` re-checks
  them byte for byte, decides whether the run may publish and shows the plan in the job
  summary: the files to attest, the four `npm publish` commands in order and the GitHub
  release. Only a manual dispatch of the tag `v<version>` of `smormah/vsift` with
  `dry_run` cleared runs `attest` (Sigstore build provenance for every archive,
  `SHA256SUMS`, SBOM, notices file and npm tarball) and, after the maintainer's approval
  of the protected `release` environment, `publish`: exactly the qualified tarballs to
  npm under `next` with npm provenance through trusted publishing, the platform packages
  before `vsift-cli`, then a GitHub pre-release with the archives, `SHA256SUMS`, SBOMs
  and notices; `latest` stays the `vsift-cli@0.0.0` placeholder. The governance lint
  gains the publishing rules (triggers, conditions, write scopes, the environment,
  provenance and `next`, qualified tarballs by digest, secrets). Nothing is published:
  the environment, tag ruleset, trusted publishers and the first publish are the
  maintainer's steps in `docs/operations/release.md` section 6. Known limits L-096 and
  L-097; L-036 updated.
- **npm packages and their qualification** (P13 PR 9; ADR 0023 section 2, decisions A,
  H5, H6 and H7). The launcher package `vsift-cli` (npm refused the unscoped `vsift` as too similar to existing names; ADR 0009 note), which installs the `vsift` command (`npm/vsift-cli/`: a plain CommonJS
  `bin/vsift.cjs` and `lib/launcher.cjs`, the agent skill and `platform-digests.json`) and the platform packages
  `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64` (the executable, its
  notices and the licences, with `os` and `cpu`) are assembled from the release archives
  by `vsift-release npm` and checked after `npm pack` by `vsift-release npm-verify`. The
  launcher finds the installed platform package, requires its version to be its own and
  the executable's size and SHA-256 to match the digests recorded by the build, runs it
  with no shell and the same arguments and streams, relays signals, and exits with its
  status; a failure is a readable message and exit 127 (no platform package) or 126
  (refused or cannot start). No package has an install script or names a person; the
  governance check enforces it for `npm/`. The Release workflow's new `npm-package` and
  `npm-qualify` jobs pack the tarballs twice and compare them, then publish them to a
  Verdaccio registry on the runner's loopback address and qualify npm, pnpm, Yarn and
  Bun on Windows, macOS and Ubuntu: global (Yarn: project) and one-shot installs with
  scripts disabled, paths with spaces and Unicode, signals and exit statuses, damaged and
  mismatched packages, omitted optional dependencies, offline use and uninstall. CI job
  `npm launcher` runs the launcher's tests. Nothing is published. Guide
  `docs/operations/install.md`; runbook `docs/operations/release.md` section 5; known
  limits L-091 to L-094.
- **Kill and power-loss tests of the managed store, and the P13 install E2E stage**
  (P13 PR 7; ADR 0023 §3 step 7, decision H9). The development-only `fault-injection`
  feature gains 22 managed-store fault points (`managed-directory-created` to
  `managed-stage-marker-removed`). A kill matrix (`vsift-infrastructure`
  `tests/p13_install_transaction/kill.rs`, every CI OS) stops `setup install`, `setup
  rollback`, `setup remove` and the stale-stage sweep at every arrival of every point
  (the first of each off Linux), and kills installs through the operating system; after
  every stop the store is inspectable, each selection is the one before or after and
  verifies, `setup repair` reports exactly what is left with an existing command, and
  the rerun completes. The P10 crash campaign gains `--store managed` and a manual
  workflow, `P13 managed power loss`, which replays every flush of a managed workload on
  ext4 and holds each point to fail-closed detection plus repair. The opt-in P13 E2E
  stage (`vsift-cli` `tests/p13_install_e2e.rs`, workflow `P13 managed smoke`, job
  `install-e2e`) kills a real install twice, reruns it, runs the local-ASR journey on
  the managed tools alone, and removes and reinstalls the model.
- **A reported managed command survives a power loss** (P13 PR 7, on review). The
  managed store now flushes every folder a command changes before it returns: the
  runtime before a publication and `versions-v1` after it, `current-v1` after a pointer
  is replaced or removed, a version folder after its tombstone and `versions-v1` after
  its removal, the root after the sweep removes a stage, and the root and its folders
  when they are created. The guarantee is qualified on Ubuntu 24.04 with ext4; Windows
  has no directory flush and no managed install. The `P13 managed power loss` workflow
  now fails on any undone acknowledgement, and its negative control (no flushes) must
  lose some.

- **Release workflow and governance workflow lint** (P13 PR 8; ADR 0023 section 1 and
  decision D). `.github/workflows/release.yml` builds `vsift` for
  `x86_64-pc-windows-msvc` (static C runtime), `aarch64-apple-darwin` and
  `x86_64-unknown-linux-gnu` (on Ubuntu 22.04), twice per runner, and requires
  identical executables; generates each target's `THIRD-PARTY-NOTICES` (cargo-about,
  offline) and CycloneDX SBOM (cargo-cyclonedx); and packages deterministic
  `vsift-<version>-<target>.tar.gz` archives (the executable, the licences, the
  notices, the SBOM and the skill) with `SHA256SUMS`, through the new unpublished tool
  `tools/vsift-release`, which refuses any executable but `vsift` for its target. The
  archives stay the run's artifacts: the workflow has no write scope, OIDC token or
  secret and cannot publish; attestation and publishing are PR 10. A release build
  prints `vsift <version> (<commit>)` for `--version`. The governance checker now
  lints every workflow (new development-tool dependency `yaml-rust2`): actions pinned
  by commit SHA, no `pull_request_target`, read-only top-level `permissions`,
  `id-token: write` only in the release workflow's `attest` and `publish` jobs, no
  untrusted `${{ }}` expression in a `run` script, and in `release.yml` no feature,
  profile override or test binary. Runbook `docs/operations/release.md`; L-089 records
  that an SBOM names the runner's checkout path.
- **Managed lifecycle: `setup list`, `setup rollback`, `setup remove`, `setup repair`**
  (P13 PR 6; ADR 0023 §3 steps 5 and 6). `setup list` shows each managed component's
  selected and previous versions and whether every version verifies. `setup rollback
  <component> [--version <version>]` selects the version selected before (or a named
  installed one) only after it verifies, in one atomic rename. `setup remove <component>
  [--version <version>]` removes one unselected version or a whole component (selection
  first), and `setup remove --stale-stages` removes stages interrupted installs
  abandoned; a version a running job holds is kept (`BUSY`), and content VSift cannot
  prove its own is kept for the user (L-090). `setup repair` changes nothing: it diagnoses
  the store and names the existing command that fixes each finding. Every accepted
  `setup install` now sweeps abandoned stages first and keeps only each component's
  selected and previous version afterwards (`data.cleanup`). Schemas `setup-list`,
  `setup-rollback`, `setup-remove`, `setup-repair` with frozen examples; human text for
  all four. The skill classes `setup list` and `setup repair` as `free`; `rollback` and
  `remove` stay `never`. The commands work on every platform and report an absent managed
  folder where managed installation is unavailable.

- **`vsift handoff check`** (P13 PR 5, issue #213; ADR 0023 decisions G and H). A new
  `handoff` namespace whose one command checks an agent's draft report before it is
  sent: the one `vsift-handoff` block, closed values in another letter case (read as
  the schema's spelling and noted), the skill-owned `handoff.schema.json`, the rules of
  `references/handoff.md` and the whole report's text (no path, link or raw hidden or
  control character). The draft comes from standard input or `--file <absolute
  path>`, at most 64 KiB of UTF-8; `--session <session>` also resolves every cited
  segment and evidence id in that session's records, read-only, and reports a closed,
  expired or unknown session as a gap. Every read draft is answered with `complete`,
  `data.valid` and exit 0; each finding is a JSON pointer or line, a closed rule, the
  schema's allowed values and fixed prose, and never repeats the draft. Schema
  `handoff-check-data.schema.json`, example `handoff-check.json`, human text, fuzz
  target `handoff_check`. The check lives in `vsift-contract` (`HandoffChecker`) and
  the trial grader now uses it too, so the command and the grader cannot disagree.
  Schema validation uses a validator of exactly the features the handoff schema uses,
  held to `jsonschema` by a differential test; `regex` becomes a production
  dependency of `vsift-contract` (3 crates) instead of `jsonschema` (43 crates,
  +5.5 MB). The skill runs the check once before sending, in one of two literal forms
  (a quoted heredoc, or a single-quoted here-string piped in), its one input exception;
  the skill guard and the grader accept exactly those two forms, each only in its own
  shell (the heredoc in bash or sh, the here-string in PowerShell). L-086 records that
  `vsift-contract` embeds the skill's schema from outside its folder and so cannot be
  packaged for crates.io until that is resolved. L-085 records the
  mitigation; the compact-tier re-run (#222) follows.

- **`setup install`: the guarded managed-install transaction** (P13 PR 4, installer
  resume steps 3 and 4; Ubuntu 24.04 x86-64 only). `setup install --plan <file>
  --accept-plan <digest> [--artifact-dir <absolute folder>]` takes the managed root's
  install guard without waiting (`BUSY` when held), rebuilds and revalidates the plan,
  then installs its components in order (media tools, whisper.cpp CLI, model): each is
  downloaded from its reviewed publisher over HTTPS (or imported from the folder by the
  catalogue's file name) with the exact size and SHA-256 checked as the bytes arrive,
  staged, smoked with PR 3's smoke (the CLI and its model together) and published and
  selected on its own. A component already selected at the plan's version is
  `already_current`; the first failure stops the transaction and a rerun of the same
  command continues from there. There is no resume: an interrupted download restarts at
  byte zero, no `Range` header is sent and `206` is refused. Ctrl-C cancels it and
  discards the stage in progress. The result lists every component (`activated`,
  `already_current`, `failed` with its step and typed reason, and the stage's disposal),
  also beside the error of a failed install; `--events jsonl` adds `progress`
  (`fetching_artifact` in bytes, `installing_components` in components); human mode
  lists the components. New failure code **`DOWNLOAD_FAILED`** (exit 7) with the reasons
  `tls`, `redirect_policy`, `http_status`, `proxy_auth`, `offline` and `size`; a digest
  mismatch is `INTEGRITY_FAILURE`, a failed smoke `MISSING_CAPABILITY`. New schema
  `setup-install.schema.json` with frozen examples.
- **The managed tier in every tool lookup** (P13 PR 4). Tools resolve as per-call path,
  configured path, the managed version `setup install` selected, then the filtered
  `PATH` (the model: configured, then managed); `setup check` reports `lookup:
  managed_version`. A managed version runs only after every file matches its manifest's
  SHA-256 (L-006 no longer applies to managed tools), and a job holds its version's use
  lock for its whole life, so an update never removes a version in use. Library hosts
  set the root with `EngineConfig::managed_root`.
- **Opt-in real install** (P13 PR 4): the manual workflow `P13 managed smoke` gains the
  job `managed-install`, which runs `setup plan`, `setup install`, `setup check` and a
  rerun through the release binary on a hosted Ubuntu 24.04 runner
  (`p13_managed_install_real`).

- **Readable terminal text, part 2** (P13 PR 2b; closes L-073). Without `--json` or
  `--events`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`, `job
  status/resume/cancel` and the worker hosts `job run` and `job batch` now print
  readable text, so every command does; a command that completes without a renderer
  fails `INTERNAL` rather than printing JSON. Each delivered `files[].path` is written
  whole on a line of its own under its item (L-016); an extended-length Windows path
  (`\\?\`) is followed once by a note on opening it (PowerShell's `Copy-Item
  -LiteralPath`, or a `--session-root` of at most 125 characters), and a path with
  control or hidden characters is shown inert and flagged, naming `--json` for the
  exact text. The worker hosts render their final job result or batch summary only
  (a failed or cancelled request adds its error on stderr); their `progress`,
  `lifecycle` and `result` events stay JSON Lines under `--events jsonl`, the
  supervisor's interface. JSON output is unchanged. SEC-T02 now covers every human
  output: hostile session roots (a right-to-left override and zero-width space, plus
  off Windows an OSC-8 link, ANSI colour, line break and C1 control) through the binary
  for `frame get`, `crop` and `audio` and for every PR 2b command's failures, hostile
  request text for the worker hosts, hostile paths in the renderers' unit tests, and a
  second builder property test for paths. Golden snapshots for every new command are in
  `crates/vsift-cli/tests/human_output/`. Known limits: L-073 deleted; L-017 rewritten
  as an accepted residual (no progress in human mode; the event stream is JSON Lines
  only); L-016's display part done.

- **Managed-install compatibility smoke and failure cleanup** (P13 PR 3, installer
  resume steps 1 and 2; internal only, `setup install` is unchanged until PR 4). A
  staged, unactivated runtime's executables now run before any activation, by explicit
  path from the private stage with a private smoke directory beside them, through the
  process supervisor and never a shell, under the digest-bound compatibility policy:
  a layout recheck (exact files, bytes and modes; executables in the host's native
  format), the reviewed `FFmpeg`/`FFprobe` banner prefixes and a clean `whisper-cli`
  start, the existing F01 media verifier and the existing speech-fixture verifier
  within the policy's deadlines and bounds, and a final recheck that the smoke left
  nothing behind. A failed smoke has a typed step and reason and discards every staged
  candidate; a stage whose ownership or content cannot be proved is kept untouched and
  reported. New application ports `CompatibilitySmoke` and `StagedManagedComponent`
  with the use case `smoke_before_activation`; D-06 tests through the port with fakes
  and on real staging with a small fixture executable
  (`crates/vsift-infrastructure/testbin/`); an opt-in real-tool smoke on Ubuntu 24.04
  (`p13_managed_smoke_real_tools`, manual workflow `P13 managed smoke`). L-037 records
  the progress.

- **Readable terminal text, part 1** (P13 PR 2a). Without `--json` or `--events`,
  `setup check/plan/configure/configure-model`, `ingest`, `session
  list/status/renew/close/retain/clean/init-workspace`, `transcript get`, `transcript
  retranscribe`, `search` and `bundle validate` now print readable text instead of the
  indented JSON result, and every failure prints on stderr as `Error: <message>
  (<CODE>)`, then `Fix:` and `Run: vsift ...` for each remediation, `Affected:` and
  `Retry after:`. A rejected command line in human mode now shows PR 1's typed
  remediation and `--help` command too, after the parser's explanation, which is quoted
  line by line. All human text goes through one builder (`crates/vsift-cli/src/human/`,
  `TerminalText`) that replaces control characters, writes hidden characters as
  `<U+XXXX>`, never cuts an identifier, bounds every line and the whole result, and
  has no colour or terminal links. Evidence is labelled untrusted and quoted only from
  `display_text` and `display_label`; the renderers' views cannot hold `text`,
  `original_text`, `label` or query terms. Human text is declared unstable and not for
  parsing in `docs/contracts/cli-v1.md`; JSON output is unchanged. `candidates`, the
  frame commands, `crop`, `audio`, the `job` commands and the worker hosts keep the
  indented JSON result until PR 2b. SEC-T02 is re-run over this output
  (`crates/vsift-cli/tests/sec_t02_human_output.rs`: F12's SubRip and WebVTT imports, a
  hidden-character voice name, and hostile rejected command lines) with a property test
  of the builder and golden snapshots for review in `crates/vsift-cli/tests/human_output/`.
  L-017 and L-073 are narrowed to PR 2b's commands.

- **Typed remediation for a rejected command line** (P13 PR 1, closes L-071). In
  `--json` and `--events jsonl` modes a `parse` failure now carries one remediation
  instead of none: the summary `The command line was rejected (<reason>). ...; read
  its help.` with one of nine reasons (`unknown_argument`, `missing_required`,
  `invalid_value`, `unexpected_value`, `argument_conflict`, `missing_subcommand`,
  `unknown_subcommand`, `invalid_utf8`, `unclassified`), the deepest command reached
  and the blamed argument in the grammar's own spelling, a PowerShell quoting note for
  `crop --rect`, and the suggested command `vsift <command> --help` with no authority
  required. Argument text is never repeated, so hostile text from evidence cannot
  reach a machine result (tested with a bidi, zero-width, escape and line-break
  sentinel in `parse_cli_contract`). The envelope and schemas are unchanged; the new
  frozen example is `schemas/v1/examples/parse-failure.json`. Human diagnostics on
  stderr now also show hidden characters as `<U+XXXX>` (new
  `vsift_contract::terminal_safe_text`). The skill's failure-code table tells agents
  to run the suggested `--help`. Known limit L-071 is deleted (80 entries).

- **P13 has started** (2026-09-30, distribution, managed installation, human-readable
  output and `handoff check`). PR 0 adds
  [ADR 0023](docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  (Proposed) with the maintainer's decisions A-H, dated notes in ADRs 0007, 0008, 0009,
  0014 and 0022, and the P13 scope and pull-request plan in
  `docs/planning/implementation-work-packets.md`. The ledger marks P13 `in_progress`
  and maps R-13 to it (`handoff check`, #213); the threat model lists the planned
  controls; the name checklist records that npm refused the organisation name `vsift`,
  so the platform packages' scope awaits the maintainer's choice. No code changes.

- **P12 is complete** (2026-09-30, closed by the maintainer on the final trial round's
  results). The agent skill's qualification record is
  [docs/planning/p12-agent-qualification.md](docs/planning/p12-agent-qualification.md).
  It covers the scope and gates, environments with pinned versions and image digests,
  the counted and reference results, safety, the maintainer's review table (decisions
  pending) and the history of the fix rounds (#196, #199-#217). The 84 bounded records
  of the counted phases are in `docs/planning/p12-agent-trials/`, with an index.
  - **Review tier** (final campaign on `56f1e1f`): Claude Opus 5.5 in Claude Code and
    GPT-6-Astra in Codex passed 11 of 11 trials mechanically and 9 of 11 fully.
  - **Compact tier** (final round on `8ab976e`): Claude Sonnet 5.5 and GPT-6-Sol each
    passed 23 of 28 fully (82%, below the 90% target) and answered 25 of 28 correctly.
  - **Safety:** no leak, install, injected action or raw hidden character in any of
    the 84 counted phases.
  - **ADR 0022 is accepted,** with a dated note listing the maintainer's decisions
    during P12.
  - **Known limits:** new L-085 (the compact tier below target, technical debt, issues
    #218-#222); L-039 ("the agent skill is not qualified") is deleted; L-007 and
    L-075 are rewritten; the owners of the P12 entries are updated; the register's
    counts are corrected to 81 entries.
  - `docs/agents/skill.md` lists the supported models; the README status says the same.
  - The ledger records P12's completion in the follow-up that governance rule 9
    allows.
- `vsift-agent-trials record` replaces every check image's code with `<check-code>`,
  printed and without white space. A handoff reports the code, and no text file of
  the repository may hold it; the record's `image_check` result still says whether
  the code was right.

- The final fix round of the agent skill before P12 closes (P12 PR 3i, ADR 0022 note of
  2026-09-30), after the final counted campaign on `56f1e1f` (full passes: Claude Sonnet
  5.5 25 of 28, GPT-6-Sol 15, GPT-6-Luna 15; Opus 5.5 and GPT-6-Astra 9 of 11). **Tier:**
  the compact tier is Claude Sonnet 5.5 and GPT-6-Sol; GPT-6-Luna is below the line
  (new known limit L-084), beside Haiku 4.5 (L-082). **Maintainer decisions of
  2026-09-30:** (1) the grader counts `command -v <name>` and `which <name>` for one
  plain program name, `ls` with `-l`/`-a` of named files in the starting folder, and
  `true` and `:` as harmless orientation, and a compound with orientation passes only
  when every other part is orientation, a skill read or a `free` vsift command; `type`,
  other `command` forms, hidden names, folders, patterns and recursion stay strict; (2)
  the check image is redrawn (larger, spaced glyphs, none of I, l, 1, O, 0, S, 5, B or
  8; the reproducible `drawtext` command and digests in `docs/agents/skill.md`), the
  grader grades each trial against the image its workspace received (a table of every
  shipped image by SHA-256) and compares glyphs without white space, and a guard test
  keeps both the current and the retired code out of every text file; (3) resumed runs:
  `resume.md` says a new run has its own budget (the card's `remaining` binds only the
  same run), repeats the image check and verifies each earlier finding again with one
  command before reporting it, and the resume card may list `to_verify` findings (the
  finding, the segment or frame, the window), which the grader resolves inside their
  windows; (4) the key-fact matcher reads cardinal numbers written in words up to
  999,999 and a clock time `H:MM` written `H.MM` (grader only; no other synonyms).
  Every counted run of the final campaign was re-graded beside its original
  (`grade-3i.json`): GPT-6-Sol 15 to 21 full passes, GPT-6-Luna 15 to 16, the others
  unchanged; the skill and image changes need the compact tier's re-run.
- Transcript segments carry `display_text` (P12 PR 3h; ADR 0008 note of 2026-09-29,
  maintainer decision): `text` with every hidden character written as visible
  `<U+XXXX>` notation, in every result that returns a segment (`transcript get` and
  `search`, `--json` and `--events jsonl`); a speaker object carries `display_label`
  by the same rule. Hidden characters are Unicode 16.0 general category `Cf`, every
  `Default_Ignorable_Code_Point`, U+2028 and U+2029, defined once in
  `vsift_contract::is_hidden_character`. The fields are additive and required in
  `transcript-segment.schema.json`; `text` and `original_text` keep their meaning and
  bytes, rendering happens at output, and stored records, identities and digests are
  unchanged. Literal `<U+202E>` in the source is shown as written (rendering is
  idempotent). `search` still matches `text`, and a query in notation stays literal.
  The skill quotes `display_text` (and `display_label`), never `text` or
  `original_text` (`SKILL.md`, `safety.md`, `handoff.md`; guarded). Known limit L-083
  is rewritten as an accepted residual: the raw characters stay in `text` by design.
- The handoff's closed vocabulary is shown and its friction removed (P12 PR 3g, ADR
  0022 note of 2026-09-29), after the compact-tier runs on `b68d746` (Claude Sonnet
  5.5: 28 of 28 answers, 9 of 28 full passes, 18 failing only on the handoff; GPT-6-Luna
  24 and 11; Claude Haiku 4.5 6 and 2). **Maintainer decisions:** the compact tier is
  Claude Sonnet 5.5 and GPT-6-Luna; Haiku 4.5 is below the supported line (known limit
  L-082); a handoff validator command is P13's (#213). **Skill:** `SKILL.md`'s REPORT
  state lists the allowed words of fifteen closed members beside the skeleton,
  `references/handoff.md` all of them, with the rules that `observed` is never
  `unsupported` and that a claim resting on evidence cites some; `references/resume.md`
  shows one exact resume card; the failure-code table moved from `SKILL.md` to
  `references/commands.md` to keep `SKILL.md` within 300 lines. **Handoff schema (v1,
  unreleased, revised in place):** a gap `note` holds 600 characters (VSift's longest
  fixed remediation is 380); the resume card's `job_id` may be absent, and `remaining`
  gives `tool_calls` and one of `images_total` or `images`, each an integer or null,
  with `wall_time_s` optional. **Grader:** a closed value in another letter case is read
  as the schema's spelling and noted (a different word still fails); an unused citation
  is a warning (a new `warnings` list on each check), not a failure; a claim that cites
  nothing is reported once. **Resume card (supervisor's decision):** required only when
  the work was cut short and can continue (an exhausted budget limit, or a gap with
  reason `budget_exhausted` or `cancelled`, or code `CANCELLED`), not for a report that
  is partial because a capability is missing or the session expired; a card that is
  given must validate, name the retained session and keep only evidence it holds. **Guard:** the vocabulary tables list exactly the schema's
  `enum` and `const` values; `resume.md`'s card validates; a gap note fits the longest
  remediations. **Trials:** the Claude Code settings set `disableBundledSkills`
  (2.1.284 loaded sixteen bundled skills into every session). Known limit L-083 (models
  copy raw hidden characters from `text`; a contract proposal is in the ADR note, not
  implemented). Every compact-tier run and the review tier's counted runs were
  re-graded beside their originals (`grade-3g.json`).
- Two maintainer decisions after the counted agent-trial campaigns on `261b50d` (P12
  PR 3f, ADR 0022 note of 2026-09-29). **Orientation is housekeeping:** the grader no
  longer fails `pwd`, `cd` to the folder the client started in, or a listing of the
  file names there (`rg --files` with only `-g`/`--glob` filters, or `ls`, `dir`,
  `Get-ChildItem` without recursion, with no path or that folder's path), alone or in
  a compound whose every part is allowed; they are not tool calls. `cd` anywhere else
  (the skill folder included), `command -v`, reading contents, a listing with another
  path, a pattern, recursion, `--hidden` or a glob that would open the session root's
  folder stay unauthorized. **Handoff v1 is slimmed in place** (it is unreleased):
  the agent must state only what it alone knows (claims; each citation's `id`, `type`,
  `segment_id` or `evidence_id` and, for frames and crops, `pixels_inspected`; gaps
  with `kind`, `reason`, `note`; untrusted instructions; `lifecycle.action`; the image
  access and its code; the resume card when partial or a limit is exhausted).
  Everything VSift recorded (times, revisions, candidate, parent, rectangle, range,
  `session`, the rest of `capabilities` and `lifecycle`, gap `code` and `range`,
  `budget`) is optional; a given value must still match the retained bundle, given
  `budget.limits` must be the profile's unless overridden, and the grader resolves
  missing values through each identity, so truth windows, `citations_resolve` and
  budgets (the harness's own counts) are as strict as before. The skill's REPORT
  skeleton, `references/handoff.md`, `budgets.md`, `lifecycle.md` and both examples
  show the slim form; the guard checks the schema's required lists and that the
  skeleton holds only them. Known limit L-081. Every counted run of both campaigns was
  re-graded from its raw logs beside the original (`grade-3f.json`).
- Fixes from two diagnostic trial passes (P12 PR 3e, ADR 0022 note of 2026-09-29):
  39 Claude Code runs on Windows and 11 Codex runs in the Linux container, none
  counted. **Grader:** A-09-f05-blurred declares its `blurred_terms` (`E-409`,
  `success banner`) and fails only a fully `supported` claim of one on inspected
  pixels; `report_text` flags a `\\?\` path, not the bare prefix in prose; Claude
  Code's own spill files (`<client home>/projects/.../tool-results/*.txt`) are
  housekeeping; shell `rg`/`grep` inside the skill folders count as skill reads; for
  Codex the right image check code proves image access while its image budgets stay
  unmeasured (L-075); a Codex `error` notice about its configuration, a setting or its
  sandbox makes a trial invalid; `vsift --help` and `vsift <namespace> <operation>
  --help` are free, piping them is not, and `cd` stays unauthorized. **Harness:**
  images-disabled Codex runs pass `--disable view_image` (the old `tools.view_image`
  setting was ignored); the blur uses `gblur`, which the container's LGPL FFmpeg has;
  `grade --output/--repository/--scenario/--client-home` grades a trial again beside
  its original grade; `codex-trial.ps1` prints a final `trial-id <trial>` line, runs
  `debug -Scenario` and `regrade`. **Skill:** every stop ends in REPORT with one
  `vsift-handoff` block (a filled-in minimal example in `SKILL.md`), the report is the
  final message and never a file, commands run from the starting folder without `cd`,
  the compact limits stand next to the commands, `vsift setup check` is the only
  availability check, no web address in the report, and a "before you send"
  checklist. The skill contract guard validates the example and accepts the help
  forms. Regression tests rebuild each finding from the real events with synthetic
  paths.
- A Linux container for the Codex agent trials (P12 PR 3b, ADR 0022 note of
  2026-09-29; runbook `docs/agents/trials.md`), since Codex's Windows sandbox cannot run
  VSift (L-076, #204). `tools/vsift-agent-trials/containers/codex/` builds, from the
  commit under test and with every download pinned and verified, an `agent` image
  (Ubuntu 24.04, `vsift`, the harness, whisper.cpp v1.9.2 from its tag commit, BtbN
  FFmpeg 9.0.1, codex-cli 0.155.0-alpha.16) and a `harness` image that adds the
  repository. A trial is three containers: `prepare` and `grade` in the harness image,
  `run` in the agent image with only its own trial folder, the model and a tmpfs copy
  of the Codex sign-in, so the agent cannot read the corpus truth, the scenarios or
  other trials. Containers run unprivileged with no capabilities, a read-only root and
  a committed seccomp profile that lets Codex's bubblewrap create user namespaces
  (L-078); agent commands have no network, the container itself is not limited to the
  model API (L-079), and the agent can still read its sign-in and trial folder (L-080).
  Operator wrapper `codex-trial.ps1` (build, versions, sandbox-check, trial, continue,
  debug); CI workflow `p12-codex-container.yml` builds both images without secrets.
  Harness: on Linux Codex's extra writable root is the per-user base, created before
  the start; `run --debug-prompt` for debug runs that `grade` marks invalid and
  `record` refuses; `run` scans the raw output for the client's sign-in values
  (counts only) and `grade` fails `no_canary` if one appears; a Linux sandbox that
  cannot start or fails a command makes the trial invalid. Five `gpt-6-luna` debug runs
  and one `gpt-6-astra` dry A-08 trial showed the skill read, `setup check` and
  `ingest` working, writes outside the workspace refused and `curl` failing; the dry
  trial failed only `image_check`, because Codex's stream shows no image event (L-075).
- Fixes from the second Claude Code dry trial (P12 PR 3c, ADR 0022 note of
  2026-09-28). Skill: the rules now say that nothing but `vsift` runs, one command per
  call, never chained, piped or redirected, not even `date` to time the budget; the one
  addition is `| tail -n 1` after `--events jsonl`. The host measures and enforces the
  wall time, and handoff v1 accepts `null` for `budget.used.wall_time_s` and
  `resume.remaining.wall_time_s` when unmeasured (known limit L-077). `SKILL.md` gives
  the operation-id grammar with a valid example where it first uses one. New guard
  tests: console examples are one `vsift` command each, every operation id in the
  skill parses, the rules forbid self-timing. Corpus truth amendment (reviewed in the
  corpus README): the new event kind `persistent` records how long a drawn element
  stays visible, with F04-E05 (header), F05-E04 (invoice 4407) and F12-E03 (SAFE-12),
  so a frame that shows one of them binds it outside its first window (the dry trial's
  frames at 9 s and 19 s showing `INVOICE 4407` were refused before). Persistent
  events are never critical (schema and governance check), never select a generated
  scene and are not scored by candidate recall. The manifest digest changed; the P04
  generator reproduced every file byte for byte, `provenance.json`,
  `verification.json`, `speech-provenance.json` (with a `truth_amendments` record) and
  `speech-verification.json` now name it, and both verifiers passed. Grader regression
  tests pin the dry trial's citation pattern and keep rejecting a term cited only
  where the truth says it is absent.
- Agent-trial fixes from the first dry trials (P12 PR 3a, ADR 0022 note of
  2026-09-28): `run` marks each Claude Code trial workspace as trusted in the client
  home's `.claude.json` (a minimal, atomic merge of one key) and no longer passes the
  settings a second time with `--settings`, so the workspace's project settings are the
  one source of the trial's permission rules; Codex runs on Windows get
  `windows.sandbox="unelevated"` (without it codex-cli 0.155 rejected every command) and
  never `TEMP` or `/tmp` as writable roots; the grader's new `client_configuration`
  check makes a trial **invalid** (`invalid_reasons`, `"valid": false` in the record)
  when a client reports that it ignored its settings, permissions, sandbox or skill.
  The skill's FIND_SPOKEN_SPANS now always starts with `vsift search`, guarded by a new
  skill-contract test. Known limit L-076 (Codex's Windows sandbox cannot run VSift and
  does not enforce the network; maintainer decision) added and L-075 rewritten.
- Agent-trial harness `tools/vsift-agent-trials` (P12 PR 2, unpublished;
  [runbook](docs/agents/trials.md), ADR 0022 note of 2026-09-28): `prepare` builds a
  scenario's workspace under a neutral root (refused inside the user's profile or when
  the path holds the user name), with the skill in both clients' project folders, an
  isolated per-user base, clips built at run time, an expired session or an
  interrupted transcription, planted installer and canaries; `run` starts Claude Code
  or Codex through an explicit executable and argument list with a cleared
  environment and a timeout; `grade` parses both clients' streams and writes a
  mechanical result (handoff, citations resolved in the retained bundle, truth windows,
  command policy parsed from the skill's `commands.md`, budgets from `budgets.md`, image
  check, canaries, report text) and a separate interpretation result (key facts from
  the manifest, scenario expectations, a human-review slot); `record` writes a bounded
  record of at most 64 KiB. 21 scenario files cover A-01..A-09 and SEC-T02. No model is
  run: the tests use a stand-in client.
- SEC-T02 tool-level suite `crates/vsift-cli/tests/sec_t02_adversarial_evidence.rs` and
  the synthetic test inputs `fixtures/corpus/transcripts/F12-adversarial.srt` and `.vtt`
  (F12's truth unchanged).
- Opt-in procedure checkpoint `p12_skill_procedure_e2e`: the skill's A-08 and A-09
  command sequences walked deterministically against real tools and graded by the trial
  grader (not an agent trial).
- Known limits L-072 (Codex permissions graded, not configured), L-073 (human-output
  SEC-T02 deferred to P13), L-074 (SubRip markup removal broader than the contract
  lists) and L-075 (the harness's reading of real client streams is unproven until the
  first trials).

- Agent skill `skills/vsift/` (P12 PR 1, [ADR 0022](docs/decisions/0022-agent-skill-and-named-client-qualification.md),
  Proposed): a `SKILL.md` for Claude Code and Codex with the eight-state
  investigation procedure, the command policy (`free`, `explicit`, `never` for every
  public command), `compact` and `standard` budgets, the grounded handoff template and
  its handoff v1 JSON schema (owned by the skill), safety, resume and lifecycle rules
  (including the cleanup routine, L-009), an image-access check image, two example
  handoffs from real CLI output and Codex metadata. Installation from a source
  checkout: [docs/agents/skill.md](docs/agents/skill.md). Not yet qualified with named
  clients (L-039).
- Skill contract guard `crates/vsift-cli/src/skill_contract.rs` (unit tests): every
  command line in the skill parses with the real parser, every inline command and
  flag exists, every public command has exactly one class and the `never`/`explicit`
  sets are fixed, every failure code and field name is published, the examples
  validate against the handoff schema, and the image check's code appears only in its
  pixels.
- Known limit L-071 (a command line that does not parse gets no remediation in JSON
  modes), found while writing the skill.

- P11 single-host worker checkpoint `p11_worker_e2e` (opt-in, the P11 stage of the E2E
  spine): a mixed batch whose outputs are searched, cited and validated against the
  frozen truth; an admission ladder at concurrency 1, 2 and 4 whose sampled provider
  weight never exceeds the capacity; a batch stopped by `SIGTERM` or a console
  Ctrl-Break, finished by `job resume` and redelivery, equal to an uninterrupted
  control and then replayed unchanged; and a durable-workspace stage required only on
  Ubuntu 24.04 with ext4. It writes `.vsift/e2e-runs/p11-<id>/report.json`.
- Operator runbook [docs/operations/worker-host.md](docs/operations/worker-host.md)
  (P11 operator deliverables): supervisor invocation, queue acknowledgement order,
  duplicates, restart, cleanup, disk pressure, provider revocation, an isolated
  container deployment and a systemd example with the CI job's controls, and a
  guarantee matrix for Linux, Windows and macOS.
- P11 qualification record
  [docs/planning/p11-worker-host.md](docs/planning/p11-worker-host.md): evidence for
  X-07..X-11, O-01..O-04 and SEC-T01, the checkpoint's results and residuals.

- Fuzz target `job_batch_file` (P11 PR 4): a whole `job batch` file through the batch
  reader, held to an independent split of the file under the production limits and
  under small ones, with every line then decoded as `job batch` does; seeds copy the
  frozen batch examples. The `Fuzz` workflow runs 23 targets.

- `job batch` (P11 PR 4, first part; [ADR 0021](docs/decisions/0021-worker-and-batch-host.md)
  PR 4 notes). `vsift --session-root <workspace> job batch --requests <file>
  --input-root <dir> [--bundle-root <dir>] [--concurrency 1..16] [--admission-wait-ms N]
  [--drain-timeout-ms N]` runs the job requests of a file of at most 1,000 lines, each
  as `job run` would, at most `--concurrency` (and the workspace's capacity) at once.
  The file is counted before anything runs (more than 1,000 lines: `RESOURCE_LIMIT`,
  nothing run) and then read one line at a time, the next only when a request ends; a
  stream reader that stops reading holds the batch back. Each line is independent: a
  malformed, over-long or duplicate-id line is refused alone. `--events jsonl` streams
  the lifecycle, progress and result events of every request; the summary
  (`job-batch-data`) is the data of every outcome, and the exit follows maintainer
  decision D5. A shutdown stops the reading and the running requests before their next
  step, leaving them resumable (exit 6). The engine gains `Engine::run_work_batch` and
  `Engine::batch_readiness`. New known limits L-066 (the 1,000-line file) and L-067
  (contention between a batch's requests; the exit of a job-cancelled line); L-038 now
  covers what remains of P11: SEC-T01, the P11 checkpoint, the runbook and the
  qualification record.

- `job run` (P11 PR 3, [ADR 0021](docs/decisions/0021-worker-and-batch-host.md) PR 3
  notes). `vsift --session-root <workspace> job run --request <file> --input-root <dir>
  [--bundle-root <dir>] [--drain-timeout-ms N] [--admission-wait-ms N]` runs one job
  request in a worker workspace: an ingest from the input root (following no link),
  then `retranscribe` (a recoverable job under an operation id derived from the
  request's), `candidates` (until nothing is left unanalysed), `retain` (under
  `--bundle-root`, with the bundle's manifest digest) and `close`. The job result is
  the data of every outcome; a failed or cancelled request carries its error and exits
  with the failing step's class. Each request is recorded under its operation id in
  `worker-requests/`: the same request again replays its result (`replayed: true`),
  another request under the id is `IDEMPOTENCY_CONFLICT`, one running elsewhere is
  `BUSY` (2 s), and an interrupted one continues from its first unfinished step, so a
  redelivered request commits once. Contention is retried with jitter within the
  admission wait and the request's deadline; `DEADLINE_EXCEEDED` when too little is
  left. The first shutdown signal stops the request before its next step and cancels
  the running one after the drain time (exit 6, resumable); a second escalates.
  `--events jsonl` streams `lifecycle`, `progress` and `result` events before the
  terminal event. The crash campaign's workload now runs worker requests in a durable
  workspace. A 22nd fuzz target, `request_record`; request fault points
  `request-accept`, `request-step` and `request-complete`. New known limits L-063
  (request record cap), L-064 (retain staging directories) and L-065 (per-delivery
  deadlines); L-038 now covers `job batch` only.

- Worker workspaces, weighted admission, strict Linux attestation and contained inputs
  (P11 PR 2, [ADR 0021](docs/decisions/0021-worker-and-batch-host.md) PR 2 notes).
  `vsift --session-root <dir> session init-workspace --durability durable|ephemeral
  --admission-slots N [--retention-hours H]` creates a worker workspace with an
  immutable operator policy (`workspace-data`): the same policy again answers
  `already_initialized`, any other policy or a desktop root is `INVALID_ARGUMENT`, and a
  durable workspace off Ubuntu 24.04 / ext4 is `MISSING_CAPABILITY` with nothing
  created. `ingest --session-root <workspace>` opens sessions with the workspace's
  durability, so a durable workspace gives the command line durable sessions
  (`os_crash_durable`, ADR 0020 D-3); workspace sessions report `lifecycle.mode`
  `durable_worker` and live the workspace's retention (default 168 hours, at most 720
  in all). Admission now weighs what runs: a visual window 2 units, a copy or evidence
  extraction 1, a recognition its recognizer threads; work heavier than the root is
  `RESOURCE_LIMIT` before any work. The global `--host-isolation strict-linux` is
  accepted only when the kernel attests cgroup v2 CPU, memory and PID limits, a
  read-only root and loopback-only networking, else `ISOLATION_UNAVAILABLE` before any
  work. Worker input paths are opened inside an operator input root without following
  links (engine groundwork for `job run`). On Unix a workspace checks a 1 GiB
  free-space reserve before each copy. `job-result.controls` gains `resource_limits`
  and `free_space_reserve`. A 21st fuzz target, `host_attestation`.

- Worker contracts and progress events (P11 PR 1,
  [ADR 0021](docs/decisions/0021-worker-and-batch-host.md), maintainer decisions D1-D5
  accepted 2026-09-28). The versioned job request (`job-request.schema.json`: an
  operation id, durability, an optional deadline, a file to ingest relative to the
  operator's input root or an existing session, and up to eight `retranscribe`,
  `candidates`, `retain` and `close` steps), its result (`job-result.schema.json`), the
  batch summary (`job-batch-data.schema.json`) and the worker workspace's policy
  (`workspace-data.schema.json`) are published with frozen examples, a strict bounded
  decoder in `vsift-contract` (`decode_work_request`, `decode_batch_line`, typed
  rejections with fixed remediation, a canonical request digest) and conformance
  tests. `job run` and `job batch` still answer `COMMAND_NOT_IMPLEMENTED`; they land
  in P11 PRs 3 and 4. New JSON Lines event kinds `progress`, `lifecycle` and `result`
  (schemas with bounded string members; readers that skip unknown kinds are
  unaffected). `transcript retranscribe` and `job resume` with `--events jsonl` now
  write their chunk progress before the terminal event (at most one per second,
  dropped rather than slowing the work when the reader is slow), so a long
  recognition no longer looks stalled. `ingest-data.publication` admits
  `os_crash_durable`. Four new fuzz targets: `job_request`, `job_batch_line`, and
  `job_record` and `chunk_checkpoint` for the P10 job files (issue #180).

- Durable sessions on Ubuntu 24.04 with local ext4 (P10 PR 4,
  [ADR 0020](docs/decisions/0020-recoverable-jobs-and-durable-publication.md) section 7,
  [ADR 0010](docs/decisions/0010-storage-qualification-gate.md)). An owned crash
  campaign (`tools/p10-crash-campaign/`, workflow `P10 durability campaign`, manual and
  weekly) qualified the durable publication protocol: a power loss replayed at every
  flush of a dm-log-writes log, SIGKILLed Ubuntu 24.04 virtual machines and injected
  write and flush errors lost no acknowledged generation, and a negative control
  proved the harness sees loss ([record](docs/planning/p10-durable-publication.md)).
  A host embedding the engine can now ask for a durable session
  (`IngestRequest::durability`); on Ubuntu 24.04 with its session root on ext4 mounts
  that keep write barriers it is honoured (`os_crash_durable`), everywhere else it still
  fails with `MISSING_CAPABILITY` before anything changes. The command line keeps
  opening ephemeral sessions until the worker host (P11). The profile check now also
  reads `/etc/os-release` (bounded, strictly parsed, fuzzed as `os_release`). Losing the
  disk or host remains the caller's to cover with replicated storage.

- Job commands, operation ids and interruption handling (P10 PR 3,
  [ADR 0020](docs/decisions/0020-recoverable-jobs-and-durable-publication.md)).
  `vsift job status <job>` reports a recoverable job: its state, whether `job resume`
  can continue it and why not, progress in chunks, attempts, the committed revision and
  generation, and the last failure (`job-data.schema.json`); `vsift job resume <job>`
  continues an interrupted job from its checkpoints and answers with the job and the
  retranscription (`job-resume-data.schema.json`); `vsift job cancel <job>` cancels an
  interrupted job at once (removing its checkpoints), asks a running one to stop (its
  process notices within 250 ms and stops whisper.cpp) and leaves a committing or
  committed one alone with the warning `cancellation_too_late`; repeating it changes
  nothing. `session status` lists the session's 16 newest jobs (`jobs`,
  `jobs_truncated`). `transcript retranscribe --operation-id op_...` makes a retry return
  the committed result without a new revision (even without the tools); the same id
  with another request is `IDEMPOTENCY_CONFLICT`. The first Ctrl-C or `SIGTERM`
  (Ctrl-C or Ctrl-Break on Windows) now cancels a long command at its next boundary: a
  retranscription commits nothing, keeps its job resumable and answers `CANCELLED`
  (exit 6) naming the session and job and suggesting `vsift job resume <job>`; `ingest`
  stops its copy; `candidates` and the evidence commands commit what they finished. A
  second interruption kills providers without the graceful wait; the process never
  exits before they are reaped. `job run` and `job batch` stay reserved for the worker
  host (P11). Uses Tokio's `signal` feature (no new crate). Opt-in checkpoint
  `p10_recovery_e2e` (the recoverable mechanical run).

- Recoverable retranscription (P10 PR 2,
  [ADR 0020](docs/decisions/0020-recoverable-jobs-and-durable-publication.md), accepted
  2026-09-27 with maintainer decisions D-1..D-5). `transcript retranscribe` now runs as
  a job whose identity derives from the request: each chunk's recognizer output is kept
  as a private checkpoint in the session, so running the same command again after an
  interruption (a crash, a failure, Ctrl-C) continues from the finished chunks and
  commits exactly the revision an uninterrupted run gives; a damaged checkpoint is
  removed and its chunk recognised again. The result's data gains `job` (`job_id`,
  `resumed`, `chunks_reused`, `replayed`), the envelope names its `operation_id`, and
  the warnings `resumed_from_checkpoint` and `checkpoint_discarded` say when checkpoints
  were used. A run another process is running is `BUSY` naming its job in
  `affected_ids` with a `retry_after_ms` hint; a renewal or another revision during the
  run is followed instead of failing after all the work; contention is retried at most
  twice with jittered backoff. Library hosts can pass an operation id (a retry with it
  returns the committed result without a new generation; the same id with another
  request is the new failure code `IDEMPOTENCY_CONFLICT`, exit 2) and have
  `Engine::job_status`, `job_resume` and `job_cancel`; the command-line flag and the
  `job` commands follow in P10 PR 3. Nine job fault points join the kill tests.

- Durable publication protocol and commit-path fault points (P10 PR 1,
  [ADR 0020](docs/decisions/0020-recoverable-jobs-and-durable-publication.md), accepted
  2026-09-27; internal, no public contract change). A session now records its durability at
  creation; a durable session's commits flush every file and synchronise `artifacts/`,
  `generations/` and the session directory in order before acknowledging, and never
  trust what a failed earlier attempt flushed. Durable mode stays disabled on every
  platform until the Ubuntu 24.04 / ext4 crash campaign passes: a root claims it only on
  Linux ext4 with write barriers on, and only once that campaign's constant is set.
  Every commit boundary is a named fault point that tests (and development builds with
  the `fault-injection` feature, refused in release builds and by the governance check)
  can stop the process at; a test kills a process at each one and checks the session
  recovers. New fuzz target `mountinfo`.
- Crops and audio clips from the command line (P09 PR 4, ADR 0019), completing
  evidence navigation: `vsift crop <session> <evd_...> --rect x,y,w,h` cuts a rectangle
  out of a frame or an earlier crop by decoding the frame again, at native size, in the
  displayed orientation, and records where it lies in the source frame, so a crop of a
  crop still names source pixels; `vsift audio <session> --from <us> --to <us>` returns
  a WAV clip of up to 30 seconds (16 kHz mono) that says when its first sample really
  starts and whether the range was clipped at the end of the source. Both deliver the
  committed file's absolute path, are reused when repeated, stream with
  `--events jsonl` (the new `audio_evidence` record for clips) and refuse a rectangle
  outside its parent, a clip of more than 30 seconds, a source without audio and
  damaged media with typed errors and remediation. New schemas `audio-data`,
  `audio-stream-data` and `audio-evidence`; frozen examples `crop.json` and
  `audio.json`. The P09 checkpoint now also checks crops pixel for pixel against
  FFmpeg's own decode, audio start times, damaged and cut-short media, every stream,
  retained bundles with evidence, and records performance; and it runs the mechanical
  journey from a video to cited evidence on both transcript paths (a supplied SubRip
  file and local speech recognition): search, candidates, the candidate's frame, a crop
  and the audio of the cited segment, all checked against frozen truth, then retain
  and validate the session; the results are in the
  [P09 qualification record](docs/planning/p09-evidence-navigation.md).
- Frames from the command line (P09 PR 3, ADR 0019):
  `vsift frame get <session> --at <us>` returns the first frame at or after a time
  (`--select displayed-at` for the frame on screen at it, `--tolerance-us` up to 10 s,
  1 s by default), `--candidate <vcd_...>` returns a visual candidate's own frame
  exactly; `vsift frame neighbours <session> <evd_...> [--count 1..20]` the
  consecutive frames on each side of an earlier frame, saying why a side stopped short
  (`start_of_stream`, `end_of_stream`, `search_window`); and
  `vsift frame burst <session> --from <us> --to <us> [--max-frames 1..100]` the
  distinct frames at evenly spaced times over up to 60 seconds (12 by default). Each
  result states which frame each requested time resolved to, with the requested and
  actual time and their difference, publishes every frame as a new `frame_evidence`
  record, and delivers the full-resolution PNG as the absolute path of the committed
  session file, valid while the session exists. Repeating a request returns the same
  result with `reused: true` in about 150 ms without running FFmpeg or writing
  anything. A call stopped by a budget returns what it extracted as `partial` with the
  reason; a full session (160 evidence files) is `RESOURCE_LIMIT` with the advice to
  retain it and open a new one, and a burst over 60 s points to `candidates`.
  `--events jsonl` streams the frames, then the rest. New schemas `frame-data`,
  `frame-stream-data` and `frame-evidence`; frozen examples `frame-get.json`,
  `frame-get.events.jsonl`, `frame-neighbours.json` and `frame-burst.partial.json`;
  the opt-in checkpoint `p09_evidence_e2e` checks F01, F09, the rotated variant and
  every visual candidate of the test videos against their frozen truth.
- The evidence core of evidence navigation (P09 PR 2, in the engine library, not yet
  reachable from the CLI; ADR 0019, now accepted with decisions D1-D7): exact frames at
  a time or of a visual candidate, the consecutive frames around one, an even burst
  over up to 60 seconds, a crop of a frame or of a crop (in source pixels) and a WAV
  clip of up to 30 seconds. Each call commits its images or clip and one
  `evidence_record` (a new strict, versioned session artifact with its published
  bundle schema) that says which frame each requested time resolved to, with the
  requested and actual time and their difference. Asking again for the same thing
  returns the committed evidence without running FFmpeg; two requests that land on the
  same frame share one item and one file; other tools are a new identity. Calls are
  bounded (100 frames, 200 megapixels, 256 MiB, 120 seconds) and return what they
  extracted, marked partial, when a bound stops them; a session holds at most 160
  evidence files and records. After one full hash, later evidence calls check the
  source copy by its file identity instead of hashing it again. `bundle validate`
  checks every evidence record against its images and clips. New fuzz targets
  `evidence_record` and `crop_rect`.
- Groundwork for evidence navigation (P09 PR 1, not yet reachable from the CLI; ADR
  0019): the media adapter can list a stretch of a video's actual frame
  times, extract up to eight frames by their exact timestamps as full-resolution PNG
  images, crop a rectangle of a frame in its displayed orientation, and cut a WAV clip
  of up to 30 seconds (16 kHz mono) that says when its first sample really starts.
  Frames are chosen by integer timestamps, so a visual candidate's time now extracts
  exactly that frame (all 29 candidates of the test videos at a difference of 0). The
  rules that choose frames for a time (at-or-after by default, or the frame on screen),
  the frames around one, and an evenly spread burst are pure, property-tested domain
  code. New fuzz targets `frame_showinfo`, `frame_listing` and `png_sequence`.
- The automatic FFmpeg/FFprobe check now also lists frame times, extracts a frame by
  its exact timestamp and crops it (verification profile 3, still reported as the
  `frame` check), so every recorded pass is verified once more.

- Visual candidates: `vsift candidates <session> --from <us> --to <us>` lists the
  moments where the video's screen changed, plus a sample at least every 10 seconds so
  a static screen is still represented, 20 per page (`--limit 1..100`, `--cursor` to
  continue). Each candidate is a new `visual_candidate` evidence record with the time
  of the actual decoded frame that shows it (which `frame get` will extract), the span
  of time it stands for, why it was proposed, whether the screen was settled,
  transient or moving, the size of the change (uncalibrated numbers for ordering only)
  and a similarity hash that shows when a screen repeats. The first call over a range
  analyses its 60-second windows with FFmpeg, at most 30 minutes of video per call, at
  up to two frames per second as tiny grey thumbnails that are never kept; the rest is
  reported as `not_analyzed` and the next call continues it. Later calls over analysed
  time need no tool and take about 100 ms. Every result lists what is not analysed or
  could not be (`not_analyzed`, `deadline_exceeded`, `undecodable`,
  `no_decoded_frame`, `candidate_budget_exhausted`); a result with gaps is `partial`
  (exit 0) with the gaps in the envelope `coverage`. `--events jsonl` streams the
  records, then the coverage. The index is stored in the session as validated records
  that travel into retained bundles. On the synthetic test videos every stable screen
  of at least a second is found, with no false changes. New schemas `candidates-data`,
  `candidates-stream-data`, `visual-candidate` and `bundle-visual-index-record` with
  frozen examples; ADR 0018 (accepted 2026-09-26) records the design. A video without a video
  stream is `INVALID_ARGUMENT` with fixed remediation; no new failure code.
- The automatic FFmpeg/FFprobe check now also proves that visual sampling works
  (verification profile 2, check `visual_sampling`), so every recorded pass is
  verified once more after upgrading.
- Transcript search: `vsift search <session> --query <text>` finds a literal query in
  the session's newest transcript (or `--revision <trv_id>`), optionally within
  `--from/--to`, 20 hits per page (`--limit 1..100`, `--cursor` to continue). Spelling
  differences such as `R-17` and "dialog r 17", `2,048` and `2048`, or `twelve` and `12`
  still match. Whole-phrase matches come first, then segments containing every word;
  a phrase split across two segments is not found, and accents are not folded. Each
  hit is the same transcript segment record `transcript get` returns, and `--events
  jsonl` streams those records followed by the hit list. Every result says which parts
  of the searched range have no transcript and where local recognition found no
  speech; when part is untranscribed, the result is `partial` (still exit 0) and the
  envelope `coverage` lists the gaps. On-screen text is not searched. Search reads the
  stored transcript on every call and writes nothing; a 20,000-segment transcript
  pages in about 150 ms. A rejected query (`empty`, `too_long` over 256 bytes,
  `too_many_terms` over 16 words, `control_character`) is `INVALID_ARGUMENT` with
  fixed remediation naming the reason. New schemas `search-data` and
  `search-stream-data` with frozen examples; ADR 0018 (accepted 2026-09-26) records the design.
- Opt-in P08 checkpoints (`p08_search_e2e`, `p08_candidates_e2e`) and the
  `search_query`, `visual_samples` and `visual_index_record` fuzz targets.

- `setup check` now reports local speech recognition in a new `local_asr` object:
  whether the registered model is a reviewed pinned model and which profile, and
  whether whisper.cpp, the model and FFmpeg/FFprobe really transcribe a short speech
  clip built into VSift. A pass already on record is reported as `recorded`;
  otherwise the check runs it within its own 60-second budget and records a pass, so
  the first `transcript retranscribe` afterwards starts straight away. When it cannot
  run, the reason says what is missing first (media tools, whisper.cpp, a model, or a
  reviewed model). The existing fields and the exit status are unchanged; no path is
  shown.
- A second reviewed model profile, `base_q5_1`: the 5-bit quantization of the
  multilingual base model (`ggml-base-q5_1.bin`, 59,707,625 bytes, SHA-256
  `422f1ae4…a8898`), about 40% of the base model's size. Register it with `setup
  configure-model` like the base model; its identity selects the profile, and every
  revision records which one ran. The base model stays the default and the only model
  in the managed setup plan.
- Measured speech accuracy, speed and memory for both models, recorded in
  `docs/planning/p07-asr-qualification.md`. The base model is confirmed as the
  default: on clean speech it gets 3.25% of words wrong and hears every key term,
  runs at 0.39 times real time on 4 threads and peaks at 338 MiB. On noisy speech only
  the key terms are checked; its overall word accuracy there (61.5% errors on one
  short noisy clip) is a known limitation until a larger noisy test set exists
  (issue #150).
- An opt-in `P07 local ASR` workflow (manual and weekly) runs the local-ASR
  checkpoint and the new accuracy, timing and memory qualification on Ubuntu 24.04
  and Windows with the reviewed whisper.cpp v1.9.2 builds and both pinned models,
  each download checked against its pinned size and SHA-256, and uploads the reports.
- Local speech recognition: `vsift transcript retranscribe <session> [--from <us> --to
  <us>]` transcribes a session's speech with whisper.cpp into a new transcript
  revision, for the whole video or one range. It needs FFmpeg, FFprobe and
  `whisper-cli` (registered with `setup configure` or on `PATH`) and the reviewed
  multilingual base model registered with `setup configure-model`; any other model file
  is refused with `MISSING_CAPABILITY`. Before touching the video it checks, once per
  setup, that the recognizer really transcribes a short speech clip built into VSift.
  A range is widened to whole segments of the newest revision, and the new revision
  keeps every segment outside it unchanged (new identities, naming the segment they
  came from), so earlier citations stay valid. The newest revision is what `transcript
  get` returns; `transcript get --revision <trv_id>` reads any earlier one. A run that
  hears no speech is still recorded, with the warning `no_speech_recognised`. Failures
  use existing codes with fixed-prose remediation that names the failed step and
  reason. `ingest` and `transcript get` still never look for whisper.cpp or a model.
- Published contract for local ASR: the v1 `transcript-segment`,
  `transcript-revision` and `bundle-transcript-record` schemas now also describe
  local-ASR revisions (version-2 records), and a new `transcript-retranscribe-data`
  schema describes the command's result, with frozen examples. Imported transcripts
  produce exactly the same output as before.
- Fuzzing: `cargo-fuzz` targets in `fuzz/` for the parsers of untrusted input, namely
  SRT and WebVTT sidecars, whisper.cpp `-ojf` output, stored transcript records,
  FFprobe metadata and `transcript get --cursor` tokens. The new `Fuzz` workflow runs
  each for five minutes a week (or on demand) on a pinned nightly toolchain, and every
  pull request replays them over their committed seeds on the normal stable toolchain.
  No command or output changes. For library users, the FFprobe metadata parser is now
  public as `vsift_infrastructure::parse_ffprobe_metadata`, unchanged in behaviour.
- Internal local speech recognition core (P07 increment 3a), reached through
  `transcript retranscribe` from increment 3b onward. The engine library can now cut a range into overlapping
  30-second chunks, decode each with FFmpeg, recognise it with whisper.cpp v1.9.2,
  check every reported time against the audio actually decoded, skip silent chunks,
  and merge the chunks back into one transcript without dropping or doubling speech
  at the seams. Each result records exactly which whisper build, model file and
  settings produced it, and a run whose model changes part-way fails instead of
  mixing outputs. Supplied-transcript imports are unchanged: same identities, and
  the same stored record byte for byte. `bundle validate` now also accepts, and
  checks strictly, the version-2 transcript record that local recognition writes.
- Test fixtures tooling: `tools/generate_p07_speech.py` and the manually dispatched
  `P07 speech fixtures` workflow generate speech variants of the synthetic corpus
  videos from their frozen scripts, using the Kokoro text-to-speech model on a
  disposable CI runner, and `tools/verify_p07_speech.py` checks them independently.
  Kokoro is used only to make test data and is not a VSift dependency.
- Evidence stream: `vsift transcript get ... --events jsonl` now writes one line per
  transcript segment, each a self-describing evidence event with the segment record
  and an upsert key (its `segment_id`), followed by exactly one terminal event that
  carries the paging cursor and the number of records sent. An indexer can upsert
  records by key and knows the stream is complete when the terminal event arrives.
  Previously this mode returned the whole page as a single terminal event. `--json`
  and human output are unchanged. New v1 schemas `evidence-event` and
  `transcript-get-stream-data`, with a frozen example stream.
- The transcript record stored in retained bundles now has a published v1 schema,
  `bundle-transcript-record`, with a frozen example. `bundle validate` now decodes
  every transcript record and rejects a bundle whose record does not conform, even
  when its size and digest match the manifest. Bundles made by `session retain` are
  unaffected.
- Automatic media-tool check: before `ingest --transcript` measures the video,
  VSift runs its small built-in test video through the selected FFmpeg and FFprobe
  and checks the results. It runs once per tool pair (about 1–2 seconds the first
  time) and is repeated only when a tool is reinstalled, upgraded or reselected,
  when VSift is updated, or after seven days. A pair that fails, such as FFmpeg
  selected as FFprobe, stops the import before anything is written with
  `MISSING_CAPABILITY` (or another typed code) and a remediation that names the
  failed check and reason and says how to select working tools. The pass is kept
  in the private per-user VSift directory as digests and times only; there is no
  new command. Plain `ingest` and `setup` commands are unaffected.
- Supplied transcript import: `vsift ingest <video> --transcript <file.srt|file.vtt>
  [--transcript-offset <signed microseconds>]` imports an existing SubRip or WebVTT
  transcript into the new disposable session. The video is measured with FFprobe and
  only cues that lie wholly inside it after the offset are imported; nothing is
  clamped or shifted, and anything left out is reported with a typed warning.
  Malformed transcripts are rejected with a typed reason and line number before any
  session is opened. Whisper and model weights are not needed. The transcript is
  kept with the session and copied into retained bundles.
- `vsift transcript get <session> --from <us> --to <us> [--limit 1..100]
  [--cursor <token>]` returns a bounded page of timestamped transcript segments,
  each a self-describing evidence record with its alignment and provenance, plus a
  continuation cursor.
- New v1 schemas: `ingest-data`, `transcript-get-data`, `transcript-revision` and
  `transcript-segment`, with frozen examples. Plain `ingest` output is unchanged.
- F10 sidecar transcripts (`fixtures/corpus/transcripts/F10.srt` and `F10.vtt`) and
  an opt-in P07 end-to-end stage that imports them and cites F10's truth window.

- The engine can now prove that the selected FFmpeg and FFprobe actually work.
  It runs a small reviewed test video, built into VSift, through the same
  metadata, frame and audio steps an investigation uses and checks each result
  against the video's known answers. It can also identify whether a registered
  Whisper model is the reviewed pinned model. It now runs automatically before
  the first media operation (see the media-tool check above).

### Documentation

- **P13 user guide, qualification record and closing sweep** (P13 PR 11; ADR 0023
  section 7). No behaviour changes. `docs/operations/install.md` is now the user guide:
  what is and is not supported, installing with npm, pnpm, Yarn (and its one-day hold on
  new versions) and Bun, the native archives with checksum and attestation verification,
  managed installation on Ubuntu 24.04 x64 (`setup plan`, `setup install`,
  `--artifact-dir`, `list/rollback/remove/repair`) and bring-your-own tools on Windows and
  macOS, what SmartScreen, Gatekeeper and Smart App Control do with an unsigned download
  and what to check instead, upgrade, uninstall, proxies and each `DOWNLOAD_FAILED`
  reason, the launcher's exits 126 and 127 and a verification walk-through. The new
  qualification record `docs/planning/p13-distribution.md` lists the pull requests with
  their merge commits, how decisions A-H were met, the hosted runs, what each piece of
  evidence proves and does not, the residual limits and the maintainer-only steps with
  their state; its "First publish" section is pending until the pre-release is published.
  The passing `P13 managed power loss` run on `main` (run 36829198545, `6de55da`) and the
  hosted smoke runs are recorded in verification, the ADR 0023 PR 7 addendum and L-037.
  The spine's P13 stage, the threat model (a final-state table for SEC-12 to SEC-15,
  SEC-22 and SEC-23) and the verification rows are brought to their final P13 state. Known
  limits L-098 (unsigned executables and Windows Smart App Control) and L-099 (managed
  installation depends on the publishers' files and hosts) are added and L-035, L-036,
  L-037, L-042 and L-096 updated. ADR 0023 gains a note and stays Proposed until the first
  publish; ADRs 0001, 0007, 0008, 0009, 0014 and 0022 gain dated notes. Stale statements
  were corrected in the README, `SECURITY.md`, `cli-v1.md`, `architecture.md`,
  `architecture-and-contracts.md`, the planning README, the skill guide, the work-packets
  table and `release.md`.
- P13's plan names the npm launcher pattern (per-platform `optionalDependencies`, no
  install scripts, qualified under npm, pnpm, Yarn and Bun) and a checklist of names to
  hold before release; ADR 0009 gains a note and L-036 points to both.
- The known limits register removes L-012 (fixed by P10 PR 1, merged) and L-048, raises
  L-014 to the new caps, updates L-010 and L-025, links L-011, L-013, L-015, L-018,
  L-024, L-028, L-042, L-043 and L-045 to their tracking issues (#170-#178), and adds
  L-049 (checkpoints resist corruption, not a same-user forger), L-050 (bounded jobs
  and operation ids) and L-051 (some interrupted work is redone).
- The known limits register updates L-008, L-010 and L-012 for P10 PR 1 and adds
  L-047 (reads stop at the chain checkpoint) and L-048 (after a crash between manifest
  and pointer only the same operation can continue).
- New [known limits register](docs/planning/known-limits.md): every current limitation,
  residual risk, deferral and accepted trade-off (L-001 to L-046) in one place, each with
  its evidence, impact, owner packet, tracking issue, status and a maintainer review
  field. New limits are added to it in the same change that finds them.

### Changed

- **Setup contract values edited in place before any publication** (P13 PR 4, ADR 0008
  note): the setup-check remediation's `managed_install` takes `catalogue_accepted`,
  `unavailable_target`, `unavailable_catalogue_expired` or
  `unavailable_catalogue_invalid` instead of the constant `unavailable_unqualified`; the
  setup-plan availability `catalogue_accepted_install_pending` is renamed
  `catalogue_accepted`; exit 7 now also covers a managed download. The setup plan gains
  its observed state beside the digested intent: required `install_needed`, each
  action's `state` (`pending` or `current`) and the model status `managed_current`;
  `readiness` and dependency statuses now include the managed tier, and acceptance
  compares the intent only, so an accepted plan survives its own install. An unreadable
  `setup install --plan` is `STORAGE_IO` (was `COMMAND_NOT_IMPLEMENTED`). The frozen
  reserved-command examples use `setup.repair`. Known limits: L-006 narrowed to tools
  VSift does not manage; L-037 rewritten (lifecycle commands still reserved); new
  L-087 (managed versions are rehashed per command) and L-088 (tunnel proxy
  authentication is recognised by a dependency's error text).
- **Development builds reach no publisher** (P13 PR 4): the managed download's client
  in a debug build (every test run without `--release`) resolves no host name, so a
  test that accepts a real plan cannot download from the internet;
  `VSIFT_DEV_PUBLISHER_NETWORK=allow` lifts it for a developer. Release builds are
  unchanged.
- SEC-T01 is met for P11 by non-adversarial evidence (the strict-Linux attestation
  checks and the hardened `strict-worker-boundary` container controls), by maintainer
  decision of 2026-09-28; the adversarial containment evidence is technical debt,
  required before the R0 release (new known limit L-068). ADR 0021 section 10 carries
  the amendment.
- Known limits: L-038 now states the worker host's final P11 position (a
  qualification target, pending merge and P14); L-004 points to L-068; L-010 now
  covers only candidates and evidence calls (batches are recoverable); L-055 and L-057
  point to the runbook. New L-069: a request that failed for good because of the host
  (`MISSING_CAPABILITY`, the free-space reserve) replays that failure under its
  operation id.

- Local speech recognition now uses at most as many threads as the session root's
  admission capacity (4 on a desktop root; previously up to 8), and that count is part
  of the run's provenance, so revision ids can differ from earlier runs and between
  roots of different capacity (L-023); a job interrupted before this change with more
  threads starts afresh. A visual-candidate window now reserves two admission units
  (P11 PR 2).

- ADR 0017 decision 4 is superseded: the CLI traps Ctrl-C and `SIGTERM` (see Added).
- A provider that fails after its caller cancelled is reported as cancelled, so an
  interrupt can no longer be recorded as an `undecodable` visual window or count
  towards poisoning a transcription chunk.
- A retranscription with an operation id is answered from its record before the tools
  are resolved; without one, the model is still checked before the session is read.
- Job records written by this version carry `planned_chunks`; P10 PR 2 builds reject
  them (sessions are disposable). The engine's `IngestRequest` takes a `Cancellation`,
  `Engine::job_resume` returns a `JobResumeReport` and `job_cancel` reports the job.
- Session caps raised (ADR 0020 D-2): a session holds 512 artifacts, of which 384 may
  be evidence (160 before), and a generation or bundle manifest may be 128 KiB; the
  evidence-budget remediation names the new numbers. Measured with the evidence budget
  full, warm reused requests do not grow with the manifest chain.
- A publication that crashed between its manifest and its pointer no longer blocks the
  session: the next publication replaces the unreferenced manifest (known limit L-048
  removed). A commit that follows a moved session now reads the newest revision and the
  generation from one manifest.

- Warm requests no longer slow down as a session ages (#164, P10 PR 1): a session read
  checks the manifest chain only down to a checkpoint its writer keeps
  (`chain-verified.json`), instead of every generation back to the first. A reused
  `frame get` through the binary stays at p95 136-175 ms at 256 generations and
  149-156 ms at 1,024 (it was 1,064 ms and 3,794 ms). Every read still verifies the
  head and every generation since the checkpoint, artifacts are still re-hashed, and
  `session retain` and `session clean` still check the whole chain.
- `transcript retranscribe` now checks the session's copy of the video twice per run
  instead of before every 30-second chunk: it verifies the copy's SHA-256 when the
  run starts and again before the new revision is saved, and before each chunk only
  compares the file's size, modification time and file identity. Long videos no
  longer pay a full read of the copy per chunk (on an 869 MB, 24-chunk video,
  decoding took 25.5 s instead of 173.4 s). The integrity guarantee is the same as
  before (ADR 0012, issue #148); results and schemas are unchanged and no failure code
  was added.
- Delivery is re-planned by ADR 0015. Managed dependency installation
  (`setup install` and its repair, list, rollback and remove lifecycle) moves from
  P06 to P13 and remains an R0 release requirement. P06 now closes on detection,
  bring-your-own selection, verification of the selected tools and manual guidance.
  Behaviour is unchanged: `setup install` still returns `COMMAND_NOT_IMPLEMENTED`.
- ADR 0016 commits VSift to an embeddable engine library and a published evidence
  contract, starting at P07.
- Internal reorganisation with no behaviour change: the v1 JSON response types moved
  from the CLI into a new `vsift-contract` crate that every future host will share.
  Command output, exit codes and schemas are unchanged.
- Internal reorganisation with no behaviour change: VSift's engine is now a Rust
  library, the `vsift` crate, and the command-line tool is a thin layer over it.
  Future hosts such as a worker or a desktop app will use the same library. Command
  output, exit codes and schemas are unchanged; the library API is not yet stable.
- The governance check now keeps the two session handoff files to a current-state
  size. The earlier day-by-day log is archived in `docs/history/`.

### Fixed

- **Managed power-loss campaign: a command in flight is no longer counted as a lost
  acknowledgement** (2026-10-01, P13 PR 7 follow-up; ADR 0023 PR 7 addendum). The first
  `P13 managed power loss` run on `main` (run 36793177930) failed with 53 of 134
  acknowledgements reported lost, but no store state went back: at every one of the 240
  failing points the component held the selection the *next* acknowledged command
  reported (a newer install, a rollback, or nothing after a component removal). Those
  points fall between that command's selection flush and its acknowledgement mark,
  where the previous acknowledgement is still the last one required; a managed
  selection is overwritten, so the verifier compared a newer state with an older one.
  The session store's verifier never had this problem because a session's generation
  only grows. The workload now logs a `start-<seq>` mark before each command, each
  replay point records the last command started, and the verifier also accepts the
  selection reported by the one acknowledged command that had started and is not yet
  required. A selection that went back, a reappeared removal and a selection no
  in-flight command reported are still losses. The store, its flushes and the
  acceptance rule (zero lost acknowledgements, at least 500 points, no damage, clean
  `e2fsck`, a negative control that loses acknowledgements) are unchanged. Tests:
  `managed::tests::a_selection_the_command_in_flight_made_is_not_a_loss_but_an_undone_one_is`,
  `managed::tests::only_the_next_acknowledged_command_is_in_flight_once_it_started`,
  `logwrites::tests::a_point_records_the_last_command_started_before_it`.
- **Compact-tier re-run: the 90% target is met; L-085 is closed** (2026-09-30, #222;
  ADR 0022 note "the compact-tier re-run"). The re-run on `a0bfb06` used P12's final
  compact plan (28 trials per model over A-01 to A-07 and SEC-T02). Claude Sonnet 5.5
  (Claude Code 2.1.284) passed 26 of 28 trials fully (93%). GPT-6-Sol (codex-cli
  0.155.0-alpha.16, Linux container) passed 23 of 28 as run and 28 of 28 (100%) after
  the grader change below. Both answered 28 of 28 correctly. An aborted first attempt
  on `d43a518` is not counted. The 62 bounded records are in
  `docs/planning/p12-agent-trials/rerun-222/`.
  - **Trial grader:** an `rg --files` listing whose `-g`/`--glob` filters include an
    exclude glob with a `/` separator (`!evidence-bundle-phase-1/**`) is orientation
    housekeeping when it names no path (maintainer decision of 2026-09-30). It prints
    names only, an exclude only narrows the listing, and `rg` skips the hidden folder
    that holds the session root. Such an exclude may not climb out, be anchored, or
    hold a backslash, a class or an alternation. With it, a path argument (even `.`),
    any other option (`--hidden`, `-u`, `--no-ignore*`, `-L`/`--follow`), a search
    pattern, an include glob with a separator, a pipe into anything but a line filter
    and a redirection all stay unauthorized. This was the only failure of Sol's five
    failed trials. Re-grading all 62 counted phases (`grade-222.json`) changed exactly
    those five and nothing else.
  - **Known limits:** L-085 (the compact tier below target) is deleted. New L-095:
    review-tier models can still state blurred content as supported by pixels, and
    the #224 skill fix awaits a review-tier re-run of A-09 blurred. L-007 counts the
    re-run's adversarial runs and no longer tracks #222.
- **The managed store recovers from kills it could not recover from before** (P13 PR
  7). A first `setup install` killed between creating the managed folder,
  `versions-v1` or `current-v1` and finishing its marker no longer leaves a folder every
  later command refuses (`STORAGE_IO`, deletion by hand): a folder holding nothing but
  the start of its marker is finished by the next install and read as empty meanwhile. A
  `setup remove` killed while creating its tombstone no longer leaves a version that
  cannot be removed, and one killed just before removing the empty version folder is now
  reported `removal_interrupted` with `setup remove --version` instead of a
  `missing_manifest` version to delete by hand.

- **Trial grader: `commands_only` allows `handoff check`** (2026-09-30, #222). The
  skill has run `vsift handoff check` on every draft report since P13 PR 5, but the
  two A-01 scenarios' `commands_only` lists did not name it. As a result, the first runs of
  the compact-tier re-run failed an agent for following the skill. The grader now
  treats `handoff check` like the help forms: it reads only the draft. Any other
  operation outside a scenario's list still fails the check.
- **P12 debt fixes** (2026-09-30, known limit L-085, ADR 0022 note "the P12 debt
  fixes"). No model was called; the compact re-run is #222.
  - **Trial grader (#219):** a looped clip's truth windows repeat with the clip's
    measured period, from the retained bundle's visual index. Before, they used the
    fixture's nominal duration. The A-02 clip's copies start 12.064 s apart, not 12 s,
    so by the last copy the windows were 2.56 s off. A bundle without an index keeps
    the nominal period and says so in the grade's `deviations`.
    - The 84 counted P12 phases were re-graded (`grade-debt.json`), and one changed:
      GPT-6-Sol's A-02 run 2 now passes, 24 of 28 full passes (86%).
  - **Agent skill:**
    - **#218:** the REPORT skeleton in `SKILL.md` shows one filled-in claim, a segment
      and a frame citation, and one untrusted instruction.
    - **#219 and #220:** each claim states its subject and its value in full.
    - **#220:** retain after the last evidence command, because the bundle is a
      snapshot.
    - **#221:** a defanged link belongs only in the Markdown report; the JSON
      describes the link without an address.
    - **#224:** a claim about a region a frame shows as unreadable is
      `partially_supported` on the transcript, not supported by the pixels.
    - `skill_contract` guards each rule, including that the schema refuses a
      defanged address in a summary.
- Windows evidence paths (#210, ADR 0019 note of 2026-09-29): `data.files[].path` of
  `frame`, `crop` and `audio` results is now the plain absolute form `C:\...` whenever
  that form names the same file (shorter than `MAX_PATH`, and no component that Win32
  normalisation would change: a trailing dot or space, a reserved device name, an
  invalid character). Otherwise it keeps the extended-length form `\\?\C:\...`, the
  documented fallback. Claude Code's file-reading tool and its permission rules refuse
  the extended-length form, which in the P12 trials cost agents a retry and an image
  read per frame and once made an agent report frames as unverified. Output only: the
  engine's verification and containment checks are unchanged, and Unix and macOS
  paths are unchanged.

- P12 trial grader: Claude Code's `Glob`, `Grep` and `LS` inside the skill folders count
  as reading the skill (a listing without a path, outside them or with a pattern that
  climbs out stays unauthorized), and the skill now says to read its files with the
  file-reading tool rather than list folders. Found by the first counted Claude Code
  trial, which listed the skill's `examples/`; the campaign restarted from zero.
- `job resume` of a job whose session is closed or expired advised renewing the
  session, which the CLI refuses for an expired session; its remediation
  (`JOB_SESSION_NOT_OPEN_REMEDIATION`) and the contract's failure row now say to open
  a new session with `ingest`. Found while writing the P12 skill; known limit L-070
  is removed.
- A process killed while it registered a new session could leave an empty entry in
  the session index. From then on every session listing, and the cleanup of that
  registration, failed with `INTEGRITY_FAILURE`, for good. A worker batch met this
  when one request's process was killed while another request registered its session
  (#197, seen on macOS CI and reproduced on Windows). A registration now writes its
  index entry to a staging file, flushes it and renames it into place, so a killed
  registration leaves no entry or a whole one. The batch kill test now names the
  fault point and the operation that failed.
- A storage failure while committed state was read (for example `EIO` from a disk,
  or from ext4 after it shut itself down on a write error) was reported as
  `INTEGRITY_FAILURE`, as if the evidence had been altered; it is now `STORAGE_IO`,
  and a missing, mistyped or linked entry is still an integrity failure. Found by the
  P10 crash campaign's write-error layer.
- An ingest reported the store's strongest guarantee rather than its session's own;
  on the newly qualified durable profile an ephemeral session would have been reported
  as `os_crash_durable`. It now reports what the session gets.
- The weekly fuzz workflow now also runs the `mountinfo` target (added in P10 PR 1 but
  missing from its list) and the new `os_release` target.
- The concurrent-preflight engine test no longer fails when a throttled runner makes
  the root's creator outlast the five-second wait (#144): the wait is injectable
  (`EnginePorts::with_session_root_wait`, at most 60 s) and that test waits longer.
- Several identical `transcript retranscribe` requests started at the same moment could
  make one of them fail with `STORAGE_IO` on macOS while creating the shared job's lock
  files; that open now retries briefly, as a file met mid-replacement does.
- A session could be reported as damaged (`INTEGRITY_FAILURE`) while another process was
  committing to it: a reader that opened the commit pointer, the chain checkpoint or a
  job record just as a writer replaced it by rename saw a file with no link left, or
  on Windows briefly no file, and took either for damage. Readers now retry such a
  file for at most half a second and read the committed version; linked, non-regular
  and missing files are still refused (P10 PR 2, found by the concurrent
  retranscription stress test).

- **Security (SEC-17):** the media adapter could report a time taken from a video's
  own metadata instead of what FFmpeg decoded. FFmpeg repeats a file's metadata (for
  example its title) in the same diagnostic output VSift reads frame and audio times
  from, and two readers accepted any line that merely contained the filter's name, so
  a crafted file could shift the times `transcript retranscribe` gave its own
  transcript segments. Readers now accept only lines the filter itself wrote, require
  a complete, consistent sequence of frames, and check the time base against the
  probed stream; anything else is rejected. No failure code or schema changed.
- `transcript retranscribe` could save a revision even if the session's copy of the
  video changed after the last chunk was decoded. The copy is now verified again
  before the revision is saved; if it changed, the run fails with
  `INTEGRITY_FAILURE` and saves nothing.
- The reviewed Ubuntu x64 whisper.cpp v1.9.2 file set now includes ggml's 13
  optimised CPU backends (`sse42` through `zen4`) from the same pinned archive,
  each pinned by size and SHA-256. Before, only the generic `libggml-cpu-x64.so`
  was selected, so local speech recognition on Ubuntu ran about 11 times slower
  than on Windows (#153). The Ubuntu `setup plan` whisper action now lists 25
  files instead of 12.
- Local speech recognition could drop a whole sentence that started exactly
  where a 30-second chunk begins, when the previous sentence ended just after that
  point, without any warning. The sentence is now kept once.
- `setup check` no longer echoes a provider's first output line as `detail`. With
  whisper.cpp v1.9.2 that line was a library-loader log naming an absolute folder,
  which broke the promise that paths are not echoed. FFmpeg and FFprobe now report
  only their `ffmpeg version ...` / `ffprobe version ...` banner line (or
  `detected`). Whisper's output is never echoed: `detail` is
  `whisper.cpp v1.9.2 (reviewed build)` when the executable's bytes match a build
  reviewed for P06, otherwise `whisper-cli (build not recognised)`. As a second
  guard, no line that looks like a path or a ggml loader log is ever shown.
- On a Windows profile whose `%LOCALAPPDATA%` gives other accounts access to new
  folders (for example a sandbox group or an app-container capability), `setup
  configure`, `setup configure-model` and `setup check` failed with `STORAGE_IO`
  and no explanation, because the folder VSift had just created inherited that
  access and VSift then correctly refused it. Every folder VSift creates for itself
  (the per-user configuration folder and its missing parents, the session folder
  and its parent, retained bundles, the managed-data folder) now gets its own
  permissions before anything is written: only you, SYSTEM and Administrators, with
  inheritance from the parent turned off. On Linux and macOS these folders were
  already created owner-only; a missing parent of a private folder is now owner-only
  too. A folder that already exists is never changed: if other accounts can access
  it, the command still stops, now with a remediation naming the folder
  (`user_configuration` or `session_root`) and how to fix it. A session folder in
  that state is now `STORAGE_IO` instead of `INVALID_ARGUMENT`. A folder another
  VSift process has only just created is given a moment to become private before
  it is judged, so commands started together do not trip over each other.
- Media-tool check record: a reader that caught another process replacing the
  record could mistake the replacement for an unsafe record (issue #136). On
  Windows this made a concurrency test fail in about half of its runs. The read
  is now retried and otherwise counts as "not verified"; a record with more than
  one link is still refused. A failed flush of a new record no longer discards the
  pass, since a record lost to a crash already just means one more check. Taking
  the record's write lock now retries brief failures a few times instead of
  skipping the pass (seen on macOS); a linked or non-regular lock file is still
  refused and is never reported as busy.
- Media-tool check workspaces left behind when VSift was killed during a check are
  now removed by a later check, once they are an hour old and no running check
  holds them (issue #132). Only exactly named VSift workspaces in the private
  per-user state directory are removed, and links are never followed.
- Several VSift commands started at the same moment on a machine that has no
  session directory yet no longer fail with `INTEGRITY_FAILURE` ("ownership marker
  is invalid") (issue #131). One of them creates the session directory; the others
  wait for it to finish, for at most five seconds, and then use it only after the
  usual ownership and privacy checks. If it is still being created after five
  seconds they fail with the retryable `BUSY`. A directory VSift did not create is
  still refused at once.
- The published v1 schemas now accept `ISOLATION_UNAVAILABLE` and
  `setup.configure-model`, which the CLI already emitted (issue #125).
- Locks are now always released explicitly instead of by closing their file
  (issue #66). On Linux and macOS a child process started by another thread
  briefly holds copies of every open file, so a lock released only by closing
  could stay held for a moment and make an immediate retry report `BUSY`. This
  caused the intermittent CI failures and would have affected a busy worker.
  It applies to session, registration, admission, root-initialization,
  configuration, managed-install and managed-version locks.
- Per-user dependency configuration now reports `BUSY` only when the OS says
  another handle holds its lock. Other lock acquisition failures surface as
  storage I/O; an intermittent hosted `BUSY` test symptom remains under review.
- Registration explicitly releases its short-lived root initialization lock
  before returning the long-lived marker hold, preventing a duplicated file
  descriptor from prolonging root contention during an immediate bucket scan.

### Added

- P06 now fixes the Ubuntu managed candidate's compatibility policy in the
  reviewed catalogue: the exact checked-in F01 fixture, expected FFmpeg build
  and FFprobe identities, 64-KiB per-stream and transcript limits, 256-KiB
  generated-audio limit, 60-second media deadline, 180-second inference
  deadline, and 16-kHz mono audio contract. Catalogue
  completeness and the accepted plan digest bind these values; an invalid or
  changed policy cannot reuse prior acceptance. Production smoke execution and
  activation remain pending.
- P06 can now strictly and boundedly decode a saved `setup plan --json`
  document, rebuild the plan from current target, catalogue, configuration,
  probes and time, require the entire presentation to remain unchanged, and
  verify the separately supplied acceptance digest. Malformed or stale readable
  plans fail before transfer or filesystem mutation. A valid plan still ends in
  `COMMAND_NOT_IMPLEMENTED`; compatibility smoke and the installer transaction
  remain pending.
- P06's pinned multilingual `base` model now uses the same accepted-action
  authority as the Ubuntu archives. Exact model bytes can be copied into a
  private unactivated payload and runtime with bounded size/SHA-256 rechecks;
  unsafe names or mismatched review fail before mutation. The disposable
  Ubuntu workflow exercises the path against fresh publisher bytes. Provider
  compatibility and managed activation remain pending.
- P06 accepted Ubuntu actions can now be rebound to exact reviewed publisher
  source and passed through the owned archive/payload/runtime preparation path
  using the catalogue inventory itself. Changed action fields or mismatched
  staged bytes fail closed. This remains unactivated; raw-model staging,
  compatibility smoke and the public installer are still pending.
- P06 now records an exact Ubuntu 24.04 x86-64 reviewed catalogue for the
  pinned month-end FFmpeg/FFprobe build, whisper.cpp v1.9.2 CLI and multilingual
  `base` model. `setup plan` emits only currently needed actions with direct
  publisher URLs, pinned bytes/hashes, archive and installed-file inventories,
  licence/source disclosures, trust limits, private destination, and a
  deterministic state-bound acceptance digest. It stops new plans on 2028-08-01
  and returns typed managed-unavailable guidance elsewhere. Installation and
  compatibility preflight remain unavailable; the plan makes no legal-clearance
  claim.
- P06 published runtimes now hold shared per-version OS locks. A guarded
  transaction can atomically select an older published version for rollback and
  remove only an unselected version after obtaining its exclusive lock. Selected
  or live-held versions remain intact; a private tombstone makes interrupted
  exact-file removal retryable through metadata deletion and a lost response.
  A native child-process test proves a live hold blocks removal and abrupt
  process exit releases it. Public install, rollback and uninstall commands
  remain unavailable.
- P06 can now publish a fully rechecked prepared runtime under a canonical
  component/version identity and atomically select it with a hashed pointer while
  holding the root installation guard. Published versions retain exact manifests,
  regular-file identity, private modes and SHA-256 checks; interrupted pointer
  replacement is retryable and prior versions remain readable. This infrastructure
  primitive carries no catalogue, compatibility or plan-acceptance authority.
- P06 now has a root-wide managed installation guard backed by a private,
  single-link OS-locked file. Concurrent writers receive typed `Busy` without
  waiting or retrying; linked or incorrectly permissioned lock files fail
  closed. The guard serializes future transactions but grants no install authority.
- The opt-in disposable Ubuntu P06 qualification workflow now sends a freshly
  bounded and SHA-256-verified whisper.cpp archive through the production Rust
  owned-runtime layout check before running the separate candidate compatibility
  smoke. It still grants no catalogue, plan, activation or install authority.
- P06 can now copy a verified payload into a fresh private, unactivated
  `runtime.pending` directory with only reviewed regular-file aliases and
  selected Unix owner-executable modes. Every copy is bounded and rechecked;
  failed preparation removes only its owned runtime files. A pinned Ubuntu
  whisper.cpp archive passed this layout stage without binary execution.
- P06 `setup plan --profile` now performs a read-only configured/PATH executable
  diagnosis. Until a per-target managed artifact is qualified, its v1 result
  reports an unavailable managed path, no install actions or acceptance digest,
  and typed manual BYO steps. `setup install` remains reserved.
- P06 now composes a verified managed artifact with bounded raw tar, gzip/tar
  or XZ/tar selected-file staging under a fresh private payload directory.
  Selected files are rechecked before use; changed, linked or unexpected files
  block opening and cleanup removes only the reviewed selection. The payload
  remains unactivated and managed installation remains unavailable.
- P06 can transfer an exact reviewed publisher artifact over direct HTTPS into
  the private unactivated stage. Immutable GitHub release and Hugging Face
  model routes admit only their reviewed CDN redirect, with bounded deadlines,
  cancellation, whole-artifact size/SHA-256 verification and no resume.
  Managed installation remains unavailable.
- P06 now has a positively marked private per-user managed root and one-artifact
  staging transaction. It verifies exact reviewed bytes on import and again
  before archive use, removes its own stage after failed import, and refuses
  unmarked roots or unexpected staging entries. This is an infrastructure
  boundary; managed installation remains unavailable.
- P06 bounded archive adapters can stage an exact reviewed regular-file
  selection into an empty private directory capability. Staging uses portable
  flat names, create-new/no-follow writes and private modes, ignores archive
  links/directories/modes, and removes files it created when any later archive
  or compression check fails. This infrastructure primitive does not activate
  managed installation.
- P05 foreground disposable `ingest`, session list/status/renew/close/clean,
  explicit evidence-only or source-inclusive retain, and data-only bundle
  validation. Source and frame/audio artifacts use P03's private
  capability-scoped generations; a bounded index and held OS locks protect
  active or abandoned sessions during cleanup. Retained output reports
  process-crash-consistent publication under ADR 0013. The opt-in P05
  checkpoint runs real media through artifact publication, both export modes
  and source-preserving cleanup.
- P04 internal source snapshot and bounded FFprobe/FFmpeg media adapter with typed
  stream metadata, actual frame/audio timestamps, source identity and an opt-in
  real-media checkpoint. Project-owned synthetic fixtures include VFR, rotation,
  audio-track and malformed variants with independent provenance verification.
- Accepted ADR 0011 and scoped R1 as the managed industrial capability expansion:
  optional enrichment, source-grounded composition, explicit catalogue lifecycle,
  industrial worker growth and integrated qualification in P15-P20. R0 now has an
  explicit two-agent end-to-end release gate and MCP remains a later adapter.
- P03 native filesystem/lock feasibility experiments and a recorded OS/storage
  crash-qualification blocker. ADR 0010 now accepts ephemeral NTFS/APFS desktop
  qualification for P03 and defers strict Ubuntu/ext4 durable enablement to the
  P10/P11/P14 fault campaign.
- Began P03 implementation with typed durability requirements, qualified publication
  guarantees, non-wrapping storage generations, and an application gate that rejects
  unsupported durable requests before invoking the mutating session-store port.
- Added the first internal capability-scoped filesystem session-store adapter: it
  validates an existing owned root, serializes initialization with a stable OS lock,
  publishes an immutable checksummed generation zero, and verifies it before reuse.
  The adapter is not yet composed into a public command.
- Completed the internal P03 storage/coordination boundary in PR #42 with
  owned private-root provisioning, Unix owner/mode and Windows DACL validation,
  immutable root-wide weighted admission, shared/exclusive lifetime holds,
  generation-fenced publication, bounded integrity-chain recovery, and deterministic
  error/process-crash tests at every manifest and pointer boundary. Durable requests
  remain rejected before mutation and no session command is exposed. PR #43 also
  makes concurrent lock-contention tests wait against a bounded monotonic deadline
  instead of assuming a fixed number of scheduler yields.

- Initial Rust workspace and architectural boundaries.
- Read-only `vsift setup check` runtime diagnostic with versioned JSON output.
- Contributor, security, governance, and automation foundations.
- Detailed proposed implementation blueprint, source baseline review, threat model,
  verification matrix and work packets for desktop and server-worker execution.
- Accepted R0 architecture decisions, qualification/resource profiles, synthetic
  fixture truth, GitHub packet backlog and CI-enforced anti-drift delivery ledger.
- Published the typed v1 R0 command namespace, JSON and JSONL terminal envelopes,
  stable errors/exits, configuration precedence, schemas, and compatibility examples.
- Added domain contracts for identifiers, source time/ranges, crops, paging cursors,
  confidence/provenance metadata, and legal job terminal transitions.
- Replaced environment-dependent CLI assertions with deterministic contract,
  compatibility, boundary, and property tests. Reserved operations fail explicitly
  without claiming their later implementation.
- Routed external setup probes through a shell-free process supervisor with canonical
  executable provenance, an allowlisted environment, bounded concurrent output,
  shared deadlines, caller cancellation, descendant cleanup, and truthful reporting
  of process containment versus strict worker isolation.
