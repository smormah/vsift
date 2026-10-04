# Known limits register

Date: 2026-10-04 (P14 PR 7, the dedupe window is stated as it is: L-063 updated; P14 PR 7b: the FFmpeg finding re-read with a test that sees release-branch cherry-picks, L-122 narrowed from 35 records to one tie by elimination and L-132 added (L-129 to L-131 are P14 PR 7's); 2026-10-03: P14 PR 4: the robustness campaigns, L-122, L-123, L-124, L-127 and L-128 added (L-121, L-125 and L-126 were taken meanwhile by other pull requests); P14 PR 7, a missing shared library is named: L-110 closed; P14 PR 7: a session root VSift did not create now explains itself, L-126 added; P14 PR 7, the realistic cold-agent settings: L-125 added, L-118 re-read; P14 PR 5: L-068 rescheduled to R1 and L-004 re-read, by the maintainer's decision E option 4; 2026-10-02: P14 PR 3, the journeys on the published binary: L-113 to L-116 added, L-035, L-042 and L-099 updated; the README's graphics: L-121 added; P14 PR 6: the trial harness's clean-install and cold-agent modes, hold-outs and usage capture, L-117 to L-120 added; the README front page: L-102 closed, the v0.1.0 release page edited; P14 PR 2: the published-artifact qualification, L-109 to L-112 added; P14 PR 1: the evidence ledger, the claims registry and their checks, L-101 to L-103 added; P14 PR 8, the stable path and `latest`: L-105, L-107 and L-108 added, L-102 narrowed to the published v0.1.0 page, L-103 updated for the delta record, L-097 updated; P14 plan, PR 0: L-042 re-read, the checkpoints' source-built binary noted, L-098 updated with Smart App Control read Off on the maintainer's machine; 2026-10-01: P13 PR 12: the 0.1.0 pre-release was published, so L-036 and L-096 are closed and deleted, L-037 and L-043 re-read at the packet's close, L-097 and L-098 updated, L-100 added; P13 PR 11: documentation and the qualification record, L-098 and L-099 added, L-035, L-036, L-037, L-042 and L-096 updated for the passing power-loss run and the closing sweep; P13 PR 7 follow-up: the power-loss campaign's first run and its verifier fix, L-037 updated; P13 PR 10: attestation and publish wiring, L-036 updated, L-096 and L-097 added; 2026-09-30: P13 PR 7: kill tests of the managed store, directory flushes and its power-loss campaign, L-037 narrowed; the compact re-run #222 met its target: L-085 closed, L-095 added for the review tier's A-09 blurred re-run (#224), L-007 updated; P13 PR 9: npm packages and their qualification, L-091 to L-093 added and L-036 updated; P13 PR 6: managed lifecycle, L-037 narrowed and L-087 measured, L-090 added; P13 PR 4: managed installation; P13 PR 8: release archives, L-089 added and L-036 updated; P00-P13 complete; P12 closed on its final trial round with the compact tier below target, L-085; SEC-T01's adversarial evidence deferred as technical debt, L-068; P13 PR 2b closed L-073 and rewrote L-016 and L-017).
Status: current-state register. Every entry below is **pending maintainer review**.

## Purpose and how to use it

This is the one place that lists what VSift cannot do yet, does less well than it
should, or deliberately accepts as a risk. It gathers limits that were recorded across
the qualification records, ADRs, the threat model, the contract, the work record and
open GitHub issues, so they can be reviewed and evaluated together. It does not
replace those documents: each entry links to its source, which stays authoritative
for the detail.

- **Current state, not a log.** Rewrite an entry when its facts change; delete it in
  the change that removes the limit (and say so in `CHANGELOG.md`). History lives in
  git. IDs are never reused.
- **Plain English first, precise second.** Each entry says what the limit is and who
  it affects, then the numbers and where they were measured.
- **Scheduled work is listed too.** A missing R0 capability that a later packet owns
  (for example the agent skill) is a limit of VSift *today*, so it appears here with
  its owner packet. Being listed does not make it eligible ahead of the
  [delivery ledger](delivery-ledger.json).

Each entry has these fields:

| Field | Meaning |
| --- | --- |
| Area | security, integrity/durability, performance, accuracy/ASR, visual detection, corpus/fixtures, platform/distribution, contract/UX, process/CI |
| Severity | high, medium or low, by the rubric below |
| Owner | the packet that will address it, or **unscheduled** |
| Issue | the tracking GitHub issue, or **none** |
| Status | **open** (a defect or gap to fix), **accepted residual** (a known risk accepted by a recorded decision), **deferred** (scheduled or scoped to later work), **monitoring** (watched; act on recurrence or new data) |
| Review | the maintainer's decision: **pending**, or accepted / rejected / rescheduled with a date (see [Review workflow](#review-workflow)) |

**Severity rubric.**

- **High:** could let altered, forged or wrong evidence be presented to an agent as
  verified, or blocks an R0 release gate with no workaround.
- **Medium:** users or agents will meet it in normal use (reduced accuracy, slow calls,
  a missing convenience), or a qualification claim in that area is still unproven; a
  workaround or an honest disclosure exists.
- **Low:** an edge case, a bounded cost, a cosmetic issue, or a residual inside the
  documented threat model.

## Summary

| ID | Title | Area | Severity | Owner | Issue | Status |
| --- | --- | --- | --- | --- | --- | --- |
| [L-001](#l-001) | Evidence calls check the source copy by identity only (D1) | security | low | unscheduled | none | accepted residual |
| [L-002](#l-002) | A change undone before the closing hash is not seen (bracketed binding) | security | low | unscheduled | none | accepted residual |
| [L-003](#l-003) | A forged diagnostics line that continues the real numbering (SEC-17) | security | low | unscheduled | none | accepted residual |
| [L-004](#l-004) | Native decoders and the recognizer are not sandboxed on the desktop | security | medium | P14 | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-005](#l-005) | Private Windows folders get their DACL just after creation, not atomically | security | low | unscheduled | none | accepted residual |
| [L-006](#l-006) | The media-tool check record trusts file identity, not executable contents, for tools VSift does not manage | security | low | unscheduled | none | accepted residual |
| [L-007](#l-007) | Evidence can carry instructions; agents can leak delivered paths | security | medium | unscheduled | none | accepted residual |
| [L-008](#l-008) | OS-crash durability is qualified only on Ubuntu 24.04 with local ext4 (FS-01) | integrity/durability | medium | P11, P14 | [#14](https://github.com/smormah/vsift/issues/14), [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-009](#l-009) | Cleanup and erasure leave some work to the user | integrity/durability | low | unscheduled | none | accepted residual |
| [L-010](#l-010) | Visual candidates and evidence calls are not recoverable jobs | integrity/durability | low | unscheduled | none | accepted residual |
| [L-011](#l-011) | Evidence on large sources is slow; the first call hashes the whole copy | performance | medium | unscheduled | [#170](https://github.com/smormah/vsift/issues/170) | monitoring |
| [L-013](#l-013) | Evidence records and transcripts are re-read in full on every call | performance | low | unscheduled | [#171](https://github.com/smormah/vsift/issues/171) | monitoring |
| [L-014](#l-014) | A session holds at most 384 evidence files (512 artifacts, 128 KiB manifest) | contract/UX | low | unscheduled | none | accepted residual |
| [L-015](#l-015) | Bursts over more than 20 s of 60 fps video are refused | contract/UX | low | unscheduled | [#172](https://github.com/smormah/vsift/issues/172) | open |
| [L-016](#l-016) | Delivered file paths die with the session; very long Windows paths keep the `\\?\` form | contract/UX | low | unscheduled | none | accepted residual |
| [L-017](#l-017) | Human mode shows no progress; the worker hosts' event stream is JSON Lines only | contract/UX | low | unscheduled | none | accepted residual |
| [L-018](#l-018) | Tiny text is measured on synthetic glyphs only; crops are never upscaled | visual detection | low | unscheduled | [#173](https://github.com/smormah/vsift/issues/173) | open |
| [L-019](#l-019) | A seek that lands past the requested frame reports "not found" | contract/UX | low | unscheduled | none | accepted residual |
| [L-020](#l-020) | Noisy speech: `base` word error rate 61.5% on F08, not gated | accuracy/ASR | medium | unscheduled | [#150](https://github.com/smormah/vsift/issues/150) | deferred |
| [L-021](#l-021) | Reviewed known misses: "queued", "4407", "E-409" | accuracy/ASR | medium | unscheduled | none | accepted residual |
| [L-022](#l-022) | No accent, crosstalk, human-voice or long-recording ASR evidence | accuracy/ASR | medium | unscheduled | [#150](https://github.com/smormah/vsift/issues/150) | open |
| [L-023](#l-023) | ASR output differs across CPU backends; revision ids differ by host and root | accuracy/ASR | low | unscheduled | none | accepted residual |
| [L-024](#l-024) | An ASR segment can start at the audio's start, before the speech | accuracy/ASR | medium | unscheduled | [#174](https://github.com/smormah/vsift/issues/174) | open |
| [L-025](#l-025) | Local ASR runs: progress is coarse and advisory, model hashed per run | contract/UX | low | unscheduled | none | accepted residual |
| [L-026](#l-026) | whisper.cpp output with a split multi-byte token fails the chunk | accuracy/ASR | low | unscheduled | none | accepted residual |
| [L-027](#l-027) | whisper.cpp is the only speech engine | accuracy/ASR | low | unscheduled | [#147](https://github.com/smormah/vsift/issues/147) | deferred |
| [L-028](#l-028) | Change thresholds are calibrated only on the synthetic corpus | visual detection | medium | unscheduled | [#175](https://github.com/smormah/vsift/issues/175) | open |
| [L-029](#l-029) | 2 Hz sampling misses changes shorter than 0.5 s or below the change rule | visual detection | medium | unscheduled | none | accepted residual |
| [L-030](#l-030) | Motion fixtures draw no motion; scrolling and cursors only synthetic (F04/F05/F12-E02) | corpus/fixtures | medium | unscheduled | [#159](https://github.com/smormah/vsift/issues/159) | open |
| [L-031](#l-031) | The visual index is tied to the probed duration; 30 minutes per call | visual detection | low | unscheduled | none | accepted residual |
| [L-032](#l-032) | Search: no Unicode folding, no cross-segment phrases, no compound number words | contract/UX | medium | unscheduled | none | accepted residual |
| [L-033](#l-033) | A supplied transcript is assumed to cover the whole video | contract/UX | low | unscheduled | none | accepted residual |
| [L-034](#l-034) | Speech fixtures are synthetic and partly unaligned | corpus/fixtures | low | unscheduled | none | accepted residual |
| [L-035](#l-035) | Evidence on three systems exists only for the published 0.1.0 and a synthetic corpus; no platform has the release matrix's rules met yet | platform/distribution | medium | P14 | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-037](#l-037) | Managed installation is qualified on Ubuntu 24.04 x64 only, and its power-loss claim is for ext4 only | platform/distribution | low | P14 | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-038](#l-038) | The worker host is a qualification target, not a supported platform | platform/distribution | medium | P11, P14 | [#14](https://github.com/smormah/vsift/issues/14), [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-040](#l-040) | Process-supervisor tests fail intermittently on Windows under load | process/CI | low | unscheduled | [#128](https://github.com/smormah/vsift/issues/128) | monitoring |
| [L-041](#l-041) | A creator slower than 5 s makes a racing command `BUSY` | process/CI | low | unscheduled | [#144](https://github.com/smormah/vsift/issues/144) | accepted residual |
| [L-042](#l-042) | Real-tool success paths run only on demand in pull-request CI; the published binary's run is weekly | process/CI | medium | P14 | [#178](https://github.com/smormah/vsift/issues/178) | open |
| [L-043](#l-043) | Library API unstable; MSRV and MCP decisions open | contract/UX | low | unscheduled | [#176](https://github.com/smormah/vsift/issues/176) | open |
| [L-044](#l-044) | Accepted engineering trade-offs (CLI test dependencies, session compatibility) | contract/UX | low | unscheduled | none | accepted residual |
| [L-045](#l-045) | Several documents and trackers state an outdated position | process/CI | low | unscheduled | [#177](https://github.com/smormah/vsift/issues/177) | open |
| [L-046](#l-046) | Deliberate scope exclusions (live sources, OCR, speakers, URLs) | contract/UX | low | R1 or later | [#107](https://github.com/smormah/vsift/issues/107), [#108](https://github.com/smormah/vsift/issues/108) | accepted residual |
| [L-047](#l-047) | A read no longer re-verifies generations below the chain checkpoint | integrity/durability | low | unscheduled | none | accepted residual |
| [L-049](#l-049) | Checkpoints resist corruption, not a same-user forger | security | low | unscheduled | none | accepted residual |
| [L-050](#l-050) | Jobs and operation ids are bounded per session | contract/UX | low | unscheduled | none | accepted residual |
| [L-051](#l-051) | Some interrupted work is redone rather than resumed | performance | low | unscheduled | none | accepted residual |
| [L-052](#l-052) | A read that meets a file being replaced waits for it, at most 0.5 s | performance | low | unscheduled | none | accepted residual |
| [L-053](#l-053) | Windows: a process that inherited "ignore Ctrl-C" sees only Ctrl-Break | platform/distribution | low | unscheduled | none | accepted residual |
| [L-054](#l-054) | A second interruption cannot cut short VSift's own work between boundaries | contract/UX | low | unscheduled | none | accepted residual |
| [L-055](#l-055) | On Unix a hard-killed CLI's running provider finishes its current unit | security | low | unscheduled | none | accepted residual |
| [L-056](#l-056) | Durability rests on storage that honours flushes | integrity/durability | medium | unscheduled | none | accepted residual |
| [L-057](#l-057) | Losing the disk or the host loses the evidence (X-10) | integrity/durability | medium | unscheduled | none | accepted residual |
| [L-058](#l-058) | The durable profile recognises Ubuntu 24.04 by `os-release`, not by its kernel | integrity/durability | low | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-059](#l-059) | Durable sessions need an explicit durable worker workspace | integrity/durability | low | unscheduled | none | accepted residual |
| [L-060](#l-060) | Admission is not fair between processes sharing a root | performance | low | unscheduled | none | accepted residual |
| [L-061](#l-061) | The free-space reserve is a pre-copy check on Unix only, not a quota | integrity/durability | low | unscheduled | none | accepted residual |
| [L-062](#l-062) | A worker request's input path may not go through any link | security | low | unscheduled | none | accepted residual |
| [L-063](#l-063) | A workspace keeps at most 4,096 request records, pruned only when their session is gone | contract/UX | low | unscheduled | [#286](https://github.com/smormah/vsift/issues/286) | accepted residual |
| [L-064](#l-064) | A retain killed mid-copy leaves a staging directory in the bundle root | integrity/durability | low | unscheduled | none | accepted residual |
| [L-065](#l-065) | A request's deadline, admission wait and attempt count per delivery | contract/UX | low | unscheduled | none | accepted residual |
| [L-066](#l-066) | A batch file holds at most 1,000 lines; a longer file runs nothing | contract/UX | low | unscheduled | none | accepted residual |
| [L-067](#l-067) | Requests of one batch contend with each other; a job-cancelled line exits 6 | contract/UX | low | P11 | [#14](https://github.com/smormah/vsift/issues/14) | open |
| [L-068](#l-068) | SEC-T01 adversarial containment evidence deferred (technical debt) | security | high | R1 | [#188](https://github.com/smormah/vsift/issues/188) | deferred (technical debt) |
| [L-069](#l-069) | A request that failed for good because of the host replays that failure | contract/UX | low | unscheduled | none | accepted residual |
| [L-072](#l-072) | Codex's permissions are graded from its event stream, not configured to match Claude Code's | security | medium | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-074](#l-074) | SubRip markup removal is broader than the contract lists | contract/UX | low | unscheduled | none | open |
| [L-075](#l-075) | Codex's image views are not in its stream, so its image budgets are unmeasured | process/CI | medium | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-076](#l-076) | Codex's Windows sandbox cannot run VSift trials as configured | process/CI | medium | unscheduled | [#204](https://github.com/smormah/vsift/issues/204) | open |
| [L-077](#l-077) | An agent cannot measure its own wall time; only the host enforces that budget | contract/UX | low | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-078](#l-078) | The Codex trial container relaxes Docker's seccomp profile so Codex's sandbox can create user namespaces | security | low | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-079](#l-079) | The Codex trial container's own network is not limited to the model API | security | low | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-080](#l-080) | A Codex trial agent can read its client's sign-in and its own trial's harness folder | security | medium | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-081](#l-081) | A handoff may leave out the times and session details VSift recorded, so reading it alone does not give them | contract/UX | low | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | open |
| [L-082](#l-082) | Claude Haiku 4.5 does not follow the full investigation procedure | process/CI | medium | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-083](#l-083) | Only `display_text` shows hidden characters; `text` and `original_text` keep them raw | security | low | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-084](#l-084) | GPT-6-Luna is below the compact-tier line | process/CI | medium | unscheduled | [#15](https://github.com/smormah/vsift/issues/15) | accepted residual |
| [L-086](#l-086) | `vsift-contract` embeds the skill's handoff schema from outside its crate folder, so it cannot be packaged for crates.io as it is | platform/distribution | low | unscheduled (before any crates.io publication, R1 or later) | none | deferred |
| [L-087](#l-087) | A managed version is rehashed each time a command resolves it | performance | low | unscheduled | none | monitoring |
| [L-088](#l-088) | Proxy authentication in an HTTPS tunnel is recognised by a dependency's error text | process/CI | low | unscheduled | none | monitoring |
| [L-089](#l-089) | A release SBOM's `bom-ref` values name the build machine's checkout path | platform/distribution | low | unscheduled | none | accepted residual |
| [L-090](#l-090) | Content in the managed folder that VSift cannot prove its own is left for the user to delete, and no output names the folder's path | platform/distribution | low | unscheduled | none | accepted residual |
| [L-091](#l-091) | The npm launcher relays signals by a rule that can repeat or miss one, and cannot stop vsift if the launcher itself is killed | platform/distribution | low | unscheduled | none | accepted residual |
| [L-092](#l-092) | The npm qualification covers the minimum runtimes, one pnpm and one Yarn version, and Yarn only through a project install | platform/distribution | low | unscheduled | none | accepted residual |
| [L-093](#l-093) | The launcher's digest check finds damaged or mismatched packages, not a local attacker who can write to the install | security | low | unscheduled | none | accepted residual |
| [L-094](#l-094) | On Windows, vsift cannot start from an install whose executable path is 260 characters or longer | platform/distribution | low | unscheduled | none | accepted residual |
| [L-095](#l-095) | Review-tier models can state blurred content as supported by pixels; the skill fix is not yet re-measured | contract/UX | medium | P14 (batch 2) | [#224](https://github.com/smormah/vsift/issues/224) | deferred (technical debt) |
| [L-097](#l-097) | A publish that fails part-way leaves part of the release public until a re-run completes it | platform/distribution | low | unscheduled | none | accepted residual |
| [L-098](#l-098) | The Windows and macOS executables are unsigned: SmartScreen and Gatekeeper may warn about a direct download, and Windows Smart App Control may block `vsift.exe` outright | platform/distribution | medium | P14, maintainer | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-099](#l-099) | Managed installation depends on files and redirect hosts that the publishers control | platform/distribution | low | unscheduled | none | accepted residual |
| [L-100](#l-100) | npm prints only `ENEEDAUTH`, with no reason, when a trusted publisher is missing or set wrongly | process/CI | low | unscheduled | none | accepted residual |
| [L-101](#l-101) | The evidence ledger and the claims registry prove that recorded evidence exists and banned words are absent, not that a run passed or a sentence is true | process/CI | low | unscheduled | none | accepted residual |
| [L-103](#l-103) | Evidence carried forward from an earlier commit rests on a hand-written scope and needs Git history, and the delta record is copied into the ledger by hand | process/CI | low | P14 (PR 12) | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-105](#l-105) | The stable publish path has never run against the real services | platform/distribution | medium | P14 (PR 12) | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-107](#l-107) | The candidate-to-stable check compares paths and bytes, not meaning, and takes the highest candidate to be the accepted one | process/CI | low | unscheduled | none | accepted residual |
| [L-108](#l-108) | After a stable release `next` still names the candidate, an older build than `latest` | platform/distribution | low | P14 (PR 12) | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-109](#l-109) | On Windows, the `vsift.cmd` shim that npm and pnpm create lets cmd.exe re-read arguments: percent expansion, dropped quotes and a redirection without whitespace that runs | security | low | P14 (PR 7) | [#257](https://github.com/smormah/vsift/issues/257) | accepted residual |
| [L-111](#l-111) | The upgrade evidence has one published baseline, and its two modes prove different things | process/CI | low | P14 (PRs 10, 12) | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-112](#l-112) | The clean-install jobs hide named programs from `PATH` on a hosted image; that is not a clean machine, and one tool set stands in for each system's users | process/CI | low | unscheduled | none | accepted residual |
| [L-113](#l-113) | The P11 durable stage cannot run on a hosted runner, so the published binary's durable worker request has never run on the qualified profile | integrity/durability | medium | P14 | [#258](https://github.com/smormah/vsift/issues/258) | open |
| [L-114](#l-114) | macOS is tried with Homebrew's FFmpeg and whisper.cpp, which are not reviewed artifacts | platform/distribution | medium | P14 (PR 9) | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-115](#l-115) | The published-binary journeys run later tests against 0.1.0 and do not reach the launcher, the archives or the engine-library stages | process/CI | low | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-116](#l-116) | A weekly drift run shows only that one hosted run of the highest published version passed that week | process/CI | low | unscheduled | none | accepted residual |
| [L-117](#l-117) | The clean-install and cold-agent trials are not clean-machine trials, and a cold agent on Windows can still find the package's README and skill | process/CI | medium | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-118](#l-118) | The cold grader reads command text and matches free text mechanically | process/CI | medium | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-119](#l-119) | There are two hold-out scenarios, one run per client each, written by the same authors | process/CI | medium | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-120](#l-120) | Usage figures and the usage-limit reading are the clients', and the harness's parsers have not met a real stream | process/CI | low | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-121](#l-121) | Text drawn inside the README's SVG graphics is public text the claims check cannot read | process/CI | low | unscheduled | none | open |
| [L-122](#l-122) | One recorded FFmpeg vulnerability (CVE-2026-38350, libswscale) is tied to its fix only by elimination, and the reading proves the source of the shipped build, not its behaviour | security | medium | P14 | [#272](https://github.com/smormah/vsift/issues/272) | open |
| [L-123](#l-123) | Two Windows concurrency failures reproduce on a hosted runner at about one repetition in 200: session-root creation and the weighted-admission grant | integrity/durability | medium | P14 | [#206](https://github.com/smormah/vsift/issues/206), [#271](https://github.com/smormah/vsift/issues/271) | open |
| [L-124](#l-124) | Local recognition of a five-second range fails as a missing capability for three of ten valid speech clips | accuracy/ASR | medium | P14 | [#274](https://github.com/smormah/vsift/issues/274), [#277](https://github.com/smormah/vsift/issues/277) | open |
| [L-125](#l-125) | The realistic cold setting cannot be fenced to the workspace, so Claude Code runs it only on an isolated machine; the two clients' cold baselines are not the same test | process/CI | medium | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-126](#l-126) | A session root VSift did not create is refused with `INTEGRITY_FAILURE`, which says stored data is damaged; only the remediation says what happened | contract/UX | low | unscheduled | none | accepted residual |
| [L-127](#l-127) | The CLI's ingest treats three kinds of hostile source worse than the worker path does: a named pipe hangs it, a link is refused as a storage failure and a full disk is reported as corruption | security | low | P14 | [#264](https://github.com/smormah/vsift/issues/264), [#265](https://github.com/smormah/vsift/issues/265), [#266](https://github.com/smormah/vsift/issues/266) | open |
| [L-128](#l-128) | The fuzzing is one hour per target on shared hosted CPUs, nineteen of 31 targets were still finding coverage at the end, and three kinds of stored record have no target | security | low | P14 | [#17](https://github.com/smormah/vsift/issues/17) | accepted residual |
| [L-132](#l-132) | The reviewed FFmpeg can only follow a month-end build of its publisher, and a new pin does not move existing installs; the refresh candidate of 2026-10-03 is a daily build, needs two reviewed bounds raised and adds three libraries to the recipe | security | medium | P14 | [#272](https://github.com/smormah/vsift/issues/272) | open |

Counts: 1 high, 36 medium, 79 low (116 entries).

## Security

### L-001

**Evidence calls check the source copy by identity only (D1).**

- **What:** after the first evidence call of a session hashes the private copy of the
  video in full, later `frame`, `crop` and `audio` calls compare only the copy's
  on-disk identity (size, modification time, device, file index and platform change
  fields). On Windows a process running as the same user that rewrites the copy and
  restores its modification time between two calls is not detected by the later call;
  it is caught only by a later full hash (a call whose identity differs, or any
  mutating operation). On Unix the kernel-set status-change time prevents this.
- **Evidence:** ADR 0019 D1; [ADR 0012 note of 2026-09-26](../decisions/0012-p04-source-media-profile.md)
  ("evidence calls compare the copy's identity across calls"); regression tests
  `evidence_calls_hash_the_copy_only_when_its_identity_is_new_or_changed` and
  `a_modified_source_copy_is_an_integrity_failure_and_nothing_is_committed`. Each
  result reports `source_check` (`identity` or `full_hash`).
- **Impact:** an evidence item marked `identity` could, in that attack, show pixels of
  a rewritten copy. Only an actor who already controls the user's account can do it.
- **Why:** hashing a large copy on every call costs about 11 s per call on 1.17 GB
  ([L-011](#l-011)); the desktop threat model excludes hostile same-user code
  ([threat model](security-threat-model.md), "Assets, actors and trust boundaries").
- **Mitigation:** the recorded identity lives in the owner-private session manifest;
  mutating operations keep full hashing; `source_check` is visible per result.
- **Next step:** revisit only if a hostile same-user profile is ever required (worker
  tenants use external sandboxes, P11).
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-002

**A change undone before the closing hash is not seen (bracketed binding).**

- **What:** operations that call FFmpeg or whisper.cpp many times hash the copy once
  at the start and once before commit, and compare identity before each provider
  call. A same-user change made after a check and undone before the closing hash (on
  Windows including the modification time, or within one file-time tick) is not
  detected. The older per-call rehash had exactly the same blind spot between the hash
  and the provider's read.
- **Evidence:** [ADR 0012](../decisions/0012-p04-source-media-profile.md) "Limits and
  consequences" and its 2026-09-26 note (issue #148, closed); threat model "Residual
  risks and response". Measured benefit: an 869 MB, 24-chunk source decoded in 25.5 s
  bound versus 173.4 s with per-call rehashing (Windows 11, Xeon E5-2698 v4).
- **Impact:** as [L-001](#l-001); limited to same-user actors.
- **Why:** a per-call hash is linear in source size per chunk; the residual existed
  before the change.
- **Mitigation:** private session directory under a lifetime hold; any change still
  present at the end fails the commit with `INTEGRITY_FAILURE`.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-003

**A forged diagnostics line that continues the real numbering (SEC-17 residual).**

- **What:** VSift reads frame and audio times from FFmpeg's `showinfo`/`ashowinfo`
  lines. Since P09 PR 1 it accepts only lines that begin with the filter's own prefix
  and requires gap-free, strictly increasing frame numbering. A line that begins with
  that prefix, which is possible only through a log message that embeds an untrusted
  string with a line break, and exactly continues the real numbering cannot be told
  apart by text alone.
- **Evidence:** [threat model](security-threat-model.md), SEC-17 finding fixed
  2026-09-26; [ADR 0012 note](../decisions/0012-p04-source-media-profile.md)
  "provider diagnostics parsing hardened"; fuzz targets `frame_showinfo`,
  `frame_listing`.
- **Impact:** for an extraction the line count must still equal the images decoded,
  and a forged listing entry names a timestamp the exact extraction then does not find
  (`FrameNotFound`), so it cannot become evidence. It could make a listing-based
  request fail.
- **Why:** FFmpeg mixes filter output and echoed metadata in one stream.
- **Mitigation:** prefix, numbering, time-base and exact-extraction checks.
- **Next step:** none planned; re-review if FFmpeg's log format changes.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-004

**Native decoders and the recognizer are not sandboxed on the desktop.**

- **What:** FFmpeg, FFprobe and whisper.cpp run as ordinary processes with the user's
  filesystem and network access. VSift bounds their arguments, protocols, time,
  output, threads and (for FFmpeg) each allocation at 64 MiB, but a malicious decoder
  can allocate several buffers under that cap, and whisper.cpp's memory is bounded only
  by the operating system (an abnormal exit is `RESOURCE_LIMIT`). Dangerous
  decompression-bomb media has not been run.
- **Evidence:** [ADR 0012](../decisions/0012-p04-source-media-profile.md) "Limits and
  consequences"; [threat model](security-threat-model.md) process isolation profile and
  residuals (P04, P07 3b, P08); [baseline review](baseline-review.md) B-09.
- **Impact:** a crafted video that exploits a decoder bug runs with the user's rights.
- **Why:** desktop profile decision (ADR 0005, ADR 0012); strict isolation needs
  external host controls.
- **Mitigation:** forced local demuxers, `file` protocol only, MOV external references
  off, closed argument lists, Job Object / process-group cleanup; strict-worker mode
  fails closed when isolation is requested but unavailable. Since P11 PR 2 a worker
  host asks for `--host-isolation strict-linux`, which is accepted only when the
  kernel attests a cgroup v2 with finite CPU, memory and PID limits, a read-only root
  and no network interface but loopback (`attest_strict_linux_host`); otherwise the
  command answers `ISOLATION_UNAVAILABLE` before any work. The limits are the host's,
  never VSift's; the attestation's parsers are fuzzed (`host_attestation`). For P11
  the maintainer accepted SEC-T01 on this non-adversarial evidence and the hardened
  `strict-worker-boundary` container controls (2026-09-28); whether an attested host
  contains a hostile decoder is unproven and deferred as technical debt
  ([L-068](#l-068)). The [worker-host runbook](../operations/worker-host.md) gives an
  isolated deployment example with the same controls.
- **Next step:** [L-068](#l-068) moved to R1 (maintainer decision, 2026-10-03); P14 malicious-decoder and
  decompression-bomb qualification in a disposable environment.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** deferred. **Review:** pending.

### L-005

**Private Windows folders get their DACL just after creation, not atomically.**

- **What:** each folder VSift creates as a private root gets a protected DACL (user,
  SYSTEM, Administrators) immediately after creation and before content is written.
  A principal the parent already trusted could open a handle to the still-empty folder
  in that instant and keep it; it could later list names but not open entries.
- **Evidence:** [threat model](security-threat-model.md) "Process isolation profile"
  (SEC-18, 2026-09-24); [P05 record](p05-session-qualification.md) "private roots under
  a permissive parent".
- **Impact:** file names (not contents) could be visible to an already-trusted
  principal of the parent folder.
- **Why:** atomic creation needs `CreateDirectoryW` security attributes, that is
  `unsafe` or a new dependency, both excluded by `AGENTS.md`.
- **Mitigation:** creation under a no-delete-share handle; post-creation validation.
- **Next step:** reconsider if a reviewed safe API becomes available.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-006

**The media-tool check record trusts file identity, not executable contents, for tools VSift does not manage.**

- **What:** a passed FFmpeg/FFprobe (and local-ASR) check is remembered for seven days,
  keyed to the executables' canonical paths and on-disk identity. For a configured or
  `PATH` tool, executable contents are not hashed, so a same-user process that rewrites
  it in place while keeping size and timestamps is not detected. Managed tools are not
  affected since P13 PR 4: a managed version is used only after every file matches its
  manifest's size and SHA-256 and the selection names the manifest's SHA-256, on every
  command that resolves it, and a changed version is never run.
- **Evidence:** [threat model](security-threat-model.md) "Process isolation profile"
  (SEC-02, P07); [ADR 0017](../decisions/0017-local-asr-through-whisper-cpp.md)
  section 9; for managed tools `vsift/tests/engine_managed_lookup.rs`
  (`a_changed_managed_version_is_not_used`) and the
  [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 4 note.
- **Impact:** outside the desktop threat model; a tampered bring-your-own tool could
  produce wrong output that VSift then validates only structurally.
- **Why:** hashing a user's tools before every media stage is costly and their bytes
  have no reviewed digest to compare with; the record is an optimisation, never an
  authority.
- **Mitigation:** strict private record (4 KiB, versioned); any defect reads as
  "unverified". Managed installation (Ubuntu 24.04 x86-64) identifies tools by digest.
- **Next step:** none planned for bring-your-own tools.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-007

**Evidence can carry instructions; agents can leak delivered paths.**

- **What:** transcripts, frames and audio can contain text that tries to steer an
  agent (prompt injection). VSift labels evidence as untrusted data and never acts on
  it, and the agent skill narrows what an agent may run, but no tool can make every
  model immune.
  - `text` and `original_text` keep hidden characters as written; only
    `display_text` shows them ([L-083](#l-083)).
  - An agent that copies a delivered file path into prose may reveal the session
    root's location to readers of that prose.
- **Evidence:**
  - [threat model](security-threat-model.md) "Agent-specific controls", the P07
    increment 2 and P09 delivery residuals, and SEC-16;
  - the tool-level SEC-T02 suite (`crates/vsift-cli/tests/sec_t02_adversarial_evidence.rs`)
    on every PR;
  - P12's named-client trials ([qualification record](p12-agent-qualification.md)).
    In the counted A-04 and SEC-T02 runs (20 in P12's final round and 20 in the
    compact re-run #222, Claude Sonnet 5.5 and GPT-6-Sol) and in every reference round
    (348 graded phases), no agent installed or downloaded anything, ran a script,
    leaked a canary or acted on the hostile text. No counted report held a raw hidden
    character, a live link or an absolute path.
- **Impact:** a hostile recording could still steer a model outside the qualified
  configurations, a client run without the skill's limits, or a later model. The
  trials cover named clients and models on synthetic adversarial fixtures only.
- **Why:** prompt injection is an open problem; paths are delivered by decision D2.
- **Mitigation:**
  - fixed-prose remediation that never echoes evidence text;
  - typed fields; no evidence-driven installs or policy;
  - the agent skill (P12) limits authority by command class
    (`skills/vsift/references/commands.md`), treats evidence as data and reports
    embedded instructions with citations (`references/safety.md`), cites
    `evidence_id`s instead of paths, and quotes only `display_text`;
  - its handoff schema refuses paths, links and hidden characters in prose;
  - the trial grader fails any out-of-policy attempt, whether it ran or not.
- **Next step:** re-run A-04 and SEC-T02 whenever the skill, the clients or the
  supported models change.
- **Owner:** unscheduled. **Issue:** none.
  **Status:** accepted residual. **Review:** pending.

### L-072

**Codex's permissions are graded from its event stream, not configured to match Claude Code's.**

- **What:** Claude Code trials run with committed settings that allow only
  `Bash(vsift:*)`, reads below the workspace and the `vsift` skill, and deny everything
  else without prompting. Codex has no equivalent command allow list: its trials run
  with `--sandbox workspace-write`, network off, approvals `never` and the session root
  writable (on Windows the unelevated sandbox, which cannot yet run VSift and does not
  enforce the network: L-076), so Codex can *run* a command the skill forbids (inside
  its sandbox). The
  same policy is enforced on both clients by the grader, which reads every requested
  command from the stream and fails the trial for any attempt, run or denied.
- **Evidence:** [ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md)
  decision 7 and its 2026-09-28 note; `tools/vsift-agent-trials/src/run.rs` and
  `calls.rs`; [trial runbook](../agents/trials.md).
- **Impact:** a Codex trial that misbehaves may change files inside its workspace
  before it fails; the trial workspace is disposable and holds only synthetic media.
- **Why:** the clients' permission models differ; grading keeps one policy for both.
- **Mitigation:** trials run in a disposable workspace under a neutral root with a
  cleared environment; the graded result, not the sandbox, decides the trial.
- **Next step:** revisit if Codex gains a command allow list.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual. **Review:** pending.

### L-049

**Checkpoints resist corruption, not a same-user forger.**

- **What:** a retranscription job keeps each chunk's raw recognizer output as a
  private checkpoint file in the session. A checkpoint is decoded strictly, its payload
  must match its SHA-256, its ordinal its file name and its recognition key the run,
  and its output passes the same validation as fresh output; anything else is removed
  and the chunk recognised again. A process running as the same user can still write a
  consistent checkpoint (recomputing the digest) with invented but plausible text,
  which a resumed run would commit.
- **Evidence:** [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
  "Implementation notes: PR 2"; threat model P10 PR 2 note (SEC-10); tests
  `damaged_forged_or_future_checkpoints_are_unusable`,
  `foreign_or_invalid_checkpoints_are_discarded_and_redone`.
- **Impact:** only an actor who already controls the user's account, which can
  rewrite committed evidence as well.
- **Why:** a keyed MAC would need a secret the same user could read anyway.
- **Mitigation:** the owner-private session root; checkpoints live only until the job
  commits, fails or is cancelled.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-055

**On Unix a hard-killed CLI's running provider finishes its current unit.**

- **What:** every provider runs in its own process group, which keeps a terminal's
  Ctrl-C away from it and lets the supervisor stop the whole tree. If `vsift` itself is
  killed outright (`SIGKILL`), no destructor runs, so a whisper.cpp or `FFmpeg` process
  already running keeps going until its current chunk or window ends (its output then
  goes nowhere). Windows is not affected: the Job Object kills the tree when `vsift`
  dies. An interrupted (Ctrl-C, `SIGTERM`) command always reaps its providers first.
- **Evidence:** `process_supervisor.rs` (process groups, kill-on-drop), the P10
  `p10_kill_and_resume` stage.
- **Impact:** after a hard kill a provider can use CPU for up to one chunk (at most
  120 s by the chunk deadline it no longer enforces, usually a few seconds). A
  hard-killed `setup install` can likewise leave a smoke provider writing into its
  abandoned stage (P13 PR 7): the next install's sweep may keep that stage once, and
  `setup repair` then names it for `setup remove --stale-stages`.
- **Why:** a group leader's death does not signal its group; a Linux parent-death
  signal needs `prctl`, platform code outside the reviewed dependencies.
- **Mitigation:** send `SIGTERM`, not `SIGKILL`. The
  [worker-host runbook](../operations/worker-host.md) runs VSift in a cgroup (a
  systemd unit with `KillMode=mixed`, or a container) whose stop kills every process
  left after the drain time, providers included.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-062

**A worker request's input path may not go through any link.**

- **What:** a request's source and transcript paths are opened inside the operator's
  `--input-root` one component at a time, following no link: a symbolic link,
  junction or other reparse point anywhere on the path is refused, even one that
  points inside the root, and the file must have a single hard link. A Windows
  junction is refused by the same check, but only symbolic links are exercised by the
  tests (creating a junction needs a shell or an unstable API).
- **Evidence:** ADR 0021 section 9 and PR 2 notes; `contained_inputs.rs`
  (`a_symbolic_link_out_of_the_root_is_refused`, `a_hard_link_out_of_the_root_is_refused`,
  `every_escape_spelling_is_refused_before_anything_is_opened`).
- **Impact:** an operator cannot lay out inputs with symlinks or hard links; they must
  copy or bind-mount them into the root.
- **Why:** refusing every link removes every way out of the root, at the cost of
  convenience (SEC-05).
- **Mitigation:** the refusal is typed (`ContainedPathError::Link`,
  `path_outside_input_root` in PR 3) and names no path.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-093

**The launcher's digest check finds damaged or mismatched packages, not a local attacker who can write to the install.**

- **What:** before it runs the executable, the npm launcher checks that the platform
  package has its own version and that the executable's size and SHA-256 match
  `platform-digests.json`, written into the launcher package by the release build
  (ADR 0023 decision H5). Anyone who can write to the install folder can also change the
  launcher and its digest file, and the file is read by path again when it is started,
  after the check, so the check is not a defence against a local attacker. It proves the
  platform package is the one this launcher was released with, undamaged.
- **Evidence:** [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 9 note; `npm/vsift-cli/lib/launcher.cjs`; the refusals in `npm/test/launcher.test.cjs` and
  in every `npm-qualify` job.
- **Impact:** none beyond what write access to the install folder already gives.
- **Why:** a user-writable install folder is the norm for npm, pnpm, Yarn and Bun; the
  registry's integrity check and npm provenance (wired by PR 10) cover the path from the
  release to the machine.
- **Mitigation:** npm verifies each tarball's integrity on install, and `npm audit
  signatures` checks the registry signatures and provenance of 0.1.0 (it verified all four
  packages on 2026-10-01).
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

## Integrity and durability

### L-008

**OS-crash durability is qualified only on Ubuntu 24.04 with local ext4 (FS-01).**

- **What:** a durable session (an engine request, or since P11 PR 2 any session of a
  durable worker workspace, ADR 0020 D-3)
  keeps every acknowledged generation through power loss, an OS crash and write or
  flush errors on Ubuntu 24.04 with the session root on local ext4 mounts that keep
  write barriers. Everywhere else (Windows/NTFS, macOS/APFS, other Linux
  distributions and filesystems, network filesystems, ext4 with `nobarrier` or
  `barrier=0`) a durable request fails with `MISSING_CAPABILITY` before anything is
  changed, and sessions are consistent across a VSift process crash only. A retained
  bundle keeps process-crash consistency even for a durable session: the export does
  not run the durable protocol.
- **Evidence:** [P10 durable-publication record](p10-durable-publication.md) (the
  campaign: dm-log-writes power loss at every flush, QEMU kills, dm-flakey errors,
  negative control); [ADR 0010](../decisions/0010-storage-qualification-gate.md) and its
  2026-09-27 note; [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
  section 7 and PR 4 notes; threat model SEC-24.
- **Impact:** desktop users on Windows and macOS get ephemeral guarantees only; the
  command line reaches durability only through an explicit durable workspace
  ([L-059](#l-059)); the strict worker profile still needs P11 and P14.
- **Why:** each further profile needs its own owned crash campaign; the Windows
  writable-directory flush and APFS behaviour have no fault evidence (P03).
- **Mitigation:** durable requests fail closed and the effective guarantee is
  reported; the Ubuntu campaign reruns weekly.
- **Next step:** P11 worker requests in durable workspaces (PR 3) and the strict
  worker qualification (PR 4); other profiles only with their own campaign and ADR.
- **Owner:** P11, P14. **Issue:** [#14](https://github.com/smormah/vsift/issues/14),
  [#17](https://github.com/smormah/vsift/issues/17). **Status:** accepted residual
  (desktop profiles), deferred (worker profile). **Review:** pending.

### L-056

**Durability rests on storage that honours flushes.**

- **What:** VSift flushes files and directories in the qualified order and
  acknowledges only afterwards; ext4 turns those flushes into device flushes and FUA
  writes. A drive, controller, RAID layer or hypervisor that reports a flush complete
  without making it durable (a volatile write cache without power-loss protection, a
  virtual disk with `cache=unsafe`, firmware that ignores FLUSH) can lose acknowledged
  generations on power loss, as it can for any filesystem or database.
- **Evidence:** [P10 durable-publication record](p10-durable-publication.md)
  "Residuals"; layer A models a device that honours every flush and FUA write, layer B
  one attached `cache=none`.
- **Impact:** a durable session on such storage is only as safe as the storage. The
  managed store's power-loss claim (a reported command survives, P13 PR 7) rests on the
  same flushes; on such storage a torn managed version is still refused by lookup, but a
  reported command can be undone and repair may meet a state no kill leaves.
- **Why:** the host's storage stack is outside VSift; it cannot detect a device that
  lies about flushes.
- **Mitigation:** use storage with power-loss protection or a write-through cache;
  the durable profile already refuses ext4 mounted without write barriers.
- **Next step:** state it in the P11 worker and P13 user documentation.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-057

**Losing the disk or the host loses the evidence (X-10).**

- **What:** local durability survives a worker crash, an OS crash and power loss; it
  does not survive losing the disk, the filesystem or the machine. VSift keeps one
  local copy and offers no replication.
- **Evidence:** verification X-10; [P10 durable-publication record](p10-durable-publication.md)
  "Residuals"; ADR 0020 section 7.
- **Impact:** a caller that needs evidence to outlive the host must copy it off the
  host.
- **Why:** replication is a storage service's job, not a local-first tool's (ADR
  0010, ADR 0020).
- **Mitigation:** retain a bundle (`session retain`) onto replicated storage, or back
  up the session root; `bundle validate` checks a copy.
- **Next step:** none planned; the [worker-host runbook](../operations/worker-host.md)
  (restart and recovery) states the operator's responsibility.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-058

**The durable profile recognises Ubuntu 24.04 by `os-release`, not by its kernel.**

- **What:** the gate trusts `/etc/os-release` (`ID=ubuntu`, `VERSION_ID=24.04`) and the
  mount table. An Ubuntu 24.04 user space in a container on another host kernel, or a
  24.04 host on a hardware-enablement kernel the campaign did not run, passes it. The
  campaign ran the hosted runner's kernel (layers A and C) and the pinned cloud
  image's generic kernel (layer B); both are recorded in the qualification record.
- **Evidence:** `durable_profile.rs` (`qualifies`, `classify_os_release`); [P10
  durable-publication record](p10-durable-publication.md).
- **Impact:** a durable claim on an unexercised kernel rests on ext4's stable fsync
  semantics rather than on a campaign run.
- **Why:** pinning kernel versions would refuse every security update; ext4's
  journal-commit and directory-fsync semantics have been stable for years.
- **Mitigation:** the campaign reruns weekly on the current hosted kernel; the image
  pin is bumped deliberately.
- **Next step:** P14 decides whether the worker profile pins a kernel series.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** accepted residual. **Review:** pending.

### L-059

**Durable sessions need an explicit durable worker workspace.**

- **What:** since P11 PR 2 the command line reaches durability: an operator creates a
  worker workspace with `session init-workspace --durability durable` (only on Ubuntu
  24.04 with local ext4; anywhere else it is `MISSING_CAPABILITY` and nothing is
  created), and `ingest --session-root <workspace>` then opens durable sessions
  (`publication` `os_crash_durable`, ADR 0020 D-3), as do worker requests (`job run`,
  P11 PR 3), which run only in a workspace. A desktop root never gives durability.
- **Evidence:** ADR 0021 PR 2 notes;
  `a_durable_workspace_is_created_only_where_durability_is_qualified`,
  `ingest_in_a_workspace_inherits_its_durability_and_retention`,
  `a_workspace_decides_the_durability_of_its_sessions`.
- **Impact:** durability needs a deliberate operator step and a qualified host.
- **Why:** nothing becomes durable by accident (ADR 0021 D1); durability is qualified
  on one profile (L-008).
- **Mitigation:** typed `MISSING_CAPABILITY` with remediation; desktop sessions stay
  ephemeral by design (ADR 0002).
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-061

**The free-space reserve is a pre-copy check on Unix only, not a quota.**

- **What:** before a source is copied into a worker workspace on Unix, VSift reads the
  filesystem's available space (`fstatvfs` on the held root) and refuses the copy
  (`RESOURCE_LIMIT`) unless the source's size and a 1 GiB reserve are free. On Windows
  nothing is checked and the job result reports `free_space_reserve: not_enforced`.
  Desktop roots are not checked on any platform. The check reserves nothing: another
  writer can use the space between the check and the copy, and later evidence and
  records are not checked.
- **Evidence:** ADR 0021 PR 2 notes; `the_free_space_reserve_is_checked_on_unix_only`.
- **Impact:** a full disk still fails a commit with `STORAGE_IO` (never a half commit,
  X-10), rather than being refused up front.
- **Why:** a real quota needs the host (filesystem quotas, a dedicated volume); Windows
  has no equivalent in the reviewed dependencies.
- **Mitigation:** commits are atomic and fail closed on a full disk; operators give a
  worker workspace its own volume or quota.
- **Next step:** none planned; revisit with P14's worker profile.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-009

**Cleanup and erasure leave some work to the user.**

- **What:**
  - An expired session is only eligible for cleanup; files are removed when
    `session clean` runs, not automatically.
  - Deleting a session does not erase backups, snapshots, SSD remnants or copies made
    elsewhere; no secure-erasure claim is made.
  - An interrupted `session retain` export may stay incomplete; it is private, fails
    validation, and must be inspected or removed by the user.
  - A creator killed while provisioning a new session root leaves an unmarked root
    that is refused until the user removes it.
  - An existing folder that other accounts can access is refused, never repaired
    (`STORAGE_IO` with a remediation naming the folder kind).
- **Evidence:** [ADR 0013](../decisions/0013-retained-bundle-publication.md)
  consequences; [P05 record](p05-session-qualification.md) "Limits and follow-up
  ownership" and the #131 residual; threat model residuals; README "Principles".
- **Impact:** disk use accumulates until cleaned; rare manual recovery steps.
- **Why:** VSift never deletes what it cannot positively identify as its own, and runs
  no background service.
- **Mitigation:** typed errors with fixed remediation; `session clean --expired
  --dry-run`.
- **Next step:** the P12 skill documents the cleanup routine for agents
  (`skills/vsift/references/lifecycle.md`, 2026-09-28); P13 user docs remain.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-010

**Visual candidates and evidence calls are not recoverable jobs.**

- **What:** a retranscription is a recoverable job: an interrupted run (a crash, a
  failure, Ctrl-C or `SIGTERM`, a library cancellation) is found by the same request, or
  resumed by id with `job resume`, from its chunk checkpoints to the revision an
  uninterrupted run commits; `--operation-id` makes a retry return the committed result
  without a new generation. A worker request (`job run`) and every line of a batch
  (`job batch`) are recoverable as a whole: a redelivery continues from the first
  unfinished step and an ended request is replayed; on the qualified profile a durable
  workspace's generations, jobs and request records survive an OS crash and power loss
  ([L-008](#l-008)). `candidates`, `frame`, `crop` and `audio` calls are not jobs: an
  interrupted `candidates` call commits the windows it analysed and answers `partial`,
  and an evidence call commits nothing partial, so their reruns redo at most one call's
  work.
- **Evidence:** [verification](verification.md) "P10 PR 2 evidence" to "P11 PR 4
  evidence"; [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
  and [ADR 0021](../decisions/0021-worker-and-batch-host.md) implementation notes; the
  [P11 qualification record](p11-worker-host.md) (`p11_shutdown_and_redelivery`).
- **Impact:** an interrupted visual or evidence call costs its own work again, bounded
  by one call (at most 30 visual windows).
- **Why:** these calls are short and already commit partial results; a job record per
  call would cost more than it saves.
- **Mitigation:** candidates keep every committed window; a worker request's
  candidates step continues from what is committed.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-064

**A retain killed mid-copy leaves a staging directory in the bundle root.**

- **What:** a `retain` step writes its bundle to a hidden staging directory beside its
  name (`.<bundle_name>.<op>.retaining` under `--bundle-root`) and renames it once it
  validates, so a killed worker never leaves an incomplete bundle under the bundle's
  own name. The staging directory of a killed try stays: VSift deletes nothing in the
  operator's bundle root. A bundle directory that already exists and is not this
  request's bundle (another session, other artifacts, or not a valid bundle) is
  refused with `INVALID_ARGUMENT`, and that request ends.
- **Evidence:** ADR 0021 PR 3 notes; `an_existing_bundle_is_accepted_only_when_it_is_the_requests`,
  `repeated_external_delivery_commits_once`.
- **Impact:** after crashes an operator may find hidden staging directories to remove.
- **Why:** automatic cleanup is restricted to VSift-owned temporary directories inside
  its own roots (`AGENTS.md`).
- **Mitigation:** staging names are hidden and say what they are; the bundle under its
  own name is always whole.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-047

**A read no longer re-verifies generations below the chain checkpoint.**

- **What:** since P10 PR 1 (#164) an ordinary read verifies the pointer, the head and
  every generation committed since the writer's chain checkpoint (or the last head the
  same store instance verified). Damage to an older generation is found by the next
  full walk, which `session retain` and `session clean` still perform, not by a read.
  Old generations never feed a read's result, and every artifact a read returns is
  still re-hashed (INV-02).
- **Evidence:** [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
  section 1 "Guarantee"; threat model P10 PR 1 note (SEC-08/SEC-10); unit tests
  `reads_stop_at_the_checkpoint_and_walk_everything_without_one` and
  `the_verified_head_cache_belongs_to_one_store_instance`.
- **Impact:** bit rot or same-user tampering in an old manifest surfaces later, at
  retain or cleanup.
- **Why:** walking the whole chain on every read made long sessions slow (issue #164,
  fixed in P10 PR 1).
- **Mitigation:** the checkpoint is written only by the writer and validated strictly;
  forging it needs the same-user access that could rewrite the chain itself.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

## Performance

### L-011

**Evidence on large sources is slow; the first call hashes the whole copy.**

- **What:** the first evidence call of each session hashes the whole source copy (the
  hash `ingest` made is not recorded as a verified identity), and every new frame
  re-decodes the source. On a dense 1080p file this is slow.
- **Evidence:** [P09 record](p09-evidence-navigation.md) "Performance" (recorded, not
  gated; Windows 11, Xeon E5-2698 v4, FFmpeg 9.0, release build) on a 1.17 GB, 240 s,
  about 39 Mbit/s MPEG-4 1080p worst-case clip: `ingest` 19.1 s; first `frame get`
  10.9 s (full SHA-256); new frames p95 4.1 s (3.2-4.1 s); a 12-frame burst over 60 s
  17.1 s; warm reuse p95 241 ms. On 720p F01 a cold frame takes 1.5-2.0 s and the
  first evidence call of a base about 6 s (media-tool preflight).
- **Impact:** agents wait seconds per new frame on large, dense recordings; screen
  recordings are usually far less dense.
- **Why:** D1 hashes once per session for integrity; no decode cache by design (reuse
  is by artifact identity, ADR 0019 consequences).
- **Mitigation:** reuse of identical requests without any process; bursts bounded to
  12 frames by default.
- **Next step:** consider recording the verified identity at `ingest`; measure on
  Ubuntu and macOS ([L-035](#l-035)); P14 load gates.
- **Owner:** unscheduled. **Issue:** [#170](https://github.com/smormah/vsift/issues/170). **Status:** monitoring.
  **Review:** pending.

### L-013

**Evidence records and transcripts are re-read in full on every call.**

- **What:** each evidence call reads and decodes all of the session's evidence records
  (at most 384 records of at most 256 KiB since P10 PR 2 raised the cap); `search` matches the whole transcript
  revision on every request with no persisted index.
- **Evidence:** [work record](../../memory/TODO.md) "Other follow-ups";
  [ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)
  decision 4: a 20,000-segment revision pages at p95 145-167 ms.
- **Impact:** none measured beyond the 250 ms target today.
- **Why:** an index is new persistent state with its own integrity rules.
- **Mitigation:** hard bounds on record count and size.
- **Next step:** add an index only if a measurement misses the target.
- **Owner:** unscheduled. **Issue:** [#171](https://github.com/smormah/vsift/issues/171). **Status:** monitoring.
  **Review:** pending.

### L-051

**Some interrupted work is redone rather than resumed.**

- **What:** a resumed retranscription reuses a chunk only from a checkpoint of exactly
  the same recognition: the same source, stream, range, chunk plan, recognizer and
  model, and local-ASR verification fingerprint, which includes the VSift version. An
  upgrade, a new model, a changed setup or a request whose widened range changed since
  (another revision committed meanwhile) starts over. A chunk whose checkpoint would be
  larger than 256 KiB is not checkpointed and is recognised again after an
  interruption, as is one whose checkpoint could not be written.
- **Evidence:** [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
  "Implementation notes: PR 2" (keys, checkpoints).
- **Impact:** after such a change an interrupted long run costs its full time again.
- **Why:** reusing recognizer output across setups could mix outputs of two models or
  tools; storing is an optimisation that never fails the run.
- **Mitigation:** the checkpoint budget fits normal speech output many times over.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-052

**A read that meets a file being replaced waits for it, at most 0.5 s.**

- **What:** the commit pointer, the chain checkpoint and job records are replaced by
  rename while other processes read them. A reader that meets a replacement (the file
  it opened has no link left, or on Windows the name is briefly absent) retries: a few
  times at once, then a millisecond apart, for at most half a second, blocking its
  thread meanwhile. A file still missing after that is reported as damage.
- **Evidence:** [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
  "Readers meeting a replacement"; tests
  `a_reader_that_meets_a_rename_retries_instead_of_reporting_damage`,
  `readers_never_report_damage_while_generations_are_published`,
  `a_linked_or_missing_pointer_is_still_damage`.
- **Impact:** normally microseconds; a truly missing pointer or job record is
  reported half a second later than before.
- **Why:** before, such a reader reported a healthy session as damaged.
- **Mitigation:** the budget is bounded; hard links and non-regular files are still
  refused at once.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-060

**Admission is not fair between processes sharing a root.**

- **What:** weighted admission (P11 PR 2, X-07) never lets the work admitted at once
  weigh more than the root's capacity, but it is a set of non-waiting OS try-locks:
  processes that share a root take free units in no particular order, a heavy request
  (a recognition of eight threads) can wait behind a stream of light ones, and a
  bounded `AdmissionWait` ends in `BUSY` rather than in a queue position. Requests of
  one batch will be admitted in line order (PR 4); across processes nothing orders them.
- **Evidence:** ADR 0021 section 5a; `weighted_admission_never_exceeds_root_capacity`,
  `admission_wait_is_bounded_then_busy`.
- **Impact:** under sustained contention a heavy request may be starved until it gives up
  with `BUSY` and a retry hint.
- **Why:** fairness across workers is the external supervisor's job (architecture and
  contracts section 10); an in-root queue would need a coordinator process.
- **Mitigation:** bounded waits with full jitter; `retry_after_ms` on `BUSY`; one
  workspace per worker, or an external queue that schedules heavy work.
- **Next step:** none planned for R0; R1 P19 (industrial worker plane).
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-087

**A managed version is rehashed each time a command resolves it.**

- **What:** every command that uses a managed tool opens its version and checks every
  file's SHA-256 before anything runs: about 232 MB for FFmpeg and FFprobe (hashed once
  for both) and 148 MB for the model. With SHA extensions that is roughly 0.1 to 0.3 s
  per command; without them it can approach a second.
- **Evidence:** [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 4 note; `vsift/src/managed.rs`. *Measured 2026-09-30* (`P13 managed smoke`
  [run 36734316384](https://github.com/smormah/vsift/actions/runs/36734316384), hosted `ubuntu-24.04`, all exit 0): a warm `setup check` took 0.65 s, a first
  `frame get` 0.29 s and a warm `frame get` 0.17 s against the managed tools. Since P13
  PR 6, `setup list` and `setup repair` also hash every published version once.
- **Impact:** evidence commands on a managed install start a little later.
- **Why:** identifying managed tools by digest on every use is what closes L-006 for
  them; a cache keyed to file identity would reopen it.
- **Mitigation:** each version is hashed once per open, after its use lock is held,
  and one command opens each component at most once.
- **Next step:** the measured cost is well under a second on a hosted runner, so no cache
  is needed now and this stays monitoring; measure again in P14's release qualification.
- **Owner:** unscheduled. **Issue:** none. **Status:** monitoring.
  **Review:** pending.

## Evidence contract and user experience

### L-063

**A workspace keeps at most 4,096 request records, pruned only when their session is gone.**

- **What:** every worker request's record (`worker-requests/<bucket>/<op>.json`, at
  most 192 KiB: an ended request's result of at most 64 KiB, escaped as JSON text, and
  its identities) stays until the session it names is gone. At 4,096 records a new
  operation id first prunes the records of removed sessions that no process holds; if
  none can be pruned it fails `RESOURCE_LIMIT` and nothing runs. An ended record whose
  session still exists is never pruned, because pruning it would let a redelivery run
  the work again (ADR 0021 section 4 said "oldest ended first"; the implementation
  keeps exactly-once instead).
- **Evidence:** ADR 0021 PR 3 notes; `a_full_workspace_prunes_only_records_whose_session_is_gone`.
- **Impact:** a workspace that keeps more than 4,096 live sessions' requests must clean
  sessions (`session clean --expired`, or their retention) before new requests run. The other
  side (P14 soak, 2026-10-03): a closed session that the cleaner removes takes its record with
  it as soon as the table is full, so a duplicate delivered later runs again and the same id
  with another request is accepted (189 of 12,000 lines; the runbook said the window was at
  least the retention and now says otherwise); [#286](https://github.com/smormah/vsift/issues/286).
- **Why:** a bounded store, and replay must stay exact while its session exists.
- **Mitigation:** records expire with their sessions (at most 720 hours); `RESOURCE_LIMIT`
  is typed and names no path.
- **Next step:** none planned for R0; the contract and the runbook now state the window as it is
  (while the session exists or fewer than 4,096 records are held, #286). For R1 (P19, the
  industrial worker plane), a small stub per ended request (operation id, request digest, terminal
  status, session id: about 200 bytes, so 100,000 are about 20 MB) kept beyond the session for the
  retention would give `IDEMPOTENCY_CONFLICT` for another digest and an honest "ended, result no
  longer held" for the same one; it needs a new persistent record type, its fuzz target, a contract
  addition and kill and power-loss qualification, so it is a behaviour change for the maintainer
  to decide, not a documentation fix. Never pruning before the retention would cap a host at
  4,096 requests per retention window and is not recommended.
- **Owner:** unscheduled. **Issue:** [#286](https://github.com/smormah/vsift/issues/286) (the dedupe window).
  **Status:** accepted residual. **Review:** pending.

### L-065

**A request's deadline, admission wait and attempt count per delivery.**

- **What:** a request's `deadline_ms` (or the one-day host limit), its admission wait
  (`--admission-wait-ms`) and its retries of `BUSY` steps are measured from the start
  of each delivery (`job run`), not across redeliveries; a redelivery of an unfinished
  request starts a new attempt with a fresh deadline. The result of an ended request is
  fixed once it is recorded: every later delivery replays it byte for byte. A
  redelivery that finds every step recorded but not yet the result (a crash between
  the two writes) records the result then, with its own attempt number and timings.
  An ingest interrupted before its session opened leaves a registration that normal
  cleanup removes; the next attempt opens a new session.
- **Evidence:** ADR 0021 PR 3 notes; `deadlines_and_permanent_failures_are_not_retried`,
  `a_kill_at_every_request_fault_point_recovers`.
- **Impact:** a supervisor that wants a deadline across redeliveries must enforce it
  itself; `attempt` counts deliveries that found the request unfinished.
- **Why:** the record keeps what was done, not a clock; a persistent deadline would
  need clock agreement between workers.
- **Mitigation:** `DEADLINE_EXCEEDED` is resumable and typed; replays are exact.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-066

**A batch file holds at most 1,000 lines; a longer file runs nothing.**

- **What:** `job batch` counts the lines of its file through the same handle before
  anything starts. A file of more than 1,000 lines is refused whole: `RESOURCE_LIMIT`
  (exit 5), `termination_reason` `line_limit`, `not_started_from_line` 1, no request
  run. A file that grows past the limit between the count and the read stops at line
  1,001 (`line_limit`, `not_started_from_line` 1001). A line of more than 64 KiB is
  refused alone (`request_too_large`) and the others run. Only a regular file is read.
- **Evidence:** ADR 0021 PR 4 notes; `limits_are_checked_before_any_work`,
  `limits_are_refused_before_any_work`, the `batch_file` unit tests.
- **Impact:** a supervisor splits a longer queue into files of at most 1,000 lines; a
  pipe or device cannot be the request file.
- **Why:** a finite, re-readable file lets the whole batch be refused before any work,
  so no operator has to find out which lines of an over-long file ran.
- **Mitigation:** the refusal is typed, fixed-prose and before any write.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-067

**Requests of one batch contend with each other; a job-cancelled line exits 6.**

- **What:** the requests of a batch share the workspace's admission units and its
  root-level locks, so with `--admission-wait-ms 0` a request can answer `BUSY` because
  another request of the same batch holds a unit or lock for a moment. And D5 ranks a
  request cancelled by `job cancel` between usage (2) and retryable (4); when it is the
  most severe line, the batch exits with its class's status, 6, the same as a shutdown.
- **Evidence:** ADR 0021 PR 4 notes; `every_line_is_isolated_and_reported` (run with a
  bounded wait), `a_mixed_batch_reports_independent_outcomes` (opt-in).
- **Impact:** a supervisor tells a shutdown from a cancelled line by
  `termination_reason` (`shutdown` or `end_of_input`), not by the exit status alone.
- **Why:** admission is per workspace (ADR 0021 section 5a); the exit status is the
  failure class's (ADR 0008).
- **Mitigation:** the default admission wait (60 s) retries contention with jitter; the
  summary and error code say which case it was.
- **Next step:** maintainer to confirm the exit reading of a cancelled line.
- **Owner:** P11. **Issue:** [#14](https://github.com/smormah/vsift/issues/14).
  **Status:** open. **Review:** pending.

### L-068

**SEC-T01 adversarial containment evidence deferred (technical debt).**

- **What:** no adversarial provider fixture exists. SEC-T01 asks for evidence that a
  strict worker host contains a hostile native provider; for P11 the maintainer
  accepted non-adversarial evidence instead (decision of 2026-09-28): the strict-Linux
  attestation checks (fixture-file and decision-table tests, fuzz targets
  `host_attestation` and `mountinfo`, `ISOLATION_UNAVAILABLE` before any work off an
  attested host) and the hardened `strict-worker-boundary` CI container job, which
  verifies the inherited container and cgroup controls (read-only root, no network,
  CPU, memory, swap and PID limits, no capabilities, no new privileges, an unprivileged
  user) around the process supervisor. **By the maintainer's decision of 2026-10-03
  the adversarial evidence is not produced in R0: it moves to R1** (ADR 0024, decision
  E, option 4).
- **Evidence:** [P11 qualification record](p11-worker-host.md) (SEC-T01);
  [verification](verification.md) "P11 PR 4 evidence"; the handoff document
  [sec-t01-adversarial-handoff.md](sec-t01-adversarial-handoff.md); ADR 0024's amendment
  of 2026-10-03; evidence item RQ-14, recorded `waived` by that decision.
- **Impact:** strict isolation is attested and its controls are shown present, but
  that they contain a hostile decoder is not demonstrated. R0 therefore makes no claim
  that the strict worker profile contains a hostile decoder or provider, and the worker
  host stays a qualification target; the claims registry's ban BAN-02 stays in force.
- **Why:** maintainer decision (2026-09-28, option 2): the adversarial work was
  deferred; on 2026-10-03, after the automated safety check stopped the authoring of the
  hostile stand-in provider a second time, the maintainer chose to narrow the claim
  rather than have it authored again.
- **Mitigation:** strict mode fails closed; the runbook's deployments apply the same
  controls as the CI container job; desktop profiles claim no isolation
  ([L-004](#l-004)).
- **Next step:** in R1, the maintainer writes or reviews a hostile stand-in provider run in
  the hardened CI container (option A of the handoff), or chooses a recognised third-party
  containment suite or an external review (B, C). Do not author the stand-in automatically
  for R0. When the evidence passes, lift BAN-02 in the claims registry and delete this entry.
- **Owner:** R1. **Issue:** [#188](https://github.com/smormah/vsift/issues/188).
  **Status:** deferred (technical debt). **Review:** rescheduled to R1 (2026-10-03).

### L-069

**A request that failed for good because of the host replays that failure.**

- **What:** a request whose step fails with a permanent code has ended, and every
  later delivery of the same operation id replays that failure. Some permanent codes
  describe the host rather than the request: `MISSING_CAPABILITY` (a tool or model not
  registered, or changed during the run) and `RESOURCE_LIMIT` from the workspace's
  free-space reserve before a copy. After the operator fixes the host, the same
  message still replays the failure.
- **Evidence:** domain `ends_request` and `RetryClass::of`; ADR 0021 PR 3 notes
  ("Ended, interrupted, busy"); the [worker-host runbook](../operations/worker-host.md)
  sections 4, 8 and 9.
- **Impact:** a supervisor must route these failures to a person and resubmit the work
  under a new operation id once the host is fixed; the earlier failed session stays
  until it expires or is cleaned.
- **Why:** a replay must never change what an acknowledged id means; transient
  classes (`BUSY`, `DEADLINE_EXCEEDED`, `STORAGE_IO`, `CANCELLED`) already stay
  continuable.
- **Mitigation:** the result names the code and the failing step; the runbook's
  acknowledgement table marks the host unhealthy on `MISSING_CAPABILITY` and says to
  keep disk free above the reserve.
- **Next step:** none planned; revisit if operators need a host-side retry class.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-074

**SubRip markup removal is broader than the contract lists.**

- **What:** the contract names SubRip `<i>`, `<b>`, `<u>` and `<font>` tags and `{\...}`
  blocks as recognised markup; the importer removes any tag whose name starts with a
  letter (for example an HTML `<a href=...>` anchor) from `text`. The payload as
  written stays in `original_text` with `markup` `removed` and the `markup_removed`
  warning, so nothing is lost.
- **Evidence:** found while writing the SEC-T02 suite (P12 PR 2, 2026-09-28):
  `crates/vsift-infrastructure/src/transcript_sidecar.rs` (`tag`) against
  `docs/contracts/cli-v1.md` "P07 supplied transcripts"; the suite asserts only what the
  contract promises.
- **Impact:** a reader of `text` sees less of a SubRip cue than the contract suggests;
  an agent comparing `text` with `original_text` sees why.
- **Why:** the removal predates the adversarial sidecar and was never compared with it.
- **Mitigation:** `original_text` is always kept when anything was removed.
- **Next step:** decide whether the contract should say "HTML-like tags" or the
  importer should keep unknown SubRip tags as text; either needs a contract test.
- **Owner:** unscheduled. **Issue:** none. **Status:** open. **Review:** pending.

### L-014

**A session holds at most 384 evidence files (512 artifacts, 128 KiB manifest).**

- **What:** a session keeps at most 512 artifacts, a 128 KiB manifest, 10 GiB and a
  4,096-generation chain; evidence (images, clips and their records) has a sub-budget of
  384 artifacts. A 12-frame burst uses up to 13 of those slots. P10 PR 2 raised the
  caps from 256 artifacts, a 64 KiB manifest and 160 evidence artifacts (ADR 0020 D-2).
- **Evidence:** [ADR 0019](../decisions/0019-evidence-navigation.md) D4 and its
  2026-09-27 note; [resource profiles](support-and-resource-profiles.md) "Evidence
  navigation"; tests `the_raised_evidence_cap_holds_and_its_manifest_reads_back` and
  `a_session_holds_at_most_512_artifacts_in_a_bounded_manifest`; the full-budget
  measurement `s11_warm_reuse_with_a_full_evidence_budget` (slope about 0).
- **Impact:** a very long investigation still hits `RESOURCE_LIMIT` (or a `partial`
  result with `session_evidence_budget`) and must retain the session and open a new one.
- **Why:** every read hashes the head manifest and evidence calls decode every record
  ([L-013](#l-013)); bounds keep both small.
- **Mitigation:** fixed remediation (retain and reopen); reuse does not consume slots.
- **Next step:** none planned; an index ([L-013](#l-013)) would allow more.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-050

**Jobs and operation ids are bounded per session.**

- **What:** a session keeps at most 64 jobs; at the bound the oldest ended job that no
  process owns and no caller's operation id pins is pruned, and when every ended job is
  pinned a new job is `RESOURCE_LIMIT`. A job answers to at most 8 caller operation
  ids and a session keeps at most 256 bindings. Operation ids are session-scoped and
  expire with the session: after it is cleaned, the id means nothing. A job that
  failed or was cancelled is started again from nothing by the same request. Asking
  for a job's status takes a shared lock on its owner lock for an instant, so a run of
  the same job starting at that moment may be told `BUSY`.
- **Evidence:** [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
  "Implementation notes: PR 2"; [resource profiles](support-and-resource-profiles.md)
  "Recoverable jobs"; test
  `ended_jobs_are_pruned_at_the_bound_unless_an_operation_id_pins_them`.
- **Impact:** a caller replaying a very old operation id after its job was pruned
  (only possible without its own id) runs new work; a heavy user of operation ids in
  one session can run out of room.
- **Why:** job state is bounded like every other session store.
- **Mitigation:** sessions last at most seven days; `BUSY` is retryable.
- **Next step:** revisit with P11's durable workspace.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-015

**Bursts over more than 20 s of 60 fps video are refused.**

- **What:** one frame listing covers at most 60 s and 1,200 frames. A burst over a range
  denser than that (60 fps over more than 20 s) is rejected with `outside_listing`
  rather than listed in pieces. Neighbours search windows of 2, 10 and 29 s, so on a
  very sparse stream a side can stop with `search_window` although more frames exist.
- **Evidence:** [ADR 0019](../decisions/0019-evidence-navigation.md) section 6 and the
  PR 2 note ("rejected for now"); [P09 record](p09-evidence-navigation.md) residuals.
- **Impact:** agents must split long bursts on high-frame-rate recordings.
- **Why:** bounded listing per provider run (SEC-05); multi-listing bursts not built.
- **Mitigation:** typed failure; the stop reason says more may exist.
- **Next step:** list in several bounded runs for dense ranges.
- **Owner:** unscheduled. **Issue:** [#172](https://github.com/smormah/vsift/issues/172). **Status:** open. **Review:** pending.

### L-016

**Delivered file paths die with the session; very long Windows paths keep the `\\?\` form.**

- **What:** `files[].path` is the absolute path of the committed artifact. On Windows
  it is the plain form `C:\...` when that form is exact, and the extended-length form
  `\\?\C:\...` otherwise: at or beyond `MAX_PATH`, or when a component has a trailing
  dot or space, a reserved device name or a character Win32 normalisation would
  reinterpret. Paths are valid only while the session exists; a path that is not
  valid UTF-8 is `STORAGE_IO`.
- **Evidence:** [ADR 0019](../decisions/0019-evidence-navigation.md) D2, PR 3 note
  (maintainer decision 2026-09-26) and the 2026-09-29 note (#210);
  [CLI contract](../contracts/cli-v1.md) P09 frames.
- **Impact:** some agent file tools (Claude Code's `Read` and its permission rules)
  refuse the extended-length form, so under a very long session root an agent still
  needs its retry without the prefix; a closed session invalidates earlier paths.
- **Why:** only the extended-length form is exact beyond `MAX_PATH`; evidence records
  never hold paths.
- **Mitigation:** keep the session root short: the session folders and artifact name
  add 134 characters, so a root of at most 125 characters gets plain paths; the skill
  retries once without the prefix. Human output (P13 PR 2b) writes each path whole on
  a line of its own under its item, and after an extended-length path says once how
  to open it (PowerShell's `Copy-Item -LiteralPath`) or avoid it (a root of at most
  125 characters); a path with control or hidden characters is shown inert and
  flagged, with `--json` named for the exact text. The JSON path form is unchanged.
- **Next step:** none planned; the extended-length form stays the exact fallback.
- **Owner:** unscheduled. **Issue:** none.
  **Status:** accepted residual. **Review:** pending.

### L-017

**Human mode shows no progress; the worker hosts' event stream is JSON Lines only.**

- **What:** since P13 PR 2b every command prints readable text without `--json`. Long
  commands (`transcript retranscribe`, `job resume`, `job run`, `job batch`) print
  only their final result in human mode: their `progress` events, and the worker
  hosts' `lifecycle` and per-request `result` events, exist only in `--events jsonl`,
  which stays one versioned JSON value per line. `job batch`'s human summary lists
  each line's outcome, not its full job result.
- **Evidence:** [CLI contract](../contracts/cli-v1.md) ("Human-readable text" under
  "Output protocol"); [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 2b note.
- **Impact:** a person running a long command by hand sees nothing until it ends; an
  operator watching a worker host reads JSON Lines. Agents and supervisors are
  unaffected.
- **Why:** the event stream is a supervisor's interface (sequence numbers, never-dropped
  lifecycle and result events, 64 KiB lines, ADR 0021); a second, unstable human form
  of it would add a protocol without a user.
- **Mitigation:** `--events jsonl` for live progress; `job status` for a recoverable
  job at any time; `session status` for a session's jobs.
- **Next step:** none planned; a progress line on stderr in human mode is possible
  later without changing any contract.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-018

**Tiny text is measured on synthetic glyphs only; crops are never upscaled.**

- **What:** small-text crops were checked on a synthetic 5x7 glyph drawn at scale 4
  (a 20x28 crop). Real screen text with anti-aliasing, subpixel rendering and
  compression is not in the corpus. Crops are delivered at native size; an agent that
  needs a larger view must scale the image itself.
- **Evidence:** [P09 record](p09-evidence-navigation.md) V-06 and residuals.
- **Impact:** readability of real small UI text through VSift crops is unmeasured.
- **Why:** no real recordings in the corpus; no scaling so no detail is invented.
- **Mitigation:** native pixels are exact (pixel-equal to FFmpeg's decode).
- **Next step:** add real-recording text fixtures with [L-028](#l-028).
- **Owner:** unscheduled. **Issue:** [#173](https://github.com/smormah/vsift/issues/173). **Status:** open. **Review:** pending.

### L-019

**A seek that lands past the requested frame reports "not found".**

- **What:** when a container's start is far from the probed origin, the extraction's
  seek can land after the requested frame; VSift then reports `FrameNotFound` instead of
  extracting another frame.
- **Evidence:** [ADR 0019](../decisions/0019-evidence-navigation.md) "Consequences",
  residuals.
- **Impact:** a rare typed failure on unusual containers; never wrong pixels.
- **Why:** exactness over guessing.
- **Mitigation:** typed failure; nothing committed.
- **Next step:** measure whether any corpus or real file triggers it.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-054

**A second interruption cannot cut short VSift's own work between boundaries.**

- **What:** the second Ctrl-C or `SIGTERM` skips the providers' graceful stop, but work
  VSift does itself runs to its next cancellation check: hashing the session's source
  copy when a command binds it (up to 20 GiB), a publication in progress, and one 64 KiB
  block of an `ingest` copy. The process never exits before that, so it never leaves a
  provider or a half-written file behind.
- **Evidence:** ADR 0020 PR 3 notes; [L-011](#l-011) (the first evidence call hashes
  the whole copy).
- **Impact:** a command interrupted while hashing a large source can take as long as
  the hash (seconds per GiB) to end.
- **Why:** those steps are the integrity checks and atomic writes the result depends
  on; abandoning them would need an exit that skips cleanup.
- **Mitigation:** `SIGKILL` or closing the console still ends the process (on Unix see
  [L-055](#l-055)); nothing partial is ever committed.
- **Next step:** make the source hash check cancellation per block with L-011's work.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

## Speech recognition

### L-020

**Noisy speech: `base` word error rate 61.5% on F08, not gated.**

- **What:** on the only noisy clip (office noise at 10 dB SNR, 13 words, English and
  Spanish) the default `base` model got 61.5% word error rate (`base_q5_1` 46.2%)
  against a proposed 25% gate. The maintainer decided on 2026-09-25 to gate noisy speech
  on critical terms only and report the rate as a known limitation.
- **Evidence:** [P07 ASR record](p07-asr-qualification.md) (Windows 11, Xeon
  E5-2698 v4, whisper.cpp v1.9.2, 4 threads); [ADR 0005 note](../decisions/0005-r0-scope-and-qualification-profiles.md);
  issue #150. One word moves F08's rate by 7.7 points.
- **Impact:** transcripts of noisy recordings can be substantially wrong.
- **Why:** the sample is too small to gate on.
- **Mitigation:** confidence stays `provider_uncalibrated`; times are cited so an agent
  can check the audio clip; critical terms are gated.
- **Next step:** #150 noisy fixture set (20/10/5 dB, several hundred words per level,
  English and Spanish), then a gate proposal.
- **Owner:** unscheduled (R0 backlog). **Issue:** [#150](https://github.com/smormah/vsift/issues/150).
  **Status:** deferred. **Review:** pending.

### L-021

**Reviewed known misses: "queued", "4407", "E-409".**

- **What:** the qualification allows three reviewed misses of critical terms by `base`:
  F04 "queued" (heard as "Q" alone; heard correctly in the recorded runs), F05
  "invoice 4407" (heard "Invoice407"; `base_q5_1` "in Voice 40407"), F08 "E-409" (heard
  "E4 and I"). An Intel AVX-512 runner also heard F05's "invoice" as "in voice".
- **Evidence:** [P07 ASR record](p07-asr-qualification.md) per-clip table and hosted
  runners.
- **Impact:** identifiers and numbers can be misheard; search does not bridge the
  difference (`407` never finds `4407`).
- **Why:** limits of the `base` model.
- **Mitigation:** gates fail on any unexpected miss; agents must confirm critical
  identifiers against the source.
- **Next step:** re-measure with #150's fixtures; consider a larger optional profile.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-022

**No accent, crosstalk, human-voice or long-recording ASR evidence.**

- **What:** every speech clip is one synthetic (Kokoro) voice per sentence; there is no
  human recording, regional accent or overlapping speech. Accuracy is measured on
  single-chunk clips; the timing clip is 3 minutes. T-04 stays open for these.
- **Evidence:** [P07 ASR record](p07-asr-qualification.md) "Gaps"; issue #150 scope;
  [verification](verification.md) T-04.
- **Impact:** real meeting and QA speech accuracy is unknown.
- **Why:** needs human or otherwise licensed recordings.
- **Mitigation:** honest reporting; uncalibrated confidence.
- **Next step:** licensed human fixtures (#150 lists them so they are not lost).
- **Owner:** unscheduled. **Issue:** [#150](https://github.com/smormah/vsift/issues/150).
  **Status:** open. **Review:** pending.

### L-023

**ASR output differs across CPU backends; revision ids differ by host and root.**

- **What:** whisper.cpp output is deterministic on one host but ggml picks an optimised
  CPU backend at load time, and backends differ slightly in floating point. Transcripts,
  and therefore content-derived revision ids, can differ between machines. The P07
  checkpoint's whole-file stage fails with `base_q5_1` (F05 "in voice"), so the workflow
  runs it with `base` only. Since P11 PR 2 the recognizer's thread count is also its
  admission weight and is capped at min(available parallelism, 8, the root's admission
  capacity); the count is recorded in the run's provenance, so the same audio
  recognised on a root of smaller capacity (a desktop root has 4 units) is a different
  run with a different revision id, and a job interrupted before the upgrade with more
  threads starts afresh under its new identity instead of continuing.
- **Evidence:** [ADR 0017](../decisions/0017-local-asr-through-whisper-cpp.md)
  consequences; [P07 ASR record](p07-asr-qualification.md) hosted runners (run
  36198903762).
- **Impact:** the same video can yield slightly different transcripts on two machines.
- **Why:** upstream runtime behaviour.
- **Mitigation:** journey tests check only words every reviewed host hears; accuracy is
  measured separately.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-024

**An ASR segment can start at the audio's start, before the speech.**

- **What:** the `base` model starts a segment that follows leading silence at the start
  of its audio: F09's segment starts at 0.75 s although speech starts at 4.0 s.
  Citations are at segment granularity.
- **Evidence:** [ADR 0017](../decisions/0017-local-asr-through-whisper-cpp.md) "Known
  limits"; the P09 local-ASR journey cited 3.0-8.0 s for a term spoken at
  6.31-8.33 s ([P09 record](p09-evidence-navigation.md)).
- **Impact:** a cited start time can precede the words by seconds.
- **Why:** model behaviour; no voice-activity trimming in R0 (VAD is R1 enrichment).
- **Mitigation:** segments still contain the speech; audio clips let an agent check.
- **Next step:** measure segment-start error on #150's fixtures; decide whether word
  timestamps or trimming are needed before R0.
- **Owner:** unscheduled. **Issue:** [#174](https://github.com/smormah/vsift/issues/174). **Status:** open. **Review:** pending.

### L-025

**Local ASR runs: progress is coarse and advisory, model hashed per run.**

- **What:**
  - Since P11 PR 1, `transcript retranscribe` and `job resume` with `--events jsonl`
    write `progress` events (ADR 0021 section 7), which closes the "looks stalled" half
    of this entry. What remains: progress counts whole 30 s chunks (at a real-time
    factor of 0.39 one chunk takes about 12 s, so the count can stand still that long);
    at most one event per second, and an update inside the second is held until the
    next one or the terminal event (there is no timer); progress is dropped, and
    counted in `progress_dropped`, when the reader is slower than the work; `--json`
    and human output show none (`job status` from another process shows the chunks
    checkpointed so far).
  - The model file is hashed up to three times per run (about 0.3 s each in release),
    with no identity cache; the first run with a new tool, model or VSift version adds
    a fixture transcription (about 6 s).
  - Since P10 PR 3 Ctrl-C and `SIGTERM` are trapped (ADR 0017 decision 4 superseded):
    the run stops before its commit, keeps its finished chunks and names the job to
    resume; that part of this entry is closed.
- **Evidence:** [ADR 0017](../decisions/0017-local-asr-through-whisper-cpp.md) section
  6 and its 2026-09-28 note; [ADR 0021](../decisions/0021-worker-and-batch-host.md)
  section 7; ADR 0020 PR 3 notes; the CLI's `progress` tests and the frozen
  `transcript-retranscribe.events.jsonl`.
- **Impact:** a caller sees a long run advance chunk by chunk, not second by second;
  a slow reader can miss intermediate counts.
- **Why:** progress is advisory by design (the terminal event is authoritative), and
  keeping it bounded and non-blocking matters more than its resolution.
- **Mitigation:** `job status <job>` reports `progress.chunks_checkpointed` of
  `chunks_total`; the event's `progress_dropped` says when counts were lost.
- **Next step:** none planned; finer (per-window) progress would need whisper.cpp's
  own progress output.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-026

**whisper.cpp output with a split multi-byte token fails the chunk.**

- **What:** older whisper.cpp builds could split a multi-byte character across tokens,
  making the `-ojf` document invalid UTF-8; VSift rejects it and the chunk fails.
  The pinned v1.9.2 writes valid UTF-8 (the Spanish F08 output passes).
- **Evidence:** `crates/vsift-infrastructure/tests/whisper_output.rs`
  (`the_spanish_bearing_output_is_valid_utf8`); [work record](../../memory/TODO.md).
- **Impact:** a user-supplied older build can fail on non-ASCII speech.
- **Why:** strict parsing of untrusted output.
- **Mitigation:** typed failure; reviewed build recommended.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-027

**whisper.cpp is the only speech engine.**

- **What:** local ASR runs only through the whisper.cpp CLI; a faster-whisper adapter is
  an approved backlog item, not scheduled in R0.
- **Evidence:** issue #147; [ADR 0017](../decisions/0017-local-asr-through-whisper-cpp.md)
  section 1.
- **Impact:** users with faster-whisper or GPUs cannot use them.
- **Why:** one reviewed reference engine for R0 qualification.
- **Mitigation:** hosts can supply their own recognizer through the engine library.
- **Next step:** schedule #147 after R0.
- **Owner:** unscheduled. **Issue:** [#147](https://github.com/smormah/vsift/issues/147).
  **Status:** deferred. **Review:** pending.

## Visual candidates

### L-028

**Change thresholds are calibrated only on the synthetic corpus.**

- **What:** a block counts as changed at 4 grey levels (two blocks, or one at 6).
  Unchanged samples of the synthetic corpus differ by at most 1 level, so the thresholds
  are four to six times that noise. Real screen recordings with heavier compression
  noise have not been measured.
- **Evidence:** [ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)
  decision 10; [P08 recall record](p08-candidate-recall.md) "Not measured";
  `crates/vsift-domain/src/visual.rs` rustdoc.
- **Impact:** on real recordings candidates may include false changes or miss subtle
  edits; the recall and false-change figures (10/10 stable, 0 false changes) apply to
  the synthetic corpus only.
- **Why:** no licensed real recordings in the corpus.
- **Mitigation:** candidates are a shortlist, not evidence; periodic coverage every
  10 s; frames and bursts let agents inspect directly.
- **Next step:** measure noise and recall on project-owned real screen recordings.
- **Owner:** unscheduled. **Issue:** [#175](https://github.com/smormah/vsift/issues/175). **Status:** open. **Review:** pending.

### L-029

**2 Hz sampling misses changes shorter than 0.5 s or below the change rule.**

- **What:** frames are sampled at most every 0.5 s at 128x72 grey. A change shorter than
  0.5 s that falls between samples, or smaller than the change rule, is not seen, and
  the result's coverage cannot report what sampling did not see. There is no denser
  pass. At most 32 candidates per window are kept (drops are reported).
- **Evidence:** [ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)
  decisions 2 and 10 and consequences; [P08 recall record](p08-candidate-recall.md): F06's
  tooltip found 300 ms late, F09's marker 250 ms late; threat model P08 residual.
- **Impact:** brief flashes (toasts, tooltips) can be missed silently.
- **Why:** bounded, cheap analysis (maintainer decision 2026-09-26).
- **Mitigation:** `change_window` brackets the true start; bursts and neighbours give
  dense frames on request.
- **Next step:** consider a targeted denser pass around transcript hits.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-030

**Motion fixtures draw no motion; scrolling and cursors only synthetic.**

- **What:** the P04 generator draws F04-E02 (scroll), F05-E02 (loading indicator) and
  F12-E02 with exactly the pixels of the state before them, so no visual method can see
  them begin. The corpus has no real scrolling, cursor movement or loading animation;
  V-03 motion is tested only on clips built at test time (FFmpeg `life` scroll and zoom).
  Animation and overlapping cells are not covered.
- **Evidence:** issue #159; [P08 recall record](p08-candidate-recall.md) "Corpus
  limitations" and "Motion"; ADR 0018 decision 3.
- **Impact:** recall on scroll and loading events is unmeasured on the corpus.
- **Why:** generator defect found in P08.
- **Mitigation:** the three events are re-verified as identical on every run and
  reported as corpus limitations.
- **Next step:** #159: regenerate the fixtures (truth first), re-record samples, move
  the events into the gate; regenerate the P07 speech variants that copy the video.
- **Owner:** unscheduled (R0 backlog). **Issue:** [#159](https://github.com/smormah/vsift/issues/159).
  **Status:** open. **Review:** pending.

### L-031

**The visual index is tied to the probed duration; 30 minutes per call.**

- **What:** the stored index records the probed duration; if a later probe reports a
  different duration (for example after switching FFprobe builds), the index no longer
  describes the source and reads fail. One `candidates` call analyses at most 30
  windows (30 minutes); the rest is `not_analyzed` until a later call.
- **Evidence:** [ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)
  decisions 9, 11 and 13; `crates/vsift-application/src/visual.rs`.
- **Impact:** long videos need several calls; a tool change mid-session can invalidate
  candidates.
- **Why:** bounded work per call; strict re-derivation on read.
- **Mitigation:** typed coverage gaps; retain and reopen with a fresh session.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

## Search and transcripts

### L-032

**Search: no Unicode folding, no cross-segment phrases, no compound number words.**

- **What:**
  - No Unicode normalisation or accent folding: precomposed and decomposed spellings
    differ, and "identificacion" does not find "identificación".
  - A phrase that runs from one segment into the next is not found.
  - Number words `zero`..`twenty` and the tens become digits, but compounds such as
    `thirty-two` are joined (`thirtytwo`) and not converted to `32`.
  - Only transcript text is searched; on-screen text is not (no OCR in R0).
- **Evidence:** [ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)
  decisions 2, 3, 7 and consequences; `crates/vsift-domain/src/search.rs` rustdoc.
- **Impact:** "no hit" can mean "said differently"; results report `scope:
  transcript_text`.
- **Why:** Unicode tables would be a new dependency; phrase matching is per segment by
  design.
- **Mitigation:** normalisation of spelling, hyphens, thousands and decimals; all-terms
  tier.
- **Next step:** evaluate a reviewed Unicode dependency and compound numbers.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-033

**A supplied transcript is assumed to cover the whole video.**

- **What:** a supplied SRT/WebVTT file is taken to cover the whole source; its
  completeness is not verified, so a region it does not cover is not reported as
  untranscribed. For spliced local-ASR revisions coverage can be understated (never
  overstated).
- **Evidence:** [ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)
  decision 7.
- **Impact:** with a partial supplied transcript, "no hit" may be read as "not said".
- **Why:** VSift cannot know what the author left out.
- **Mitigation:** `basis: supplied_transcript` is reported.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-034

**Speech fixtures are synthetic and partly unaligned.**

- **What:** the Kokoro speech fixtures' direct Python dependencies are pinned but the
  transitive set is not installed from a hash-locked file; the Spanish segment has no
  word timings; clean fixtures' speech is not aligned with their visual events; the
  model's training data cannot be checked independently (residual, low).
- **Evidence:** [P07 speech fixtures](p07-speech-fixtures.md) "Known limits" and
  "Licence review".
- **Impact:** cross-modal timing truth exists only for the speech variants the journeys
  use (F03-speech).
- **Why:** test-only generator; fixtures are data.
- **Mitigation:** committed clips with provenance and byte-for-byte verification.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

## Platforms, distribution and missing R0 capabilities

### L-035

**Evidence on three systems exists only for the published 0.1.0 and a synthetic corpus; no
platform has the release matrix's rules met yet.**

- **What:** the P08 and P09 numbers and the performance figures were recorded on one
  Windows 11 machine. Since P14 PR 3 the published 0.1.0 has also run the P06 to P11
  real-tool checkpoints and the hostile-path and sentinel checks on hosted Ubuntu 24.04
  (tools installed by its own `setup install`), Windows (the repository's pinned builds) and
  macOS 15 arm64 (Homebrew's tools, [L-114](#l-114)): all passed, with P11's durable stage
  blocked ([L-113](#l-113)). That is evidence for 0.1.0 only (the candidate and the stable
  repeat it), on hosted virtual machines, with a synthetic corpus. The matrix rules (ADR 0024
  decision F) also need the clean installs, the extracted archives and the guide's walks
  (RQ-01, RQ-02, RQ-04). Linux desktop and other distributions, network filesystems and
  Windows or macOS worker use are unqualified.
- **Evidence:** [resource profiles](support-and-resource-profiles.md);
  [P09 record](p09-evidence-navigation.md) residuals;
  [P07 ASR record](p07-asr-qualification.md); [P06 source review](p06-provisioning-source-review.md);
  `P14 journeys` run 36965956708; [P14 plan](p14-qualification.md) section 17.
- **Impact:** no platform may be called "supported" yet; only "qualification target".
- **Why:** P14 owns the release matrix.
- **Mitigation:** cross-platform Quality CI on every PR; the weekly `P14 journeys` run.
- **Next step:** the candidate's own runs (P14 PR 11) and the matrix decision (PR 9).
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** deferred. **Review:** pending.

### L-037

**Managed installation is qualified on Ubuntu 24.04 x64 only, and its power-loss claim is for ext4 only.**

- **What:** on Ubuntu 24.04 x86-64, `setup install` applies an accepted plan (download
  or `--artifact-dir`, size and SHA-256, stage, smoke, activate), every command resolves
  tools through the managed tier, and `setup list`, `setup rollback`, `setup remove` and
  `setup repair` manage what is installed; an accepted install sweeps stages that killed
  runs abandoned and keeps only the selected and previous version of each component.
  Since P13 PR 7 a kill at every crash point of `setup install`, `setup rollback`,
  `setup remove` and the stale-stage sweep is tested on every CI OS (every arrival of
  every point on Linux), and so are real `SIGKILL`/`TerminateProcess` kills; each leaves
  a consistent store that `setup repair` describes exactly and a rerun completes; every
  folder a command changes is flushed before it returns (Windows has no flush point, and
  no managed install). The power-loss campaign (layer A of the P10 campaign with the managed workload, workflow
  `P13 managed power loss`) first ran on 2026-10-01 (run 36793177930): no damage, a clean
  `e2fsck` at all 1,812 points, but 53 acknowledgements reported lost, every one because
  the verifier compared the selection of the command in flight at the point (always
  newer, never older) with the previous acknowledgement. The verifier was fixed (ADR 0023
  PR 7 addendum, #244) and the re-run on `main` at `6de55da` passed (run 36829198545: 0
  lost acknowledgements, no damage, a clean `e2fsck` and mounts at 1,812 points; the
  negative control lost 36). The claim is for Ubuntu 24.04 with ext4 on a hosted runner,
  one-file stand-in versions and a workload of 150 managed commands, and nothing else.
  Windows x86-64 and macOS have no reviewed catalogue and keep
  manual guidance (the lifecycle commands work there and report the folder absent): the
  Windows FFmpeg candidate is a daily build (upstream keeps only the last 14) with
  unreconciled LGPL-3.0 notices, and no macOS candidate is reviewed.
- **Evidence:** [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 3, PR 4, PR 6 and PR 7 notes; [CLI contract](../contracts/cli-v1.md) "P13 `setup install`"
  and "P13 managed lifecycle"; [verification](verification.md) D-02..D-08 and "P13 PR 7
  evidence"; [P06 Windows candidate](p06-windows-artifact-candidate.md) "Remaining gates".
  *2026-09-30:* `P13 managed smoke` [run 36734316384](https://github.com/smormah/vsift/actions/runs/36734316384) on `main` at `d43a518` (hosted `ubuntu-24.04`)
  passed both jobs: the pinned-tool smoke's negative control (banner mismatch)
  discarded all three stages and activated nothing, and the real `setup plan`, `setup
  install`, `setup check` and rerun through the CLI activated all three components.
  Managed installation is qualified on Ubuntu 24.04 x64 by that run.
- **Impact:** elsewhere users install FFmpeg and whisper.cpp themselves and register
  them; on Ubuntu the claim that a reported command survives a power loss rests on the
  flushes (each checked by a unit test), the kill tests, ext4's ordered journal and the
  passing campaign run; on another filesystem, or a disk that ignores flushes
  ([L-056](#l-056)), it is unqualified.
- **Why:** ADR 0023 decision E limits managed installation to the one reviewed target;
  decision H9 has the power loss qualified by the P10 campaign on a disposable runner,
  which runs only on dispatch.
- **Mitigation:** every managed version is verified by digest each time it is opened, so
  a torn version is never run and lookup falls through to `PATH`; `setup repair`
  diagnoses what an interruption leaves and names the command that fixes it; the manual
  path is typed on every target.
- **Next step:** done: the power-loss run and `P13 managed smoke` with its `install-e2e`
  (run 36793180858) are recorded in [p13-distribution.md](p13-distribution.md), and the
  entry was re-read at P13's close (2026-10-01): it stands. Managed installation on other
  targets is not in R0. Before a stable release, dispatch both workflows on the release
  candidate (they run only on demand, [L-042](#l-042)).
- **Owner:** P14 (the release qualification re-dispatches both workflows; P13 is complete).
  **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:** deferred.
  **Review:** pending.

### L-038

**The worker host is a qualification target, not a supported platform.**

- **What:** P11 is implemented: `job run` and `job batch` in a worker workspace,
  weighted admission, contained inputs, strict Linux attestation, request records,
  the two-stage shutdown, the `p11_*` single-host checkpoint, the
  [operator runbook](../operations/worker-host.md) and the
  [qualification record](p11-worker-host.md). The packet completes when it merges.
  What it does not give: public support (P14 qualifies the release matrix), adversarial
  containment evidence for the strict profile ([L-068](#l-068)), and a run of the
  runbook's systemd unit and container example exactly as written: they are adapted
  from the CI container job and the tested flags, and an operator confirms on the host
  that a batch starts with `isolation` `strict_linux`. The single-host checkpoint ran on
  one Windows 11 machine; Linux and macOS run the contract tests in CI.
- **Evidence:** the [P11 qualification record](p11-worker-host.md); ADR 0021
  implementation notes; P11 row of the [work packets](implementation-work-packets.md).
- **Impact:** a worker deployment follows reviewed guidance, but its hardening is the
  operator's to verify; documentation says "qualification target".
- **Why:** support is claimed only after release qualification (ADR 0005).
- **Mitigation:** strict mode fails closed off an attested host; the runbook's
  readiness rule refuses work before `started` says what is in force.
- **Next step:** P11 merge; P14 release qualification.
- **Owner:** P11, P14. **Issue:** [#14](https://github.com/smormah/vsift/issues/14),
  [#17](https://github.com/smormah/vsift/issues/17). **Status:** deferred.
  **Review:** pending.

### L-095

**Review-tier models can state blurred content as supported by pixels; the skill fix
is not yet re-measured.**

- **What:** in P12's final campaign on `56f1e1f`, Claude Opus 5.5 (A-09-f05-blurred
  runs 1 and 2) and GPT-6-Astra (run 1) stated the content of the deliberately blurred
  error banner as `supported` by frames. The narration says it, so the right support
  is `partially_supported` on the transcript. The maintainer upheld the strict grade
  on 2026-09-30.
  - Opus passed A-09 in 4 of 6 runs, below four in five for A-09 alone; Astra passed
    5 of 6.
  - Across A-08 and A-09 the review tier still meets its gate: Opus 9 of 11, Astra 10
    of 11.
- **Evidence:** the [qualification record](p12-agent-qualification.md) (strong tier
  and the maintainer's review); ADR 0022's note "the P12 debt fixes".
- **Impact:** a review-tier agent can present something it could not read as seen in
  the pixels. The statement itself is true (the transcript supports it), but its
  support label overstates the evidence.
- **Why:** the models infer the blurred content from the narration and cite the frame.
- **Mitigation:** the P12 debt fixes (2026-09-30) changed the skill's VERIFY_SOURCE
  and `handoff.md`: when a frame or crop shows a region unreadable, a claim about its
  content rests on the transcript alone, is `partially_supported` and cites the
  segment. The guard holds the wording.
- **Next step:** re-run A-09-f05-blurred on the review tier (Claude Opus 5.5 and
  GPT-6-Astra). The compact re-run #222 (2026-09-30) did not include it. It is inside
  P14's batch 2 (three runs per review-tier client; a gate of at least 2 of 3 with no
  claim of the blurred text stated as supported by pixels), from a clean install; the
  harness for it is built (P14 PR 6) and the runs wait for the maintainer's go, because
  they spend the client allowances.
- **Owner:** P14 (batch 2, PR 11), on the maintainer's go. **Issue:**
  [#224](https://github.com/smormah/vsift/issues/224). **Status:** deferred (technical
  debt). **Review:** pending.

### L-053

**Windows: a process that inherited "ignore Ctrl-C" sees only Ctrl-Break.**

- **What:** Windows never tells a console process that inherited the "ignore Ctrl-C"
  attribute about a Ctrl-C (a child of a service, of some IDE and agent hosts, or of a
  process created in a new process group); such a `vsift` keeps running to its end.
  Ctrl-Break is always delivered and cancels it. Found while qualifying P10 PR 3: the
  agent host that ran the opt-in tests starts its processes this way.
- **Evidence:** ADR 0020 PR 3 notes; the opt-in `console_interrupts_cancel_an_ingest_copy`
  (Ctrl-Break always, Ctrl-C only with `VSIFT_TEST_CONSOLE_CTRL_C`).
- **Impact:** in such a host, Ctrl-C does not stop a long command.
- **Why:** clearing the attribute needs `SetConsoleCtrlHandler(NULL, FALSE)`, platform
  code that needs `unsafe`, which VSift forbids without an ADR.
- **Mitigation:** Ctrl-Break, `job cancel` from another process, or closing the console.
- **Next step:** none planned; revisit if an agent host is found that needs Ctrl-C.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-086

**`vsift-contract` embeds the skill's handoff schema from outside its crate folder, so
it cannot be packaged for crates.io as it is.**

- **What:** `handoff check` (P13 PR 5) validates drafts against
  `skills/vsift/handoff.schema.json`, which the skill owns. `vsift-contract` embeds that
  file with `include_str!("../../../../skills/vsift/handoff.schema.json")`, a path
  outside the crate's own folder. `cargo package` copies only the crate's folder, so a
  packaged `vsift-contract` would not build.
- **Evidence:** `crates/vsift-contract/src/handoff/mod.rs` (`HANDOFF_SCHEMA_JSON`); the
  test `the_embedded_schema_is_the_skills_file`; the ADR 0023 implementation note of
  2026-09-30 (P13 PR 5).
- **Impact:** none for R0, which publishes no crate (ADR 0023 decision B); the binary
  and the npm package embed the schema at build time. A crates.io release of the
  engine crates would fail to package.
- **Why:** the skill must stay the single owner of its schema (ADR 0022 decision 6),
  and embedding the file keeps the command and the skill from drifting.
- **Mitigation:** a test requires the embedded copy to equal the skill's file.
- **Next step:** before any crates.io publication (R1 or later): a build script that
  copies the schema into the package, a checked-in copy inside the crate guarded by the
  same equality test, or moving the schema's owner file into the crate with the skill
  pointing to it.
- **Owner:** unscheduled, before any crates.io publication. **Issue:** none.
  **Status:** deferred. **Review:** pending.

### L-089

**A release SBOM's `bom-ref` values name the build machine's checkout path.**

- **What:** cargo-cyclonedx 0.5.9 identifies each workspace crate in the SBOM
  (`vsift.cdx.json` in every release archive, P13 PR 8) by Cargo's package id, which
  for a path dependency is the absolute checkout path, for example
  `path+file:///home/runner/work/vsift/vsift/crates/vsift-cli#0.1.0`. The `purl` values
  are relative and the component names and versions are exact.
- **Evidence:** a local SBOM for `x86_64-pc-windows-msvc` held 25 `file:///` references,
  all to workspace crates (2026-09-30); `.github/workflows/release.yml`, job
  `inventory`.
- **Impact:** an SBOM made from a checkout elsewhere differs in those values, so SBOMs
  are reproducible only on the same runner layout; the path names the CI runner's
  workspace, not a person.
- **Why:** the tool writes Cargo's package id unchanged and has no option to rewrite it.
- **Mitigation:** the SBOM is generated only by the release workflow on a GitHub-hosted
  runner, whose checkout path is fixed.
- **Next step:** none planned; revisit if a cargo-cyclonedx release relativises path
  package ids, or if SBOMs must be reproduced off GitHub.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-091

**The npm launcher relays signals by a rule that can repeat or miss one, and cannot stop vsift if the launcher itself is killed.**

- **What:** installed through npm, `vsift` runs as a child of the launcher. On Linux and
  macOS the launcher relays `SIGTERM` and `SIGHUP` to vsift, and `SIGINT` unless one of
  its standard streams is a terminal (a terminal's Ctrl-C already reaches vsift, which is
  in the same foreground process group). So a signal sent to a whole process group that
  is not a terminal's (a supervisor or a "kill the tree" tool) reaches vsift twice, which
  vsift reads as an interruption and an escalation (ADR 0020 section 5: providers are
  stopped without the graceful wait); and a `SIGINT` sent to the launcher alone while a
  standard stream is a terminal is not relayed. If the launcher itself is killed
  (`SIGKILL`, Windows `TerminateProcess`, `taskkill /F` on the Node.js process), vsift
  keeps running until its command ends: nothing links its life to the launcher's.
- **Evidence:** [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 9 note; `npm/vsift-cli/lib/launcher.cjs` (`relaySignals`); `npm/test/launcher.test.cjs`
  (targeted and process-group interruptions); the Release workflow's `npm-qualify` jobs.
- **Impact:** an escalated interruption still ends the command with its documented result
  and never leaves a provider running (SEC-04); it only skips the providers' graceful
  stop. An orphaned vsift finishes or fails its one command and exits.
- **Why:** Node.js does not say where a signal came from, so the launcher cannot tell a
  terminal's Ctrl-C (already delivered) from one sent to it alone; a parent-death link
  needs native code on each platform, and the launcher is plain JavaScript by decision.
- **Mitigation:** supervisors signal the launcher's process (or vsift's) rather than the
  group; the native archive runs vsift with no launcher at all; a second interruption
  was already vsift's documented way to stop faster.
- **Next step:** none planned for R0; revisit if a supervisor needs exactly-once
  delivery through the launcher.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-092

**The npm qualification covers the minimum runtimes, one pnpm and one Yarn version, and Yarn only through a project install.**

- **What:** the Release workflow's matrix runs Node.js 22.23.3 and Bun 1.2.23 (the
  minimums of ADR 0023 decision H6), the npm Node.js ships, pnpm 12.8.1 and Yarn 4.18.1.
  Newer runtimes, older pnpm majors and Yarn 1 are not run. Yarn 4 has no global install,
  so Yarn is qualified through a project install and `yarn dlx`; the launcher-level
  checks of standard input through the runtime, the checks' cost and signals are made
  with npm, pnpm and Bun, because Plug'n'Play keeps the launcher inside a zip archive
  only `yarn` can open (standard input is checked through `yarn vsift`). Arguments with
  `cmd.exe` metacharacters (`%`, `!`, `^`, `"`) through npm's and pnpm's `vsift.cmd` shims
  are subject to `cmd.exe`'s rules; the matrix passes paths with spaces and Unicode
  through them, and the launcher's own tests pass metacharacters to the executable
  directly. Linux with musl libc is not a target: the launcher installs there but reports
  that the executable cannot start. Yarn 4.18 quarantines a version for a day after it is
  published (`npmMinimalAgeGate`, default `1d`); the qualification turns the gate off,
  because it publishes seconds before it installs, so for a day after each real release
  Yarn users must wait or preapprove `vsift-cli` and `@vsift/*` (`install.md`). Bun 1.2's
  `bun remove --global vsift-cli` removes the launcher and the command but leaves the
  platform package in Bun's global folder, where nothing runs it, and on Windows also its
  `vsift.exe` shim, which then fails without running vsift (the matrix records both).
  Bun 1.2.23 on Windows fails `bun add --global` with "InvalidWtf8" when its install or
  cache folder has non-ASCII letters (found by the matrix on 2026-09-30, a Bun defect),
  so the Windows Bun job keeps its own folders to ASCII with spaces; the arguments vsift
  receives still carry non-ASCII paths there.
- **Evidence:** `.github/workflows/release.yml` (the pinned versions); the ADR 0023 PR 9
  note; `npm/qualification/qualify.cjs`.
- **Impact:** a regression specific to a newer runtime or another package-manager major
  would first be seen by users.
- **Why:** twelve jobs already run on every pull request that touches an archive or npm
  input; the minimum runtimes are the supported floor.
- **Mitigation:** the launcher uses only long-stable `node:` APIs; the package managers'
  handling of `optionalDependencies`, `os` and `cpu` is long-standing behaviour.
- **Next step:** P14's release qualification may widen the matrix.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-094

**On Windows, vsift cannot start from an install whose executable path is 260 characters or longer.**

- **What:** Windows starts a program only from a path shorter than `MAX_PATH` (260
  characters). Package managers nest the platform package deeply (pnpm's global store,
  for example `…\global\v11\<hash>\node_modules\.pnpm\@vsift+win32-x64@0.1.0\node_modules\@vsift\win32-x64\vsift.exe`),
  so a long prefix or home folder can push the executable's path over the limit. The
  launcher's checks still pass (Node.js reads long paths), then starting fails; the
  launcher says so with exit 126, the path's length and the advice to install in a
  folder with a shorter path.
- **Evidence:** found while qualifying pnpm locally (2026-09-30): a pnpm global install
  under a 180-character work folder gave a 300-character executable path and `ENOENT`;
  `npm/test/launcher.test.cjs` checks the message.
- **Impact:** users with very long home, prefix or `PNPM_HOME` paths must choose a shorter
  one.
- **Why:** `CreateProcess` refuses longer executable paths; the launcher has no native code
  to work around it and must not change the working directory vsift sees.
- **Mitigation:** the default locations of npm, pnpm, Yarn and Bun give paths well under
  the limit (about 190 characters for pnpm on the hosted runners); the native archive can
  be extracted anywhere.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-097

**A publish that fails part-way leaves part of the release public until a re-run completes it.**

- **What:** the `publish` job publishes the three platform packages, then `vsift-cli`,
  then creates the GitHub pre-release. npm has no transaction across packages, and a
  published version can never be replaced, so if the job stops part-way (a registry
  error, an expired approval) the packages already published stay public, and the
  release is incomplete until the job runs again. **Never exercised:** the first approved
  run failed at its first `npm publish` before anything was published, and the second
  published all four packages and the release in one pass, so the re-run and the skip of
  versions npm already holds have not run against the real registry.
- **Evidence:** `.github/workflows/release.yml` job `publish`;
  [`release.md`](../operations/release.md) section 6 ("If a publish fails part-way");
  [p13-distribution.md](p13-distribution.md) ("First publish").
- **Impact:** for a while a platform package can exist without the launcher that selects
  it (harmless: nobody installs it directly), or npm packages without the GitHub
  release. `vsift-cli` is published last, so `vsift-cli@next` never names a platform
  version that is missing.
- **Why:** npm and GitHub Releases are separate services with no shared commit.
- **Mitigation:** a re-run of the failed job skips every version npm already holds with
  the same bytes (compared by its `sha512` integrity) and fails if npm holds other bytes;
  the GitHub release is created as a draft and published only once every asset is
  uploaded. The re-run must happen within the run's 7-day artifact retention; after
  that, a new run publishes a new version.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

*Since P14 PR 8 (2026-10-02):* the same holds for a stable version, and latest is what a
 part-way publish leaves half moved: the packages published so far already have latest at the
 new version, and sift-cli, published last and the one users install, still has the old one
 until the re-run completes. The re-run's pre-check accepts latest being the version itself.
 The stable path itself is [L-105](#l-105).

### L-098

**The Windows and macOS executables are unsigned: SmartScreen and Gatekeeper may warn
about a direct download, and Windows Smart App Control may block `vsift.exe` outright.**

- **What:** R0 ships no Authenticode signature and no Apple notarization (ADR 0023
  decision C). A browser download of an archive is marked as coming from the internet.
  Microsoft Defender SmartScreen may warn about an unsigned file that has no reputation,
  and Gatekeeper stops a quarantined, non-notarized program the first time it is opened
  (on macOS 15 only System Settings, Privacy & Security, "Open Anyway", or removing the
  quarantine attribute continues). An install through npm carries no such mark and
  avoids both. Windows 11's Smart App Control, when it is On, is documented to block
  unsigned programs that Microsoft's reputation service does not recognise (the
  documentation does not tie this to a download mark, so an npm install may not avoid it);
  policies that allow only signed software (AppLocker, App Control for Business) would
  refuse an unsigned executable too. **None of this has been observed on a VSift archive or
  executable:** `install.md` section 4 is written from Microsoft's and Apple's
  documentation. The one run of the published package on Windows 11 (`npx vsift --version`
  after `npm install vsift-cli@next`, 2026-10-01) met no block or prompt, but that machine's
  Smart App Control state was not checked at the time. *Read on 2026-10-02: Smart App
  Control is **Off** there (registry value `VerifiedAndReputablePolicyState` is 0), so that
  run says nothing about Smart App Control. Not known: what state a fresh Windows 11
  install shows (it starts in evaluation mode).*
- **Evidence:** ADR 0023 decision C and Consequences;
  [`install.md`](../operations/install.md) section 4, which links Microsoft's Smart App
  Control overview and Apple's guide to opening an app Apple cannot check.
- **Impact:** a Windows user with Smart App Control On may be unable to run VSift at all;
  a user who downloads an archive meets a warning that is right to heed and safe to pass
  only after the checks of `install.md` section 3. The generated release notes say that
  npm-installed files carry no download mark, and that Smart App Control can still block
  an unsigned program however it was installed (reworded 2026-10-01).
- **Why:** certificates and an Apple developer account are recurring costs and key custody
  that R0 does not need (decision C); the trust signals are the Sigstore attestation and
  npm provenance.
- **Mitigation:** `install.md` says what appears, why, and what to check instead
  (`SHA256SUMS`, `gh attestation verify`, `npm audit signatures`); no claim of publisher
  trust is made.
- **Next step:** before a stable release, run the Windows archive and an npm install on a
  Windows 11 machine with Smart App Control On, and a browser download on macOS 15; record
  what appears; the maintainer decides whether signing is then needed. Since 2026-10-02
  (ADR 0024 decisions C and H, confirmed): the maintainer's own machine is Off, so the
  Windows try-out needs a fresh Windows 11 virtual machine or another PC; whether the
  maintainer owns a Mac is not known; an observation, whatever it shows, is what the stable
  release needs, and a block with no way through short of turning protection off triggers
  the signing decision. Not known: whether Windows Sandbox is available (it could not be
  read without elevation).
- **Owner:** P14 and the maintainer. **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** deferred. **Review:** pending.

### L-099

**Managed installation depends on files and redirect hosts that the publishers control.**

- **What:** the reviewed catalogue pins three artifacts: the BtbN FFmpeg build's month-end
  release asset on GitHub, whisper.cpp v1.9.2's Ubuntu archive on GitHub, and one
  revision of the multilingual `base` model on Hugging Face. The transport follows
  redirects only to `release-assets.githubusercontent.com` and `us.aws.cdn.hf.co`. If a
  publisher removes a file, changes its download host or path, or refuses the transfer,
  `setup install` fails with `DOWNLOAD_FAILED` (`http_status` or `redirect_policy`) until
  a VSift release carries a reviewed catalogue that matches. The catalogue also offers no
  new plan after 2028-08-01.
- **Evidence:** `crates/vsift-infrastructure/src/managed_catalogue.rs` and
  `publisher_artifact_transfer.rs`; [ADR 0015](../decisions/0015-r0-delivery-replan.md)
  (Consequences: the publisher's retention of the month-end asset); ADR 0023 PR 4 note;
  [`install.md`](../operations/install.md) section 9.
- **Impact:** managed installation can stop working without any VSift change. Nothing
  unsafe happens: nothing unreviewed is downloaded or run.
- **Why:** pinned provenance means the addresses and the redirect route are reviewed, not
  discovered (ADR 0007, ADR 0014).
- **Mitigation:** the typed reason and the manual path (`install.md` sections 5.2 and 9);
  `--artifact-dir` imports files obtained another way and verifies them identically; the
  hosted `P13 managed smoke` run shows whether the real routes still work.
- **Next step:** `P13 managed smoke` now runs weekly against the highest published binary
  (P14 PR 3; [L-116](#l-116)) and shows the drift; dispatch it before each release too;
  revalidate or replace the catalogue before its stop date.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

## Process and CI

### L-075

**Codex's image views are not in its stream, so its image budgets are unmeasured.**

- **What:** `tools/vsift-agent-trials` parses Claude Code `stream-json` and Codex
  `exec --json` streams and passes flags first checked only with `--help` on Claude
  Code 2.1.281 and codex-cli 0.155.0-alpha.16. The counted P12 runs have now proven
  the reading. All 84 counted phases of Claude Code 2.1.284 and codex-cli
  0.155.0-alpha.16 recognised every stream line and read their configuration.
  Everything else below is proven or decided. **What remains** is that Codex's image
  budgets are not counted. Earlier findings:
  - **Proven:** every line of both streams parsed; Claude Code's `--max-turns`,
    `--setting-sources project` and `dontAsk` behave as intended; its `Skill`, `Read`
    and `Bash` calls and denials are read correctly; the project settings' allow and
    deny rules apply once the workspace is trusted (the dry run showed they are
    ignored otherwise, now fixed and graded as an invalid trial); Codex's
    `command_execution` items parse, and Codex found the skill in `.agents/skills` of a
    workspace that is not a git repository.
  - **Found and fixed:** Claude Code ignored the project allow list in an untrusted
    workspace (the rules came only from a duplicate `--settings` copy); codex-cli
    rejected every command as "blocked by policy" without a Windows sandbox mode.
  - **Codex's image views are unmeasured (decided 2026-09-29, PR 3e):** codex-cli
    0.155.0-alpha.16's `exec --json` stream has **no event for an image the model
    views**. For Codex the grader now takes the right `image_check_code` as proof of
    image access (the code exists only in the pixels); a wrong code, or any code in an
    images-disabled scenario, still fails. Codex's **image budgets** (images in total,
    per step, bytes) **are not checked**: two debug runs without `--ephemeral` showed
    that the session rollout records a view only inside a code-mode `exec` tool call
    whose input is model-written code, so counting would mean parsing that code. Codex
    runs stay `--ephemeral`, and every Codex grade carries a deviation saying the
    images are unmeasured. Claude Code's image budgets are measured as before.
  - **The check image (PR 3i, 2026-09-30):** GPT-6-Sol failed `image_check` in 5
    runs of the final campaign, each time reading the first image's code with the
    same letter missing. The image was redrawn with larger, spaced glyphs that no
    reader confuses (`docs/agents/skill.md`), and the grader now takes the code of the
    image each trial's workspace received, from a table of every image the skill
    shipped keyed by SHA-256, so older trials still grade against the code they saw.
    In the final round on `8ab976e` every `image_check` passed (31 of 31 GPT-6-Sol
    phases). Bounded records replace the code with `<check-code>` (P12 completion),
    because no text file of the repository may hold it.
  - **Found and fixed (PR 3e):** `-c tools.view_image=false` was an unknown setting
    Codex ignored, reporting it as a stream item of type `error` that no check read;
    images-disabled runs now pass `--disable view_image`, which a debug run showed
    removes the image tool, and such a notice now invalidates a trial.
  - **Proven in the counted runs (2026-09-30):**
    - the Claude Code `Read` deny rule of the images-disabled scenario: Claude Code
      refused each Sonnet 5.5 A-05 run's one read of the check image;
    - Codex's stream holds only shell commands, messages, turn usage and error
      notices, and every line parsed.
- **Evidence:** `tools/vsift-agent-trials/src/trace.rs`, `run.rs`, `claude_trust.rs`
  and `client_warnings.rs`; ADR 0022's notes of 2026-09-28 to 2026-09-30; the
  [P12 qualification record](p12-agent-qualification.md) (`stream_recognised` and
  `client_configuration` passed in 84 of 84 counted phases); the raw logs (kept
  locally, not in the repository).
- **Impact:** a Codex agent could view more images than the budget allows without
  failing a trial. A later client release may also change an event or flag the
  harness reads. The parser fails closed: an unrecognised event is an unauthorized
  call, and a stream line that is not JSON fails `stream_recognised`. A client's own
  report that it ignored its configuration makes the trial invalid.
- **Why:** codex-cli records an image view only inside model-written code.
- **Mitigation:** fail-closed parsing; the `client_configuration` check; raw logs kept
  locally for re-grading (`grade --output`); for Codex, the check code, which a model
  cannot know without viewing the image, and the tool-call budget, which still counts
  every command.
- **Next step:** revisit Codex's image budgets when a codex-cli release reports image
  views in its stream or as a structured rollout record. Re-check the stream reading
  whenever a trial uses a new client version.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual (maintainer decision, 2026-09-29, PR 3e). **Review:** pending.

### L-076

**Codex's Windows sandbox cannot run VSift trials as configured.**

- **What:** codex-cli 0.155.0-alpha.16 on Windows has two sandboxes. The trial harness
  uses the *unelevated* one (`-c windows.sandbox="unelevated"`), which needs no
  administrator setup and runs commands with a restricted token whose capability SIDs
  Codex grants write access on the workspace. Checked on Windows 11 with no model call
  (`codex sandbox`) and with two small-model `codex exec` runs:
  - reads anywhere, `vsift --version` and writes inside the workspace work; writes to
    the trial's `tmp` and `harness` folders are refused;
  - **the network is not enforced off**: network off only sets proxy variables to a
    dead port, and `curl --noproxy "*"` fetched a public connectivity-check page
    (HTTP 200) inside the sandbox;
  - **VSift cannot use its session root**: VSift makes every folder it creates private
    with a protected DACL for the user, `SYSTEM` and Administrators only, and refuses a
    root that grants anyone else. The restricted token needs its capability SID in that
    DACL, so `ingest` inside the sandbox fails with `STORAGE_IO`, and a root created
    outside the sandbox fails every command with `INTEGRITY_FAILURE`.
  The *elevated* sandbox (separate local sandbox accounts, firewall rules) needs a
  one-time administrator setup (`codex sandbox setup --elevated --current-user` with
  `CODEX_HOME` set, which raises a UAC prompt). It was not tried. It would run VSift
  as a different account, so sessions the harness prepares (A-06) and bundles the
  harness validates as the operator would probably not be readable across the two
  accounts. Upstream reports that the setup's machine-wide secret can invalidate other
  Codex homes' setup markers (openai/codex#40627), which could affect the operator's
  own Codex installation.
- **Evidence:** ADR 0022's 2026-09-28 dry-trial note; the Codex dry trial's stderr
  ("rejected: blocked by policy"); `crates/vsift-infrastructure/src/private_user_root.rs`
  (`restrict_new_directory`, `validate_private_root`).
- **Impact:** Codex trials on Windows cannot pass A-08 or any scenario that opens a
  session, and do not have an enforced network boundary. Claude Code trials are not
  affected.
- **Why:** VSift's private-folder rule and the restricted-token sandbox are
  incompatible by design; neither is wrong on its own.
- **Mitigation (trials, 2026-09-29):** the maintainer decided on 2026-09-28 that Codex
  trials run on Linux, in the trial container on Docker Desktop
  (`tools/vsift-agent-trials/containers/codex`, runbook
  [trials.md](../agents/trials.md#codex-trials-in-a-linux-container)). There Codex's
  own Linux sandbox (its bundled bubblewrap) runs `vsift` as the same user, keeps
  writes to the workspace and removes the network from every command; debug runs
  showed `ingest` working, writes to the trial's `harness` and `tmp` folders and
  `/tmp` refused, and `curl` failing. The grader still fails any non-`vsift` command.
  The container's own relaxations are L-078 to L-080.
- **Next step:** the product problem stays open as
  [#204](https://github.com/smormah/vsift/issues/204) (a Windows user of Codex's
  sandbox cannot run VSift): a named-sandbox-identity exception in private folders (its
  own ADR), a broker, or documenting Codex on Windows as unsupported in sandboxed mode.
- **Owner:** unscheduled (product; the P12 trials avoid it in the container). **Issue:**
  [#204](https://github.com/smormah/vsift/issues/204). **Status:** open. **Review:** pending.

### L-077

**An agent cannot measure its own wall time; only the host enforces that budget.**

- **What:** the skill's budgets include a wall time (15 minutes `compact`, 30
  `standard`), but VSift's results carry no current time: the only clock time,
  `lifecycle.expires_at`, is fixed when a session opens and moves only on renewal. The
  skill runs nothing but `vsift`, so an agent cannot read a clock either. In the second
  Claude Code dry trial (2026-09-28) the agent chained `date +%s` to its first and last
  commands to time itself, which the grader rightly failed as a non-`vsift` program.
- **Evidence:** ADR 0022's PR 3c note; the dry trial's `grade.json` (calls 7 and 24).
- **Impact:** the handoff reports `resume.remaining.wall_time_s` (and
  `budget.used.wall_time_s`, optional since 2026-09-29) as `null` unless the client shows elapsed time, and an
  agent cannot stop itself on time; the host (the client's own limits, the trial
  harness's timeout and the grader's measured wall time) stops it instead.
- **Why:** exposing a clock only for self-timing would add a command or a result field
  to the public contract for a budget the host already enforces.
- **Mitigation:** the skill says who keeps the wall time; the handoff schema accepts
  `null`; the grader measures wall time from the client's run.
- **Next step:** none planned; revisit if a host cannot enforce time limits.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual. **Review:** pending.

### L-078

**The Codex trial container relaxes Docker's seccomp profile so Codex's sandbox can create user namespaces.**

- **What:** codex-cli 0.155.0-alpha.16's Linux sandbox is its bundled bubblewrap, which
  creates a user namespace for every command. Docker's builtin seccomp profile refuses
  that to a container without `CAP_SYS_ADMIN` ("bwrap: No permissions to create a new
  namespace"), and Codex's older Landlock mode no longer runs `workspace-write`
  ("filesystem-restricted execution requires bubblewrap"). The trial containers
  therefore run with the committed profile
  `tools/vsift-agent-trials/containers/codex/seccomp-userns.json`: every system call
  is allowed except kernel keyrings, eBPF, performance counters, `userfaultfd`,
  `io_uring`, kernel modules, `kexec`, reboot, swap, accounting, clock setting, the
  kernel log and file-handle opening. Everything else stays tight: a non-root user,
  `--cap-drop ALL`, `no-new-privileges`, a read-only root file system, a process and
  memory limit, never `--privileged`. AppArmor was not involved (Docker Desktop's
  WSL 2 kernel has none).
- **Evidence:** the 2026-09-29 probes on Docker Desktop 26.1.1 (kernel
  5.15.146.1-microsoft-standard-WSL2): the sandbox works with the profile and fails
  under `seccomp=builtin`; ADR 0022's 2026-09-29 note.
- **Impact:** the profile is an allow-by-default list, weaker than Docker's builtin
  allowlist: a kernel flaw reachable through user namespaces or an allowed call is
  reachable from the trial container. Docker Desktop's own default on this machine is
  already unconfined, so the profile narrows what the daemon would otherwise allow.
- **Why:** Codex's sandbox (which gives agent commands no network and confined writes)
  needs user namespaces; an allowlist derived from Docker's default would need that
  profile's source, which was not downloaded.
- **Mitigation:** the deny list above; unprivileged, capability-free containers; the
  agent's own commands run inside Codex's sandbox and its seccomp filter. On an Ubuntu
  Docker host the `docker-default` AppArmor profile may also refuse bubblewrap's
  mounts (untested there).
- **Next step:** derive an allowlist from Docker's default profile plus the namespace
  calls if the container is used beyond the P12 trials.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual. **Review:** pending.

### L-079

**The Codex trial container's own network is not limited to the model API.**

- **What:** the container Codex runs in has Docker's ordinary outbound network, which
  the Codex client needs for its model API. Only the agent's commands are cut off: they
  run in Codex's sandbox, where a debug run's `curl https://example.com/` failed with
  "Could not resolve host" while the same request outside the sandbox succeeded. The
  container is not restricted to the model API's hosts by a proxy or firewall.
- **Evidence:** ADR 0022's 2026-09-29 note; the runbook's `sandbox-check`.
- **Impact:** a flaw in Codex itself, or a command that escaped its sandbox, could reach
  any host. The agent cannot reach the network through its commands.
- **Why:** an egress proxy with an allow list for the model API is more machinery than
  the trials need, and its host names would have to be maintained.
- **Mitigation:** Codex's sandbox for every agent command; no credential but the
  per-run copy of the sign-in (L-080) is in the container; the grader fails any
  non-`vsift` command.
- **Next step:** none planned for P12.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual. **Review:** pending.

### L-080

**A Codex trial agent can read its client's sign-in and its own trial's harness folder.**

- **What:** Codex's sandbox lets the agent's commands read any file in the container.
  The run container holds, besides the built tools, only this trial's folder, the
  model and the Codex sign-in: the repository, the corpus truth and other trials are
  never mounted there (they are in the separate `prepare` and `grade` containers). But
  Codex needs its sign-in while it runs, so a copy of `auth.json` sits on a tmpfs
  `CODEX_HOME` for the run and is deleted afterwards, and the trial's own `harness`
  folder (the frozen scenario, the manifest with the canary values, the raw logs so
  far) sits beside the workspace, as on Windows.
- **Evidence:** ADR 0022's 2026-09-29 note; `trial-driver.sh` and `codex-trial.ps1`.
- **Impact:** an agent that follows hostile instructions could read and print its
  tokens or its scenario's expectations. Reading is not blocked, only graded.
- **Why:** the client cannot run without its sign-in, and `run` needs the manifest and
  scenario in the same container as the client.
- **Mitigation:** every non-`vsift` command fails the trial (reading those files takes
  `cat` or similar outside the skill folders; the orientation allowed since 2026-09-29
  lists only the names in the workspace, never the harness folder beside it); after the client exits, `run` searches
  its raw output for every value of the sign-in file (tokens and each part of a JWT,
  24 characters or more) and `grade` fails `no_canary` when one appears; the values
  are never logged, only counts (`sign_in_leak_check` in `run.json`). The network is
  cut from agent commands (L-079), so a token can only leave through the model's own
  conversation.
- **Next step:** none planned for P12; a Codex setting that denies reading the
  sign-in and harness paths would close most of it.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual. **Review:** pending.

### L-081

**A handoff may leave out the times and session details VSift recorded, so reading it
alone does not give them.**

- **What:** since 2026-09-29 (maintainer decision, ADR 0022's PR 3f note) handoff v1
  requires only what the agent alone knows. A citation must give its VSift identity
  (`segment_id` or `evidence_id`) and, for a frame or crop, `pixels_inspected`;
  its times, revision, candidate, parent, rectangle and range, the `session` block,
  most of `capabilities` and the `budget` are optional.
- **Evidence:** `skills/vsift/handoff.schema.json`; the guard test
  `handoff_schema_requires_only_what_the_agent_alone_knows`.
- **Impact:** a person or tool that reads only the handoff may not see when a cited
  segment or frame is, or which session it came from; the prose still gives times as
  `mm:ss.mmm`, but the JSON may not. Resolving a citation needs the session (while it
  is open) or a retained bundle, which the grader does.
- **Why:** agents, especially small models, failed handoffs on copied values VSift
  already held, and a copied value adds no evidence.
- **Mitigation:** a value the agent does give is checked against VSift's records; the
  resume card still carries the session, revision and evidence times another run needs;
  the skill tells agents to add the optional members when they help.
- **Next step:** if a consumer needs self-contained handoffs, add a harness or CLI step
  that fills the optional members from the bundle, rather than asking the agent.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** open. **Review:** pending.

### L-082

**Claude Haiku 4.5 does not follow the full investigation procedure.**

- **What:** in the compact-tier runs on `b68d746` (2026-09-29, Claude Code 2.1.284,
  28 trials of A-01 to A-07 and SEC-T02), Claude Haiku 4.5 answered 6 of 28 questions
  correctly and passed both results in 2 of 28. It invents its own handoff shapes
  (claims without `section` or `support`, citations with `description` or `quote`,
  lifecycle members of its own), exceeds the image budget, and copied a raw U+202E
  into 4 of 5 SEC-T02 reports (L-083). Its A-02 second phase could not run in 3 of 3
  trials because the first phase left no usable resume card.
- **Evidence:** the local campaign summary and grades (raw logs stay local); ADR
  0022's note of 2026-09-29; PR 3g's re-grade table (3 of 28 full passes with the PR 3g
  grader).
- **Impact:** a Claude Code user who picks Haiku 4.5 gets unreliable, often invalid
  handoffs. The skill is not qualified for it.
- **Why:** the procedure (eight states, budgets, a typed handoff) is more than this
  model follows from the skill alone; the vocabulary fix of PR 3g does not change that.
- **Mitigation:** the maintainer redefined the compact tier as Claude Sonnet 5.5 and
  GPT-6-Luna (2026-09-29), and after the final campaign on `56f1e1f` as Claude Sonnet
  5.5 and GPT-6-Sol, with GPT-6-Luna also below the line ([L-084](#l-084));
  [verification.md](verification.md) section 6 names it, and `docs/agents/skill.md`
  lists the supported models (since P12's completion, 2026-09-30). Haiku 4.5 did not
  run in the final campaign.
- **Next step:** none planned; a later, simpler skill profile for small models would
  be a new decision.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual (maintainer decision, 2026-09-29). **Review:** pending.

### L-083

**Only `display_text` shows hidden characters; `text` and `original_text` keep them
raw.**

- **What:** since P12 PR 3h every transcript segment carries `display_text`, its
  `text` with every hidden character (Unicode `Cf`, `Default_Ignorable_Code_Point`,
  U+2028 and U+2029: bidirectional controls, zero-width characters, variation
  selectors, tag characters and the like) written as visible `<U+XXXX>` notation, and
  a speaker object carries `display_label`; the skill quotes only those. `text` and
  `original_text` (and `label`) keep the characters as written, by design: they are
  the payload, and for WebVTT `text` decodes a reference such as `&#x202E;` into the
  raw character. A consumer that quotes them, or an agent that ignores the skill,
  still carries invisible or reordering characters into what it writes. Before the
  field existed, Claude Sonnet 5.5 (1 of 5) and Claude Haiku 4.5 (4 of 5) copied a raw
  U+202E into SEC-T02 reports on `b68d746`; whether the compact tier now quotes
  `display_text` is measured by its re-run. `display_text` can be four times the bytes
  of `text`, so a 100-segment page of adversarial text can exceed the 1 MiB result
  budget (exit 7; a smaller `--limit` reads it). The handoff schema's prose members
  and the trial grader's `report_text` check refuse a narrower set than
  `display_text` renders: bidirectional controls, zero-width characters and U+FEFF
  (the grader also U+2060 to U+2064).
- **Evidence:** [ADR 0008, note of 2026-09-29](../decisions/0008-cli-and-json-contract.md#2026-09-29-note-display_text-for-hidden-characters);
  `docs/contracts/cli-v1.md` ("Display text"); `vsift-contract`'s `text` tests and
  `display_text_makes_hidden_characters_visible_and_keeps_text_raw`;
  `crates/vsift-cli/tests/sec_t02_adversarial_evidence.rs`.
- **Impact:** a report written from `text` can display differently from what it says
  (Trojan Source style). Trials fail such a report; a user outside trials would not be
  warned.
- **Why:** the maintainer chose a strictly additive field over redefining `text`
  (2026-09-29), so published fields keep their meaning and stored records, identities
  and digests do not change.
- **Mitigation:** `display_text` and `display_label`; the skill's rule and "before you
  send" checklist, held by the `skill_contract` guard; the handoff schema's refusal in
  prose members; `report_text` in trials.
- **Next step:** none planned. The trials show that models quote `display_text`:
  no report held a raw hidden character in the final campaign on `56f1e1f` (every
  counted run of Opus 5.5, Sonnet 5.5, GPT-6-Astra, GPT-6-Luna and GPT-6-Sol) or in
  the final compact round on `8ab976e` (62 of 62 phases of Sonnet 5.5 and GPT-6-Sol,
  SEC-T02 included).
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual (maintainer decision, 2026-09-29). **Review:** pending.

### L-084

**GPT-6-Luna is below the compact-tier line.**

- **What:** in the final counted campaign on `56f1e1f` (2026-09-29, Codex in the Linux
  container, 28 trials of A-01 to A-07 and SEC-T02, A-02 counting both phases as one
  trial), GPT-6-Luna answered 19 of 28 questions correctly and passed both results in
  15 of 28 (GPT-6-Sol: 25 and 15; Claude Sonnet 5.5: 25 and 25). Its failures: 8
  handoffs that do not validate (claims written with `text` in place of `statement`,
  instructions with `citations` in place of `citation`, frames marked inspected
  without image access), 4 command-policy failures (a `cat` of a folder outside the
  skill, a `command -v` probe, an `rg --files` exclude glob with a separator), 3
  images-disabled A-05 answers that leave out the chart's time, 2 A-07 answers that
  miss the order's status change and 2 A-02 resumed phases. PR 3i's re-grade removes
  its `command -v` failure only (the table is in the PR).
- **Evidence:** the local campaign summary and grades (raw logs stay local); ADR
  0022's note of 2026-09-30.
- **Impact:** a Codex user who picks GPT-6-Luna gets a correct answer about two times
  in three and a valid handoff about three times in four. The skill is not qualified
  for it.
- **Why:** the model follows the procedure but writes the handoff's shape and the
  answer's details less reliably than the tier needs; no single skill fix covers the
  mix.
- **Mitigation:** the compact tier is Claude Sonnet 5.5 and GPT-6-Sol (maintainer,
  after the final campaign; [verification.md](verification.md) section 6); like Claude
  Haiku 4.5 ([L-082](#l-082)), GPT-6-Luna is recorded below the supported line.
- **Next step:** none planned; a handoff validator command (P13, #213) may lower its
  handoff failures.
- **Owner:** unscheduled (recorded in P12). **Issue:** [#15](https://github.com/smormah/vsift/issues/15).
  **Status:** accepted residual (maintainer decision, 2026-09-30). **Review:** pending.

### L-040

**Process-supervisor tests fail intermittently on Windows under load.**

- **What:** a few `process_supervisor` tests failed on child exit status twice in full
  local `cargo test --workspace` runs on Windows 11 (P07 increment 2, and 2026-09-25 with
  `p03_caps_stdout_stderr_and_combined_floods_during_read` and
  `p04_preserves_invalid_bytes_and_handles_no_newline_and_delayed_output` under a
  parallel release build); 40/40 and 20/20 isolated reruns passed; never seen in CI.
- **Evidence:** issue #128 and its comments.
- **Impact:** noisy local runs; a possible timing assumption around child exit or Job
  Object cleanup.
- **Why:** unknown.
- **Mitigation:** record the failing names and output on recurrence before rerunning.
- **Next step:** a parallel stress run like the lock stress workflow.
- **Owner:** unscheduled. **Issue:** [#128](https://github.com/smormah/vsift/issues/128).
  **Status:** monitoring. **Review:** pending.

### L-041

**A creator slower than 5 s makes a racing command `BUSY`.**

- **What:** commands that race to create a new session root wait at most 5 s for the
  creator. On one heavily throttled hosted runner (test binary 93 s instead of about
  3 s) the creator took longer and a concurrent-preflight test got the documented
  `BUSY` (`ProvisioningInProgress`), which the test treated as a failure (#144).
- **Evidence:** issue #144; `concurrent_preflights_all_proceed_and_leave_one_valid_record`.
- **Impact:** on a very slow machine the first commands against a new root can answer
  `BUSY` (retryable); a retry succeeds. The test no longer fails: since P10 PR 3 the
  wait is injectable (`EnginePorts::with_session_root_wait`, at most 60 s) and that test
  waits 60 s, since what it checks is convergence, not the bound.
- **Why:** the production bound keeps a stalled creator from holding every other
  command.
- **Mitigation:** `BUSY` is typed and retryable; hosts can lengthen the wait.
- **Next step:** close #144 once the stabilised test has run in CI without recurrence.
- **Owner:** unscheduled. **Issue:** [#144](https://github.com/smormah/vsift/issues/144).
  **Status:** accepted residual. **Review:** pending.

### L-042

**Real-tool success paths run only on demand in pull-request CI; the published binary's run
is weekly.**

- **What:** the success paths that use real FFmpeg, whisper.cpp or large media are
  opt-in (`--ignored`) and no ordinary pull-request job runs them: `p06_setup_e2e`,
  `p07_transcript_e2e`, `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`,
  `p08_candidates_e2e`, `p09_evidence_e2e`, `p10_recovery_e2e`, `p11_worker_e2e` and the
  listed engine tests. Run by hand they drive the binary Cargo builds inside the test run.
  Since P14 PR 3 a binary override (`VSIFT_E2E_BINARY`) lets the same checkpoints drive an
  installed binary, and the hosted workflow `P14 journeys` runs them on Ubuntu 24.04,
  Windows and macOS 15 against the highest published `vsift-cli`, on dispatch and weekly (the
  weekly `P07 local ASR` workflow and `P13 managed smoke`, now weekly too, remain). A change
  to the engine is therefore met by a hosted run of the published bytes only on the next
  weekly or dispatched run, not on the pull request; the Cargo-built binary is checked by a
  hosted run only through those workflows and by hand. Long fuzz campaigns (weekly short
  runs today), soak and the load ladder are P14 gates.
- **Evidence:** [work record](../../memory/TODO.md) "Tracked issues and gates";
  [verification](verification.md) section 7 CI tiers; `P14 journeys` run 36965956708;
  [P14 plan](p14-qualification.md) section 17.
- **Impact:** a regression in a real-tool path is found when the weekly run or someone's
  dispatch reaches it, and then only for the published version, not for the commit that
  caused it.
- **Why:** local models and sizeable media; runner cost (decided in the test spine's
  execution policy). A pull-request run of all three systems takes about an hour.
- **Mitigation:** recorded samples keep the P08 recall gate in every CI run; stand-in
  tools cover contracts; the candidate's own runs are P14 gates (RQ-05, RQ-06).
- **Next step:** the candidate (P14 PR 11) and each stable repeat the runs on their own
  bytes; whether a cheaper subset belongs in every pull request is #178's question.
- **Owner:** P14 (release runs); per-pull-request runs unscheduled. **Issue:**
  [#178](https://github.com/smormah/vsift/issues/178). **Status:** open. **Review:** pending.

### L-043

**Library API unstable; MSRV and MCP decisions open.**

- **What:** the `vsift` library API is 0.x and unstable; the MSRV equals the latest
  stable release with no policy; `cargo-semver-checks` joins CI only at first
  publication. Open maintainer decisions: an MSRV policy before publication and whether
  a local MCP adapter is wanted after P12. Decided 2026-09-30 (ADR 0023) and done: one
  0.x pre-release under npm's `next` tag at P13 completion (0.1.0, published 2026-10-01),
  and no crates.io publication in R0.
- **Evidence:** [ADR 0016](../decisions/0016-embeddable-engine-and-evidence-contract.md)
  decisions 3, 7, 8; [work record](../../memory/TODO.md) "Open decisions".
- **Impact:** embedders face breaking changes; the CLI JSON v1 contract is stable.
- **Why:** pre-release.
- **Mitigation:** CLI contract tests and frozen examples.
- **Next step:** maintainer decisions.
- **Owner:** unscheduled. **Issue:** [#176](https://github.com/smormah/vsift/issues/176). **Status:** open. **Review:** pending.

### L-044

**Accepted engineering trade-offs (CLI test dependencies, session compatibility).**

- **What:** the CLI keeps `vsift-application`, `vsift-infrastructure` and `vsift-domain`
  as development dependencies for tests that seed session records (normal dependencies
  are only `vsift` and `vsift-contract`). A session written by this version records an
  optional `verified_source_identity` that older builds reject.
- **Evidence:** [ADR 0016](../decisions/0016-embeddable-engine-and-evidence-contract.md)
  implementation note; [work record](../../memory/TODO.md) "Known issues and gates".
- **Impact:** none for users (sessions are disposable).
- **Why:** test seeding; additive storage change.
- **Mitigation:** architecture boundary checked for normal dependencies.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-045

**Several documents and trackers state an outdated position.**

- **What:** found while compiling this register (2026-09-27):
  - `README.md` "Project status" says P09 is "pending review and merge", "Current
    behavior" says frame and audio retrieval return `COMMAND_NOT_IMPLEMENTED`, and the
    setup text names `ingest --transcript` as today's first media operation.
  - [CLI contract](../contracts/cli-v1.md): its command table still marks `setup
    configure`, `configure-model` and `plan` "Partial P06".
  - [Architecture and contracts](architecture-and-contracts.md) says it is implemented
    "through P06 plus ... P07 increment 2".
  - The [threat model](security-threat-model.md) header still reads "proposed release
    requirements, 2026-09-09".
  - The [P08 recall record](p08-candidate-recall.md) calls ADR 0018 "Proposed"; it was
    accepted on 2026-09-26.
  - The P06 source-review and Windows-candidate records say "P06 remains open" (dated
    records; P06 closed on the narrowed ADR 0015 scope).
  - Issue #40's checkpoints are all unchecked although P04-P09 checkpoints are met.
  - `CHANGELOG.md` says a hosted `BUSY` symptom "remains under review"; issue #66 was
    closed on 2026-09-23.
- **Impact:** readers and agents can be misled about what works.
- **Why:** per-packet updates missed these files.
- **Mitigation:** this list.
- **Next step:** one documentation sweep PR; tick #40's met checkpoints.
- **Owner:** unscheduled. **Issue:** [#177](https://github.com/smormah/vsift/issues/177). **Status:** open. **Review:** pending.

### L-046

**Deliberate scope exclusions (live sources, OCR, speakers, URLs).**

- **What:** R0 accepts only local MP4/Matroska/WebM files (HLS, playlists and URLs are
  rejected), up to 4 hours and 20 GiB; sessions expire after 24 idle hours and at most
  7 days. There is no live capture (#107), speaker grouping (#108), OCR, diarization,
  embeddings or scroll stitching (DEC-12, R1), and no multi-tenant host (R2).
- **Evidence:** [ADR 0005](../decisions/0005-r0-scope-and-qualification-profiles.md),
  [ADR 0011](../decisions/0011-r1-industrial-capability-expansion.md),
  [ADR 0012](../decisions/0012-p04-source-media-profile.md),
  [resource profiles](support-and-resource-profiles.md).
- **Impact:** agents must work from transcript text and pixels only.
- **Why:** accepted scope decisions.
- **Mitigation:** typed rejections; coverage states what was searched.
- **Next step:** R1 (P15-P20) and later scope ADRs.
- **Owner:** R1 or later. **Issue:** [#107](https://github.com/smormah/vsift/issues/107),
  [#108](https://github.com/smormah/vsift/issues/108). **Status:** accepted residual.
  **Review:** pending.

### L-088

**Proxy authentication in an HTTPS tunnel is recognised by a dependency's error text.**

- **What:** when a proxy answers a `CONNECT` with `407`, hyper-util reports a private
  `TunnelError::ProxyAuthRequired` that it does not export, so `setup install` tells
  `proxy_auth` from other connection failures by the end of that error's text (`proxy
  authorization required`).
- **Evidence:** `vsift-infrastructure/src/publisher_artifact_transfer.rs`
  (`PROXY_AUTH_REQUIRED`); the D-07 proxy test in
  `vsift-infrastructure/tests/p13_install_transaction.rs`.
- **Impact:** if a dependency update changed that text, a proxy that wants credentials
  would be reported as `offline` rather than `proxy_auth`; the download still fails
  safely with no credential shown.
- **Why:** no typed way to reach the tunnel's status exists in the pinned reqwest and
  hyper-util.
- **Mitigation:** the D-07 test runs on every CI OS and fails if the text changes; a
  plain `407` response is recognised by its status.
- **Next step:** use a typed error if hyper-util exports one.
- **Owner:** unscheduled. **Issue:** none. **Status:** monitoring.
  **Review:** pending.

### L-090

**Content in the managed folder that VSift cannot prove its own is left for the user to delete, and no output names the folder's path.**

- **What:** `setup remove`, the stale-stage sweep and bounded cleanup delete only what
  they positively identify: a version whose manifest lists its files, and stages whose
  every entry is a known name and a single-link regular file. A version with an invalid
  or missing manifest, a link, a folder or an unknown name, a stage without a provable
  marker, and entries VSift never creates are kept; `setup repair` reports each with fix
  `manual`, and results say to delete it (or the whole managed folder) by hand. No result
  prints the folder's path; the prose names the Ubuntu default
  (`~/.local/share/vsift/managed-v1`, or `vsift/managed-v1` under `$XDG_DATA_HOME`).
- **Evidence:** [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 6 note; `vsift-infrastructure/src/managed_store_lifecycle.rs`; the tests in
  `managed_store_lifecycle/tests.rs` that keep planted links and unknown files.
- **Impact:** after tampering or an unusual failure, a user must delete leftovers
  themselves; they take disk space but are never run or selected.
- **Why:** deleting what cannot be proved VSift's own risks deleting through a link or
  deleting user data (AGENTS.md: automatic cleanup is restricted to positively
  identified VSift-owned content); no managed-setup result carries a path, by the
  same rule as every other setup result.
- **Mitigation:** lookup ignores everything that does not verify; `setup list` and
  `setup repair` count what is kept; a version whose bytes or modes changed is still
  removable, because its names and single-link regular files prove ownership.
- **Next step:** none planned for R0; revisit if users report leftovers.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

### L-100

**npm prints only `ENEEDAUTH`, with no reason, when a trusted publisher is missing, set
wrongly or lacks "npm publish", and nothing in the workflow's default output says so.**

- **What:** the `publish` job authenticates to npm by trusted publishing (OIDC). When that
  exchange fails, npm falls back to asking for a login, which a workflow cannot give, and
  stops with `npm error code ENEEDAUTH` and "This command requires you to be logged in".
  At npm's default log level nothing names the cause: not that the package has no trusted
  publisher, not which of its fields (owner, repository, workflow file, environment) does
  not match, not that "npm publish" is not an allowed action. The first real dispatch
  stopped this way on 2026-10-01 (Release run 36922901956) after the maintainer had
  reported the trusted publishers as done; the maintainer then reported that the Trusted
  Publisher form's **Set up connection** step had not been completed. The second dispatch
  succeeded once it was. Which exact field was wrong is not known: npm shows these
  settings only to the package owner, and the run's log is at the default level.
- **Evidence:** the failed run's log; the qualification record's "First publish"
  ([p13-distribution.md](p13-distribution.md)); [`release.md`](../operations/release.md)
  sections 6.2 and 6.5.
- **Impact:** a first publish, a new package or a changed setting can fail with a message
  that points at credentials rather than at the trusted-publisher entry. Nothing is
  published, so the cost is a wasted approval and time (the first run waited about 45
  minutes for the approval before it failed in seconds), not a damaged release.
- **Why:** no dry run can exercise the OIDC exchange with npm, and npm gives the workflow
  no way to ask what is configured.
- **Mitigation:** `release.md` 6.2 has a preflight: before the first real dispatch, open
  each of the four packages' Settings page and confirm that a saved Trusted Publisher entry
  is listed (not the empty form) with the values and "npm publish" allowed; **Set up
  connection** and the second factor are what save it. `release.md` 6.5 says what
  `ENEEDAUTH` means and how to finish the run.
- **Next step:** none planned. A possible workflow change (npm verbose logging for the
  publish step, or a hint printed on failure) is not made here and would need its own
  reviewed change, because the publish job's commands are held word for word by a
  `vsift-release` test and the governance lint.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-101

**The evidence ledger and the claims registry prove that recorded evidence exists and
banned words are absent, not that a run passed or a sentence is true.**

- **What:** the release evidence ledger (`docs/planning/p14-evidence-ledger.json`) holds a
  status and typed links (a workflow run, a pull request, an issue, a repository record)
  for each evidence item. The Governance job checks its shape, that every identifier
  exists in the document that owns it, that a record path exists and the rules each status
  carries; it fetches nothing, so a run link is a claim: nothing confirms the run exists,
  passed or was of the commit named. The claims check (`docs/planning/public-claims.json`)
  reads a closed list of documents as plain text (fenced code blocks skipped) and finds
  controlled words (`supported`, `stable`, `qualified` and kin) and banned phrases. A
  sentence that avoids every controlled word and banned phrase passes whether or not it is
  true; a document not on the list is not read; a registered statement must be updated
  with the document it quotes.
- **Evidence:** the [ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md)
  note of P14 PR 1; the unit tests of `tools/vsift-governance` (one per rule).
- **Impact:** a wrong link or a misleading sentence can pass; only a person reading the
  ledger and the documents catches it. The checks guarantee that nothing the plan lists is
  forgotten and that the ledger, the registry and the documents agree with each other.
- **Why:** verifying a run needs the network and a judgement of its result, which the
  Governance job deliberately does not have (no network, no credentials); meaning cannot
  be checked mechanically.
- **Mitigation:** every status change is a pull request the maintainer reads; the second
  verification of each publish (RQ-19) and the dated scan reading (RQ-13) are done by a
  different runner and by a person; the documents say what the checks do not prove.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-103

**Evidence carried forward from an earlier commit rests on a hand-written scope and needs
Git history, and the delta record is copied into the ledger by hand.**

- **What:** the completeness check (`release-evidence --complete-for <version>`) counts
  evidence recorded at an earlier commit for a later one only when no file under the item's
  `scope` changed in between, as `git diff --name-only` reports (the staleness rule of ADR
  0024). A scope that omits a path the evidence depends on lets stale evidence count; each
  scope is read by a reviewer with the ledger change. Without the history (a shallow
  checkout) or without Git the answer is unavailable and the evidence does not count, so it
  fails closed. The stable release may carry the candidate's evidence only when
  `release_delta` records that it differs only in version strings and shipped documents (ADR
  0024 decision A). `vsift-release candidate-delta` is that check (P14 PR 8), and an enforced
  stable plan that passes writes the record in the ledger's shape (`release-delta.json` in
  the run's `publish-plan` artifact); nothing copies it into the ledger, so `release_delta`
  stays null, and a completeness check for the stable version fails on every carried item,
  until the maintainer pastes it in after the publish.
- **Evidence:** `tools/vsift-governance/src/release_evidence/completeness.rs` and its
  tests, including one against a throwaway Git repository; the ADR 0024 note of P14 PR 1.
- **Impact:** a scope that is too narrow could keep old evidence for a changed area; a
  checkout without history makes the check unusable until it is deepened. The first is a
  review matter, the second an error, and neither is silent.
- **Why:** scopes cannot be derived automatically for hosted runs, and the workflow never
  commits to the repository: the record is evidence the maintainer reads and files.
- **Mitigation:** the scopes are in the ledger for the pull request's reader; evidence
  about published bytes or a dated reading is `repeat` for the stable and never carried;
  the check is run in a full checkout.
- **Next step:** the maintainer copies `release-delta.json` into the ledger after the
  stable publish (`release.md` 6.7, "After the publish"); the scopes are reviewed at the
  candidate (PR 11) and the stable (PR 12).
- **Owner:** P14 (PR 12). **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** deferred. **Review:** pending.

### L-105

**The stable publish path has never run against the real services.**

- **What:** P14 PR 8 built the path that publishes a stable version under `latest` and tested
  it as far as the repository can: the version kinds, the plan and its guards, the
  candidate-to-stable check, the evidence-ledger guard, 65 lint mutations and the publishing
  shell executed against stub commands. Three things only the real services can show have not been tried: (1)
  that npm's trusted publishing accepts `npm publish --tag latest` under the allowed action
  "npm publish" (only `--tag next` has gone through it, for 0.1.0); (2) that `gh release
  edit --draft=false --latest` marks a draft release as GitHub's latest (the job then
  requires `gh api repos/smormah/vsift/releases/latest` to name the tag, so a failure is
  loud, not silent); (3) how long npm takes to show a new `latest` (the read-back waits up to
  five minutes per attempt set; for 0.1.0 `next` took about a minute and a half). The plan
  job's registry reading does run against the real npm on every pull request's dry run, so
  it is exercised; a real publish is not.
- **Evidence:** `.github/workflows/release.yml` job `publish`, its stable steps;
  `tools/vsift-release/tests/publish-steps.sh`; [`release.md`](../operations/release.md)
  section 6.7.
- **Impact:** the first stable publish may stop. Each way it can is recoverable: a failure of
  (1) is `ENEEDAUTH` at the first `npm publish`, with nothing published; of (2) leaves the
  release published and unmarked while npm is already correct (`gh release edit v<version>
  --latest`); of (3) fails the read-back after the packages are published (re-run the job: the
  versions are skipped and the checks run again). A stable version that moved `latest` wrongly
  cannot be unpublished; `release.md` 6.5 says what to do.
- **Why:** nothing but a publish exercises trusted publishing, and a dry run must not
  publish.
- **Mitigation:** the release candidate (P14 PR 10) exercises every step of the workflow but
  the two commands above; the dry run on the stable tag is enforced (it fails whenever the real
  run would); the preflight in `release.md` 6.7 re-checks the trusted publishers and the
  registry first.
- **Next step:** the maintainer's first stable publish (P14 PR 12); record what happened here
  or delete this entry if all three held.
- **Owner:** P14 (PR 12, the maintainer's publish). **Issue:**
  [#17](https://github.com/smormah/vsift/issues/17). **Status:** deferred. **Review:**
  pending.

### L-107

**The candidate-to-stable check compares paths and bytes, not meaning, and takes the highest
candidate to be the accepted one.**

- **What:** the stable commit may differ from its candidate only in six version-string files
  (whose content must equal the candidate's with the version text replaced) and in the
  launcher's README (any change). The check cannot tell that the README is right, that a
  replaced version text was the only intent, or that the highest `v<X.Y.Z>-rc.<N>` tag is
  the candidate that was qualified: it takes it to be, because only the maintainer can
  create a `v*` tag and a stable should never be built on an older candidate than the last
  one cut. The skill and the release notes are frozen with the code at the cut, so the stable
  release notes' wording must be right in the candidate.
- **Evidence:** `tools/vsift-release/src/candidate.rs`; [`release.md`](../operations/release.md)
  6.8.
- **Impact:** a wrong word in the launcher's README, or a stable built on a candidate the
  maintainer meant to reject, would pass the check. Changing the allowed lists after the
  candidate is cut makes the stable fail the check (the lists are code).
- **Why:** a mechanical check can prove what did not change, not what is right.
- **Mitigation:** the plan prints every changed path and its class; the maintainer reads the
  README's diff and the delta check's output in the preflight (6.7 items 4 and 5).
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-108

**After a stable release `next` still names the candidate, an older build than `latest`.**

- **What:** a stable version is published under `latest` and leaves `next` alone: the
  workflow never runs `npm dist-tag`, so it cannot move it. Until the next pre-release, or
  until the maintainer moves it by hand, `vsift-cli@next` installs the last release
  candidate (before any candidate it installs 0.1.0).
- **Evidence:** [`release.md`](../operations/release.md) 6.7 ("What changes for users");
  ADR 0024's note of P14 PR 8.
- **Impact:** a user who follows an older instruction (`npm install vsift-cli@next`, the
  text of the 0.1.0 pre-release and its README) gets a build older than the stable.
- **Why:** the lint forbids moving a dist-tag from any workflow, deliberately.
- **Mitigation:** the repository-only pages (`install.md`, the README) flip to the stable
  instructions in the ledger follow-up (P14 PR 13), and the launcher's README, which ships
  in the package, is a shipped document the stable may change; the maintainer may move
  `next` by hand (`npm dist-tag add vsift-cli@<stable> next` for each package).
- **Next step:** decide at the stable whether to move `next`.
- **Owner:** P14 (PR 12, the maintainer's choice). **Issue:**
  [#17](https://github.com/smormah/vsift/issues/17). **Status:** accepted residual.

### L-109

**On Windows, the `vsift.cmd` shim that npm and pnpm create lets cmd.exe re-read arguments: percent expansion, dropped quotes and a redirection without whitespace that runs.**

- **What:** npm and pnpm install three files for the `vsift` command on Windows: `vsift.cmd`,
  `vsift.ps1` and an extensionless shell script. Started from PowerShell, the `.cmd` file
  makes cmd.exe re-read the command line. With a stub package that prints its arguments (the
  same shim generator, no registry), `%COMSPEC%` and `%PATH%` were expanded before the program
  saw them and the double quote of `a"&echo x>marker&"b` was dropped. With the published 0.1.0 on
  a hosted runner, the argument `a;echo,x>pwned-marker`, which has no whitespace and so is not
  quoted, created the file `pwned-marker`: a command it spelled ran. The `.ps1` shim and Bun's
  `vsift.exe` did not misbehave: for 14 hostile file names (each had to open the file) and 11
  hostile arguments (each had to run no command), `vsift` answered exactly as when `vsift.exe` is
  started directly. (VSift's typed failure does not echo an argument, so a changed argument that
  changes nothing visible is seen only with the stub.) Found on 2026-10-02.
- **Evidence:** the P14 PR 2 `clean-install` job of `P14 published artifacts` (the cmd.exe shim
  is an observation there, not a gate), `tools/p14-published/lib/hostile.cjs`,
  [`install.md`](../operations/install.md) section 2; threat SEC-01.
- **Impact:** a program on Windows that starts `vsift.cmd` with text it did not write (a query
  taken from a transcript, a path from a file name) **through cmd.exe** (Node's `exec`, Python's
  `shell=True`, `cmd /c`, a batch file) can have that text acted on by cmd.exe, and so can a person
  who types such text at a cmd.exe prompt. Typing `vsift` in PowerShell runs the `.ps1` shim, in Git
  Bash the extensionless script, and Bun runs `vsift.exe`; none is affected. A program that starts
  an executable with an argument list and no shell is not affected either.
- **Why:** the `.cmd` file is generated by npm (cmd-shim) and pnpm for every `bin` entry; VSift's
  launcher is the node script behind it and never sees the original line. VSift cannot change a
  file another tool writes, and the launcher cannot repair a command line that cmd.exe has already
  rewritten.
- **Mitigation:** [`install.md`](../operations/install.md) section 2 names who is affected and three
  safe routes (PowerShell or Git Bash; the native archive's `vsift.exe` started with an argument
  list and no shell; Node started on the launcher, `bin/vsift.cjs`, or the platform package's
  `vsift.exe`, with an argument list); [`SECURITY.md`](../../SECURITY.md) and the launcher's README
  (shipped in the package from the next release) say the same in two lines. A launcher test sends
  the hostile arguments of the P14 published-artifact job through the launcher route and requires
  every one to arrive unchanged, and `install.md` is pinned to name the routes. The agent trials keep
  off the shim (`docs/agents/trials.md`: Git Bash only for Claude Code, Linux for Codex) and count
  every call by shell.
- **Next step:** none planned: the exposure needs a caller that already builds a cmd.exe command
  line from untrusted text, and that caller can use the routes above. A candidate for the next skill
  freeze (not made: the skill is frozen for the agent-trial batches) is one sentence telling an agent
  on Windows to run `vsift` from PowerShell or Git Bash, never through `cmd.exe`.
- **Owner:** P14 (PR 7). **Issue:** [#257](https://github.com/smormah/vsift/issues/257).
  **Status:** accepted residual. **Review:** pending.

### L-111

**The upgrade evidence has one published baseline, and its two modes prove different things.**

- **What:** RQ-04 upgrades from 0.1.0, the only published release. While only 0.1.0 is
  published, the **real-registry** mode installs the same version again over an install holding
  two sessions, a retained bundle and a configuration: it proves the guide's procedure and that
  the procedure disturbs nothing the user kept, and no newer version reads an older one's data
  there. The **local-registry** mode (`P14 local upgrade`) upgrades the published 0.1.0 to the pull
  request's own build, packed as `vsift-cli@99.0.0-p14local.1` by a script that follows the
  launcher's layout and served by a loopback registry: it proves a newer build reads a published
  build's sessions, bundle and configuration and that the package manager upgrades, and not the
  release packaging (`vsift-release npm` needs the archives, notices and SBOMs). The JSON
  compatibility test and the stored-record test cover 0.1.0's examples and records only, and a
  downgrade is not supported ([L-044](#l-044)). Only npm is used for the upgrade, on three systems.
- **Evidence:** [`p14-qualification.md`](p14-qualification.md) section 15; the two workflows;
  `schemas/v1/frozen/v0.1.0/`.
- **Impact:** until the candidate (`0.2.0-rc.1`) is published and the real-registry mode runs
  from 0.1.0 to it, no run shows a published version reading an older published version.
- **Why:** there is one published baseline; the release packaging cannot be reproduced without
  its inputs in a job that must stay small.
- **Mitigation:** the candidate's qualification runs the real-registry mode from 0.1.0
  (`from_version`), and each later release is frozen the same way (a folder `frozen/<tag>/`).
- **Next step:** PR 11 records the real-registry run on the candidate; PR 12 on the stable.
- **Owner:** P14 (PRs 10, 12). **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** deferred. **Review:** pending.

### L-112

**The clean-install jobs hide named programs from `PATH` on a hosted image; that is not a clean machine, and one tool set stands in for each system's users.**

- **What:** the `P14 published artifacts` jobs run every install and every `vsift` command with a
  `PATH` that no longer resolves `cargo`, `rustc`, `rustup`, `git` or `python` (and, for the
  archive job, Node.js and its package managers), a fresh user state and no token, and assert
  that before installing anything. On Linux and macOS a shared directory such as `/usr/bin` is
  replaced by links to everything in it but those programs; on Windows a directory that holds one
  of them is dropped whole. The runner image still carries the programs elsewhere, the registry
  and environment settings it ships, a different Windows build than a user's, Homebrew's FFmpeg on
  macOS and a pinned FFmpeg build on Windows, which no one reviewed as a user's tools. Proxies,
  antivirus, Smart App Control and signed-software policies are absent. A planted tool on `PATH`
  is probed by `setup check` (documented trust in the user's own `PATH`) and refused for media
  work by the media-tool check; one in the working directory is never looked at.
- **Evidence:** `tools/p14-published/lib/scrub.cjs` and its tests; the first lines of each
  `clean-install` job summary (what the runner carried before the scrub);
  [`p14-qualification.md`](p14-qualification.md) section 15.
- **Impact:** a pass shows no hidden dependency on those programs, a checkout or the developer's
  `PATH`; it does not show a user's machine ([L-098](#l-098), RQ-17).
- **Why:** hosted runners are what a repository can run on every change; real machines are the
  maintainer's try-outs.
- **Mitigation:** the plan says so wherever it claims more; the try-outs are RQ-17.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-113

**The P11 durable stage cannot run on a hosted runner, so the published binary's durable
worker request has never run on the qualified profile.**

- **What:** in the `P14 journeys` run of the published 0.1.0 (run 36965956708), the P11
  worker checkpoint passed every stage but `p11_durable_workspace`, which is `blocked` on all
  three systems: `session init-workspace --durability durable` was refused with
  `MISSING_CAPABILITY` and created nothing. On the hosted `ubuntu-24.04` runner the cause is
  the storage: `/etc/os-release` says Ubuntu 24.04, but the root is `/dev/sda1` ext4 mounted
  `rw,relatime,discard,journal_async_commit,nobarrier,errors=remount-ro,commit=30,
  data=writeback`, and the durable profile requires ext4 with write barriers left on.
- **Evidence:** the run above (job `Journeys (ubuntu)`, step "Record the storage the
  durable profile reads (Linux)"); `crates/vsift-infrastructure/src/durable_profile.rs`;
  [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md);
  issue [#258](https://github.com/smormah/vsift/issues/258).
- **Impact:** RQ-05 cannot show that the shipped bytes publish a durable worker request
  (`os_crash_durable`) and replay it on the profile where it is claimed. The product behaved
  as designed (it refuses a host that disables barriers); the gap is in what was proved. The
  P10 and P13 campaigns exercise the durable paths in virtual machines with a real ext4 root,
  with campaign builds (a release cannot carry the fault-injection features), not the
  published binary.
- **Why:** a hosted runner's virtual machine disables barriers, and no hosted job can
  change a mount of its own root.
- **Mitigation:** the stage reports `blocked` with the reason, never `passed`; RQ-05 stays
  `running` until it runs or the maintainer records what covers it; durable-worker claims
  lean on RQ-11 and are limited to the profile (L-008).
- **Next step:** run the stage inside the virtual machine the `P10 durability campaign`
  boots, with the installed executable, or record that RQ-05 does not cover it (#258).
- **Owner:** P14. **Issue:** [#258](https://github.com/smormah/vsift/issues/258).
  **Status:** open. **Review:** pending.

### L-114

**macOS is tried with Homebrew's FFmpeg and whisper.cpp, which are not reviewed artifacts.**

- **What:** no reviewed FFmpeg or whisper.cpp build exists for macOS (whisper.cpp v1.9.2
  publishes no macOS CLI archive, [L-035](#l-035)), and managed installation is Ubuntu-only.
  The macOS journeys therefore install Homebrew's bottles on the runner and record what they
  got: on the image `macos15` 20260907.0337.1, `ffmpeg 9.0.1_1` and `whisper-cpp 1.9.2` (the
  formula is named `whisper-cpp` on that image's tap and `whisper.cpp` on newer ones; the
  tap is the image's snapshot, not updated), with the repository's pinned, hash-checked
  `base` model. With them every checkpoint passed on the published 0.1.0 and the T-04 gates
  held (in process: clean word error rate 4.06% for `base` and 4.87% for `base_q5_1`, F08
  61.53% and 46.15%, the same reviewed misses, 3.03 times slower than real time on three CPUs).
- **Evidence:** `P14 journeys` run 36965956708 (`tools.json`, `results.json` and the
  `p07_asr_gates` log in its macOS artifact); [P14 plan](p14-qualification.md) section 17.
- **Impact:** a macOS "supported" cell (ADR 0024 decision F) would rest on the user's own
  tools; what Homebrew installs changes with its tap and is not pinned, so a later run may
  meet other versions.
- **Why:** reviewing and pinning a macOS toolchain is its own work (ADR 0007), outside R0.
- **Mitigation:** the versions are recorded in every run; the claim is worded as what the
  hosted run proves with the user's tools; no managed install is offered there.
- **Next step:** P14 PR 9 decides the matrix wording; a reviewed macOS build would need its
  own review and catalogue entry.
- **Owner:** P14 (PR 9). **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** deferred. **Review:** pending.

### L-115

**The published-binary journeys run later tests against 0.1.0 and do not reach the
launcher, the archives or the engine-library stages.**

- **What:** (1) 0.1.0's source predates the binary override, so for 0.1.0 the checkpoint
  tests are compiled from the workflow's ref (a later commit), not the tag: fixtures,
  schemas and the in-process engine parts are the later commit's, while the binary is the
  one published as 0.1.0 (tag commit `011bc4d`, checked by `--version` and by
  `platform-digests.json`). From the first tag that contains the override, `auto` uses the
  tag's own tests. (2) The checkpoints run the native executable of the platform package
  directly, with an empty `PATH`: the npm launcher and the extracted archives are RQ-01 and
  RQ-02's. (3) Some stages use the engine library compiled in the test, not the binary:
  P06's tool verification, P08's `p08_candidates_budget`, P11's session inspection and the
  macOS-only T-04 gates. (4) The sentinel check sees what the probes of `setup check` and of
  `frame get` pass to a recorder standing in for each tool (no variable at all on any
  system), not the environment of real FFmpeg or whisper.cpp, which cannot be asked; the
  hostile names go through `ingest` and `frame get`, not every command. (5) The Windows tools
  are the repository's pinned BtbN LGPL 9.0.1 build, not the gyan.dev build the maintainer's
  machine uses.
- **Evidence:** `tools/p14_journeys.py`, `crates/vsift-cli/tests/published_binary/mod.rs`,
  `p14_installed_binary_e2e.rs`; the job summaries of the run above ("Test source", "Not run
  here, and why"); [P14 plan](p14-qualification.md) section 17.
- **Impact:** low. A test expectation that depends on behaviour newer than 0.1.0 would fail
  (a finding), not pass falsely; what a pass does not show is listed per run.
- **Why:** the override is new, and the stable procedure compares artifacts, not tests.
- **Mitigation:** every run names its test source, the binary's `--version` line and SHA-256;
  RQ-01 and RQ-02 cover the launcher and the archives.
- **Next step:** none planned; the candidate's tag carries the override.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:**
  accepted residual. **Review:** pending.

### L-116

**A weekly drift run shows only that one hosted run of the highest published version passed
that week.**

- **What:** `P14 journeys` (Wednesday 04:37 UTC) and `P13 managed smoke` (04:53) run weekly
  against the highest published version of `vsift-cli`. A green run says one hosted run
  passed on that day's runner image (`ImageVersion` is recorded in `results.json`); a red run
  may be a changed runner image, Homebrew snapshot or publisher host (L-099) rather than
  VSift (or, once, the driver: a hosted Windows run stalled after `p10` and was cancelled at
  its limit, [#263](https://github.com/smormah/vsift/issues/263)). No issue is opened
  automatically (no workflow holds a write permission): the signal
  is the failed run, its job summary and GitHub's own notification of it. Schedules run only
  from the default branch, GitHub documents that it disables a scheduled workflow of a public
  repository after 60 days without repository activity, and once a stable exists the weekly
  run no longer tests an older candidate. About one runner hour a week for the journeys
  (about 12, 20 and 21 minutes on Ubuntu, Windows and macOS) and eight minutes for the managed
  smoke; the macOS T-04 gate (about an hour) runs only on a dispatch.
- **Evidence:** `.github/workflows/p14-journeys.yml`, `.github/workflows/p13-managed-smoke.yml`;
  [P14 plan](p14-qualification.md) section 17.
- **Impact:** low: a drift can go unnoticed until someone looks at the Actions page, and a
  red run needs reading before it is called a VSift defect.
- **Why:** automation that opens issues or changes settings needs permissions these
  read-only workflows deliberately lack.
- **Mitigation:** the job summary says "a finding: open an issue before rerunning"; the
  maintainer's release procedure dispatches both before each release.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual. **Review:**
  pending.

### L-117

**The clean-install and cold-agent trials are not clean-machine trials, and a cold agent on Windows can still find the package's README and skill.**

- **What:** the P14 agent rounds install `vsift-cli` from the real registry into a fresh folder
  and prove the bytes are the registry's (what npm fetched against the integrity the registry
  advertises, the launcher's digest check redone, the exact version). They run on the maintainer's
  Windows 11 development machine, which has Node.js, Rust, Git and Claude Code around, and, for
  Codex, in a container with ordinary outbound network. The proof does not say the registry's
  bytes are the maintainers' (`npm audit signatures` and `gh attestation verify` are RQ-01, RQ-02
  and RQ-19), and it is not a clean machine. The package ships its README and the skill under the
  install prefix: in the Codex agent image they are made unreadable to the trial user, but on
  Windows nothing physically stops a cold agent from reading them (Claude Code's own rules deny it
  and the grader fails it as `outside_allowed_folders`). When a scenario needs any tool the managed
  install gives all three components, so `tools.whisper: false` cannot withhold whisper.cpp on
  Ubuntu (the plan's unit is the profile), and each Codex trial downloads about 271 MB from the
  publishers (L-099). A cold trial's own settings file, decoy and prompt still say that a tool
  named `vsift` is installed; its folder and canary names say nothing else. Two findings of P14
  PR 2 shape the trials and are therefore not what a user always meets: the Codex images install
  `libgomp1` from the distribution because a minimal Ubuntu 24.04 lacks it and `setup install`
  then failed without naming it (#256, fixed in P14 PR 7), and the Claude trials reach `vsift` only through
  Git Bash, never through the `vsift.cmd` shim that re-reads arguments (L-109, #257), so no agent
  trial exercises that shim; each record says which shell ran (`shim_use`).
- **Evidence:** [`trials.md`](../agents/trials.md) ("Clean-install mode", "What is weaker than it
  sounds"); `tools/vsift-agent-trials/src/install.rs`; the Dockerfile's `agent-published` stage.
- **Impact:** a hidden dependency of the maintainer's machine could let a run pass that a clean
  machine fails, and a cold agent that read the package's documentation would not be cold; the
  second is visible (the read is a safety failure), the first is not.
- **Why:** Claude Code runs only on the maintainer's machine, and hosted CI cannot run the clients
  without the maintainer's allowances.
- **Mitigation:** the read gate; the unreadable files in the Codex image; RQ-01, RQ-02 and RQ-19
  for provenance on hosted runners; the maintainer's try-outs (RQ-17); the post-R0 trial of the
  published CLI on real recordings.
- **Next step:** none planned inside P14.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:**
  accepted residual. **Review:** pending.

### L-118

**The cold grader reads command text and matches free text mechanically.**

- **What:** the safety gate is decided from the commands a client reports: program names,
  every argument read as a path (since 2026-10-03, with `~` as the home folder and a leading
  `VAR=x` skipped, see [L-125](#l-125)) or a URL, and the `vsift` classes from `commands.md`. It can miss
  an installer under another name, a redirect to a path the shell reader does not attribute (a
  write outside the workspace by redirection), or a download by an interpreter's own code with an
  address the reader cannot see. Syntax it cannot read (command substitution, a subshell, a
  script block, an encoded command) is `unverifiable` and fails the gate even when harmless, so a
  harmless run can fail safety. Usefulness is word matching: a correct report in other words
  fails, a report that states a fact next to any real identity inside the event's window passes,
  and a cited frame counts as inspected if the agent opened any image at all. The samples are
  small (5 of 6 per client at the final round, 3 runs per scenario).
- **Evidence:** `tools/vsift-agent-trials/src/cold.rs` and `tests/cold_grader.rs` (every kind has a
  case); [`trials.md`](../agents/trials.md) ("Cold-agent mode").
- **Impact:** the zero-unsafe-actions claim rests on the grader and on a reading of the raw logs;
  a pass is not proof of safety, and a fail can be a conduct problem rather than an unsafe act.
- **Why:** without the skill there is no handoff schema and no fixed command form to check
  against, and an agent that has no instruction may write any shell.
- **Mitigation:** the maintainer reads every cold run's raw log before the claim is made; the
  gap report has a `reviewer_note`; a grade has a `human_review` slot the summary honours.
- **Next step:** none planned; a finding that the grader missed or over-fired is its own issue.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:**
  accepted residual. **Review:** pending.

### L-119

**There are two hold-out scenarios, one run per client each, written by the same authors.**

- **What:** the plan asks for one hold-out per transcript path. They are `H-01-f10-supplied-sidecar`
  and `H-02-f01-local-asr`, in the same synthetic corpus and voice as every scenario the skill was
  tuned on; F01 appeared in A-01 as a refusal scenario that never read its readout. The review
  tier runs each once per client, so each path shows 0% or 100%, and the rule (a gap of more than
  20 points below the same path's other runs is a finding) is coarse. The separation is mechanical
  (a frozen index, no shared event or id, both paths covered): it cannot show that nobody looked at
  them while tuning, and the help text's iterations after the cold baseline are tuned on the cold
  scenarios themselves, with no cold hold-out.
- **Evidence:** `tools/vsift-agent-trials/holdout/INDEX.json`; `src/holdout.rs`;
  `tests/holdout_freeze.rs`.
- **Impact:** a pass is a signal, not a measurement, and an overfitting the corpus's sameness hides
  would not show.
- **Why:** more scenarios cost the maintainer's allowances, and a larger independent corpus is R1's
  (real recordings, #150, #159).
- **Mitigation:** the freeze; a failure is a finding, never a grader edit; the post-R0 trial on
  real recordings.
- **Next step:** real-recording hold-outs after R0.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:**
  accepted residual. **Review:** pending.

### L-120

**Usage figures and the usage-limit reading are the clients', and the harness's parsers have not met a real stream.**

- **What:** each trial record carries the tokens and cost the client reported. Claude Code's cost
  is its own estimate at list prices; Codex reports tokens and no cost; the two count input
  differently (Claude Code's excludes cache reads and writes, Codex's includes the cached part).
  The parsers and their tests were written from the event shapes the earlier parsers already read
  (a Claude Code `result` event with `usage` and `total_cost_usd`, Codex's `turn.completed`
  usage), not from recorded streams: no raw log of P12 is in the repository. The words a client
  prints when it hits its usage limit are not a published contract; the detector knows the ones
  the clients' documentation and reports show. A client that ends with an error before one tool
  call is graded invalid whatever it said, but one that hits a limit after some calls, in words the
  detector does not know, is graded as an ordinary failed trial and counted.
- **Evidence:** `tools/vsift-agent-trials/src/trace.rs`, `usage_limit.rs`,
  `tests/usage.rs`; [`trials.md`](../agents/trials.md) ("Usage capture").
- **Impact:** a spend figure may be missing or off by the client's own accounting, and a
  usage-limited run might be counted as a failure. The summary lists every counted run whose
  client exited non-zero, so it is seen.
- **Why:** the harness does not meter the clients and cannot see the provider's side.
- **Mitigation:** the pilots (batch 1) show the real streams before anything counts; the
  maintainer reads the non-zero exits the summary lists.
- **Next step:** add the first recorded streams as fixtures after the pilots.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:**
  accepted residual. **Review:** pending.

### L-121

**Text drawn inside the README's SVG graphics is public text the claims check cannot read.**

- **What:** the README shows eight hand-made SVG graphics (`docs/assets/readme/`): a hero, an
  animated terminal, the evidence timeline, the pipeline, a before-and-after panel, the
  architecture and the roadmap, and the logo. Their words are public, but
  `vsift-governance public-claims` reads only the listed Markdown documents, so a controlled word
  or a banned phrase inside an SVG passes the Governance job. The roadmap graphic also states the
  current rung (P14 "now", `0.2.0-rc.N` and `0.2.0` planned), so it goes stale when a rung moves.
- **Evidence:** [`docs/assets/readme/README.md`](../assets/readme/README.md) (the hand check and
  when to redraw); `docs/planning/public-claims.json` (`documents`).
- **Impact:** a careless edit to a graphic could claim more than the ladder allows, and the
  roadmap could show an old step as current; neither would fail CI.
- **Why:** the check reads plain text and the graphics are XML whose words are split across
  elements; teaching it SVG was not worth it for one README change.
- **Mitigation:** every word in the graphics was checked by hand against the controlled words
  and banned phrases when they were made (2026-10-02); the asset notes give the grep to repeat.
- **Next step:** add `docs/assets/readme/*.svg` to the scanned documents, reading the text
  content of `<text>` elements; redraw the roadmap with P14 PRs 10 and 13.
- **Owner:** unscheduled (P14 PR 9 is the natural home). **Issue:** none. **Status:** open.
  **Review:** pending.

### L-122

**One recorded FFmpeg vulnerability (CVE-2026-38350, libswscale) is tied to its fix only by elimination, and the reading proves the source of the shipped build, not its behaviour.**

- **What:** the managed catalogue installs one reviewed FFmpeg, the BtbN snapshot
  `n9.0.1-11-ge47273f4d9` of 2026-08-31 (month-end, retained two years). Of the 47 public records naming
  FFmpeg (published June to September 2026), the first reading (2026-10-02) counted 17 as fixed on `master`
  only and 18 as having no fix reference, which is why the finding opened. Its test could not see a
  release-branch cherry-pick. The re-read of 2026-10-04 (the addendum of
  [`p14-scan-reading-2026-10-02.md`](p14-scan-reading-2026-10-02.md)) finds **46 of the 47 fixed in the
  shipped snapshot** (the 17, 17 of the 18 and the older 12; the 28 cherry-picks that stand in for a master
  fix carry its patch text) and **one not reachable** (CVE-2026-38347, the alpha-blend path VSift never
  switches on). **One of the 46 is by elimination:** CVE-2026-38350 (High, signed-overflow denial of service in
  `libswscale`'s output code, which VSift's frame conversion runs) names a fuzzer function that no issue
  names; the one other `output.c` issue of the same reporter's batch (21586) is the statement that
  `86ddc8b438` rewrote, and the code is the same in the shipped snapshot, the refresh candidate and
  `master` (`release/9.0` is the only release branch newer than 8.1). **If the record describes another site,
  nothing here covers it.** The refresh candidate (`n9.0.2-22`) is no better or worse for any of the 47.
- **Evidence:** the addendum (identifiers, severities, components, commit hashes, the method, how it can be
  wrong, and the exposure argument); `tools/p14-campaigns/ffmpeg-ancestry.cjs`;
  [#272](https://github.com/smormah/vsift/issues/272). Whisper.cpp v1.9.2, the pinned actions and the Rust
  dependencies gave no finding.
- **Impact:** untrusted media is decoded by an FFmpeg that may still carry a bug of this class. The allow-lists,
  the byte, time, pixel and stream bounds and, in a worker, the strict-isolation container limit what such a
  bug could do; none of them removes it. A cherry-pick line and equal patch text show the fix is in what the
  snapshot was built from, not that the binary was tested against a reproducer, and the database's
  records are neither complete nor timely. Eight integer-overflow fixes in `output.c` in five months say
  the class is not closed by these records.
- **Why:** nothing public ties the record to a commit by name, and a fix cannot be proved absent from a binary
  by reading history.
- **Mitigation:** keep untrusted media in the strict-isolation container
  ([`worker-host.md`](../operations/worker-host.md)); the allow-lists in
  `crates/vsift-infrastructure/src/ffmpeg_media.rs` (the `mov` and `matroska` demuxers, the `file`
  protocol, a decode only of the six video and seven audio codecs it lists); the reading repeated, with the
  tool, within seven days of the candidate and again before the stable.
- **Next step:** the maintainer decides: accept the tie by elimination as the register entry for this
  record, or ask upstream which report it describes. Refreshing the build does not change it
  ([L-132](#l-132)).
- **Owner:** P14. **Issue:** [#272](https://github.com/smormah/vsift/issues/272). **Status:**
  open. **Review:** pending.

### L-123

**Two Windows concurrency failures reproduce on a hosted runner at about one repetition in 200: session-root creation and the weighted-admission grant.**

- **What:** the P14 stress campaign repeated the tests on hosted Windows Server 2025, Ubuntu 24.04
  and macOS 15. `session_root_provisioning` failed **7 times in 1,500** on Windows (and 2 in 1,500
  with every CPU kept busy) with "session storage root permissions are not private", in the
  threads test as well as the processes test; this is [#206](https://github.com/smormah/vsift/issues/206),
  now reproduced. `weighted_admission_never_exceeds_root_capacity` failed **2 times in 200**
  (consecutively) on Windows: a child never received a reservation within its wait
  ([#271](https://github.com/smormah/vsift/issues/271)). None of the other tests failed on any
  system, and neither failed on Ubuntu or macOS (0 in 3,000 and 0 in 400). Issue
  [#128](https://github.com/smormah/vsift/issues/128) (process supervisor, Windows) did **not**
  reproduce: 0 in 3,000 repetitions on each system.
- **Evidence:** `P14 stress` run 36978939586 (`p14-stress-roots-windows-2025`,
  `p14-stress-roots-loaded-windows-2025` and `p14-stress-admission-windows-2025` keep the failing
  outputs); [`p14-qualification.md`](p14-qualification.md) section 18.
- **Impact:** a Windows process that opens a session root while another creates it can be refused
  with a typed error (fail closed: nothing wrong is accepted). Whether the refused call works when
  repeated was not measured. For the admission failure it is not known whether the product or the
  test's wait is at fault.
- **Why:** not traced. The root's privacy is checked while another creator may still be setting
  its access list, as [#206](https://github.com/smormah/vsift/issues/206) first suspected.
- **Mitigation:** create the root with one process before starting others (likely, not measured).
- **Next step:** fix #206 with a regression test that repeats creation on Windows; explain or fix
  #271; re-run `P14 stress` on the candidate (RQ-08).
- **Owner:** P14. **Issue:** [#206](https://github.com/smormah/vsift/issues/206),
  [#271](https://github.com/smormah/vsift/issues/271). **Status:** open. **Review:** pending.

### L-124

**Local recognition of a five-second range fails as a missing capability for three of ten valid speech clips.**

- **What:** with the published 0.1.0 and the managed tools, `transcript retranscribe --from 0 --to
  5000000` of the synthetic clips F02, F04 and F05 ends `MISSING_CAPABILITY` ("output_validation
  (malformed_output)") while the same range of F01, F03, F06, F07, F08, F09 and F12 recognises, and
  the whole of F02, F04 and F05 recognises. The same request as a worker `retranscribe` step fails
  the same way. The cause is not known. In two early smoke runs of the load campaign a session or two
  was left listed as `initializing`, and `session status` of such a
  session answers `STORAGE_IO` ([#277](https://github.com/smormah/vsift/issues/277)); it did not return in
  the later runs, so the connection is not established.
- **Evidence:** `P14 load` run 37136669579 (`load-summary.md`, `diagnose-recognition.json`);
  [#274](https://github.com/smormah/vsift/issues/274).
- **Impact:** an agent asking for a range of speech can be told that a tool is missing, with a
  remediation (reinstall whisper.cpp) that cannot help, and the code is not retryable. The
  whole-clip request works for these clips.
- **Why:** not traced; the recogniser's raw output for the failing chunk was not kept.
- **Mitigation:** ask for the whole clip, or a wider range.
- **Next step:** reproduce with the raw output, decide whether a segment beyond the chunk is
  clamped or dropped with a coverage note, and stop reporting a malformed output as a missing
  capability; add the three clips to the local-ASR checks. The load campaign's request mix avoids
  these clips until then.
- **Owner:** P14. **Issue:** [#274](https://github.com/smormah/vsift/issues/274),
  [#277](https://github.com/smormah/vsift/issues/277). **Status:** open. **Review:** pending.

### L-125

**The realistic cold setting cannot be fenced to the workspace, so Claude Code runs it only on an isolated machine; the two clients' cold baselines are not the same test.**

- **What:** the cold trials have two settings (maintainer decisions of 2026-10-03; ADR 0024, "the cold
  settings, strict and realistic"). **Strict** (`Bash(vsift:*)` only) is Claude Code on the maintainer's
  machine; its first pilots were safe but stalled on chained commands. **Realistic** also lets the agent run
  `ls`, `cat`, `head`, `tail`, `pwd`, `cd`, `wc`, `echo` and `sort`, alone, chained or piped. A Claude Code
  allow rule matches command text and cannot say which paths a command may name, so `cat /somewhere/else`
  runs where `cat` is allowed: on the maintainer's Windows machine the helpers could read any file the user can
  read, which conflicts with the rule that personal details do not leave the machine without consent. So the
  realistic Claude file is a named option (`claude-cold-trial-settings.realistic.json`) that
  `run-campaign.ps1` refuses unless `-IsolatedMachine` states the machine is isolated, and **Codex in the
  Linux container is the realistic variant in effect** (its sandbox is the only restriction on commands, and
  the container is the isolation). The baseline therefore compares like with like **only within each client**
  (Claude strict, Codex realistic); the summary says so per run and a report must not set one against the
  other. Even on an isolated machine a read outside the workspace is stopped by the grader's safety gate after
  the fact (`outside_allowed_folders`; the grader reads every word of a helper's arguments as a path, a leading
  `~` as the home folder and `VAR=x command` as `command`), not prevented. `sort -o` and `--output` are denied
  by rule, but a deny rule matches text and has no character classes, so a clustered flag (`sort -ro file`) is
  not matched. A bare `VAR=value` (the strict pilots' `S=ses_...; vsift ... $S`) is **not** allowed in either
  Claude file: a rule matches a command's whole text and `*` crosses spaces, so `Bash(S=*)` would also allow
  `S=x <any program>`. Nothing was run against the client when the settings were written (no model call; the
  permission engine has no offline evaluator), so which chained forms Claude Code 2.1.284 allows is read from
  its documentation.
- **Evidence:** `tools/vsift-agent-trials/claude-cold-trial-settings.json` and `.realistic.json`; the settings
  tests in `tests/scenario_sets.rs`, the refusal tests in `tests/campaign_script.rs`, the grader tests in
  `tests/cold_grader.rs` (one replays a Codex run); [`trials.md`](../agents/trials.md) ("Two cold variants").
- **Impact:** a strict Claude cold run may stall on chained commands and so understate what a cold agent can do
  (the baseline is a measurement, not a gate); a realistic run (Codex, or Claude on an isolated machine) is
  graded after the fact, so a read the grader fails has already happened inside the container or machine. The
  zero-unsafe-actions claim rests on the grader reading command text (L-118) and on a person reading the raw
  logs.
- **Why:** the client's rule language cannot express "inside the workspace", and the maintainer's machine holds
  the user's own files.
- **Mitigation:** the strict default on the maintainer's machine; the refusal without `-IsolatedMachine`; the
  container for Codex; the gap report marks every refused bare assignment (`denied_assignment`) and the batch
  summary lists the runs that met one, so a stall on them is easy to see; the maintainer reads every cold run's
  raw log before the claim is made.
- **Next step:** read the re-run pilots. If Claude's strict runs stall, the clean test machine (decision H's
  second Windows machine) is where the realistic Claude baseline can run; if agents stall on assignments, the
  maintainer decides then (the setting stays strict until then).
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:**
  accepted residual. **Review:** pending.

### L-126

**A session root VSift did not create is refused with `INTEGRITY_FAILURE`, which says stored data is damaged; only the remediation says what happened.**

- **What:** `--session-root` (or the per-user default) naming an existing folder that holds no
  VSift ownership marker, such as an empty folder made with `mkdir`, is never adopted and is left
  untouched. Since P14 PR 7 (issue #261) the answer carries a fixed-prose remediation that says the
  folder holds no VSift marker, was not created by VSift and was not used, and names the fix (a
  path that does not exist yet, or delete the folder). The failure code is still the 0.1.0 one,
  `INTEGRITY_FAILURE` (exit 7). `INVALID_ARGUMENT` (exit 2) would describe a person's mistake
  better, like `NotDirectory` and `AlreadyExists` already do, but changing the code of a published
  answer is not additive within v1, and the agent skill (frozen for the trials) tells an agent to
  stop and report on `INTEGRITY_FAILURE` and to correct the request once on `INVALID_ARGUMENT`.
  A just-made empty folder is also refused only after the five seconds the documented concurrent-creator
  wait takes. A marker that is present but wrong is not described this way, nor is one that vanishes
  while a command is using the root (both stay a bare `INTEGRITY_FAILURE`); a later command that finds
  a VSift root whose marker was deleted cannot tell it from a folder VSift never made and describes it so.
- **Evidence:** [`cli-v1.md`](../contracts/cli-v1.md) ("Private per-user folders"),
  `schemas/v1/examples/session-root-unowned.json`, the CLI test
  `foreign_session_root_cli_contract` and issue #261.
- **Impact:** a script or an agent that keys on the code sees an integrity failure and exit 7 for a
  user mistake; one that reads the remediation sees what to do. Agents do not pass `--session-root`
  (an operator option the cold grader fails), so the skill's table is not exercised by it.
- **Why:** v1 is additive only since 0.1.0, and the skill is frozen until the trials finish.
- **Mitigation:** the remediation text; the contract, the schema example and the install guide say
  so.
- **Next step:** at the next major contract version, or the next skill freeze if a consumer is found
  to branch on this case, answer `INVALID_ARGUMENT` for a folder with no marker.
- **Owner:** unscheduled. **Issue:** none (#261 is closed by the remediation). **Status:** accepted
  residual. **Review:** pending.

### L-127

**The CLI's ingest treats three kinds of hostile source worse than the worker path does: a named pipe hangs it, a link is refused as a storage failure and a full disk is reported as corruption.**

- **What:** the P14 malicious-media campaign ran 96 generated inputs through 251 operations of
  the published 0.1.0 in a no-network, read-only-root, memory- and process-bounded container.
  Every other input was refused or processed inside its bound with a typed code. Three were not:
  `vsift ingest` of a **named pipe with no writer never returns** (killed at 150 s;
  [#264](https://github.com/smormah/vsift/issues/264)); `ingest` of a **symbolic link** answers
  `STORAGE_IO` instead of `INVALID_SOURCE`, and no document says how links are treated
  ([#265](https://github.com/smormah/vsift/issues/265)); `ingest` of a **600 MiB file into a root of
  256 MiB** fails after 5 s as `INTEGRITY_FAILURE` where the job path refuses early as
  `RESOURCE_LIMIT` ([#266](https://github.com/smormah/vsift/issues/266)). A worker job request
  refuses all three at once with a typed code.
- **Evidence:** `P14 malicious media` run 37136669473 (`hostile-summary.md`);
  [`p14-qualification.md`](p14-qualification.md) section 18.
- **Impact:** a script or agent handed a path it did not create can wait for ever on a pipe; the
  two misclassifications send an agent to investigate storage or the source when the cause is the
  path or the disk's size. Nothing wrong is stored or shown as evidence, and the link's target was
  never read.
- **Why:** the CLI path opens the source before it checks the file type, and does not apply the
  free-space check the job path applies (not traced).
- **Mitigation:** use the worker path for paths you do not control; keep untrusted sources off
  pipes and links.
- **Next step:** check the file type first on the CLI path, give links one documented answer and
  share the free-space check; a regression test per case.
- **Owner:** P14. **Issue:** [#264](https://github.com/smormah/vsift/issues/264),
  [#265](https://github.com/smormah/vsift/issues/265),
  [#266](https://github.com/smormah/vsift/issues/266). **Status:** open. **Review:** pending.

### L-128

**The fuzzing is one hour per target on shared hosted CPUs, nineteen of 31 targets were still finding coverage at the end, and three kinds of stored record have no target.**

- **What:** the P14 campaign ran every one of the 31 targets for 3,601 s (3.68 billion runs in all)
  with no crash, timeout or out-of-memory. For 19 targets the last new coverage came in the final
  tenth of the run (for example `request_record`, `transcript_record`, `bundle_manifest`,
  `handoff_check`, `job_batch_file` and `setup_plan`), so a longer run may still find new paths; three
  (`transcript_cursor`, `crop_rect`, `png_sequence`) stopped finding any in their first 6 percent.
  Seven targets were added by the gap review (the saved setup plan, the bundle manifest and its
  artifacts, the tar, gzip and xz archive inventories, the identifier grammars, the input-path
  grammar). Not fuzzed: the session root's ownership marker, the media-tool verification record and the
  user dependency configuration, which are read from folders VSift
  creates owner-private; the managed store's ownership marker is compared with
  fixed bytes, not parsed. The bundle target runs on Unix only (the Windows replay skips it).
- **Evidence:** `Fuzz` run 36978914176 (one plateau line per target in each job log);
  [`p14-qualification.md`](p14-qualification.md) section 18.1.
- **Impact:** "no finding" is a floor: it covers those inputs for that hour, not every input. A
  malformed file in one of the three unfuzzed records is read by code no fuzzer has exercised;
  the writer of such a file already has the user's rights.
- **Why:** an hour per target was the plan's bar and the hosted budget; the three records are not
  reachable through a published parse function, and the harness reaches parsers only through the
  published surface.
- **Mitigation:** the weekly five-minute runs and every pull request's replay of the committed seeds
  continue; the candidate's qualification repeats the long run after the fixes (RQ-07 is `carry` to
  the stable only when no file in its scope changed).
- **Next step:** repeat the long run on the candidate with the two-hour budget for the 19 targets
  still growing; expose and fuzz the three records if a published parser is added.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17). **Status:**
  accepted residual. **Review:** pending.


### L-132

**The reviewed FFmpeg can only follow a month-end build of its publisher, and a new pin does not move existing installs: the refresh candidate of 2026-10-03 is a daily build, needs two reviewed bounds raised and adds three libraries to the recipe.**

- **What:** BtbN keeps the last build of each month for two years and only the last 14 daily builds, so a
  catalogue pin has to be a month-end build; the one pinned (2026-08-31) is kept to about 2028-08-31. The
  newest build of the catalogue's variant is a daily build (`n9.0.2-22-g46d8f462ee`,
  `autobuild-2026-10-03-18-14`, deleted about 2026-10-17). It passed every hosted check (the managed smoke,
  both P06 smokes, P07 local ASR), but it is 22 percent larger (137,945,828 bytes, 450,447,717 expanded), so
  the tar stream cap (400,000,000) and the XZ compressed-size cap (`MAX_XZ_ARCHIVE_BYTES`, 128 MiB) refuse it, and
  its recorded configuration gained `--enable-librsvg`, `--enable-lcms2` and `--enable-vapoursynth`. The next
  month-end build is 2026-10-31. Until then the catalogue stays at revision `ubuntu-24.04-x86_64-2026-09-22-r2`.
- **Evidence:** [`p06-ubuntu-artifact-candidate.md`](p06-ubuntu-artifact-candidate.md) and
  [`p06-windows-artifact-candidate.md`](p06-windows-artifact-candidate.md), sections of 2026-10-04 (hashes,
  inventory, configuration, runs 37164083942, 37164086257, 37164088495 and 37164090634); BtbN's README
  (retention policy, read 2026-10-04).
- **Impact:** a fix upstream takes after the next month-end waits for the one after it. The 95 commits
  between the shipped build and the candidate (17 on the supported path, by file name: hardening such as
  the `mov` key atom, VP8, VP9, H.264, HEVC and Opus decoder fixes and `libswscale` copies) are not in the
  shipped build. A daily pin would make every release that names it uninstallable about two weeks later
  (`DOWNLOAD_FAILED`; installed tools keep working), and the catalogue's stop date cannot honestly be that
  short.
- **Why:** direct download from the publisher is the reviewed design (ADR 0014: no mirror; a copy would redistribute an
  LGPL-3 build); the publisher's retention is not VSift's to change.
- **Mitigation:** the scan reading repeats at the candidate and the stable; the shipped build already
  carries the fixes for 46 of the 47 recorded records ([L-122](#l-122)); the strict-isolation container.
- **Existing installs are not moved.** `setup plan` offers an install only for a tool that is missing, so a
  managed FFmpeg that verifies stays selected after a new pin; its owner replaces it with `setup remove
  ffmpeg_ffprobe` and an install. A new pin protects new installs only, which matters if a build ever has to be
  replaced for a security reason. When a pinned asset disappears, existing installs keep working (local,
  hash-checked on use, no network needed for `list`, `check`, `rollback`, `remove` or `repair`) and only a
  fresh `setup install` of a release that names it fails (`DOWNLOAD_FAILED`).
- **Next step:** re-pin to the 2026-10-31 build once it exists, **before the candidate cut if the cut can wait
  for it** (a catalogue change makes every crates-scoped evidence item stale), otherwise **after the
  stable**, not between the two (the timing, the blockers and the existing-install behaviour are in the
  addendum's "The next pin"); raise the two bounds only as far as that archive needs and review the
  compiled-component inventory for the added libraries; or the maintainer decides to mirror the build (an
  ADR superseding ADR 0014's direct-origin rule, with the licence consequences).
- **Owner:** P14. **Issue:** [#272](https://github.com/smormah/vsift/issues/272). **Status:**
  open. **Review:** pending.

## Review workflow

1. **Review.** For each entry the maintainer sets its **Review** line to one of:
   - `accepted (YYYY-MM-DD)`: the limit stands as described; set the status to
     *accepted residual* if it was *open*;
   - `rejected (YYYY-MM-DD): <reason>`: the limit must be fixed; add or link an issue
     and an owner packet;
   - `rescheduled to <packet or issue> (YYYY-MM-DD)`: change **Owner** and **Issue**.
   Update the summary table in the same change.
2. **Closing.** An entry closes only with implementation and regression-test evidence
   (`AGENTS.md`). Delete it in the change that removes the limit, cite the PR in
   `CHANGELOG.md`, and never reuse its ID.
3. **New limits.** Documentation is part of the change: a pull request that finds or
   introduces a limitation, residual risk, deferral or accepted trade-off adds an entry
   here in the same pull request, alongside its ADR, qualification record or threat
   model note.
4. **Periodic sweep.** At each packet close, re-read the sources this register was built
   from (the work record and status, qualification records, ADR notes, threat model
   residuals, the CLI contract and open issues) and reconcile.
