# The visual change rule on real recordings (#175, L-028)

Status (2026-10-19, cycle 2, `0.2.3`): **an evaluation, first measurement done; no rule changed.** Issue:
[#175](https://github.com/smormah/vsift/issues/175). Register: [L-028](known-limits.md#l-028). The real-media set and its
campaign: [`real-media-set.md`](real-media-set.md).

## 1. The question

Profile `r0-visual-v1` decides that two samples show different screens when, on the 16x9 grid of block means of a
128x72 grey sample, **two blocks move by at least 4 grey levels, or one block by at least 6**
([ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md), "Change rule";
`VisualChangePolicy::R0`). The thresholds were set on the synthetic corpus, whose unchanged samples differ by at most
1 level. #175 asks whether real recordings (compression noise, flicker, cursors, anti-aliased text) need other
thresholds or another profile.

## 2. How it was measured

**A measurement, not labels.** No person has labelled the moments the screen changes in any item yet
(`changes_us`, `real-media-set.md` section 6), so the measurement builds its own reference from the full-resolution
frames, independent of the rule:

- The opt-in test `crates/vsift-infrastructure/tests/p08_real_media_visual.rs` decodes each item's samples exactly as
  an index extension does (the real `FfmpegVisualSampler`, through the shared `tests/visual_decode`), builds the index
  with `R0`, and decodes **the same frames a second time at native resolution** with the adapter's own `select`
  expression and no scaling. Each window's native frames are paired with its samples and checked: their own block
  means differ from the adapter's samples by at most 1 level on average, on every window of every item, so each native frame is the sampled frame.
- **Every pair of consecutive samples** is measured twice: the rule's view (largest block difference, blocks moved at
  2 to 16 levels, `R0`'s decision, the hash distance) and the native reference (pixels moved by 16, 32 and 64 levels,
  their box, the densest 32x32 tile).
- **A still pair** is one in which no native pixel moved by 64 levels: a person sees no change. 64 and not 32,
  because the synthetic corpus's own static screens move by 32 to 63 levels at every keyframe (the encoder redrawing
  text edges, up to 507 pixels spread over a 609x362 box, invisible to a person and to the block means). **A changed
  pair** gets a size class by the share of the frame that moved by 64 levels:

  | Class | Share of the frame | At 1280x720 | What it usually is |
  | --- | --- | --- | --- |
  | speck | under 20 ppm | under 18 px | a caret, a stray pixel |
  | tiny | 20 to 199 ppm | 18 to 183 px | a cursor, one character |
  | small | 200 to 1,999 ppm | 184 to 1,842 px | a word, a small icon |
  | medium | 0.2% to 2% | 1,843 to 18,431 px | a line of text, a caption, a menu |
  | large | 2% or more | 18,432 px or more | a panel, a page, a scroll |

- It also replays **a grid of 92 other thresholds** of the same shape (block level 3 to 8, 1 to 4 blocks, single
  block 6 to 24) on the same pairs. The grid compares pairs only: the real analysis also compares each sample with the
  first sample of its screen state and merges motion, so the grid compares rules; it does not predict candidate counts.
- **Where:** the hosted `P14 real media` workflow with `mode: visual` (Ubuntu 24.04, the pinned FFmpeg
  `n9.0.1-11-ge47273f4d9-20260831`), on the items the test-media release holds. Run
  [39219218493](https://github.com/smormah/vsift/actions/runs/39219218493) of 2026-10-19 on this branch's first commit
  measured RM10, RM06 and RM09 in 69 s; its artifact `p14-real-media` holds `visual-measurement.json` (numbers only:
  ids, digests, counts, times; no pixel and no block mean). Nothing ran on a development machine except the control
  below.
- **The control:** the same test over the synthetic corpus (`measures_the_change_rule_on_the_synthetic_corpus`,
  eleven short clips, run locally): the noise floor is at most 1 level, there is no false change, and every changed
  pair the corpus has (the edits ADR 0018 calibrated on) is seen and has its candidate. The reference agrees with the
  corpus truth where the truth is known.

## 3. What it found

**RM10 is the measurement that answers #175:** the set's only mirrored screen recording (the Census Bureau's data
site, 3:21, H.264 1280x720 at 30 fps, under 0.016 bits per pixel counting the whole file's rate with its audio: heavy compression; burned-in captions). RM06 (a NASA
conversation, 854x480 at 59.94 fps) and RM09 (a NASA arrival, 320x180 at 15 fps) are camera footage: shown for
contrast, not for calibrating a screen rule.

