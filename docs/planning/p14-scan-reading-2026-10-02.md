# P14 scan reading, 2026-10-02 (RQ-13, R-SEC03)

Status: a dated record of what was read on 2026-10-02 at the pull request #259 head, for the
published 0.1.0. It is **not** the reading for the candidate: the plan asks for one within seven
days of each candidate and again before the stable. It proposes dispositions; the maintainer
decides them, and nothing was dismissed, closed, patched or changed in any setting by this reading.

**Result.** RQ-13 is recorded as **failed**: one finding, [#272](https://github.com/smormah/vsift/issues/272),
stands unresolved against the rule "no open high or critical finding affecting a supported path". Cargo, the
GitHub alerts, the pinned actions, whisper.cpp and the Node.js and npm pins gave no finding.

## How it was read, and what is deliberately not kept

- **Cargo and the binary** by a hosted job (workflow `P14 scan reading`, run
  [37136669647](https://github.com/smormah/vsift/actions/runs/37136669647) at the head; the first run of the same job
  is [36998250575](https://github.com/smormah/vsift/actions/runs/36998250575)): `cargo deny` 0.20.2, the published
  `vsift-cli` 0.1.0 installed from the real registry, `objdump` and `readelf` on its Linux executable, the
  release's own SBOM against `cargo metadata`. Nothing was installed on a developer machine.
- **GitHub** by read-only `gh api` calls: counts and states of CodeQL, Dependabot and secret-scanning alerts,
  the latest CodeQL analyses, and the published advisories that name each pinned action. No alert was
  opened, dismissed or reopened.
- **FFmpeg and whisper.cpp** from the National Vulnerability Database's public interface (anonymous reads), the
  public FFmpeg repository's commit ancestry (GitHub's compare endpoint, commit hashes only) and GitHub's
  repository advisories. **Only identifiers, publication dates, severities, the component each record names, and
  the version ranges it gives are kept here.** The records' own descriptions are not reproduced; a reader who needs
  them opens the identifier at the National Vulnerability Database.
- **Runtime pins** from public indexes: the Node.js release index and the Docker registry's tag manifest for
  `ubuntu:24.04` (an anonymous token request and one manifest request; nothing identifying is sent).

## Source by source

| Source | What was read | Result | Disposition |
| --- | --- | --- | --- |
| Cargo | `cargo deny check` for the workspace and `cargo deny --all-features --manifest-path fuzz/Cargo.toml check` for the fuzz crate (advisories, bans, licences, sources) | all four checks `ok` for both; the lockfile at the tag is the one the SBOM below was compared with | none needed |
| GitHub alerts | open code-scanning, Dependabot and secret-scanning alerts; Dependabot alerts in any state | 0 open in each; 0 Dependabot alerts in any state; the latest CodeQL analysis of `main` (commit `06abfc0`) has 0 results over 28 rules | none needed. Job success is not a review; this reads the alert store, which has been empty throughout |
| Pinned actions | the published advisories that name `actions/checkout` 7.0.1, `upload-artifact` 7.0.1, `setup-node` 7.0.0, `download-artifact` 8.0.1, `setup-python` 6.3.0, `dependency-review-action` 5.0.0, `attest-build-provenance` 4.2.2, `oven-sh/setup-bun` 2.2.0 and `EmbarkStudios/cargo-deny-action` 2.1.1 | one advisory names an action: GHSA-cxww-7g56-2vh6 (high) names `download-artifact` versions 4.0.0 up to 4.1.3; the pin is 8.0.1 and every workflow pins by commit | not affected; none for the other eight |
| whisper.cpp v1.9.2 and the `base` model | the four National Vulnerability Database records that name whisper.cpp: CVE-2025-14569 (medium 5.3), CVE-2026-10298, CVE-2026-17512 and CVE-2026-17513 (low 3.3 each); GitHub repository advisories (none published) | the records give affected versions in the 1.8 line (one gives `1.8.4-58`, one a commit), none names 1.9.2; all four are `Deferred` at the database | no finding against 1.9.2 as recorded. The records' state is "not yet analysed", so this is the absence of a statement, not a clearance |
| **FFmpeg** (BtbN snapshot `n9.0.1-11-ge47273f4d9`, 2026-08-31) | 47 records naming FFmpeg, published 2026-06-18 to 2026-09-23; for each the fix commit (where the record gives one) tested for ancestry against the snapshot | 12 fixed by an older release line (in the snapshot); **17 fixed on `master` only, not in the snapshot** (1 critical, 13 high, 3 medium); **18 with no fix commit in the record** (14 high, 4 medium) | **unresolved: [#272](https://github.com/smormah/vsift/issues/272)**, see below |
| Runtime and build | the published Linux executable; Node.js and npm in the publish job; the worker image's base | glibc symbols up to `GLIBC_2.34`; needs `libssl.so.3`, `libcrypto.so.3` (`OPENSSL_3.0.0`), `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`; Node.js 24.21.0 (ships npm 11.19.0) is the newest 24 release and 22.23.3 the newest 22 (2026-09-23), both after the latest security releases (24.18.1, 22.23.2); `ubuntu:24.04` has moved on since the worker image's digest `sha256:a61567bd…` was chosen (the tag points at `sha256:a853f94d…` today) | the pins are current for Node.js and npm. The base-image digest is older than the tag and the image was **not scanned** (no scanner ran), so its packages are unread. See the gaps |
| Inventory | the release's CycloneDX SBOM for the Linux executable against `cargo metadata --filter-platform x86_64-unknown-linux-gnu` at the tag | 164 library components in the SBOM; 165 crates resolved; the one difference is the root package `vsift-cli 0.1.0`, which an SBOM names as its application; none unresolved in the other direction | none needed. The SBOM lists the Rust graph only: the native tools and the OpenSSL runtime are not in it |

## The FFmpeg finding

What the unresolved records are, grouped by whether the component each names is code VSift's
configuration can reach. The configuration: FFmpeg and FFprobe run with the demuxers `mov` and
`matroska` and the protocol `file` only (`MediaProviderConformance::r0`, `-protocol_whitelist file`),
bounded resources, and decoders only after a probe of at most 5 MB and 5 s. A record's component is
the file its description names; "unnamed" means the description names none. Suffix: `H` high,
`C` critical, `M` medium; `f` the record's fix is on `master` and not in the snapshot, `n` the record
gives no fix commit.

| Reading | Count | Records (CVE-2026-…) |
| --- | --- | --- |
| Outside the configuration: a muxer, an RTP, DASH or HLS part, the MPEG program-stream or AVI demuxer, a hardware decoder, a protocol | 12 | 12706 Mn, 64830 Hn, 64832 Hn, 64834 Hn, 65703 Hf, 75142 Hf, 75143 **Cf**, 75144 Hf, 75145 Mf, 75146 Hf, 75147 Hf, 30754 Hn |
| A demuxer or codec VSift enables (HEVC, PNG, the MP4 demuxer's IAMF parsing) | 4 | 64831 Hn, 66037 Mf, 66040 Hf, 75141 Hf |
| The scaler and filters VSift uses for frames and crops (`libswscale`, `libavfilter`) | 8 | 38343 Mn, 38344 Hn, 38345 Mn, 38346 Hn, 38347 Hn, 38348 Hn, 38349 Hn, 38350 Hn |
| A decoder outside the allow-list (reached only if the probe opens one) | 2 | 58049 Hn, 64835 Hn |
| No component named | 9 | 64833 Hn, 65704 Hf, 65705 Hf, 65706 Hf, 66036 Hf, 66038 Mf, 66039 Hf, 66041 Hf, 18393 Mn |

**What this reading does and does not show.** The 12 outside the configuration (including the one
critical record, in a network output) are not reachable by the allow-lists, which is a statement about
VSift's arguments and not a test: no run started FFmpeg with another demuxer. The other 23 include
**18 high-severity records** (counting the unnamed) that cannot be ruled out. A fix on `master` can also
exist on the 9.0 release line under another commit hash; the 11 commits between the `n9.0.1` tag and the
snapshot were read by subject and match no component above, but the 9.0 branch since 2026-08-28 was not
read. For the 18 records with no fix commit, upstream may have fixed them without the record saying so.
The ancestry test is exact only in one direction (an ancestor is in the snapshot).

**Proposed dispositions** (the maintainer decides; each is a change this reading does not make):

1. Refresh the reviewed FFmpeg to a build that has the fixes, as a new catalogue revision with the
   catalogue's own review (P04 conformance, the hostile-media campaign, `setup install` on the three
   systems). This closes the 17 and re-reads the 18.
2. Read the 18 records with no fix reference upstream by component and write each down as fixed, not
   reachable or open.
3. If the snapshot stays for the candidate, record the 12 as an accepted residual in the register with the
   allow-lists as the reason, add a test that fails if FFmpeg is started with another demuxer or protocol,
   and say in the candidate's release notes which FFmpeg it uses and that untrusted media belongs in the
   strict-isolation container.

## Gaps in the reading

- The worker image's packages and the Ubuntu base were not scanned; the digest is older than the tag.
- The native tools are not in the SBOM and not in `cargo deny`; their readings above are by version and
  need repeating when the managed catalogue changes.
- macOS and Windows runtimes (the Homebrew FFmpeg on macOS, the pinned Windows build) were not read:
  they are not part of the supported path claim yet.
- The National Vulnerability Database's records are not a complete or timely vulnerability list; GitHub's
  advisory database was read for the actions only.
- The published Windows and macOS executables' system library requirements were not read (only Linux).

## Triage record

| Finding | Issue | Severity (proposed) | Disposition |
| --- | --- | --- | --- |
| Reviewed FFmpeg snapshot lacks 17 fixes; 18 records without a fix reference | [#272](https://github.com/smormah/vsift/issues/272) | high until the snapshot is refreshed or the exposure shown nil | open; the maintainer chooses between refresh and an accepted residual ([L-122](known-limits.md#l-122)) |

## Repeating it

`gh workflow run "P14 scan reading"` (hosted part), then the read-only `gh api` calls and public reads
above, within seven days of the candidate and again before the stable. The ledger's RQ-13 entry cites
the run and this record.
