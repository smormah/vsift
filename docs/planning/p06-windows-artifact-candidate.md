# P06 Windows x64 artifact candidate — not yet qualified

Date: 2026-09-13. Target: Windows 11 x64 desktop. This is a read-only source and
archive-inventory review, **not** an accepted installer catalogue or permission to
activate downloads. P06 and its source gate remain open.

Availability risk: the initially selected 2026-09-09 BtbN asset is a *daily* build, not a
month-end retained build. Its [pinned source retention policy](https://github.com/BtbN/FFmpeg-Builds/blob/847e5e1cacc2945ac46528d34d754bd36051680c/README.md#release-retention-policy)
keeps only the last 14 daily builds and month-end builds for two years. The
candidate URL may disappear; a temporary successful download is not a durable
managed-install source. Before accepting a catalogue, choose and review a
retained/maintained source with an explicit expiry and replacement policy.

### Month-end replacement under review

The [2026-08-31 BtbN release](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-08-31-13-27)
is a month-end build covered by the publisher's two-year retention policy
(subject to that publisher continuing the policy). Its tag resolves to commit
`8267213e26c1031621e6e1210fe3aa4867214f6a`. The exact Windows x64 LGPL
archive `ffmpeg-n9.0.1-11-ge47273f4d9-win64-lgpl-9.0.zip` is 147,007,942
bytes, with independently recalculated SHA-256
`2484854ad6988d34560f4e6ea7a6ecb9dde0af7c229d2591815d056b04ec4f56`,
matching the GitHub release asset digest. Its 48 ZIP entries total 358,125,718
expanded bytes. The three selected files under the single version-named root
were hashed from the verified archive without executing them:

| Relative file | Bytes | SHA-256 |
| --- | ---: | --- |
| `LICENSE.txt` | 7,651 | `da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768` |
| `bin/ffmpeg.exe` | 114,400,768 | `63a0b3c76a245bc0d986853612d9ec43a2a2d1f1c7a3fa40ee459c248075b3a6` |
| `bin/ffprobe.exe` | 114,198,528 | `1ce64d9fdbfce857de2dd1f157c37eaa61c7501a356273dbcbe8b1674aef5879` |

The proposed opt-in runner revision pins this month-end candidate rather than the daily
asset. It still needs its own hosted compatibility run, binary notice/source
review, expiry/update policy and production installer controls. Retention is
not a signature, a licence clearance or a promise of permanent availability.

## Initial daily candidate origins and archived integrity

| Component | Immutable source selection | Bytes | Archived SHA-256 | Licence observation |
| --- | --- | ---: | --- | --- |
| FFmpeg and FFprobe | [BtbN 2026-09-09 build](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-09-09-14-51), tag commit `847e5e1cacc2945ac46528d34d754bd36051680c`, archive `ffmpeg-n9.0.1-27-g9b0578816c-win64-lgpl-9.0.zip` | 170,475,591 | `1f20ec59455ddec021598fae3f8bd37a7bbd6b8647faaa10cf77688e70970563` | The archive carries an LGPL-3.0 text; its static binary and third-party notices still need distribution-path review. |
| whisper.cpp CPU CLI | [Upstream v1.9.2 release](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.2), tag commit `306c88f4d1286aec1bf96e544632897886af5501`, archive `whisper-bin-x64.zip` | 8,194,445 | `49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a` | [Pinned source licence](https://github.com/ggml-org/whisper.cpp/blob/v1.9.2/LICENSE) is MIT; the binary archive does not include the licence text. |
| Multilingual `base` model | [ggerganov model commit](https://huggingface.co/ggerganov/whisper.cpp/commit/80da2d8bfee42b0e836fc3a9890373e5defc00a6), file `ggml-base.bin` at that exact revision | 147,951,465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` | Model repository declares MIT. P07 still decides whether this model meets accuracy/resource gates. |

The two GitHub archive hashes were independently recalculated on downloaded bytes
in a dedicated temporary review directory and matched the GitHub release asset API.
All 16 selected extracted-file digests below were recomputed from the verified
archives and checked against this record without executing the binaries.
The model SHA-256 and byte size are the pinned Git LFS pointer at the linked model
commit; the model has **not** been downloaded or hashed locally. None of these
hashes may be taken from a future network response or user-supplied plan when
installing: the accepted values must live in reviewed VSift source. HTTPS and a
release-page checksum alone are not a publisher signature.

## Extracted-file inventory observed from verified archives

The FFmpeg archive has 48 entries, including `ffplay.exe` and documentation that
VSift does not need. A minimal candidate extraction selects only the following
three regular files under its single version-named root:

| Relative file | Bytes | SHA-256 |
| --- | ---: | --- |
| `LICENSE.txt` | 7,651 | `da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768` |
| `bin/ffmpeg.exe` | 132,656,640 | `c09c9818d91357e6c8ecbba6d832377ca309690dda35baac6019e716070bd934` |
| `bin/ffprobe.exe` | 132,456,960 | `02739830b1ffeb4a8357de1b069cbee81df1c9654ede15a35d53069f7a372291` |

The whisper archive has 37 entries, including unrelated server, streaming and
test executables. A minimal CPU candidate must allow exactly `Release/whisper-cli.exe`,
`Release/whisper.dll`, `Release/ggml.dll`, `Release/ggml-base.dll` and the nine
`Release/ggml-cpu-*.dll` files below. Whether all nine CPU backends are necessary
must be confirmed by a controlled compatibility test, not assumed from `--help`.

| Relative file under `Release/` | Bytes | SHA-256 |
| --- | ---: | --- |
| `whisper-cli.exe` | 479,232 | `95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d` |
| `whisper.dll` | 1,368,064 | `792fc523c7ad16e6b9c348e30ad5e5f591165cbcf6a80ca8d0db02a38ce3eea2` |
| `ggml.dll` | 67,584 | `894c6237ee7849843213906a2b6a0b371aaa6234048d465f206d910ae846fafb` |
| `ggml-base.dll` | 666,624 | `1482359d921b4c1b183d49db1d770f9b5e90d86a618b8b648d4845c2471ad6b0` |
| `ggml-cpu-alderlake.dll` | 809,984 | `d1c5411561361f7ce71ff8455ecf01f666f581b0608fa91a1dfe7d3fd6a25bd1` |
| `ggml-cpu-cannonlake.dll` | 853,504 | `2ef36f05fa252ff4fdcb8d42ebce1ceba4f3d3de12b93bed15bdee6237dccd63` |
| `ggml-cpu-cascadelake.dll` | 850,944 | `505899aaf3f99c5d714361640f561458ea97f8a09eb0614568a66bead2115cb0` |
| `ggml-cpu-haswell.dll` | 811,008 | `f8cf2f35a06498d783d77fde42004dd54d2f8236b0d42ac323b94bba65a603c4` |
| `ggml-cpu-icelake.dll` | 850,944 | `78ad143ee2e674d037b4840ef33b5748a0659762a26e0ae2b621c4f9451cbde8` |
| `ggml-cpu-sandybridge.dll` | 803,328 | `ee47db7dc40fb30eca73e62a05306059c2c3c42aecddf2e8d6ad7e530069b815` |
| `ggml-cpu-skylakex.dll` | 852,992 | `164e2793897944a43ee071ce6c0b09018088bdf4dd8b14ac0755c58849cf8c50` |
| `ggml-cpu-sse42.dll` | 790,016 | `7318a9a3b95a85b2453c437b274412bbbae89e5ecdf5babb19b99edc06ded063` |
| `ggml-cpu-x64.dll` | 794,112 | `af0f1c2f28ff9e3f472481dd969907bda85fa39d4fde17617d4bb0b389301b60` |

## Remaining gates before catalogue acceptance

1. Reconcile the static FFmpeg LGPL-3.0 build and its third-party/source notices
   against a direct per-user download, including whether the chosen build can be
   responsibly offered by an MIT/Apache-2.0 tool. Do not conflate BtbN build-script
   licence with the downloaded binary licence.
2. Run the exact selected binaries and pinned model on a disposable Windows x64
   qualification host with no elevated rights. The F01 tone-only compatibility
   smoke below passed; real-speech transcription, resource measurement,
   containment and Windows 11 desktop qualification remain open. No reviewed
   binary was executed on the maintainer desktop during this review.
3. Pin redirect host policy, maximum compressed/extracted bytes and entry count;
   reject traversal, duplicate names, links, devices and non-whitelisted files.
   Independently test corrupt archive, wrong architecture and missing file cases.
4. Document where the MIT and LGPL notices/source offers accompany the installed
   versions. Resolve the daily-build retention risk and define revocation and
   replacement of this pinned candidate. The other
   named R0 targets still require manual/BYO guidance unless separately qualified.

Until these gates and installer D-01..D-10 pass, `setup plan`/`setup install`
remain unavailable and this candidate is **not** an accepted trust anchor.

## Opt-in candidate compatibility experiment

`P06 Windows candidate smoke` is a manual `workflow_dispatch` job. Its
candidate-only script at `tools/p06_windows_candidate_smoke.py` repeats the
reviewed hashes, limits download and ZIP processing, and runs the three pinned
components against project-owned F01 media on a disposable Windows runner.
The workflow requests no repository permissions and does not persist checkout
credentials. It receives no repository secrets. Its output must be reviewed
before any catalogue decision; merely adding the job is not a passed test.

F01 currently has tone audio, not spoken words. This experiment can establish
binary loading, media extraction and model-backed inference, but **cannot**
establish transcript accuracy or fulfil P06's full E2E stage. P07's speech
fixture, the legal/notice review, production installer safeguards and desktop
target qualification remain separate gates. The qualification script is not
invoked by `vsift setup` and must never be promoted into the installer.

Protected [PR #55](https://github.com/smormah/vsift/pull/55) merged the initial
daily-asset opt-in
runner as `1c805c11519ec43ad91cc6beb18d8d39c40ee495`, after Windows,
macOS and Ubuntu Quality, Governance, Documentation, strict-worker, dependency
policy/review and CodeQL/Rust checks passed. The manually dispatched
[candidate run 34737109736](https://github.com/smormah/vsift/actions/runs/34737109736)
passed on Windows Server 2025 (build 26100), x64, runner image
`windows-2025-vs2026` `20260907.229.1`. It verified both archive SHA-256
values and independently downloaded and hashed the pinned model bytes. The
selected extracted-file hashes matched the candidate inventory. The selected
FFmpeg and FFprobe reported `n9.0.1-27-g9b0578816c-20260909`; FFprobe read
F01 and FFmpeg extracted 16 kHz mono WAV. The whisper.cpp v1.9.2 candidate
loaded the CPU backend and pinned model, processed that WAV and wrote a text
artifact. Six negative archive guardrail tests passed in the same run.

The run did **not** establish meaningful speech transcription: F01 audio is a
tone sentinel. It also did not measure peak resources, inspect all CPU variants,
test Windows 11, grant an installer trust anchor, validate notice handling or
complete the D/E2E gates. The runner's unprivileged checkout is not a sandbox
claim for arbitrary third-party executables.
