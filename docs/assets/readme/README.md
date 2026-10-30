# README graphics

The pictures on the project's front page. Each SVG is self-contained (no external fonts,
images or scripts), so GitHub can show it through `<img>` and it can be reused anywhere
without its surroundings. Each one carries its own dark card, so it reads the same in light
and dark themes, and a `<title>` and `<desc>` for screen readers.

| File | What it shows | Source of truth |
| --- | --- | --- |
| `hero.svg` | Logo, wordmark, the category (a video evidence engine); three frames pass through a sieve and come out as cited evidence | The real session below |
| `logo.svg` | The mark alone, 128 px | — |
| `demo-terminal.svg` | Animated terminal (CSS keyframes, about 31 s loop) replaying the session: ingest, retranscribe, search, candidates, frame get, then a draft with a made-up citation refused by `handoff check` and the report without it passing | Real output of the build named below, trimmed |
| `evidence-timeline.svg` | 0–14 s timeline: speech, screen changes and the three real frames | The real session; the frames embedded as 480 px PNG |
| `pipeline.svg` | From video to cited handoff, with the "on your machine" boundary | `docs/architecture.md` |
| `before-after.svg` | Doing it by hand against doing it with VSift | — |
| `architecture.svg` | The five crates, the contract crate and the external processes | `AGENTS.md`, `Cargo.toml` |
| `roadmap.svg` | R0 done, the 0.2.x line (0.2.3 now, 0.2.4 next), 0.3.0, 1.0.0 and R1 planned | `delivery-ledger.json`, ADR 0024, ADR 0027, `backlog-iterations.md` (the release calendar), the R1 plan |
| `frame-*.png` | The three frames `frame get` returned, at full size | The real session |
| `crop-10s-failed.png` | The 480x110 crop of the `1017 FAILED` row that `crop` returned | The real session |

**The real session.** A build of `main` at commit `8a35d1f4` (it reports `0.2.3-rc.1`), with its
configuration and session folders isolated in a temporary folder, on
`fixtures/corpus/generated/F04-speech.mp4` (14 s, synthetic; shown as `walkthrough.mp4`), FFmpeg 9.0
and local ASR with whisper.cpp v1.9.5 `base`: session `ses_477beb509a9ca0dca8ce010caef54bdb`,
revision `trv_ee2e0040…`, speech segment `tsg_2d12cbeb…` (0.000–7.000 s, 82.20%), visual candidates
`vcd_39f16b3a…`, `vcd_822b22cc…` and `vcd_d80c7914…` at 0, 7 and 10 s, frames `evd_67a452fe…`,
`evd_4c3284fd…` and `evd_2243ab20…`, crop `evd_4191e17c…` (`--rect 120,280,480,110`). The three
frames are byte for byte the ones `vsift-cli@0.1.0` returned (same SHA-256), so the PNG files did
not change; the ids did. Every command shown is in the published `0.2.3`; the output comes from
this build because the README's output is recorded again from the published release at the 0.3.0
switch ([the prepared sections](../../planning/readme-0.3.0-sections.md)).

**The made-up claim.** `draft.md` and `report.md` were written for the demo, against the session
above. Both make the same three claims about the segment and the frames at 7 and 10 s; `draft.md`
also says that at 00:12 order 1017 is retried and shows COMPLETED, citing
`evd_7f3a9c1e5b2d4860a1c7e93f0d6b2a54`, an id no command returned. Both refusal and pass are real
output of `handoff check`. Its wording must stay what happened: a claim with no evidence behind it
is refused. It never detects a false statement, and a false sentence that cites real evidence
passes (BAN-09).

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
- **Redraw `roadmap.svg` when "now" moves:** when a release of the 0.2.x line is published (move
  the NOW marker and the done line), and when R1 starts. Its text dates by design; the other
  graphics should not.
