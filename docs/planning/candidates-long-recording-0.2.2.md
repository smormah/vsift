# `candidates` on a long recording: the measurement for #367 (2026-10-17)

Status: measured, decided and built in cycle 1 (`0.2.2`). The maintainer chose option (a) on 2026-10-18: an opt-in
filter that leaves out periodic-only candidates, `candidates --omit periodic` ([`cli-v1.md`](../contracts/cli-v1.md),
"Leaving out periodic candidates"), which the skill uses to shortlist the screens of a range with no transcript. Register:
[L-152](known-limits.md#l-152). Issue: [#367](https://github.com/smormah/vsift/issues/367).

## What was measured, and on what

- **The recording is generated from the committed synthetic corpus, never from a private recording:** the eight visual
  fixtures `F01.mp4` to `F08.mp4` (`fixtures/corpus/generated/`), each scaled and padded to 1920x1080 at 20 fps and held on
  its last frame for 40 seconds, joined (419 s), and that sequence three times: **20:57 (1,257 s), H.264, no audio,
  3,464,979 bytes, SHA-256 `0b71c6a2bdd02d792df47dd86abec62ee4a6a05f5bf59cabd8e373b561dc5900`** (built twice, the same
  bytes both times, with the FFmpeg on the measuring machine). It is a walkthrough of eight screens, each changing for 6 to
  20 seconds and then still for 40, shown three times. A real recording moves more: the issue's had about 15 candidates a
  minute, this one 6 to 8.
- **The tool:** `node tools/p14-campaigns/candidates-shortlist.cjs build|measure|project` (its header says how), with a
  fresh session root and per-user folders under a scratch folder for every run.
- **The binary:** `vsift` built from `main` at `d2fa40a` (release profile; a debug build gave the same calls and
  candidates and the same characters within ten), on Windows 11. The machine was shared with other builds and tests while it ran, so the seconds vary
  from run to run by a factor of two or more; they are upper bounds, not a benchmark.
- **"Shortlist the screens"** means reading every candidate of the whole recording, as an agent does before it chooses
  which frames to open.

## Results: today (the behaviour of `0.2.1` and `main`)

| Procedure | Calls | Characters | Seconds in all | Longest call | Candidates |
| --- | --- | --- | --- | --- | --- |
| One range for the whole video, the default page of 20, the cursor to null (what the issue's agent did) | 7 | 95,112 | 84.1 | **83.4**, the first: it analyses all 21 windows before it prints anything | 140 |
| The `0.2.1` skill: ranges of at most 5 minutes, `--limit 100 --view brief`, each range's cursor to null | 5 | 38,349 | 96.9 | 21.6 | 140 |

- **What the 140 candidates are:** 92 `periodic_coverage` alone (66%), 47 `visual_change`, 1 `first_frame`; 12 distinct
  `visual_hash` values in **39 runs of equal hash** (about one run per screen shown); 6 to 8 candidates a minute. No
  candidate was dropped and no window was left `not_analyzed`.
- **The first call's silence** is the whole-video call's: 83.4 s here for 21 windows of synthetic 1080p, over 120 s in the
  issue for 17:34 of a real recording. The skill's rule of one range of at most 5 minutes a call (in `0.2.1`) already keeps
  every call to at most 5 windows (16 to 22 s here; 54 s for 5 windows in a later run under heavier load). An analysis call
  also pays a fixed cost before its first window (a call for one window took 14 to 16 s, the session's first 36 s), which
  was not broken down.
- **The image budget is the tighter limit:** the `standard` profile has 24 images, and neither 140 candidates nor 39 runs
  fit in it; 12 distinct screens would.

## What each option would give (projected from the same 140 items)

| Option | Items | Characters (brief view, 5-minute ranges) | Calls | Public surface |
| --- | --- | --- | --- | --- |
| Today | 140 | 38,349 | 5 | none |
| (a) An option that leaves out items whose only reason is `periodic_coverage` | 48 | about 18,100 | 5 | a new option of `candidates`: its name, how it binds the cursor (`candidate_query_digest`), how `coverage` reports what it left out, the schema text, a contract test |
| (b) An option that keeps one item per run of equal `visual_hash` | 39 | about 15,700 | 5 | as (a); the design pass of #376 warned that the hash is a coarse 9x8 difference hash, so a screen whose text changed can fall into the run before it |
| (a) and (b) together | 39 | about 15,700 | 5 | as (a) |
| (c) A smaller analysis per call by default (a bounded first window) | 140 | as today | 5 or more | changes an existing call's answer (more `partial` with `not_analyzed`); `cli-v1.md` says a call analyses up to 30 windows |
| (d) Skill text only: open one frame per run of equal hash, skip periodic-only items when choosing frames | 140 read, at most 39 opened | 38,349 | 5 | the skill and a new trial freeze; no CLI change |

Not measured, by the decision of 2026-10-11 (design pass #376): progress events and parallel windows.

## After: `--omit periodic` (built)

The same recording and procedure with the option (`measure ... omit`), on the branch's build:

| Procedure | Calls | Characters | Candidates listed | Left out, as stated | Distinct screens (hashes) |
| --- | --- | --- | --- | --- | --- |
| The skill without the option (before; the same build gives the same answer, byte for byte) | 5 | 38,349 | 140 | none | 12 |
| The skill with `--omit periodic` (after) | 5 | **18,324** | 48 (1 `first_frame`, 47 `visual_change`) | 92, in `data.omitted.count` of each range | 12 |

The projection said about 18,100; the measured answer is 18,324, because each page also carries its `omitted` member and
the last, one-minute range now lists nothing but still answers. The calls stay at five (one per 5-minute range); the last
range's call holds no candidate at all, since every candidate there was periodic. Every distinct screen of the recording
is still listed. The 39 runs of equal hash become 39 runs in 48 items, so an agent that opens one frame per run still
needs more than the 24 images of the `standard` profile for the whole recording; the image budget, not the list, is then
the limit, and the skill keeps asking for one stretch at a time.

## The decision this needed (taken 2026-10-18: option a)

Whether `candidates` gains a filter (a, b or both) and in what form; whether a call's default analysis shrinks (c); or
whether only the skill changes (d). **(a) is the smallest change that halves the characters** on this recording (38,349
to about 18,100) and leaves every answer without the option as it is. Built as: the cursor binds the choice (the query
digest gains a line only when something is left out, so an existing cursor stays valid), and every answer that leaves
anything out says so in `data.omitted`. With (a) and the skill's 5-minute ranges the calls stay
at five for 21 minutes; the first-call silence is already bounded by that rule. **Evidence to close (from the issue):**
this measurement as "before", the same procedure after whatever is built, and a contract test for anything added.
