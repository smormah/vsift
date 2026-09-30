# ADR 0008: CLI namespace and JSON contract

- Status: Accepted
- Date: 2026-09-10
- Resolves: DEC-09, DEC-10

## Context

Humans and agents need one stable integration surface. Unstructured output and
ad-hoc command growth make small-model operation and compatibility unreliable.

## Decision

The R0 namespace is `setup`, `session`, `ingest`, `transcript`, `search`, `candidates`,
`frame`, `audio`, `crop`, `bundle` and `job`. `setup` alone shows help and never
installs. Commands support concise human output and an explicit versioned JSON result;
streaming progress uses an explicit JSONL event mode.

Keep the existing `setup.check` version-one payload compatible. New operations use
the documented v1 envelope with command-specific typed data, warnings, error, coverage
and lifecycle fields. stdout contains results, stderr contains bounded diagnostics.
Fields, error codes, exit codes, cursors and identifiers are public API with fixtures.

## Consequences

P01 defines exact schemas before adding the commands. Later hosts call application
use cases and conform to the same semantics rather than scraping CLI output or
reimplementing business rules.

## 2026-09-29 note: `display_text` for hidden characters

**Context.** A transcript segment's `text` is the payload without markup, not display
text: it keeps bidirectional controls, zero-width characters and other characters a
reader cannot see, and for WebVTT it decodes a character reference such as `&#x202E;`
into the raw character. In the P12 SEC-T02 trials, models copied a raw U+202E from
`text` into their reports (Claude Sonnet 5.5 in 1 of 5 runs, Claude Haiku 4.5 in 4 of
5; known limit L-083), although the skill told them to write such characters as
`<U+202E>`: a model cannot reliably escape a character it cannot see.

**Decision (maintainer, 2026-09-29).** VSift makes these characters visible itself,
through a strictly additive field; the meaning and bytes of every existing field stay
as they are.

1. Every transcript segment record, wherever a result returns one (`transcript get`
   and `search`, in `--json` items and `--events jsonl` evidence records), gains
   `display_text`: `text` with every hidden character written as `<U+XXXX>` (`U+`, the
   code point in uppercase hexadecimal of at least four digits, in angle brackets). A
   speaker object gains `display_label`, its `label` rendered the same way, because a
   WebVTT voice name is evidence text too. No other result carries text from evidence:
   candidates, frames, crops, audio, jobs and worker results carry none.
2. **Hidden characters** are general category `Cf`, every
   `Default_Ignorable_Code_Point`, and U+2028 and U+2029, from Unicode 16.0.0. `Cf`
   holds every bidirectional control and the zero-width characters, joiners, invisible
   operators, the byte-order mark, the soft hyphen and the tag characters (a known
   channel for smuggling instructions into model input). `Default_Ignorable_Code_Point`
   is Unicode's own list of characters a renderer shows as nothing; it adds the
   combining grapheme joiner, the Hangul fillers (used in look-alike spoofing) and the
   variation selectors (which can encode arbitrary bytes after a visible character),
   and reserves ranges for future ones, so a character added later is already covered.
   The line and paragraph separators are rejected at import like other control
   characters; they are included so the rule does not depend on that. Left out, because
   they are visible: spaces of any width (a space shows as a space), private-use
   characters, noncharacters and other unassigned code points (shown as a replacement
   glyph). A few `Cf` characters, such as the Arabic number signs, have a glyph but act
   on the layout of the text after them; the whole category is taken so the rule is one
   Unicode property, not a judgement per character. The table lives once, in
   `vsift_contract::is_hidden_character` (the contract crate already owns
   `sanitize_untrusted_text`, the rule for putting untrusted text into output), with a
   unit test for every group named here.
3. **Rendering happens at output, never in storage.** Stored revisions, retained
   bundles, segment and revision identities and digests do not change;
   `bundle-transcript-record.schema.json` does not gain the member.
4. **Literal notation is not escaped.** Text that already reads `<U+202E>` is shown as
   written. Rendering is idempotent and never escapes its own output twice. The cost is
   that `<U+202E>` in `display_text` may be a hidden character or those characters as
   written; `text` says which, and both are visible, which is the point.
5. **Search is unchanged.** It matches `text`; a hidden character separates words like
   punctuation. A query stays literal: `<U+202E>` is the words `u` and `202e` and never
   matches a hidden character. Matching notation against hidden characters would make a
   query mean two things, and nothing needs it: a reader searches for the visible words.
