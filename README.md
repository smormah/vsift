# VSift

VSift gives AI coding agents local, structured access to the evidence inside technical videos.

The initial use case is a recorded QA walkthrough: VSift combines timestamped speech with relevant visual states so an agent can investigate the demonstrated problem without requiring the user to transcribe the recording or capture screenshots manually.

> **Project status:** contract stage. The v1 CLI/JSON boundary is published; the media
> pipeline is not implemented yet.

The accepted [implementation blueprint](docs/planning/README.md) covers the desktop
and server-worker design, security review, test matrix and delivery work packets.
The [delivery ledger](docs/planning/delivery-ledger.json) and CI governance check guard
scope and completion evidence. See the [v1 CLI contract](docs/contracts/cli-v1.md) for
the exact implemented-versus-reserved boundary.

R0 is intended to be a complete product journey: a compatible coding agent receives a
local video, obtains or imports a timestamped transcript, retrieves relevant visual
evidence and produces a grounded handoff without manual audio extraction or screenshot
collection. Release qualification will exercise that flow through named OpenAI Codex
and Claude Code clients with local shell and image access. The scoped
[R1 industrial expansion](docs/planning/r1-industrial-capability-expansion.md) adds
managed cross-video indexing, enrichment, reconstruction and operated worker growth;
it is not required to make the first release useful.

## Principles

- **Local first:** routine inspection does not require uploading recordings to a hosted service.
- **Disposable by default:** planned investigation sessions expire unless explicitly retained; physical cleanup requires a subsequent invocation or host maintenance.
- **Source grounded:** the original video and audio remain authoritative.
- **Agent friendly:** commands provide stable, versioned JSON alongside readable terminal output.
- **Provider neutral:** FFmpeg, transcription engines, OCR, and future integrations sit behind explicit boundaries.
- **Security requirements:** shell-free processes, verified downloads, bounded execution and contained cleanup; see the [baseline review](docs/planning/baseline-review.md) for current implementation gaps.

## Current behavior

The first walking skeleton provides runtime diagnostics:

```console
vsift setup check
vsift setup check --json
vsift setup check --events jsonl
```

The full R0 command namespace is visible through `vsift --help` so integrations can
target a stable grammar. Until its owning work packet ships, every other operation
returns the typed `COMMAND_NOT_IMPLEMENTED` result and exit 2; no media or storage work
is implied by successful argument parsing.

FFmpeg and FFprobe are required for media processing. A compatible Whisper backend enables local transcription but is not required when a usable transcript already exists.

## Architecture

VSift is a native Rust CLI distributed through native installers and, eventually, npm. Specialist media and machine-learning tools run as isolated external processes.

```text
CLI -> Application -> Domain
          ^
          |
    Infrastructure
```

See [Architecture](docs/architecture.md) and the [architecture decision records](docs/decisions/README.md) for the governing design.

## Development

Install the stable Rust toolchain, including `rustfmt` and Clippy, then run:

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

Detailed setup and repository conventions are in [Development](docs/development.md). Contributions are welcome; please read [Contributing](CONTRIBUTING.md) and [Security](SECURITY.md) first.

## Data lifecycle

VSift will distinguish three explicit storage lifecycles:

1. Ephemeral investigation sessions, which are the default and expire automatically.
2. Portable evidence bundles retained at a user-selected location.
3. A future opt-in persistent catalogue for cross-video indexing.

No one-off recording will be silently added to a permanent global index.

## Licence

Licensed under either the Apache License, Version 2.0 or the MIT licence, at your option.
