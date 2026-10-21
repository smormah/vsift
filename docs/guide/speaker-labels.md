# Who said what: speaker labels

From `0.3.0`, VSift can label a transcript's segments by voice: `S1`, `S2` and so on. You can then read who said
each part, search one person's words, and record the names you know. This page says how, and what a label can and
cannot tell you.

## Label the voices

Speaker labels need the speaker-diarization tool and its two models. `setup install` installs them where VSift
manages its tools; you can also give their paths each time:

```console
vsift transcript speakers <session> --json
vsift transcript speakers <session> --speakers 2 --json
vsift transcript speakers <session> --diarizer <tool> --segmentation-model <model.onnx> --embedding-model <titanet.onnx> --json
```

Give `--speakers` when you know how many people speak; it is passed on as given. Without it, the tool decides.
On the hosted test machine it took about a twelfth of the recording's length: around five minutes for an hour; a slower computer takes longer. One run hears at
most two hours; for a longer recording, label a part at a time with `--from` and `--to` (in microseconds). Labels
of two parts are not linked: `S1` in one part need not be `S1` in the other.

Transcripts and search work without the tool: if it is missing, only `transcript speakers` fails, with
`MISSING_CAPABILITY` and what to install.

## Read and search by speaker

Once a transcript has labels, `transcript get` and `search` show them on every segment (`speaker_label`) and list
them once (`speakers`). To read one person's words only:

```console
vsift search <session> --query "invoice" --speaker S2 --json
```

## Record a name

A label says "the same voice", never who the person is. If you know who `S2` is, say so:

```console
vsift transcript name-speaker <session> --label S2 --name "Alice Example" --json
vsift transcript name-speaker <session> --label S2 --clear --json
```

The name is shown as your statement: `Alice Example (S2, named by the user)`. Search accepts it too
(`--speaker "Alice Example"`). VSift never guesses a name from a voice or from a name on the screen, and an agent
using the VSift skill records a name only when you give it.

## What to trust

- **Labels are provisional.** Two people with similar voices can end up under one label, and nothing in the answer
  shows that it happened ([L-207](../planning/known-limits.md#l-207)). If a label seems to answer itself, check it:
  listen to the segment's clip (`vsift audio`).
- **The precision is a segment.** Each segment gets one label, or none. A segment in which two people speak has
  **no confident speaker**; so does one where the tool heard little speech. The answer lists those segments.
- **The evidence is thin so far** ([L-206](../planning/known-limits.md#l-206)): synthetic conversations and a few
  real recordings. On two NASA recordings, a two-person call and a three-person interview, about 2 segments in 100
  got the wrong label.
- **Nothing about a voice is kept.** The tool works out what each voice sounds like in its own memory and forgets
  it when it ends. VSift keeps only the labels, their times and the names you recorded, in the session, which is
  removed when it expires unless you keep it.
- **Older versions cannot open it.** A release before `0.3.0` reads a session with speaker labels as damaged
  (`INTEGRITY_FAILURE`) and refuses all of it ([L-218](../planning/known-limits.md#l-218)).
