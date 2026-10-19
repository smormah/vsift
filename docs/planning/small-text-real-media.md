# Small on-screen text in real recordings (#173, L-018)

Status (2026-10-19, cycle 2, `0.2.3`): **an evaluation, first measurement done, the maintainer's decisions taken (section 4);
nothing that ships changes. A person's look at real crops is next.** Issue:
[#173](https://github.com/smormah/vsift/issues/173). Register: [L-018](known-limits.md#l-018). The real-media set and its
campaign: [`real-media-set.md`](real-media-set.md).

## 1. The question

`crop` returns a frame's own pixels at native size and never scales them, so that VSift never invents detail
([P09 record](p09-evidence-navigation.md), V-06). Small text was checked only on a synthetic 5x7 glyph drawn at scale 4.
#173 asks two things about real recordings, where compression dominates small text: **can an agent read the cropped
values**, and **does the no-upscaling decision need a documented alternative on the agent's side**?

## 2. How it was measured

**On the real recording, mechanically (hosted).** The real-media campaign now measures the text in native crops of
every item that covers `screen-ui-small-text` (`measureSmallText` in `tools/p14-campaigns/lib/real-media-campaign.cjs`).

- **The crops:** each of the three frames the campaign takes is cut into a grid of crops of at most 480x120 that
  covers it (18 at 1280x720).
- **The checks:** each crop's PNG, as VSift wrote it, is read back (`lib/small-text.cjs`, no dependency) and checked
  to be at native size.
- **The text lines:** each crop's lines of text are measured. A line is a band of consecutive rows that each hold many
  strong horizontal edges (48 grey levels or more), between 4 and 64 rows tall, and not cut by the crop's edge. The
  band is the part of a line where strokes are dense: on Segoe UI, drawn without compression, it is about 0.8 of the
  font's pixel size:

  | Font size (px) | 9 | 10 | 11 | 12 | 13 | 14 | 16 | 18 |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | Band (px) | 8 | 7 | 10 | 11 | 11 | 11 | 13 | 15 |

- **Where:** `P14 real media` on RM10, run [39223773538](https://github.com/smormah/vsift/actions/runs/39223773538) of
  2026-10-19 on this branch's first commit. Only numbers are kept: no crop leaves the runner, by the maintainer's rule
  for the hosted run.

**Legibility, on synthetic text compressed like RM10 (local).** No crop of a real item may leave the hosted runner, and
no real recording is decoded on a development machine. So the reading test used text rendered locally:

- **The text:** the line "Bexar County, Texas 2,009,324 15.2% Harris 4,731,145" in Segoe UI at 9 to 18 px, on a
  1280x720 page, drawn by FFmpeg (gyan.dev 9.0 build).
- **Two encodings, H.264 at 30 fps:**
  - **calm:** a slow scroll, about 210 kbit/s, under 0.008 bits per pixel;
  - **stressed:** a jittering scroll at about 100 kbit/s.
- **What was compared:** a native 480x300 crop of one frame against the same crop enlarged twice by pixel replication.
- **The reader:** the evaluating agent (Claude Opus 5.5) read both. This is one model's reading and not an agent trial.

## 3. What it found

**RM10's interface text is small.** RM10 is the Census Bureau screen recording, H.264 1280x720, under 0.016 bits per
pixel.

- **The crops:** 54 native crops of three frames, all at native size, all read back.
- **The text lines:** 49 lines of text. The median band is **8 px**, the 10th percentile 6 px and the smallest 4 px;
  the largest is 24 px, the captions and headings.
- **What that means:** by the table above, half the text is about 10 px type or smaller, and a tenth is about 7 px or
  smaller. That is the size of a table's figures and a site's interface text at 720p.

**Legibility is decided by the compression of the frame more than by the size of the text** (synthetic, one reader):

- **Calm frame:** every line, 9 px included, is legible at native size; enlarging adds nothing.
- **Stressed frame:** the 10 to 13 px lines are smeared past reading, and the 9 px line only partly survives. The 14
  and 16 px lines read as text, but with doubtful digits: "4,731,145" looks like "4,731,14!" on one and "4,/31,145" on
  the other.
- **Enlarged twice:** the stressed crop shows the same smear, larger. Pixel replication cannot bring back what the
  encoder discarded, and interpolation would only invent it.
- **The worst case is not illegibility:** it is a **plausible wrong digit**.

## 4. The maintainer's decisions (2026-10-19, recorded on #173)

1. **Crops stay unscaled.** VSift keeps P09's decision: crops at native size, never upscaled. Enlarging does not
   recover detail lost to compression, which dominates on real recordings. An agent or person who wants a bigger view
   can still scale the image itself.
2. **The advice on small numbers goes into the skill and the help text in cycle 3, with #454.** The agent-side
   alternative that matters is the choice of frame, not its scale. Read small text, and above all numbers:
   - from a frame where the screen has settled: a candidate with stability `settled`, or a frame shortly after the
     change rather than during it;
   - after comparing a neighbouring frame (`frame neighbours`, `frame burst`);
   - and treat a number read from text under about 10 px as uncertain unless a second frame or the transcript
     agrees.

   **Documented now** in the user guide's limits page ([`limits.md`](../guide/limits.md), "VSift does not read text in
   a picture"). The skill and `--help` say it with #454 (help that leads a cold agent), under that cycle's freeze; the
   skill is not changed here.
3. **The hosted run may keep a handful of native crops of public, openly licensed items, for a person to judge
   legibility (check 9). A private item's crops are never kept.** Built with this decision (the pull request of #173):
   - **How many:** at most **4 crops an item and 12 a run.** Four crops of an item's smallest text are enough for a
     person's yes or no per item; twelve cover three screen recordings in one artifact.
   - **Which:** the crops whose smallest line of text is smallest, ties to the earlier crop. Each is VSift's own PNG,
     unscaled, named `<id>-crop-<n>.png`, with the item's attribution and licence in `ATTRIBUTION.txt` beside them.
   - **Whose:** only an item that is not private and whose licence is public domain, CC0, CC BY or CC BY-SA
     (`mayKeepCrops` in `lib/real-media-campaign.cjs`). A private item is still measured, by numbers only.
   - **Where:** under `crops/<id>/` in the `p14-real-media` artifact (90 days); the report lists them as
     `measurements.small_text.kept_crops`. The media and the listening clips still never leave the runner.
4. **Next:** the person's yes or no on RM10's kept crops, from one campaign dispatch of this change; then RM01 once it
   is mirrored: `gh workflow run p14-real-media.yml --ref main -f items="RM01 RM10"`.

## 5. Limits of this measurement

- **Text size:** one real recording, three frames. The band measure is a stroke-density estimate, calibrated on one
  font, not a glyph reader. A dark-mode screen with light text, or text on a picture, may be measured differently.
- **Legibility:** synthetic text with one font, at two compressions, read by one model. It shows the mechanism
  (compression of a moving frame destroys digits and enlarging does not help), not a rate for real recordings.
