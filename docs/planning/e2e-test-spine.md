# Incremental end-to-end test spine

Status: accepted test-development framework; no executable end-to-end runner exists
yet. Tracking issue: [#40](https://github.com/smormah/vsift/issues/40).

## Purpose

VSift must not wait until release qualification to discover that independently tested
components do not form a usable video investigation. Starting with P04, each packet
connects its production capability to one cumulative, opt-in test journey. The journey
grows with the product while the delivery ledger continues to control implementation
eligibility.

This framework is not evidence that an unimplemented stage works. A checkpoint reports
each stage as `passed`, `failed`, `blocked`, or `not_implemented`; it never replaces a
missing stage with a mock and calls the complete journey successful.

## Governing journeys

Two journeys converge on the same grounded evidence result:

- **A-08 / local ASR:** start with a synthetic local video and no transcript; inspect
  dependencies, extract audio, run the qualified local whisper.cpp adapter, search the
  timestamped transcript, refine visual evidence and validate every source reference.
- **A-09 / supplied transcript:** start with the same video and a valid supplied
  transcript; prove ASR is skipped, then exercise dependency degradation and unreadable
  visual handling without fabrication or silent installation.

The mechanical runner validates tool behavior and citations independently of model
prose. P12 adds bounded trials through named Codex and Claude Code clients; a mocked
agent cannot satisfy those trials.

## Incremental attachment points

| Packet | Required addition to the cumulative journey | First useful checkpoint |
| --- | --- | --- |
| P04 | Generate the approved synthetic media; bind its identity; run real FFprobe and bounded FFmpeg frame/audio operations | Video-to-timestamped-media artifacts |
| P05 | Open a disposable session, publish artifacts, and prove explicit retain/cleanup behavior | Repeatable session-scoped media run |
| P06 | Check real dependency readiness and exercise typed, permission-respecting remediation | Fresh/BYO dependency run |
| P07 | Attach supplied-transcript and local-ASR paths to the same fixture truth | Video-to-timestamped-transcript run |
| P08 | Produce bounded visual candidates and transcript search results with coverage metadata | Video-to-searchable-candidates run |
| P09 | Refine exact frames, neighbours, crops and audio; validate requested versus actual timestamps and lineage | Complete mechanical video-to-evidence run |
| P10 | Kill and resume at stage/commit boundaries without accepting corrupt evidence | Recoverable mechanical run |
| P11 | Exercise finite batch, admission, cancellation and structured result behavior | Single-host worker run |
| P12 | Run A-01..A-09 through named Codex and Claude Code clients | Complete video-to-grounded-handoff run |
| P13 | Repeat from clean native/npm installation without a Rust toolchain | Installed-user run |
| P14 | Execute the supported release matrix and preserve reviewed evidence | R0 release qualification |

Every P04-P13 completion record must identify the attached stage and its checkpoint
evidence, or explicitly state why that packet has no applicable attachment. A unit or
component integration test remains necessary even when the cumulative journey passes.

## Execution model

The first executable harness belongs to P04, when deterministic media and real media
operations exist. Its exact command and typed report contract are defined in that
packet rather than prematurely freezing an empty CLI. The following rules already
apply:

- It is opt-in during ordinary development and is run after substantial vertical
  increments. It is not an every-PR or mandatory hosted-CI job.
- Release qualification must run it; nightly or dedicated machines may run selected
  noninteractive profiles when their dependencies are available.
- It uses project-owned synthetic fixtures from `fixtures/corpus`; private meetings,
  production credentials and third-party media are forbidden.
- Runs are bounded by explicit time, output, process, memory and disk budgets. The
  process supervisor remains the only external-command boundary.
- Local reports go beneath ignored `.vsift/e2e-runs/<run-id>/`. Evidence promoted for
  review contains hashes, summaries and redacted diagnostics, not unrestricted model
  conversations or media.
- Missing dependencies produce `blocked` with typed remediation. The harness never
  downloads, installs, changes policy or expands agent permissions without explicit
  authorization.

## Evidence record

Each checkpoint record must include:

- repository revision and dirty-state indicator;
- scenario, fixture manifest version and media hashes;
- OS, architecture, filesystem and effective resource profile;
- VSift, FFmpeg/FFprobe, whisper.cpp, model and client versions where applicable;
- stage status, elapsed time, bounded diagnostics and artifact hashes;
- source-reference validation results and any known coverage gaps;
- explicit authorization record for any setup action; and
- overall `passed` only when every stage required at that checkpoint passed.

Model interpretation and mechanical correctness are separate results. A persuasive
answer cannot hide a missing artifact, invalid timestamp, failed stage or unauthorized
action.

## Development and release gates

- **Packet checkpoint:** the packet's new stage passes locally against the cumulative
  journey after its component tests pass.
- **Mechanical checkpoint:** after P09, both transcript paths reach validated source
  evidence without an agent or manual transcript/screenshot preparation.
- **Agent checkpoint:** after P12, A-08/A-09 pass through both named clients with fixed
  permissions, budgets and retained bounded trial records.
- **Distribution checkpoint:** after P13, the agent checkpoint begins from a clean
  supported-machine installation without Rust.
- **Release checkpoint:** P14 runs all supported profiles plus security, fault, load
  and release-integrity gates described in the verification specification.

Failures become regression tests at the lowest useful layer. The end-to-end result
stays failed or blocked until the responsible production behavior and regression
evidence merge; fixture truth is never weakened to make a run pass.
