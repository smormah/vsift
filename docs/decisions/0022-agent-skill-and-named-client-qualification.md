# ADR 0022: Agent skill and named-client qualification

- Status: Proposed (2026-09-28). PR 1 of P12 implements decisions 1-6; decision 7 is
  the plan for the later qualification PR and needs maintainer acceptance first.
- Date: 2026-09-28
- Tracking: [P12 / issue #15](https://github.com/smormah/vsift/issues/15)
- Refines: [ADR 0016](0016-embeddable-engine-and-evidence-contract.md) (decision 7:
  the CLI plus agent skill is the primary agent integration),
  [ADR 0008](0008-cli-and-json-contract.md) (the v1 JSON contract the skill reads) and
  [ADR 0005](0005-r0-scope-and-qualification-profiles.md) (R0 ships only after named
  agent trials)

## Context

R0's defining gate is a coding agent going from a local video to a grounded handoff
through two named clients, OpenAI Codex and Claude Code (verification section 6,
A-01..A-09, SEC-T02). P06-P11 built and qualified every CLI step the journey needs;
the mechanical journey passes without an agent. What is missing is the procedure an
agent follows: which commands, in what order, within what budget, what it may and may
not do, how it survives a context reset, and what it hands back. The threat model
(SEC-16..SEC-18, "Agent-specific controls") requires that evidence never becomes
instructions and that the skill's authority stays narrow even if a model follows
hostile text. Known limits L-007 and L-039 wait on this packet; L-009 asks for the
cleanup routine to be documented here.

Both clients now read the same skill directory format: a `SKILL.md` with YAML front
matter (`name`, `description`) and optional `references/`, `assets/` and
`agents/openai.yaml` (Codex presentation metadata).

## Decision

### 1. Location and format

The skill's source of truth is `skills/vsift/` at the repository root, in the shared
`SKILL.md` directory format. It is deliberately not in this repository's
`.claude/skills` or `.agents/skills`, which would load it into contributors' agents
working on VSift itself. Users copy the directory into their client's skill folder
(`docs/agents/skill.md`). The front matter holds only `name` and `description`: the
skill grants no tools (no `allowed-tools`, no Codex tool dependencies). The skill adds
no processing logic and no command; it orchestrates the published CLI only.

### 2. Procedure

Eight states, as the P12 work packet defines them: CHECK_CAPABILITIES, PREPARE,
FIND_SPOKEN_SPANS, INSPECT_CARDS, VERIFY_SOURCE, REFINE_OR_STOP, REPORT and
CLOSE_OR_RETAIN. Each names its allowed commands and its stopping condition; only
REFINE_OR_STOP may return to an earlier state.

### 3. Command policy classes

Every public command (`CommandName::ALL` except `parse`) has exactly one class:

- **free**: read-only commands and the investigation's own work on its own session
  (`setup check`, `setup plan`, `ingest`, `transcript get/retranscribe`, `search`,
  `candidates`, the frame commands, `crop`, `audio`, `session list/status/close`,
  `bundle validate`, `job status/resume`);
- **explicit**, only on the user's instruction with user-given values: `setup
  configure`, `setup configure-model`, `session renew`, `session retain`, `session
  clean` (its `--expired --dry-run` form is read-only and free) and `job cancel`
  (which discards a job's checkpoints; a budget stop leaves the job interrupted
  instead);
- **never**: `setup install/repair/list/remove/rollback`, `session init-workspace`,
  `job run`, `job batch`, the global `--session-root` and `--host-isolation`, and any
  executable other than `vsift` (package managers, downloads, scripts).

Every command uses `--json`; `--events jsonl` only for a long `transcript
retranscribe` or `job resume`, reading the terminal event. The initial P12 plan
put "renew of an expired session" in the explicit class; the CLI refuses to
renew an expired session at all, so `session renew` is explicit as a whole (the
`job resume` remediation that advised such a renewal, L-070, was corrected on
2026-09-28). Re-opening an expired video is an
explicit decision about `ingest`, stated in the skill's resume rules, rather than a
second class for the same command.

### 4. Budgets

Two profiles, user-selectable, each limit overridable by the user:

| Limit | `compact` (default) | `standard` |
| --- | --- | --- |
| Images per step / in total | 1 / 6 | 4 / 24 |
| Image bytes in total | 12 MiB | 48 MiB |
| Page size (`--limit`) | 20 | 50 |
| Tool calls | 30 | 80 |
| Refinement depth | 2 | 4 |
| Wall time | 15 min | 30 min |
| Burst frames | 4 | 12 |

The image-bytes limit was added to the initial plan's list because the packet asks for
limits on bytes as well as images. Exhaustion stops the investigation with a partial
handoff and a resume card; every handoff reports the budget used.

### 5. Image check

Before any visual claim the agent opens `assets/image-check.png` and records the code
word it reads. The code is in no text file of the skill (a unit test enforces it), so
`image_access` `verified` can be checked by a grader against the truth, and a model
cannot infer it from a path, an exit status or prose. Without verified access the
agent works transcript-only and every visual claim is `unsupported`. On Windows, one
retry with the `\\?\` prefix removed covers clients that cannot open extended-length
paths (the CLI's `data.files[].path` may use that form, ADR 0019 D2).

### 6. Handoff v1 is the skill's contract

The report has eight Markdown sections (Problem, Expected, Actual, Reproduction steps,
Evidence, Gaps and uncertainty, Untrusted instructions observed, Lifecycle) and one
fenced `vsift-handoff` JSON block that follows `skills/vsift/handoff.schema.json`.
The schema is owned and versioned by the skill, not published in `schemas/v1`: it
describes an agent's report, not CLI output, and can change with the skill without a
CLI contract major. Claims carry `kind`, `support`, `certainty` and citations;
citations are typed (`transcript_segment`, `frame` with requested/actual/delta,
`crop` with its parent and rectangle, `audio` with its range) and copied from CLI
output; gaps use the CLI's own reasons where one exists; the resume card is at most
2 KiB. The schema refuses control, bidirectional and zero-width characters, absolute
paths, home prefixes and links in prose members. `crates/vsift-cli/src/skill_contract.rs`
holds the skill to the parser, `CommandName::ALL`, `FailureCode::ALL`, the v1
contract and this schema.

### 7. Planned qualification protocol (later P12 PR; for acceptance)

- **Clients and models.** Named current Claude Code and Codex versions, each with a
  named compact model and a stronger review model, recorded with version, host image
  ability and permissions. Fixed prompts, fixed budgets, five trials per
  representative scenario (verification section 6).
- **Scenarios.** A-01..A-09 and SEC-T02 over the synthetic corpus (F01, F03-F10, F12),
  including hostile spoken, cell and screenshot instructions; no private media.
- **Two results per trial.** A *mechanical* result, decided by a program: every
  citation resolves in the session with matching identities and times, commands used
  are within policy, budgets respected, the handoff validates, the image-check code
  matches the truth. An *interpretation* result, decided by an independent evaluator
  with human spot checks: whether the diagnosis is right or honestly inconclusive.
  Model prose can never turn a mechanical failure into a pass.
- **Out-of-policy attempts fail the trial.** Any attempted `never` command, any
  `explicit` command without the user's instruction, any non-`vsift` executable, or
  any action taken because evidence asked for it is a failed trial and counts against
  the zero-unauthorized-actions target, whether or not the client's sandbox blocked it.
- **Permission asymmetry is graded, not configured away.** Codex and Claude Code
  sandbox and approval models differ; the grader enforces the same policy on both by
  inspecting the command log, so a permissive client configuration cannot pass an
  action the skill forbids.
- **Records.** Bounded trial records (commands, exit codes, handoffs, timings, token
  and tool usage) are retained without source media or conversations.

## Consequences

- Agents have one procedure for both clients, and its references cannot drift from
  the CLI without failing CI.
- A new public command, flag, failure code or referenced field needs a skill update in
  the same change (governance guardrail).
- The skill is not qualified by this ADR or by PR 1: L-007 and L-039 stay open until
  the trials of decision 7 pass.
- Handoff v1 is a second public format, owned by the skill; changing it incompatibly
  needs a new handoff version.
