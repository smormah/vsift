# Frequently asked questions

Short answers, with a link to where the full one lives.

## Basics

**What does VSift do?** It reads a video on your machine and gives you what was said (a transcript with times),
the moments the picture changed, and the exact frames, crops and audio clips behind them, each with an id you
can cite. [Concepts](concepts.md) explains the words.

**Do I need an AI assistant?** No. Everything works from the command line by hand, as in
[your first investigation](first-investigation.md). An assistant that can run commands can do the same steps
for you ([let your agent investigate](let-your-agent-investigate.md)).

**Is my video uploaded anywhere?** Not by VSift: it works on your machine and talks to no server while you use it.
(`vsift setup install` on Ubuntu 24.04 downloads the reviewed tools, after you accept a plan, and nothing else
downloads anything.) If you give VSift's output to an AI assistant, whatever the assistant does with it, including
sending it to its model, is up to that assistant.

**Does it need the internet?** To install VSift and the tools, yes. To use it, no.

**Is it free?** Yes. VSift is open source under either the MIT or the Apache 2.0 licence, as you choose.

## Videos and transcripts

**Which videos does it open?** MP4, MOV, Matroska and WebM files with common codecs, up to 4 hours and 20 GiB.
An AVI or FLV file is refused: re-save it. The details are in [limits](limits.md).

**Which languages?** The speech model is multilingual, but VSift's tests used English and one Spanish sentence, so
treat anything else as untried ([limits](limits.md)).

**How accurate is the transcript?** On clean synthetic speech about 3% of the words were wrong; on a noisy test clip
more than half. Nobody has measured real voices. Always check numbers and names against the audio
([limits](limits.md#the-words-speech-recognition)).

**Can I use subtitles I already have?** Yes, SRT or WebVTT: [use an existing transcript](use-an-existing-transcript.md).

**Why are times in microseconds?** So that a time is one whole number with no rounding. Seconds times 1,000,000:
10 seconds is `10000000`. The output also prints them as `00:00:10.000000`.

**Can it read text that appears on the screen?** No. VSift gives you the frame, and a crop at native size for small
text, and you (or an assistant that can open images) read it. It does not do character recognition.

**Can it find a person, an object or a logo?** No.

## Using it

**How do I get JSON for a program?** Add `--json` to any command: one result on standard output. `--events jsonl`
gives a stream of events. The shape is versioned and only grows: [JSON reference](reference/json.md) and the
[contract](../contracts/cli-v1.md). The readable text without those options may change in any release, so a program
should not parse it.

**What do the exit codes mean?** `0` worked; `1` a bug; `2` a wrong command line or a missing tool; `3` an unusable
video or transcript; `4` busy, try again; `5` a limit; `6` stopped; `7` a file, disk or download problem.
[Troubleshooting](troubleshooting.md) has every code.

**Why does `session list` say "More on the next page" when there is nothing more?** It walks VSift's folder in
fixed slices and a slice can be empty. Keep repeating the command with the number it gives until the line goes away.

**Why does the output keep saying the text is untrusted?** Because it is: the words in a recording can contain
instructions aimed at an assistant. VSift labels them so nobody mistakes them for yours
([evidence and citations](evidence-and-citations.md)).

**How long does it keep my files?** A session expires 24 hours after it is opened (renewable up to seven days) and
its files are removed by a cleanup you run. A bundle you retain stays until you delete it
([clean up and uninstall](clean-up-and-uninstall.md), [keep and share evidence](keep-and-share-evidence.md)).

**Can I run it on a server?** There is a worker mode for a supervisor that feeds it requests, with a runbook. It is
a target that has been tried on one kind of machine, not something to rely on yet: [worker-host runbook](../operations/worker-host.md).

## Trust and problems

**Windows or macOS warns me about the file.** The programs are not signed. The [install guide](../operations/install.md#4-windows-and-macos-smartscreen-gatekeeper-and-what-to-check-instead)
says what the warnings mean and how to check a download instead of trusting its name.

**Where do I report a problem or ask a question?** [SUPPORT.md](../../SUPPORT.md). A suspected security problem goes
through [SECURITY.md](../../SECURITY.md), not a public issue.

**Which version does this guide describe?** The one named on the [first page](index.md). It is checked against the
code every time the code changes.
