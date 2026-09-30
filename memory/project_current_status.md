# VSift current status

As of 2026-09-30. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
Today it can:
- check and register its dependencies, show a read-only setup plan, report whether
  local speech recognition really works here (`setup check` `local_asr`), and on
  Ubuntu 24.04 x86-64 install the reviewed FFmpeg, whisper.cpp and model itself
  (`setup install`, P13 PR 4, in review);
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an offset;
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`),
  continue an interrupted transcription where it stopped, and report, resume or cancel
  that work by its job id (`job status/resume/cancel`), with chunk progress;
- return timestamped transcript segments, search them, list the moments where the
  screen changed, and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, and validate retained bundles;
- stop a long command cleanly on Ctrl-C or `SIGTERM`, never leaving a helper running;
- run as a worker under an external supervisor (`session init-workspace`, `job run`,
  `job batch`), each step at most once per operation id, with weighted admission and a
  shutdown that leaves every started request resumable;
- on Ubuntu 24.04 with local ext4, keep every acknowledged result of a durable
  workspace through an OS crash or power loss;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user;
- check an agent's draft report before it is sent (`handoff check`, P13 PR 5, in review).

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an
investigation with the CLI and write a cited report. P12's named-client trials
qualified it:

- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex each passed
  11 of 11 trials mechanically and 9 of 11 fully.
- **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol each passed 23 of 28 fully. That
  is 82%, below the 90% target, and is recorded as debt.
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or
  copied a hidden character into a report. Codex ran in a Linux container (L-076, #204).

**P00-P12 are complete.** P12 closed on 2026-09-30 by maintainer decision on its
final round's results (#223, `1284e54`); the ledger marks it `complete`.
**P13 is in progress** (started 2026-09-30). PRs 0, 1, 2a, 2b and 3 and the P12 debt
fixes are merged (human output is done); PR 4 (`setup install` and the managed lookup
tier) is in review. The packet is not complete.

## P13 in one view

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(**Proposed**) records the maintainer's decisions A-H of 2026-09-30; the scope and
the 13 pull requests are in `implementation-work-packets.md` ("P13 scope and pull
requests"). The ledger marks P13 `in_progress` and maps R-03, R-13 and R-14 to it.

- **Delivers:** native archives and `release.yml` (Windows x64, macOS 15 arm64, Linux
  x64 glibc) with SBOMs, notices, checksums and Sigstore provenance; the npm launcher
  `vsift` over `@<scope>/…` packages; managed installation on Ubuntu 24.04 x64 (`setup
  install/list/rollback/remove/repair`); human output by default; `handoff check`.
- **Open:** the platform-package scope. npm refused `@vsift`; the maintainer picks
  `@vsift-cli`, `@vsifthq` or `@vsiftdev` and records it in an ADR 0009 note.
- **Done:** PR 0 (#226): ADR 0023, ledger, issue #16. PR 1 (#228, L-071 closed): typed
  parse remediation. PR 2a (#229) and PR 2b (#231, `02df4eb`, L-073 closed): readable
  text for every command through `TerminalText`, SEC-T02 over all of it. PR 3 (#230,
  `e22ee59`): `smoke_before_activation` smokes staged, unactivated candidates (layout,
  banners, F01 media and speech fixtures, recheck); a failure discards every candidate
  or keeps and reports an unprovable stage. Hosted run 36701212028 passed the real tools.
- **In review: PR 4** (`p13-pr4-install`, resume steps 3-4). `setup install --plan
  --accept-plan [--artifact-dir]` takes the install guard without waiting (`BUSY`),
  rebuilds and revalidates the plan, then installs media tools, whisper.cpp CLI and model
  in order: download over HTTPS (or import from the folder by the catalogue's file names)
  with exact size and SHA-256, stage, smoke (CLI and model together), `publish_and_select`
  per component. `already_current` skips a selected version; the first failure stops it
  (`blocked` after it) and a rerun continues. No resume, no `Range`, `206` refused.
  `DOWNLOAD_FAILED` (exit 7: `tls`, `redirect_policy`, `http_status`, `proxy_auth`,
  `offline`, `size`); digest mismatch `INTEGRITY_FAILURE`; failed smoke
  `MISSING_CAPABILITY`. Every lookup: per call, configured, managed, `PATH`; a managed
  version runs only after its manifest digests match (L-006 closed for it) and a job holds
  it. `setup plan` observes tools outside the managed store (L-086). Tests on every CI OS
  use local HTTP, TLS and proxy servers through the dev-only `install-test-hooks`. The
  real install (`P13 managed smoke`, job `managed-install`) runs after merge.
  **Next:** PR 5 (`handoff check`), then 6 (list/rollback/remove/repair, cleanup, sweep).
- **Nothing is published** until P13 completes and the maintainer approves one 0.x
  pre-release under npm's `next` tag; the npm setup is the maintainer's.
- **Found while planning:** the clean-install agent run is P14's (H10); Linux needs OpenSSL 3.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Accepted**; record [p12-agent-qualification.md](../docs/planning/p12-agent-qualification.md)
with 84 trial records. Built: the skill and `skill_contract` guard (#196), the harness
`tools/vsift-agent-trials` (#201), fix rounds #203-#217; review 19 accepted, 6 rejected.

## Found in P12 (still open)

- **L-085 (debt):** compact tier 82%; fixes for #218-#221 and #224 done, the re-run
  (#222) follows P13 PR 5. **Also open:** L-074, L-075, L-076 and #204, L-078 to L-084;
  grader readings (`untrusted_listed`, an `rg --files` exclude glob) in `memory/TODO.md`.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`;
  `setup install` on Ubuntu 24.04 x86-64 (PR 4, in review).
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`.
- `transcript retranscribe`, `transcript get`, `job status|resume|cancel|run|batch`,
  `search`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean/init-workspace` and `bundle validate`.
