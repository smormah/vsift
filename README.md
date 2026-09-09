# VSift

VSift gives AI coding agents local, structured access to the evidence inside technical videos.

The initial use case is a recorded QA walkthrough: VSift combines timestamped speech with relevant visual states so an agent can investigate the demonstrated problem without requiring the user to transcribe the recording or capture screenshots manually.

> **Project status:** foundation stage. The command contracts and architecture are being established before the media pipeline is implemented.

## Principles

- **Local first:** routine inspection does not require uploading recordings to a hosted service.
- **Disposable by default:** one-off investigations expire unless the user explicitly retains an evidence bundle.
- **Source grounded:** the original video and audio remain authoritative.
- **Agent friendly:** commands provide stable, versioned JSON alongside readable terminal output.
- **Provider neutral:** FFmpeg, transcription engines, OCR, and future integrations sit behind explicit boundaries.
- **Secure by design:** external processes never run through a shell, downloads require integrity verification, and cleanup is constrained to VSift-owned paths.

## Current command

The first walking skeleton provides runtime diagnostics:

```console
vsift setup check
vsift setup check --json
```

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
