# P14 scan reading, 2026-10-08 (RQ-13, R-SEC03), for the third release candidate 0.2.0-rc.3

Status: a dated record of what was read on 2026-10-08, the day the third candidate was published (publish run 37746979716,
started 08:00 UTC), at `main` `83dca856e7a0` (the tag `v0.2.0-rc.3`), for the published `0.2.0-rc.3`. It repeats the
method of [`p14-scan-reading-2026-10-07.md`](p14-scan-reading-2026-10-07.md), which follows
[`p14-scan-reading-2026-10-05.md`](p14-scan-reading-2026-10-05.md) and
[`p14-scan-reading-2026-10-02.md`](p14-scan-reading-2026-10-02.md) with its addendum of 2026-10-04; this record does not repeat their method
sections. Nothing was dismissed, closed, patched or changed in any setting, the FFmpeg and whisper.cpp catalogue was not touched, nobody at
the FFmpeg or whisper.cpp projects was contacted, and nothing was installed on a developer machine.

**Result.** RQ-13 is recorded as **passed with the same residual the maintainer accepted on 2026-10-05**:
[#272](https://github.com/smormah/vsift/issues/272) (CVE-2026-38350, a High record tied to its fix by elimination only,
[L-122](known-limits.md#l-122)). **Nothing new was found, and the sources of the earlier readings did not change in substance since
2026-10-07:** no FFmpeg or whisper.cpp record was published in the National Vulnerability Database after the last reading, the catalogue's
tools are the same builds, no dependency, workflow, policy or toolchain file changed between the second and third candidates (the
lockfile differs only in the eight workspace version strings), and every other source is clean. Two small movements, neither a finding, are
in the table: the upstream FFmpeg `release/9.0` branch gained one commit, and one old whisper.cpp record was touched for a translation.
The observation of 2026-10-07 ([#322](https://github.com/smormah/vsift/issues/322), [L-137](known-limits.md#l-137)) is narrowed by this
candidate: the one upstream fix that was reachable from VSift (a heap read for 1 to 200 samples of audio) now has a floor of 100 ms in
front of it; the re-pin of whisper.cpp stays after the stable release.

## How it was read

- **Cargo, the executable and the SBOM** by a hosted job: `P14 scan reading`, run
  [37752978771](https://github.com/smormah/vsift/actions/runs/37752978771) (dispatched from `main` with `version=0.2.0-rc.3`, passed):
  `cargo deny` 0.20.2 for the workspace and the fuzz crate, the published `vsift-cli` 0.2.0-rc.3 from the real registry
  (`vsift 0.2.0-rc.3 (83dca856e7a0)`), `readelf` on its Linux executable, the release's SBOM against `cargo metadata`. The duplicate-version
  notes are not printed in that job's log; they were counted by running the same `cargo deny` 0.20.2 on the same tree by hand (below).
- **GitHub** by read-only `gh api` calls (counts and states; nothing opened, dismissed or reopened) and the published advisories that name
  each pinned action at its pinned version.
- **FFmpeg and whisper.cpp** from the National Vulnerability Database's public interface (anonymous reads with the `curl` client's own
  identifier, no key, no contact detail; two windows of at most 120 days, 2026-06-01 to 2026-09-28 and 2026-09-28 to 2026-10-08, merged
  into one array), the same database's public change history for the one old record that moved, GitHub's commit, compare and release
  endpoints through `gh api` (commit hashes and public release metadata only) and GitHub's repository advisories. Only identifiers, dates,
  severities, components and commit hashes are kept; no record's description is reproduced.
- **Runtime pins** from the Node.js release index, npm's registry (`npm view`) and the Docker registry's tag manifest for `ubuntu:24.04` (an
  anonymous token request and one manifest request).

## Source by source

| Source | What was read | Result | Disposition |
| --- | --- | --- | --- |
| Cargo | `cargo deny check` for the workspace and `cargo deny --all-features --manifest-path fuzz/Cargo.toml check` (the hosted job, and by hand the same version on the same tree) | `advisories ok, bans ok, licenses ok, sources ok` for both; 0 advisory errors, warnings or notes; the policy's own warnings only (six duplicate-version notes for the workspace: `base64`, `getrandom`, `io-lifetimes`, `r-efi`, `syn`, `windows-sys`, and one licence exception not encountered; four duplicate-version notes for the fuzz crate: `base64`, `io-lifetimes`, `syn`, `windows-sys`), as on 2026-10-07. `Cargo.lock` differs from the second candidate's in the eight workspace version strings only | none needed |
| GitHub alerts | open and all-state code-scanning, Dependabot and secret-scanning alerts; the latest CodeQL analysis of `main` | 0 in each, in any state; the CodeQL analysis of `main` at `83dca856e7a0` (2026-10-08, 05:52 UTC) has 0 results over 28 rules | none needed. This reads the alert store, which has been empty throughout; job success is not a review |
| Pinned actions | the published advisories that name `actions/checkout` 7.0.1, `upload-artifact` 7.0.1, `download-artifact` 8.0.1, `setup-node` 7.0.0, `setup-python` 7.0.0, `oven-sh/setup-bun` 2.2.0, `attest-build-provenance` 4.2.2, `cargo-deny-action` 2.1.1, `dependency-review-action` 5.0.0 and `github/codeql-action` (pinned by commit `977e6cea`) | none for the first nine; the one advisory that names the `codeql-action` line (GHSA-vqf5-2xx6-9wfm, 3.26.11 to 3.28.2, patched in 3.28.3) does not apply to a 2026 commit | not affected. Nothing under `.github/`, `deny.toml` or `rust-toolchain.toml` differs between the tags `v0.2.0-rc.2` and `v0.2.0-rc.3`, and every workflow pins by commit |
| whisper.cpp v1.9.2 and the `base` model | the National Vulnerability Database for records naming whisper.cpp, published 2026-06-01 to 2026-10-08; the older CVE-2025-14569 by identifier and by its change history; the project's repository advisories and release list | the same three records as on 2026-10-07 (CVE-2026-10298, -17512, -17513; all `Deferred`; last modified 2026-07-22, 2026-07-27 and 2026-07-27, so none changed) plus CVE-2025-14569 (published 2025-12-12, `Deferred`, **last modified 2026-10-07 20:10 UTC, after the last reading**: the public change history shows that entry as "Added Translation" by NVD, after the vendor-side change of 2026-06-17; its affected range is "up to 1.8.2", its severities are Low, Medium and Medium and it does not name 1.9.2); 0 repository advisories; the newest release is still v1.9.5 (2026-10-06) | no finding against 1.9.2 **in those sources**: the records are "not yet analysed", so this is the absence of a statement, not a clearance. The release history is not clean (the observation below) |
| **FFmpeg** (BtbN snapshot `n9.0.1-11-ge47273f4d9`, 2026-08-31, the catalogue's build, unchanged since 0.1.0; upstream commit `e47273f4d9227152dcbf543cebaf9e2430ddbcc4`) | all 58 records the keyword `ffmpeg` finds, published 2026-06-18 to 2026-09-27 (none in the window 2026-09-28 to 2026-10-08; the latest modification is CVE-2026-93302 on 2026-10-02, before the reading of 2026-10-07); the tool `tools/p14-campaigns/ffmpeg-ancestry.cjs` run over all of them against the shipped snapshot | the same 58 as on 2026-10-07. 35 carry a commit reference of the FFmpeg project; for all of them every resolved fix is in the snapshot: 16 as an ancestor and 28 as a release-branch cherry-pick whose patch text equals the master commit's; **no fix is `absent`, `reverted` or `differs`**; one hash (CVE-2026-96611, a pull request's head commit) is not on the mirror, as before, and its merge commit was read on 2026-10-04. The other 23 have no commit reference; none of the 58 was modified since 2026-10-02, so the earlier readings of those stand. **The upstream `release/9.0` branch moved:** it ended at `46d8f462ee` (2026-09-30) and now ends at `27b46f0fbc` (2026-10-08), one commit ahead, whose title (all that was read) is a fix to a Vorbis encoder's frame size: no record names it | one residual stands, unchanged: CVE-2026-38350, **tied by elimination** ([#272](https://github.com/smormah/vsift/issues/272), [L-122](known-limits.md#l-122)). The new branch commit is not a finding: no record, no severity, and the refresh to a month-end build waits for 2026-10-31 ([L-132](known-limits.md#l-132)) |
| Runtime and build | the published Linux executable; Node.js and npm in the publish job; the worker image's base | `GLIBC_2.34` at most; needs `libssl.so.3`, `libcrypto.so.3` (`OPENSSL_3.0.0`), `libgcc_s.so.1`, `libm.so.6`, `libc.so.6` and the loader, as in 0.1.0 and the earlier candidates. Node.js 24.21.0 (2026-09-07, ships npm 11.19.0) is the newest 24 release and 22.23.3 (2026-09-23) the newest 22; the newest security releases are 24.18.1 and 22.23.2 (2026-07-28), both older; no advisory names npm 11.19.0 (npm's registry has 11.21.0 under `next-11` and 12.2.0 as `latest`, neither bundled with a Node.js release). `ubuntu:24.04` points at `sha256:534baea6…` (as on 2026-10-07); the worker image's digest is still `sha256:a61567bd…` | the pins are current for Node.js; npm 11.19.0 is behind the registry's newest 11.x with no advisory against it. The base-image digest is older than the tag and the image's packages were **not scanned** (no scanner ran): the gap of 2026-10-02 stands |
| Inventory | the release's CycloneDX SBOM for the Linux executable against `cargo metadata --filter-platform x86_64-unknown-linux-gnu` at the tag | 164 library components; 165 crates resolved; the one difference is the root package `vsift-cli 0.2.0-rc.3`, which an SBOM names as its application; none unresolved in the other direction | none needed. The SBOM lists the Rust graph only: the native tools and the OpenSSL runtime are not in it |

## The whisper.cpp observation, after the third candidate's floor ([#322](https://github.com/smormah/vsift/issues/322))

- The catalogue still pins **v1.9.2** (2026-08-04). Upstream's newest release is still **v1.9.5** (2026-10-06); there is no release since the
  last reading and no CVE, advisory or severity for any of the hardening in v1.9.3 to v1.9.5, so the rule of plan section 6 (no open high or
  critical finding) is not engaged.
- **What changed in VSift:** the third candidate carries the floor that the maintainer's decision of 2026-10-07 ordered for after the stable
  release and then brought forward ([#330](https://github.com/smormah/vsift/pull/330), [L-137](known-limits.md#l-137)): a chunk whose window is
  shorter than 100 ms is recorded as a gap without being decoded, and decoded audio of fewer than 1,600 samples is never given to the
  recogniser. That closes the one fix the reachability assessment found reachable (`8631825d`, a heap read past the audio buffer for 1 to 200
  samples) for every whisper.cpp build, a user's own included. **What was not shown:** nothing ran whisper.cpp against a short range (the
  `P07 local ASR` run of this candidate has no stage for one), so the floor is shown by tests with a recogniser that counts its calls and by
  opt-in tests with a real FFmpeg, not by the real recogniser. The model-file fixes, VAD, the parallel path and the loader stay out of reach
  as assessed on 2026-10-07 (only the two pinned, hash-checked models run); the **pin itself is unchanged** and the re-pin comes with the
  FFmpeg refresh after `0.2.0`.

## What it does not show, and what is weaker than it sounds

- **A repeat, not a new method, for the FFmpeg row.** It is the same reading of public records and public history as 2026-10-04, 2026-10-05
  and 2026-10-07, run again a day later: its answer is the same because the inputs are the same (no new record, no modified record, the same
  build). It does not test a binary. Eight integer-overflow fixes in `libswscale/output.c` in five months say the class is not closed.
- **The records are a sample.** The database is neither complete nor timely and the keyword `ffmpeg` finds a record only if its text says
  so; records published before 2026-06-01 were not read.
- **Gaps carried over:** the worker image's packages are unscanned (the base digest has moved twice since it was chosen); the published
  Windows and macOS executables' system library requirements were not read (only Linux); the Homebrew tools of the macOS journeys and the
  pinned Windows tools are outside this reading; the native tools are not in the SBOM or in `cargo deny`.
- **A status, not a review of every alert:** zero alerts in a store that has been empty throughout shows no alert was raised, not that the
  code is free of findings.
- **A release list is not a vulnerability list.** The commit titles of whisper.cpp's newer releases and of FFmpeg's branch say what upstream
  changed; they do not say that the pinned build is exploitable, or that nothing else is wrong in it.
- **The one branch commit was read by its title only**, as the method keeps to hashes, dates and titles.

## Triage record

| Finding | Issue | Severity (proposed) | Disposition |
| --- | --- | --- | --- |
| The one FFmpeg record tied to its fix by elimination (CVE-2026-38350) | [#272](https://github.com/smormah/vsift/issues/272) | high (the record's) until accepted | open; accepted by the maintainer on 2026-10-05 with L-122 as the register entry |
| whisper.cpp releases 1.9.3 to 1.9.5 carry memory-safety hardening the pinned 1.9.2 lacks; the one reachable fix (a heap read in the child for 1 to 200 samples of non-silent audio) now has a 100 ms floor in front of it | [#322](https://github.com/smormah/vsift/issues/322) | none assigned (no record; assessed from source on 2026-10-07) | the issue stays open for the re-pin; accepted for R0 by the maintainer on 2026-10-07 with L-137 as the register entry, narrowed by the floor in this candidate; the re-pin follows the stable release |

## Repeating it

`gh workflow run p14-scan-reading.yml --ref main -f version=<version>` (the hosted part), then the read-only calls above, within seven days of
the stable release's publish and before its plan: the ledger's RQ-13 entry for the stable cites the run and its own record.
For the FFmpeg row use the tool and the commands of the addendum of 2026-10-04 (two NVD windows of at most 120 days, merged
into one array, one `--snapshot shipped=<commit>`). For whisper.cpp also compare the pin with the project's release list.