- `handoff check` (P13 PR 5, in review): a draft report from stdin or `--file`.
- Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`. A command line
  that does not parse names its mistake and the `--help` to read (P13 PR 1).
- Readable terminal text without `--json` for every command (unstable, not for parsing).
- Still `COMMAND_NOT_IMPLEMENTED`: setup repair/list/rollback/remove (P13 PR 6).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | Complete (2026-09-27, `e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10 | Complete (2026-09-28, `3f27ce3`): jobs, resume, cancellation, durable Ubuntu/ext4 |
| P11 | Complete (2026-09-28, `40c4038`); SEC-T01 adversarial evidence is technical debt (#188, L-068) |
| P12 | Complete (2026-09-30, maintainer decision): skill, harness, named-client trials; review tier qualified, compact tier 82% (debt, L-085); merge `1284e54` |
| P13 | In progress (started 2026-09-30, ADR 0023 Proposed): PRs 0, 1, 2a, 2b and 3 merged (#226, #228, #229, #231, #230); PR 4 (`setup install`) in review; lifecycle commands, distribution and `handoff check` (#213) to come |
| P14 | Not started |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. The worker lives in the engine (`worker.rs`, `batch.rs`); the CLI
only presents. The agent skill (`skills/vsift/`) sits outside the crates and only
calls the `vsift` binary; its guard is a test module of `vsift-cli` because the parser
is crate-private. The trial harness `tools/vsift-agent-trials` depends only on `vsift`
(and runs the `vsift` binary for everything else) and on `vsift-contract` for the shared
handoff check.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy with and without
  features, workspace tests, warning-denied rustdoc and governance.
- **CI on every PR:** Quality on Ubuntu, macOS and Windows; Documentation, Governance,
  fuzz harness replay, the strict worker boundary, dependency policy and CodeQL.
- **Merging:** squash merges to protected `main`; history in git, `CHANGELOG.md` and
  `docs/history/`.
