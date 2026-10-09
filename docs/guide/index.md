# VSift user guide

<!-- guide-version: 0.2.0 -->

VSift turns a video on your machine into **timestamped speech**, **the moments the screen changed** and
**the exact frames and audio that prove them**, each with an id you can cite. You use it yourself, from a
terminal, or you let an AI assistant on your machine use it. Nothing is uploaded.

This guide teaches you to do things with it. It is written for anyone with a video, not only for programmers.

**Checked against vsift 0.2.0**: the release `0.2.0` and the release candidate `0.2.0-rc.3` it was built from have
the same code, so this guide names the release, not the candidate. Where this guide says what a command prints, that
is real output of that build on practice recordings from the repository, and the examples are re-run whenever the
code changes ([how](#how-this-guide-is-kept-true)). The second candidate, `0.2.0-rc.2`, has the same commands and
options and a few fixes fewer (audio under a tenth of a second could reach the speech recogniser, an `audio` range of
31 microseconds or less was answered with seconds of audio, a copy that ran out of time was called an invalid video
with no explanation, and the agent skill lacked two evidence rules). The first candidate, `0.2.0-rc.1`, lacks two more
(a read of a session that overlaps a publish on Windows, and the answer for a video
over the size limit). The earlier pre-release, `0.1.0`, has the same commands and
options, but older wording in `vsift --help` and in a few error messages, none of the fixes to rare cases made since
(a named pipe or a link given as the video, too little room for the video, a short speech-recognition range, a
failed open), and one optional JSON member fewer (`missing_shared_library` in the result of `setup install`).
Everything else here applies to it.

## Where to start

| You are | Start with |
| --- | --- |
| A person with a recording and a question | [Your first investigation](first-investigation.md), then [investigate a recording](investigate-a-recording.md) |
| A person with subtitles or a transcript already | [Use an existing transcript](use-an-existing-transcript.md) |
| A person whose AI assistant will use VSift | [Let your agent investigate](let-your-agent-investigate.md), then [evidence and citations](evidence-and-citations.md) |
| Reading a report an assistant wrote | [Evidence and citations](evidence-and-citations.md) and [concepts](concepts.md) |
| A programmer or integrator | [The CLI contract](../contracts/cli-v1.md), the [JSON reference](reference/json.md) and the [command reference](reference/commands.md) |
| Running VSift for other people | [The worker-host runbook](../operations/worker-host.md) |
| Stuck on an error | [Troubleshooting](troubleshooting.md) |
| Wondering whether to trust it | [Limits](limits.md) and the [support matrix](../planning/support-and-resource-profiles.md) |

## Install it first

[The install guide](../operations/install.md) covers every route (npm, pnpm, Yarn, Bun or a download), getting
FFmpeg and FFprobe, the optional speech tools, upgrading and uninstalling. Install the release with
`npm install --global vsift-cli`; the `@next` tag is only for release candidates.

## All the pages

**Tutorial** (learn by doing)

- [Your first investigation](first-investigation.md): a 14-second practice recording, from nothing to a claim you can prove.

**How-to** (one task per page)

- [Investigate a recording](investigate-a-recording.md): the whole route for your own video.
- [Use an existing transcript](use-an-existing-transcript.md): subtitles you already have, and fixing their timing.
- [Keep and share evidence](keep-and-share-evidence.md): saving a bundle that outlives the session, and checking one.
- [Let your agent investigate](let-your-agent-investigate.md): setting up an AI assistant and reading its report.
- [Clean up and uninstall](clean-up-and-uninstall.md): closing sessions, removing what VSift stored, removing VSift.

**Concepts** (what the words mean)

- [Concepts](concepts.md): session, source, transcript, candidate, evidence, bundle, handoff, budget.
- [Evidence and citations](evidence-and-citations.md): how to write down what you found so others can check it.

**Reference** (complete, and generated from the code)

- [Command reference](reference/commands.md): every command and option, from `vsift --help`.
- [JSON reference](reference/json.md): every JSON document, from the v1 schemas.
- [The CLI contract](../contracts/cli-v1.md): what each command promises, for programmers.

**Help**

- [Troubleshooting](troubleshooting.md): every failure code and what to do.
- [FAQ](faq.md) and [limits](limits.md).

## How this guide is kept true

- **It describes only what the release has.** A feature that is planned appears in the planning documents,
  never here.
- **Its examples are real and re-run.** A check runs the marked commands against a freshly built `vsift` on the
  practice recordings and fails if what a page shows is not what the command prints. The values that change on
  every run (identifiers, times of day, hashes, the size of an image file) are compared by kind, not by value. A
  shortened output says so, with `...`.
- **The reference pages are generated** from `vsift --help` and the v1 schemas and cannot be edited by hand; a
  check fails when they are out of date.
- **It says which version it was checked against** (above). Another release gets its own check, and its own
  guide when its behaviour differs.
- **It follows the project's rules about what may be claimed.** Words that promise support or stability appear
  only where the project can show the evidence ([the support matrix](../planning/support-and-resource-profiles.md)).

If a page and VSift disagree, VSift is right and the page has a bug: please [say so](https://github.com/smormah/vsift/issues).
