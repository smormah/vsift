# Value measurement, phase 1: the method (M3)

Written 7 Nov 2026 for the planner to review, and accepted with the rulings of section 12. This is
the public copy, frozen by SHA-256 with the value measurement's harness
([`docs/agents/trials.md`](../../agents/trials.md), "The value measurement"): private paths and
session identities are removed, and nothing else is changed except the dated note in 12.3. M1 to M6
are the work items of the plan; "the planner" is the project's design reviewer. Nothing here has
run, and no result is claimed.

## 1. The question and the gate

Does VSift 0.3.0 make an AI assistant's work on a recording easier, cheaper or more correct than the
same assistant driving FFmpeg and whisper.cpp itself, and where does it not?

- **It gates the public launch** (the maintainer, 6 Nov: "Yes, it gates launch").
- **A result against VSift trims the claims** (6 Nov: "Trim claims"):
  - an efficiency loss means the launch goes ahead with no efficiency claim;
  - a correctness loss on the long or visual tasks holds the launch until fixed;
  - a tie or loss on the short speech-only clip blocks nothing and is said openly.
- **Run on 0.3.0-rc.1**, not on `main`.
- **The skill is trimmed first** (M5a, and M5b if it fits), and passes the existing skill trials
  again before phase 1, so the measured skill is the one that ships.

## 2. Hypotheses, fixed before any run

| | Hypothesis | Threshold that counts for it (standing as proposed, 6 Nov) |
| --- | --- | --- |
| H1 | Fewer round trips on long or visual tasks, at equal or better correctness | at least 30% fewer than R1 on T4 to T6 |
| H2 | Fewer tokens on the same tasks; a tie or worse on short speech-only | at least 20% fewer than R1 on T4 to T6 |
| H3 | Fewer wrong times and missed moments | V1 beats R1 on T2 and T3 correctness; equal means the claim is dropped |
| H4 | A second question costs much less | T7 at least 40% cheaper with VSift than R1's second question |
| H5 | A smaller model with VSift reaches the larger model's raw correctness | phase 2 only; no claim unless met |
| H6 | Checkability | shown, not scored (section 8) |

Short clip (T1): if V1 costs more than 1.5 times R1, the README says plainly that a short clip
doesn't need VSift, and the skill's size is a fix to make.

Three runs a cell gives a signal, not statistics. Public text says "in our trials", names the models
and links the record.

## 3. Arms

Every arm gets the same video, the same question, the same report requirement (section 6), the same
budgets, and FFmpeg, FFprobe, whisper-cli and the reviewed model on `PATH`. Only what is added differs.

| Arm | Added | Purpose |
| --- | --- | --- |
| R0 raw, cold | nothing; a neutral prompt naming the tools | what an assistant does unaided (the trigger of this work) |
| R1 raw, guided | the raw method guide (section 4), placed where the client reads skills | **the strong baseline**; the comparison that counts |
| V0 VSift, cold | VSift on `PATH`, no skill (the existing cold mode) | VSift's own help and errors alone |
| V1 VSift, skill | VSift and the trimmed skill (the existing skill mode) | VSift as shipped |
| V2 (phase 2) | V0 or V1 plus macro prototypes | the macros, and H5 with a smaller model |

The arms were proposed on 6 Nov and carried by the maintainer's go on the gate; they were not
confirmed one by one. In V0 and V1 the assistant may still call FFmpeg directly; that is allowed and
recorded.

## 4. The raw method guide (R1): rules for writing it

It must make the raw route win where it can. A weak guide makes every VSift win worthless.

- **Who:** not the author of VSift's skill; the maintainer reads it before the freeze. About as long
  as the trimmed skill (within 25%), so length is not the difference.
- **What it must teach, at least:**
  - extracting 16 kHz mono PCM for whisper-cli, and writing SRT and JSON with timestamps;
  - frame-accurate seeking (output seeking, or input seeking with decoding), reading `pts_time` with
    FFprobe, the container's start time, and variable frame rate;
  - finding screen changes with the `select` scene score at thresholds suited to screen recordings
    (lower than for film), with `showinfo` for the times;
  - **contact sheets** (`tile`), so that many frames cost one image, and `crop` for small text;
  - keeping every output in one work folder, named by time, and reusing it for a follow-up question;
  - checking the audio offset against the picture where speech and screen disagree;
  - writing the report with the time and the file behind each statement.
