# P14 scan reading, 2026-10-05 (RQ-13, R-SEC03), for the release candidate 0.2.0-rc.1

Status: a dated record of what was read on 2026-10-05, the day the candidate was published, at `main`
`d5792ce31db1` (the tag `v0.2.0-rc.1`), for the published `0.2.0-rc.1`. It is the reading the plan asks for within
seven days of a candidate (plan section 6); the stable release needs another. It follows the method of
[`p14-scan-reading-2026-10-02.md`](p14-scan-reading-2026-10-02.md) and its addendum of 2026-10-04, which this record
does not repeat. Nothing was dismissed, closed, patched or changed in any setting, and nothing was installed on a
developer machine.

**Result.** RQ-13 is recorded as **failed** for the candidate, for the one reason it was failed for 0.1.0:
[#272](https://github.com/smormah/vsift/issues/272) (a High record, CVE-2026-38350, tied to its fix by elimination
only, [L-122](known-limits.md#l-122)). **Nothing else was found, and nothing changed since 2026-10-04:** no FFmpeg or
whisper.cpp record was published or modified after 2026-09-27, the catalogue's tools are the same builds, and every other
source is clean. The residual is the maintainer's to accept (L-122's next step); until then the rule "no open high or
critical finding affecting a supported path, unless accepted by the maintainer with a register entry" is not shown as met.

## How it was read

- **Cargo, the executable and the SBOM** by a hosted job: `P14 scan reading`, run
  [37330697823](https://github.com/smormah/vsift/actions/runs/37330697823) (dispatched from `main` with
  `version=0.2.0-rc.1`, passed): `cargo deny` 0.20.2 for the workspace and the fuzz crate, the published `vsift-cli`
  0.2.0-rc.1 from the real registry, `readelf` on its Linux executable, the release's SBOM against `cargo metadata`.
- **GitHub** by read-only `gh api` calls (counts and states; nothing opened, dismissed or reopened) and the published
  advisories that name each pinned action at its pinned version.
- **FFmpeg and whisper.cpp** from the National Vulnerability Database's public interface (anonymous reads with the
  `curl` client's own identifier, no key, no contact detail), GitHub's commit and compare endpoints through `gh api`
  (commit hashes only) and GitHub's repository advisories. Only identifiers, dates, severities, components and commit hashes
  are kept; no record's description is reproduced.
- **Runtime pins** from the Node.js release index, npm's registry (`npm view`) and the Docker registry's tag manifest for
  `ubuntu:24.04` (an anonymous token request and one manifest request).

## Source by source

| Source | What was read | Result | Disposition |
| --- | --- | --- | --- |
| Cargo | `cargo deny check` for the workspace and `cargo deny --all-features --manifest-path fuzz/Cargo.toml check`; the advisory summary as JSON | `advisories ok, bans ok, licenses ok, sources ok` for both; 0 advisory errors, warnings or notes; the policy's own warnings only (six duplicate-version notes for the workspace: `base64`, `getrandom`, `io-lifetimes`, `r-efi`, `syn`, `windows-sys`, and one licence exception not encountered; four duplicate-version notes for the fuzz crate) | none needed |
| GitHub alerts | open and all-state code-scanning, Dependabot and secret-scanning alerts; the latest CodeQL analyses of `main` | 0 in each, in any state; the CodeQL analysis of `main` at `d5792ce` (2026-10-05) has 0 results over 28 rules | none needed. This reads the alert store, which has been empty throughout; job success is not a review |
| Pinned actions | the published advisories that name `actions/checkout` 7.0.1, `upload-artifact` 7.0.1, `download-artifact` 8.0.1, `setup-node` 7.0.0, `setup-python` 7.0.0 (bumped by #195 since the last reading), `oven-sh/setup-bun` 2.2.0, `attest-build-provenance` 4.2.2, `cargo-deny-action` 2.1.1, `dependency-review-action` 5.0.0 and `github/codeql-action` (pinned by commit `977e6cea`, tag `codeql-bundle-v2.27.0`, 2026-09-08; not in the last reading) | none for the first nine. One advisory names the `codeql-action` line: GHSA-vqf5-2xx6-9wfm (high, a token written to debug artifacts) for 2.26.11 to below 3.0.0 and 3.26.11 to 3.28.2, patched in 3.28.3 (2025-01); the pin is a 2026 commit | not affected. Every workflow pins by commit |
| whisper.cpp v1.9.2 and the `base` model | the National Vulnerability Database for records naming whisper.cpp, published 2026-06-01 to 2026-10-05; the project's repository advisories | the same three records as on 2026-10-04 (CVE-2026-10298, -17512, -17513; all `Deferred`, none modified since 2026-10-03, none published after 2026-07-27) plus the older CVE-2025-14569 of the first reading (published 2025-12-12, last modified 2026-06-17, `Deferred`); none names 1.9.2; 0 repository advisories | no finding against 1.9.2 as recorded; the records are "not yet analysed", so this is the absence of a statement, not a clearance |
| **FFmpeg** (BtbN snapshot `n9.0.1-11-ge47273f4d9`, 2026-08-31, the catalogue's build, unchanged since 0.1.0: the plan of the candidate's offline-install run names it) | all 58 records the keyword `ffmpeg` finds, published 2026-06-18 to 2026-09-27 (none after 2026-09-27, none modified after 2026-10-02); the tool `tools/p14-campaigns/ffmpeg-ancestry.cjs` run over all of them against the shipped snapshot | the same 58 as on 2026-10-04. 35 carry a commit reference of the FFmpeg project; for all of them every resolved fix is in the snapshot: 16 as an ancestor and 28 as a release-branch cherry-pick whose patch text equals the master commit's; **no fix is `absent`, `reverted` or `differs`**; one hash (CVE-2026-96611, a pull request's head commit) is not on the mirror, as before, and its merge commit was read on 2026-10-04. The other 23 have no commit reference (the records that name other software, and the ones the 2026-10-04 reading followed by hand through the issue or file they name); none of the 58 was modified since, so those readings stand | one residual stands, unchanged: CVE-2026-38350, **tied by elimination** ([#272](https://github.com/smormah/vsift/issues/272), [L-122](known-limits.md#l-122)) |
| Runtime and build | the published Linux executable; Node.js and npm in the publish job; the worker image's base | `GLIBC_2.34` at most; needs `libssl.so.3`, `libcrypto.so.3` (`OPENSSL_3.0.0`), `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`, as in 0.1.0. Node.js 24.21.0 (2026-09-07, ships npm 11.19.0) is the newest 24 release and 22.23.3 the newest 22; the newest security releases are 24.18.1 and 22.23.2 (2026-07-28), both older; no advisory names npm 11.19.0 (npm's registry has 11.21.0 under `next-11` and 12.2.0 as `latest`, neither bundled with a Node.js release). `ubuntu:24.04` now points at `sha256:534baea6…`; the worker image's digest is still `sha256:a61567bd…` | the pins are current for Node.js; npm 11.19.0 is behind the registry's newest 11.x with no advisory against it. The base-image digest is older than the tag and the image's packages were **not scanned** (no scanner ran): the gap of 2026-10-02 stands |
| Inventory | the release's CycloneDX SBOM for the Linux executable against `cargo metadata --filter-platform x86_64-unknown-linux-gnu` at the tag | 164 library components; 165 crates resolved; the one difference is the root package `vsift-cli 0.2.0-rc.1`, which an SBOM names as its application; none unresolved in the other direction | none needed. The SBOM lists the Rust graph only: the native tools and the OpenSSL runtime are not in it |

## What it does not show, and what is weaker than it sounds

- **A repeat, not a new method.** The FFmpeg row is the same reading of public records and public history as 2026-10-04, run
  again the day after: its answer is the same because the inputs are the same (no new record, no modified record, the same build).
  It does not test a binary. Eight integer-overflow fixes in `libswscale/output.c` in five months say the class is not closed.
- **The records are a sample.** The database is neither complete nor timely and the keyword `ffmpeg` finds a record only if its
  text says so; records published before 2026-06-01 were not read.
- **Gaps carried over:** the worker image's packages are unscanned (the base digest has moved twice since it was chosen); the
  published Windows and macOS executables' system library requirements were not read (only Linux); the Homebrew tools of the
  macOS journeys and the pinned Windows tools are outside this reading; the native tools are not in the SBOM or in `cargo deny`.
- **A status, not a review of every alert:** zero alerts in a store that has been empty throughout shows no alert was raised,
  not that the code is free of findings.

## Triage record

| Finding | Issue | Severity (proposed) | Disposition |
| --- | --- | --- | --- |
| The one FFmpeg record tied to its fix by elimination (CVE-2026-38350) | [#272](https://github.com/smormah/vsift/issues/272) | high (the record's) until accepted | open; the maintainer chooses between accepting the residual with L-122 as the register entry and asking upstream which report the record describes |

No other finding was raised by this reading.

## Repeating it

`gh workflow run p14-scan-reading.yml --ref main -f version=<version>` (the hosted part), then the read-only calls above, within seven days of
the stable release's publish and before its plan: the ledger's RQ-13 entry for the stable cites the run and its own record.
For the FFmpeg row use the tool and the commands of the addendum of 2026-10-04 (two NVD windows of at most 120 days, merged
into one array, one `--snapshot shipped=<commit>`).
