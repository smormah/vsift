<div align="center">

<img src="docs/assets/readme/hero.svg" alt="VSift, a video evidence engine: evidence you can cite and check, for you or your AI assistant, on your own machine. Three frames of a recording pass through a sieve and come out as cited speech, screen-change and frame evidence." width="100%">

### Point it at a video. Get back what was said, what changed on screen, and the proof.

VSift turns a recording into evidence that people, scripts and AI assistants can cite and check, on your own machine. Point it at a video and you, or your AI assistant, get **timestamped speech**, **the moments the screen changed** and **the exact frames behind them**; VSift itself uploads nothing.

</div>

## Install

You need Node.js 22 or later.

```console
npm install --global vsift-cli
vsift setup check
vsift ingest ./your-video.mp4
```

`vsift setup check` tells you what is missing for video (FFmpeg) and speech (whisper.cpp and a model), and exactly how to fix it. On Ubuntu 24.04 x64 VSift can install them for you (`vsift setup plan`, then `vsift setup install`) and installs nothing until you accept its plan; elsewhere you install them yourself and register them once.

[**Install the tools yourself**](#install-the-tools-yourself) · [**Use it with your AI assistant**](#use-it-with-your-ai-assistant) · [**Already have a transcript?**](#already-have-a-transcript) · [**Without Node.js**](#without-nodejs)

<div align="center">

[![CI](https://github.com/smormah/vsift/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/smormah/vsift/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/vsift-cli?label=vsift-cli&color=cb3837)](https://www.npmjs.com/package/vsift-cli)
[![Release](https://img.shields.io/github/v/release/smormah/vsift?label=release&color=2a4a73)](https://github.com/smormah/vsift/releases)
[![Last commit](https://img.shields.io/github/last-commit/smormah/vsift/main?color=2dd4bf)](https://github.com/smormah/vsift/commits/main)
[![Licence](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence)
![Status](https://img.shields.io/badge/status-early%20(0.x)-orange)

[**Install**](#install) · [**AI assistant**](#use-it-with-your-ai-assistant) · [**How it works**](#how-it-works) · [**Status**](#honest-status) · [**Help**](#help-and-reference)

<br>

<img src="docs/assets/readme/demo-terminal.svg" alt="An animated terminal replaying real, trimmed output on a 14-second synthetic recording: ingest opens a session, local speech recognition transcribes it, a search for 1017 finds the narration from 0 to 7 seconds, candidates lists screen changes at 0, 7 and 10 seconds, and frame get returns the frame at 10 seconds with its hash. A draft report written for the demo then claims that at 12 seconds order 1017 is retried and shows COMPLETED, citing a frame that does not exist. handoff check refuses it with evidence_not_in_session; the report without that claim is valid." width="100%">

<sub>Real output from a build of this repository's <code>main</code> branch at commit <code>8a35d1f4</code>, using commands the published release has, trimmed for length, on a 14-second synthetic test clip from this repository's corpus. <a href="#see-it-work">The full walk-through is below.</a></sub>

</div>

**A claim with no evidence behind it is refused.** The draft in this demo was written for it: it says order 1017 was retried at 00:12 and cites a frame that does not exist, so `vsift handoff check` refuses it. The check tests a report's structure and citations, not whether a sentence is true: a false sentence that cites real evidence passes. That is why every citation leads to the original frame, for you to open and judge.

---

## Other ways to set up

### Install the tools yourself

VSift needs two things besides itself, and finds them on your `PATH` or wherever you register them:

- **Video:** [FFmpeg](https://ffmpeg.org/download.html), which comes with FFprobe. Any video work needs it.
- **Speech:** [whisper.cpp](https://github.com/ggml-org/whisper.cpp)'s `whisper-cli` and a speech model, for local speech recognition. A transcript you already have needs neither.

Install them with whatever you normally use, then register any that are not on your `PATH`, with absolute paths, and check again:

```console
vsift setup configure ffmpeg --executable /absolute/path/to/ffmpeg
vsift setup configure ffprobe --executable /absolute/path/to/ffprobe
vsift setup configure whisper --executable /absolute/path/to/whisper-cli
vsift setup configure-model --file /absolute/path/to/ggml-base.bin
vsift setup check
```

whisper.cpp publishes no macOS command-line build, so on a Mac you build it from its source or take it from a package manager. The [installation guide](docs/operations/install.md#52-windows-macos-and-other-machines-bring-your-own) names the reviewed model and shows Windows paths.

### Use it with your AI assistant

VSift ships an **agent skill** that takes an AI assistant from a local video to a grounded, cited handoff: it knows the method, the evidence budgets and the safety rules, such as never installing tools on its own, and it reads with the brief view. Copy `skills/vsift` into your assistant's skill folder (Claude Code: `~/.claude/skills/vsift/`; Codex: `~/.agents/skills/vsift/`) and ask about a recording:

```text
Please look at ./team-sync.mp4 and tell me what was decided about the release date,
with the times and frames that show it. Use the VSift skill.
```

The skill has been trialled with Claude Code and Codex; other assistants that can run local commands can call the same CLI, but they have not been tried. In the named-client trials on this repository's synthetic recordings, the smaller models passed 26 of 28 (Claude Sonnet 5.5) and 28 of 28 (GPT-6-Sol; 23 of 28 as run, before the maintainer's reading of one command) full trials, and no run installed anything, leaked a secret or acted on injected text. The later rounds on the 0.2.1 skill gave mixed results for the smaller models; on the 0.2.2 skill the round with the skill met every gate, and without the skill a smaller Claude model often left out the identifiers behind its facts ([#454](https://github.com/smormah/vsift/issues/454)). 0.2.3 ships the same skill, and its trials were not repeated. These are small trials on synthetic video: the [qualification record](docs/planning/p12-agent-qualification.md) and the [changelog](CHANGELOG.md) list the misses and the limits. Setup and details: [using the skill](docs/agents/skill.md).

### Already have a transcript?

Then you only need the video tools. Give VSift your SRT or WebVTT file and it skips local speech recognition altogether:

```console
vsift ingest ./your-video.mp4 --transcript ./your-video.vtt
```

### Without Node.js

Bun 1.2 or later installs it too (`bun add --global vsift-cli`). Prefer no JavaScript runtime at all? Every release has native archives for Windows 11 x64, macOS 15 on Apple silicon and Linux x64 on [GitHub Releases](https://github.com/smormah/vsift/releases), with checksums and Sigstore build-provenance attestations. The [installation guide](docs/operations/install.md) covers every route, upgrading, uninstalling, proxies and how to verify what you downloaded.

## Why

A recording often holds the answer: a meeting you missed, a product demo, a tutorial, a talk you downloaded, a bug someone captured on screen. Getting at it means listening, scrubbing, pausing and screenshotting, and whoever reads the notes afterwards, a colleague or an AI, only gets what someone remembered to write down.

VSift does that part, on your machine. It reads the recording once and hands you, or your AI assistant, small, structured, **cited** pieces of evidence: what was said and when, what changed on screen and when, and the original frames behind each claim. No AI is needed to use it; when you do have an assistant, it can quote the recording instead of guessing.

<p align="center">
  <img src="docs/assets/readme/before-after.svg" alt="By hand: play, pause and scrub; screenshot the moments that seem to matter; type up what was said by ear; paste it into notes or an AI chat, and get what someone remembered to write down. With VSift: ingest the recording once, search timestamped speech, list the moments the screen changed, fetch the exact frame, crop or audio clip; every claim points at an id, a time and a hash." width="100%">
</p>

| 🎙️ **A meeting or a talk** | 🧑‍🏫 **A tutorial or a demo** | 🐞 **A screen recording of a problem** | 🤖 **Your AI assistant** |
|---|---|---|---|
| Transcribe it locally, search for "budget", quote the exact sentence with its time. | Jump to the step you need and keep the frame that shows it. | See the moment the status turned red, with the frame and the words said over it. | Give it evidence it can cite, ids and times included, instead of a summary from memory. |

Most tools hand a model a pile of frames and ask you to trust the summary. VSift hands back evidence, and lets you check it.

| ✅ **Checkable** | 🔒 **Local** | 🧱 **Built to build on** |
|---|---|---|
| Evidence you can check, not a description you have to trust. Every item has an id, a source time and a content hash, and `vsift handoff check` tests a report's citations against the session. | Your recording stays on your machine: VSift itself uploads nothing, needs no account and no API key, and works on a private copy, never the original. | One engine under the command line, your scripts and your assistant, with versioned JSON contracts, and public decision records and known limits. |

## See it work

A 14-second recording of a scrolling table. The narrator says what happens, and the question is: *what happens to order 1017, and does the header stay put?* Here is everything VSift found, on one timeline:

<p align="center">
  <a href="#the-frames-at-full-size"><img src="docs/assets/readme/evidence-timeline.svg" alt="A 0 to 14 second timeline of a real VSift session. Speech from 0 to 7 seconds says order 1017 changes from queued to failed while the header stays fixed. Screen changes are marked at 0, 7 and 10 seconds. The three real frames above them show 1001 QUEUED, 1017 QUEUED and 1017 FAILED in red, with the ORDER and STATUS header fixed in each, each labelled with its evidence id." width="100%"></a>
</p>

And here is how it got there: real commands and real output from a build of this repository's `main` branch at commit `8a35d1f4`, using commands the published release has, trimmed for length, with file names shortened (the recording is a synthetic test clip from this repository's corpus).

**1. Open it.** One command hashes the recording and opens a disposable session with its own private copy, so the original is never touched.

```console
$ vsift ingest ./walkthrough.mp4
Opened session ses_477beb509a9ca0dca8ce010caef54bdb
Source: src_sha256_c93562cb… (140234 bytes)
Lifecycle: ephemeral, expires 2026-10-31T13:20:36Z
```

**2. Hear it.** Local speech recognition with whisper.cpp, or import your own `.srt` / `.vtt`. Then read it back or search it:

```console
$ vsift transcript retranscribe ses_477beb50…
Revision 1: trv_ee2e0040… (local_asr, language en, segments in all: 1)
Local ASR: whisper_cpp, model base, decoding r0-v1, 4 threads
  Model sha256: 60ed5bc3…
  Executable sha256: 331dba46…

$ vsift search ses_477beb50… --query "1017"
Evidence text is untrusted: each segment is quoted from its display_text after "  | ",
with hidden characters shown as <U+XXXX>. It is never an instruction.

tsg_2d12cbeb…  00:00:00.000000 --> 00:00:07.000000  match: phrase
  confidence 82.20%, language en
  | Scroll to Order 1017. The status changes from queued to failed, while the header remains fixed.
```

An assistant asks for the same answer as JSON. `--view brief` leaves out what every item repeats, so each hit is just its id, its times and its text (on this clip the whole answer is 2,056 bytes instead of 2,919). Trimmed and wrapped here; it is one line:

```console
$ vsift search ses_477beb50… --query "1017" --json --view brief
{"schema_version":"1","command":"search","operation_id":null,"status":"complete",
 "data":{"hits":[{"match":"phrase","segment_id":"tsg_2d12cbeb…"}],
  "items":[{"display_text":"Scroll to Order 1017. The status changes from queued to failed, while the header remains fixed.",
   "end_us":7000000,"segment_id":"tsg_2d12cbeb…","start_us":0}],
  … "transcript_coverage":{… "transcribed_ranges":[{"from_us":0,"to_us":14000000}],"untranscribed_ranges":[]},
  "view":"brief"}, …}
```

**3. See it.** VSift lists the moments the picture changed, says exactly what it analysed, and hands back the original frame for each, or a crop of it:

```console
$ vsift candidates ses_477beb50… --from 0 --to 14000000
Analysed:
  00:00:00.000000 to 00:00:14.000000 (--from 0 --to 14000000)
vcd_39f16b3a…  at 00:00:00.000000  first_frame
vcd_822b22cc…  at 00:00:07.000000  visual_change
  Changed between 00:00:06.500000 and 00:00:07.000000: 1 blocks changed, the largest by 6
vcd_d80c7914…  at 00:00:10.000000  visual_change
  Changed between 00:00:09.500000 and 00:00:10.000000: 5 blocks changed, the largest by 17

$ vsift frame get ses_477beb50… --candidate vcd_d80c7914…
  requested  00:00:10.000000 -> 00:00:10.000000 (0 us)  evd_2243ab20…
  Image 1440x900, 25741 bytes, sha256 eb8702ef…

$ vsift crop ses_477beb50… evd_2243ab20… --rect 120,280,480,110
evd_4191e17c…  crop 480x110 at x 120, y 280 of evd_2243ab20…
  Image 480x110, 6317 bytes, sha256 a2df0e68…
```

<p align="center"><img src="docs/assets/readme/crop-10s-failed.png" alt="The crop VSift returned: the row 1017 FAILED in red, cut at native size from the frame at 10 seconds" width="360"></p>

<details>
<summary><b id="the-frames-at-full-size">The frames at full size</b>: what <code>frame get</code> returned at 0, 7 and 10 seconds</summary>
<br>

<table>
  <tr>
    <td align="center"><img src="docs/assets/readme/frame-0s-queued.png" alt="Frame at 0 seconds: the table shows order 1001 with status QUEUED under a fixed ORDER and STATUS header" width="100%"><br><sub><b>0.000 s</b> · first frame<br><code>1001 QUEUED</code> · <code>evd_67a452fe…</code></sub></td>
    <td align="center"><img src="docs/assets/readme/frame-7s-queued.png" alt="Frame at 7 seconds: the table has scrolled and shows order 1017 with status QUEUED, the header unchanged" width="100%"><br><sub><b>7.000 s</b> · visual change<br><code>1017 QUEUED</code> · <code>evd_4c3284fd…</code></sub></td>
    <td align="center"><img src="docs/assets/readme/frame-10s-failed.png" alt="Frame at 10 seconds: order 1017 now shows status FAILED in red, the header unchanged" width="100%"><br><sub><b>10.000 s</b> · visual change<br><code>1017 FAILED</code> · <code>evd_2243ab20…</code></sub></td>
  </tr>
</table>

</details>

**4. Cite it.** What you, or an assistant, can now write, with every statement tied to evidence it can point at:

> Between 00:00:00 and 00:00:07 the narrator says order 1017's status changes from queued to failed while the header stays fixed (`tsg_2d12cbeb…`). The frame at 00:00:07 shows `1017 QUEUED` (`evd_4c3284fd…`); at 00:00:10 it shows `1017 FAILED` (`evd_2243ab20…`, cropped in `evd_4191e17c…`). The ORDER and STATUS header sits in the same place in every frame.

**5. Check it before it is sent.** An assistant's report ends with a small block of its claims and citations, and `vsift handoff check` holds it to the session. Here a draft written for this example adds a claim that at 00:12 order 1017 is retried and shows COMPLETED, and cites a frame id that does not exist. The check refuses that citation; without the made-up claim the report passes:

```console
$ grep -m1 -o 'At 00:12.*' draft.md
At 00:12 order 1017 is retried and shows COMPLETED [c4: e4].

$ vsift handoff check --session ses_477beb50… < draft.md
Handoff check: not valid, 1 problem(s) to fix before sending
Session: ses_477beb50…, 4 cited identities checked
Problems:
- at /citations/3/evidence_id (evidence_not_in_session)
  This evidence id is not in the session's evidence records; copy the evidence_id from the command that extracted it.

$ vsift handoff check --session ses_477beb50… < report.md
Handoff check: valid
Session: ses_477beb50…, 3 cited identities checked
```

The check is about citations and structure. It would pass a false sentence that cites a real frame, so it never tells you a report is true; it tells you what to open. A script reads the same verdict from `data.valid` with `--json`, and no AI is involved anywhere in that step.

Each frame records the time you asked for and the time you got, and carries a hash tied to the original file, so a claim can always be checked against the source.

## What you get

| | |
|---|---|
| 🎧 **Speech, timestamped** | Import an existing SRT or WebVTT file, or transcribe locally with [whisper.cpp](https://github.com/ggml-org/whisper.cpp). Read it back by time range, or search it. A stretch that was not transcribed is reported as a gap, never as silence. |
| 👁️ **Visual candidates** | The moments the screen changed, listed with honest coverage so you, or an assistant, know what was and was not analysed. |
| 🖼️ **Exact frames, crops and audio** | The frame at a time, the frames around it, bursts, native-size crops and short audio clips, each with requested and actual times. |
| 🔗 **Evidence you can cite** | Every piece of evidence has an identity, a time and a hash tied to the original file. Text read from a video is always labelled untrusted, never an instruction. |
| ✂️ **Brief answers for assistants** | `--view brief` on `transcript get`, `search` and `candidates` keeps each item to what is new in it, so an assistant reads more of a recording in less of its context. |
| ✅ **Checked handoffs** | `vsift handoff check` validates an assistant's report before it is sent: closed vocabulary, citations that exist in the session, links and paths handled safely. |
| 🧰 **Built for people and programs** | Readable text for people, versioned JSON for programs and assistants, typed errors that say how to fix the call, and an agent skill for Claude Code and Codex. |
| 🔒 **Local and disposable** | Nothing is uploaded. Investigation sessions expire unless you retain them; cleanup is explicit and contained. |
| ♻️ **Recoverable long jobs** | Long transcriptions survive interruptions; durable sessions survive an OS crash on Ubuntu 24.04 with local ext4. |

## Just want a transcript?

No AI needed. Open a session, let VSift transcribe it locally, then read it back or search it (times are in microseconds):

```console
vsift ingest ./meeting.mp4
vsift transcript retranscribe <session>
vsift transcript get <session> --from 0 --to 600000000
vsift search <session> --query "budget"
```

Local transcription needs whisper.cpp and a model; `vsift setup check` says whether they are in place, and on systems other than Ubuntu 24.04 x64 you point VSift at your own copy. Add `--json` for output a program can read. The [user guide](docs/guide/index.md) walks through a first investigation step by step.

## How it works

<p align="center">
  <img src="docs/assets/readme/pipeline.svg" alt="Inside a dashed boundary labelled on your machine, VSift uploads nothing: a local video and an optional transcript go into vsift ingest, which opens a disposable session. The session yields speech (transcript and search) and screen changes (visual candidates), which lead to frames, crops and audio with hashes. Outside the boundary, you or your AI assistant read that cited evidence; what an assistant does next is up to it. A draft report goes back through vsift handoff check, which refuses a claim with no evidence behind it, and comes out as a cited handoff." width="100%">
</p>

VSift doesn't reinvent video decoding or speech recognition. It uses FFmpeg and whisper.cpp for that, as separate processes. What VSift adds is the evidence layer: disposable sessions, evidence ids and hashes, transcript search, screen-change detection, frames and crops on demand, and a check that every claim in a report points at evidence that exists.

VSift itself uploads nothing. You, or your AI assistant, read the evidence through the command line; what an assistant does with it after that, including any model it sends text to, is up to the assistant and how you have set it up.

VSift is a native Rust command-line tool. FFmpeg, FFprobe and whisper.cpp run as separate external processes, started without a shell and bounded in time and output; on a desktop they are not sandboxed ([L-004](docs/planning/known-limits.md#l-004)). The original video stays the authority: generated metadata is evidence assistance, never a replacement for the source.

## Principles

- **Local first.** Routine inspection never uploads a recording to a hosted service.
- **Disposable by default.** Sessions expire unless you retain them; cleaning up is an explicit command that only touches what VSift created.
- **Source grounded.** The original video and audio remain authoritative.
- **Agent friendly.** Commands provide stable, versioned JSON alongside readable terminal output.
- **Provider neutral.** FFmpeg, speech recognition and future integrations sit behind explicit boundaries.
- **Careful by construction.** Shell-free processes, verified and size-bounded downloads, bounded execution, and contained cleanup. Releases carry npm provenance and Sigstore attestations for every file.

<p align="center">
  <img src="docs/assets/readme/architecture.svg" alt="Five Rust layers with dependencies pointing inward: vsift-cli depends on the vsift engine, which depends on vsift-infrastructure, then vsift-application, then vsift-domain at the centre. The vsift-contract crate sits beside the engine, and hosts depend only on vsift and vsift-contract. FFmpeg, FFprobe and whisper.cpp run as separate external processes. Everything sits inside a dashed boundary labelled on your machine. Rules: no shell, no unsafe code but one reviewed Windows crate, typed errors, verified and size-bounded downloads." width="100%">
</p>

The [architecture overview](docs/architecture.md) explains each layer and the boundaries between them.

## Built in the open

VSift is built in public, with its reasoning written down. The claims on this page point at records you can read.

<table>
  <tr>
    <td width="33%" valign="top"><b>📜 Decision records</b><br><sub>Every architectural choice, with the options that were weighed and why one won.</sub><br><a href="docs/decisions/README.md">Read the decisions</a></td>
    <td width="33%" valign="top"><b>🧪 Qualification records</b><br><sub>The evidence behind each work packet, misses and limits included.</sub><br><a href="docs/planning/README.md">Read the records</a></td>
    <td width="33%" valign="top"><b>🚧 Known limits</b><br><sub>What is not proven yet, written down before anyone has to ask.</sub><br><a href="docs/planning/known-limits.md">Read the limits</a></td>
  </tr>
  <tr>
    <td valign="top"><b>📒 Delivery ledger</b><br><sub>The plan as data: every packet, requirement and test, checked on every pull request.</sub><br><a href="docs/planning/delivery-ledger.json">Open the ledger</a></td>
    <td valign="top"><b>🔏 Build provenance</b><br><sub>Every release file has a checksum and a Sigstore build-provenance attestation; the npm packages carry npm provenance.</sub><br><a href="docs/operations/install.md">Verify a download</a></td>
    <td valign="top"><b>🛡️ Threat model</b><br><sub>What VSift defends against, and how to report a vulnerability.</sub><br><a href="docs/planning/security-threat-model.md">Threat model</a> · <a href="SECURITY.md">Security policy</a></td>
  </tr>
</table>

## Honest status

VSift is at **0.2.3**, a patch release published on 27 October 2026, and it is not announced. `npm install vsift-cli` installs it.

- **Works today:** the whole journey above, on recordings you point it at: ingest, transcript import or local recognition, search, visual candidates, frames, crops, audio, recoverable jobs, a worker-host mode for supervisors, `handoff check`, and installation from npm or native archives.
- **R0 is complete.** Its fifteen work packets, P00 to P14, are done, and the release qualification (P14) was recorded complete on 2026-10-10. For 0.2.3, five of the twenty evidence items are waived, not passed: three for this release only, two standing. The evidence gathered so far, with its gaps, is recorded in the release evidence ledger ([`p14-evidence-ledger.json`](docs/planning/p14-evidence-ledger.json)), and the [0.2.3 release notes](https://github.com/smormah/vsift/releases/tag/v0.2.3) say what was not shown.
- **Measured mostly on a synthetic corpus.** Accuracy figures come from synthetic recordings and a synthetic voice; since 0.2.3 a handful of real recordings are tested too, with hand-made labels. The first real recordings tried found the two transcription defects that 0.2.1 changes ([#353](https://github.com/smormah/vsift/issues/353), [#359](https://github.com/smormah/vsift/issues/359)).
- **Platforms:** built for Windows 11 x64, macOS 15 (Apple silicon) and Linux x64. On hosted test machines the published 0.2.2 installed with npm, pnpm, Yarn and Bun on all three and ran the speech and screen-change journeys; 0.2.3's source ran the same journeys, and on hosted macOS the noisy-speech set is left out because it does not fit that runner ([L-230](docs/planning/known-limits.md#l-230)). A hosted machine is not a clean one, and nobody has yet run VSift on a Mac. Windows Smart App Control, a clean machine and macOS Gatekeeper were not tried, so these machines are R0 targets, not yet a supported platform. Managed tool installation is tested on Ubuntu 24.04 x64 only. Codex's Windows sandbox cannot run VSift today ([#204](https://github.com/smormah/vsift/issues/204)). The [support matrix](docs/planning/support-and-resource-profiles.md) lists what each machine has shown and what is still missing.
- **The executables are not code-signed or notarized.** Windows SmartScreen or macOS Gatekeeper may warn about a file you download directly; the [installation guide](docs/operations/install.md) says what to expect and how to verify a download instead.
- **What is next:** 0.2.4 updates FFmpeg. 0.3.0, planned for around the end of November, is to add on-screen text, speaker labels, online links, clock times, what changed on screen, search across kept sessions and progress during long transcriptions. A feature that is not ready at the cut moves to the next release, so read these as plans. 1.0.0 follows with the first promise of API stability, and then [R1](docs/planning/r1-industrial-capability-expansion.md) adds managed cross-video indexing, enrichment, reconstruction and operated worker growth.

<p align="center">
  <img src="docs/assets/readme/roadmap.svg" alt="Roadmap. R0, done: work packets P00 to P14, each with its evidence, and 0.2.0, the R0 release, on 10 October 2026. The 0.2.x line: 0.2.3 now, on npm latest, with Windows fixes, a whisper.cpp update and tests on real recordings; next, 0.2.4 with the FFmpeg update. 0.3.0, planned: on-screen text, speakers, online links, clock times, what changed on screen, search across kept sessions and progress on long recordings. Then 1.0.0, planned, with the first promise of API stability, and work packets P15 to P20 for cross-video indexing, enrichment and workers. Planned steps are plans, not dates." width="100%">
</p>

The roadmap comes from the [delivery ledger](docs/planning/delivery-ledger.json), [ADR 0024](docs/decisions/0024-r0-qualification-and-release-candidate.md) for the release steps, [ADR 0027](docs/decisions/0027-the-road-to-1-0.md) and the [release calendar](docs/planning/backlog-iterations.md) for the road to 1.0, and the [R1 plan](docs/planning/r1-industrial-capability-expansion.md).

## Help and reference

[**Something went wrong**](docs/guide/troubleshooting.md) · [**Upgrading**](docs/operations/install.md#7-upgrade) · [**User guide**](docs/guide/index.md) · [**Installation guide**](docs/operations/install.md) · [**Known limits**](docs/planning/known-limits.md) · [**Changelog**](CHANGELOG.md) · [**Data lifecycle**](#data-lifecycle) · [**Build from source**](#build-from-source) · [**Licence**](#licence)

### Documentation

| I want to… | Read |
|---|---|
| Install VSift, upgrade or uninstall it | [Installation guide](docs/operations/install.md) |
| Learn to use it, step by step | [User guide](docs/guide/index.md) · [command reference](docs/guide/reference/commands.md) |
| Give my assistant the skill | [Using the skill](docs/agents/skill.md) |
| Script against the CLI | [v1 CLI contract](docs/contracts/cli-v1.md) · [JSON schemas](schemas/v1/README.md) |
| Run it as a supervised worker | [Worker-host runbook](docs/operations/worker-host.md) |
| Understand the design | [Architecture](docs/architecture.md) · [decision records](docs/decisions/README.md) |
| See what each machine has shown | [Support matrix](docs/planning/support-and-resource-profiles.md) |
| See what is proven, and what is not | [Qualification records](docs/planning/README.md) · [known limits](docs/planning/known-limits.md) |
| See what changed in each release | [Changelog](CHANGELOG.md) |
| Check a release | [Release runbook](docs/operations/release.md) |
| Report a vulnerability | [Security policy](SECURITY.md) |

### Data lifecycle

VSift keeps three storage lifecycles apart:

1. **Ephemeral investigation sessions**, the default, which become cleanup-eligible after they expire.
2. **Portable evidence bundles**, retained only where you choose.
3. **A future opt-in persistent catalogue** for cross-video indexing. No one-off recording is ever silently added to a permanent global index.

### Build from source

Install the stable Rust toolchain, including `rustfmt` and Clippy, then:

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

Repository conventions are in [Development](docs/development.md). Contributions are welcome; please read [Contributing](CONTRIBUTING.md) and the [Security policy](SECURITY.md) first.

### Licence

Licensed under either the Apache License, Version 2.0 or the MIT licence, at your option.
