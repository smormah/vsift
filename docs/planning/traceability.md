# R0 delivery traceability

Status: accepted scope mapping. The machine-readable source is
[`delivery-ledger.json`](delivery-ledger.json); CI validates its structural invariants.

| Requirement | Primary packets | Test groups | Corpus fixtures |
| --- | --- | --- | --- |
| R-01 agent video investigation | P04, P07-P09, P12 | M, T, V, A | F01-F10, F12 |
| R-02 stable CLI/JSON | P01 | C-01..C-10 | F01 |
| R-03 dependency lifecycle | P02, P06 | P, D | F01, F08 |
| R-04 transcript import/local ASR | P07 | T-01..T-06 | F08-F10 |
| R-05 source visual/audio retrieval | P04, P08, P09 | M, V | F01-F07, F09 |
| R-06 provenance and uncertainty | P01, P04, P07-P09 | C-10, M-05, T, V | F02-F10 |
| R-07 explicit session lifecycle | P03, P05 | S | F01, F05 |
| R-08 concurrent host execution | P02, P03, P10, P11 | P, S, X | F01, F05, F11 |
| R-09 recovery/idempotency | P03, P10 | S-07..S-12, X-01..X-06 | F01, F05, F11 |
| R-10 durable worker workspace | P03, P10, P11 | X-01..X-11 | F01, F05, F11 |
| R-11 resource budgets | P02, P04, P11 | P-03..P-08, M-02..M-03, X-07..X-09 | F06, F11 |
| R-12 headless observability | P01, P11 | C, O | F01, F05, F11 |
| R-13 agent skill/handoff | P12 | A-01..A-07, SEC-T02 | F03-F08, F12 |
| R-14 distribution/provenance | P06, P13, P14 | D, R-SEC01..03 | F01 |

The ledger also records decision status, packet dependencies, issue/PR evidence and
the immutable project invariants. A requirement is complete only after every primary
packet is complete and its release proof is attached. Packet completion cannot be
inferred from code presence or an assistant's status message.

## Scope guardrails

R0 owns local media input, disposable/retained workspaces, bounded single-host worker
execution, evidence retrieval and native/npm distribution. It excludes OCR,
embeddings, model-generated frame captions, automatic stitching, cross-video catalogue,
SQLite, remote URLs, HTTP/MCP servers, tenancy and cloud selection. An excluded item
needs an explicit scope ADR and ledger change before implementation.

When a proposed change cannot cite an R0 requirement and packet, place it in the R1/R2
backlog rather than expanding an active packet. A security or correctness fix may be
accepted immediately but still needs a mapped threat/finding and regression proof.
