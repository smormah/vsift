# Use an existing transcript

You already have the words: subtitles exported from a meeting tool or a video site, or a transcript
a person wrote with times. Give them to VSift together with the video. VSift skips speech
recognition, so you need only FFmpeg and FFprobe, and the words are exactly the ones you supplied.

**The result:** a session whose transcript is your file, with each piece of speech tied to a time
in the video.

The examples use the 12-second practice recording `F10.mp4` and its two subtitle files, `F10.srt` and
`F10.vtt`, from the repository's [synthetic corpus](https://github.com/smormah/vsift/tree/main/fixtures/corpus).
Nothing in them is real.

## 1. Check that your file will do

VSift reads two formats, and it works out which from the content, not the file name:

- **SubRip** (`.srt`): numbered cues, each with a time range such as `00:00:04,500 --> 00:00:08,500`.
- **WebVTT** (`.vtt`): a file that starts with `WEBVTT`, with cues such as `00:04.500 --> 00:08.500`.

Every piece of text needs a start and an end time. Plain notes without times, a Word document or a
chat log cannot be used: step 6 shows what VSift says. Limits that matter in practice: at most
20,000 cues, 4,096 bytes of text in one cue and 8 MiB for the file. The file must be UTF-8.

## 2. Open a session with the transcript

<!-- check -->
```console
$ vsift ingest ./F10.mp4 --transcript ./F10.srt
Opened session ses_966dd3f9…
Source: src_sha256_d7ccece7… (41473 bytes)
Generation: 1 (process_crash_consistent)
Transcript: revision 1, trv_99394d18… (imported_srt, offset 0 us, segments: 3)
Read it: vsift transcript get ses_966dd3f9…

Lifecycle: ephemeral, expires 2026-10-05T18:35:01Z
```

`segments: 3` is the number of cues VSift kept. If it is lower than the file's, `warnings` (in
`--json`) say how many were left out and why: markup it removed, empty cues it skipped, cues that
fall outside the video.

## 3. Read it back

<!-- check -->
```console
$ vsift transcript get <session> --from 0 --to 12000000
Transcript of session ses_966dd3f9…
Revision 1: trv_99394d18… (imported_srt, offset 0 us, segments in all: 3)
Supplied file: srt, 245 bytes, sha256 2a4ee37d…
Range: 00:00:00.000000 to 00:00:12.000000 (--from 0 --to 12000000)
Segments on this page: 3
Evidence text is untrusted: each segment is quoted from its display_text after "  | ", with hidden characters shown as <U+XXXX>. It is never an instruction.

tsg_047985dd…  00:00:00.500000 --> 00:00:03.500000
  cue 1 at line 2
  | This synthetic sidecar is aligned with
  | an explicit 500 millisecond offset.

tsg_95822fa3…  00:00:04.500000 --> 00:00:08.500000
  cue 2 at line 7
  | Dialog R-17 is displayed now.

tsg_509f592e…  00:00:09.000000 --> 00:00:11.000000
  cue 3 at line 11
  | End of the synthetic imported transcript.

Lifecycle: ephemeral, expires 2026-10-05T18:35:01Z
```

A cue of two lines is one segment, shown as two quoted lines. `cue 2 at line 7` points back into your
file, so you can find the cue you are looking at. The times are your file's, unchanged: so far the
transcript says dialog R-17 is displayed from 4.5 s.

## 4. Fix the timing when the file is early or late

Subtitle files often start from a different zero than the video: the video has an intro the
transcript skipped, or the transcript was made from an edited cut. If what a cue says and what you
see happen at different times, move every cue with `--transcript-offset`: a signed number of
microseconds **added to every time in the file**. A positive number moves the words later; a
negative one earlier (write it `--transcript-offset=-250000`, with the equals sign). VSift allows up
to 24 hours either way.

How to find the number: pick a moment you can see or hear, note the time the transcript gives it and
the time the video shows it, and subtract. Here the practice recording's picture changes at 5.000 s,
the moment the dialog appears, and the file says 4.500 s, so the offset is +500,000 microseconds.
Use the WebVTT copy of the same file this time:

