# README graphics

The pictures on the project's front page. Each SVG is self-contained (no external fonts,
images or scripts), so GitHub can show it through `<img>` and it can be reused anywhere
without its surroundings. Each one carries its own dark card, so it reads the same in light
and dark themes, and a `<title>` and `<desc>` for screen readers.

| File | What it shows | Source of truth |
| --- | --- | --- |
| `hero.svg` | Logo, wordmark, tagline; three frames pass through a sieve and come out as cited evidence | The real session below |
| `logo.svg` | The mark alone, 128 px | — |
| `demo-terminal.svg` | Animated terminal (CSS keyframes, about 36 s loop) replaying the session: ingest, retranscribe, search (text and `--json --view brief`), candidates, frame get, crop, handoff check | Real output of `vsift-cli@0.2.1`, trimmed |
| `evidence-timeline.svg` | 0–14 s timeline: speech, screen changes and the three real frames | The real session; the frames embedded as 480 px PNG |
| `pipeline.svg` | From video to cited handoff, with the "on your machine" boundary | `docs/architecture.md` |
| `before-after.svg` | Doing it by hand against doing it with VSift | — |
| `architecture.svg` | The five crates, the contract crate and the external processes | `AGENTS.md`, `Cargo.toml` |
| `roadmap.svg` | R0 done, the 0.2.x line (0.2.1 now, 0.2.2 and the re-pins next) and R1 planned | `delivery-ledger.json`, ADR 0024, `backlog-iterations.md`, the R1 plan |
| `frame-*.png` | The three frames `frame get` returned, at full size | The real session |
| `crop-10s-failed.png` | The 480x110 crop of the `1017 FAILED` row that `crop` returned | The real session |

**The real session.** `vsift-cli@0.2.1` from npm, installed into a temporary prefix with its
configuration and session folders isolated in a short temporary folder, on
`fixtures/corpus/generated/F04-speech.mp4` (14 s, synthetic; shown as `walkthrough.mp4`), local ASR
with whisper.cpp v1.9.2 `base`: session `ses_b7ff4f4f5de12f4af02f9fa86823436a`, revision
`trv_e094613c…`, speech segment `tsg_f5a4c0cb…` (0.000–7.000 s, 82.20%), visual candidates
`vcd_4d4de712…`, `vcd_2be29f4f…` and `vcd_6d9d6bdc…` at 0, 7 and 10 s, frames `evd_1fc0339d…`,
`evd_901de99c…` and `evd_dac79c65…`, crop `evd_8e029895…` (`--rect 120,280,480,110`). The three
frames are byte for byte the ones `vsift-cli@0.1.0` returned (same SHA-256), so the PNG files did
not change; the ids did. Anything labelled as output must stay real output; trimmed output says it
is trimmed.

**Palette and type.** Navy `#0e1a2b` (background), `#13233a` and `#172a44` (cards), `#25405f`
(lines), header blue `#2a4a73` (from the frames), text `#e8eef6`, muted `#9fb2c8`, dim `#62799a`;
teal `#2dd4bf` (VSift, speech, done), amber `#f5b942` (screen changes, now), red `#e5535a`
(the FAILED status), violet `#a78bfa` (the agent, outer layers). Sans: the system UI stack;
mono: `ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`. Animations stop under
`prefers-reduced-motion`.

**Rules for any edit.**

- The words in these files are public text, and since P14 PR 9a the claims check reads them (the
  text, title and description elements; `public-claims` fails on a controlled word or a banned
  phrase, like a document). It cannot see a stale roadmap (L-121). Repeating this hand check as well
  costs nothing, and must print nothing but the CSS keyframe percentages:

  ```console
  grep -o -i -E "support|stable|qualified|qualif(y|ies)|certified|guarantee|production|strict|hostile|tenant|publisher|signed|filesystem|managed install|codex|any model|any coding|any llm|100%" docs/assets/readme/*.svg | sort | uniq -c
  ```

- No user names, drive letters, profile paths or email; session ids and hashes are fine. Frames
  come from the synthetic corpus only.
- Keep each file well under 150 KB, and keep words that date quickly out of the graphics, except
  the roadmap.
- **Redraw `roadmap.svg` when "now" moves:** when a release of the 0.2.x line is published (move
  the NOW marker and the done line), and when R1 starts. Its text dates by design; the other
  graphics should not.