### 3.1 The noise floor: real compression does not reach the thresholds

| Item | Still pairs | Largest block difference on still pairs (median, 95th, 99th percentile, max) | Still pairs `R0` calls a change |
| --- | ---: | --- | --- |
| RM10 (screen) | 112 of 402 | 1, 1, 1, **4** (one pair) | **0** (0 a minute) |
| RM06 (camera) | 5 of 1,247 | 1, 4, 4, 4 | 0 |
| RM09 (camera) | 66 of 3,167 | 1, 2, 44, 44 | 1 (0.04 a minute) |
| Synthetic corpus (11 clips) | 2 to 38 per clip | at most 1 | 0 |

On the real screen recording, an unchanged screen differs between samples by 1 level, as on the synthetic corpus; the
thresholds (4 for two blocks, 6 for one) stay four to six times above it, and **no false change** was seen. RM09's one
false change is a fade: the whole picture darkened by less than 64 levels per pixel (so the reference calls it still)
while a block mean moved by 44; a person would call it a change, so it is the reference's blind spot, not noise.

### 3.2 What the rule sees, by the size of the change

Share of changed pairs `R0` calls a change, and share a candidate accounts for (a change candidate at the pair's later
sample, or a motion candidate whose span holds it):

| Item | speck | tiny | small | medium | large |
| --- | --- | --- | --- | --- | --- |
| RM10, `R0` decides change | 0 of 22 | 5 of 27 (0.19) | 15 of 23 (0.65) | 161 of 171 (0.94) | 47 of 47 (1.00) |
| RM10, a candidate accounts for it | 0 | 0.26 | 0.48 | 0.75 | 0.89 |
| RM06, `R0` decides change | 1 of 6 | 0 of 15 | 64 of 187 (0.34) | 897 of 954 (0.94) | 80 of 80 |
| RM09, `R0` decides change | 0 of 8 | 0 of 29 | 127 of 262 (0.48) | 1,263 of 1,277 (0.99) | 1,525 of 1,525 |

**Lines of text, captions, menus and pages are seen** (94 to 100%). **A word or a cursor is seen about half the time or
less, a single character almost never:** the small and tiny changes `R0` missed moved their largest block by 0 to 5
levels, mostly 1 to 3. At 1280x720 one block of the grid is 80x80 native pixels (6,400), so a glyph's few dozen inked
pixels move its block's mean by a level or two: under the rule by construction, and within reach of the noise
([L-029](known-limits.md#l-029) says the same in words). The gap between "decides change" and "a candidate accounts for it" on medium changes (0.94 to
0.75) is the per-window budget and the merging of motion, below.

### 3.3 Candidates and the budget

| Item | Candidates a minute | By reason | Windows over budget, candidates dropped | Change candidates opened by a still pair |
| --- | --- | --- | --- | --- |
| RM10 | **32.2** | 65 change, 21 motion start, 19 settled, 2 periodic, 1 first frame | **3 of 4 windows at 32, 25 dropped** | 0 of 105 |
| RM06 | 25.4 | 84 change, 89 motion start, 84 settled, 6 periodic, 1 first | 1 of 11, 6 dropped | 0 of 257 |
| RM09 | 10.6 | 73 change, 72 motion start, 44 settled, 107 periodic, 1 first | 0 of 29 | 4 of 189 |

On RM10 **every change candidate stands on a real change** of its own pair (86 medium, 15 large, 2 small, 2 tiny) and
only 6 of 105 were opened by a block move under 8 levels: the 32 candidates a minute are the captions and the page
changing, not noise. **The binding constraint on a captioned screen recording is the budget of 32 candidates a
window**, which RM10 reaches in its first three minutes, not the thresholds.

**Timings** (hosted runner, indexing the whole video with `R0`): RM10 4.6 s for 201 s of 720p (44 seconds of video a
second), RM06 19.4 s for 624 s of 480p at 59.94 fps (32), RM09 5.3 s for 1,682 s of 180p (317). The native second
decode took about as long again; it exists only in the measurement.

### 3.4 Other thresholds, on RM10's pairs

| Rule (block level, blocks, single block) | False changes a minute | tiny | small | medium | large |
| --- | ---: | ---: | ---: | ---: | ---: |
| 3, 1 (any block at 3) | 0.30 | 0.41 | 0.91 | 1.00 | 1.00 |
| 3, 2, 6 | 0 | 0.26 | 0.83 | 0.98 | 1.00 |
| **4, 2, 6 (`R0`)** | **0** | **0.19** | **0.65** | **0.94** | **1.00** |
| 4, 3, 8 | 0 | 0.15 | 0.48 | 0.92 | 1.00 |
| 5, 2, 8 | 0 | 0.11 | 0.39 | 0.92 | 1.00 |
| 6, 2, 8 | 0 | 0.07 | 0.35 | 0.90 | 1.00 |
| 8, 2, 12 | 0 | 0.04 | 0.09 | 0.84 | 1.00 |

**A stricter rule removes no false change (there are none) and loses changes a person sees**: at block level 6 and a
single block of 8 (6, 2, 8), the share of small edits seen falls from 0.65 to 0.35 and of lines of text from 0.94 to 0.90. **A looser rule
(3, 2, 6)** would see more of the small edits (0.83 against 0.65) with no false change on this recording, but it stands one level
above a noise peak RM10 showed once (4), and on RM10 it would add candidates to windows that are already over budget.
The full grid of 92 rules for every item is in the run's artifact (`threshold_grid`).

### 3.5 What the campaign's check 7 needs

The campaign's first form of check 7 ("a candidate within one second of the labelled change") says little on a busy
screen: on RM10, seven in eight of the **still** pairs (0.875) have a candidate within a second. Check 7 now counts a labelled change as
found when a change candidate's `change_window` holds it (widened by half a second each side for the labeller's own
timing) or a `motion_start` candidate's span holds it, and it reports the one-second figure beside its chance figure
(the share of the recording within a second of some candidate) so that a reader can see when the loose figure is
uninformative (`labelledChanges` in `tools/p14-campaigns/lib/real-media-checks.cjs`, with its test).