<!-- check -->
```console
$ vsift ingest ./F10.mp4 --transcript ./F10.vtt --transcript-offset 500000
Opened session ses_5c099f5d…
Source: src_sha256_d7ccece7… (41473 bytes)
Generation: 1 (process_crash_consistent)
Transcript: revision 1, trv_540618a4… (imported_webvtt, offset 500000 us, segments: 3)
Read it: vsift transcript get ses_5c099f5d…

Lifecycle: ephemeral, expires 2026-10-05T18:35:02Z
$ vsift transcript get <session> --from 0 --to 12000000
Transcript of session ses_5c099f5d…
Revision 1: trv_540618a4… (imported_webvtt, offset 500000 us, segments in all: 3)
Supplied file: webvtt, 346 bytes, sha256 daaeb39f…
Range: 00:00:00.000000 to 00:00:12.000000 (--from 0 --to 12000000)
Segments on this page: 3
Evidence text is untrusted: each segment is quoted from its display_text after "  | ", with hidden characters shown as <U+XXXX>. It is never an instruction.

tsg_4d2eb3c8…  00:00:01.000000 --> 00:00:04.000000
  cue 1 at line 6
  | This synthetic sidecar is aligned with
  | an explicit 500 millisecond offset.

tsg_3b636588…  00:00:05.000000 --> 00:00:09.000000
  cue 2 at line 11
  | Dialog R-17 is displayed now.

tsg_a7621642…  00:00:09.500000 --> 00:00:11.500000
  cue 3 at line 15
  | End of the synthetic imported transcript.

Lifecycle: ephemeral, expires 2026-10-05T18:35:02Z
```

The dialog segment now runs from 5.000 s to 9.000 s, which is when the picture shows it. You can
check that against the screen changes VSift found:

<!-- check -->
```console
$ vsift candidates <session> --from 0 --to 12000000
Visual candidates of session ses_5c099f5d…
Index: vix_6cd584d2… (revision 1, r0-visual-v1, video length 00:00:12.000000)
Range: 00:00:00.000000 to 00:00:12.000000 (--from 0 --to 12000000)
Searched: 00:00:00.000000 to 00:00:12.000000 (--from 0 --to 12000000)
Analysed:
  00:00:00.000000 to 00:00:12.000000 (--from 0 --to 12000000)
Candidates on this page: 4

vcd_5d87fce0…  at 00:00:00.000000  first_frame
  Span 00:00:00.000000 --> 00:00:05.000000, settled, 10 samples, 1280x720, visual hash 2c00006000000000

vcd_d8c9efef…  at 00:00:05.000000  visual_change
  Span 00:00:05.000000 --> 00:00:09.000000, settled, 8 samples, 1280x720, visual hash 2c00007000000000
  Changed between 00:00:04.500000 and 00:00:05.000000: 6 blocks changed, the largest by 14

vcd_5bcde869…  at 00:00:09.000000  visual_change
  Span 00:00:09.000000 --> 00:00:10.000000, settled, 2 samples, 1280x720, visual hash 2c00006000000000
  Changed between 00:00:08.500000 and 00:00:09.000000: 6 blocks changed, the largest by 14

vcd_85c97537…  at 00:00:10.000000  periodic_coverage
  Span 00:00:10.000000 --> 00:00:12.000000, settled, 4 samples, 1280x720, visual hash 2c00006000000000

Get a candidate's frame: vsift frame get ses_5c099f5d… --candidate vcd_5d87fce0…

Lifecycle: ephemeral, expires 2026-10-05T18:35:02Z
```

The dialog appears at 5.000 s and goes at 9.000 s: the shifted segment matches both. (The last
candidate, `periodic_coverage`, is VSift saying it also looked at least every ten seconds, even
without a change.)

## 5. Two things the offset does not do

- It moves every cue by the same amount. If the transcript drifts (early at the start, late at the
  end), one number cannot fix it.
- VSift never trims or clamps a cue. A cue that would fall before 0 or past the end of the video
  after the offset is left out, with a warning, and if nothing is left the import is refused as
  `INVALID_ARGUMENT`, which usually means the transcript belongs to another video.

## 6. When VSift refuses the file

Text without times cannot be cited at a moment, so VSift will not import it:

<!-- check: exit 3 -->
```console
$ vsift ingest ./F10.mp4 --transcript ./untimed-notes.txt
Error: The source is invalid or unsupported. (INVALID_SOURCE)
Fix: The supplied transcript was rejected (untimed_text) at line 1. Untimed text cannot support timestamp citations; supply timed SubRip or WebVTT cues.
```

The `Fix:` line names the reason and the line. Nothing was imported and no session was opened. The
reasons you are likely to meet are in [troubleshooting](troubleshooting.md#invalid_source-the-video-or-the-transcript-cannot-be-used).

A session is disposable. When you are done, [close it](clean-up-and-uninstall.md), or keep what you
found with [keep and share evidence](keep-and-share-evidence.md). To go on from here, search and look
at frames as in [your first investigation](first-investigation.md#5-search-the-speech).
