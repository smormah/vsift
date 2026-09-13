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

The FFmpeg archive contains 73 entries, 66 regular files, no links and
370,667,773 expanded bytes. These selected regular files were hashed from the
verified archive without execution:

| Relative file under its version root | Bytes | SHA-256 |
| --- | ---: | --- |
| `LICENSE.txt` | 7,651 | `da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768` |
| `bin/ffmpeg` | 116,038,416 | `ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc` |
| `bin/ffprobe` | 115,829,520 | `0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7` |

The whisper.cpp archive contains 44 entries, 35 regular files, eight symbolic
links and 24,519,182 expanded bytes. It includes an MIT `LICENSE` file. The
following selected regular files were hashed from the verified archive:

| Relative file under `whisper-bin-ubuntu-x64/` | Bytes | SHA-256 |
| --- | ---: | --- |
| `LICENSE` | 1,078 | `94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d` |
| `whisper-cli` | 976,312 | `61fa94d25ba9a4695118883011f35e8521c158145ec73bcd8805a7c11760e6d7` |
| `libggml.so.0.18.1` | 54,936 | `1985fa3dc169a16715a0998da0a075b29be8f68ea2501e3c043be53be7f11857` |
| `libggml-base.so.0.18.1` | 910,680 | `bc41368cecccc3db8b4f52ad168b51413ee6c005a772b1d3e4f4b3bb47777553` |
| `libggml-cpu-x64.so` | 878,024 | `b7c084e19dc63a83acf9d6dac8d2cba089026996bf805659e10d650d5a51c216` |
| `libwhisper.so.1.9.2` | 611,280 | `afd9560fa2dd20a7c0f9aa682f9c4f339b2d223f2ad6fa200fc229bc3b1606d6` |

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
archive guardrail tests and read-only selected-file extraction passed. The hosted
run has not yet passed; the script is not the production installer and does not
prove real-speech accuracy or resource suitability.