- **What it must not do:** install anything, use a network, or contain any truth about the test
  videos.
- It is frozen with the run (its SHA-256 in every record), like the skill.

## 5. Tasks (public recordings only)

| Task | Recording | Kind | Question source |
| --- | --- | --- | --- |
| T1 | F05 (synthetic corpus) | short, speech only | corpus truth (F05-E03) |
| T2 | F04 or F06 (corpus) | short, visual change (the 1017 row; the 500 ms tooltip) | corpus truth |
| T3 | F09 (corpus) | timing trap: lead/lag, variable frame rate, audio offset | corpus truth |
| T4 | RM09, 28 min | long speech, silence first, outdoor noise | M2 labels plus NASA's captions |
| T5 | RM10, 3½ min | screen recording, small text | M2 labels |
| T6 | RM07, 9½ min | talk with slides and a demo | M2 labels plus the Commons captions |
| T7 | RM09 and RM10 again | a second question in a fresh context, the first run's folder kept | M2 labels |

- **Transcripts:** T4 to T6 get a supplied transcript in every arm (the same SRT), so whisper's CPU
  time does not swamp the comparison; one T4 variant transcribes locally in every arm. Speech
  questions are not asked where the supplied captions hold the answer word for word; those tasks ask
  about the screen or about time.
- **Labels (M2):** two testers mark the timeline independently in a labelling page kept
  outside the repository; a label stands where they agree within 1 s, and the
  maintainer settles the rest. M2's session writes two questions per real item from the agreed labels
  and the captions, each with a truth window and key facts, and lands them by a reviewed change.
  Testers appear as "A" and "B" only, everywhere.
- **No load:** long items run where the trials run today; no load runs on the dev machine.

## 6. The report every arm writes

A plain answer, in which each statement about when something happens gives a time, and the file,
frame or evidence id it rests on. The VSift handoff block is **not** required in the comparison; it is
measured apart (section 8), so that its cost is not charged to V0 and V1.

## 7. Measures and grading

Per run, from the event stream (existing unless marked new):

- round trips (each tool call was its own step in all 254 recorded runs);
- input, cached, cache-write and output tokens, and the client's cost estimate;
- images opened and their bytes (Claude Code only; L-075);
- wall time, split into tool time and model time (**new**);
- failed or retried calls; safety (canaries, installs, network) as today;
- for T7, the second phase against the first.

The cross-arm grader (**new**, M1) reads the report's prose:

- **Timed claims:** each sentence with a time (`mm:ss`, `h:mm:ss`, seconds) is a claim.
- **Key fact matched:** the existing truth matcher (`truth.rs`), on claims whose time lies in the
  fact's window (corpus tolerances; real items ±1 s, the labels' agreement).
- **Wrong claim:** a time outside every window for the fact it states, or a stated fact the truth
  contradicts.
- **Unverifiable claim:** a statement about the recording with no time and no file, frame or id
  behind it.
- A person reads every run whose mechanical grade and prose disagree, as today.

## 8. Checkability (H6), shown apart

On T2 and T5, one VSift report and one R1 report each get one invented statement with a citation
(an id, or a frame file, that does not exist). `vsift handoff check` on the VSift report must refuse
it; for the raw report, record what a script could check at all. This is a demonstration for the
README's "Evidence with a name" line, not a score.

## 9. Runs and cost

| Phase | Client and model | Arms x tasks x runs | Runs | Estimate |
| --- | --- | --- | --- | --- |
| 1 | Claude Code 2.1.284, Sonnet 5.5 | 4 x 7 x 3 | 84 | about $30 to $50 by the client's estimate; 6 to 10 h unattended |
| 1, check | Opus 5.5 | R1 and V1 x T2, T5, T7 x 3 | 18 | about $20 to $30 |
| 1, optional | Codex (pinned image) | as phase 1 | about 84 | tokens only; no cost figure |
| 2 | Sonnet 5.5 and a smaller model | V2 against V1 and R1 on the macro tasks | about 30 | after phase 1 |

