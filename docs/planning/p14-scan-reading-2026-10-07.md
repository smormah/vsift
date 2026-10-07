# P14 scan reading, 2026-10-07 (RQ-13, R-SEC03), for the second release candidate 0.2.0-rc.2

Status: a dated record of what was read on 2026-10-07, the day the second candidate was published (09:43 UTC), at `main`
`7c722d1fc46a` (the tag `v0.2.0-rc.2`), for the published `0.2.0-rc.2`. It repeats the method of
[`p14-scan-reading-2026-10-05.md`](p14-scan-reading-2026-10-05.md), which in turn follows
[`p14-scan-reading-2026-10-02.md`](p14-scan-reading-2026-10-02.md) and its addendum of 2026-10-04; this record does not repeat
their method sections. Nothing was dismissed, closed, patched or changed in any setting, the FFmpeg and whisper.cpp
catalogue was not touched, nobody at the FFmpeg project was contacted, and nothing was installed on a developer machine.

**Result.** RQ-13 is recorded as **passed with the same residual the maintainer accepted on 2026-10-05**:
[#272](https://github.com/smormah/vsift/issues/272) (CVE-2026-38350, a High record tied to its fix by elimination only,
[L-122](known-limits.md#l-122)). **Nothing else was found in the sources of the earlier readings, and nothing changed in them
since 2026-10-05:** no FFmpeg or whisper.cpp record was published or modified in the National Vulnerability Database after the
last reading, the catalogue's tools are the same builds, and every other source is clean. **One new observation, outside those
sources, is not a finding of a rank the rule names and is recorded for the maintainer's decision:** the whisper.cpp project has
published three releases newer than the pinned v1.9.2 whose change lists include memory-safety hardening, with no CVE, no
advisory and no severity ([#322](https://github.com/smormah/vsift/issues/322), [L-137](known-limits.md#l-137)).

## How it was read

- **Cargo, the executable and the SBOM** by a hosted job: `P14 scan reading`, run
  [37611430394](https://github.com/smormah/vsift/actions/runs/37611430394) (dispatched from `main` with
  `version=0.2.0-rc.2`, passed): `cargo deny` 0.20.2 for the workspace and the fuzz crate, the published `vsift-cli`
  0.2.0-rc.2 from the real registry (`vsift 0.2.0-rc.2 (7c722d1fc46a)`), `readelf` on its Linux executable, the release's
  SBOM against `cargo metadata`.
- **GitHub** by read-only `gh api` calls (counts and states; nothing opened, dismissed or reopened) and the published
  advisories that name each pinned action at its pinned version.
- **FFmpeg and whisper.cpp** from the National Vulnerability Database's public interface (anonymous reads with the
  `curl` client's own identifier, no key, no contact detail; two windows of at most 120 days, 2026-06-01 to 2026-09-28 and
  2026-09-28 to 2026-10-08, merged into one array), GitHub's commit, compare and release endpoints through `gh api` (commit
  hashes and public release metadata only), and GitHub's repository advisories. Only identifiers, dates, severities,
  components and commit hashes are kept; no record's description is reproduced.
- **Runtime pins** from the Node.js release index, npm's registry (`npm view`) and the Docker registry's tag manifest for
  `ubuntu:24.04` (an anonymous token request and one manifest request).

## Source by source

| Source | What was read | Result | Disposition |
| --- | --- | --- | --- |
| Cargo | `cargo deny check` for the workspace and `cargo deny --all-features --manifest-path fuzz/Cargo.toml check`; the advisory summary as JSON | `advisories ok, bans ok, licenses ok, sources ok` for both; 0 advisory errors, warnings, notes or helps; the policy's own warnings only (six duplicate-version notes for the workspace: `base64`, `getrandom`, `io-lifetimes`, `r-efi`, `syn`, `windows-sys`, and one licence exception not encountered; four duplicate-version notes for the fuzz crate), as on 2026-10-05 | none needed |
| GitHub alerts | open and all-state code-scanning, Dependabot and secret-scanning alerts; the latest CodeQL analysis of `main` | 0 in each, in any state; the CodeQL analysis of `main` at `7c722d1` (2026-10-07) has 0 results over 28 rules | none needed. This reads the alert store, which has been empty throughout; job success is not a review |
| Pinned actions | the published advisories that name `actions/checkout` 7.0.1, `upload-artifact` 7.0.1, `download-artifact` 8.0.1, `setup-node` 7.0.0, `setup-python` 7.0.0, `oven-sh/setup-bun` 2.2.0, `attest-build-provenance` 4.2.2, `cargo-deny-action` 2.1.1, `dependency-review-action` 5.0.0 and `github/codeql-action` (pinned by commit `977e6cea`) | none for the first nine; the one advisory that names the `codeql-action` line (GHSA-vqf5-2xx6-9wfm, 3.26.11 to 3.28.2, patched in 3.28.3) does not apply to a 2026 commit | not affected. No pin changed since 2026-10-05 and every workflow pins by commit |
| whisper.cpp v1.9.2 and the `base` model | the National Vulnerability Database for records naming whisper.cpp, published 2026-06-01 to 2026-10-08; the older CVE-2025-14569 by identifier; the project's repository advisories | the same three records as on 2026-10-05 (CVE-2026-10298, -17512, -17513; all `Deferred`; last modified 2026-07-22, 2026-07-27 and 2026-07-27, so none changed since the last reading) plus CVE-2025-14569 (published 2025-12-12, last modified 2026-06-17, `Deferred`); none names 1.9.2; 0 repository advisories | no finding against 1.9.2 **in those sources**: the records are "not yet analysed", so this is the absence of a statement, not a clearance. **The release history is not clean: see the new observation below** |
| **FFmpeg** (BtbN snapshot `n9.0.1-11-ge47273f4d9`, 2026-08-31, the catalogue's build, unchanged since 0.1.0; upstream commit `e47273f4d9227152dcbf543cebaf9e2430ddbcc4`) | all 58 records the keyword `ffmpeg` finds, published 2026-06-18 to 2026-09-27 (none after 2026-09-27; the latest modification is CVE-2026-93302 on 2026-10-02, before the last reading); the tool `tools/p14-campaigns/ffmpeg-ancestry.cjs` run over all of them against the shipped snapshot | the same 58 as on 2026-10-05. 35 carry a commit reference of the FFmpeg project; for all of them every resolved fix is in the snapshot: 16 as an ancestor and 28 as a release-branch cherry-pick whose patch text equals the master commit's; **no fix is `absent`, `reverted` or `differs`**; one hash (CVE-2026-96611, a pull request's head commit) is not on the mirror, as before, and its merge commit was read on 2026-10-04. The other 23 have no commit reference; none of the 58 was modified since 2026-10-02, so the earlier readings of those stand. The upstream `release/9.0` branch still ends at `46d8f462ee` (2026-09-30) | one residual stands, unchanged: CVE-2026-38350, **tied by elimination** ([#272](https://github.com/smormah/vsift/issues/272), [L-122](known-limits.md#l-122)) |
| Runtime and build | the published Linux executable; Node.js and npm in the publish job; the worker image's base | `GLIBC_2.34` at most; needs `libssl.so.3`, `libcrypto.so.3` (`OPENSSL_3.0.0`), `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`, as in 0.1.0 and 0.2.0-rc.1. Node.js 24.21.0 (2026-09-07, ships npm 11.19.0) is the newest 24 release and 22.23.3 (2026-09-23) the newest 22; the newest security releases are 24.18.1 and 22.23.2 (2026-07-28), both older; no advisory names npm 11.19.0 (npm's registry has 11.21.0 under `next-11` and 12.2.0 as `latest`, neither bundled with a Node.js release). `ubuntu:24.04` points at `sha256:534baea6…` (as on 2026-10-05); the worker image's digest is still `sha256:a61567bd…` | the pins are current for Node.js; npm 11.19.0 is behind the registry's newest 11.x with no advisory against it. The base-image digest is older than the tag and the image's packages were **not scanned** (no scanner ran): the gap of 2026-10-02 stands |
| Inventory | the release's CycloneDX SBOM for the Linux executable against `cargo metadata --filter-platform x86_64-unknown-linux-gnu` at the tag | 164 library components; 165 crates resolved; the one difference is the root package `vsift-cli 0.2.0-rc.2`, which an SBOM names as its application; none unresolved in the other direction | none needed. The SBOM lists the Rust graph only: the native tools and the OpenSSL runtime are not in it |

## The new observation: whisper.cpp releases newer than the pinned v1.9.2 ([#322](https://github.com/smormah/vsift/issues/322))

The earlier readings looked for records and advisories, which do not exist for whisper.cpp 1.9.2, and did not compare the pin with the
project's release history. This reading did (GitHub's release list and compare endpoint; commit hashes and titles only):

- The catalogue pins **v1.9.2** (2026-08-04). Upstream has published **v1.9.3** (2026-08-20), **v1.9.4** (2026-09-11) and **v1.9.5**
  (2026-10-06); `compare/v1.9.2...v1.9.5` is 582 commits over 300 files, almost all of them ggml backends for other hardware.
- Their change lists name memory-safety hardening of code VSift's CPU path runs: a heap out-of-bounds read in the mel spectrogram for
  audio shorter than 201 samples (12.5 ms at 16 kHz; `8631825d`, merged 2026-08-07, in v1.9.3), a stack-buffer overflow on a malformed
  model file's tensor header (`df1547b6`, v1.9.3), and later input checks (a tensor type, the VAD model's layer count, an integer
  overflow in the parallel chunk offsets) in v1.9.4 and v1.9.5.
- **No CVE, advisory or severity exists** for any of them, so the rule of plan section 6 (no open high or critical finding) is not
  engaged, and nothing here is a finding of that rank.
- **Reachability was not assessed.** VSift writes each recognition chunk as a WAV and runs `whisper-cli` on it, with bounds on output,
  time and memory and, in a worker, strict isolation; the adapter sets no minimum length of its own, and whether the chunk planner can
  produce a chunk under 201 samples is not shown either way. The model is the pinned, hash-checked one, so the malformed-model fixes
  matter only for a user's own model. The effect of the read would be heap bytes read past a buffer in a child process.
- **What it means for the stable release:** the catalogue is frozen with the candidate (`crates/` may not change before the stable
  commit), so a re-pin before `0.2.0` would be a third candidate. The natural place is the FFmpeg re-pin planned for after the stable
  ([L-132](known-limits.md#l-132)). The maintainer decides: accept with [L-137](known-limits.md#l-137) as the register entry, or assess
  reachability first.

## What it does not show, and what is weaker than it sounds

- **A repeat, not a new method, for the FFmpeg row.** It is the same reading of public records and public history as 2026-10-04
  and 2026-10-05, run again two days later: its answer is the same because the inputs are the same (no new record, no modified
  record, the same build). It does not test a binary. Eight integer-overflow fixes in `libswscale/output.c` in five months say the
  class is not closed.
- **The records are a sample.** The database is neither complete nor timely and the keyword `ffmpeg` finds a record only if its
  text says so; records published before 2026-06-01 were not read.
- **Gaps carried over:** the worker image's packages are unscanned (the base digest has moved twice since it was chosen); the
  published Windows and macOS executables' system library requirements were not read (only Linux); the Homebrew tools of the
  macOS journeys and the pinned Windows tools are outside this reading; the native tools are not in the SBOM or in `cargo deny`.
- **A status, not a review of every alert:** zero alerts in a store that has been empty throughout shows no alert was raised,
  not that the code is free of findings.
- **A release list is not a vulnerability list.** The commit titles of whisper.cpp's newer releases say what upstream hardened; they
  do not say that the pinned build is exploitable, or that nothing else is wrong in it.

## Triage record

| Finding | Issue | Severity (proposed) | Disposition |
| --- | --- | --- | --- |
| The one FFmpeg record tied to its fix by elimination (CVE-2026-38350) | [#272](https://github.com/smormah/vsift/issues/272) | high (the record's) until accepted | open; accepted by the maintainer on 2026-10-05 with L-122 as the register entry |
| whisper.cpp releases 1.9.3 to 1.9.5 carry memory-safety hardening the pinned 1.9.2 lacks (no record, reachability not assessed) | [#322](https://github.com/smormah/vsift/issues/322) | none assigned (unassessed) | open; the maintainer chooses between accepting it with L-137 and assessing reachability first |

## Repeating it

`gh workflow run p14-scan-reading.yml --ref main -f version=<version>` (the hosted part), then the read-only calls above, within seven days of
the stable release's publish and before its plan: the ledger's RQ-13 entry for the stable cites the run and its own record.
For the FFmpeg row use the tool and the commands of the addendum of 2026-10-04 (two NVD windows of at most 120 days, merged
into one array, one `--snapshot shipped=<commit>`). For whisper.cpp also compare the pin with the project's release list.
