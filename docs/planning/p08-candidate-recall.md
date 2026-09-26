# P08 visual-candidate recall record

Status: measured 2026-09-26 on P08 PR 4 (`p08/candidates`, which merges PR 3 and PR 1),
Windows 11, Xeon E5-2698 v4, FFmpeg 9.0 (gyan.dev full build), release build.
Design: [ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)
(Proposed), decisions 9-14. Verification IDs: V-02..V-05, S-11.

## Method

Expectations come only from the frozen corpus truth
([`fixtures/corpus/manifest.json`](../../fixtures/corpus/manifest.json)); nothing is
derived from the candidates being scored. Two runs score the same rules:

- **Always-run gate** (`crates/vsift-infrastructure/tests/p08_candidate_recall.rs`):
  the samples FFmpeg 9.0 decoded from F01-F10 and F12
  (`crates/vsift-infrastructure/tests/data/visual_samples/`) replayed through the real
  index extension. It needs no media tool, so every CI run gates recall.
- **Live, through the binary** (opt-in `crates/vsift-cli/tests/p08_candidates_e2e.rs`,
  stage `p08_candidates_fixtures`): each fixture ingested and paged with `candidates
  --from 0 --to <duration>` exactly as an agent would. It produced the same candidates
  as the recorded samples (29 in total, identical counts per fixture).

Scoring, for every visual event (all kinds except `speech` and `malformed`):

- **hit**: some candidate's `representative_us` lies in the event's `[start, end)`;
- **timestamp error**: the first such candidate's time minus the event start;
- **false change**: a change candidate (`visual_change`, `motion_start`,
  `settled_after_motion`) whose `change_window` contains no event boundary;
- **gate**: every `stable` event of at least 1 s is hit, and the static fixtures F01
  and F07 have no change candidate. Other events are reported, not gated.

## Per-event results

| Event | Kind | Truth window (s) | Hit | Candidate | Timestamp error |
| --- | --- | --- | --- | --- | --- |
| F01-E01 | stable (gated) | 0-6 | yes | first frame | 0 |
| F02-E01 | stable (gated) | 0-4 | yes | first frame | 0 |
| F02-E02 | change | 4-8 | yes | visual change | 0 |
| F02-E03 | change | 8-12 | yes | visual change (and periodic at 10 s, same hash) | 0 |
| F03-E01 | stable (gated) | 0-4 | yes | first frame | 0 |
| F03-E02 | change | 4-9 | yes | visual change (the G18 cell edit) | 0 |
| F04-E01 | stable (gated) | 0-3 | yes | first frame | 0 |
| F04-E02 | scroll | 3-7 | no | corpus limitation | - |
| F04-E03 | stable (gated) | 7-10 | yes | visual change (1001 to 1017) | 0 |
| F04-E04 | change | 10-14 | yes | visual change (queued to failed) | 0 |
| F05-E01 | stable (gated) | 0-5 | yes | first frame | 0 |
| F05-E02 | change | 5-9 | no | corpus limitation | - |
| F05-E03 | stable (gated) | 9-20 | yes | visual change | 0 |
| F06-E01 | transient | 4.2-4.7 | yes | visual change, `transient` | 300 ms |
| F07-E01 | stable (gated) | 0-10 | yes | first frame | 0 |
| F09-E01 | change | 3.25-7.25 | yes | visual change | 250 ms |
| F10-E01 | stable (gated) | 5-9 | yes | visual change | 0 |
| F12-E01 | adversarial | 0-8 | yes | first frame | 0 |
| F12-E02 | stable (gated) | 8-12 | yes | periodic coverage (corpus limitation) | 2 s |

F08 has no visual event (its truth is speech only); it produced a first frame and one
periodic candidate.

## Summary

- **Gate:** passed. 10 of 10 gated stable events hit; 0 change candidates in F01 and F07.
- **Recall by kind:** stable 10/10, change 5/6, transient 1/1, adversarial 1/1, scroll 0/1.
  Both misses are corpus limitations.
- **Timestamp error:** median 0, maximum 2 s (F12-E02, hit only by its 10 s periodic
  sample). The two non-zero change errors are the 0.5 s sampling interval: F06's
  tooltip starts at 4.2 s and is first sampled at 4.5 s; F09's marker appears at
  3.25 s and is first sampled at 3.5 s. Each candidate's `change_window` brackets the
  true start.
