# Troubleshooting

When a VSift command fails it says why, in a fixed form, and almost always what to do next. This page
explains how to read that, lists every failure code with its exit status, and walks through the situations
people meet most. If you are an AI assistant or a program, the same facts are in the JSON form of the error
([JSON reference](reference/json.md)).

## How to read an error

A failure goes to the error stream, prints nothing else, and ends the command with a non-zero exit status:

```text
Error: The command line arguments are invalid. (INVALID_ARGUMENT)
Fix: The requested time is at or after the end of the video, so no frame is displayed there. Nothing was changed. Request a time before the end of the video.
```

- `Error:` is a fixed sentence and **the code in brackets**, which is what to look up below and what a program
  should act on.
- `Fix:` is one thing to do. When the fix is a command, a `Run:` line gives it exactly (for a rejected command line,
  the help for the command you meant). There can be several `Fix:` lines.
- `Affected:` lists the ids involved, and `Retry after: <ms> ms` appears when waiting is the fix.
- With `--json` the same facts are in the `error` object of the result: `code`, `message`, `retryable`,
  `retry_after_ms`, `affected_ids` and `remediation`.

The fixed sentences of an error never repeat text from your video, a transcript or a file name, so a hostile
file name or a line of a transcript cannot reach your terminal through them. (That also means an error often does
not name the file it is about: you know which command you just ran.) The one exception is a rejected command line:
there VSift quotes the command line under a label that says it is untrusted text.

## The failure codes

The exit status is the command's own, so a script can tell the classes apart. The exit statuses 126 and
127 never come from `vsift` itself; they are the npm launcher's (see [Problems installing](#problems-installing-or-starting-vsift)).