Basis: the recorded medians (skill runs $0.31 to $0.63, cold runs $0.03 to $0.29), with long real
items guessed at three to five times a short run. The pilots of each new arm record real usage before
the counted runs; the planner reads them before the go.

## 10. Order of work

1. M5a and M5b: trim the skill (and the finishing command, with its ADR amendment and contract tests,
   if it fits the cut); the skill trials again.
2. M2: the testers' labels; the maintainer settles disagreements; the questions written and landed.
3. M1: the raw arm and its guide (section 4), the cross-arm grader, the two-phase repeat mode, and the
   checkability demonstration; dry runs and pilots.
4. M4: phase 1 on 0.3.0-rc.1, with the maintainer's go per batch; the reading; the gate decision
   against section 2.
5. The README section ("Why not just use FFmpeg and whisper.cpp?", agreed 6 Nov) lands with the
   launch, plus the numbers sentence only for measures that passed.
6. M6: macros chosen from the phase 1 chains, for 1.0.

## 11. What this method cannot show

- Two clients at most, so public text never says "any AI" or "any agent".
- Real items are few and short of truth; the corpus items are synthetic.
- Usage figures are the clients' own (L-120), and Claude Code's and Codex's are not comparable.
- A guide written to make the raw route win is still one guide; a better one could close a gap.
- The macros (phase 2) are prototypes until they ship.

## 12. The planner's review, 7 Nov 2026: accepted, with these rulings (binding on M1 and M4)

1. **Fixed before any run.** This note and the raw guide are frozen by SHA-256 before the first counted run, and every record names both hashes. No counted run is repeated to improve a result. A run is excluded only for an infrastructure failure, such as the client failing to start, a runner lost or a quota hit, and only with a written reason in the record. Every excluded run is listed in the reading.
2. **"Equal correctness" and "a correctness loss" are defined now,** so that the gate is mechanical:
   - per task and arm, the score is the mean share of key facts matched over the 3 runs, together with the total of wrong claims;
   - **equal or better** (H1, H2): V1's score is at least R1's minus 0.10, and V1 has no more wrong claims than R1;
   - **a correctness loss** (the launch hold): on any of T2 or T4 to T6, V1's score is more than 0.10 below R1's, or V1 has more wrong claims than R1 in at least 2 of the 3 run pairs. A loss is read by a person against the runs before the hold is declared.
3. **Who writes the raw guide (R1).** A fresh session writes it, briefed only to write the best possible guide for an assistant that answers questions about recordings with FFmpeg, FFprobe and whisper.cpp. It sees the task kinds in section 5 but never the test videos or their truth, and nothing of VSift's skill beyond its length. A second fresh session then reviews it for missing techniques, and the gaps it finds are fixed. The maintainer reads it before the freeze. The guide must also teach how to keep and reuse a work folder for T7, so that the second-question saving isn't won by default.

   *Amended by the planner, 7 Nov 2026, in the ruling on M1's design:* the length rule of section 4 becomes an upper bound only. The guide may hold at most 1.25 times the words a typical V1 run loads after the trim (`SKILL.md` plus the references every run reads: `safety.md`, `handoff.md`, `commands.md` and `lifecycle.md`). There is no lower bound, and the guide may be one main file plus files read on demand.
4. **Equal conditions.** All arms have the same time and turn budgets, the same client settings and the same machine image. In R0 and R1, `vsift` is not on `PATH` and its skill isn't installed. V0 and V1 have FFmpeg and whisper-cli available, and each such call is recorded.
5. **Reported, not gated.** V0 against R0 is reported beside the headline V1 against R1, since it shows VSift's own help without the skill. Unverifiable claims are reported, not gated.
6. **Pilots before the go.** Each new arm gets 1 pilot run on T2 and T5, and the pilots' real usage replaces section 9's estimate before the maintainer's go on M4. If the estimate rises above $100, the planner puts it to the maintainer.
7. **Order and dates.** M5a and M1 are needed before `0.3.0-rc.1`, and M2's labels before M4. The planner briefs M5a, M5b and M1 from this note. M1 (Opus) includes the raw guide's two fresh sessions as its first step, run by the coordinator, never by M1 itself.
