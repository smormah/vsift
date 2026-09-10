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
