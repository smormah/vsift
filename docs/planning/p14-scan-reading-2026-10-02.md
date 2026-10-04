# P14 scan reading, 2026-10-02 (RQ-13, R-SEC03)

Status: a dated record of what was read on 2026-10-02 at the pull request #259 head, for the
published 0.1.0. It is **not** the reading for the candidate: the plan asks for one within seven
days of each candidate and again before the stable. It proposes dispositions; the maintainer
decides them, and nothing was dismissed, closed, patched or changed in any setting by this reading.

**Result.** RQ-13 is recorded as **failed**: one finding, [#272](https://github.com/smormah/vsift/issues/272),
stands unresolved against the rule "no open high or critical finding affecting a supported path". Cargo, the
GitHub alerts, the pinned actions, whisper.cpp and the Node.js and npm pins gave no finding.

**Correction of 2026-10-04.** The FFmpeg counts below (17 fixes "not in the snapshot", 18 records with no
fix reference) came from a test that could not see fixes taken into FFmpeg's release branch as
cherry-picks. The [addendum at the end](#addendum-2026-10-04-the-ffmpeg-finding-re-read-p14-pr-7b-272)
re-reads them with a test that can: read it together with the FFmpeg row and "The FFmpeg finding".
The text of 2026-10-02 is left as it was written.

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
the run and this record. For the FFmpeg row, use the tool and the method of the addendum below.

## Addendum 2026-10-04: the FFmpeg finding re-read (P14 PR 7b, #272)

Status: a dated record of what was read on 2026-10-04 at `main` `812eb5f`, after the maintainer's
decision to refresh the reviewed FFmpeg. It corrects the FFmpeg row and "The FFmpeg finding" above;
the rest of the 2026-10-02 reading stands. RQ-13 stays **failed**: the reading for the candidate is
still to be done. No alert, issue or setting was changed and nothing was published.

**Result.** The 2026-10-02 test asked whether each record's fix commit is an *ancestor* of the
snapshot and counted a fix that was not as missing. FFmpeg takes a fix into a release branch as a
*cherry-pick*: a new commit, with another hash, whose message ends
`(cherry picked from commit <master hash>)`. The first reading said its test was exact in one
direction only; this one shows that the other direction matters.

| Of the 35 records left open on 2026-10-02 | Shipped snapshot `n9.0.1-11-ge47273f4d9` (in the catalogue today) | Refresh candidate `n9.0.2-22-g46d8f462ee` |
| --- | --- | --- |
| 17 with a fix on `master` ("fixed on master only") | **17 fixed**, all by a cherry-pick on `release/9.0` | 17 fixed |
| 18 with no fix reference | **17 fixed** (one, CVE-2026-38350, by elimination: see below), 1 not reachable | 17 fixed, 1 not reachable |

So the shipped snapshot is not missing the 17 fixes. Every one of them reached `release/9.0` between
2026-07-21 and 2026-08-12, before the snapshot was built on 2026-08-31, and **all 28 cherry-picks
that stand in for a master fix (these 17, six of the 18 and the five of the older twelve that have
one) carry the same patch text as the master commit** (compared hunk by hunk, line numbers aside).
The one critical record (75143, the RIST protocol reader) is fixed in the shipped snapshot and was, as
before, a protocol VSift never starts. Nothing about the 47 records is
better in the candidate than in the snapshot already shipped.

**Nothing is known to be open; one tie is by elimination.** The eight `libswscale` and `libavfilter` records
CVE-2026-38343 to -38350 carry almost no reference (one names an issue). They come from one reporter's batch of
ten issues on the project's code host (21583 to 21592, all filed against the commit `dd2976b9e1` of 2026-01-22).
Five were tied to their issues by the function or file each names (see the table); CVE-2026-38347 is the
alpha-blend one, below. The last two, CVE-2026-38349 and CVE-2026-38350, both High (NVD 7.5), a signed-overflow
denial of service in the output code that VSift's frame conversion runs, were traced further:

- **CVE-2026-38349** is issue 21592: its text names `hScale16To19_c` and the `yuv2rgba64_1` template, as the
  record does. It was closed on 2026-03-06 by `1e63151355` (an ancestor of both snapshots). At the statement
  the issue reports (`output.c` line 1292 at the reporter's commit), `abuf0[i * 2 + 1] * (1 << 11)` reads
  `* (SUINT)(1 << 11)` in the shipped snapshot, the candidate and `master`.
- **CVE-2026-38350** is tied to issue 21586 **by elimination, not by name**: no issue names its function
  (`target_sws_fuzzer`), and 21586 (`output.c` line 1480) is the one other `output.c` issue of the batch.
  `86ddc8b438` (2026-02-26, an ancestor) rewrote exactly that statement in the 64-bit full-chroma template
  (`int Y, U, V` and `(G + Y) >> 14` became `unsigned` and `(int)(G + Y) >> 14`); the code is the same in the
  shipped snapshot, the candidate and `master`. **If the record describes some other site, nothing in this
  reading covers it.** That is the residual; it stays in [L-122](known-limits.md#l-122) for the maintainer.
- **Upstream.** The only release branch newer than 8.1 is `release/9.0`; there is no `release/9.1`
  (`n9.1-dev` is `master`). Both sites read the same on `master`, `release/9.0` and the shipped snapshot,
  so there is no unfixed upstream state to wait for.

CVE-2026-38347 (`ff_sws_alphablendaway`) is a third record of the batch and is **not reachable**: the function is
installed only when `alphablend` is not `none`; the option defaults to `none`, VSift never sets it and a media
file cannot (source read at both snapshots: `libswscale/utils.c` lines 1620 to 1625 and
`libswscale/options.c`).

**Exposure (an argument, not a clearance).** `libswscale` is fed only frames that VSift's own FFmpeg run
decoded from a stream the probe listed with a supported codec (H.264, HEVC, VP8, VP9, AV1 or MPEG-4 video:
an unsupported stream is refused before any decode, `ffmpeg_media.rs`), of at most 16,000,000 pixels, under
`-max_alloc` 64 MiB, through a graph VSift writes (`select`, `scale`, `showinfo`, a gray conversion or the PNG
encoder's own choice), with no scaler flag, no `alphablend` and no option from the media. Both sites are
conversion arithmetic on extreme sample values. The alpha site needs a source with an alpha plane, which
to this reading none of the six decoders produces. The full-chroma site is entered for an RGB output when the
source chroma is not subsampled or the output width is odd, so crafted 4:4:4 content of more than eight bits
can reach it. A signed overflow is undefined behaviour in the C source; the likely outcome is a wrong pixel
or a crash of the FFmpeg child, which VSift runs under time, memory and output bounds and, in a worker, in
the strict-isolation container. This says what reaches the code, not that a bug there is impossible.

**How it was read** (the tool is `tools/p14-campaigns/ffmpeg-ancestry.cjs`; its tests are in the
`Fuzz` workflow's Node.js step):

1. **Records.** The National Vulnerability Database, anonymous requests only, keyword `ffmpeg`,
   published 2026-06-01 to 2026-10-04: 58 records, of which 47 name the FFmpeg project; the other 11
   name software that runs or bundles it (a browser, a remote-desktop client, a media server, web
   applications, a TLS library). The libraries' names as keywords (`libavcodec`, `libavformat`,
   `libswscale`, `libavfilter`, `libswresample`, `libavutil`) found nothing more. The 47 are the same as
   on 2026-10-02: none was published since 2026-09-23.
2. **Fix hashes.** Every commit URL in a record's references, in any of the three forms the records use
   (the project's code host, its GitHub mirror, its gitweb), full or abbreviated, resolved to a full hash
   by GitHub's commit endpoint. One hash (CVE-2026-96611) is a pull request's head commit that the
   mirror does not hold; the pull request's merge commit, read on the code host, is an ancestor.
3. **Ancestry.** GitHub's compare endpoint, `<fix>...<snapshot>`: `ahead` or `identical` means the fix is
   in the snapshot.
4. **Cherry-picks.** The commits reachable from a snapshot but not from `master` (the compare
   `master...<snapshot>`: 210 for the shipped snapshot, 305 for the candidate; 204 and 297 carry the
   trailer; the others are release bookkeeping, such as "Update for 9.0.2") matched to the fix by the
   `(cherry picked from commit ...)` line. Then **the patches are compared**: the changed files and the
   hunks of the fix and of its cherry-pick, line numbers dropped (the tool does this; it is not a hash such
   as `git patch-id`, which also ignores whitespace, and a hunk whose context moved would show as different).
   All 28 are identical. The branch also holds two reverts (of a `bwdif` fix and of a scheduler change), neither
   of a fix any of the 47 depends on; the tool reports a cherry-pick that a later commit reverts.
5. **Records with no commit** (11 of the 18; seven of the 18 do name one, see below). The pull request or
   issue the record names, read on the project's code host (anonymous: state, closing date, merge commit),
   and the history of the file the record names on `master` and on both snapshots; for CVE-2026-38347
   alone, the source of the two files above at both snapshots.
6. **What is kept here.** Identifiers, dates, severities, the component each record names and commit
   hashes. The records' descriptions are not reproduced. No identity or contact detail was sent: the
   requests carry only the client's default identifier (GitHub's through `gh api`).

### How this method can be wrong

It is a reading of public records and public history, not a test of a binary: no FFmpeg was run and no
reproducer exists for these records.

- **A mismatch would look like** a cherry-pick whose patch differs (`patch=differs`: a partial or adapted
  backport, to be read by hand), a cherry-pick reverted later (`reverted`), or a fix with no trailer and
  no ancestry (`absent`). None of the 47 does.
- **False positives (a fix counted that is not there).** A trailer is a statement; the patch comparison is
  what makes it evidence, and it covers only what the fix changed, not whether the fix is complete. A record
  can name the commit that introduced a bug instead of the one that fixed it (CVE-2026-70628 to -70632 each
  list one from 2005 to 2020); such a hash is an ancestor for the wrong reason, which is why those five are
  read on their other two hashes. A fix could be undone by a later change that is not labelled a revert.
  A snapshot's version string names its source commit; a vendor build may carry other patches (BtbN's carries
  the ones in its build repository, not read here).
- **False negatives (a fix missed).** A hand-written equivalent without a trailer reads `absent`; a hash the
  mirror does not hold is reported and followed by hand (one case); a record with no commit reference
  ("unreferenced": 11 of the 18) depends on a person finding the issue and the commit, as above, and a wrong
  match there is possible (CVE-2026-38350 is the case where it is by elimination).
- **The records themselves** are a sample of public knowledge: the database is neither complete nor timely,
  a record may be wrong about the component or the version, and nothing unpublished is in it.

**Seven of the 18 did name a fix.** CVE-2026-64830 to CVE-2026-64835 and CVE-2026-18393 carry a commit
in NVD's references (their last modification dates, 2026-07-28 and 2026-08-28, are before the first
reading), which the first pass listed as having none. Six are cherry-picks and one an ancestor.

### The 17

| Record (CVE-2026-) | Severity | Names | Fix evidence | Shipped | Candidate | Reach in VSift (re-read) |
| --- | --- | --- | --- | --- | --- | --- |
| 65703 | High | TDSC video decoder | master `fd3ee52fab`, cherry-pick `3b85fbe890` (2026-07-21) | fixed | fixed | probe may open it |
| 65704 | High | TY demuxer | master `de771bd527`, cherry-pick `52f7983f15` (2026-07-21) | fixed | fixed | outside: demuxer not enabled |
| 65705 | High | vf_floodfill filter | master `f186c50cf5`, cherry-pick `30a52276f9` (2026-07-21) | fixed | fixed | outside: filter never built |
| 65706 | High | vf_swaprect filter | master `a7e38b617b`, cherry-pick `b3c7ebc1ed` (2026-07-21) | fixed | fixed | outside: filter never built |
| 66036 | High | vf_hqdn3d filter | master `5d7112c60e`, cherry-pick `62294b6a8a` (2026-08-02) | fixed | fixed | outside: filter never built |
| 66037 | High | IAMF parsing (also in the MP4 demuxer) | master `86708357d1`, cherry-pick `f15e730cd2` (2026-07-21) | fixed | fixed | reachable |
| 66038 | High | LCL (ZLIB) video decoder | master `e7cbfd1c50`, cherry-pick `7c0b8c8594` (2026-07-21) | fixed | fixed | probe may open it |
| 66039 | High | MACE6 audio decoder | master `aafb5c655e`, cherry-pick `947c57d9e6` (2026-07-21) | fixed | fixed | probe may open it |
| 66040 | High | PNG and APNG encoders | master `b506fafec9`, cherry-pick `5185caaeb8` (2026-07-21) | fixed | fixed | reachable (frames leave as PNG) |
| 66041 | High | vf_quirc filter | master `4da9812e25`, cherry-pick `c20d78c683` (2026-07-21) | fixed | fixed | outside: filter never built |
| 75141 | High | hvcC box writer (muxer side) | master `acf5d7cdc1`, cherry-pick `c7132ef8f6` (2026-08-12) | fixed | fixed | outside: muxer |
| 75142 | High | MPEG-PS muxer | master `9d786e4b5e`, cherry-pick `b274f0d21b` (2026-08-12) | fixed | fixed | outside: muxer |
| 75143 | Critical | RIST protocol reader | master `1c10bcc2e1`, cherry-pick `8880a174d0` (2026-08-12) | fixed | fixed | outside: protocol |
| 75144 | High | VC-2 RTP packetizer | master `1cdeb3c4e7`, cherry-pick `1afd5c3dda` (2026-08-12) | fixed | fixed | outside: muxer |
| 75145 | Medium | AV1 RTP packetizer | master `b4c199c590`, cherry-pick `7646bb4c42` (2026-08-12) | fixed | fixed | outside: muxer |
| 75146 | High | DASH demuxer | master `65b0dab903`, cherry-pick `999f8ba75c` (2026-08-12) | fixed | fixed | outside: demuxer not enabled |
| 75147 | Medium | AV1 RTP packetizer | master `983dae9c19`, cherry-pick `f175bd5082` (2026-08-12) | fixed | fixed | outside: muxer |

### The 18

| Record (CVE-2026-) | Severity | Names | Fix evidence | Shipped | Candidate | Reach in VSift (re-read) |
| --- | --- | --- | --- | --- | --- | --- |
| 12706 | Medium | RASC video decoder (`decode_move`) | no reference; `2f60af465a` (2026-05-03) names the same function; ancestor | fixed | fixed | probe may open it |
| 18393 | Medium | TDSC video decoder (cursor) | `242ff799c7` (2026-05-02); ancestor | fixed | fixed | probe may open it |
| 30754 | High | RTP H.264/HEVC packetizer | no reference; pull request 20746 merged as `d03483bd26` (2025-10-30); ancestor | fixed | fixed | outside: muxer |
| 38343 | Medium | libavfilter `vf_scale` | no reference; issue 21587 closed by `9adced3278` (2026-03-06); ancestor | fixed | fixed | reachable |
| 38344 | High | libswscale `slice.c` | no reference; issue 21583 closed by `dc9bf66796` (2026-03-06); ancestor | fixed | fixed | reachable |
| 38345 | Medium | libswscale `utils.c` (division) | no reference; issue 21585 closed by `04fe98482a` (2026-03-03); ancestor | fixed | fixed | reachable |
| 38346 | High | libswscale `output.c` (`yuv2planeX_8_c`) | no reference; issue 21584 closed by `a59180022a` (2026-03-06); ancestor | fixed | fixed | reachable |
| 38347 | High | libswscale `alphablend.c` | no reference, no fix; the code is never run (source read at both snapshots) | n/a | n/a | not reachable |
| 38348 | High | libswscale `utils.c` (overflow) | no reference; issue 21588 closed by `946ce12e1c` (2026-03-06); ancestor | fixed | fixed | reachable |
| 38349 | High | libswscale (`hScale16To19_c`, in `swscale.c`) | no reference; issue 21592 (names the function) closed by `1e63151355` (2026-03-06); ancestor; the statement it reports is changed in both snapshots and `master` | fixed | fixed | reachable (alpha source) |
| 38350 | High | libswscale `output.c` (fuzzer target) | no reference, no issue names the function; the other `output.c` issue of the batch, 21586 (line 1480), is the statement `86ddc8b438` (2026-02-26; ancestor) rewrote; **tied by elimination** | fixed (by elimination) | fixed (by elimination) | reachable (4:4:4 content of more than 8 bits) |
| 58049 | High | RASC video decoder (`decode_dlta`) | no reference; `f8d7795dcc` on `release/9.0` (master `11ff18a6c8`, 2026-06-28) names the same check; ancestor | fixed | fixed | probe may open it |
| 64830 | High | VobSub demuxer | master `dbd495f066`, cherry-pick `dcf8ce2802` (2026-07-21) | fixed | fixed | outside: demuxer not enabled |
| 64831 | High | Vulkan HEVC hardware decoder | master `92737390dc`, cherry-pick `3d129a4a85` (2026-07-12) | fixed | fixed | outside: hardware decoder |
| 64832 | High | NVDEC hardware decoder | master `4c6217477f`, cherry-pick `3d5ad47c40` (2026-07-21) | fixed | fixed | outside: hardware decoder |
| 64833 | High | S/PDIF muxer | master `6f80e27654`, cherry-pick `385ac2fadc` (2026-07-21) | fixed | fixed | outside: muxer |
| 64834 | High | RTP/ASF demuxer | master `11d5f475be`, cherry-pick `3c441711a3` (2026-07-21) | fixed | fixed | outside: demuxer not enabled |
| 64835 | High | ADX audio decoder | master `1836ef9684`, cherry-pick `c10e7f5dc1` (2026-07-21) | fixed | fixed | probe may open it |

The last column is a re-read of each record's component against VSift's configuration (the `mov` and
`matroska` demuxers and the `file` protocol only; only the filters VSift builds; decoders after a
bounded probe, which may open a decoder outside the allow-list). It is an assumption, as the first
reading's was: the first reading's groups (which put nine records under "no component named") are
superseded by this column only where it names a component. The twelve records fixed by an older
release line on 2026-10-02 stay as they were, and each of their fixes is an ancestor of both snapshots
or a cherry-pick (CVE-2026-70628 to -70632 carry both).

### What it does not show

- **Intent, not execution.** A cherry-pick line and equal patch text show that the fix is in the
  source the snapshot was built from. No reproducer was run against the built binary; none was
  available, and a downloaded binary may run only in the existing hosted harnesses.
- **The records are a sample.** The database is neither complete nor timely, and its fix references
  are sometimes missing (11 of the 18 had none). Records published before 2026-06-01 were not read.
- **One tie is by elimination** (CVE-2026-38350, above): High, reachable, denial-of-service class. It is the
  residual of this reading, and nothing here accepts it on the maintainer's behalf.

### The refresh candidate, and why the catalogue does not change in this pull request

The maintainer decided on 2026-10-04 to refresh the reviewed FFmpeg (disposition 1 of #272). With the
corrected reading the refresh no longer closes any of the 47 records: they are closed already. Its value
is the rest of upstream's work since 2026-08-31, and its cost is below.

- **The build.** The newest release-branch snapshot of the variant the catalogue uses (BtbN
  `linux64-lgpl`, static, release branch 9.0): `ffmpeg-n9.0.2-22-g46d8f462ee-linux64-lgpl-9.0.tar.xz` of
  the release `autobuild-2026-10-03-18-14` (upstream `46d8f462eeb87ee1f704d8c44a0ee24fca471ad1`, FFmpeg 9.0.2
  of 2026-09-17 plus 22 commits). Not a `master` snapshot (`N-127142`: unreleased development code, a
  different line from the one reviewed) and not the 8.1 line (`n8.1.3-14`, older). Its Windows twin, from the same release, is the one the
  repository's Windows jobs pin today (the same 2026-08-31 snapshot), so a refresh moves both.
- **What it adds.** 95 upstream commits (the `n9.0.2` point release and the commits after it). Counted
  by file name, 17 touch code VSift runs for a supported stream: five in the MP4 demuxer and the shared
  demuxing code (four of them the `mov` key atom), one in encryption-info parsing, seven in the VP8, VP9,
  H.264, HEVC, Opus and AAC decoders and four in `libswscale`. The other 78 touch muxers, protocols,
  other demuxers and codecs, filters VSift never builds, hardware code and other CPUs' assembly. It is
  hardening, not the closing of a recorded finding.
- **What it costs.** The archive grows by 22 percent (137,945,828 bytes; 450,447,717 expanded) and
  **breaks two reviewed bounds**: the tar stream cap (400,000,000) and the XZ compressed-size cap
  (128 MiB, `MAX_XZ_ARCHIVE_BYTES`) both refuse it, so a refresh is also a reviewed raise of those two
  numbers. The build recipe changed too: BtbN's build scripts moved by 65 commits and the recorded
  configuration gained three components (`--enable-librsvg`, `--enable-lcms2`, `--enable-vapoursynth`),
  which is new third-party code in a binary whose compiled-component inventory is already not claimed.
- **Retention decides the pin.** BtbN's published policy (its README, read 2026-10-04): the last build
  of each month is kept for two years; the last 14 daily builds are kept. The shipped pin is a
  month-end build (kept to about 2028-08-31; the catalogue stops issuing plans on 2028-08-01). The
  candidate is a daily build: it stays downloadable until 14 newer daily builds exist, about
  **2026-10-17**. When a pinned file disappears, `setup install` of every VSift whose catalogue names it
  stops at the download with `DOWNLOAD_FAILED`; nothing is activated, what is installed keeps working
  and the offline route (`--artifact-dir`) works only for someone who kept the file; the repair is a
  new catalogue revision in a new release. The review of 2026-09-13 already rejected a daily build for
  this reason, and a catalogue clock cannot honestly say "two weeks" in a release.
- **Evidence that it works.** The candidate was pinned on a branch that is **not** part of this
  pull request (`p14-pr7b-ffmpeg-candidate-evidence`, commit `f4695c3`: the pins, the revision, the two
  bounds and the version strings) and run on hosted runners: `P13 managed smoke` (run 37164083942:
  the real `setup plan`, `setup install`, `setup check` and the kill-and-rerun stage on Ubuntu 24.04 all
  passed, FFmpeg and FFprobe reporting `n9.0.2-22-g46d8f462ee-20261003`), `P06 Ubuntu candidate smoke`
  (37164086257: archive layout in 120.8 s, F01 media operations and model-backed inference in 1.70 s,
  peak child RSS 293,188 KiB), `P06 Windows candidate smoke` (37164088495) and `P07 local ASR`
  (37164090634: the adapter, the CLI checkpoint and the T-04 gates, Ubuntu and Windows; see the
  qualification record for its numbers). The details are in the two P06 candidate records.
- **Recommendation.** Pin a month-end build only. The next is the last build of October (2026-10-31);
  re-pin to it on the timing under "The next pin" below, with the procedure in the candidate records. A mirror (a copy kept by the project) would allow a refresh between month-ends but
  redistributes an LGPL-3 build, which ADR 0014 rules out ("VSift does not host, mirror or proxy provider
  binaries"); a release-tagged upstream
  build (BtbN also builds each upstream tag, for example `ffmpeg-n9.0.2-linux64-lgpl-9.0.tar.xz` in the
  2026-09-19 daily) identifies the source better but is kept like any other daily. Neither is the better
  pin; the month-end is.

### The next pin, and what existing installs do

The intended next pin is **the last build of October 2026 (2026-10-31, kept two years)** once it exists,
re-reviewed as the candidate record's procedure says and re-run through the same hosted smokes (`P13 managed
smoke`, `P06 Ubuntu` and `P06 Windows candidate smoke`, `P07 local ASR`) and the pull request's CI.

- **Before the candidate cut or before the stable?** Recommended: **before the candidate cut, and only if the
  cut can wait for it**: a catalogue change is a change under `crates`, which makes every evidence item scoped
  to the crates stale for the candidate (a cut before it, then a re-pin, means the journeys, the campaigns and
  the agent rounds would run again on a second candidate). If the candidate is cut before 2026-11-01, it
  ships the shipped pin (a month-end build, kept to about 2028-08-31, whose records are read as fixed here), and
  **the re-pin waits until after the stable**, as a release of its own; it is not slipped in between the
  candidate and the stable.
- **What blocks it:** the build existing and being on a line this reading covers (a new upstream release
  line is a new review); the maintainer accepting two raised archive bounds and the compiled-component
  inventory for the libraries the recipe then carries; the hosted smokes and the pull request's CI passing
  on it; and the reading for it (the tool, within seven days of the candidate and again before the stable).
- **Existing installs are not moved by a re-pin.** `setup plan` offers an install only for a tool that is
  missing; a managed FFmpeg that verifies stays selected (`ExistingProbeOnly`), so a user who installed the
  old build keeps it until they run `setup remove ffmpeg_ffprobe` and install again. A re-pin therefore
  protects new installs only. That is acceptable while the old build's records are read as fixed (above) and
  is a limit to say plainly if a build must be replaced for a security reason ([L-132](known-limits.md#l-132)).
- **When a pinned asset disappears,** existing installs keep working: the files are local, are re-verified
  by hash on use, and `setup list`, `check`, `rollback`, `remove` and `repair` need no network. A fresh
  `setup install` from any release that names the vanished asset stops at the download with
  `DOWNLOAD_FAILED`, activating nothing; the offline route works only for someone who kept the file; the
  repair is a new catalogue revision in a new release. The shipped pin's file is kept until about 2028-08-31.

### Dispositions, updated (the maintainer decides)

1. Treat the 17 and the 17 of the 18 as fixed in the shipped snapshot, on the evidence above, and narrow
   [L-122](known-limits.md#l-122) to the one tie by elimination (CVE-2026-38350) and what a source reading
   cannot show.
2. For CVE-2026-38350: accept the residual with L-122 as the register entry, or ask upstream which
   report the record describes (reachable, High, denial-of-service class, bounded as above).
3. Re-pin to the 2026-10-31 build on the timing above; repeat this reading with the tool for the candidate
   and again before the stable.
4. The hostile-media and load campaigns (P14 PR 4) test **published** versions from the registry; their
   re-run on a refreshed build happens on the release candidate (P14 PR 11), not here.

### Repeating it

```console
curl -sS "https://services.nvd.nist.gov/rest/json/cves/2.0?keywordSearch=ffmpeg&pubStartDate=2026-06-01T00:00:00.000&pubEndDate=2026-09-28T00:00:00.000" > nvd-1.json
node tools/p14-campaigns/ffmpeg-ancestry.cjs --records nvd-1.json --snapshot shipped=<upstream commit of the catalogue's build> --snapshot candidate=<commit of the candidate> --out result.json
```

The database allows a 120-day window per request and a few requests per half minute without a key; send no
key, no contact detail and no custom identifier. The tool needs `gh` signed in (read-only calls) and
lists every record without a commit as `unreferenced`: those, and every `absent`, are read by hand as in
the table above.
