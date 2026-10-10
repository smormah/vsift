# Limits: what VSift cannot do, in plain words

This page lists the limits a user is likely to meet, so you can judge what to trust. Each one links to its
entry in the [known-limits register](../planning/known-limits.md), which is the full record and the
authority. This page describes VSift 0.2.1 as it is, not as it will be.

The short version: **VSift is tested on synthetic videos and a synthetic voice only.** Nobody has yet run it on
real meetings, accents or noisy rooms, so every accuracy figure below says what was measured, on what.

## What VSift can read

- **Containers and codecs.** MP4 and MOV files and Matroska and WebM files; video in H.264, H.265, VP8, VP9, AV1
  or MPEG-4; audio in AAC, MP3, Opus, Vorbis, FLAC or uncompressed PCM. Anything else (AVI, FLV, WMV) is
  refused, not guessed at: re-save it first.
- **Size.** Up to 4 hours and 20 GiB, 32 streams and 16 megapixels per picture. A bigger file is refused.
- **Ten minutes to open a video.** Opening a video copies it into VSift's own folder, and the copy is stopped after
  ten minutes, however far it got. A disk in your computer is nearly always fast enough. A big video on
  a network drive, a memory stick, an SD card or a folder that downloads from the cloud when first read may not be:
  copy it to a disk in your computer first and open that copy
  ([what the message looks like](troubleshooting.md#invalid_source-the-video-or-the-transcript-cannot-be-used)).
  Nothing raises the ten minutes, and trying again starts the copy from the beginning.
- **Per command.** A page of results is 20 by default and 100 at most (`--limit`); a burst of frames is 12 by
  default and 100 at most (`--max-frames`); an audio clip is 30 seconds at most and is a WAV at 16 kHz mono;
  a session keeps at most 512 stored items; one result is at most 1 MiB. Bigger asks fail with
  `RESOURCE_LIMIT`: ask for less.
- **A supplied transcript.** SRT or WebVTT in UTF-8, at most 20,000 cues, 4,096 bytes per cue and 8 MiB.

## The words: speech recognition

- **Tested only on a synthetic voice** ([L-022](../planning/known-limits.md#l-022)). The recogniser is
  whisper.cpp with its `base` model. On clean synthetic speech it got about 3% of the words wrong. No human
  voice, regional accent, overlapping speakers or long recording has been measured.
- **Noise makes it much worse** ([L-020](../planning/known-limits.md#l-020)). On the one noisy test clip it got
  more than half the words wrong. A transcript of a noisy recording can be substantially wrong.
- **Names and numbers are the usual casualties** ([L-021](../planning/known-limits.md#l-021)). In the tests,
  "queued", "invoice 4407" and "E-409" were misheard. Search will not bridge the difference: `407` never finds
  `4407`. Check any number you rely on against the audio clip.
- **The confidence figure is the recogniser's own** and is not a probability. VSift marks it as uncalibrated.
- **Speech recognition prints nothing until it is finished** ([L-017](../planning/known-limits.md#l-017)), and a long
  recording takes many minutes. Press Ctrl-C to stop; running it again continues.
- **A stretch can be left without a transcript (0.2.1 and later; 0.2.0 fails the whole run instead,
  [L-145](../planning/known-limits.md#l-145)).** When the recogniser's answer for a 30-second stretch cannot be
  placed in that stretch of audio, VSift leaves the stretch out, keeps the rest and says so: the result is
  `partial` and lists the stretch as not transcribed by that run, and it stays listed through every later run, however
  far from it, until a run reads it. A recording with long pauses can have a few. When you were re-transcribing a recording
  that already had a transcript, the words it had that reach into such a stretch are kept whole (unless the new words
  overlap them); a word said there that no transcript has cannot be found. Transcribing just that stretch again may cover it (the audio is
  then cut at other points), and a transcript file you supply can. If most stretches fail, the command fails
  instead ([troubleshooting](troubleshooting.md#missing_capability-a-tool-or-model-is-missing)). **A worker `job run`
  or `job batch` request does not report this:** its retranscribe step is `complete` either way, and the revision the
  step names holds the warning (`transcript get --revision`).

## The pictures: screen changes and frames

- **Tested only on drawn video** ([L-028](../planning/known-limits.md#l-028)). The size of a change VSift counts was
  set on synthetic screens. Real recordings with heavy compression noise may show false changes or miss subtle
  ones.
- **It looks twice a second** ([L-029](../planning/known-limits.md#l-029)). A change shorter than half a second
  can fall between looks and is not reported, and the coverage lines cannot say so. A burst of frames around a
  moment finds it if you suspect one.
- **No real motion has been measured** ([L-030](../planning/known-limits.md#l-030)). The practice recordings
  draw no scrolling, cursor movement or loading animation, so how well VSift sees those is unknown.
- **VSift does not read text in a picture.** It gives you the frame; you (or an assistant that can open images)
  read it. A crop at native size helps with small text.
- **Candidates are places to look, not findings.**

## Where and how it was tested

- **Windows 11, macOS 15 on Apple silicon, Ubuntu 24.04 on x64.** VSift is built for these. The published 0.1.0
  was installed and run on hosted test machines of each, which are not clean machines, and nobody has run it on a Mac
  ([L-035](../planning/known-limits.md#l-035), the [support matrix](../planning/support-and-resource-profiles.md)).
  Another Linux may work; nobody has tried. Windows on Arm, Intel Macs and Windows 10 are refused with a clear message.
- **Managed tool installation is Ubuntu 24.04 only** ([L-037](../planning/known-limits.md#l-037)). Everywhere else you
  install FFmpeg, FFprobe and whisper.cpp yourself. On macOS the journeys were tried with Homebrew's builds, which
  VSift does not review ([L-114](../planning/known-limits.md#l-114)).
- **The programs are not signed** ([L-098](../planning/known-limits.md#l-098)). Windows SmartScreen or Smart App
  Control and macOS Gatekeeper may warn about or stop a downloaded file; nobody has yet seen them react to a
  VSift file. [Install guide, section 4](../operations/install.md#4-windows-and-macos-smartscreen-gatekeeper-and-what-to-check-instead).
- **Going back to an older VSift does not work reliably:** it may refuse a session a newer one wrote, so finish or
  close your sessions first ([L-044](../planning/known-limits.md#l-044)).

## Safety and your data

- **FFmpeg, FFprobe and whisper.cpp are not sandboxed on a desktop** ([L-004](../planning/known-limits.md#l-004)).
  VSift starts them without a shell and limits their time and output (and, for FFmpeg, each memory allocation), but
  a flaw in one of them, triggered by a crafted video, would run with your rights. Open videos you would be willing to open in any player.
- **Nothing is uploaded by VSift.** An assistant that reads its output may send what it reads to its own service;
  that is up to the assistant and how you set it up.
- **Text in a video can try to give orders to an assistant** ([L-007](../planning/known-limits.md#l-007)). VSift
  labels it untrusted and an assistant using the skill refuses to act on it, but no tool can make every model immune.
- **One local copy.** Sessions and bundles live on your disk. If the disk or the machine is lost, so is the evidence
  ([L-057](../planning/known-limits.md#l-057)); a bundle copied elsewhere survives. Cleanup deletes files the ordinary
  way, not securely.
- **Durability across a power cut** is shown only on Ubuntu 24.04 with a local ext4 disk
  ([L-008](../planning/known-limits.md#l-008)), and only as far as the disk honours its flushes
  ([L-056](../planning/known-limits.md#l-056)). Elsewhere a crash of VSift itself is survived; a power cut during a
  write may not be.

## AI assistants

- **Two assistants have been tried**, Claude Code and Codex, each with two models, on synthetic videos, with the skill.
  Other assistants and models are untested; two models were tried and fell short of the bar
  ([L-082](../planning/known-limits.md#l-082), [L-084](../planning/known-limits.md#l-084)).
  Codex's own Windows sandbox cannot run VSift ([L-076](../planning/known-limits.md#l-076)).
  [The skill guide](../agents/skill.md#models-and-clients-trialled) has the figures.
- **A report can still be wrong.** The trial reports were right most of the time, not always; the cited evidence is
  what lets you check. One known slip: stating the content of a blurred region as seen in the pixels when only the
  narration said it ([L-095](../planning/known-limits.md#l-095)).
- **The words in a transcript keep hidden characters in `text`.** Quote `display_text`, which shows them as
  `<U+XXXX>` ([L-083](../planning/known-limits.md#l-083)).

## Behaviour that may change

- **The readable text VSift prints without `--json` may change in any release.** Programs should read `--json`,
  whose shape is versioned and only ever grows ([CLI contract](../contracts/cli-v1.md)).
- **The library behind the command line is not a published interface yet**
  ([L-043](../planning/known-limits.md#l-043)).
