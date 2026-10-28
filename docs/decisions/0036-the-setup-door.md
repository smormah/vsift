# ADR 0036: The setup door: plain `vsift setup`, attended by default, unattended on request

- Status: **Proposed, 2026-10-27; the design accepted by the planner the same day, and decision 1 (the unattended
  acceptance) ratified by the maintainer the same day**, superseding that part of ADR 0023 (section 6). The shape of
  the door (sections 1 to 8) is the maintainer's design of 2026-10-27. The outcome of each open decision is under
  "Decisions for the planner and the maintainer". It is built in two pull requests (section 12): the first (#537)
  is merged; the second builds sections 4 to 6 (the implementation notes below say where each differs in detail).
- Date: 2026-10-27
- Supersedes: the part of [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) that accepts a
  managed plan only by its quoted digest, for the unattended setup door alone (section 6, the maintainer's ratification
  of 2026-10-27). `setup install` keeps that rule.
- Refines: [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) (a plan is
  installed only with its digest quoted back), [ADR 0032](0032-managed-installs-on-every-platform-and-the-engine-release-workflow.md)
  (managed installs and optional components), [ADR 0008](0008-cli-and-json-contract.md) (bare
  `vsift setup` prints help) and [ADR 0022](0022-agent-skill-and-named-client-qualification.md)
  (the skill's command policy). Their text stays as accepted; section 10 says what this record
  changes in each.
- Tracking: the 0.3.0 story W6f; the register entry L-233 if a limitation remains.

## Context

A person who has just installed VSift with npm has to reach "video and speech both work" before
the first video. Today that takes the installation guide and up to four commands: `setup check`,
`setup plan --json > plan.json`, reading the digest out of that file, then `setup install --plan
plan.json --accept-plan <digest>`. Those commands were designed for an AI assistant and a script,
and they stay. A person needs one door that checks first, says what is done and what isn't, and
walks them through the rest.

What exists (checked on 2026-10-27 in `main` at `8a3926ea`):

- **The readable `setup check` (W6e, #536)**: a table of video, speech and the extras, one line on
  where the person stands, and one `Next:` line (`crates/vsift-cli/src/human/setup_check.rs`).
  "Ready" means the work can be done; "complete" only ever means video and speech both work. Its
  `Next` type was left for this record to replace.
- **Managed installs (ADR 0023, ADR 0032)**: `Engine::plan_setup` and `Engine::install_setup`. The
  transaction takes the managed root's install guard, which never waits (a held guard is `BUSY`),
  rebuilds the plan, requires it to equal the saved one and the quoted digest to accept it, then
  installs the components one at a time in plan order: media tools, the whisper.cpp program, the
  model. The first failure stops it, and the components after it are `blocked`, never fetched
  (`crates/vsift-application/src/install.rs`).
- **The speech test needs video.** The install's speech smoke decodes its fixture with FFmpeg: the
  staged one, or the one already selected (`SpeechSmokeRequest::tools` in
  `crates/vsift-infrastructure/src/managed_smoke.rs`). So speech can be downloaded and verified
  without video, but not tested or activated.
- **Bare `vsift setup` prints setup help** and probes nothing (`cli-v1.md`, ADR 0008), so nobody
  loses anything when it becomes the door.
- **Nothing in VSift prompts today**, and nothing reads standard input as an answer. Progress is
  shown only by the live line of ADR 0033 (standard error, terminal only) and by `--events jsonl`.
- **Colour.** Nothing in VSift colours its output. `anstream` and `anstyle` are already in
  `Cargo.lock` through `clap`.

## Decision

### 1. The command

`vsift setup` with no subcommand is the door. Every subcommand (check, plan, install, configure,
configure-model, list, repair, rollback, remove) keeps its behaviour and JSON. `vsift setup --help`
still prints the setup help, which now leads with the door. The door's own options:

| Option | Meaning |
| --- | --- |
| `--include <extra>` | Add an extra, by its person-facing name. Repeatable. In 0.3.0 the only accepted name is `web-videos` (today's `online_links`). `on-screen-text` and `speakers` are accepted once their catalogue groups exist (W5b, W5c); adding a name is additive. |
| `--unattended` | Nobody is there to answer: run once from the parameters below and report pass, fail or error (section 6). |
| `--install <part>[,<part>]` | With `--unattended` only: the core parts to install, `video`, `speech` or both. Naming them is the acceptance (section 6). |
| `--accept-plan <digest>` | With `--unattended` only, optional: the stricter acceptance. The plan the run builds must have exactly this digest, or nothing is installed. |

The global `--json` applies only with `--unattended`. `--events` is refused with the door (its
progress is for a person; a script that wants progress events uses `setup install --events jsonl`).
The door always uses the `desktop` profile and the default probe deadline. `--profile worker` stays
on `setup check` and `setup plan`, because a worker host is set up by its operator.

### 2. When a person is there

A person is there when standard input and standard output are both terminals. Then the door:

1. **Checks**, with the same engine call and the same table as `setup check`, headed
   `VSift <version> setup`. The table and the verdict line are one renderer shared by both
   commands, so they cannot drift.
2. **When the setup is complete**, ends with the extras line (section 7) and
   `Setup complete. Try it: vsift ingest ./your-video.mp4`. When `vsift <video>` ships (W6g, ADR
   0037) that story changes the line to `vsift ./your-video.mp4`.
3. **When something core is missing and VSift can install it here**, it builds the plan (the same
   `plan_setup` call `setup plan` makes, with the included extras), then shows one offer for
   everything core that is missing. Each piece is named once with its publisher, its size and its
   licence; then the total download, "checked against a fixed fingerprint", "for your user only".
   One question follows: `Install video and speech now? [y/N]` (or the one part). Exact addresses,
   SHA-256 digests and file locations are not repeated: they stay in `setup plan`, and the offer's
   last line before the question says so.
4. **Reads one answer.** `y` or `yes`, in any case and with surrounding spaces, is yes. Anything
   else is no: Enter, any other word, a closed input, Ctrl-C at the question, or no answer within
   ten minutes. There is exactly one question per run; nothing about extras is asked (section 7).
5. **On no**, changes nothing and ends `Nothing was installed. Run vsift setup again when you're
   ready.`
6. **On yes**, installs (section 4), shows the progress (section 5), checks again and shows the
   table again, then the verdict and one `Next:` line.
7. **When a missing part cannot be installed here** (no qualified catalogue for this target, or not
   for that component: speech on Windows and macOS until W5a, every part on macOS today), the table
   shows it, one line says `VSift can't install <part> on this computer yet.`, the next lines link
   the installation guide's do-it-yourself section and give the one `setup configure` (or
   `setup configure-model`) command. Nothing more is attempted for that part: no folder search, no
   paste step (deferred by the maintainer). The offer still covers any other part that can be
   installed.
8. **Checks that ran out of time** are not an offer: the door says so, as `setup check` does, and
   ends `Next: vsift setup`, since a plan built from timed-out probes would install the wrong
   things.

The last line is always where the person stands or the one thing to do next. Before this door
ships, `setup check`'s `Next:` names concrete commands (W6e); with it, every incomplete readable
`setup check` ends `Next: vsift setup`, and `Next` becomes that one value plus "try a video".

### 3. When nobody is there and `--unattended` is not given

When standard input or standard output is not a terminal, the door never prompts and never
waits. It writes the same read-only table and verdict as `setup check`, then
`Nobody is here to answer, so nothing was installed. To install without questions: vsift setup
--unattended --install video,speech` (naming the parts that are missing), and exits with
`setup check`'s exit status. `--json` without `--unattended` is refused before anything runs:
`INVALID_ARGUMENT`, remediation "`vsift setup` is for a person at a terminal; use `setup check
--json` to read the setup, or `setup --unattended --install <parts> --json` to install." So an
assistant or a script that calls the door by mistake gets a typed answer and never a question.

### 4. One install path: the existing transaction

The door installs only through `Engine::install_setup`, with the plan the door showed and that
plan's own digest as the acceptance. There is no second install path. The engine still takes the
install guard, rebuilds the plan and requires it to equal the one shown, so if the machine changed
between the offer and the yes, nothing is installed and the door says to run it again. To pass the
plan in process, the contract crate gains a constructor from an evaluated plan to the existing
`SavedSetupPlan` (the same envelope `setup plan --json` writes and `setup install --plan` reads),
so the engine's acceptance code is unchanged.

**Video and speech as two independent parts (engine change, the maintainer's design).** In the
transaction, the core becomes two parts, like the optional groups of ADR 0032: video (the
`ffmpeg_ffprobe` component) and speech (the whisper.cpp program and the model, which stay one unit,
staged and smoked together as today). The proposal:

- **Downloads run side by side**: the two parts' downloads and verifications run concurrently,
  at most one download per part at a time, under the one install guard the transaction already
  holds. No new lock, and no change to the guard's never-wait rule.
- **Activation stays one atomic step per component, in a fixed order**: video first, then speech,
  which is smoked with the video just activated (or the one already selected) as its companion.
- **One failing doesn't stop the other.** A speech failure leaves video activated (true today). A
  video failure no longer stops speech's download and verification. If no working video tools are
  selected when speech is ready to test, speech is reported "waiting for video" in the readable
  view and `blocked` in the JSON (an existing value, whose meaning, not activated because another
  component did not install, is unchanged), and **its verified download is kept** (the planner's
  decision 2). A rerun after video is fixed verifies the kept file again by its reviewed size and
  SHA-256 against the plan before it is used, discards it only if it no longer matches, and then
  tests and activates speech without downloading it again. The rule that a partial download is
  never resumed is unchanged: a complete, verified file is not a partial one.
- **Optional extras** keep ADR 0032's rule: installed after the core, each a part of its own, and
  never able to fail the core.

`setup install` gets the same transaction, so its JSON shape is unchanged; only which components
are `blocked` after a video failure changes (speech is now attempted up to its test).

### 5. What the person sees while it installs

Third-party detail appears at three moments only: before the yes (the offer), during the install
(one line per piece), and after (the table).

- **On a terminal**: a heading `Installing video and speech: 3 downloads, about 270 MB`, then one
  line per piece (`FFmpeg`, `whisper.cpp`, `speech model`) with a bar, the bytes so far of the
  reviewed size, and a stage word: waiting, downloading, checking, testing, ready, or the failure in
  plain words (`the download stopped: no connection`). Lines are redrawn at most four times a
  second on standard output; no question is asked during the install.
- **The data behind it** comes from a new internal observer on the transaction: per component, its
  stage and the bytes received, reported by the one download function
  (`download_reviewed_publisher_artifact`) and the transaction's steps. It is a library-level
  addition to `install_setup`, not part of `--events jsonl`: `setup install --events jsonl` keeps
  emitting exactly the `installing_components` counts it emits today.
- **Without a terminal** (the unattended form): one plain line per piece as it finishes, no bars.
- **Ctrl-C** cancels through the existing cancellation: a component already activated stays, the
  one in progress is discarded as today, and the door ends with the table and `Next: vsift setup`.

### 6. Unattended: parameterised, pass, fail or error

`vsift setup --unattended --install video,speech [--include web-videos] [--accept-plan <digest>]
[--json]` runs once, asks nothing, and reports one of three outcomes:

- **pass** (exit 0): every named part works after the run, including parts that already worked
  (nothing to do is a pass).
- **fail** (the exit status `setup install` gives the same failure today): the install ran and a
  named part does not work; each part's result and next step are reported.
- **error** (the exit status of the failure code): it could not start: an invalid argument, a part
  named that VSift cannot install here (refused before anything changes, `MISSING_CAPABILITY` with
  the do-it-yourself remediation), timed-out checks, a busy install guard (`BUSY`), or an
  `--accept-plan` digest that does not match (the existing stale-plan failure).

**Naming the parts is the acceptance (the maintainer's decision, 2026-10-27, ratified the same day:** "Ratify it: in unattended vsift setup, naming the parts (for example --install video,speech) counts as my yes, with the pinned and verified downloads unchanged; record it in ADR 0036 as superseding that part of ADR 0023."**)** ADR 0023 accepts a
plan only when its digest is quoted back, so a caller has reviewed exactly what will arrive. Here
naming `video` or `speech` accepts the plan the run builds for those parts. This is safe for the
same reason a reviewed digest is: every artifact the plan can contain is pinned in VSift's own
reviewed catalogue by URL, size and SHA-256, verified before anything is extracted or run, and
smoked before it is selected, so the caller cannot be handed anything VSift has not reviewed. What
the caller gives up is pinning the catalogue revision; `--accept-plan <digest>` keeps that as the
stricter form, and `setup plan` / `setup install --accept-plan` are unchanged.

So that naming only `video` installs only video, the plan request gains the core parts to plan
(default: all), an additive library field. A plan of all parts, which is what `setup plan` builds,
has the same intent and digest as today, so saved plans stay valid.

With `--json`, the result is a new command, `setup` (additive to JSON v1: a schema, an example,
contract tests and `cli-v1.md`): `outcome` (`pass` or `fail`), one entry per core part and included
extra with its status and next step, and the `setup.install` data when an install ran (`null`
when nothing needed installing). An error is the ordinary failure envelope. No existing command's
JSON changes.

### 7. The extras

The table shows each extra the catalogue knows on every run: added, not added, or not available
here (web videos outside Ubuntu for now). Once the core is complete the door adds one line for the
extras that could be added here: `Optional extras: web videos. Add one with: vsift setup --include
web-videos`. Nothing about extras is ever asked. `--include` works with and without a person: with
one, the extra joins the offer, which shows what it adds, its size, and its licence and site-terms
note. An action that needs a missing extra pointing back to `vsift setup --include …` belongs to
the action's own story.

### 8. Symbols and colour, with a plain fallback

On a terminal the status words gain a symbol (`✓` ready, `✗` not set up or failed, `–` none added)
and colour (green, red, dim), and the progress lines draw bars. Everything is plain, words only,
exactly the `setup check` table, when standard output is not a terminal, `NO_COLOR` is set to
anything, or `TERM` is `dumb`. On Windows, colour is used only when the console accepts virtual
terminal sequences; the symbols are written as Unicode through the console's wide-character API,
which Rust's standard output already uses for a console. The colour decision and the Windows
console handling come from `anstyle-query` (already in `Cargo.lock` through `clap`; a direct
dependency needs the AGENTS.md review and `cargo deny` in the pull request). A status word is
always present, so meaning never depends on colour.

### 9. What the door never does, and why agents are unaffected

- It never installs without an explicit yes: a typed `y`/`yes` at the one question, or parts named
  with `--unattended`.
- It never asks for, or runs anything with, `sudo` or elevation; everything installs for the user
  alone, in the managed root, as today.
- It never searches folders for tools, and never reads a path from anything but the person's own
  `setup configure` command.
- It never prompts without a terminal, and never waits for an answer without one.
- Every other command keeps never prompting. The door is the only command that reads an answer.

**Agents.** In 0.3.0 the skill keeps today's route (`setup check`, `setup plan`, and the user's own
`setup install`). The skill's command policy (`skills/vsift/references/commands.md`) gains `vsift
setup` as **never**, and the policy test's never-class set gains the `setup` operation. This is a
skill text change, so it needs a trial freeze (`v030-rules.md`: a draft freeze in the pull request,
rewritten at the merge rebase). Moving the skill to the unattended form is a later release's
decision, and even then it still asks the person first.

### 10. What this changes in earlier records

- **ADR 0023**: a digest quoted back stays the acceptance for `setup install`; the door adds two
  more: the person's yes to the plan they were shown (accepted with that plan's own digest), and
  the named parts of an unattended run (section 6). A dated note in ADR 0023 points here.
- **ADR 0023 / ADR 0032, the transaction**: video and speech become independent parts (section 4);
  "the first failure stops the transaction" now holds within a part.
- **ADR 0008 and `cli-v1.md`**: bare `vsift setup` no longer prints help; `vsift setup --help` does.
- **ADR 0022**: one more never-class command.

### 11. Tests

- **Scripted terminal tests**, in process, through two injected ports: a console port (whether a
  person is there, whether colour is allowed, the width, and the answers, scripted per test) and a
  setup-steps port (check, plan, install with progress), with a fake in tests and the engine in
  production. They cover yes, no (Enter, `n`, a closed input), a failed piece (video fails, speech
  waits; speech fails, video stays), a part that cannot be installed here, timed-out checks, no
  terminal, the unattended pass, fail and error, `--accept-plan` mismatch, and the colour fallback
  (`NO_COLOR`, `TERM=dumb`, not a terminal), with golden files in
  `crates/vsift-cli/tests/human_output/`.
- **Binary tests** with no terminal: the read-only report and its exit status, `--json` refused,
  `--unattended` without `--install` refused, and, on hosts without a catalogue, a named part
  refused before anything is created. On the Ubuntu CI runner the existing authenticating proxy
  stops the first download, so an unattended run is proven to reach the install and fail cleanly
  without network. Nothing downloads.
- **Engine tests** for the independent parts, over the existing `ManagedComponentInstaller` fake
  in `crates/vsift-application/tests/p13_install_transaction.rs`.
- **JSON**: every existing contract test unchanged; a schema and contract test for the new
  `setup` result.
- **Policy**: the skill-policy test with `setup` in the never class.

### 12. Delivery

Two pull requests under this record, each standing on its own (the deadline rule):

1. **The door**: sections 1 to 3 and 6 to 9, installing through today's sequential transaction,
   with one line per piece as it finishes (the counts the transaction already reports), the
   colour fallback, the skill policy and the records.
2. **The independent parts and the live progress**: section 4's engine change and section 5's
   bars and stages.

If the second misses the 0.3.0 cut, the door ships with sequential installs and plain lines, and
the limitation is registered as L-233.

## Decisions for the planner and the maintainer

The planner answered on 2026-10-27, relayed by the coordinator. Decisions 3 to 7 were left to this record's
recommendation, on the condition that none changes the meaning of a JSON v1 field or a safety rule.

1. **The unattended acceptance (section 6)** changes an accepted safety decision of ADR 0023, so it is the
   maintainer's. **Outcome: ratified by the maintainer on 2026-10-27**, relayed by the planner, in his words:
   "Ratify it: in unattended vsift setup, naming the parts (for example --install video,speech) counts as my yes, with the pinned and verified downloads unchanged; record it in ADR 0036 as superseding that part of ADR 0023." This record therefore supersedes, for the unattended door only, ADR 0023's rule that a managed plan is
   accepted only by its digest quoted back; `setup install --accept-plan` and the attended door's yes keep it.
   Built in the second pull request, beside the transaction changes it shares the plan and install code with.
2. **Speech's verified download when video fails.** Proposed: discard it and download it again on the rerun.
   **Outcome (the planner): keep it**, as the design said, and verify it again by size and SHA-256 against the
   plan before activation, discarding it only if it no longer matches (section 4). Built in the second pull request.
3. **Two pull requests or one** (section 12). **Outcome: two**, as proposed.
4. **`--json` without `--unattended`.** **Outcome: refused**, as proposed: `INVALID_ARGUMENT` under the new
   `setup` command identifier, whose remediation names `setup check --json` and the plan route. `--events` is
   refused the same way. Additive: a new command identifier the schemas' pattern already admits.
5. **The new `setup` JSON result for the unattended form.** **Outcome: as proposed**, with section 6, so it waits
   for decision 1.
6. **The answer timeout.** **Outcome: ten minutes, then no**, as proposed.
7. **The colour dependency.** **Outcome: a direct dependency, narrower than proposed**: `anstyle-query` alone
   (MIT or Apache-2.0, already in `Cargo.lock` through `clap`, no new crate in the lock). It reads `NO_COLOR` and
   `TERM` and turns on a Windows console's virtual terminal sequences; the door writes its three colours itself,
   from fixed sequences (section 8 and the implementation note).

## Implementation note, 2026-10-27 (the first pull request: the door, attended)

What the first pull request built, and where it differs in detail from the sections above:

- **The door** (`crates/vsift-cli/src/door/`): the `Console` and `SetupSteps` ports, the engine and system
  implementations, and the conversation of section 2. `crates/vsift-cli/src/human/setup_door.rs` writes its text;
  `setup_check.rs` gives both views one `push_overview` (the table, updates, verdict and details) and one
  `Standing` (video and speech as a person meets them).
- **One install path.** The door passes the plan to the engine by serialising the evaluated plan's `setup.plan`
  result and reading it back through the strict saved-plan reader (`SavedSetupPlan`, `validate_envelope`), then
  `setup::install_saved`, which `setup install` now also calls. So no contract constructor was needed (section 4
  proposed one), and the engine's acceptance is exactly the one `setup install --plan` meets.
- **Timed-out checks** end `Next: vsift setup check`, in the door and in `setup check`, not `vsift setup`
  (section 2, step 8): the advice above it is to add `--timeout-seconds`, which `setup check` has and the door,
  by section 1, does not.
- **Without a terminal** (section 3) the last lines name `setup install --help` for installing without questions,
  until the unattended form exists (the second pull request).
- **`setup check`** ends `Next: vsift setup` whenever a desktop setup is not complete (except after a timeout),
  and its extras line names `vsift setup --include web-videos`. A worker profile keeps the concrete step.
- **While it installs**, the live line on standard error says how many pieces are done, from the transaction's
  existing `installing_components` counts; one line per piece follows when it finishes. The bars and stages of
  section 5 come with the second pull request.
- **Colour** (section 8): `TerminalText::push_status` is the one place an escape sequence is written, chosen by a
  closed `Tone`, so the builder's rule that no pushed text can drive the terminal still holds.
- **Ctrl-C** is trapped from the question on, so at the question it is a no, and during the install it cancels
  through the existing cancellation. Before the question it ends the process, as for any short command.
- **The skill** classes `vsift setup` as never in `references/commands.md`, and the policy test's never set has
  `setup`. That moved the trial freeze, written again on the `main` the pull request was rebased onto to merge.

## Implementation note, 2026-10-27 (the second pull request: independent parts, live progress, unattended)

What the second pull request built, and where it differs in detail from the sections above:

- **The transaction** (`crates/vsift-application/src/install.rs`): the installer port splits fetching (download,
  import or a kept download, verified by size and SHA-256) from staging (unpacking and preparing the runtime). The
  core is two parts, video and speech; their fetches run side by side on the command's own task (a two-future join,
  so the application stays free of an async runtime), one fetch per part at a time. Activation is video first. When
  video did not install and the installer reports no working video tools, speech's verified downloads are kept
  (`DownloadKeeping`) and its components stay `blocked`; a cancellation discards them instead. Optional parts are
  unchanged: after a complete core, each a part of its own.
- **Kept downloads** (`crates/vsift-infrastructure/src/managed_artifact_store.rs`): a verified, unextracted stage is
  renamed to `kept-<reviewed SHA-256>` in the managed root. The next run renames it back to a fresh stage name, so a
  killed run leaves an ordinary stage for the sweep, and reads it in full against its reviewed size and SHA-256
  before it is used; one that no longer matches is discarded and downloaded again. The stale-stage sweep leaves kept
  downloads; `setup install` removes those whose bytes its plan no longer installs, and inspection checks their
  shape (`setup repair` reports one it cannot prove).
- **The observer** (section 5) is `InstallObserver` in the application and `ProgressObserver::with_install_pieces`
  in the engine: per piece, its stage (`PieceStage`) and bytes. `--events jsonl` is unchanged except that the
  `fetching_artifact` events of the two parts can alternate.
- **The bars** need a terminal that takes symbols and colour (`Styling::Rich`). With plain words on a terminal the
  door keeps the first pull request's live count on standard error; section 8 keeps bars to the rich terminal.
- **The plan request** gains the core parts (`CorePartSet`; `SetupPlanRequest::parts`, `SetupInstallRequest::parts`):
  a part left out is offered nothing. A plan of both parts is `setup plan`'s, with the same digest.
- **Unattended** (`crates/vsift-cli/src/door/unattended.rs`): it refuses, before anything changes, timed-out checks
  (`DEADLINE_EXCEEDED`), a named part with no reviewed catalogue here and an included extra unavailable here (both
  `MISSING_CAPABILITY`), and an `--accept-plan` digest that is not the plan's (`INVALID_ARGUMENT` with the
  stale-plan remediation, compared in the door before the install guard is taken). An extra that fails never makes
  the outcome a fail; its entry names the next step. A `--json` fail is a `complete` result with a non-zero exit, as
  a blocked `setup check` is. The `setup` result carries the install's `setup.install` data unchanged.
- **The planner's two wording changes:** the close says `Setup complete: video and speech both work.` once, then
  `Try it: vsift ingest ./your-video.mp4`. After the `[y/N]` question the door breaks the line only when nobody
  typed an answer (the time limit, Ctrl-C, a closed input); a typed answer's Enter is the only break, so a
  person never sees an empty line there, and what follows never sits on the question's line.

## Consequences

- A person gets from `npm install` to "setup complete" with one command and one yes, and comes back
  to the same command to see where they stand.
- One renderer serves `setup check` and the door; one transaction serves `setup install` and the
  door; one acceptance check guards both.
- The door is the first command that reads an answer. The rule that no other command prompts is
  now written down and tested.
- The unattended form lets a company script exactly the parts and extras it allows, without a
  digest, while every artifact stays pinned and verified.

## Alternatives

- **A separate `vsift setup guide`.** The first round of the design; set aside because a person
  should come back to one door, and `setup check` is its read-only view.
- **A prompt in `setup install`.** Rejected: `setup install` is the assistant's and the script's
  command, and no existing command may start prompting.
- **Unattended acceptance only by digest.** Kept as the optional stricter form; as the only form it
  makes scripting an install depend on reading a digest out of a file first.
- **Installing speech before video.** Rejected: speech cannot be tested without video tools.
