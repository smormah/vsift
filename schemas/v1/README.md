# VSift JSON schemas v1

These files are the machine-readable public v1 boundary:

- `setup-check-response.schema.json` — backward-compatible setup diagnosis;
- `setup-plan.schema.json` — current read-only reviewed-catalogue plan and
  typed managed-unavailable states; a digest never authorizes installation by
  itself;
- `setup-plan-unqualified.schema.json` — historical P06 check-first response
  before catalogue acceptance, retained for v1 compatibility evidence;
- `operation-response.schema.json` — terminal result for new operations;
- `terminal-event.schema.json` — JSONL terminal wrapper (validate its `result` with
  `operation-response.schema.json` too);
- `config.schema.json` — strict explicit configuration document reserved for P06.

Every file under `examples/` is a frozen valid instance checked by the Rust contract
suite. Response readers must tolerate additive fields within major v1. Strict request
and configuration readers reject unknown fields. Unknown major versions are rejected.

See the human-readable [CLI contract](../../docs/contracts/cli-v1.md) for limits,
exit codes, lifecycle semantics, and implementation status.
