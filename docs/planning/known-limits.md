# Known limits register

Date: 2026-09-30 (P13 PR 6: managed lifecycle, L-037 narrowed and L-087 measured, L-090 added; P13 PR 4: managed installation; P13 PR 8: release archives, L-089 added and L-036 updated; P00-P12 complete; P12 closed on its final trial round with the compact tier below target, L-085; SEC-T01's adversarial evidence deferred as technical debt, L-068; P13 PR 2b closed L-073 and rewrote L-016 and L-017).
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
| [L-007](#l-007) | Evidence can carry instructions; agents can leak delivered paths | security | medium | unscheduled | [#222](https://github.com/smormah/vsift/issues/222) | accepted residual |
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
| [L-035](#l-035) | Evidence exists for Windows 11 only; macOS and Linux are unproven | platform/distribution | medium | P14 | [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-036](#l-036) | No native packages, npm launcher, SBOM, signing or provenance | platform/distribution | high | P13 | [#16](https://github.com/smormah/vsift/issues/16) | deferred |
| [L-037](#l-037) | Managed installation is qualified on Ubuntu 24.04 x64 only, and its kill and power-loss qualification is still to come | platform/distribution | low | P13 | [#16](https://github.com/smormah/vsift/issues/16) | deferred |
| [L-038](#l-038) | The worker host is a qualification target, not a supported platform | platform/distribution | medium | P11, P14 | [#14](https://github.com/smormah/vsift/issues/14), [#17](https://github.com/smormah/vsift/issues/17) | deferred |
| [L-040](#l-040) | Process-supervisor tests fail intermittently on Windows under load | process/CI | low | unscheduled | [#128](https://github.com/smormah/vsift/issues/128) | monitoring |
| [L-041](#l-041) | A creator slower than 5 s makes a racing command `BUSY` | process/CI | low | unscheduled | [#144](https://github.com/smormah/vsift/issues/144) | accepted residual |
| [L-042](#l-042) | Real-tool success paths run only on demand, not in hosted CI | process/CI | medium | P14 | [#178](https://github.com/smormah/vsift/issues/178) | open |
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
| [L-063](#l-063) | A workspace keeps at most 4,096 request records, pruned only when their session is gone | contract/UX | low | unscheduled | none | accepted residual |
| [L-064](#l-064) | A retain killed mid-copy leaves a staging directory in the bundle root | integrity/durability | low | unscheduled | none | accepted residual |
| [L-065](#l-065) | A request's deadline, admission wait and attempt count per delivery | contract/UX | low | unscheduled | none | accepted residual |
| [L-066](#l-066) | A batch file holds at most 1,000 lines; a longer file runs nothing | contract/UX | low | unscheduled | none | accepted residual |
| [L-067](#l-067) | Requests of one batch contend with each other; a job-cancelled line exits 6 | contract/UX | low | P11 | [#14](https://github.com/smormah/vsift/issues/14) | open |
| [L-068](#l-068) | SEC-T01 adversarial containment evidence deferred (technical debt) | security | high | maintainer discussion, before P14 | [#188](https://github.com/smormah/vsift/issues/188) | deferred (technical debt) |
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
| [L-085](#l-085) | The compact tier is below its 90% task-success target (82% on both clients) | contract/UX | high | maintainer, before P14 | [#218](https://github.com/smormah/vsift/issues/218)-[#222](https://github.com/smormah/vsift/issues/222), [#224](https://github.com/smormah/vsift/issues/224) | deferred (technical debt) |
| [L-086](#l-086) | `vsift-contract` embeds the skill's handoff schema from outside its crate folder, so it cannot be packaged for crates.io as it is | platform/distribution | low | unscheduled (before any crates.io publication, R1 or later) | none | deferred |
| [L-087](#l-087) | A managed version is rehashed each time a command resolves it | performance | low | unscheduled | none | monitoring |
| [L-088](#l-088) | Proxy authentication in an HTTPS tunnel is recognised by a dependency's error text | process/CI | low | unscheduled | none | monitoring |
| [L-089](#l-089) | A release SBOM's `bom-ref` values name the build machine's checkout path | platform/distribution | low | unscheduled | none | accepted residual |
| [L-090](#l-090) | Content in the managed folder that VSift cannot prove its own is left for the user to delete, and no output names the folder's path | platform/distribution | low | unscheduled | none | accepted residual |

Counts: 3 high, 24 medium, 56 low (83 entries).

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
- **Next step:** resolve [L-068](#l-068) before release; P14 malicious-decoder and
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
    In the counted A-04 and SEC-T02 runs (20, Claude Sonnet 5.5 and GPT-6-Sol) and in
    every reference round (348 graded phases), no agent installed or downloaded
    anything, ran a script, leaked a canary or acted on the hostile text. No counted
    report held a raw hidden character, a live link or an absolute path.
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
  supported models change (the next is the compact-tier re-run,
  [#222](https://github.com/smormah/vsift/issues/222)).
- **Owner:** unscheduled. **Issue:** [#222](https://github.com/smormah/vsift/issues/222).
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
  120 s by the chunk deadline it no longer enforces, usually a few seconds).
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
- **Impact:** a durable session on such storage is only as safe as the storage.
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
  sessions (`session clean --expired`, or their retention) before new requests run.
- **Why:** a bounded store, and replay must stay exact while its session exists.
- **Mitigation:** records expire with their sessions (at most 720 hours); `RESOURCE_LIMIT`
  is typed and names no path.
- **Next step:** none planned.
- **Owner:** unscheduled. **Issue:** none. **Status:** accepted residual.
  **Review:** pending.

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
  user) around the process supervisor. The adversarial evidence is technical debt and
  must be resolved before the R0 release.
- **Evidence:** [P11 qualification record](p11-worker-host.md) (SEC-T01);
  [verification](verification.md) "P11 PR 4 evidence"; the handoff document
  [sec-t01-adversarial-handoff.md](sec-t01-adversarial-handoff.md).
- **Impact:** strict isolation is attested and its controls are shown present, but
  that they contain a hostile decoder is not demonstrated; the strict worker profile
  cannot be released on this evidence.
- **Why:** maintainer decision (2026-09-28, option 2): the adversarial work is
  deferred for maintainer discussion.
- **Mitigation:** strict mode fails closed; the runbook's deployments apply the same
  controls as the CI container job; desktop profiles claim no isolation
  ([L-004](#l-004)).
- **Next step:** maintainer discussion (see the handoff document); resolved before P14.
- **Owner:** maintainer discussion, before P14. **Issue:** [#188](https://github.com/smormah/vsift/issues/188).
  **Status:** deferred (technical debt). **Review:** pending.

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

**Evidence exists for Windows 11 only; macOS and Linux are unproven.**

- **What:** the P08 and P09 real-tool checkpoints and all performance numbers were
  recorded on one Windows 11 machine. Local ASR was also measured on hosted Ubuntu 24.04
  and Windows Server 2025 runners. macOS runs only the ordinary Quality CI: no media,
  ASR or evidence checkpoint, and whisper.cpp v1.9.2 publishes no macOS CLI archive, so
  there is no reviewed macOS build. Linux desktop and other distributions, network
  filesystems, and Windows/macOS worker use are unqualified.
- **Evidence:** [resource profiles](support-and-resource-profiles.md);
  [P09 record](p09-evidence-navigation.md) residuals;
  [P07 ASR record](p07-asr-qualification.md); [P06 source review](p06-provisioning-source-review.md).
- **Impact:** no platform may be called "supported" yet; only "qualification target".
- **Why:** P14 owns the release matrix.
- **Mitigation:** cross-platform Quality CI on every PR.
- **Next step:** run the opt-in checkpoints on Ubuntu and macOS before P14; decide the
  macOS whisper.cpp route.
- **Owner:** P14. **Issue:** [#17](https://github.com/smormah/vsift/issues/17).
  **Status:** deferred. **Review:** pending.

### L-036

**No native packages, npm launcher, SBOM, signing or provenance.**

- **What:** VSift can only be installed by building it from source with Rust. No native
  release artifact is published, and there is no npm launcher, no signing or
  notarization, no provenance and no trusted publishing. Since P13 PR 8,
  `.github/workflows/release.yml` builds, checks and packages the three native archives
  with notices, an SBOM each and `SHA256SUMS`, but keeps them as run artifacts only.
  The platform packages' scope is recorded in ADR 0009's notes; no crate is
  published in R0 (ADR 0023).
- **Evidence:** [ADR 0009](../decisions/0009-package-identity-and-distribution.md);
  [ADR 0016](../decisions/0016-embeddable-engine-and-evidence-contract.md) decision 1;
  threat model "Installation and distribution policy"; baseline B-11.
- **Impact:** R-14 is an R0 release gate.
- **Why:** scheduled in P13.
- **Mitigation:** none needed before release.
- **Next step:** P13 PRs 9 and 10, following its launcher pattern and name checklist in
  [`implementation-work-packets.md`](implementation-work-packets.md) and
  [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md):
  Sigstore and npm provenance, no Authenticode or notarization in R0.
- **Owner:** P13. **Issue:** [#16](https://github.com/smormah/vsift/issues/16).
  **Status:** deferred. **Review:** pending.

### L-037

**Managed installation is qualified on Ubuntu 24.04 x64 only, and its kill and power-loss qualification is still to come.**

- **What:** on Ubuntu 24.04 x86-64, `setup install` applies an accepted plan (download
  or `--artifact-dir`, size and SHA-256, stage, smoke, activate), every command resolves
  tools through the managed tier, and since P13 PR 6 `setup list`, `setup rollback`,
  `setup remove` and `setup repair` manage what is installed; an accepted install sweeps
  stages that killed runs abandoned and keeps only the selected and previous version of
  each component. Kill and power-loss qualification of the managed store (PR 7) is not
  done: the operations are designed crash-consistent (one atomic rename per selection,
  the pointer last, removals and sweeps that a rerun finishes), but no crash campaign
  has exercised them. Windows x86-64 and macOS have no reviewed catalogue and keep
  manual guidance (the lifecycle commands work there and report the folder absent): the
  Windows FFmpeg candidate is a daily build (upstream keeps only the last 14) with
  unreconciled LGPL-3.0 notices, and no macOS candidate is reviewed.
- **Evidence:** [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
  PR 3, PR 4 and PR 6 notes; [CLI contract](../contracts/cli-v1.md) "P13 `setup install`"
  and "P13 managed lifecycle"; [verification](verification.md) D-02, D-03, D-05..D-08;
  [P06 Windows candidate](p06-windows-artifact-candidate.md) "Remaining gates".
  *2026-09-30:* `P13 managed smoke` [run 36734316384](https://github.com/smormah/vsift/actions/runs/36734316384) on `main` at `d43a518` (hosted `ubuntu-24.04`)
  passed both jobs: the pinned-tool smoke's negative control (banner mismatch)
  discarded all three stages and activated nothing, and the real `setup plan`, `setup
  install`, `setup check` and rerun through the CLI activated all three components.
  Managed installation is qualified on Ubuntu 24.04 x64 by that run.
- **Impact:** elsewhere users install FFmpeg and whisper.cpp themselves and register
  them; on Ubuntu, a power loss during an install, rollback or removal is not yet shown
  to be detected and repaired.
- **Why:** ADR 0023 decision E limits managed installation to the one reviewed target;
  PR 7 owns the kill and power-loss tests (decision H9).
- **Mitigation:** every managed version is verified by digest each time it is opened, so
  a torn version is never run and lookup falls through to `PATH`; `setup repair`
  diagnoses what an interruption leaves and names the command that fixes it; the manual
  path is typed on every target.
- **Next step:** P13 PR 7 (kill and power-loss tests of the managed store, the P13
  install E2E stage).
- **Owner:** P13. **Issue:** [#16](https://github.com/smormah/vsift/issues/16).
  **Status:** deferred. **Review:** pending.

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

### L-085

**The compact tier is below its 90% task-success target (82% on both clients).**

- **What:** [verification](verification.md) section 6 targets at least 90% task
  success on the agreed compact-model corpus, with 100% mechanically valid citations.
  In P12's final round on `8ab976e` (2026-09-30), Claude Sonnet 5.5 (Claude Code
  2.1.284) and GPT-6-Sol (codex-cli 0.155.0-alpha.16) each ran 28 trials of A-01 to
  A-07 and SEC-T02.
  - Each passed 23 of 28 fully (82%) and answered 25 of 28 correctly.
  - 3 of their 62 phases missed a citation check.
  - The maintainer closed P12 on these results (2026-09-30) and deferred the target
    as technical debt.
- **Causes** (the ten misses, [qualification record](p12-agent-qualification.md)):
  1. **Invented handoff shapes.** `SKILL.md`'s REPORT skeleton shows
     `"claims": []`, so models guess the claim's shape. This is 3 of Sonnet's 5
     misses (claims with `text` and no `id`) and 1 of Sol's (instructions with
     `citations` and `description`)
     ([#218](https://github.com/smormah/vsift/issues/218)).
  2. **A-02 resume** is the weakest scenario: Sonnet 2 of 3, Sol 1 of 3. Two of the
     three misses bind a true value to evidence outside the first loop's window
     ([#219](https://github.com/smormah/vsift/issues/219)). One of them (Sol, 490 s)
     was a grader error, fixed below; the other names a previous value without
     evidence for it.
  3. **SEC-T02 slips on Sol**, 2 of 5 full passes: a frame cited after `session
     retain`, the instruction shape above, and SAFE-12 never stated
     ([#220](https://github.com/smormah/vsift/issues/220)).
  4. **The skill's defanged link form** (`hxxps://`) written in a JSON summary, which
     the schema refuses (Sonnet A-04 run 2,
     [#221](https://github.com/smormah/vsift/issues/221)).
  5. **Strong tier, A-09 blurred:** the review tier stated the blurred banner's content
     as supported by pixels in 3 of 4 runs, rejected on review
     ([#224](https://github.com/smormah/vsift/issues/224)).
- **Evidence:** the [qualification record](p12-agent-qualification.md) (per-scenario
  tables and every miss); the bounded records in
  [p12-agent-trials/](p12-agent-trials/README.md); verification section 6.
- **Impact:** a user of a compact model gets a fully valid, correct handoff about four
  times in five, and a correct answer about nine times in ten. The failures are
  handoff-shape and citation-binding errors, not unsafe actions: every counted phase
  passed the command policy, the canary check and the report-text check.
  - The review tier passed 11 of 11 trials mechanically on both clients: Claude Opus
    5.5 and GPT-6-Astra.
  - After the maintainer's review of 25 runs (2026-09-30), A-08 passed 5 of 5 on both
    clients. A-08 plus A-09 passed 9 of 11 (Opus) and 10 of 11 (Astra). The residual is
    three rejected blurred-banner claims stating unreadable content as supported by
    pixels ([#224](https://github.com/smormah/vsift/issues/224)).
- **Why:** the maintainer decided on 2026-09-30 to close P12 on the final round's
  results and record what is short of target as known limits and follow-ups.
- **Mitigation added (P13 PR 5, 2026-09-30):** `vsift handoff check` (#213). The
  skill now runs it once on the draft before sending and fixes what it reports: the
  closed vocabulary, the claim and citation shapes, missing members, a missing resume
  card, links and paths, each named by pointer or line with the allowed values. The
  check is the grader's own, so a draft that passes it passes `handoff_valid` and
  `report_text`. The compact-tier re-run that measures the effect (#222) is still to
  come; the limit stays open until then.
- **Fixes so far** (2026-09-30, ADR 0022 note "the P12 debt fixes"; no model called):
  1. **#218:** the REPORT skeleton shows one filled-in claim, a segment and a frame
     citation, and one untrusted instruction, all validated by the guard.
  2. **#219, grader:** the A-02 clip's copies start 12.064 s apart, not 12 s (FFmpeg
     places each copy after the padded audio). The grader now derives the period from
     the bundle's measured duration. Sol's A-02 miss at 490 s was a true claim that
     the grader misplaced.
  3. **#219, skill:** each claim names its subject and value. A value mentioned as a
     previous state ("change from 12") still needs its own evidence; that reading
     stays strict.
  4. **#220:** retain after the last evidence command, because the bundle is a
     snapshot.
  5. **#221:** a defanged link belongs only in the Markdown; the JSON describes the
     link.
  6. **#224:** a region a frame shows as unreadable supports nothing; a claim about
     its content is `partially_supported` on the transcript.
- **Re-grade** of the 84 counted phases with the new grader (`grade-debt.json`): only
  Sol's A-02 run 2 changes. GPT-6-Sol is now 24 of 28 (86%); Sonnet 5.5 stays at
  23 of 28 (82%). Compact citation failures fall from 3 to 2 of 62 phases.
- **Mitigation:**
  - `docs/agents/skill.md` names the supported models and says the compact tier is
    below target;
  - the grader reports each handoff error as a typed, fixable message;
  - the handoff validator (P13, [#213](https://github.com/smormah/vsift/issues/213))
    may lower handoff failures further.
- **Next step:** the compact re-run
  ([#222](https://github.com/smormah/vsift/issues/222)) is still pending. It runs
  after `vsift handoff check` (#213) exists, before P14, and shows whether the skill
  changes reach the 90% target. It also re-runs A-09 blurred on the strong tier
  (#224).
- **Owner:** maintainer, before P14. **Issue:**
  [#218](https://github.com/smormah/vsift/issues/218)-[#222](https://github.com/smormah/vsift/issues/222).
  **Status:** deferred (technical debt). **Review:** pending.

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

**Real-tool success paths run only on demand, not in hosted CI.**

- **What:** the success paths that use real FFmpeg, whisper.cpp or large media are
  opt-in (`--ignored`): `p07_transcript_e2e`, `p07_local_asr_e2e`,
  `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`, `p09_evidence_e2e`
  and the listed engine tests. Only the P07 local ASR workflow runs some of them on
  hosted runners, on demand. Long fuzz campaigns (weekly short runs today), soak and the
  load ladder are P14 gates.
- **Evidence:** [work record](../../memory/TODO.md) "Known issues and gates";
  [verification](verification.md) section 7 CI tiers.
- **Impact:** a regression in a real-tool path is found only when someone runs the
  checkpoint.
- **Why:** local models and sizeable media; runner cost (decided in the test spine's
  execution policy).
- **Mitigation:** recorded samples keep the P08 recall gate in every CI run; stand-in
  tools cover contracts.
- **Next step:** consider a scheduled hosted run of the P08/P09 checkpoints.
- **Owner:** P14 (release runs); scheduled runs unscheduled. **Issue:** [#178](https://github.com/smormah/vsift/issues/178).
  **Status:** open. **Review:** pending.

### L-043

**Library API unstable; MSRV and MCP decisions open.**

- **What:** the `vsift` library API is 0.x and unstable; the MSRV equals the latest
  stable release with no policy; `cargo-semver-checks` joins CI only at first
  publication. Open maintainer decisions: an MSRV policy before publication and whether
  a local MCP adapter is wanted after P12. Decided 2026-09-30 (ADR 0023): one 0.x
  pre-release under npm's `next` tag at P13 completion, and no crates.io publication in
  R0.
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
