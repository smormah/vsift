# P06 Ubuntu x64 artifact candidate — not qualified

Date: 2026-09-13. Target for investigation: Ubuntu 24.04 x64 on a disposable
hosted runner. This is a read-only source and archive-inventory review, not an
accepted managed-install catalogue, compatibility result or P06 E2E checkpoint.
No downloaded executable was run on the maintainer desktop.

| Component | Direct publisher origin | Bytes | Independently calculated SHA-256 |
| --- | --- | ---: | --- |
| FFmpeg/FFprobe LGPL build | [BtbN 2026-08-31 month-end release](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-08-31-13-27), `ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz` | 113,372,924 | `204fc02692b11249c3e688ad18538ce2939129a1fc6abc32a6b2638a024496cf` |
| whisper.cpp CPU CLI | [Upstream v1.9.2 release](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.2), `whisper-bin-ubuntu-x64.tar.gz` | 9,497,583 | `46811a3ecf584307480a220b9ef5ff81b7b22dc41577cbc274ce3afc61f753b1` |
| Multilingual `base` model | [Pinned model revision](https://huggingface.co/ggerganov/whisper.cpp/commit/80da2d8bfee42b0e836fc3a9890373e5defc00a6), `ggml-base.bin` | 147,951,465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |

The two archives were downloaded from their release URLs to a temporary review
directory. Their calculated digests matched the publishers' release-asset digests.
The model row repeats the previously verified pinned LFS value; this review did
not download it again. BtbN says the month-end release is retained for two years,
so catalogue acceptance requires an explicit expiry and reviewed replacement.
It must not silently switch to a floating `latest` asset.

The [BtbN release tag](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-08-31-13-27)
resolves to build-repository commit
[`8267213e26c1031621e6e1210fe3aa4867214f6a`](https://github.com/BtbN/FFmpeg-Builds/tree/8267213e26c1031621e6e1210fe3aa4867214f6a).
That snapshot contains the publisher's
[packaging script](https://github.com/BtbN/FFmpeg-Builds/blob/8267213e26c1031621e6e1210fe3aa4867214f6a/build.sh),
[Linux x64 LGPL variant](https://github.com/BtbN/FFmpeg-Builds/blob/8267213e26c1031621e6e1210fe3aa4867214f6a/variants/linux64-lgpl.sh),
and [dependency build scripts](https://github.com/BtbN/FFmpeg-Builds/tree/8267213e26c1031621e6e1210fe3aa4867214f6a/scripts.d).
The binary version suffix resolves to upstream FFmpeg commit
[`e47273f4d9227152dcbf543cebaf9e2430ddbcc4`](https://github.com/FFmpeg/FFmpeg/commit/e47273f4d9227152dcbf543cebaf9e2430ddbcc4).
The [whisper.cpp v1.9.2 tag](https://github.com/ggml-org/whisper.cpp/tree/306c88f4d1286aec1bf96e544632897886af5501)
resolves to `306c88f4d1286aec1bf96e544632897886af5501`.
These links identify available source and build inputs; they do not prove that
every dependency at build time is reproducible from those snapshots or supply
a cryptographic publisher signature for either binary. A future catalogue must
retain the independently calculated fixed archive hashes above as its trust
anchor and disclose the publisher/source links.

For this month-end candidate, stop issuing new install plans by **2028-08-01**
unless a replacement archive is reviewed and pinned. This proposed cutoff is
one month before the publisher's two-year retention window ends; it is a
maintenance policy, not a promise that GitHub will serve the asset until then.

The FFmpeg archive contains 73 entries, 66 regular files, no links and
370,667,773 expanded bytes. These selected regular files were hashed from the
verified archive without execution:

On 2026-09-14 the same publisher URL was downloaded to a temporary review
file on the maintainer machine, measured at 113,372,924 bytes and verified
against the pinned SHA-256 above. No executable was run. Opt-in Rust test
`p06_ffmpeg_archive` (`2a960ba`) passed the production read-only XZ/tar
inventory reader with exactly 73 entries and 370,667,773 declared expanded
bytes. The local inspection took about 82 seconds. This proves format
compatibility for fixed archive bytes only, not production time/resource
suitability, extraction or managed installation.

The later selected-file integrity increment (`5454dcb`) updates this opt-in
test to compare the three selected regular files below against their recorded
size and SHA-256 while inspecting the complete archive. The earlier run above
predates this additional assertion. A fresh opt-in run against the pinned
publisher URL passed on 2026-09-14 after the archive size and SHA-256 matched;
the selected-file inspection took 145.67 seconds without executing binaries.

| Relative file under its version root | Bytes | SHA-256 |
| --- | ---: | --- |
| `LICENSE.txt` | 7,651 | `da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768` |
| `bin/ffmpeg` | 116,038,416 | `ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc` |
| `bin/ffprobe` | 115,829,520 | `0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7` |

On 2026-09-14 the pinned FFmpeg archive was downloaded again from the same
publisher URL into a temporary file, verified against the SHA-256 above, and
inspected without executing a binary. Its sole `LICENSE.txt` is the GNU Lesser
General Public License **version 3** text, matching the selected file hash
above. This is a concrete archive notice, not a complete inventory of every
compiled library's licence or corresponding source. The [publisher release](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-08-31-13-27)
labels this asset `LGPL, static`; [FFmpeg's own licence guidance](https://ffmpeg.org/legal.html)
explains that build configuration and linked components matter. A managed plan
must link the exact archive notice and the build's source/configuration
references, and describe the build accurately without claiming legal clearance.

The whisper.cpp archive contains 44 entries, 35 regular files, eight symbolic
links and 24,519,182 expanded bytes. It includes an MIT `LICENSE` file. The
following selected regular files were hashed from the verified archive:

On 2026-09-14 the same publisher URL was downloaded to a temporary review
file on the maintainer machine, measured at 9,497,583 bytes and verified
against the pinned SHA-256 above. No executable was run. Opt-in Rust test
`p06_whisper_archive` (`c52df98`) independently verified the file digest,
then passed the production read-only gzip/tar inventory reader with exactly
44 entries, 24,519,182 declared expanded bytes and all eight reviewed link
headers. This proves format compatibility for the fixed archive bytes only;
it does not qualify extraction, runtime behaviour or a managed install.

The later selected-file integrity increment (`5454dcb`) updates this opt-in
test to compare the six selected regular files below against their recorded
size and SHA-256 while inspecting the complete archive. The earlier run above
predates this additional assertion. A fresh opt-in run against the pinned
publisher URL passed on 2026-09-14 after the archive size and SHA-256 matched;
the selected-file inspection took 2.48 seconds without executing binaries.

| Relative file under `whisper-bin-ubuntu-x64/` | Bytes | SHA-256 |
| --- | ---: | --- |
| `LICENSE` | 1,078 | `94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d` |
| `whisper-cli` | 976,312 | `61fa94d25ba9a4695118883011f35e8521c158145ec73bcd8805a7c11760e6d7` |
| `libggml.so.0.18.1` | 54,936 | `1985fa3dc169a16715a0998da0a075b29be8f68ea2501e3c043be53be7f11857` |
| `libggml-base.so.0.18.1` | 910,680 | `bc41368cecccc3db8b4f52ad168b51413ee6c005a772b1d3e4f4b3bb47777553` |
| `libggml-cpu-x64.so` | 878,024 | `b7c084e19dc63a83acf9d6dac8d2cba089026996bf805659e10d650d5a51c216` |
| `libwhisper.so.1.9.2` | 611,280 | `afd9560fa2dd20a7c0f9aa682f9c4f339b2d223f2ad6fa200fc229bc3b1606d6` |

The selected `LICENSE` matches [whisper.cpp v1.9.2's MIT notice](https://github.com/ggml-org/whisper.cpp/blob/v1.9.2/LICENSE).
The [pinned model card](https://huggingface.co/ggerganov/whisper.cpp/blob/80da2d8bfee42b0e836fc3a9890373e5defc00a6/README.md)
declares MIT for the converted Whisper weights and identifies their OpenAI
origin; [OpenAI Whisper's notice](https://github.com/openai/whisper/blob/main/LICENSE)
is MIT. These are publisher/repository disclosures; no model bytes were
redownloaded during this notice inspection.

The symlinks are expected SONAME aliases and parakeet aliases in the upstream
archive. P06's extraction policy must not blindly materialize archive links.
One candidate transformation is to select and verify required regular files,
then create reviewed regular-file copies for required SONAME aliases; this is
not accepted until a hosted compatibility test establishes the exact required
libraries and the security tests reject malicious link metadata. Do not infer
that `whisper-cli --help` proves model-backed operation.

Before catalogue acceptance, verify binary-specific notices and corresponding
source references, pin the complete required file/alias inventory, run bounded
F01 media and model-backed smoke on hosted Ubuntu 24.04, measure resource use,
test malicious archives and failures, then prove the production installer and
P06 E2E stage. Supplied tone audio cannot prove speech accuracy. Manual/BYO
fallback remains necessary on every named target.

## Opt-in hosted experiment

`.github/workflows/p06-ubuntu-candidate-smoke.yml` runs a candidate-only Python
qualification script on a disposable Ubuntu 24.04 x64 runner with no repository
permissions or persisted checkout credentials. The script repeats the independently
recorded archive/model hashes, checks archive budgets and paths, extracts only the
selected regular files, materializes reviewed SONAME aliases as regular copies,
and runs FFprobe, FFmpeg and whisper.cpp against owned F01 tone media. Four local
archive guardrail tests and read-only selected-file extraction passed. The script
is not the production installer and does not prove real-speech accuracy or
resource suitability.

Protected [PR #61](https://github.com/smormah/vsift/pull/61) merged this
candidate runner as `a9ecd1b` after Ubuntu, Windows and macOS Quality,
Governance, Documentation, strict-worker, dependency policy/review and
CodeQL/Rust checks passed. The opt-in [hosted run 34782768288](https://github.com/smormah/vsift/actions/runs/34782768288)
passed on Ubuntu 24.04.5 x64, glibc 2.39, kernel
`6.17.0-1022-azure`, runner image `20260907.300.1`. It verified both archive
hashes and downloaded model hash, selected-file hashes, non-link alias layout,
FFprobe F01 inspection, FFmpeg 16 kHz mono audio extraction and whisper.cpp
model-backed inference. Four archive guardrail tests passed in the same job.
The FFmpeg and FFprobe executables reported
`n9.0.1-11-ge47273f4d9-20260831`.

This is candidate compatibility evidence only. F01 audio is a tone, so the run
cannot measure speech accuracy. It did not measure peak resources, run the Rust
installer, qualify arbitrary Ubuntu desktop configurations, resolve binary
notices or close D-01..D-10 and the P06 E2E stage. The link-copy layout needs a
full required-library inventory and production extraction tests before it can
become an accepted catalogue entry.

## Follow-up hosted observation

Protected [PR #68](https://github.com/smormah/vsift/pull/68) merged the
bounded diagnostic runner as `78e005d` after Ubuntu and Windows Quality and
all other required checks passed. Its first macOS Quality attempt hit the
existing intermittent P03 `Busy` failure tracked in
[issue #66](https://github.com/smormah/vsift/issues/66); the unchanged-commit
rerun passed. The opt-in [hosted Ubuntu run 34851335044](https://github.com/smormah/vsift/actions/runs/34851335044)
then passed all five archive/diagnostic guardrail tests and verified the two
archive hashes, model hash, selected files, F01 media operations and model-backed
inference on Linux `6.17.0-1022-azure` x64 with glibc 2.39.

The pinned FFmpeg binary reported `n9.0.1-11-ge47273f4d9-20260831`. Its
observed build configuration included `--enable-version3`, `--enable-openssl`,
`--enable-libx264` **absent** (`--disable-libx264`), `--disable-libx265` and
`--disable-libfdk-aac`; the full bounded configuration is in the hosted job
log. `ffmpeg -L` printed a GNU Lesser General Public License statement; the
script captured only one line of that multi-line statement, so the archive's
independently checked LGPL version 3 `LICENSE.txt` remains the precise notice
evidence. This does not enumerate the obligations or source location of every
compiled component. A plan-facing notice and source reference review remains
open.

F01 model-backed inference took **23.05 seconds**. The runner reported
**290,820 KiB** as the largest peak RSS among *all* child processes in the
smoke, including archive/media operations, using Linux `RUSAGE_CHILDREN`.
That figure is not an isolated whisper.cpp memory measurement or a controlled
resource qualification. F01 is tone-only, so it cannot establish speech
accuracy. These observations do not exercise the production Rust installer,
promote this candidate to an accepted catalogue, or close D-01..D-10/P06 E2E.
