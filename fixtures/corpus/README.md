# VSift synthetic R0 corpus

This directory defines the public, rights-safe ground truth used to qualify the media
pipeline and agent workflow. `manifest.json` is the reviewed truth source;
`manifest.schema.json` validates its structure. P04 fixture media is generated from
deterministic project-owned recipes in `tools/generate_p04_fixtures.py`; generated
hashes and command provenance are recorded in `generated/provenance.json`. Run the
independent `tools/verify_p04_fixtures.py` before using generated media as evidence.

The manifest deliberately separates expected evidence from future selection code.
An implementation may not generate expected timestamps by asking its own algorithm
where events occurred. Changes to truth require review and a reason, especially when
a failing benchmark would become easier.

Rules:

- Use synthetic names, speech, screens and values. Never include real meetings,
  credentials, private source code or third-party media.
- Keep generation inputs editable and licence them under the repository's dual licence.
- Record actual generated-file hashes, generator version and FFmpeg invocation before
  using a fixture as release evidence.
- Treat F11 malformed variants as hostile. Generate/run them only in disposable,
  resource-limited test environments.
- F12 contains synthetic prompt-injection text; it is test data, never an instruction.
- Every expected event uses integer microseconds on the normalized presentation timeline.

P00 defines truth. P04 generates F01-F10/F12 visual media, F11 malformed variants,
tone-based audio sentinels and rotated/audio-track variants. The verifier independently
checks generated hashes, timestamps and selected decoded pixels. The source recipes
are reproducible with the same FFmpeg build; `generated/provenance.json` records that
build's version and hash. P07 adds speech artifacts matching the frozen scripts.
P08/P12 use the frozen corpus for measured retrieval and agent evaluation. See the
[P04 qualification record](../../docs/planning/p04-media-qualification.md).

## Supplied-transcript sidecars (P07)

`transcripts/F10.srt` and `transcripts/F10.vtt` are hand-written, rights-safe synthetic
sidecars for F10 under the repository licence. They are equivalent cue for cue and are
written 500 ms early, so the explicit offset `--transcript-offset 500000` places the cue
"Dialog R-17 is displayed now." exactly on F10-E01's frozen truth window
(5,000,000-9,000,000 us); the other two cues (one names the 500 millisecond offset) lie
inside F10's 12 s duration. They do not change `manifest.json`. The repository stores
them with LF line endings, which fixes their digests:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `transcripts/F10.srt` | 245 | `2a4ee37d826754eac43da03e7cc63f718b16aed4d3ce163c380ebd8ab9630847` |
| `transcripts/F10.vtt` | 346 | `daaeb39fde45b449e2204e6b9ef99f4428267a6e073052723c997f82560c3611` |

Small malformed T-01 variants live beside their tests in
`crates/vsift-infrastructure/tests/data/transcripts`; byte-level variants (encodings,
byte-order marks, line endings, control characters) are written inline in the tests.
