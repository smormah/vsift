# User guide: specification

Status: accepted into P14 scope by the maintainer on 2026-10-02. The R0 part is built in P14 PR 9b
(the second half of PR 9); everything later is built by the packet that ships the feature. This
page is the plan for the guide, not the guide: the guide is [`docs/guide/`](../guide/index.md).

## Why

Today VSift has a thorough installation guide, an agent skill guide, a CLI contract (the exact
reference) and a worker-host runbook. What it does not have is a place that teaches a person how
to *do things*: investigate a first recording, read what comes back, understand the words, and
fix the usual problems. That gap hurts newcomers most, and it widens with every feature R1 adds.
The guide is plain Markdown with relative links, so it can be moved or published elsewhere
without rework.

## Who reads it

| Reader | Needs |
| --- | --- |
| A person trying VSift on a recording | A first success quickly, then the meaning of what they see |
| A person whose agent uses VSift | What the agent is doing for them, what it will and will not do, and how to read its report |
| A programmer or integrator | The contract (already in `docs/contracts/cli-v1.md`), linked, not repeated |
| A supervisor running workers | The runbook (already in `docs/operations/worker-host.md`), linked, not repeated |

The agent skill (`skills/vsift`) stays the agent's own instructions and is not the human guide.

## Structure (organised by what the reader wants to do, never by release number)

`docs/guide/` (created by P14 PR 9):

| Part | Pages (R0) | Purpose |
| --- | --- | --- |
| **Start here** | `index.md` (what to read for what) | A route in for each reader above |
| **Tutorials** | `first-investigation.md` (a first recording, start to finish, with real output) | Learn by doing, on a small synthetic clip the reader can download from the repository |
| **How-to recipes** | `investigate-a-recording.md`, `use-an-existing-transcript.md`, `keep-and-share-evidence.md`, `let-your-agent-investigate.md`, `clean-up-and-uninstall.md` | One task per page, steps first |
| **Concepts** | `concepts.md` (session, source, transcript revision, evidence, visual candidate, handoff, budget, untrusted text) and `evidence-and-citations.md` | Plain-words meaning, with the one diagram each needs |
| **Reference** | `reference/commands.md` and `reference/json.md`, **generated** from `vsift --help` and the v1 schemas | Complete, always in step with the code; links to the contract |
| **Help** | `troubleshooting.md` (typed failures and their fixes), `faq.md`, `limits.md` (a plain-words view of the known limits that matter to a user) | The page people actually need when stuck |

R1 and later add pages under the same parts (for example a how-to for a managed catalogue), and
reference pages regenerate themselves. R2 is not defined in this repository; the structure
leaves room for it and the guide says nothing about it.

## Rules

1. **Write only what exists.** A page describes a command the published release has. A planned
   feature gets an outline in the planning documents (for example the R1 page list in
   `r1-industrial-capability-expansion.md`), never a public page.
2. **The claims ladder applies** (ADR 0024 decision G). The guide's pages are added to the
   public-claims registry's `documents` list when they are created, so the controlled words and
   banned phrases are checked in them as in the README.
3. **Examples are real and checked.** Every command and output shown is produced by a real run;
   a CI check runs the guide's commands against the published or just-built binary on the
   synthetic corpus and fails when the shown output no longer matches (the pattern is the
   frozen JSON examples' contract tests). Trimmed output is labelled as trimmed.
4. **The reference is generated**, never hand-edited: `vsift --help` for commands and options,
   the v1 schemas for the JSON. A CI check fails when the generated pages are stale.
5. **Every packet ships its guide pages.** A change that alters what a user sees or does updates
   the matching page in the same pull request; this is added to the definition of done in
   `implementation-work-packets.md`. P15 onward each list their guide pages in their own plan.
6. **Portable Markdown.** Plain Markdown, relative links, no GitHub-only syntax beyond what a
   static site generator also reads, images as files in `docs/assets/`. Choosing a documentation
   tool and publishing the guide anywhere else is not part of P14.
7. **Versioned when it matters.** While only one release line exists the guide describes the
   latest release and says which version it was checked against. A versioned guide arrives with
   the first release whose behaviour differs from its predecessor.
8. **Plain words first.** Short sentences, the reader's task as the title, the result before the
   explanation, a plain-words meaning for every term at first use.

## What P14 PR 9 delivers

The structure above with `index.md`, the tutorial, `concepts.md`, `troubleshooting.md`, `faq.md`
and the generated reference; the recipes may follow in the same pull request or the next, as the
effort allows (the tutorial, concepts and troubleshooting come first). The two CI checks (examples
and generated reference) and the registry entries ship with it. Evidence is recorded under RQ-18
(documents and capabilities agree) and the walked guides of RQ-01 to RQ-04; this adds no evidence
item and no requirement.

**What PR 9b built (2026-10-04).** All the pages of the table above, the generated reference
included, in the pull request; plus `files/` (the practice transcript and two small files the
examples use). The two checks are `tools/guide/generate-reference.cjs` (`--write` and `--check`)
and `tools/guide/check-examples.cjs`, run by the `Guide` workflow; they are described in
[`development.md`](../development.md#the-user-guide-and-its-checks). Where the pages differ from
this plan:

- The `limits.md` page lists the entries of the register that matter to a user; the register
  stays the full record.
- The guide names a **release** (`0.2.0`), not a candidate, so that the stable commit needs no
  change to it (release process 6.8).
- Rule 3's "published or just-built binary" is the just-built one: the guide is checked against
  the source, and its first page says what the published pre-release lacks.
- Nothing in the guide is generated from the contract: the contract stays the exact reference
  and the guide links to it.

## Not in P14

A documentation site tool or hosting; versioned guides; translations; R1 and later pages;
video or animated walkthroughs beyond the README's; any page promoting the project.
