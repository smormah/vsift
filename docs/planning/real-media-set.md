# The real-media set and its opt-in campaign (#363)

Status: **design and candidate list for the maintainer's approval (2026-10-18), cycle 2 (`0.2.3`).** Nothing is downloaded
yet and no item is approved: every item in the manifest is `proposed`. This change adds the design, the pinned manifest with
its schema, and the fetch-and-verify tool with its tests; the campaign runner and its hosted workflow are the next change.
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

Eight candidates, **426,605,905 bytes (about 427 MB) in all**, every one on Wikimedia Commons, researched on 2026-10-18 from
the file pages and the Commons API (size, SHA-1, upload time, licence, author, duration, codecs) and, for the one transcode,
from the server's response headers. **Nothing was downloaded, watched or heard:** what an item sounds like is what its page,
captions and metadata say, and is checked when an approved item is first labelled. Each is in the manifest as `proposed`,
with the full licence links, attribution and notes. The maintainer approves or declines each one; an approved item is then
pinned on a hosted runner.

| Id | Recording (source page) | Licence | Duration, video, audio | The download | What it tests |
| --- | --- | --- | --- | --- | --- |
| RM01 | [Editing basics - watchlists](https://commons.wikimedia.org/wiki/File:Editing_basics_-_watchlists.webm), Sage Ross (WMF), 2013 | CC BY-SA 3.0 | 2:16; VP8 1280x1024; Vorbis | `RM01-editing-basics-watchlists.webm`, the original from upload.wikimedia.org, **6,192,962 bytes**, SHA-1 `9cc8ceb7...` | A narrated screen recording: page changes, menus, small interface text (#173, #175); human captions whose last cue ends at `99:59:59` |
| RM02 | [Structured Data on Commons uploads by the Metropolitan Museum of Art 01](https://commons.wikimedia.org/wiki/File:Structured_Data_on_Commons_uploads_by_the_Metropolitan_Museum_of_Art_01.webm), GFontenelle (WMF), workshop by the Met's collection-information manager, 2021 | CC BY-SA 3.0 | 12:54; VP8 1184x720 25 fps; Vorbis | `RM02-met-structured-data-demo.webm`, the original, **26,240,356 bytes**, SHA-1 `f0083f98...` | A live tool demonstration: speech with pauses while the presenter works (#353's shape, #383), screen changes and small text (#173, #175), about 31 chunks. **Check before approval:** other call participants may be visible in tiles |
| RM03 | [After the lifting of the block: Case study of Turkey](https://commons.wikimedia.org/wiki/File:After_the_lifting_of_the_block_Case_study_of_Turkey.webm), Wikimania 2021, Basak and Khutuck | CC BY-SA 3.0 | 19:16; VP9 1920x1080 30 fps; Opus | `RM03-wikimania-turkey-talk.webm`, the original, **146,468,542 bytes**, SHA-1 `a6deaabe...` (or its 480p transcode, 29,909,095 bytes, no published checksum) | The 15 to 30 minute talk: about 46 chunks of continuous speech (#380 to #384), Turkish-accented English (L-022), a music and title-card lead before the first words at about 13.4 s (#174; a non-speech lead rather than silence) |
| RM04 | [Como contribuir com o Structured Data on Commons](https://commons.wikimedia.org/wiki/File:Como_contribuir_com_o_Structured_Data_on_Commons.webm), Ederporto, 2019 | CC BY-SA 4.0 | 5:00; VP9 1280x720 24 fps; Opus | `RM04-como-contribuir-sdc-pt-br.webm`, the original, **10,636,205 bytes**, SHA-1 `5a2ab7b9...` | Brazilian Portuguese speech (L-022) with interface changes; Portuguese captions as a possible reference; an audio track wrongly tagged English |
| RM05 | [President Reagan's Trip to Fullerton and San Bernardino on November 1, 1988](https://commons.wikimedia.org/wiki/File:President_Reagan%27s_Trip_to_Fullerton_and_San_Bernardino_on_November_1,_1988.webm), Reagan Library (White House Television Office) | CC BY 3.0 (reviewed on Commons) | 15:53; VP8 654x480 29.97 fps; Vorbis | `RM05-reagan-fullerton-1988.webm`, the original, **112,179,359 bytes**, SHA-1 `dcd47fbd...` | Speech under real noise (#150, L-020): remarks at a rally in a gymnasium and at a helicopter landing zone through a field microphone; old analogue video |
| RM06 | [Space Station Astronaut Discusses Record Breaking Mission with Previous Record Holder](https://commons.wikimedia.org/wiki/File:SPACE_STATION_ASTRONAUT_DISCUSSES_RECORD_BREAKING_MISSION_WITH_PREVIOUS_RECORD_HOLDER.webm), NASA Johnson Space Center, 2023 | Public domain (US government work) | 10:31; VP9 1280x720 59.94 fps; Opus | `RM06-nasa-iss-record-conversation.webm`, the original, **84,997,974 bytes**, SHA-1 `a982fbda...` | A two-person conversation with turn-taking and the pauses of a space-to-ground link (#353's shape, #359, #383), about 25 chunks; a 59.94 fps source |
| RM07 | [Introduction to Wikidata Lexeme Forms - Lucas Werkmeister - Lexicodays 2024](https://commons.wikimedia.org/wiki/File:Introduction_to_Wikidata_Lexeme_Forms_-_Lucas_Werkmeister_-_Lexicodays_2024.webm) | CC BY-SA 4.0 | 9:39; the 480p VP9 transcode of a VP8 1920x1080 original | `RM07-lexeme-forms-480p.webm`, Commons' 480p transcode, **21,315,686 bytes**, no published checksum (ETag `78fc96f3...` taken as its MD5, unconfirmed) | German-accented English with a tool demonstration (L-022, #380, #381), about 23 chunks; **human-made English captions, the set's best reference transcript** |
| RM08 (optional) | [Mark Carney speaking about climate action and energy security in Calgary, 15 May 2026](https://commons.wikimedia.org/wiki/File:Mark_Carney_speaking_about_climate_action_and_energy_security_in_Calgary,_15_May_2026.webm), Jasonhargrove | CC BY 4.0 | 0:22; VP9 1080x1920 30 fps nominal; Opus | `RM08-calgary-vertical-phone.webm`, the original, **18,574,821 bytes**, SHA-1 `3f936fd5...` | A vertical phone recording of a speech in a hall (rotation, aspect, noise); re-encoded before upload, so its frame rate may no longer vary; a current politician, which the maintainer may prefer to leave out |

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

- the **source page** that states the licence, and the **file URL** on an allowed host (`upload.wikimedia.org`,
  `archive.org` and the Internet Archive's storage nodes `*.us.archive.org`; a host is added only in the reviewed change that
  adds an item from it);
- the exact **size in bytes**, the **SHA-256**, and the **checksum the source publishes** (the Commons API's SHA-1, the
  Internet Archive's MD5 or SHA-1), with the source's own **revision** (the Commons upload time) where it has one;
- the **licence** (an identifier from a closed list: `CC0-1.0`, `CC-BY-n.0`, `CC-BY-SA-n.0`, `public-domain`,
  `public-domain-us-government`), its text's URL and the URL where the source states it for this file;
- the **author and the attribution text**, the media's duration, container and codecs, the shapes it covers and the issues
  it should help settle;
- its **status**: `proposed` (listed for review, never fetched), `approved` (the maintainer approved it on a recorded date),
  `withdrawn` (kept for the record, never fetched, not counted);
- later, hand-made **labels** (section 6).

**The tool:** `node tools/p14-campaigns/real-media.cjs` (Node.js 22, no dependency), with its rules in
`tools/p14-campaigns/lib/real-media.cjs` and its tests in `tools/p14-campaigns/test/real-media.test.cjs`:

| Command | What it does |
| --- | --- |
| `check [<manifest>]` | Validates the manifest, lists the items and says which required shape no item covers |
| `attribution [<manifest>]` | Prints the attribution and licence of every item that is not withdrawn |
| `pin <dir> --item RMnn` | For an approved item with no SHA-256 yet: fetches it bounded by its size, verifies the source's checksum, keeps the file and prints the SHA-256 to write into the manifest in a reviewed change |
| `fetch <dir> [--item RMnn]...` | Fetches approved, pinned items into `<dir>/<id>/<file_name>` |

**The download rules of `AGENTS.md`, and how the tool meets them:**

- **HTTPS only**, on the allowed hosts, with no credentials or port in the URL; every redirect is checked the same way, at
  most five. The request names the tool only (`User-Agent: vsift-real-media-fetch/1`): never a person, an address or an
  account, even where a source's courtesy convention asks for contact details.
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
  off as soon as it passes the pinned size; a short one is refused. No item may exceed 300 MiB, and the items that are not
  withdrawn 750 MiB together.
- **Safe placement:** the destination must be an absolute path to an existing folder **outside the repository** (so a fetched
  file can never be committed by accident). Each file is written exclusively to `<dir>/<id>/.<file_name>.partial` and renamed
  into place only after verification. A file already in place is checked, never overwritten: with the pinned digest it is
  kept (`present`), with another it is refused and left alone. The tool deletes nothing but its own partial file of the
  same run, and never touches anything else in the folder. There is no archive to extract: every item is a single media file.
- **Source media is never changed.** The campaign gives VSift the fetched file as it is; VSift's own session copies it.

**Where the files go.** Locally, a folder the person running the campaign names, outside the repository (for example
`%LOCALAPPDATA%\vsift-real-media` or `~/.cache/vsift-real-media`); on a hosted runner, the runner's temporary folder, gone with
the runner. **The media is never uploaded as a workflow artifact**; only the campaign's report is.

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

`node tools/p14-campaigns/real-media-campaign.cjs <vsift> <media dir> <work dir> [--item RMnn]...` (the next change). Like the
other campaigns it drives the binary it is given by absolute path, never one on `PATH`, with a fresh session root and per-user
folders under its work folder, and it is never part of an ordinary pull-request run. Per item:

| Step | Commands | What it records |
| --- | --- | --- |
| Setup | `setup check` | The tools and the model, by identity; a `blocked` or `[timeout]` on a working setup is a finding (#391's shape) |
| Ingest | `ingest <file>` | Seconds; the video's duration, streams and frame rate as VSift reads them |
| Transcription | `transcript retranscribe <session>` (whole video, `base`, then `base_q5_1` on the long items) | Seconds and real-time factor, peak memory, chunks, silent chunks, `provider_segments_rejected`, `seam_duplicates_removed`, `local_asr.unplaced_ranges` |
| Seam shift | `transcript retranscribe <session> --from 15000000 --to <end>` | A second revision whose seams fall 15 s from the first one's, so each seam of one lies inside a chunk of the other (to be confirmed against the chunk plan when the runner is written) |
| Reads | `transcript get` (pages), `search` for words of the transcript at seams and away from them | Coverage, the hits, the seconds |
| Visual | `candidates` over the whole video in ranges of at most five minutes, `--omit periodic` as the skill does | Candidates per minute, the seconds per range |
| Evidence | `frame get` at three candidates, `crop` of a small-text region on the screen recording, `audio` around a seam | The frames' requested and actual times |
| Handoff | `handoff check` of a mechanical draft citing the run's own ids | `data.valid` |
| Close | `session close` | That the session closes and its folder goes |

**What it checks are properties, not exact words** (an item has no frozen transcript):

1. **No failure on valid speech:** no `MISSING_CAPABILITY`, `INTEGRITY_FAILURE` or unexpected failure on any item; a
   transcription of an item with speech commits a revision with segments.
2. **Coverage is stated truthfully:** every part of the video is in exactly one of `transcribed_ranges`,
   `untranscribed_ranges` and `no_speech_ranges`; `no_speech_ranges` lie where the audio is quiet (checked against the
   decoded audio's level), never across labelled speech; `search` is `partial` exactly when the range has a gap.
3. **No seam loss beyond the tolerance:** between the two revisions of the seam shift, every word sequence of three or more
   words that one revision holds near a seam of the other is found in the other (a word lost at a seam is said mid-chunk in
   the other run); every loss is listed with its time, and a loss longer than `SEAM_COVERAGE_TOLERANCE_MICROS` is a finding.
4. **Repeats at seams are counted** (#380): words that stand twice within two seconds at a seam, per seam and per hour.
5. **Timestamp differences between chunks** (#381): for the same words in the two revisions, the difference of their times;
   the distribution (median, 95th percentile, maximum) is the measurement #381 asks for before the tolerance is changed. A
   measurement of the raw chunk outputs themselves needs an engine-level `--ignored` test that writes each chunk's validated
   segments before the merge; it is part of the runner's change if the binary's results are not enough.
6. **Rejected segments** (#383): the kinds and counts of `provider_segments_rejected`, and where `unplaced_ranges` lie
   against the labelled speech.
7. **Candidates at visible changes** (#175): with `changes_us` labelled, the share of labelled changes with a candidate within
   one second, and the candidates per minute of a still screen (false changes).
8. **Speech start** (#174): with `speech_start_us` labelled, the first segment's start minus it.
9. **Small text** (#173): the crop of the labelled small-text region is at native size and legible to a person; recorded by
   hand, as a yes or no per crop.
10. **Timings:** seconds and real-time factor per step and item, against the budgets of the
    [qualification profiles](support-and-resource-profiles.md); a run several times slower than the last one is a finding.

The report is JSON plus a short Markdown summary, with the binary's `--version` and SHA-256 (never its path), the tools' and
model's identities, the machine's class, the items' ids and digests, and the attribution section. It holds measurements and
the run's own words only where a check needs them (a lost word sequence), never a whole transcript.

## 8. How it runs

- **Locally, on demand:** a developer fetches the approved items once into a folder outside the repository and runs the
  campaign against a build of their worktree. It needs FFmpeg, FFprobe, whisper-cli and a pinned model, as the other real-tool
  checkpoints do. **Nothing is downloaded on a workstation without its owner's word:** fetching is a download, so the person
  running it decides, item by item.
- **Hosted, by manual dispatch only** (a workflow `P14 real media`, the next change): on Ubuntu 24.04 first, Windows and macOS
  later. It stages the pinned FFmpeg, whisper.cpp and models as `P07 local ASR` does
  ([`p07_local_asr_tools.py`](../../tools/p07_local_asr_tools.py)), fetches the approved items with `real-media.cjs fetch`
  into the runner's temporary folder, runs the campaign against either a build of the commit or a published version (as the
  journeys do), and uploads the report only. It has `contents: read` and no secret. It is not scheduled: the items are long
  enough that a run costs real runner time (the 15 to 30 minute lecture alone transcribes in roughly 6 to 12 minutes at the
  `base` model's measured real-time factor), so it runs when a change touches transcription, the merge or the visual index,
  for a re-pin, and once per release window when such a change merged ("run costly tests sparingly").
- **Evidence:** a passed run on a candidate's own bytes can become a new evidence item of the release qualification; whether
  it is required for a candidate is the maintainer's decision (section 10).

## 9. How results feed the follow-ups

- **Seams (#380 to #384):** checks 3 to 6 give the measurements each issue asks for "on real recordings first"; each issue's
  next step is decided on them, and the register entries L-155 to L-159 are rewritten with the numbers.
- **#173, #174, #175:** checks 7 to 9, once the labels exist, give the first real figures; a threshold or profile change is a
  new visual profile version (#175) with its own review.
- **#150 and L-020, L-022:** with a reference transcript, a coarse word-recall figure per item (noisy, accented, non-English).
  It is not a gate: one or two items are not a sample, and a gate stays for #150's fixture set.
- **A defect found** is an issue first, then one re-run; its fix leaves a regression fixture that does not need the recording.

## 10. Decisions for the maintainer

1. **Approve the candidates item by item** (section 3). An approved item is then pinned on a hosted runner and its SHA-256
   recorded in a reviewed change.
2. Whether a passed campaign on a candidate's bytes becomes a required evidence item for `0.2.3-rc.1`, or stays advisory.
3. Whether the hosted workflow runs on Ubuntu only at first (proposed), or on all three systems.

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
