# The README's prepared 0.3.0 sections

Status: prepared on 2026-10-30, **not public text yet**. The README shows what the published release has, so
each section below waits for its condition. Nothing here claims a feature: it is the text to paste once the
feature is merged, qualified and published. The change that pastes a section checks it against the claims
registry (`public-claims`) and rewrites it from what really shipped.

The README's shape does not change: the first screen, then badges and navigation, the demo, "Other ways to set
up", the deep-reading sections and "Help and reference".

| Section | Replaces in the README | Condition | State on 2026-10-30 |
| --- | --- | --- | --- |
| [The three commands](#the-three-commands) | The commands and the two sentences under `## Install` | `vsift setup` and `vsift <video>` are both in a published release | Both on `main`, neither qualified nor published |
| [With your AI assistant](#with-your-ai-assistant) | The first paragraph of "Use it with your AI assistant" | `vsift skill install` and the plugin are in a published release | On `main`, not published; no agent has tried them (L-242) |
| [Optional extras](#optional-extras) | Adds a section and a fourth route link beside "Without Node.js" | `--include` is in a published release | Not on `main` for every extra |
| [Speakers](#speakers) | Adds a row to "What you get" | The speakers story (#500) is merged and qualified | Not merged |
| [Speech on Windows](#speech-on-windows) | The sentence on what VSift installs for you | The Windows speech install is merged and qualified | Not merged |

If `vsift setup` misses the release, the README keeps `vsift setup check`. If `vsift <video>` misses it, the README
keeps `vsift ingest`. The two switch independently.

**At the switch, also:** record the demo and the walk-through again from the published release, with its version
in the label (`docs/assets/readme/README.md` says how), and redraw `roadmap.svg`.

**When the last section is pasted,** delete this page, or move it to `docs/history/` if something still points
at it. A section that did not ship goes with it: this page must not outlive its purpose.

## The three commands

````markdown
You need Node.js 22 or later.

```console
npm install --global vsift-cli
vsift setup
vsift ./your-video.mp4
```

`vsift setup` checks what your computer has and sets up video and speech for you. It shows what it would download and installs nothing until you say yes.
````

The route links under it gain "Optional extras". While the install on Windows and macOS is not in the release,
the second sentence names the systems it covers, as the sentence it replaces does.

## With your AI assistant

````markdown
VSift ships an **agent skill** that takes an AI assistant from a local video to a grounded, cited handoff: it knows the method, the evidence budgets and the safety rules, such as never installing tools on its own. One command installs it, from the bytes your `vsift` was built with:

```console
vsift skill install --client claude-code --client codex
```

Claude Code also takes it as a plugin. Pin the marketplace to the release you installed:

```text
/plugin marketplace add smormah/vsift#v0.3.0
/plugin install vsift@vsift
```

The plugin holds the skill and nothing that runs. Then ask about a recording:
````

Keep the example prompt and the paragraph of trial results, updated from the 0.3.0 batches. Name only the clients
the trials ran on as tried. The other `--client` values have their folders from each client's own documentation and
stay in [`docs/agents/skill.md`](../agents/skill.md), not on the front page, until a trial has used them (L-241).

## Optional extras

````markdown
### Optional extras

VSift works without these. Each adds one thing, and `vsift setup` lists them on every run:

- **On-screen text:** read and search the text shown in a recording.
- **Speakers:** label who said what.
- **Web videos:** open a public video page. Linux only for now.

Add one with `vsift setup --include on-screen-text`.
````

List only the extras the release has, with the names `vsift setup --include` really takes.

## Speakers

````markdown
| 🗣️ **Speakers** | Label who said what, locally, and cite it like any other evidence. |
````

Rewrite this from the speakers story's own limits. Say what was measured and on what.

## Speech on Windows

Today's sentence says VSift can install the tools on Ubuntu 24.04 x64. When the Windows speech install ships, widen
it to the systems that were tried. Say "qualified" only through a registered statement, and retire BAN-06 for the
platform in the same change.