6. **Compatibility.** Within v1 readers ignore additive response members, and producers
   must not reinterpret existing ones (this ADR). `display_text` and `display_label`
   are new members, always present, and required in `transcript-segment.schema.json`.
   The option of redefining `text` itself as display-safe (ADR 0022's proposal) was not
   taken: it would change a published field's meaning.
7. **Consumers.** The skill quotes `display_text` (never `text` or `original_text`)
   whenever evidence text appears in a report. `display_text` is at most four times the
   bytes of `text` (a two-byte hidden character becomes eight), so an adversarial page of
   100 segments can exceed the 1 MiB result budget and fail with exit 7; a smaller
   `--limit` reads it.

`docs/contracts/cli-v1.md` ("Display text"), `schemas/v1/transcript-segment.schema.json`
and the SEC-T02 suite (`sec_t02_adversarial_evidence`) state and test this.

## 2026-09-30 note: P13 contract changes

[ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) (P13,
Proposed) lists the contract changes P13 makes and why:

- the namespace gains `handoff` (`handoff check`, #213);
- human output becomes readable terminal text by default, with no TTY detection and no
  colour, escaping control and hidden characters with `display_text`'s rule;
- a new failure code, `DOWNLOAD_FAILED`, in exit class 7;
- new response schemas for `setup install/list/remove/rollback/repair` and `handoff
  check`;
- a typed parse remediation that never echoes argument text (L-071).

Before the first publication, values that have never shipped may change in place
rather than by a new version: the setup-check `lookup` value `managed_version`, the
remediation's `managed_install` values, the setup-plan availability
`catalogue_accepted` (from `catalogue_accepted_install_pending`) and the setup plan's
observed-state members (`install_needed`, an action's `state`, the model status
`managed_current`). After the first
published artifact, v1 changes are additive only, as above.

## 2026-09-30 note: the `handoff` namespace (P13 PR 5)

Under ADR 0023 decision H4 (a v1 value that has never shipped may be added in place
before the first publication), P13 PR 5 adds the `handoff` namespace and its one
command, `handoff check` (`CommandName::HandoffCheck`, identifier `handoff.check`),
with `schemas/v1/handoff-check-data.schema.json` and the frozen example
`schemas/v1/examples/handoff-check.json`. No existing value changes. The command
answers every draft it could read with `complete` and `data.valid` (exit 0); only an
unreadable draft (over 64 KiB, not UTF-8, a relative or unreadable `--file`) is
`INVALID_ARGUMENT`. Its findings carry a closed `rule`, an RFC 6901 pointer built from
the handoff schema's own member names, a line number, the schema's allowed values and
fixed prose, never text from the draft. `docs/contracts/cli-v1.md` ("P13 `handoff
check`") is the contract.

## 2026-09-30 note: P13 PR 4 in-place edits made

P13 PR 4 made the setup edits listed above, before any publication:

- the setup-check `lookup` enum gains `managed_version` (the version `setup install`
  selected, between the configured path and the filtered `PATH`);
- the setup-check remediation's `managed_install`, formerly the constant
  `unavailable_unqualified`, now takes `catalogue_accepted`, `unavailable_target`,
  `unavailable_catalogue_expired` or `unavailable_catalogue_invalid` for this host;
- the setup-plan availability `catalogue_accepted_install_pending` is renamed
  `catalogue_accepted`, and the managed-install `next_step` texts say that `setup
  install` applies the plan;
- the setup plan gains its observed state beside the digested intent (ADR 0023 PR 4
  note): the required `install_needed`, each action's required `state` (`pending` or
  `current`) and the model status `managed_current`; `readiness` and the dependency
  statuses now include the managed tier, and acceptance compares the intent only;
- `DOWNLOAD_FAILED` (exit 7) joins the failure codes, whose exit class 7 now covers a
  managed download too;
- `setup-install.schema.json` describes the `data` of a `setup.install` result, which a
  failed install also carries beside its error (a failure's data, as `job.run` already
  does), with frozen examples `setup-install.json` and `setup-install.failed.json`;
- the progress event gains the stages `fetching_artifact` (bytes) and
  `installing_components` (the new unit `components`).

The frozen examples of a reserved command now use `setup.repair`, which still answers
`COMMAND_NOT_IMPLEMENTED`.
