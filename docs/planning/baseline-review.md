# Review of the existing foundation

Review date: 2026-09-09. Source: `df85f70`. Method: source and configuration inspection.
These are observed design gaps, not a penetration-test certification or claims of
exploitation. No malicious binaries or media were executed for this review.

## Findings that must precede media ingestion

| ID | Evidence | Exposure / limitation | Planned resolution |
| --- | --- | --- | --- |
| B-01 | `process_dependency_probe.rs`: `Command::output()` followed by 240-character truncation | Both streams are collected before truncation. A noisy executable can consume unbounded memory within its deadline | P02: stream both pipes with hard byte caps and kill/reap on violation |
| B-02 | Same adapter: bare `ffmpeg`, `ffprobe`, `whisper-cli` names | Executable provenance depends on process search rules and local PATH; calling something a dependency does not make it trusted | P02/P06: approved absolute path, sanitized environment and working directory, explicit trust policy |
| B-03 | Same adapter: `kill_on_drop(true)` and a timeout | No explicit descendant containment, graceful escalation, or verified cleanup/reaping contract | P02: platform process supervisor and destructive lifecycle tests in isolated fixtures |
| B-04 | Available = successful exit with first non-empty line | Version compatibility, executable identity, model availability and actual operation capability are unverified | P06: registry of tested provider versions, profile-specific readiness, opt-in smoke tests |
| B-05 | `print_human` emits external version/diagnostic text; error strings are merely shortened | ANSI/control sequences and sensitive path details may reach terminal/logs | P01/P02: bounded sanitization, structured error codes and safe display fields |
| B-06 | CLI uses `Cli::parse`, `println!`, and stderr-only serialization failure | Invalid JSON-mode requests and broken pipes do not have a complete documented result/error protocol | P01: explicit parse/output boundary, fixtures for failure and pipe closure |
| B-07 | CLI integration test inherits machine PATH and accepts status 0 or 2; checks JSON substrings | It does not prove structural JSON validity, deterministic readiness, isolation, deadlines or failure contracts | P01/P02: isolated fixture executables and full schema/semantic assertions |
| B-08 | Application probes serially with a per-probe timeout | Total deadline differs from one probe deadline, and there is no caller cancellation contract | P02: overall operation deadline, explicit cancellation and ordered results |
| B-09 | Architecture prose attributes resource isolation to child processes | Child processes are not inherently memory-limited, filesystem-confined or network-isolated | Correct documentation now; enforce/report platform limits in P02/P11 |
| B-10 | There is no session/storage/job implementation | No established locking, recovery, idempotency, retention, durability or concurrent publication guarantee | P03/P10/P11 before server readiness claims |
| B-11 | Workflow completion is required; security scanning is configured | Successful scanner execution is not proof of absence of findings; native tools/models are outside Cargo lockfile scanning | P14: finding-aware release gate and runtime inventory review |

Priority: B-01 through B-07 are prerequisites for executing untrusted inputs through
the media pipeline. B-10 is a prerequisite for concurrent and durable use. The current
diagnostic command is a scaffold and must be described as such until those gates pass.

## Existing strengths to retain

- Explicit crate boundaries, typed readiness states and provider port.
- Shell-free argument arrays, timeout intent, no unsafe code in owned crates.
- Locked Rust dependencies, strict linting and three-platform CI.
- SHA-pinned Actions, dependency review, CodeQL and repository protection.
- Clear source-authority and disposable-storage decisions.

## Limits and follow-up

No media decoder, installer, network listener, archive parser, database or worker
scheduler currently exists in the implementation. Their threats in the accompanying
model are prospective design requirements. Eight existing tests are useful scaffold
checks, but do not substantiate production resilience or media accuracy.

The proposed plan must not be used as evidence that a mitigation has shipped. Each
finding closes only with its implementation PR and regression test references.

## 2026-09-10 P01 disposition

PR #20 (`3d7a7d2a53fd8e6d2025726d34c59e7603fc8e71`) closed B-06 with an
explicit bounded output/parse boundary and structural regression tests. It completed
the P01 portion of B-05 (safe public rendering) and B-07 (deterministic semantic/schema
CLI assertions). Provider-pipe bounding, executable isolation, and process deadline
proof remain open under P02; those findings are not closed by the public contract.
