# Speaker labels: the listening sheet for RM06 and RM11 (0.3.0)

Status (2026-10-21, `0.3.0`, #108): **waiting for the maintainer.** About ten minutes. The speaker references of RM06
and RM11 were read from their transcripts, not heard ([`speaker-labels-evaluation.md`](speaker-labels-evaluation.md)
sections 5 and 6.2, [L-206](known-limits.md#l-206)). Listening to the moments below confirms them, or shows where they
are wrong. The checks are the speaker changes the scores depend on most: short turns, the first words after a change of
speaker, the segments the recommended engine got wrong, and the RM11 stretches where, told three speakers, the engine
merged Mission Control into the interviewer.

## How to listen

1. Open the file in any player and jump to each start time (minutes:seconds). Listen for about five seconds.
2. Decide who speaks **first** from that moment, by role, as the table describes it. Put one word in the last column:
   **right** (the person in the table), **wrong** (someone else; say who if you can) or **can't tell**.
3. At the end of each file, answer the two questions under its table.
4. Report back by pasting your ticks into the chat, one line per check, for example:

   ```text
   R06-1 right
   R11-3 wrong, it is the interviewer
   R11-q1 yes
   ```

Nothing needs to be installed or run: only a media player.

## RM06: Space Station Astronaut Discusses Record Breaking Mission (NASA, 10:24)

File: `C:\dev\vsift-real-media\RM06\RM06-nasa-iss-record-conversation.mp4`

The people: the **caller** on the ground (the previous record holder, who asks the questions), the **astronaut** on the
station (who answers), and a **controller** who closes the event in the last few seconds.

| Check | Start | Who the reference says speaks first | To find it | Right, wrong or can't tell |
| --- | --- | --- | --- | --- |
| R06-1 | 00:11 | the caller (a question after the astronaut's first words) | | |
| R06-2 | 00:17 | the astronaut (a reply of about two seconds) | | |
| R06-3 | 01:30 | the caller (after the astronaut's long answer) | | |
| R06-4 | 06:08 | the caller (a short question of about nine seconds) | | |
| R06-5 | 10:05 | the controller, closing the event (not the caller) | the closing words | |

- **R06-q1:** Is anyone heard who is none of these three? (yes or no; where?)
- **R06-q2:** At 00:00 to 00:03, who makes the call-up: the caller, the controller, or can't tell? (left unscored)

## RM11: Expedition 70 Astronaut Talks with KAKE-TV, Wichita, Kansas (NASA, 18:36)

File: `C:\dev\vsift-real-media\RM11\RM11-nasa-exp70-kake-interview.mp4`

The first nine and a half minutes are music; speech starts at 09:26. The people: **Mission Control** in Houston (opens
and closes the event), the **interviewer** at the television station on the ground (asks the questions), and the
**astronaut** on the station (answers). Mission Control and the interviewer are both on the ground side of the link:
told three speakers, the engine heard them as one, so checks R11-1, R11-3 and R11-7 matter most.

| Check | Start | Who the reference says speaks first | To find it | Right, wrong or can't tell |
| --- | --- | --- | --- | --- |
| R11-1 | 09:26 | Mission Control | "Station, this is Houston" | |
| R11-2 | 09:33 | the astronaut (the engine gave this reply an extra label) | "Houston, this is Station" | |
| R11-3 | 10:35 | Mission Control (not the interviewer) | "expect a comm check" | |
| R11-4 | 10:59 | the interviewer (the first words from the station on the ground) | "Station, this is ..." | |
| R11-5 | 11:07 | the astronaut | "welcome aboard" | |
| R11-6 | 14:52 | the interviewer, cutting in (the engine gave it an extra label) | "about your space walk" | |
| R11-7 | 18:19 | Mission Control, closing the event (not the interviewer) | after the astronaut's goodbye | |

- **R11-q1:** Do Mission Control and the interviewer sound like two different people to you? (yes, no or unsure)
- **R11-q2:** Is anyone heard who is none of these three, for example a second astronaut? (yes or no; where?)

## What happens with the answers

Every **right** confirms the reference as it is. A **wrong** or a "yes" to a question about another voice is corrected
in `tools/p16_speakers_rm06_reference.json` or `tools/p16_speakers_rm11_reference.json`, the item is scored again from
the logged turns, and the evaluation document and L-206 say what changed. When every check is answered, L-206 records
the references as heard.
