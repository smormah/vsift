# P14 scan reading, 2026-10-10 (RQ-13, R-SEC03), for the release 0.2.0

Status: a dated record of what was read on 2026-10-10, the day after `0.2.0` was published (publish run 37946261087, 2026-10-09), at `main`
`1daf84097041` (which differs from the stable commit `eeb2a22a46a8` only in the work record: the comparison is below), for the published
`0.2.0`. It repeats the method of [`p14-scan-reading-2026-10-08.md`](p14-scan-reading-2026-10-08.md) for the third candidate, which follows
[`p14-scan-reading-2026-10-07.md`](p14-scan-reading-2026-10-07.md), [`-10-05`](p14-scan-reading-2026-10-05.md) and
[`-10-02`](p14-scan-reading-2026-10-02.md) with its addendum of 2026-10-04; this record does not repeat their method sections. **It was made in two
passes on the same day:** the first read only this repository's data and the hosted run's artifacts (the session was not allowed anything
outside them); the second, authorised by the supervisor, read the public sources of the third candidate's method and is the one this record
reports. Nothing was dismissed, closed, patched or changed in any setting, the FFmpeg and whisper.cpp catalogue was not touched, nobody at the
FFmpeg or whisper.cpp projects was contacted, nothing was posted, opened or commented anywhere by the reading (the tracking issue #351 was filed afterwards by the maintainer's decision), and nothing was installed on a developer machine.

**Result: RQ-13 is recorded `passed` for `0.2.0`, with one new finding named and accepted by the maintainer.** Everything else agrees with 2026-10-08. The finding: nine public records naming FFmpeg were published on 2026-10-08 between 16:17 and 18:17 UTC, after
the reading of the third candidate. **Seven are outside what VSift's configuration reaches or are fixed in the shipped snapshot; one is a
serializer VSift never calls; one is not:** [CVE-2026-107678](https://nvd.nist.gov/vuln/detail/CVE-2026-107678) (Medium: CVSS 4.0 5.7, 3.1 4.7;
CWE-674, uncontrolled recursion; the record's source is VulnCheck) is a stack exhaustion in `av_encryption_init_info_free()` that the **MP4 demuxer
reaches through `pssh` boxes** (`libavformat/mov.c`), and VSift enables that demuxer for every MP4 or MOV file it ingests (`demuxers: &["mov",
"matroska"]`, `crates/vsift-infrastructure/src/ffmpeg_media.rs`). The record's versions are "through 9.0.2", which includes the shipped snapshot (2026-08-28,
between 9.0.1 and 9.0.2), and no commit on `master` since the snapshot touches the recursion (the record names no commit; its pull request was not read). It is a
denial-of-service class record (the FFmpeg child crashes; VSift bounds the child's time, memory and output, and a worker runs it in the strict
container) and **it is not high or critical**, so the plan's rule (section 6: no open high or critical finding on a supported path) is not breached by it.
**The maintainer decided on 2026-10-10 to accept it for R0 and fix it after the stable release** (register entry [L-144](known-limits.md#l-144), tracking issue
[#351](https://github.com/smormah/vsift/issues/351)), the way the two standing residuals were accepted, which are unchanged: [#272](https://github.com/smormah/vsift/issues/272)
(CVE-2026-38350, [L-122](known-limits.md#l-122), accepted 2026-10-05) and [#322](https://github.com/smormah/vsift/issues/322) (the pinned
whisper.cpp, [L-137](known-limits.md#l-137), accepted 2026-10-07).

## How it was read

- **Cargo, the executable and the SBOM** by a hosted job: `P14 scan reading`, run
  [38007675152](https://github.com/smormah/vsift/actions/runs/38007675152) (dispatched from `main` with `version=0.2.0`, passed in about two and a half
  minutes): `cargo deny` 0.20.2 for the workspace and the fuzz crate, the published `vsift-cli` 0.2.0 from the real registry
  (`vsift 0.2.0 (eeb2a22a46a8)`), `readelf` on its Linux executable, and the release's SBOM against `cargo metadata`. The job's artifact also holds the
  duplicate-version notes, so they were counted from it and not by a second run of `cargo deny` by hand.
- **GitHub** by read-only `gh api` calls: this repository's alert counts in each state and the CodeQL analyses of `main` (nothing opened, dismissed or
  reopened); the published advisories that name each pinned action at its pinned version; the public commit, compare, branch, tag and release endpoints of
  the FFmpeg and whisper.cpp repositories (commit hashes, dates and the first line of a commit message only); GitHub's code search for the callers of two
  functions; and the whisper.cpp repository advisories.
- **FFmpeg and whisper.cpp** from the National Vulnerability Database's public interface (anonymous reads with the `curl` client's own identifier, no
  key, no contact detail, `curl -q` so that no configuration file is read; two windows of at most 120 days, 2026-06-01 to 2026-09-28 and 2026-09-28
  to 2026-10-10, merged), the same database's public change history for the one old record that moved, and `tools/p14-campaigns/ffmpeg-ancestry.cjs`
  over all 67 records against the shipped snapshot. Only identifiers, dates, severities, components and commit hashes are kept; no record's description
  is reproduced.
- **Runtime pins** from the Node.js release index, npm's registry (`npm view`, with an empty user and global configuration and a fresh cache folder, so that
  none of the maintainer's configuration was read) and the Docker registry's anonymous token and tag manifest for `ubuntu:24.04`.

## Source by source

| Source | What was read | Result | Disposition |
| --- | --- | --- | --- |
| Cargo | `cargo deny check` for the workspace and `cargo deny --all-features --manifest-path fuzz/Cargo.toml check` (the hosted job); the advisory summary of the job's JSON output | `advisories ok, bans ok, licenses ok, sources ok` for both; 0 advisory errors, warnings or notes; the policy's own warnings only: six duplicate-version notes for the workspace (`base64`, `getrandom`, `io-lifetimes`, `r-efi`, `syn`, `windows-sys`) and one licence exception not encountered, and four for the fuzz crate (`base64`, `io-lifetimes`, `syn`, `windows-sys`), the same as on 2026-10-08. `Cargo.lock` and `fuzz/Cargo.lock` differ from the third candidate's in the workspace version strings only (24 changed lines, every removed line equal to its added twin once `0.2.0-rc.3` is read as `0.2.0`) | none needed |
| GitHub alerts | code-scanning alerts (open, dismissed, fixed), Dependabot alerts (open, fixed, dismissed) and secret-scanning alerts (open, resolved); the CodeQL analyses of `main` | 0 in each state of each store; the CodeQL analyses of `main` at `eeb2a22a46a8` (2026-10-09, 14:30 UTC, the stable commit), at `1daf84097041` and at `61abec934091` each have 0 results over 28 rules and no error | none needed. This reads the alert store, which has been empty throughout; job success is not a review |
| Pinned actions | the published advisories that name `actions/checkout` 7.0.1, `upload-artifact` 7.0.1, `download-artifact` 8.0.1, `setup-node` 7.0.0, `setup-python` 7.0.0, `oven-sh/setup-bun` 2.2.0, `attest-build-provenance` 4.2.2, `cargo-deny-action` 2.1.1, `dependency-review-action` 5.0.0 (each by `affects=<action>@<version>`) and `github/codeql-action` (pinned by commit `977e6cea`, tag `codeql-bundle-v2.27.0`) | none for the first nine; the one advisory that names the `codeql-action` line (GHSA-vqf5-2xx6-9wfm, high, 2025-01-24: 3.26.11 to 3.28.2, patched in 3.28.3) does not apply to a 2026 commit. Nothing under `.github/`, `deny.toml` or `rust-toolchain.toml` differs between `v0.2.0-rc.3` and the stable commit, and every workflow pins by commit | not affected |
| whisper.cpp v1.9.2 and the `base` model | the National Vulnerability Database for records naming whisper.cpp, published 2026-06-01 to 2026-10-10 (two windows) and the older CVE-2025-14569 by identifier and by its change history; the project's repository advisories, releases and tags | the same three records as on 2026-10-08 (CVE-2026-10298, -17512, -17513; all `Deferred`, Low; last modified 2026-07-22, 2026-07-27 and 2026-07-27, so none changed; none in the second window) plus CVE-2025-14569 (published 2025-12-12, `Deferred`, last modified 2026-10-07 20:10 UTC, the translation already read on 2026-10-08: its change history has nothing newer; its severities are Low, Medium and Medium and it does not name 1.9.2); 0 repository advisories; the newest release is still v1.9.5 (2026-10-06), tags v1.9.5, v1.9.4, v1.9.3, v1.9.2 | no finding against 1.9.2 **in those sources**: the records are "not yet analysed", so this is the absence of a statement, not a clearance. The residual of #322 stands, narrowed by the 100 ms floor of the third candidate (#330); the re-pin follows the stable release |
| **FFmpeg** (BtbN snapshot `n9.0.1-11-ge47273f4d9`, 2026-08-31, upstream commit `e47273f4d9227152dcbf543cebaf9e2430ddbcc4` of 2026-08-28, the catalogue's build, unchanged) | all 67 records the keyword `ffmpeg` finds, published 2026-06-18 to 2026-10-08; the ancestry tool over all of them against the shipped snapshot; the upstream `release/9.0` branch, the tags and the releases | **The 58 records of 2026-10-08 are unchanged**: none published after 2026-09-27, none modified after 2026-10-02 (CVE-2026-38350, the standing residual, last modified 2026-09-09); 35 carry a commit reference, and for all of them every resolved fix is in the snapshot (16 as an ancestor and 28 as a cherry-pick whose patch text is identical; none `absent`, `reverted` or `differs`). **Nine are new, all published 2026-10-08 (VulnCheck's records): the table below.** The `release/9.0` branch moved: it ended at `27b46f0fbc` (2026-10-08) and now ends at `67b60c310b` (2026-10-09), two commits ahead (a hardware-decoder data race and a busy loop in the IMF demuxer; titles only; no record names either); the tag `n9.0.2` exists | **one new finding: CVE-2026-107678**, see below. The residual of #272 stands as accepted on 2026-10-05 |
| Runtime and build | the published Linux executable (the hosted job); Node.js, npm and the base image | `GLIBC_2.34` at most; needs `libssl.so.3`, `libcrypto.so.3` (`OPENSSL_3.0.0`), `libgcc_s.so.1`, `libm.so.6`, `libc.so.6` and the loader, as in `0.1.0` and every candidate. Node.js 24.21.0 (2026-09-07, ships npm 11.19.0) is still the newest 24 release and 22.23.3 (2026-09-23) the newest 22; the newest security releases are 24.18.1 and 22.23.2 (2026-07-28), both older (the index's newest release overall is v26.11.1); npm's registry has 11.21.0 under `next-11` and 12.2.0 under `next-12`, neither bundled with a Node.js release, and 11.19.0 is not deprecated; `ubuntu:24.04` still points at `sha256:534baea6…`; the worker image's digest is still `sha256:a61567bd…` | the pins are current for Node.js; npm 11.19.0 is behind the registry's newest 11.x with no advisory against it. The base-image digest is older than the tag and the image's packages were **not scanned** (no scanner ran): the gap of 2026-10-02 stands |
| Inventory | the release's CycloneDX SBOM for the Linux executable against `cargo metadata --filter-platform x86_64-unknown-linux-gnu` | 164 library components; 165 crates resolved; the one difference is the root package `vsift-cli 0.2.0`, which an SBOM names as its application; none unresolved in the other direction | none needed. The SBOM lists the Rust graph only: the native tools and the OpenSSL runtime are not in it |

**The tree the job read is the stable commit's.** `git diff` between `eeb2a22a46a8` and `1daf84097041` over `crates`, `tools`, `npm`, `skills`,
`schemas`, `fixtures`, `.github`, `Cargo.lock`, `fuzz/Cargo.lock`, `deny.toml` and `rust-toolchain.toml` is empty (what changed between them is the
changelog, the decision record, the register, the ledger, the plan, the checklist and the two handoff files), and between `v0.2.0-rc.3` and
`eeb2a22a46a8` the only differences under those paths are the two lockfiles, the launcher's `package.json` (the version) and its README.

## The nine new FFmpeg records (published 2026-10-08, after the third candidate's reading)

The configuration is the one of the first reading: FFmpeg and FFprobe run with the demuxers `mov` and `matroska` and the protocol `file` only
(`-protocol_whitelist file`), bounded resources, and decoders only after a bounded probe; the output is PNG and WAV. `fixed` is on the shipped
snapshot's own line (an ancestor, or a cherry-pick whose patch text is identical); the other hashes the records list are the fixes for other release
lines and are `absent` by construction.

| Record (CVE-2026-) | Severity (the record's source) | Names | Fix evidence | In the shipped snapshot | Reach in VSift |
| --- | --- | --- | --- | --- | --- |
| 107660 | Medium (4.0: 6.3; 3.1: 4.8) | the TLS protocol (`tls_open`, certificate validation) | `0cd2a70eac`, `57a2e704b4`, `d377212904` (2026-09-12 to 2026-09-19) | **not fixed** (the fixes are dated after the snapshot) | outside: the protocol is `file` only |
| 107675 | Medium (6.0; 5.9) | the libssh SFTP protocol (host key) | `2b822b7fb6` (2026-09-12) | **not fixed** | outside: the protocol is `file` only |
| 107676 | Medium (4.8; Low 3.3) | `libavutil` HDR10+ metadata serializer | `2c2f6e96e3` (2026-10-04) | **not fixed** | outside: the serializer's callers in FFmpeg's source are an encoder (`libaomenc.c`) and a muxer (`matroskaenc.c`); VSift writes PNG and WAV and starts no encoder or muxer of these |
| 107677 | Medium (5.7; 4.7) | the DASH demuxer (and `av_strireplace`, whose callers are the DASH files, an `ffprobe` text format and the whisper filter) | `ed27bfcbbb` (2026-10-03) | **not fixed** | outside: the demuxer is not enabled |
| **107678** | **Medium (5.7; 4.7), CWE-674** | **the MP4 demuxer's `pssh` boxes, `av_encryption_init_info_free()` recursion (stack exhaustion)** | **none named; pull request 24593 (not read); no commit on `master` since 2026-08-28 touches `libavutil/encryption_info.c` except an unrelated one of 2026-09-02** | **not fixed (the record says "through 9.0.2")** | **reachable: `mov` is one of the two demuxers VSift enables; a crafted MP4 with very many `pssh` boxes ends the FFmpeg child (crash of the child, a denial of service bounded by VSift's time, memory and output limits; in a worker, inside the strict container)** |
| 107695 | High by CVSS 4.0 (7.1), Medium by 3.1 (6.5) | the HLS demuxer (infinite loop) | `0e6eef3551` (ancestor), `c364ab176f` (cherry-pick, patch identical), `a4ddaba8bb` (the 8.1 line) | fixed | outside: the demuxer is not enabled |
| 107696 | High by CVSS 4.0 (7.1), Medium by 3.1 (6.5) | `ff_rtsp_connect()` in the RTSP code (infinite loop) | none named; pull request 24902 | unknown (not read) | outside: the RTSP demuxer and protocol are not enabled |
| 107697 | Medium (5.3; 4.3) | the HLS demuxer (protocol checks on child playlists) | `01044d0453` (ancestor), `23602df9cd` (cherry-pick, identical), `191715f023` (the 8.1 line) | fixed | outside: the demuxer is not enabled |
| 107698 | Medium (5.3; 5.4) | the RTSP code (server-side request forgery; the record's versions end at 8.0.1 and 7.1.3) | `2326bc5f69`, `ea9e85e549` (ancestors); `7c011995e3`, `f9aa8729bc` (older release lines) | fixed | outside: the RTSP code is not enabled |

**What this shows, and does not.** Nine records are new and eight are outside what VSift's arguments reach or are fixed on the shipped line; that is a
statement about VSift's arguments and not a test: no run started FFmpeg with another demuxer. **CVE-2026-107678 is the exception.** The source of the shipped snapshot shows the chain: `mov_read_pssh` (`libavformat/mov.c`, lines 8064 to 8181 at the snapshot) appends the
init info of every `pssh` box it reads to a linked list (`cur->next = info`), and `av_encryption_init_info_free` (`libavutil/encryption_info.c`, line 216) frees a list by calling itself
on `info->next`, one stack frame per box (read through GitHub's file endpoint at the snapshot's commit). Its reach is
an assessment from the source and the arguments, not a reproduction: nothing ran FFmpeg on a crafted file here (running it would need a build and a
crafted input, which this reading does not do). The record is "Undergoing Analysis" at NVD and its scores are the reporter's. Whether a limit of 5 MB of
probe and of the container's stack makes the recursion reachable inside VSift's bounds was not tried.

**Disposition (the maintainer's decision of 2026-10-10):** accepted for R0 as a denial of service of a bounded, killable child on a record that is not high or
critical, the way L-122 and L-137 were accepted, and **fixed after the stable release**: when the 2026-10-31 month-end FFmpeg build is chosen for the re-pin (L-132), look for an
upstream fix for it (pull request 24593, or a commit on `master` or `release/9.0`) or cherry-pick one, and add a malformed-MP4 `pssh` case to the malicious-media campaign so that
the effect on VSift's child is measured. Register entry L-144 (`accepted residual`, `accepted (2026-10-10)`), issue #351. RQ-13 is recorded `passed` for `0.2.0` on the plan's rule
with this finding named in the entry and in its `does_not_prove`.

## What it does not show, and what is weaker than it sounds

- **A reading of public records and public history, not a test of a binary.** It repeats the 2026-10-04 method: nothing ran FFmpeg. Eight integer-overflow
  fixes in `libswscale/output.c` in five months say the class is not closed, and nine more records in one afternoon say the flow has not stopped.
- **The records are a sample, and these nine are the freshest.** They are "Undergoing Analysis": the scores are the reporter's, a record may be wrong about the
  component or the version, and the database is neither complete nor timely. Records published before 2026-06-01 were not read.
- **The nine were read by component and fix date**, with the first line of a commit message and the callers of two functions from GitHub's code search; the pull
  requests (on the project's own code host, which is not among the sources this reading used) were not read, and CVE-2026-107678's fix state rests on the mirror's history.
- **Gaps carried over:** the worker image's packages are unscanned (the base digest has moved twice since it was chosen); the published Windows and macOS executables'
  system library requirements were not read (only Linux); the Homebrew tools of the macOS journeys and the pinned Windows tools are outside this reading; the native
  tools are not in the SBOM or in `cargo deny`.
- **A status, not a review of every alert:** zero alerts in a store that has been empty throughout shows no alert was raised, not that the code is free of findings.

## Triage record

| Finding | Issue | Severity (proposed) | Disposition |
| --- | --- | --- | --- |
| **CVE-2026-107678**: a `pssh` recursion in the MP4 demuxer's free path, present in the shipped snapshot, reachable through an enabled demuxer | [#351](https://github.com/smormah/vsift/issues/351) | medium (the record's; denial of service of a bounded child) | **accepted for R0 by the maintainer on 2026-10-10 with L-144 as the register entry; fix after the stable** (the re-pin of L-132, and a malformed-MP4 `pssh` case in the malicious-media campaign) |
| The eight other new records (107660, 107675, 107676, 107677, 107695, 107696, 107697, 107698) | none | medium; two High by CVSS 4.0 only (107695, 107696), both outside the configuration | not reachable by VSift's arguments, or fixed on the shipped line; recorded here, no register entry |
| The one FFmpeg record tied to its fix by elimination (CVE-2026-38350) | [#272](https://github.com/smormah/vsift/issues/272) | high (the record's) until accepted | open; accepted by the maintainer on 2026-10-05 with L-122 as the register entry; unchanged on 2026-10-10 |
| whisper.cpp releases 1.9.3 to 1.9.5 carry memory-safety hardening the pinned 1.9.2 lacks; the one reachable fix has a 100 ms floor in front of it since the third candidate | [#322](https://github.com/smormah/vsift/issues/322) | none assigned | open for the re-pin after `0.2.0`; accepted for R0 on 2026-10-07 with L-137 as the register entry; no release newer than v1.9.5 |

## The requests made (nothing personal went out)

Each was anonymous and read-only (`GET`), made with the client's own default identifier, with no key, no account, no cookie and no contact detail; `curl` was run
with `-q`, and `npm` with an empty user and global configuration and a fresh cache. **National Vulnerability Database** (`services.nvd.nist.gov`): six requests, the
keyword search `ffmpeg` and the keyword search `whisper.cpp` over the two windows, `cves/2.0` for CVE-2025-14569 and `cvehistory/2.0` for it. **GitHub** (`gh api`,
GitHub's own authentication): this repository's alert and analysis counts, ten advisory lookups by action and version, public commit, compare, branch, tag, release and
advisory lookups for `FFmpeg/FFmpeg` and `ggml-org/whisper.cpp` (commit hashes and dates only), and two code searches by function name. **The Node.js project**
(`nodejs.org`): the release index, one request. **npm's registry** (`npm view`): two requests, `npm` dist-tags and the `npm@11.19.0` record. **The Docker registry**: one
anonymous token request (`auth.docker.io`) and one manifest request (`registry-1.docker.io`) for `ubuntu:24.04`. The saved responses are in a scratch folder outside the
repository and are not committed.

## Repeating it

`gh workflow run p14-scan-reading.yml --ref main -f version=<version>` (the hosted part), then the read-only calls above. For the FFmpeg row use the tool and the commands of the
addendum of 2026-10-04 (two NVD windows of at most 120 days, merged into one array, one `--snapshot shipped=<commit>`). For whisper.cpp also compare the pin with the project's
release list.
