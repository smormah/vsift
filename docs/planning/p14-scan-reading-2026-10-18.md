# P14 scan reading, 2026-10-18 (RQ-13, R-SEC03), for the release candidate 0.2.2-rc.1

Status: a dated record of what was read on 2026-10-18, the day `0.2.2-rc.1` was published (publish run 39078284454, ended 00:09:55 UTC), at the tag `v0.2.2-rc.1`'s
commit `43a033dbf813` (on that day also the head of `main` and of `release/0.2.x`), for the candidate that the stable `0.2.2` will be cut from. It repeats the method of
[`p14-scan-reading-2026-10-15.md`](p14-scan-reading-2026-10-15.md) (for `0.2.1-rc.3` and the stable `0.2.1`) over the window since that reading, and does not repeat the method
sections of the earlier readings it lists. It reads only public sources, with the tools' own default clients. Nothing was dismissed, closed, patched or changed in any setting, the FFmpeg
and whisper.cpp catalogue was not touched, nobody upstream was contacted, nothing was posted or commented anywhere, no issue was opened and nothing was installed on a developer machine.

**Result: RQ-13 is recorded `passed` for `0.2.2-rc.1`: the reading names no new finding that touches VSift, and everything else agrees with 2026-10-15.** The item's pass rule (plan
section 6) is no open high or critical finding on a supported path unless fixed, mitigated, or accepted by the maintainer with a register entry. The maintainer's acceptances stand
unchanged: CVE-2026-107678 ([L-144](known-limits.md#l-144), [#351](https://github.com/smormah/vsift/issues/351)), CVE-2026-38350 ([L-122](known-limits.md#l-122),
[#272](https://github.com/smormah/vsift/issues/272)), the pinned whisper.cpp ([L-137](known-limits.md#l-137), [#322](https://github.com/smormah/vsift/issues/322)), and CVE-2026-110810
([#399](https://github.com/smormah/vsift/issues/399), Low, accepted on 2026-10-15 for `0.2.1` under L-137 and fixed with the re-pin of cycle 2; a Low record does not fall under the pass rule,
and whether to restate that acceptance for `0.2.2` is the maintainer's). Nothing read makes any of them worse.

## What changed in the tree since the last reading

Between `fa380021f2aa` (the tree of 2026-10-15) and `43a033dbf813`: `Cargo.lock` gains **`fs4` 1.1.0** (used on Windows only, for the room check of #266; it depends on `rustix` and
`windows-sys` 0.61.2), several crates move from `windows-sys` 0.59.0 to 0.61.2, and **`process-wrap` gains its `creation-flags` feature** (#404's console of its own); `fuzz/Cargo.lock`
follows. The workflows gain one pinned action, **`actions/cache` (restore and save) v6.1.0** at `55cc8345863c`. The catalogue (FFmpeg BtbN `n9.0.1-11-ge47273f4d9`, upstream
`e47273f4d922`; whisper.cpp v1.9.2 and its `base` model), the demuxer allow-list, Node.js 24.21.0 for the publish job and 22.23.3 for the tests, Rust 1.98.1 and the worker image pin
(`ubuntu@sha256:a61567bd...`) are unchanged.

## How it was read

- **The hosted part:** `P14 scan reading`, run [39082029193](https://github.com/smormah/vsift/actions/runs/39082029193), dispatched on the tag with `version=0.2.2-rc.1` on 2026-10-18 at
  00:14 UTC (one job, about two and a half minutes, success), **read through its artifact this time** (`p14-scan-reading`, nine files: the two `cargo deny` reports and the advisory summary, the
  published binary's version line, `binary.txt`, the SBOM and `inventory.json`).
- **By hand on 2026-10-18:** `cargo deny check` and `cargo deny --all-features --manifest-path fuzz/Cargo.toml check` (0.20.2) on the tag's tree, so the advisory database read is the day's
  (the tool's own client fetched it at `999ec89d5d3f`, 2026-10-17 15:23 UTC); RustSec's public history; GitHub's alert stores, CodeQL analyses and advisory database (read-only `gh api`);
  the National Vulnerability Database's public interface (anonymous, `curl -q` with its default identifier, no key); FFmpeg's code host (anonymous reads); the Node.js release index, npm's
  registry (`npm view` with an empty user and global configuration and a fresh cache), the Docker registry (an anonymous token and the manifest of `ubuntu:24.04`) and the Rust releases. The
  FFmpeg ancestry tool (`tools/p14-campaigns/ffmpeg-ancestry.cjs`) ran over the window's records against the shipped snapshot as the addendum of 2026-10-04 says, through `gh api`. The saved
  responses are in a scratch folder outside the repository and are not committed.

## Source by source

| Source | What was read | Result | Disposition |
| --- | --- | --- | --- |
| Cargo | the hosted run's artifact, and `cargo deny` by hand for both lockfiles | `advisories ok, bans ok, licenses ok, sources ok` for both, hosted and by hand; the advisory summary is 0 errors, 0 warnings, 0 notes for both; the workspace's 7 warnings are the six duplicate versions (`base64`, `getrandom`, `io-lifetimes`, `r-efi`, `syn`, `windows-sys`) and the one licence exception not encountered, the fuzz crate's 4 are duplicates | none needed |
| RustSec | 14 commits of the advisory database (`edb58f5974b8` to `999ec89d5d3f`); 9 advisories added (RUSTSEC-2026-0365 to 0373: `negentropy`, `pest_grammars`, `zip` three, `zvariant`, `zbus_names`, `zvariant_derive`, `ammonia`) and one changed (`lopdf`) | none names a crate of either lockfile (284 locked crate names); `fs4`, `rustix`, `windows-sys` and `process-wrap` have no RustSec entry; GitHub's database has one for `rustix` (GHSA-c827-hfw6-qwvm) whose range ends at 0.38.x, and the locked `rustix` is 1.1.5 | none needed |
| GitHub alerts | code-scanning alerts (open, dismissed, fixed), Dependabot alerts (open, fixed, dismissed, auto-dismissed) and secret-scanning alerts (open, resolved); CodeQL analyses | 0 in each state of each store; CodeQL's analysis of `main` at the tag's commit (2026-10-17 18:38 UTC, CodeQL 2.27.2) has 0 results over 28 rules and no error; `release/0.2.x` has no analysis of its own (its head is that commit; CodeQL runs on pushes to `main` and on pull requests to `release/**`) | none needed. Four earlier analyses of other refs ended "unsuccessful execution" and were each followed by a successful one on the same ref; none is the tag's commit |
| Pinned actions | the advisories that name each of the eleven pinned actions at its version, `actions/cache` v6.1.0 included; the advisories of the `actions` ecosystem published or updated from 2026-10-15 to 2026-10-18 | none for any of the eleven, and none for `actions/cache` at any version; none in the window. **A method note:** a lookup of `github/codeql-action` by its bundle number (2.27.0) wrongly matches an advisory for v2.x of the action; the pinned commit is action version 4.37.10, whose lookup returns none, so later readings query the action's own version | not affected |
| whisper.cpp v1.9.2 and the `base` model | the NVD for `whisper.cpp` and `ggml`, published from 2026-10-14 and modified from 2026-10-15; the project's advisories, releases and commits | only CVE-2026-110810 (#399: moved from "Received" to "Deferred" on 2026-10-15, when an exploitability rating was added; the scores, all Low, unchanged; a GitHub copy of it, unreviewed, adds nothing) and CVE-2026-108955 (PowerInfer's own code, now "Deferred"); 0 repository advisories; the newest release is still v1.9.5 and `master` has no commit since 2026-10-12 | no new finding. #399 and the residual of #322 stand |
| **FFmpeg** (the catalogue's build, unchanged) | the NVD keyword `ffmpeg`, published from 2026-10-14 (7 records) and modified from 2026-10-15 (12), and the library keywords (nothing more): 14 records merged; the ancestry tool over them; FFmpeg's code host for pull requests 24593 and 23455; `release/9.0` and `master` since 2026-10-15 | **Three new records, none of them FFmpeg code:** CVE-2026-111728 (Seanime, argument injection, Medium), CVE-2026-102608 (ZoneMinder, shell injection, Critical by CVSS 3.1) and CVE-2026-102834 (JavaCV's `IPCameraFrameGrabber`, an integer overflow, High): VSift uses none of these products and starts every tool without a shell. **Known records that moved:** CVE-2026-96611 is now High (CVSS 3.1 7.8, "before 9.0") and is in `mov_read_ispe`, which VSift's MP4 demuxer reaches; its fix (pull request 23455, merged as `2cc7b87bdb75`) is an ancestor of the snapshot: fixed. CVE-2026-30754 is now "Analyzed", High, "before 8.1", in the RTP muxer, which VSift never runs; its merge `d03483bd26` is an ancestor. CVE-2026-107697 and -107698 are "Analyzed" with the same scores and the same fix verdicts as on 2026-10-10. The four wrapper-package records of 2026-10-14 are "Deferred". CVE-2026-109311 (#400, closed) is "Awaiting Analysis" since 2026-10-16 with the same scores and still names no FFmpeg commit. `release/9.0` has no commit since 2026-10-15 and no new tag; `master` has 98, none to `encryption_info.c` or `libswscale/output.c`, one refactor of `mov.c` | **no new finding.** The residuals of #272 and #351 stand |
| CVE-2026-107678 (L-144, #351) | the record, pull request 24593, `master` and `release/9.0` | unchanged: last modified 2026-10-14 ("Analyzed", CVSS 4.0 5.7, 3.1 4.7, through 9.0.2); the pull request is open, not merged, last updated 2026-10-06; no commit to `encryption_info.c` on either branch; **not fixed in the snapshot** | accepted for R0 (2026-10-10); unchanged |
| Runtime and build | the published Linux executable (the hosted artifact); Node.js, npm, Rust and the base image | `GLIBC_2.34` at most; needs `libssl.so.3`, `libcrypto.so.3` (`OPENSSL_3.0.0`), `libgcc_s.so.1`, `libm.so.6`, `libc.so.6` and the loader, as every earlier version. Node.js: 24.22.0 is still the newest 24 release (the publish job's 24.21.0 is one minor release behind; not a security release), 22.23.3 is the newest 22, the newest security releases (24.18.1, 22.23.2) are older than both pins, no release since 2026-10-15. npm: `latest` 12.2.0; 11.19.0 is not deprecated and has no advisory. **Rust 1.99.1 was released on 2026-10-16** with no security item: it fixes a miscompilation of `bool` return values across C calls on x86-64 (rust-lang/rust#163911) that is a regression of 1.99.0; the pin 1.98.1 is before it, and a move of the pin should skip 1.99.0. **The `ubuntu:24.04` tag now names `sha256:68ee22004a91...`** (it named `534baea6...` on 2026-10-08 and 2026-10-15); the worker image's pin `a61567bd...` is the older digest, as before | the pins are current except Node.js one minor release behind and Rust two releases behind, with no advisory against either. The images' packages were **not scanned**: the gap of 2026-10-02 stands |
| Inventory | the release's CycloneDX SBOM for the Linux executable against `cargo metadata` (the hosted artifact) | 164 library components; 165 crates resolved; the one difference is the root package `vsift-cli 0.2.2-rc.1`; none unresolved in the other direction | none needed. The SBOM lists the Rust graph only |

## What it does not show, and what is weaker than it sounds

- **A reading of public records and public history, not a test of a binary.** Nothing ran FFmpeg or whisper.cpp on a crafted input.
- **The records are a sample, and the freshest are the least analysed.** GitHub's date-window advisory searches return reviewed advisories only; the NVD's change history of CVE-2026-109311 did not yet show its change of 2026-10-16.
- **Four fix hashes of CVE-2026-107697 and -107698 that the tool reports `absent`** (older release lines) were not traced again by hand; each record also has a fix that is in the snapshot, as on 2026-10-10.
- **Gaps carried over:** the worker image's packages are unscanned; the Windows and macOS executables' system libraries were not read; the Homebrew tools of the macOS journeys and the pinned Windows tools are outside this reading; the native tools are not in the SBOM or in `cargo deny`.
- **A status, not a review of every alert:** zero alerts in a store that has been empty throughout shows no alert was raised, not that the code is free of findings.

## Triage record

| Finding | Issue | Severity | Disposition |
| --- | --- | --- | --- |
| No new finding on 2026-10-18 | none | n/a | none needed |
| CVE-2026-110810: whisper.cpp's model loader; the pin v1.9.2 lacks the fix of v1.9.5 | [#399](https://github.com/smormah/vsift/issues/399) | low (the record's) | accepted by the maintainer on 2026-10-15 for `0.2.1` under L-137; fixed with the re-pin (cycle 2); the record is now "Deferred", unchanged in substance |
| CVE-2026-107678: a `pssh` recursion in the MP4 demuxer's free path, unfixed in the shipped snapshot | [#351](https://github.com/smormah/vsift/issues/351) | medium (the record's) | accepted for R0 by the maintainer on 2026-10-10 (L-144); unchanged |
| The one FFmpeg record tied to its fix by elimination (CVE-2026-38350) | [#272](https://github.com/smormah/vsift/issues/272) | high (the record's) | accepted by the maintainer on 2026-10-05 (L-122); unchanged |
| whisper.cpp releases 1.9.3 to 1.9.5 carry hardening the pinned 1.9.2 lacks | [#322](https://github.com/smormah/vsift/issues/322) | none assigned | accepted for R0 on 2026-10-07 (L-137); unchanged |

## The requests made (nothing personal went out)

Each was a read-only `GET` or `HEAD`, made with the client's own default identifier, with no key, no account, no cookie and no contact detail; no service asked for one. **The National
Vulnerability Database** (`services.nvd.nist.gov`): 23 requests (record searches and reads, and six change histories). **FFmpeg's code host** (`code.ffmpeg.org`): 2 (the two pull
requests). **The Node.js project** (`nodejs.org`): the release index, 1. **The Docker registry**: one anonymous token request (`auth.docker.io`) and two manifest requests
(`registry-1.docker.io`) for `ubuntu:24.04`. **npm's registry**: four `npm view` commands. **GitHub** (`gh api`, GitHub's own authentication, as in the earlier readings): about 120 read-only
calls (this repository's alerts and analyses, RustSec, the advisory database, the ancestry tool's commit and compare reads, whisper.cpp, `npm/cli`, `nodejs/node`, `rust-lang/rust`), and the
advisory database that `cargo deny` fetched with its own client. Nothing was written anywhere.

## Repeating it

`gh workflow run p14-scan-reading.yml --repo smormah/vsift --ref v<candidate> -f version=<candidate>` (the hosted part, on the candidate's tag), then the read-only calls above over the window
since this reading. For the FFmpeg row use the tool and the commands of the addendum of 2026-10-04, then the same keyword with `lastModStartDate`, and the open pull requests of the records that
name no fix. Look up a pinned action's advisories by the action's own version, not the CodeQL bundle number.