- **False changes:** 0 (0 per minute).
- **Candidates per minute:** 12.9 over 135 s of media.
- **Throughput:** the eleven cold calls through the binary took 18.5 s for 135 s of
  media (7.3 media seconds per second, dominated by process start and the first call's
  preflight on these 6-20 s clips); 60 s windows of a 1440x900 20 fps clip ran at 29
  media seconds per second (PR 3); the 30-minute 640x360 10 fps session below at 67.
  Warm calls over the fixtures took 72-150 ms through the binary.

## Corpus limitations (issue #159)

Three events are drawn with exactly the pixels of the state before them by the P04
generator (`tools/generate_p04_fixtures.py`, `scene()`), so no visual method can see
them begin: F04-E02 (the scroll is not rendered; the screen keeps rows 1001-1008),
F05-E02 (the loading indicator is not drawn) and F12-E02 (hit only by periodic
coverage). The always-run test re-verifies from the recorded samples that each is
still pixel-identical to its predecessor. The truth is not changed; issue #159 tracks
regenerating the motion fixtures.

## Motion (V-03), clips built at run time

The corpus renders no scrolling, so the checkpoint builds two clips from a random cell
pattern with FFmpeg's `life` source, encoded with the native MPEG-4 Part 2 encoder:

| Clip | Motion | Stops | `settled_after_motion` | Candidates (bound) |
| --- | --- | --- | --- | --- |
| Scroll under a fixed 96 px header band | slow 100 px/s 3-7 s, fast 700 px/s 10-12 s | 7 s, 12 s | 7.28 s, 12.48 s | 5 (9): first frame, motion start 3.12 s, settled 7.28 s, motion start 10.40 s, settled 12.48 s |
| Zoom to 1.6x | 4-7 s | 7 s | 7.28 s | 4 (6): first frame, motion start 4.16 s, settled 7.28 s, periodic 10.40 s |

Every stop is followed within 0.5 s by an ordered `settled_after_motion` candidate,
and each motion collapses into one `motion_start` and one settled candidate; the static
header did not hide the scroll. Animation and overlapping cells are not covered.

## Lead/lag (V-04)

F03, F04, F05 and F09 speech variants, imported with a SubRip cue written at run time
from the frozen script over the generator's speech span: searching `127.50`, `1017`,
`E-409` and `marker beta`, the candidates within 10 s of each hit include one inside
F03-E02 (4 s), F04-E03 (7 s), F05-E03 (9 s) and F09-E01 (3.5 s). With local speech
recognition instead of the import (whisper.cpp v1.9.2, `base`), all four terms were
heard and the same candidates were found.

## Continuation, damaged media and cost (V-05, S-11)

- **Budget:** an engine limited to one window per call indexed a 180 s clip in three
  calls (`not_analyzed` gaps 2, 1, 0; index revisions 1-3); a fourth analysed nothing,
  and the binary then read the 47 candidates warm with no tool on `PATH` in 92 ms.
- **Damaged media:** F11's damaged tail damages only the audio, so its video is
  analysed completely (1 candidate, no gap); a video truncated at run time to 60 % of
  F05 is `partial` with one `undecodable` gap and no candidate, and is not decoded
  again; an audio-only file is `INVALID_ARGUMENT` with the no-video remediation and
  commits nothing.
- **S-11:** a 30-minute session (F02 looped, 640x360, 10 fps): the first call analysed
  30 windows in 26.9 s and reported the 31st (a sub-second tail) as `not_analyzed`; the
  second analysed it in 2.4 s. Warm pages through the binary, including process
  start: 480 candidates, p95 146 ms at limit 20 (24 pages) and 152 ms at limit 100
  (5 pages). In the engine, the largest index a session can hold (four hours, every
  window at its 32-candidate budget, a 2.2 MB record) pages at p95 101 ms (limit 20)
  and 103 ms (limit 100) (`engine_candidates`, `--release --ignored`). Target 250 ms.

## Not measured

Real screen recordings with heavier compression noise, cursor movement, animation,
overlapping cells, and changes shorter than the 0.5 s sampling interval that fall
between samples. The change thresholds are four to six times the measured noise of the
synthetic corpus; they are not calibrated against real recordings.
