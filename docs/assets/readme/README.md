# README graphics

The pictures on the project's front page. Each SVG is self-contained (no external fonts,
images or scripts), so GitHub can show it through `<img>` and it can be reused anywhere
without its surroundings. Each one carries its own dark card, so it reads the same in light
and dark themes, and a `<title>` and `<desc>` for screen readers.

| File | What it shows | Source of truth |
| --- | --- | --- |
| `hero.svg` | Logo, wordmark, tagline; three frames pass through a sieve and come out as cited evidence | The real session below |
| `logo.svg` | The mark alone, 128 px | — |
| `demo-terminal.svg` | Animated terminal (CSS keyframes, 24 s loop) replaying the session | Real output of `vsift-cli@0.1.0`, trimmed |
| `evidence-timeline.svg` | 0–14 s timeline: speech, screen changes and the three real frames | The real session; the frames embedded as 480 px PNG |
| `pipeline.svg` | From video to cited handoff, with the "on your machine" boundary | `docs/architecture.md` |
| `before-after.svg` | Doing it by hand against doing it with VSift | — |
| `architecture.svg` | The five crates, the contract crate and the external processes | `AGENTS.md`, `Cargo.toml` |
| `roadmap.svg` | Done, now and next | `delivery-ledger.json`, ADR 0024, the R1 plan |
| `frame-*.png` | The three frames `frame get` returned, at full size | The real session |

**The real session.** `vsift-cli@0.1.0` from npm, on `fixtures/corpus/generated/F04-speech.mp4`
(14 s, synthetic), local ASR with whisper.cpp `base`: session
`ses_1179db5b0e627c16e2fa26d0a8892e81`, speech segment `tsg_064d9d71…` (0.000–7.000 s, 82.20%),
visual candidates at 0, 7 and 10 s, frames `evd_de565174…`, `evd_fb4171c4…` and `evd_efa4af05…`.
Anything labelled as output must stay real output; trimmed output says it is trimmed.

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
- **Redraw `roadmap.svg` when a rung moves:** with the first release candidate (P14 PR 10) and when
  P14 completes (PR 13).