## 4. Recommendation (for the maintainer's decision)

1. **Keep the thresholds of `r0-visual-v1` (4 for two blocks, 6 for one).** On the one real screen recording measured,
   compression noise stays at 1 level, there is no false change, and every change candidate stands on a change a person
   would see. A stricter rule only loses edits; a looser one gains some small edits for one level less margin over a
   noise peak and more pressure on a budget that is already the constraint. **No new profile version is proposed.**
2. **Do not make the thresholds configurable.** A configurable rule would make candidate identities and the index's
   profile depend on a setting, for no measured benefit; the profile identifier exists so that a future rule is a new
   profile with its own review, which remains the way to change it.
3. **What the evidence does point at is the budget, not the rule.** A captioned screen recording fills the 32-candidate
   budget in every full minute (RM10: three windows full, 25 dropped). Whether the budget should grow, or captions
   (changes confined to one band of the frame) be treated differently, is a separate question for [L-029](known-limits.md#l-029)
   and [L-152](known-limits.md#l-152); this evaluation does not propose a change to either.
4. **Small edits stay a documented limit.** A word is seen about half the time, a character almost never, at 720p:
   inherent in an 80x80-pixel block at this resolution. The agent's route to them is the frame and the crop
   (`frame get`, `crop`), and a transcript, not the shortlist.
5. **Measure again** when RM01 (a Wikipedia screen tutorial at 1280x1024, VP8) is in the test-media release, and on
   RM04 and RM07 (interface demonstrations): one dispatch, `gh workflow run p14-real-media.yml --ref <branch> -f
   mode=visual -f items="RM01 RM04 RM07 RM10"`. If a second screen recording shows a noise floor above 2 levels or a
   false change, this recommendation is reopened. **And once a person has labelled `changes_us`** for RM10 and RM01,
   check 7 gives the first figures against human labels.

## 5. Limits of this measurement

- **One screen recording.** RM10 is heavily compressed (under 0.016 bits per pixel) and captioned; a lightly
  compressed or uncaptioned recording, dark-mode screens and VP8/VP9 sources (RM01, RM04, RM07) are not measured.
- **The reference is pixels, not meaning.** "Changed" means native pixels moved by 64 levels: it counts a cursor as a
  change and misses a fade or a change of low contrast (RM09's one false change). Human labels remain the truth when
  they exist.
- **Pairs, not candidates, in the grid.** The other rules are compared on consecutive pairs only; their candidate
  counts would also depend on the anchor comparison, motion merging and the budget.
