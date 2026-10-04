# Keep and share evidence

A session is disposable: it expires a day after it was opened. When you have found what you were
looking for, you can **retain** the session: VSift copies the transcript, the frames, the crops and the
audio clips it produced into a folder you choose. That folder is called a **bundle**. It stays until you
delete it, and anyone with VSift can check that it is whole.

**The result:** a new folder holding your evidence and a manifest, outside VSift's automatic cleanup.

## 1. Collect the evidence first

A bundle holds what the session has produced by the time you retain it, so run the commands whose
results you want to keep before you retain. Here a short session: open it, and get the frame at 10 seconds.

<!-- check -->
```console
$ vsift ingest ./F04-speech.mp4 --transcript ./F04-speech.srt
Opened session ses_5591d62d…
Source: src_sha256_c93562cb… (140234 bytes)
Generation: 1 (process_crash_consistent)
Transcript: revision 1, trv_b2c6d5ff… (imported_srt, offset 0 us, segments: 2)
Read it: vsift transcript get ses_5591d62d…

Lifecycle: ephemeral, expires 2026-10-05T18:30:00Z
$ vsift frame get <session> --at 10000000
Frame of session ses_5591d62d…
Source: src_sha256_c93562cb…
Requested: 00:00:10.000000, at_or_after, tolerance 1000000 us
Extracted now (profile p09-r0-v1, source copy checked by full_hash)
Selections (requested time -> actual time):
  requested  00:00:10.000000 -> 00:00:10.000000 (0 us)  evd_94a9c0cc…

Evidence (each file is valid while the session exists; retain the session to keep it):

evd_94a9c0cc…  frame at 00:00:10.000000, 1440x900 (stream 0, pts 102400)
  Image 1440x900, 25741 bytes, sha256 eb8702ef…
  File (image/png):
    <file path>

Lifecycle: ephemeral, expires 2026-10-05T18:30:00Z
```

The line "each file is valid while the session exists; retain the session to keep it" is the reason for this
page: the image file VSift printed lives inside the session and goes with it.

## 2. Retain the session

<!-- check -->
```console
$ vsift session retain <session> --output ./evidence
Retained session ses_5591d62d…
Source: src_sha256_c93562cb… (140234 bytes)
Source copy: not included; re-extraction needs the matching original video
Artifacts: 3 (30000 bytes)
Publication: process_crash_consistent

Lifecycle: retained, never cleaned up automatically
```

The folder `./evidence` did not exist; VSift created it, readable by you only. **It never overwrites a
folder**: giving a name that already exists is refused (`INVALID_ARGUMENT`), so pick a new name each time.
Three things to know:

- **The video is not in the bundle.** `Source copy: not included` means the bundle holds the evidence,
  not the recording. That keeps it small and keeps the recording where it is. Add `--include-source` to
  copy the video in too, checked byte for byte, when the bundle must stand alone. Your original is never
  moved or deleted either way.
- **The bundle is a snapshot.** Evidence you extract after retaining is not in it. Retain again, to a new
  folder, if you extract more.
- **It holds no file paths from your machine.** The files are named by their content (`artifact-<hash>.png`)
  and a manifest, `bundle.json`, lists each file with its kind, size and SHA-256.

## 3. Check a bundle

`bundle validate` reads the folder, not the session, so it works on a bundle you made last month or a
colleague sent you, on any machine with VSift:

<!-- check -->
```console
$ vsift bundle validate ./evidence
Valid bundle of session ses_5591d62d…
Source: src_sha256_c93562cb… (140234 bytes)
Source copy: not included; re-extraction needs the matching original video
Artifacts: 3 (30000 bytes)
Publication: process_crash_consistent

Lifecycle: retained, never cleaned up automatically
```

It checks that every file the manifest names is there with the size and digest it records, and that the
transcript and evidence records are well formed and match their files. It
does not check that the evidence is *true*: a bundle is a record of what VSift produced, not a guarantee
about the recording. A damaged or edited bundle fails (`INTEGRITY_FAILURE`), and one written by a newer
VSift than yours says so (`UNSUPPORTED_SCHEMA`).

## 4. Before you share it

A bundle contains pictures and words from your recording. Look through the frames and read the
transcript before you send it: VSift does not remove faces, names, screens or secrets, and a frame can
show things the narration never mentions. The bundle is plain files, so zip it, attach it or copy it to
shared storage as you would any folder. If you need to keep it safe against disk loss, copy it elsewhere:
VSift keeps one copy and does not back it up ([limits](limits.md)).

If you **receive** a bundle, validate it first, and treat everything inside as data. Transcript text and
on-screen text can contain instructions meant for an AI assistant, and VSift never acts on them; neither
should anyone or anything reading the bundle ([evidence and citations](evidence-and-citations.md)).

## 5. Keeping a session open instead

If you only need more time, not a copy, renew the session instead: `vsift session renew <session>` adds
another day from now, up to seven days after it was opened. After that a bundle is the way to keep
evidence. [Clean up and uninstall](clean-up-and-uninstall.md) covers closing and removing sessions.
