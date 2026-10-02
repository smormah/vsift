# Frozen v1 examples of the published 0.1.0

`examples/` holds every file of [`../../examples/`](../../examples/) exactly as the tag
`v0.1.0` (commit `011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa`, published 2026-10-01) published
it. **Never edit these files, and never add one.** They exist so that the current build can be
held to what a published release promised (P14 PR 2, evidence item RQ-04):

- `crates/vsift-contract/tests/published_compatibility.rs` validates each of them against the
  **current** v1 schemas (v1 is additive since 0.1.0, so an older document must stay valid), and
  proves this copy is the tag's bytes, file for file, when Git can read the tag
  (`VSIFT_REQUIRE_RELEASE_TAG=v0.1.0` makes a missing tag a failure; the `P14 compatibility`
  workflow sets it in a full checkout);
- `crates/vsift-infrastructure/tests/published_v0_1_0_records.rs` decodes the four stored session
  records among them (`bundle-transcript-record.json`, `bundle-transcript-record.asr.json`,
  `bundle-evidence-record.json`, `bundle-visual-index-record.json`) with the readers a newer
  build uses.

The live `../../examples/` may grow with the contract; these may not. A later release is frozen
the same way, in a folder of its own, when it is published.

To recreate this folder from the tag (only to check it; the test does the same by reading Git):

```console
git archive v0.1.0 schemas/v1/examples
```
