# Development

## Prerequisites

- Git
- Rustup
- The Rust toolchain declared in `rust-toolchain.toml`

FFmpeg and Whisper are not required to compile or run unit tests. Integration tests that require specialist runtimes must detect and report their prerequisites explicitly.

## First build

```console
git clone https://github.com/smormah/vsift.git
cd vsift
cargo build --workspace --locked
cargo test --workspace --locked
```

Cargo automatically selects the repository toolchain through `rust-toolchain.toml`.

## Required checks

Run these before opening a pull request:

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo doc --workspace --no-deps --locked
```

CI runs the same checks on Windows, macOS, and Linux.

If you changed a parser the fuzz targets use, or anything in `fuzz/`, also run the
fuzz harness checks (the workspace excludes `fuzz/`, so the commands above skip it):

```console
cargo fmt --manifest-path fuzz/Cargo.toml --check
cargo clippy --manifest-path fuzz/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path fuzz/Cargo.toml --locked
```

## Fuzzing

`fuzz/` holds `cargo-fuzz` targets for the parsers of untrusted input
([ADR 0016](decisions/0016-embeddable-engine-and-evidence-contract.md), decision 6):
`transcript_srt`, `transcript_webvtt`, `whisper_full_json`, `transcript_record`,
`ffprobe_metadata`, `transcript_cursor` and `search_query` (P08: query normalisation and
matching over a query, a line feed and segment text). It is a separate package with its own
lockfile. Each target body is a plain function in `fuzz/src/lib.rs`; the stable replay
tests above run it over every seed in `fuzz/seeds/<target>/`, and the libFuzzer entry
points in `fuzz/fuzz_targets/` (feature `libfuzzer`) run it under libFuzzer.

libFuzzer needs a nightly toolchain, so the `Fuzz` workflow runs it weekly and on
manual dispatch, never as a per-PR check. To fuzz locally on Linux (x86-64, with a C++
compiler), use the workflow's pinned versions:

```console
rustup toolchain install nightly-2026-09-01 --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
mkdir -p fuzz/corpus/transcript_srt
cargo +nightly-2026-09-01 fuzz run --features libfuzzer transcript_srt \
  fuzz/corpus/transcript_srt fuzz/seeds/transcript_srt -- -max_total_time=300 -timeout=10
```

The first corpus directory collects new inputs and, like `fuzz/artifacts/`, is ignored
by Git. A crash leaves its input in `fuzz/artifacts/<target>/`; reproduce it with
`cargo +nightly-2026-09-01 fuzz run --features libfuzzer <target> <crash-file>`. Every
crash becomes a permanent regression test at the parser's own layer, not in `fuzz/`.
Add a seed only by copying an existing reviewed fixture into `fuzz/seeds/<target>/` and
listing its origin in `fuzz/tests/replay.rs`; the replay tests fail on an unlisted or
drifted seed. On Windows with MSVC, the same commands work when the directory holding
`clang_rt.asan_dynamic-x86_64.dll` (the MSVC `bin\HostX64\x64` directory) is on `PATH`.

## Architectural placement

Before adding code, identify its owner:

| Concern | Crate |
| --- | --- |
| Business concept or invariant | `vsift-domain` |
| Use-case orchestration or provider port | `vsift-application` |
| Filesystem, process, network, model, or storage implementation | `vsift-infrastructure` |
| Versioned JSON wire type, or mapping a domain/application value into one | `vsift-contract` |
| Composing use cases and adapters into an operation every host can call, or its typed result and error | `vsift` (engine) |
| Argument parsing, configuration precedence, human output, exit codes, or building the engine | `vsift-cli` |

A host depends on `vsift` and `vsift-contract` only. When a host needs a new capability,
add a typed engine operation rather than reaching into the application or
infrastructure crates. The `vsift` library API is 0.x and unstable; only the CLI and its
v1 JSON are stable public surfaces.

Do not create a general-purpose `utils` or `helpers` module. Name modules after the capability or concept they own.

## Testing

- Domain tests verify invariants without I/O.
- Application tests use small explicit fakes for ports.
- Infrastructure tests exercise real boundaries using isolated temporary directories and rights-safe fixtures.
- Engine tests use `vsift` as a library, without the CLI, with an injected controlled
  clock, a sequential identifier source and temporary directories, so identities and
  expiry are exact. Tests that need real FFmpeg/FFprobe are `#[ignore]`d and opt-in.
- Contract tests serialize `vsift-contract` values and validate them against
  `schemas/v1` and its frozen examples, without running the CLI.
- CLI tests execute the compiled binary and verify public output and exit codes.
- JSON changes require compatibility-focused contract tests.

Tests must be deterministic and must not depend on internet access. Media fixtures must be generated by the project or have documented redistribution rights.

## Dependencies

Before adding a crate, review:

- whether the standard library already solves the problem clearly;
- maintenance activity and security history;
- transitive dependency cost;
- platform support and minimum Rust version;
- licence compatibility;
- whether it introduces native build requirements.

Commit `Cargo.lock` because VSift is an application. Dependency changes must pass `cargo deny check` in CI.

The fuzz harness's lockfile (`fuzz/Cargo.lock`) adds only `libfuzzer-sys` 0.4.13,
`arbitrary` 1.4.2 and `jobserver` 0.1.35 (a build helper of `cc`) to the workspace's
graph; its review, and the
open licence question for `libfuzzer-sys`'s declared NCSA term, are in the ADR 0016
note of 2026-09-24.

P02 uses `process-wrap` 10 for safe cross-platform access to Windows Job Objects and
Unix process groups. Its enabled features, MSRV, transitive footprint and isolation
limitations are recorded in [ADR 0003](decisions/0003-external-runtime-adapters.md).

## Documentation

P03's cap-std/cap-fs-ext 4.0.3 storage dependencies and their review are in
the [storage feasibility record](planning/p03-storage-feasibility.md). Reproduce
the bounded probe with `cargo test --locked -p vsift-infrastructure --test
storage_feasibility -- --nocapture`. Report filesystem identity with the result;
these observations qualify API behavior, not durable publication. The production
adapter also uses the already-transitive Rustix 1.1 process feature on Unix to verify
root ownership, and Windows-only `windows-acl` 0.3.0 for read-only DACL inspection.
The latter is MIT licensed and unarchived but has low maintenance activity (last push
2023-06-09); its narrow use is isolated behind `cfg(windows)`, cargo-deny reports no
advisory, and replacement remains appropriate if a maintained safe DACL reader appears.
`deny.toml` explicitly includes development dependencies in licence and duplicate
checks; cargo-deny otherwise omits them from these checks by default. Keep the
explicit settings when evaluating future test-only platform wrappers.

Public behaviour belongs in the README or command documentation. Architectural decisions belong in `docs/decisions`. Security assumptions belong in `SECURITY.md` and the relevant design document.
