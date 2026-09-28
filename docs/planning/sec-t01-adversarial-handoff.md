# SEC-T01 adversarial containment evidence: handoff (technical debt)

Status: **technical debt, open for maintainer discussion**. Recorded 2026-09-28.
Tracking: [issue #188](https://github.com/smormah/vsift/issues/188), known-limits register entry
"SEC-T01 adversarial containment evidence deferred". Must be resolved before the R0 release
qualification (P14).

## In plain English

VSift runs third-party tools (FFmpeg, whisper.cpp) on untrusted video. On a server that runs
VSift as a worker, those tools run inside a locked-down Linux sandbox. P11 proved the sandbox is
**present and correctly configured**. It did not prove that the sandbox **holds when a tool behaves
maliciously**. That second proof, the adversarial one, is deferred. It is recorded here so it is
decided deliberately and not forgotten.

## What P11 delivered (accepted by the maintainer for P11)

- **Host attestation.** `--host-isolation strict-linux` runs only when the kernel shows cgroup v2
  CPU, memory and process-count limits, a read-only root filesystem and loopback-only networking.
  Otherwise the command fails with `ISOLATION_UNAVAILABLE` before doing any work. The attestation
  parsers are unit-tested and fuzzed (`host_attestation`, `mountinfo`), and the real attestation
  runs and passes inside the hardened CI container
  (`p11_strict_linux_attestation_holds_inside_the_hardened_container`).
- **Hardened container in CI.** The `strict-worker-boundary` job in `.github/workflows/ci.yml` runs
  VSift's process-supervision qualification inside a container with no network, a read-only
  filesystem, process, memory and CPU limits, all Linux capabilities dropped, no privilege
  escalation, and an unprivileged user. It confirms those controls are inherited by child processes.
- **Supervision controls that apply everywhere.** No shell; a cleared child environment with only
  allowlisted variables; bounded output; deadlines; process-group or job-object termination.

## What is deferred (the technical debt)

The adversarial half of the SEC-T01 requirement in `docs/planning/verification.md` is outstanding.
It calls for a deliberately hostile stand-in provider to attempt each prohibited action inside the
strict worker environment. The requirement lists those actions. For each one it must show three
things:

1. the action is contained;
2. VSift reports a typed failure within a bounded time, without a crash or a hang;
3. nothing sensitive leaks into stdout, stderr or the event stream.

## Why it was deferred

While the hostile stand-in was being designed, an automated safety check stopped the implementing
session. The work was paused rather than worked around, so the maintainer can decide how such a
fixture should be authored and reviewed.

## Options to discuss

| Option | Summary | Trade-off |
| --- | --- | --- |
| A. Maintainer-authored fixture | The maintainer (or a trusted engineer) writes and reviews the hostile stand-in; the existing hardened CI job runs it | Full control and review; needs maintainer time |
| B. Established third-party containment suite | Adopt a recognised container-escape and containment test suite against the strict worker profile | Independent and well known; adds a dependency and licence/maintenance review |
| C. External security review | Commission a review or penetration test of the worker deployment | Strongest independent assurance; cost and scheduling |
| D. Combination | For example B in CI plus C before a public release | Most assurance; most effort |

## Acceptance criteria (whichever option is chosen)

- Every prohibited behaviour listed under SEC-T01 is exercised against the strict worker profile.
- Each is contained, reported as a typed failure within a bounded time, and leaves no leak in
  stdout, stderr or events (sentinel checks).
- The evidence runs in CI, or its run records are archived and linked from the P11 qualification
  record and `verification.md`.
- The known-limits entry and issue #188 are closed with links to the evidence.

## Engineering notes for whoever implements it

These are neutral facts found during P11. They are not a design.

- The first media step on a new tool pair runs VSift's built-in media-tool verification. A
  stand-in provider is therefore also invoked by that preflight, before any requested work.
- The process supervisor clears the child environment and passes only allowlisted variables, so a
  stand-in cannot be configured through arbitrary environment variables.
- A hardened container running the worker needs writable, bounded scratch mounts (for example
  tmpfs) for the per-user configuration, the workspace and the verification working area. Its root
  stays read-only.

## Related documents

- `docs/planning/verification.md` (SEC-T01)
- `docs/planning/security-threat-model.md` (SEC-04, SEC-05, SEC-06, SEC-19, SEC-25)
- `docs/planning/p11-worker-host.md` (P11 qualification record)
- `docs/operations/worker-host.md` (operator runbook)
- `docs/decisions/0021-worker-and-batch-host.md`
- `docs/planning/known-limits.md`