| Code | Exit | What it means | What to do |
| --- | ---: | --- | --- |
| `INTERNAL` | 1 | Something failed inside VSift that is not your input's fault. | Run the command again. If it happens again, report it (last section), with the output of `vsift setup check --json`. Do not attach your recording or transcript. |
| `INVALID_ARGUMENT` | 2 | The command line is wrong, or a value does not make sense for this video or session. | Read the `Fix:` line and run the `Run:` command: it is the help for the command you meant. Examples below. |
| `UNSUPPORTED_SCHEMA` | 2 | A stored record, bundle or worker request is in a version this VSift does not know. | Use the VSift that wrote it, or a newer one. For a worker request, fix its `schema_version`. |
| `MISSING_CAPABILITY` | 2 | A tool or model VSift needs is missing or does not pass its check, or this machine cannot do what you asked (a durable workspace needs Ubuntu 24.04 with ext4). | Run `vsift setup check` and follow it. [Below](#missing_capability-a-tool-or-model-is-missing). |
| `ISOLATION_UNAVAILABLE` | 2 | You asked for the Linux worker boundary (`--host-isolation strict-linux`, an operator option) on a machine that cannot show it. | Leave the option out. Operators: [worker runbook](../operations/worker-host.md). |
| `COMMAND_NOT_IMPLEMENTED` | 2 | A command this version reserves but does not have. No command answers it in this release. | Nothing to fix; a page that names the command is ahead of your version. |
| `IDEMPOTENCY_CONFLICT` | 2 | A worker request reused an operation id for a different request. | Use a new operation id. Operators only: [runbook](../operations/worker-host.md). |
| `INVALID_SOURCE` | 3 | VSift cannot use the video or the transcript you named. | [Below](#invalid_source-the-video-or-the-transcript-cannot-be-used). |
| `BUSY` | 4 | Another VSift process is using the same session or the machine's share of capacity. It is meant to be retried. | Wait for the `Retry after:` time and run the same command again. |
| `DEADLINE_EXCEEDED` | 5 | The command ran past its time limit. | Ask for a smaller range of the video and run it again. Work that was already finished, such as the chunks of a transcription, is kept. |
| `RESOURCE_LIMIT` | 5 | The request is larger than a built-in limit: too many frames or results, a file over a size bound, a recogniser that ran out of memory. | Ask for less: a shorter range, fewer frames, a smaller file. [Limits](limits.md) lists the numbers. |
| `CANCELLED` | 6 | You pressed Ctrl-C, or a supervisor stopped the command. | Run it again. Long work (a transcription) continues from where it stopped. |
| `STORAGE_IO` | 7 | Reading or writing a file failed: the path is wrong, the file cannot be read, a link where a file is expected, the disk is full. | [Below](#storage_io-a-file-could-not-be-read-or-written). |
| `INTEGRITY_FAILURE` | 7 | Something VSift stored does not match its own record, or a folder VSift did not create was named as its session root. | [Below](#integrity_failure-what-is-stored-is-not-what-vsift-wrote). |
| `DOWNLOAD_FAILED` | 7 | `setup install` could not fetch a tool. | The reason is in the message; the [install guide](../operations/install.md#9-proxies-firewalls-and-corporate-environments) has a table of reasons and what to do. |

## `INVALID_ARGUMENT`: the command line or a value is wrong

The most common failure, and the one with the most help in the message. Two real ones:

**A session with no transcript yet.** You searched before there were any words. Give VSift a transcript, or
let it listen:

<!-- check -->
```console
$ vsift ingest ./F04-speech.mp4
Opened session ses_9f3a6c1e…
Source: src_sha256_c93562cb… (140234 bytes)
Generation: 1 (process_crash_consistent)
Transcript: none imported

Lifecycle: ephemeral, expires 2026-10-05T19:00:00Z
```

<!-- check: exit 2 -->
```console
$ vsift search <session> --query hello
Error: The command line arguments are invalid. (INVALID_ARGUMENT)
Fix: This session has no transcript yet. Open the video again with ingest --transcript to import a SubRip or WebVTT file, or run transcript retranscribe with this session to transcribe its speech locally with whisper.cpp.
```

**A time past the end of the video:**

<!-- check: exit 2 -->
```console
$ vsift frame get <session> --at 99000000
Error: The command line arguments are invalid. (INVALID_ARGUMENT)
Fix: The requested time is at or after the end of the video, so no frame is displayed there. Nothing was changed. Request a time before the end of the video.
```

Other causes you will meet:

- **A missing or unknown option.** The message carries a `Run: vsift <command> --help` line: run it.
- **PowerShell and `--rect`.** `crop --rect 10,20,300,80` is split by PowerShell at the commas into several
  arguments. Quote it: `--rect "10,20,300,80"` (the message says so).
- **A negative number.** Write it with an equals sign: `--transcript-offset=-250000`.
- **A transcript whose times do not fit the video** (`no_cues_within_source`): check the offset and that the
  file belongs to this video ([use an existing transcript](use-an-existing-transcript.md)).
- **`handoff check --file`** needs an absolute path.

## `INVALID_SOURCE`: the video or the transcript cannot be used

VSift refuses a file it cannot trust or read, and for most reasons it says nothing more than that the source is
invalid:

<!-- check: exit 3 -->
```console
$ vsift ingest ./not-a-video.mp4
Error: The source is invalid or unsupported. (INVALID_SOURCE)
```

Go through these, in order:

1. **Is it a video VSift opens?** MP4 and MOV files and Matroska and WebM files, with video in H.264, H.265, VP8,
   VP9, AV1 or MPEG-4 and audio in AAC, MP3, Opus, Vorbis, FLAC or uncompressed PCM. An AVI, FLV or WMV file, or
   a video in another codec, is refused: re-save it as MP4 or MKV with your video editor or FFmpeg, then open the new
   file. Check that the file plays in an ordinary player.
2. **Is it a regular file?** A folder, a named pipe or a device is refused. So is a link: name the file the link points to.
3. **Is it too big?** Over 20 GiB, or four hours, or very many streams, or pictures larger than 16 megapixels.
   [Limits](limits.md) has the numbers.
4. **Did it take ten minutes and then fail?** Then nothing is wrong with the video, whatever the first line says.
   Opening a video copies it into VSift's own folder, and the copy is stopped after ten minutes. This one has a
   `Fix:` line, which begins "Copying this video into VSift's session took longer than the ten-minute limit". It
   happens with a big video on a network drive, a memory stick, an SD card or a folder that downloads from the cloud
   when first read. Copy the video to a disk in your computer and open that copy: that usually fixes it. If VSift's
   own folder is on a network drive or a slow disk, that can be the slow side instead. Trying again from the same
   place starts the copy from the beginning.
5. **If it is a transcript** (the message comes from `ingest --transcript`): the `Fix:` line names the reason and the
   line. The usual ones are text with no times (`untimed_text`), a timestamp in the wrong form
   (`invalid_timestamp`), cues out of order and a file that is not UTF-8. Export it again as SRT or WebVTT.

## `STORAGE_IO`: a file could not be read or written

<!-- check: exit 7 -->
```console
$ vsift ingest ./no-such-video.mp4
Error: Storage or output I/O prevented completion. (STORAGE_IO)
```

This one has no `Fix:` line, because VSift does not echo paths. In practice:

- **The path is wrong.** Check the spelling and the folder you are in. A relative path is read from your current
  folder.
- **The file cannot be read** (permissions, another program has it locked) or **the disk is full**: free some
  space (a session holds a copy of the video, so it needs at least that much room, plus room for the frames) and
  run it again.
- **A link where a file is expected.** Name the file itself.
- **An id that does not exist.** A session id with a typo, or one that has been cleaned up, can end here: check
  `vsift session list`. An expired session cannot be used; open the video again.
- **VSift's own folders are open to other accounts.** The message says so, in fixed words, and the fix is to
  remove the extra access or delete that folder and retry ([install guide](../operations/install.md#12-problems)).

A failed `ingest` can leave a session in the list marked `initializing`; a cleanup removes it
([clean up and uninstall](clean-up-and-uninstall.md)).

## `MISSING_CAPABILITY`: a tool or model is missing

VSift needs FFmpeg and FFprobe for everything that looks inside a video, and, for recognising speech, whisper.cpp and
a model. When one is missing, the error says which kind of thing, and `setup check` says exactly what:

```console
$ vsift ingest ./F04-speech.mp4 --transcript ./F04-speech.srt
Error: A required local capability is unavailable. (MISSING_CAPABILITY)
Fix: Importing a supplied transcript needs FFmpeg and FFprobe to measure the video, but not Whisper or a model. Install or locate trusted builds, register them with setup configure, then run setup check.
```

```console
$ vsift setup check
VSift setup check
Profile: desktop
Status: blocked
[missing] FFmpeg (media_processing): not found on PATH [filtered PATH]
  Install or locate this trusted tool, then rerun setup check with its absolute path using --ffmpeg. Managed installation is not available for this target.
[missing] FFprobe (media_processing): not found on PATH [filtered PATH]
  Install or locate this trusted tool, then rerun setup check with its absolute path using --ffprobe. Managed installation is not available for this target.
[missing] Whisper (transcription): not found on PATH [filtered PATH]
  Install or locate this trusted tool, then rerun setup check with its absolute path using --whisper. Managed installation is not available for this target.
Media executable probes are blocked until FFmpeg and FFprobe respond.
Local ASR model: not registered (setup configure-model --file <path>)
Local ASR check: not run: FFmpeg and FFprobe are needed and must pass their own check
Executable probes only show that each tool responds. The local ASR check transcribes a short speech clip built into VSift with the selected tools and model. A supplied transcript can avoid local ASR.
```

*Both blocks are real output from a Windows 11 machine whose `PATH` had no media tools; they are not checked
automatically because they depend on the machine.* The fix is the same everywhere: install the tool, or tell
VSift where it is:

```console
vsift setup configure ffmpeg --executable /absolute/path/to/ffmpeg
vsift setup configure ffprobe --executable /absolute/path/to/ffprobe
vsift setup check
```

On Ubuntu 24.04 `vsift setup plan --profile desktop` lets VSift install reviewed builds itself, after you read
and accept the plan. A tool that is present but "failed VSift's media-tool check" is a different problem: it is
an FFmpeg that starts and does the wrong thing, so install a standard build. [Install guide, section 5](../operations/install.md#5-install-ffmpeg-ffprobe-whispercpp-and-the-speech-model).

## `INTEGRITY_FAILURE`: what is stored is not what VSift wrote

Two causes cover nearly every case.

- **A `--session-root` folder VSift did not create.** If you gave `--session-root` a folder that already existed,
  even an empty one you made yourself, VSift will not adopt it. Name a path that does not exist yet, or delete the
  folder if it holds nothing you need ([install guide](../operations/install.md#12-problems)). `--session-root` is an
  operator option that ordinary use never needs: leave it out and VSift uses its own private folder.
- **A bundle that was changed or damaged** since it was saved. `bundle validate` names that. Restore the bundle from
  where it came from.

## Problems installing or starting VSift

If `vsift` starts and prints a message beginning `vsift (npm launcher):`, the launcher in front of the real program
refused to run it and exited with 126 or 127:

| Exit | What happened | What to do |
| ---: | --- | --- |
| 127 | No program for your machine is installed: optional dependencies were left out (`--omit=optional`), the lockfile came from another system, or the machine is not one the release is built for | Reinstall `vsift-cli` with optional dependencies, on Windows 11 x64, macOS 15 on Apple silicon or Linux x64 |
| 126 | The program was found but refused: a different version, a changed file, or it could not start (on Linux, glibc or OpenSSL 3 missing; on Windows, a very long install path) | Reinstall `vsift-cli`. [Install guide, section 10](../operations/install.md#10-what-the-launcher-checks-and-its-exit-codes-126-and-127) |

On Windows, do not start `vsift` through `cmd.exe` with text you did not write (a search taken from a
transcript, for example): the command file npm writes lets `cmd.exe` read the command line a second time. Use
PowerShell or Git Bash ([install guide, section 2](../operations/install.md#2-install-with-a-package-manager)).

## When nothing here helps

1. Run `vsift setup check --json` and `vsift --version`.
2. Look for the same error in [the issues](https://github.com/smormah/vsift/issues).
3. If you report it, remove secrets, personal information, transcripts and sensitive paths first, and use a
   synthetic or rights-safe video if you can ([SUPPORT.md](../../SUPPORT.md)). A suspected security
   problem is not reported in public: [SECURITY.md](../../SECURITY.md).
