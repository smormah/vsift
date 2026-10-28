# The real-media set and its opt-in campaign (#363)

Status (2026-10-20, cycle 2, `0.2.3`): **step 1** (the design, the candidates, the manifest and the fetch-and-verify tool)
is merged (#460). **All eight items are pinned** (2026-10-20: the five hand downloads, checked against Commons; the release
`test-media-2026-10-19` holds all eight since 2026-10-20). **Step 2** (#465, merged) holds the maintainer's decisions (section 10), the campaign runner and its manual
hosted workflow, and what followed Wikimedia's refusal of the tool (sections 12 and 14): **the set is eight approved items
(440,841,704 bytes)**. Three come from sources that accept the tool's neutral user agent (the Internet Archive and NASA's
library) and are fetched and pinned; five are Wikimedia files with no copy elsewhere, which the maintainer downloads by hand
and the tool pins against the Commons SHA-1 (`pin --from-file`). Every hosted run fetches from the project's own test-media
release, never from Wikimedia; the release is prepared for the maintainer's review, not created by a tool. A private local set
of the maintainer's own recordings is supported for local runs only, reported by measurements alone (section 13).
Issue: [#363](https://github.com/smormah/vsift/issues/363). Register: [L-180](known-limits.md#l-180).

## 1. Why

Every accuracy and robustness figure VSift has comes from the synthetic corpus ([`fixtures/corpus/`](../../fixtures/corpus/README.md)):
generated screens and one synthetic voice per sentence. **The three real defects of `0.2.0` came from real recordings and a
second machine, never from that corpus:** a chunk with little speech failed a whole transcription (#353), chunk seams dropped
speech with no warning (#359), and a healthy FFmpeg was reported as timed out (#391). Real recordings have pauses, typing,
music, room noise, several speakers, accents, other languages, long silences and compression; none of that is in the
corpus, so the trials, the campaigns and the journeys cannot see this class of defect.

The purpose is **to find bugs fast in the development cycle**, not to publish accuracy figures: a defect found by a real
recording becomes an issue, its fix gets a regression test that does not need the recording (provider output of the
failing shape, as #353's tests do), and the item stays in the set so the next run shows that it holds. **The audience is
anyone with a video** (screen recordings, walkthroughs, meetings, lectures, recorded talks), so the set is not only bug
reports.

## 2. What the set must cover, and what each shape settles

Five to eight items, a few hundred MB in all, each short where it can be. One item may cover several shapes.

| Shape (`covers` in the manifest) | What it exercises | Findings it should help settle |
| --- | --- | --- |
| `conversational-pauses` | Speech with long pauses, typing or clicking between phrases: chunks with little speech | #353's shape (regression), L-158 / #383 (rejected segments in sparse chunks) |
| `long-speech-seams` | Continuous speech across many 30-second seams | #359, #380 to #384 (L-155 to L-159), above all #381's half-second tolerance |
| `screen-ui-small-text` | A screen recording with UI changes and small text | #173 (L-018, tiny text), #175 (L-028, change thresholds) |
| `noisy-speech` | Speech under real background noise or music | #150 (L-020) |
| `non-english-or-accented` | A non-English speaker, or a strong accent | L-022 |
| `leading-silence` | Silence or a title card before the first word | #174 (L-024, a segment that starts at the audio's start) |
| `lecture-or-meeting` | A lecture or meeting of 15 to 30 minutes | L-022 (long recordings), #380 to #384 at scale, timings |
| `vertical-variable-frame-rate` (optional) | A phone video: vertical, variable frame rate | #363's list; rotation and VFR paths on real files |

The set does not replace the synthetic corpus: the corpus has exact, frozen truth; the real set has realistic signal and
only coarse, hand-made labels (section 6).

## 3. Candidates for approval

**The set as it stands (2026-10-19)** is in section 14: the table below is the candidate list of 2026-10-18 that the
maintainer decided on, with RM06 now fetched from NASA's own upload to the Internet Archive and two items added there (RM09,
RM10).

Eight candidates, **426,605,905 bytes (about 427 MB) in all**, every one on Wikimedia Commons, researched on 2026-10-18 from
the file pages and the Commons API (size, SHA-1, upload time, licence, author, duration, codecs) and, for the one transcode,
from the server's response headers. **Nothing was downloaded, watched or heard:** what an item sounds like is what its page,
captions and metadata say, and is checked when an approved item is first labelled. Each is in the manifest as `proposed`,
with the full licence links, attribution and notes. The maintainer approves or declines each one; an approved item is then
pinned on a hosted runner.

| Id | Recording (source page) | Licence | Duration, video, audio | The download | What it tests |
| --- | --- | --- | --- | --- | --- |
| RM01 | [Editing basics - watchlists](https://commons.wikimedia.org/wiki/File:Editing_basics_-_watchlists.webm), Sage Ross (WMF), 2013 | CC BY-SA 3.0 | 2:16; VP8 1280x1024; Vorbis | `RM01-editing-basics-watchlists.webm`, the original from upload.wikimedia.org, **6,192,962 bytes**, SHA-1 `9cc8ceb7...` | A narrated screen recording: page changes, menus, small interface text (#173, #175); human captions whose last cue ends at `99:59:59` |
| RM02 (held) | [Structured Data on Commons uploads by the Metropolitan Museum of Art 01](https://commons.wikimedia.org/wiki/File:Structured_Data_on_Commons_uploads_by_the_Metropolitan_Museum_of_Art_01.webm), GFontenelle (WMF), workshop by the Met's collection-information manager, 2021 | CC BY-SA 3.0 | 12:54; VP8 1184x720 25 fps; Vorbis | `RM02-met-structured-data-demo.webm`, the original, **26,240,356 bytes**, SHA-1 `f0083f98...` | A live tool demonstration: speech with pauses while the presenter works (#353's shape, #383), screen changes and small text (#173, #175), about 31 chunks. **Check before approval:** other call participants may be visible in tiles |
| RM03 | [After the lifting of the block: Case study of Turkey](https://commons.wikimedia.org/wiki/File:After_the_lifting_of_the_block_Case_study_of_Turkey.webm), Wikimania 2021, Basak and Khutuck | CC BY-SA 3.0 | 19:16; VP9 1920x1080 30 fps; Opus | `RM03-wikimania-turkey-talk.webm`, the original, **146,468,542 bytes**, SHA-1 `a6deaabe...` (or its 480p transcode, 29,909,095 bytes, no published checksum) | The 15 to 30 minute talk: about 46 chunks of continuous speech (#380 to #384), Turkish-accented English (L-022), a music and title-card lead before the first words at about 13.4 s (#174; a non-speech lead rather than silence) |
| RM04 | [Como contribuir com o Structured Data on Commons](https://commons.wikimedia.org/wiki/File:Como_contribuir_com_o_Structured_Data_on_Commons.webm), Ederporto, 2019 | CC BY-SA 4.0 | 5:00; VP9 1280x720 24 fps; Opus | `RM04-como-contribuir-sdc-pt-br.webm`, the original, **10,636,205 bytes**, SHA-1 `5a2ab7b9...` | Brazilian Portuguese speech (L-022) with interface changes; Portuguese captions as a possible reference; an audio track wrongly tagged English |
| RM05 | [President Reagan's Trip to Fullerton and San Bernardino on November 1, 1988](https://commons.wikimedia.org/wiki/File:President_Reagan%27s_Trip_to_Fullerton_and_San_Bernardino_on_November_1,_1988.webm), Reagan Library (White House Television Office) | CC BY 3.0 (reviewed on Commons) | 15:53; VP8 654x480 29.97 fps; Vorbis | `RM05-reagan-fullerton-1988.webm`, the original, **112,179,359 bytes**, SHA-1 `dcd47fbd...` | Speech under real noise (#150, L-020): remarks at a rally in a gymnasium and at a helicopter landing zone through a field microphone; old analogue video |
| RM06 | [Space Station Astronaut Discusses Record Breaking Mission with Previous Record Holder](https://commons.wikimedia.org/wiki/File:SPACE_STATION_ASTRONAUT_DISCUSSES_RECORD_BREAKING_MISSION_WITH_PREVIOUS_RECORD_HOLDER.webm), NASA Johnson Space Center, 2023 | Public domain (US government work) | 10:31; VP9 1280x720 59.94 fps; Opus | `RM06-nasa-iss-record-conversation.webm`, the original, **84,997,974 bytes**, SHA-1 `a982fbda...` | A two-person conversation with turn-taking and the pauses of a space-to-ground link (#353's shape, #359, #383), about 25 chunks; a 59.94 fps source |
| RM07 | [Introduction to Wikidata Lexeme Forms - Lucas Werkmeister - Lexicodays 2024](https://commons.wikimedia.org/wiki/File:Introduction_to_Wikidata_Lexeme_Forms_-_Lucas_Werkmeister_-_Lexicodays_2024.webm) | CC BY-SA 4.0 | 9:39; the 480p VP9 transcode of a VP8 1920x1080 original | `RM07-lexeme-forms-480p.webm`, Commons' 480p transcode, **21,315,686 bytes**, no published checksum (ETag `78fc96f3...` taken as its MD5, unconfirmed) | German-accented English with a tool demonstration (L-022, #380, #381), about 23 chunks; **human-made English captions, the set's best reference transcript** |
| RM08 (declined) | [Mark Carney speaking about climate action and energy security in Calgary, 15 May 2026](https://commons.wikimedia.org/wiki/File:Mark_Carney_speaking_about_climate_action_and_energy_security_in_Calgary,_15_May_2026.webm), Jasonhargrove | CC BY 4.0 | 0:22; VP9 1080x1920 30 fps nominal; Opus | `RM08-calgary-vertical-phone.webm`, the original, **18,574,821 bytes**, SHA-1 `3f936fd5...` | A vertical phone recording of a speech in a hall (rotation, aspect, noise); re-encoded before upload, so its frame rate may no longer vary; a current politician, which the maintainer may prefer to leave out |

**Licences and share-alike.** Five items are CC BY-SA (3.0 or 4.0), two CC BY (3.0, 4.0), one public domain. Share-alike binds
only copies and adaptations of the media; the repository holds neither (section 5), so it constrains nothing in the repository.

**Every shape is covered** (`real-media.cjs check` says so), but **two shapes rest on RM03 alone**: the 15 to 30 minute
recording and the lead-in before speech. If RM03 is declined, the alternates are:

- [Introduction to QuickStatements (Wikidata Leveling Up Days 2024)](https://commons.wikimedia.org/wiki/File:Introduction_to_QuickStatements_(Wikidata_Leveling_Up_Days_2024).webm),
  CC BY-SA 4.0, 16:31, talk and demonstration with human captions: its 480p transcode is 46,526,246 bytes (the original is
  552,913,783, too large);
- [Space Station Crew Members Discuss Life in Space with Military Media](https://commons.wikimedia.org/wiki/File:Space_Station_Crew_Members_Discuss_Life_in_Space_with_Military_Media.webm),
  NASA, public domain, 17:48, a press interview with several speakers, 217,356,139 bytes;
- [The Catalan Challenge](https://commons.wikimedia.org/wiki/File:The_Catalan_Challenge.webm), Wikimania 2021, CC BY-SA 3.0,
  28:34, one speaker, 248,038,052 bytes, but marked "licence review needed" on Commons.

Other alternates considered: the [VisualEditor reference tutorial](https://commons.wikimedia.org/wiki/File:VisualEditor_reference_tutorial.ogv)
(CC BY-SA 3.0, 5:05, an Ogg Theora original of 90,941,788 bytes at 1920x1072, a different decoder path) for the screen
recording; [Expedition-53 Ellington Field Return](https://archive.org/details/Expedition-53-Ellington-Field-Return) (NASA, public
domain, Internet Archive, 3:41, 57,669,665 bytes, MD5 and SHA-1 published) for noise. **Declined in the research:** recorded
video calls whose participants show in tiles with their names (most Wikimedia office hours and community calls), anything with
minors or members of the public speaking about themselves, non-commercial lecture series, Internet Archive re-uploads by people
who do not hold the rights, and YouTube re-uploads whose licence review on Commons is still open (the Blender tutorials).

## 4. Where the media lives: a pinned manifest, never the files

**The media is not in the git repository** (size, and a licence that is not the repository's own). The repository holds
[`tools/p14-campaigns/real-media/manifest.json`](../../tools/p14-campaigns/real-media/manifest.json), validated by
[`manifest.schema.json`](../../tools/p14-campaigns/real-media/manifest.schema.json) and by the tool itself. For each item it
pins:

- the **source page** that states the licence, and the **file URL** at its source (`archive.org` and the Internet Archive's
  storage nodes `*.archive.org`, `images-assets.nasa.gov`, and `upload.wikimedia.org` as **provenance only**: Commons refuses
  the tool, so the tool never contacts it; a host is added only in the reviewed change that adds an item from it);
- the **mirror** (`mirror_url`): the item's verbatim copy in the project's own test-media release, which every fetch contacts
  when it is present (section 14);
- the exact **size in bytes**, the **SHA-256**, and the **checksum the source publishes** (the Commons API's SHA-1, the
  Internet Archive's MD5 or SHA-1), with the source's own **revision** (the Commons upload time) where it has one;
- the **licence** (an identifier from a closed list: `CC0-1.0`, `CC-BY-n.0`, `CC-BY-SA-n.0`, `public-domain`,
  `public-domain-us-government`), its text's URL and the URL where the source states it for this file;
- the **author and the attribution text**, the media's duration, container and codecs, the shapes it covers and the issues
  it should help settle;
- its **status**: `proposed` (listed for review, never fetched), `approved` (the maintainer approved it on a recorded date),
  `withdrawn` (kept for the record, never fetched, not counted). **The set is its approved items: at most eight, and
  500,000,000 bytes together** (the maintainer's limits of 2026-10-19, enforced by the tool);
- later, hand-made **labels** (section 6).

**The tool:** `node tools/p14-campaigns/real-media.cjs` (Node.js 22, no dependency), with its rules in
`tools/p14-campaigns/lib/real-media.cjs` and its tests in `tools/p14-campaigns/test/real-media.test.cjs`:

| Command | What it does |
| --- | --- |
| `check [<manifest>]` | Validates the manifest, lists the items and says which required shape no item covers |
| `attribution [<manifest>]` | Prints the attribution and licence of every item that is not withdrawn |
| `pin <dir> --item RMnn` | For an approved item with no SHA-256 yet: fetches it from its source (never the mirror) bounded by its size, verifies the source's checksum, keeps the file and prints the SHA-256 to write into the manifest in a reviewed change |
| `pin <dir> --item RMnn --from-file <file>` | The same from a copy downloaded by hand (for a source that refuses the tool): checks the exact size and the source's published checksum (an item without one is refused), copies the file into place with its `ATTRIBUTION.txt` and leaves the copy given as it is |
| `fetch <dir> [--item RMnn]...` | Fetches approved, pinned items into `<dir>/<id>/<file_name>`, from the mirror when there is one |
| `cross-check <dir> [--item RMnn]...` | The hosted form: every approved item fetched and held to its pin (an unpinned one pinned there), each SHA-256 printed |
| `release-files <media dir> <out dir>` | Prepares the test-media release for review: every approved item present with its pinned SHA-256, `ATTRIBUTION.txt` and `NOTES.md` written, the `gh release create` command printed, never run |

**The download rules of `AGENTS.md`, and how the tool meets them:**

- **HTTPS only**, on the allowed hosts, with no credentials or port in the URL; every redirect is checked the same way, at
  most five; on github.com only the test-media release's assets. The request names the tool only (`User-Agent:
  vsift-real-media-fetch/1`): never a person, an address or an account, even where a source asks for contact details (the
  maintainer's decision of 2026-10-19). A source that refuses it is never retried with another user agent or through a
  browser; the maintainer downloads such a file by hand. A refusal is shown with its reason and never with the client address
  an error page echoes back.
- **Pinned provenance:** the URL, size, SHA-256 and the source's own checksum are in a reviewed file; an item is fetched
  only once the maintainer has approved it and its SHA-256 is pinned.
- **Integrity:** the SHA-256 and the source's checksum are computed while the file streams in, and the file is placed only
  when both match. **The first pin** is anchored to the checksum the source publishes (SHA-1 or MD5, weak against a
  deliberate collision but recorded by the source before we fetched anything), and is made on a hosted runner; every later
  fetch is checked against the SHA-256. **A Commons transcode has no published checksum:** its object store's ETag is
  recorded as the MD5 to compare (if it is not the file's MD5 the pin fails closed and the item is reviewed again), and
  beyond that its pin is trust on first use over HTTPS. A transcode can also be regenerated by Commons, which changes its
  bytes; the fetch then fails closed and the item is pinned again in a reviewed change. Originals are preferred for this
  reason; a transcode is proposed only where the original is too large.
- **Bounded size:** an announced length other than the pinned size is refused before anything is written; a transfer is cut
  off as soon as it passes the pinned size; a short one is refused. No item may exceed 300 MiB, the items that are not
  withdrawn 750 MiB together, and the approved ones 500 MB.
- **Safe placement:** the destination must be an absolute path to an existing folder **outside the repository** (so a fetched
  file can never be committed by accident). Each file is written exclusively to `<dir>/<id>/.<file_name>.partial` and renamed
  into place only after verification. A file already in place is checked, never overwritten: with the pinned digest it is
  kept (`present`), with another it is refused and left alone. The tool deletes nothing but its own partial file of the
  same run, and never touches anything else in the folder. There is no archive to extract: every item is a single media file.
- **Source media is never changed.** The campaign gives VSift the fetched file as it is; VSift's own session copies it.

**Where the files go.** Locally, a folder the person running the campaign names, outside the repository (for example
`%LOCALAPPDATA%\vsift-real-media` or `~/.cache/vsift-real-media`); on a hosted runner, the runner's temporary folder, gone with
the runner. **The media is never uploaded as a workflow artifact**; only the campaign's report is, with at most twelve native
crops of public, openly licensed screen recordings for a person to judge small text (decision 10).

## 5. Attribution

- Each item's attribution text, author, licence and source are in the manifest; `real-media.cjs attribution` prints them, and
  `fetch` writes them to `ATTRIBUTION.txt` beside each file, so they travel with the media wherever the folder is copied.
- **Every report that names an item** (a campaign report, an issue, a qualification record) names it by its id and carries
  its attribution line; the campaign report has an attribution section built from the manifest.
- **Share-alike** (CC BY-SA) constrains only the media and adaptations of it: the repository holds no copy and no adaptation,
  only URLs, digests and our own measurements, so its own MIT OR Apache-2.0 licence is unaffected. A clip cut from an item for
  a test, or a frame quoted in an issue, is an adaptation or a copy and keeps the item's licence and attribution. **A
  regression fixture made from a finding never copies the media** (it is provider output of the failing shape, written by
  hand), so it carries the repository's licence.

## 6. Labels: what is truth here

The set has no frozen truth like the corpus. What it has, reviewed like truth and **never derived by running VSift on the
item**:

- `speech_start_us`, where the first word starts, by listening (for #174);
- `changes_us`, the moments a person sees the screen change, by watching (for #175);
- `reference_transcript`, a human transcript or subtitle file the source publishes (Commons timed text), where one exists,
  for coarse word-recall figures;
- the shapes the item covers.

Labels are added after an item is approved, by a person, in a reviewed change. Until then the campaign checks only
properties that need no labels.

## 7. The opt-in campaign

Built in step 2 (2026-10-18):

```console
node tools/p14-campaigns/real-media-campaign.cjs --vsift <exe> --media <dir> --work <dir> \
  --ffmpeg <exe> --ffprobe <exe> --whisper <exe> --model <file> [--item RMnn]...
```

Every path is absolute; `--media` is the folder `real-media.cjs` fetched into, and each file is checked against its pin
before it is used. Like the other campaigns it drives the binary it is given, never one on `PATH`, with per-user folders and a
session root per item under a new `run-<time>` folder of `--work` (it deletes nothing), and it is never part of an ordinary
pull-request run. It writes `report.json` and `summary.md` there and exits 1 when an item fails a property check. The
property checks are pure functions in `lib/real-media-checks.cjs`, tested on hand-made answers
(`test/real-media-checks.test.cjs`). The length of `--work` no longer matters on Windows for a `vsift` built from `main` at
`d82d118d` or later (#469, fixed by #478; section 12). Per item:

| Step | Commands | What it records |
| --- | --- | --- |
| Setup | `setup check` | The tools and the model, by identity; a `blocked` or `[timeout]` on a working setup is a finding (#391's shape) |
| Ingest | `ingest <file>` | Seconds; the video's duration, streams and frame rate as VSift reads them |
| Transcription | `transcript retranscribe <session>` (the whole video, with the model given; `base` in the hosted run) | Seconds and real-time factor, chunks, silent and no-audio chunks, the revision's warnings with their counts (`provider_segments_rejected`, `provider_end_trimmed`, `seam_duplicates_removed`), `local_asr.unplaced_ranges` |
| Seam shift | `transcript retranscribe <session> --from <shift> --to <end>` | A second revision whose chunk plan starts at `<shift>`, so each seam of one lies inside a chunk of the other. A range run is widened to the segments of the revision it replaces, so `<shift>` is the start of a segment of the whole run: the earliest within the first ten minutes whose offset in the 25 s chunk step is at least 7.5 s from a step boundary (else the one nearest the middle of a step). A fixed 15 s was widened back to 0 on RM10, whose first segment runs to 17.9 s, and the two runs came out identical; the first boundary of RM09 (25 s, after a silent lead) is a step boundary itself, and the shifted run again kept the plan. Each segment's `alignment` names its chunk and that chunk's audio window, so both plans are read from the answers; when the shifted plan shares most seams with the whole one, the seam check is reported as not run |
| Reads | `transcript get` (pages), `search` for words of the transcript at seams and away from them | Coverage, the hits, the seconds |
| Visual | `candidates` over the whole video in ranges of at most five minutes, `--omit periodic` as the skill does (run first, so its index gives the duration) | Candidates per minute and by reason, the calls and seconds |
| Evidence | `frame get` at three candidates, `crop` of the top-left 480x120 of a frame (the labelled small-text region once labels exist), `audio` around the worst seam loss (or the first seam) | The frames' requested and actual times; the crop's size; the clip, copied to `<item>/listen/` for a listener and never uploaded |
| Handoff | `handoff check` of a mechanical draft citing the run's own ids | `data.valid` |
| Close | `session close`, then `session clean --expired` | That the session closes and its folder, with VSift's copy of the recording, is removed |
| `vsift <video>` (ADR 0037, from 2026-10-28; public items only) | the bare form on a copy of the item in `<item>/watch/`, with a session root of its own | The wall time and real-time factor, the outcome, the screen changes and frames written. Check `watch`: the three outputs are beside the copy and are the bytes the answer hashed, the transcript's header names the item's pinned SHA-256, the frames are min(changes, 200) and each is the bytes its SHA-256 says and is in the list, and the session was closed. A private item is not watched: its files would hold the recording's words in the work folder |

**What it checks are properties, not exact words** (an item has no frozen transcript):

1. **No failure on valid speech:** no `MISSING_CAPABILITY`, `INTEGRITY_FAILURE`, `INVALID_SOURCE` or `INTERNAL` on any
   step, and every other failure is listed; `setup check` is `ready` with every tool `available` and local ASR `verified`; a
   transcription of an item with speech commits a revision with segments.
2. **Coverage is stated truthfully:** transcribed and untranscribed ranges are disjoint and together are the range read;
   `no_speech_ranges` lie inside the transcribed ones; `transcript get` and `search` are `partial` exactly when a gap is
   listed. (Checking `no_speech_ranges` against the decoded audio's level is not built.)
3. **No seam loss:** at each seam of one revision (the stretch two chunks both heard, plus three seconds each side), the
   words the other revision heard mid-chunk are looked for nearby, a word counting as found when a word triple it belongs to
   is there. The same comparison at control stretches inside one chunk measures how much two cuts of the same audio disagree
   anyway. **A finding:** seam recall more than 0.10 below the control's, or a run of eight or more words missing at one
   seam. Shorter runs are listed with their times for a listener. Both revisions are tested, each against the other.
   **Each missing run is classified** (2026-10-19, after the first real runs): `repetition` when the other revision's run
   repeats itself (few distinct word triples: a recogniser's loop on noise or silence, as RM09's arrival had), `script` when
   the two revisions heard the stretch in different writing systems (below), else `loss`. Only a `loss` of eight words or
   more is a finding ("listen"), and the recall gap is judged with the explained runs counted as found
   (`seam_recall_merge`; the raw `seam_recall` is kept).
3a. **Language flips** (a check of its own): the stretches the two chunk plans heard in different writing systems. Whisper
   detects the language per chunk, so on a recording with two languages (RM09: an English briefing with a Russian
   cosmonaut) the same English speech can come out as Russian text in a chunk cut one way and English in a chunk cut the
   other. A flip is a finding for a person to judge; it is not a merge fault.
4. **Repeats at seams are counted** (#380): word triples that stand twice within four seconds at a seam, the copies from
   different chunks and segments, per item and per hour.
5. **Timestamp differences between chunks** (#381): segments of the two fresh runs with the same text (three words or more,
   unique) are paired, and the differences of their starts and ends summarised (median, 95th percentile, maximum, how many
   exceed half a second). It is a measurement, not a check. A measurement of the raw chunk outputs before the merge would
   need an engine-level `--ignored` test; it is not built, because the segments' own alignment already names the chunk.
   **Seam joins** (`seam_joins`, since #381's measurement): in each revision, the gap between a kept segment of one chunk and
   the next kept segment, of the following chunk, at a seam, binned (overlap, touching, up to 100 ms, 250 ms, 500 ms, 1 s,
   longer), against the same gaps between consecutive segments of one chunk away from its seams (the control: ordinary
   pauses). The seam guard leaves a stretch of what the chunks heard uncovered only when it is no longer than the tolerance,
   so a hole it left is a seam gap of up to half a second; `within_tolerance_share` is the share of joins with a gap above
   zero and up to half a second, and `within_tolerance_at_us` where each such gap starts, for a listener. A measurement, not a
   check.
6. **Rejected segments** (#383): the kinds and counts of `provider_segments_rejected`, and where `unplaced_ranges` lie
   against the labelled speech.
7. **Candidates at visible changes** (#175): with `changes_us` labelled, the share of labelled changes a candidate's
   `change_window` brackets (half a second of slack each side for the labeller) or a motion candidate's span holds; the
   share with a candidate within one second is kept beside its chance figure (the share of the recording within a second
   of some candidate: seven in eight of RM10's still moments, so the loose figure alone says little); and the candidates
   per minute of a still screen (false changes). **Without labels**, the measurement `mode: visual` (section 8) compares
   the change rule with what changed in the native frames ([`visual-thresholds-real-media.md`](visual-thresholds-real-media.md)).
8. **Speech start** (#174): with `speech_start_us` labelled, the first segment's start minus it. **Without labels**
   (`chunk_start_segments`, since #174's measurement): in each revision, the segments that start at their chunk's first sample
   (the recogniser's own start within 20 ms of it), the run's first chunk counted apart, their lengths, how many are longer than
   10 s, and, where the other chunk plan wrote the same words (three or more, once each) in a segment that does not start at its
   chunk's start, how much later it started them: the time such a segment claims before its words, as far as two cuts can tell.
9. **Small text** (#173): the crop of the labelled small-text region is at native size and legible to a person; recorded by
   hand, as a yes or no per crop. **Measured mechanically since 2026-10-19:** for an item that covers
   `screen-ui-small-text`, each of the three frames is cut into a grid of native crops of at most 480x120. Each crop's
   PNG is read back and checked to be at native size, and the height of every line of text in it is measured
   (`lib/small-text.cjs`; `measurements.small_text`). The report gives the line heights; legibility stays a person's
   judgement ([`small-text-real-media.md`](small-text-real-media.md)), on the crops the run keeps for it: at most 4 an
   item with the smallest text and 12 a run, of public, openly licensed items only, never a private item's
   (`kept_crops`, decision 10).
10. **Timings:** seconds and real-time factor per step and item, against the budgets of the
    [qualification profiles](support-and-resource-profiles.md); a run several times slower than the last one is a finding.

The report is JSON plus a short Markdown summary, with the binary's `--version` and SHA-256 (never its path), the tools' and
model's identities, the machine's class, the items' ids and digests, and the attribution section. It holds measurements and
the run's own words only where a check needs them (a lost word sequence), never a whole transcript. **Since #486** it also
names the machine that recognised (`runner`: the CPU model, the logical CPU count and which of the CPU flags that decide
whisper.cpp's kernels it has, read on that machine) and, for each revision, the recogniser threads VSift gave whisper.cpp
(`threads`) and, for a public item only, `segment_fingerprints`: each segment's start, end, chunk and the first 16 hex digits of
the SHA-256 of its text, so two runs can be compared segment by segment from their reports without any text leaving the run.

## 8. How it runs

- **Locally, on demand:** a developer fetches the approved items once into a folder outside the repository and runs the
  campaign against a build of their worktree. It needs FFmpeg, FFprobe, whisper-cli and a pinned model, as the other real-tool
  checkpoints do. **Nothing is downloaded on a workstation without its owner's word:** fetching is a download, so the person
  running it decides, item by item.
- **Hosted, by manual dispatch only:** the workflow `P14 real media` ([`p14-real-media.yml`](../../.github/workflows/p14-real-media.yml)),
  **on Ubuntu 24.04 only** (decided 2026-10-18): `gh workflow run p14-real-media.yml --ref <branch> [-f items="RM01 RM04"]`.
  With `-f mode=visual` it runs, instead of the campaign, the measurement of the visual change rule on the fetched items
  (#175: `crates/vsift-infrastructure/tests/p08_real_media_visual.rs`, which decodes each item twice and keeps numbers only,
  as `visual-measurement.json` in the report artifact); `-f mode=both` runs the two.
  With `-f keep_segments="RM09:915-940"` (#485; whole seconds, at most four ranges and 120 s an item) the campaign also keeps,
  as text, the transcript segments of both revisions that overlap each range, with their chunk, its audio window and the
  recogniser's own times, in `kept-segments/<item>/segments.json` of the artifact beside the item's attribution, so a person can
  read a seam finding without a local run. **Only for a public, openly licensed item**, by the rule that keeps crops: never a
  private item's (the campaign refuses `--keep-segments` with `--private-manifest`, and a test drives a private item and one
  without an open licence and finds nothing written).
  It fetches the approved items with `real-media.cjs cross-check` into the runner's temporary folder **from the test-media
  release**, never from Wikimedia (a pinned item must match
  its pin; one not yet pinned is pinned there and its SHA-256 printed, which is the hosted cross-check of a pin made
  elsewhere), stages the pinned FFmpeg, whisper.cpp and models as `P07 local ASR` does
  ([`p07_local_asr_tools.py`](../../tools/p07_local_asr_tools.py)), builds the commit's `vsift`, runs the campaign and keeps
  only `report.json`, `summary.md`, the pin lines, the kept crops (decision 10) and any segments asked for (#485) (90 days). It has `contents: read` and no secret. A pull request that
  changes the campaign's tools runs their tests and `real-media.cjs check` only. It is not scheduled: the items are long
  enough that a run costs real runner time (the 15 to 30 minute lecture alone transcribes in roughly 6 to 12 minutes at the
  `base` model's measured real-time factor), so it runs when a change touches transcription, the merge or the visual index,
  for a re-pin, and once per release window when such a change merged ("run costly tests sparingly").
- **Evidence: advisory** (decided 2026-10-18). A run is evidence for `0.2.3-rc.1`, but no release gate reads it: the job fails
  when an item fails a property check, and a finding becomes an issue after a person has listened or looked. Whether it gates
  from `0.2.4` is decided once it has proved stable.

## 9. How results feed the follow-ups

- **Seams (#380 to #384):** checks 3 to 6 give the measurements each issue asks for "on real recordings first"; each issue's
  next step is decided on them, and the register entries L-155 to L-159 are rewritten with the numbers.
- **#175:** the first real figures, without labels, are in [`visual-thresholds-real-media.md`](visual-thresholds-real-media.md)
  (RM10, 2026-10-19: no false change; the recommendation is to keep the thresholds).
- **#173, #174, #175:** checks 7 to 9, once the labels exist, give the first real figures; a threshold or profile change is a
  new visual profile version (#175) with its own review.
- **#150 and L-020, L-022:** with a reference transcript, a coarse word-recall figure per item (noisy, accented, non-English).
  It is not a gate: one or two items are not a sample, and a gate stays for #150's fixture set.
- **A defect found** is an issue first, then one re-run; its fix leaves a regression fixture that does not need the recording.

## 10. The maintainer's decisions (2026-10-18 and 2026-10-19, recorded on #363)

1. **Approved:** RM01, RM03 (the original, not the 480p transcode), RM04, RM05, RM06 and RM07, about 382 MB; each may be
   fetched by the hosted workflow and placed on the maintainer's machine, outside every repository, for local runs.
2. **RM02 is held** until its call tiles are checked for visible participants: it stays `proposed` and is never fetched until
   then.
3. **RM08 is declined** (a current politician; the vertical shape is optional): it is `withdrawn`, never fetched, its id never
   reused. The vertical phone shape stays uncovered.
4. **The campaign is advisory** evidence for `0.2.3-rc.1`; gating is decided once it has proved stable.
5. **The hosted workflow runs on Ubuntu only.**
6. **2026-10-19: no contact in the user agent.** It stays `vsift-real-media-fetch/1`. Sources that accept it come first (the
   Internet Archive, NASA, government and library archives), with the go to fetch public, freely licensed items of the design's
   shapes that show no private person's data, at most eight items and 500 MB in the set; a source that refuses the neutral
   user agent is never retried with another one or through a browser.
7. **2026-10-19: hand downloads** for an approved Wikimedia item with no equivalent elsewhere: the maintainer downloads it in a
   browser, and `pin --from-file` checks the exact size and the Commons SHA-1 before it places the file.
8. **2026-10-19: a VSift test-media release** of verbatim copies with their attribution, from which every hosted run fetches.
   The maintainer consented; the coordinator reviews and runs the `gh release create` command (section 14), never a tool.
9. **2026-10-19: a private local set** of two of the maintainer's own recordings, for local runs only (section 13).
10. **2026-10-19 (recorded on #173): kept crops.** The hosted run may keep a handful of native crops of public, openly
    licensed items for a person to judge legibility; a private item's crops are never kept. Built as at most 4 crops an
    item and 12 a run, the crops with the smallest text, with the item's attribution
    ([`small-text-real-media.md`](small-text-real-media.md) section 4).

## 11. What the set must never contain

- **The maintainer's own recordings**, or any recording the project made of a person, its trials included.
- **Anything with personal data visible or audible:** a private person's name, address, e-mail, phone number, account page,
  inbox, documents or face in close-up; a meeting of private persons; a public meeting where members of the public speak about
  themselves. A public speaker in their public role, or an organisation's staff presenting its work, is acceptable.
- **Anything whose licence is not one of the four kinds** (public domain, CC0, CC BY, CC BY-SA), is unclear, or is a re-upload
  by someone who does not hold the rights; non-commercial and no-derivatives licences are excluded.
- **Media in the repository:** not as a file, not through Git LFS, not as a release asset. The repository holds the manifest,
  the tool, the reports' measurements and hand-written regression fixtures only.
- **Instructions:** speech, on-screen text and titles in an item are untrusted input like any video's, and are never followed.

If an item is later found to break one of these rules, it is `withdrawn` in a reviewed change and every fetched copy deleted
by its holder; its id is never reused.

## 12. Step 2: what happened (2026-10-18)

**The approvals are in the manifest:** RM01 and RM03 to RM07 are `approved` with the maintainer's approval date, RM02 is
`proposed` with the reason it is held, RM08 is `withdrawn` with the reason it was declined. `real-media.cjs check` reports
every required shape covered by the items that are not withdrawn.

**The pins: blocked by Wikimedia's client rules, nothing fetched.** The maintainer's go covered the six approved files from
upload.wikimedia.org, into `C:\dev\vsift-real-media`, through the tool only. Every request was refused before any media
byte was sent:

- on this machine, `real-media.cjs pin <dir> --item RM01`, five times within a quarter of an hour: HTTP 429, "Too many requests.
  Please contact" Wikimedia's bot-traffic address; a header-only request with the same user agent was answered 200 with the
  file's size, so the refusal is of downloads by an unidentified client, not of the host;
- on a hosted runner, `P14 real media` run 39172122182 (`cross-check`, all six items): the same 429 for every item.

Wikimedia's user-agent policy asks automated clients for contact information in the user agent. The tool sends
`vsift-real-media-fetch/1` and nothing else, by the maintainer's rule that no personal detail goes to an external service
without consent, and a courtesy convention is not consent. **The maintainer decides what the user agent may say** (an alias
address, or a project page as the contact); then the six items are pinned on a hosted runner, each pin is cross-checked by a
second, independent fetch, and RM07's ETag is compared with its MD5. No other source was tried, as the go required. The tool
now shows a refusal's own reason on one line with any address the page echoes back about the client removed (Wikimedia's
error page prints the client's IP address), so a public log never repeats it.

**The runner works, shown on a synthetic input only.** A 3:23 recording built on this machine from the corpus's own speech
clips (the twelve P07 utterances three times, over FFmpeg's test pattern; never committed) ran through every step with the
release build of this branch and the pinned whisper.cpp 1.9.2 `base` model: setup ready and verified, transcription in 201 s
(real-time factor 0.99 on a busy machine), the shifted run in 165 s, seam recall 0.94 and 0.95 against a control of 1.00, no
lost run of three words or more, no repeat at a seam, timing differences between the two plans of 260 ms median and 640 ms at
most (one of six pairs beyond half a second), 1.5 candidates a minute, three frames at their candidates' times, a native
480x120 crop, a valid handoff, the session closed. It says nothing about real speech.

**A defect found on the way (Windows), now #469 (L-187):** with the per-user folder (`LOCALAPPDATA`) longer than 141
characters, every media command fails `MISSING_CAPABILITY` with the remediation "The selected FFmpeg and FFprobe failed
VSift's media-tool check at the probe step (process_failure) ... Reinstall FFmpeg and FFprobe", although `setup check` calls
both available and nothing is wrong with them; at 141 characters it passes, at 143 it fails. With a session root of 205
characters or more, `ingest` succeeds and then `candidates` and `frame get` fail `MISSING_CAPABILITY` with no remediation at
all. Both look like paths beyond Windows' 260-character limit handed to FFmpeg (the media-tool check copies its fixture deep
inside the per-user folder; a session's copy lies deep inside its root). Measured with the release build of this branch
(`vsift 0.2.2`, from `main` at `79fdc502`) and the gyan.dev FFmpeg 9.0 build, on Windows 11. It was found because the
campaign's first working folder was deep inside a temporary folder. **Fixed by #478 (`d82d118d`, 2026-10-19):** the cause
was the providers' working directory (Windows starts no process whose working directory is longer than 258 characters), and
every provider now runs in its own executable's folder, so a long per-user folder or session root no longer fails media
commands; the advice to keep the campaign's `--work` short is withdrawn. `0.2.2` and earlier still have the defect.

## 13. The private local set (decided 2026-10-19)

The maintainer has consented to local use, and only local use, of two private recordings that are good examples of real
work (a conversation and a paid tutorial). They are not part of the set and never enter the repository, a workflow, a
release or a report's text.

- **Where they are listed:** a private manifest kept **outside every repository**, beside the files, named with
  `--private-manifest <absolute path>`. It has the same envelope as the committed manifest with `set_id`
  `vsift-real-media-private` and private items only: `id` (`P01`, `P02`, ...), `status: "approved"` with the maintainer's
  `approval` (the consent's date), `private: true`, `file_name` (a plain name beside the manifest), `bytes`, `sha256` (or
  null), `covers`, and optionally `labels`, `notes` and `tests`. No title, source, licence or attribution is recorded.
  `lib/real-media.cjs` validates it, refuses one inside the repository, reports its problems by member and never with a
  value, and refuses a private item in the committed manifest.
- **How they run:** `node tools/p14-campaigns/real-media-campaign.cjs --private-manifest <file> --vsift <exe> --work <dir>
  --ffmpeg ... --ffprobe ... --whisper ... --model ...`, on a local machine only: the campaign refuses a private manifest
  when `CI` or `GITHUB_ACTIONS` is set, and no workflow names one (a test reads every workflow).
- **What their reports hold:** measurements only (coverage, losses and recall at chunk joins, repeats as counts and times,
  timing differences, timings, candidate counts) and failure codes. **No transcript text, no names, no file names** and no
  revision ids: items appear as `P01` and `P02`. The step labels, which are also the log lines, are fixed phrases (a
  search is `search 1`, never its words), a failed check keeps only its name and the number of its findings, and the
  report passes through a filter that drops every string other than VSift's own codes and enumerations (a missing run's
  kind is kept only as `loss`, `repetition` or `script`, since 2026-10-20). No listening clip
  of a private item's audio and no transcript file is kept; its session, which holds the copy VSift makes, is closed and then
  removed (`session clean --expired`) at the end of the run, so no copy of the recording stays in the work folder (checked
  on a synthetic stand-in run through the private path on 2026-10-19).
- **Tests:** `test/real-media-campaign.test.cjs` drives the steps with a stand-in `vsift` whose transcript, file name
  and names are made-up private words, and shows that none of them reaches the private report, the summary or a log line,
  that no clip is copied, and that the same run of a public item would carry them (so the test can see a leak);
  `test/real-media.test.cjs` covers the private manifest's rules and the refusal in CI.
- **Reports to the coordinator** about these items give measurements and failure codes only; their content is never
  printed, quoted or summarised.

## 14. The set as it stands, the sources and the test-media release (2026-10-19)

**Sources tested with the neutral user agent** (`vsift-real-media-research/1` for metadata, `vsift-real-media-fetch/1` for
files): archive.org and its storage nodes, and images-assets.nasa.gov, answered; **upload.wikimedia.org refuses** downloads
(section 12) and is now a provenance-only host the tool never contacts; loc.gov's API answered HTTP 403 and was not used
further; svs.gsfc.nasa.gov was unreachable. None of the four Wikimedia talks and tutorials (RM01, RM03, RM04, RM07) is on an
allowed source; the Reagan footage (RM05) is in the National Archives' catalog as one 378,799,456-byte file with no checksum,
three quarters of the set's budget, so it is not used.

| Id | Recording | Licence | Where it comes from | Bytes | Pinned SHA-256 |
| --- | --- | --- | --- | ---: | --- |
| RM01 | Editing basics - watchlists (Sage Ross, WMF) | CC BY-SA 3.0 | Commons, downloaded by hand | 6,192,962 | `dfbf5ca9...` |
| RM03 | After the lifting of the block: Case study of Turkey (Wikimania 2021) | CC BY-SA 3.0 | Commons, by hand | 146,468,542 | `80fccac2...` |
| RM04 | Como contribuir com o Structured Data on Commons (Ederporto) | CC BY-SA 4.0 | Commons, by hand | 10,636,205 | `40361e5a...` |
| RM05 | President Reagan's Trip to Fullerton and San Bernardino, 1 November 1988 (Reagan Library) | CC BY 3.0 | Commons, by hand | 112,179,359 | `4aa842a5...` |
| RM06 | Space Station Astronaut Discusses Record Breaking Mission (NASA Johnson) | Public domain | [Internet Archive](https://archive.org/details/jsc2023m000168_NTV_VF_Astronauts_Discuss_Record_Breaking_Mission_230912-MP4.mp4), NASA Johnson's own upload; SHA-1 checked | 60,378,293 | `a48781d5...` |
| RM07 | Introduction to Wikidata Lexeme Forms, Lexicodays 2024 (480p transcode) | CC BY-SA 4.0 | Commons, by hand (ETag compared as MD5) | 21,315,686 | `afd19162...` |
| RM09 | NASA's SpaceX Crew-8 Crew Arrival at Kennedy Space Center | Public domain | [NASA Image and Video Library](https://images.nasa.gov/details/KSC-20240225-MH-JBS01-0001-CCP_SpaceX_Crew_8_Arrival_KSC-M4585) (no checksum published: trust on first use, cross-checked hosted) | 72,873,726 | `ee5501f1...` |
| RM10 | How to Create a Map from a Table Using data.census.gov (U.S. Census Bureau) | Public domain | [Internet Archive](https://archive.org/details/yt_wJJiJfJpK0o) mirror of the Bureau's channel; SHA-1 checked | 10,796,931 | `a035daeb...` |

RM09 adds about five minutes of arrival with no speech before the first word (leading silence; the noise is to be heard),
outdoor remarks (noisy speech) and 28 minutes of length; RM10 is a real screen recording of a data site with small text and
burned-in captions. Both were looked at frame by frame for private persons' data before use (none). RM02 (held) and RM08
(withdrawn) are not in the set.

**Hand downloads for the maintainer** (Commons serves a browser; the tool never contacts it). Save each file under any name,
then pin it, which checks the exact size and the Commons SHA-1 and copies it into `C:\dev\vsift-real-media\<id>\`:

```console
node tools/p14-campaigns/real-media.cjs pin C:\dev\vsift-real-media --item RM01 --from-file <the downloaded file>
```

| Id | Direct URL | Bytes | Source checksum |
| --- | --- | ---: | --- |
| RM01 | https://upload.wikimedia.org/wikipedia/commons/e/e6/Editing_basics_-_watchlists.webm | 6,192,962 | SHA-1 `9cc8ceb7e9cd6b13f78ebf0936b1cb8fffebbdf6` |
| RM03 | https://upload.wikimedia.org/wikipedia/commons/b/b0/After_the_lifting_of_the_block_Case_study_of_Turkey.webm | 146,468,542 | SHA-1 `a6deaabe0f173611af5e1082ddd4637259027435` |
| RM04 | https://upload.wikimedia.org/wikipedia/commons/d/d7/Como_contribuir_com_o_Structured_Data_on_Commons.webm | 10,636,205 | SHA-1 `5a2ab7b99ed3d7281ff3f1b44d3ee10514daa49a` |
| RM05 | https://upload.wikimedia.org/wikipedia/commons/7/7c/President_Reagan%27s_Trip_to_Fullerton_and_San_Bernardino_on_November_1%2C_1988.webm | 112,179,359 | SHA-1 `dcd47fbd8dca707828a04e3429cf2d3e4ae044d1` |
| RM07 | https://upload.wikimedia.org/wikipedia/commons/transcoded/e/ec/Introduction_to_Wikidata_Lexeme_Forms_-_Lucas_Werkmeister_-_Lexicodays_2024.webm/Introduction_to_Wikidata_Lexeme_Forms_-_Lucas_Werkmeister_-_Lexicodays_2024.webm.480p.vp9.webm | 21,315,686 | MD5 `78fc96f30f6e04762296e0d458d6402c` (the object store's ETag, unconfirmed: if it is not the MD5 the pin refuses the copy, and RM07 needs a decision) |

**The test-media release.** Every approved item has a `mirror_url` in the release `test-media-2026-10-19` of this repository;
`url` and `source_checksum` stay as the provenance, and every fetch contacts the mirror. The tag lies outside the `release
tags` ruleset (`refs/tags/v*`), no workflow runs on a tag push or a release event (`release.yml` is dispatched by hand and
publishes only on `v<version>`), and the release is a pre-release that is never "latest", so nothing that reads VSift's
releases sees it. `node tools/p14-campaigns/real-media.cjs release-files C:\dev\vsift-real-media <an empty output
folder>` checks that every approved file is present with its pinned SHA-256, writes `ATTRIBUTION.txt` and `NOTES.md`, and
prints the command for the coordinator to review and run.

**A partial release first (decided 2026-10-19):** the release starts with the three items already pinned (RM06, RM09,
RM10) and takes the five hand-downloaded ones later. `release-files ... --partial` takes the approved items that are pinned
and present, names the rest in `NOTES.md` and `ATTRIBUTION.txt` as still to come, and prints both the `gh release create`
command for the three and the `gh release upload --clobber` command for the five (with `ATTRIBUTION.txt`, rewritten by a
second `release-files` run without `--partial`), followed by `gh release edit --notes-file` for the full notes. A file that
is present but differs from its pin is refused in either mode. The hosted run works on whatever the release holds: an item
it does not hold yet is reported **not yet mirrored**, which is not a failure (`cross-check` takes the others, and the
campaign runs with `--available-only`); a run in which no item could be investigated fails. An item whose source refuses
the tool and that is not yet pinned in the manifest is pinned from the release against the checksum its source publishes
(the Commons SHA-1, RM07's ETag as MD5), so a hand-downloaded copy uploaded to the release is held to Commons all the same.
The full release, once all eight are there:

```console
gh release create test-media-2026-10-19 --repo smormah/vsift --target main --prerelease --latest=false
  --title "VSift test media 2026-10-19 (not a VSift release)" --notes-file "<out>\NOTES.md"
  "C:\dev\vsift-real-media\RM01\RM01-editing-basics-watchlists.webm" ... (the eight files) "<out>\ATTRIBUTION.txt"
```

The assets are the eight verbatim files (440,841,704 bytes) and `ATTRIBUTION.txt`; the partial release's are RM06,
RM09 and RM10 (144,048,950 bytes) and `ATTRIBUTION.txt`. Until the release exists, a hosted run reports every item as not
yet mirrored and fails for want of anything to investigate.

**The hand pins (2026-10-20).** The maintainer downloaded RM01, RM03, RM04, RM05 and RM07 in a browser, and the
coordinator pinned each with `pin --from-file`: every size and Commons SHA-1 matched, and RM07's ETag matched as its MD5 (so
the ETag is confirmed as the transcode's MD5). Their SHA-256 values are in the manifest, each checked again against the
copies in `C:\dev\vsift-real-media` before it was written, with that provenance in the item's notes. `release-files` now
accepts all eight; the coordinator uploaded the five to the release on 2026-10-20 with the renewed `ATTRIBUTION.txt`
and notes.

### 14.1 The first real runs (2026-10-19, local)

RM06, RM09 and RM10 ran on this machine (Windows 11, 40 logical CPUs, shared with other work) with the release build of this
branch (`vsift 0.2.2` from `main` at `79fdc502`, SHA-256 `5d9a7010...`), the gyan.dev FFmpeg 9.0 build and whisper.cpp 1.9.2
with the `base` model. **No failure, a truthful coverage statement, every search found its segment, every frame at its
candidate's time, a native crop and a valid handoff on all three.** RM06 and RM10 passed every check; RM09 failed the seam
and language checks (findings below). The other five items wait for the hand downloads.

| | RM10 (Census screen recording, 3:21) | RM06 (NASA conversation, 10:24) | RM09 (NASA arrival, 28:02) |
| --- | --- | --- | --- |
| Transcription, real-time factor | 0.43 to 1.65 (busy machine) | 0.48 | 0.55 |
| Chunks, segments | 8, 41 | 25, 167 | 68 (7 silent), 351 |
| Shift point, achieved | 34.2 to 35.96 s, yes | 37.66 s, yes | 334.2 s, yes (25 s, the first boundary after the silent lead, was a step boundary and kept the plan) |
| Seam recall / control (whole run; shifted run) | 0.989 / 0.977; 0.959 / 0.992 | 0.967 / 0.961; 0.981 / 0.977 | 0.940 (0.979 explained) / 0.927; 0.942 (0.947) / 0.848 |
| Missing runs of 3+ words (loss, loop, script) | 0, 0, 0; 1, 0, 0 | 2, 0, 0; 1, 0, 0 | 1, 1, 3; 6, 0, 2 |
| Repeated word triples at seams | 7 (125 an hour) | 16 (92 an hour) | 74 (158 an hour) |
| Timing differences of the same segment in the two plans (#381): median, 95th percentile, maximum start; pairs over 0.5 s | 0, 0.18 s, 0.18 s; 0 of 4 | 0.06 s, 3.06 s, 4.26 s; 4 of 30 | 0.04 s, 0.84 s, 2.80 s; 7 of 65 |
| Candidates a minute; windows over their budget (candidates dropped) | 31.6; 3 (25) | 24.8; 1 (6) | 6.8; 0 |

**What RM09 showed, for issues (the words quoted are from a public-domain NASA recording):**

1. **Speech in one language dropped or rendered in another when a chunk's language is detected otherwise** (#474, L-188; the language
   check and two of the seam check's "listen" runs). The recording is an English briefing in which a Russian cosmonaut
   answers in Russian. In the shifted run, chunk 44 (1,434.2 to 1,464.2 s) wrote two short Russian segments up to 1,440.2 s
   and **nothing for the English answer that follows (about 1,441 to 1,459 s)**, which the whole run heard ("You know,
   sometimes when people ask that question ... For six months, you're going to need food ..."); chunk 42 (from 1,384.2 s)
   wrote **one 25-second Russian segment over the English question** "And we'll wrap with this one. What is your last meal
   going to be before launching the space?" (as the whole run wrote it); the whole run, cut elsewhere, rendered parts of the cosmonaut's Russian as mixed
   English and Russian. In each case `transcript get` and `search` call the stretch transcribed. Five such stretches in
   one run. It looks like whisper.cpp's per-chunk language detection on a mixed-language recording; whether VSift should fix
   one language per run, detect it per stretch, or report a chunk whose language differs from its neighbours is a decision.
2. **A recogniser loop in a silent lead counted as speech:** before the first word (about 5:15) the runs wrote "Thank you."
   and, in one plan, a phrase repeated fifteen times over 290 to 305 s; the first segment of the whole run starts at 25 s.
   Data for #150 and L-020 (noise) and for #383 (what a rejected or hallucinated segment covers).
3. **A segment that starts at its chunk's start, before the speech (#174):** the whole run's first briefing segment, "All
   right, good afternoon and welcome to NASA's Kennedy Space Center", is 300.00 to 321.28 s: 300 s is the chunk's start,
   and the words start about 15 s later.
4. **An earlier plan of the same item (shift at 15 s, 2026-10-19, run kept locally)** also showed a chunk that wrote nothing
   for the first 17 seconds of its audio: its first segment, 1,040.00 to 1,060.76 s, holds only the end of a reporter's
   question that the other plan heard from 1,044.1 s on; again the stretch is called transcribed. It is not language
   detection (both plans wrote English there); a listener should confirm the words are in the audio.

**The seam guard itself (#359) held:** no `loss` run of eight words or more on RM06 and RM10, and on RM09 the long runs are
explained by the recogniser (loops, a language flip) or are the language cases above. The timing differences answer #381's
question for the first time on real speech: most matched segments agree to within 60 ms, but RM06's 95th percentile is
3.06 s and its maximum 4.26 s, far beyond the half-second tolerance; segments that move that much are different
segmentations of the same words as much as timestamp noise, which the next step should separate before the tolerance is
changed.

### 14.2 The seam tolerance measured on real speech (#381, hosted, 2026-10-19)

The timing differences of 14.1 compare two chunk plans, so they mix timestamp noise with segments cut differently. What the
tolerance decides is narrower: a stretch of what two chunks of **one** plan heard that no kept segment covers, of up to half a
second, is left. Such a stretch is a gap of up to half a second where one chunk's kept segment meets the next chunk's at a
seam (`seam_joins`, check 5). Within one chunk whisper.cpp writes segments that touch, so the same gaps between one chunk's
own segments are the control. Hosted `P14 real media` run 39224705780 (Ubuntu 24.04, whisper.cpp and `base` as `P07 local
ASR` stages them, the merge of `main` at `a5f35b2d`), both chunk plans of each item:

| | Seam joins (whole / shifted plan) | Seam gaps of 0 to 0.5 s | Of which up to 0.1 s / 0.25 s (both plans) | Seam gaps over 0.5 s | Joins inside one chunk | Their gaps of 0 to 0.5 s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| RM06 | 29 / 26 | 4 / 1 | 2 / 4 | 5 / 2 | 90 / 80 | 0 / 1 |
| RM09 | 59 / 58 | 5 / 4 | 2 / 4 | 12 / 12 | 187 / 159 | 1 / 0 |
| RM10 | 7 / 8 | 1 / 0 | 0 / 1 | 0 / 3 | 22 / 20 | 0 / 0 |
| All six revisions | 187 | **15** (8 per cent) | 4 / 9 | 34 | 558 | **2** (0.4 per cent) |

- **What the tolerance leaves is a merge artefact, and rare.** One seam join in twelve has a gap of up to half a second,
  against one join in 280 inside a chunk, so those gaps are where the guard left a stretch, not pauses. Over the six
  revisions (about 84 minutes of audio) that is one such stretch every five to six minutes, each at most half a second, 9 of
  the 15 at most 0.25 s. Whether they hold a word is not known: the merge dropped what was there, and nobody has listened.
  From this change the campaign lists where each one starts (`within_tolerance_at_us`) for a listener.
- **The gaps over half a second at seams** (34) are not the tolerance's: the guard closes every such stretch of what the chunks
  heard, so these are stretches no chunk wrote anything for (pauses, or speech both chunks missed), or, on `main` before #382,
  copies the repeat rule dropped.
- **Identical text in two plans:** median start and end differences 40 ms on RM06 and RM09 (0 on RM10); the 95th percentiles
  of about one second for ends and up to 3 s for starts are segments cut differently (#174), not noise of one segment's times.
- **The decision: the tolerance stays half a second** (the maintainer's, 2026-10-19, recorded on #381; L-156). The derivation still bounds it (a larger value
  could lose speech beyond a window's edge). A smaller one is safe at the edge, and would make the guard restore a copy for 6
  of the 15 stretches at 0.25 s, or 11 at 0.1 s. Since #382 such a copy is kept even where its words repeat, so the price is
  that many more repeats. With nothing known about what the 15 stretches hold, no value below half a second can be shown to
  lose less than it repeats. **What would change it:** a listener finds words in the listed stretches. The figure to reach for
  is then 0.1 s, about twice the median noise.
- **A second run** (39229213722, this measurement's final form) recognised the whole runs identically and the shifted runs a
  little differently (hosted runs do not always recognise alike, #486): 12 of 185 seam joins with a gap of up to half a second, against 2 of 569 inside a chunk. **Where to
  listen** (the start of each such gap, in seconds, whole run unless marked): RM06 99.96, 124.96, 149.82, 503.0 and 482.44
  (shifted); RM09 554.52, 629.04, 678.0, 1,051.52, 1,379.4 and 768.52 (shifted); RM10 178.14.
### 14.3 Segments that start at their chunk's start, measured on real speech (#174, hosted, 2026-10-19)

The `base` model often starts the first segment of a chunk at the chunk's first sample, whatever the audio holds (L-024). After
a pause, noise or a silent lead such a segment claims time before its first word, and how much depends on where the chunk was
cut (14.1: RM09's first briefing words cited from 300.00 s, the chunk's start, about 15 s early; from 315.00 s in a plan cut
elsewhere). Hosted run 39224705780 (Ubuntu, both chunk plans of each item; `chunk_start_segments`, check 8) measured it without
labels. The run's first chunk is counted apart.

| | Segments at their chunk's start (whole / shifted plan) | Their length: median, 95th percentile, maximum | Longer than 10 s | Same words started later by the other plan, mid-chunk |
| --- | --- | --- | ---: | --- |
| RM06 | 18 of 167 / 20 of 152 | 5.2 s, 7.4 s, 8.0 s / 5.4 s, 12.0 s, 12.0 s | 0 / 2 | 1 pair: 2.7 s later / none |
| RM09 | 39 of 366 / 43 of 316 | 5.3 s, 19.0 s, 22.5 s / 5.8 s, 13.3 s, 24.0 s | 4 / 6 | 3 pairs, median 1.0 s, max 3.1 s / 6 pairs, median 3.7 s, max 4.1 s |
| RM10 | 5 of 41 / 4 of 42 | 6.3 s, 9.4 s, 9.4 s / 6.1 s, 7.0 s, 7.0 s | 0 / 0 | none |

- **About one segment in nine starts at its chunk's first sample.** Many are honest, because speech that runs across a seam does
  start there. Where the other plan heard the same words mid-chunk (10 pairs), it started them **1 to 4 s later**. That is
  what a citation of such a segment can claim before the words. The pairing needs the same three words or more in both plans,
  so the longest claims (RM09's 15 to 17 s, 14.1 and #383) are not among the pairs, and the 12 segments over 10 s are where to
  look for them.
- **It can hide missing words, too.** A segment that claims 17 s before its words covers that time in coverage, so words said
  there that no chunk kept are called transcribed (RM09 at 1,040 s, #383; L-158 (b)); whether that is what hides RM09's seam
  loss at 922 to 929 s is #485.
- **What could be done (#174). The maintainer decided on 2026-10-19: option 3 in `0.2.3`, option 1 evaluated in R1:**
  1. **Token times from whisper.cpp's DTW alignment** (`-dtw base`). Each word gets a time, and a segment's start is its first
     word's, in noise too. Cost: a new decoding profile (`r0-v2`) and P07 accuracy re-qualified on it; the word times need a
     parser of the `t_dtw` field and a stored member, which older releases read as damage (as with L-160).
  2. **Trim a chunk-start segment to the first 20 ms frame above -50 dBFS**, the rule that already calls a chunk silent. It is
     exact and deterministic, but it helps only silent leads (F09's 0.75 s to 4.0 s); in noise like RM09's lead nothing is
     below the threshold. It needs the trim stored with the segment and the frames at hand on a resume, which a checkpoint does
     not keep today.
  3. **Say it, and change nothing in the times.** The guide's limits page says so from this change (it did not before), and
     each segment's `alignment.provider_start_us` of 0 already marks the kind. Free, and it fixes nothing.
- **The measurement's own limits:** two plans of one recogniser, not a labelled truth; `speech_start_us` labels (check 8) would
  give the error itself.

### 14.4 The same binary recognises differently on another CPU (#486, hosted, 2026-10-20)

From #486's first step the report names the machine that recognised (`runner`), the recogniser threads of each revision
(`threads`) and each public revision's segments as times and text digests (`segment_fingerprints`). The same commit was then
dispatched twice (runs 39466357442 and 39466382364): the same `vsift` binary (SHA-256 `beca9662aaa5...`), the same pinned
whisper.cpp, model and media, and 4 recogniser threads on 4 logical CPUs in both. The runners differed:

| | Run 39466357442 | Run 39466382364 |
| --- | --- | --- |
| CPU | AMD EPYC 9V45 (AVX-512, AVX-512 VNNI and BF16) | AMD EPYC 7763 (AVX2, no AVX-512) |
| Real-time factor (RM06 / RM09 / RM10) | 0.14 / 0.13 / 0.14 | 0.26 / 0.25 / 0.24 |
| RM06 whole run | 167 segments | 167 segments, **147 identical in order**; the first difference is segment 19 (75.00 s), whose end is 79.24 s in one and 79.32 s in the other |
| RM09 whole run | 368 segments | 368 segments, **different from the first segment on** (25.00 to 35.00 s against 25.00 to 55.00 s, in the silent lead); 344 texts in common; the shifted run's start point moved with it (283 s against 66 s) |
| RM10 whole and shifted runs | 41 and 42 segments | **identical**, segment for segment |

- **The thread count is not the cause:** it was 4 in every run, set by VSift from the runner's 4 logical CPUs
  (`recognizer_threads`), as in every earlier hosted run.
- **The difference follows the CPU in this pair.** ggml, whisper.cpp's tensor library, picks its kernels from the CPU's
  features, and floating-point sums in a different order give slightly different numbers; a beam search can turn that into
  another timestamp or another segmentation. Two runs on the same CPU model were not part of this pair, so whether one CPU
  model always recognises alike is not shown. Earlier runs did not record the CPU, so they cannot be sorted by it.
- **What it means:** hosted before-and-after figures mean something only within one CPU model. A user's two machines can give
  two slightly different transcripts of one recording; nothing VSift publishes claims otherwise (L-200).
- **The rule from now on (the maintainer's decision of 2026-10-20, recorded on #486):** a before-and-after comparison of hosted
  runs counts only a pair whose reports name the same CPU model (`runner.cpu_model`); a pair on two models is reported as such
  and is not evidence of a change. Exact comparisons are made on LOKI, one fixed machine, once its lane runs.

### 14.5 What the two chunks of RM09's seam at 925 to 930 s hold (#485, hosted, 2026-10-20)

#485: the whole run's seam check finds 16 to 19 words missing at 922 to 929 s that the shifted run heard mid-chunk, and the
revision calls the stretch transcribed and lists nothing. Hosted run 39466921335 kept the segments of both revisions over 915
to 940 s (`keep_segments="RM09:915-940"`; the recording is public domain; the text is in the run's artifact,
`kept-segments/RM09/segments.json`). In words, without quoting the speech at length:

- **Whole run, chunk 36 (audio 900 to 930 s):** one Russian sentence of the cosmonaut's ("I am sure everything will be fine,
  crewed spaceflight will go on"), written **twice**, as two segments with the same text: 900.00 to 922.50 s (from the chunk's
  first sample, #174's pattern) and 922.50 to 929.50 s. The second is a recogniser repetition: it says nothing new and covers
  the time in which the next words were said.
- **Whole run, chunk 37 (audio 925 to 955 s):** its first kept segment starts at 927.96 s, 2.96 s into its audio (English: the
  interpreter's "we'll see each other again" and the host's thanks follow).
- **Shifted run, chunk 25 (audio 908 to 938 s), which heard the stretch mid-chunk:** the same sentence once (915.00 to
  920.80 s), then the words that are missing from the whole run, in Russian and English (920.80 to 927.70 s: "and I think all
  will be well, and we shall continue human spaceflight"; 927.70 to 930.90 s: "we will surely see each other again").

**So the missing words are in neither chunk's kept segments of the whole run.** Chunk 36 repeated its sentence instead of writing
them. Whether chunk 37 wrote them in the 2.96 s before its first kept segment cannot be read from any answer, because the merge's
dropped segments are not published. The merge's rules make it likely: a segment of chunk 37 over 925.00 to 927.96 s would have its
midpoint in chunk 36's core, where chunk 36 has a segment (the repeated copy), so rule 1 drops it, and the repeated copy covers
its time, so the guard has nothing to restore. The synthetic layout `a_repeated_copy_in_one_chunk_can_hide_the_next_chunks_words`
(`crates/vsift-domain/src/asr/tests.rs`) reproduces exactly that, and pins it as a limit (L-201).

**Where a fix could belong, for the maintainer's decision (#485); nothing is fixed:**

1. **The merge, narrowly:** a segment whose text repeats the segment before it in the same chunk (the same words, at the next
   time) is a recogniser loop, and owns no time: rule 1 lets the neighbour chunk's segment there be kept, and the guard judges
   holes without it. The repeated copy could be kept or dropped (dropping it removes a repeat). It needs the loop to be told
   from speech that does repeat ("no, no"), by the same word-count floor as the repeat rule.
2. **The guard, broadly:** restore a dropped segment whose words no kept segment holds even where its time is covered. It finds
   more losses of this kind and others, and brings back many copies that differ only by recognition (more repeats, L-155).
3. **Recognition:** whisper.cpp's repetition on this chunk is the root, and on a mixed-language chunk (#474) it is the
   recogniser's. A per-chunk check for a repeated segment could also mark the chunk's coverage as doubtful. It is part of #474's
   design in R1.

Option 1 is the smallest change that would have kept these words. Whether it holds on more recordings needs the dropped segments
of a run, which a future option of the campaign (or an engine-level measurement) would have to keep. Hosted runs recognise
slightly differently by CPU (#486, L-200); this run's CPU was not recorded (its branch predates the runner field).

**The fix (the maintainer's decision of 2026-10-20: option 1) and the run after it.** A segment that repeats the one before it in
its own chunk now claims no time in the merge's rules 1 and 2 (`repeats_the_segment_before`, `crates/vsift-domain/src/asr.rs`).
Hosted run 39486605862 kept the same range on the fixed branch:

- **Whole run, the seam at 925 to 930 s:** chunk 36 wrote the same two copies as before (identical times and text), and **chunk
  37's segment over 925.00 to 927.96 s is now kept**: "will be okay and we'll continue human spaceflight" (its first 2.96 s, the
  segment rule 1 had dropped). So the words were in chunk 37 all along, as the merge's rules suggested.
- **The whole run's seam check at that seam:** before, `loss`, 16 words at about 922.1 to 929.0 s (run 39466921335); after, `loss`,
  9 words at about 922.3 to 926.1 s, and `script`, 7 words at about 927.9 to 929.6 s. What is left is the Russian half of the
  sentence ("and I think all will be well"), which chunk 37 heard as English, and its next words, written in another script: the
  language limit of L-188, not the merge. The finding (eight words or more) still shows at that seam, with 9 words.
- **Elsewhere the two runs are not a pair:** this branch predates the `runner` field (#494), so neither run names its CPU, and by
  the maintainer's decision of 2026-10-20 (#486) a before-and-after comparison counts only a pair on the same CPU model. The
  shifted runs started at different points (283 s before, 66 s after), which happens on different CPUs (section 14.4), so the
  rest of the two reports is not compared. The seam above is read from the kept segments themselves.

### 14.6 RM03's three seam findings (#539, hosted, 2026-10-28)

RM03's first campaign runs fail on three seam findings. Two hosted runs of RM03 give **the same three, at the same times and
with the same word counts**: run 40654763827 on `35a991f6` (`main` before the progress stack, #508 to #513; both revisions sealed
at the end, AMD EPYC 9V45) and run 40595722898 on `98565a40` (both revisions published as they go, which `transcript
retranscribe` does on `main`; AMD EPYC 7763). So the progress stack did not cause them, and they are not the variation by CPU of
section 14.4. The first run kept the segments over 65 to 95 s, 752 to 790 s and 815 to 842 s (`kept-segments/RM03/segments.json`;
RM03 is CC BY-SA 3.0).

1. **The whole run's seam at 75 to 80 s, 15 words (about 76.9 to 82.7 s): a chunk heard as another language, looping.** Chunk 3
   (audio 75 to 105 s) wrote one segment over its whole window, 75.00 to 105.00 s: 56 Turkish words, 50 of them one word
   repeated. whisper.cpp detects the language per chunk (`-l auto`), and this chunk of English speech came out as Turkish; the
   whole revision's `language` is `null` (VSift's value when the chunks disagree) and the shifted revision's `en`. The loop owns
   the time of chunk 2's last words in the overlap, so chunk 2 is kept only up to 76.68 s, and the guard sees no hole because the
   loop covers it. The shifted run heard those words mid-chunk ("administrative measure taken by the government and then it was
   backed up with a judicial one"). The same loop is behind a 7-word run at the next seam (about 97.1 to 99.9 s). In all, about 25 s
   of English has no English text in the whole revision. **This is L-188's limit with the loop L-201 leaves**, not a new fault of
   the merge. The campaign named it as neither: the language check compares writing systems, and Turkish is in Latin script like
   English (`language_flips` 0); the run is classified `loss` because the loop is in the revision that lacks the words.
2. **The whole run's seam at 825 to 830 s, 10 words (about 825.7 to 829.7 s): the same speech worded differently.** Chunk 32
   (audio 800 to 830 s) wrote its last segment over 825.00 to 829.88 s, in the final five seconds of its window, and chunk 33
   starts at 829.96 s. The shifted run heard the stretch mid-chunk with other words ("cities" where the whole run has
   "universities", "1,000" where it has "1000"), which break the word triples the check matches. Both revisions have text over
   the whole stretch.
3. **The shifted run's seam at 761.08 to 766.08 s, 10 words (about 760.9 to 763.9 s): the same, the other way.** Chunk 28 of the
   shifted run (audio 736.08 to 766.08 s) wrote its last full segment over 761.28 to 765.76 s with other words than the whole run
   heard there mid-chunk.

**Over the recording, seams lose no more than two cuts of one chunk disagree anyway:** the whole run's seam recall is 0.897
against 0.894 at the control stretches inside one chunk, and the shifted run's 0.933 (explained runs counted as found) against
0.877. Which wording of findings 2 and 3 is right needs a listener. **For the maintainer's decision (#539):** L-188's design for
`1.0.0` would remove finding 1; a narrow merge rule of L-201's kind (a segment whose own words loop claims no time) would keep chunk
2's last 3.3 s beside the loop; and the campaign could report each chunk's detected language (to see a flip in the same script)
and classify a run as `repetition` when the revision lacking it holds a loop there. The check's threshold is unchanged.
