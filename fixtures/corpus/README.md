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
