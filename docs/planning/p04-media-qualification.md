# P04 source and media qualification record

Status: complete in protected PR #44, merge
`4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`. Predecessor: protected main
`67bfe1438ad4e3a586aff5b6ae0a61710c340861` (P02/P03 complete).
Issue: [#7](https://github.com/smormah/vsift/issues/7).
Decision: [ADR 0012](../decisions/0012-p04-source-media-profile.md).

## Implemented boundary

P04 adds internal source/media primitives only. `SourceSnapshot::stage` requires an
initialized private P03 session, uses a held no-follow source handle and a held
session artifact directory, and never writes or removes the original. It refuses
relative paths, special files, Windows UNC/device paths, reserved names and ADS,
and unsupported container magic. The staged bytes are SHA-256 identified, with a
20 GiB logical limit and 600-second read limit. Storage admission and lifetime
holds include the copy. A separate original-source rehash detects byte changes;
the staged copy is rehashed before provider calls. A same-user mutation race after
that verification remains outside the desktop guarantee.

`FfmpegMedia` uses P02's trusted executable and process supervisor. Its fixed
profile `p04-r0-v1` accepts `mov` or `matroska` and input protocol `file` only;
MOV external data references and absolute aliases are disabled. There is no shell,
raw option API, inherited environment or writable provider output path. Provider
stdout and stderr have separate limits. The adapter rejects unknown/zero/over-four-
hour duration, over-32 streams, missing/zero/over-16-megapixel video dimensions,
unusable time bases, unsupported rotation and malformed JSON. An unreviewed codec
stays visible in metadata as `Unsupported` and extraction returns a typed error.
Audio-only and silent video remain useful for their supported stage.

The exact-frame policy seeks at most five seconds before the normalized request,
decodes and selects the first displayed frame at or after it, then reads the
selected frame's actual PTS from `showinfo`. Requests outside the source or without
a frame inside the caller's tolerance fail. Displayed PNG dimensions must match
orientation-correct metadata. Audio extraction is a maximum ten-second mono 16 kHz
PCM chunk; the first decoded `ashowinfo` PTS is reported independently from the
request. A remuxed AAC-only fixture exposes a real 64 ms priming gap, which is
reported rather than forced to zero. Each operation has a 15- or 30-second provider
deadline and root-wide weighted admission. Probe stdout is capped at 4 MiB,
diagnostics at 64 KiB, frame PNG at 64 MiB, PCM at 4 MiB and additionally 320 KiB
for ten seconds. FFmpeg's `max_alloc` is 64 MiB per allocation, not a whole-process
memory guarantee.

## Project-owned fixture evidence

`fixtures/corpus/manifest.json` remains independent frozen truth. Run:

```console
python tools/generate_p04_fixtures.py --ffmpeg /absolute/path/to/ffmpeg
python tools/verify_p04_fixtures.py --ffmpeg /absolute/path/to/ffmpeg --ffprobe /absolute/path/to/ffprobe
```

The standard-library generator uses original project-owned 5×7 bitmap glyphs and
fixed 20 Hz visual states. It emits F01-F10/F12 videos, genuine VFR F09 Matroska
with a 2 s video start and 750 ms audio offset, and derived rotation, audio-only
and multiple-language-track variants. F11 includes empty, truncated, damaged-tail,
external-file and network-playlist bytes plus excessive-stream/dimension parser
documents. Source media totals about 2 MiB. Tone audio is a P04 extraction
sentinel; P07 will add speech artifacts matching the frozen scripts.

[`generated/provenance.json`](../../fixtures/corpus/generated/provenance.json) records
the manifest and FFmpeg hashes/version, exact normalized generator argv, output
hashes and byte sizes. A second run on the same pinned FFmpeg build produced an
identical provenance file and media hashes. The separate
[`generated/verification.json`](../../fixtures/corpus/generated/verification.json)
records independent FFprobe timestamps/dimensions, F09 VFR and audio alignment,
rotation/track layout, hash checks, and decoded-pixel checks for F02's repeated
A/B/A states, F03 G18, F06's 500 ms tooltip and F09's 3.25 s marker. Fixture truth
was not changed to fit extraction.

## Opt-in cumulative checkpoint

```console
cargo test -p vsift-infrastructure --locked --test p04_media_e2e -- --ignored --nocapture
```

The runner does not download or install dependencies. Missing FFmpeg/FFprobe is
reported `blocked`. It records source/media `passed` or `failed` for F01, F09, F10,
rotated portrait, audio-only, selected Spanish audio and truncated media. Its JSON
report is written under ignored `.vsift/e2e-runs/<run-id>/report.json` with the
revision/dirty state, manifest/source/artifact hashes, OS/architecture, operator-
reported filesystem when supplied, effective resource profile, provider versions,
actual timestamps, bounded diagnostic, authorization and gaps. To record a
filesystem observed independently, set `VSIFT_E2E_FILESYSTEM` (for example `NTFS`)
for this one command; the report labels the value as operator-reported.

P05-P14 stages are `not_implemented`, and `complete_journey` stays
`not_implemented` even when the P04 checkpoint passes. This satisfies the P04
attachment in the [E2E spine](e2e-test-spine.md) and leaves issue
[#40](https://github.com/smormah/vsift/issues/40) open.

## Named test and threat mapping

| Gate | Evidence |
| --- | --- |
| M-01 | No-follow staging with quoted/space/option-like names; explicit multi-audio index and language; P02 exact-argv tests |
| M-02 | Empty/playlist/truncated rejection; bounded JSON, stream, duration and dimension tests; F11 parser variants |
| M-03 / SEC-05 | 20 GiB/600 s source, 4 h/32 streams/16 MP probe, 15/30 s watchdog, 64 MiB PNG and ten-second PCM limits; full decoder memory isolation remains P11/P14 |
| M-04 / SEC-06 | Magic and forced-demuxer policy rejects playlists; MOV external references disabled; protocol `file` only; desktop filesystem isolation remains unclaimed |
| M-05 | F01 B-frames, F09 VFR/nonzero start/750 ms audio offset, rotated portrait and displayed dimensions; independent fixture verification |
| M-06 | No-audio F10, audio-only M4A, selected language tracks and typed unsupported-codec metadata |
| V-01 / SEC-17 | First-at-or-after frame, observed PTS/delta, VFR boundaries/final frame/out-of-range, source SHA-256 and displayed dimensions; no inferred frame number or fabricated source claim |

The native FFmpeg 9.0 Windows/NTFS checkpoint is development evidence, not a
cross-platform release support claim. Ordinary protected Quality jobs exercise
source/parser contracts on Windows, macOS and Ubuntu; P14 still owns the full
supported-profile media, malicious-decoder, fault and soak matrix. P05 will link
source snapshots into a real session lifecycle. P06 will qualify managed provider
builds. P07 adds speech, and P08/P09 add candidate search/reinspection.
