# R0 delivery and R1 scope traceability

Status: accepted scope mapping. The machine-readable source is
[`delivery-ledger.json`](delivery-ledger.json); CI validates its structural invariants.

| Requirement | Primary packets | Test groups | Corpus fixtures |
| --- | --- | --- | --- |
| R-01 agent video investigation | P04, P07-P09, P12 | M, T, V, A | F01-F10, F12 |
| R-02 stable CLI/JSON | P01 | C-01..C-10 | F01 |
| R-03 BYO dependency readiness | P02, P06 | P, D | F01, F08 |
| R-04 transcript import/local ASR | P07 | T-01..T-06 | F08-F10 |
| R-05 source visual/audio retrieval | P04, P08, P09 | M, V | F01-F07, F09 |
| R-06 provenance and uncertainty | P01, P04, P07-P09 | C-10, M-05, T, V | F02-F10 |
| R-07 explicit session lifecycle | P03, P05 | S | F01, F05 |
| R-08 concurrent host execution | P02, P03, P10, P11 | P, S, X | F01, F05, F11 |
| R-09 recovery/idempotency | P03, P10 | S-07..S-12, X-01..X-06 | F01, F05, F11 |
| R-10 durable worker workspace | P03, P10, P11 | X-01..X-11 | F01, F05, F11 |
| R-11 resource budgets | P02, P04, P11 | P-03..P-08, M-02..M-03, X-07..X-09 | F06, F11 |
| R-12 headless observability | P01, P11 | C, O | F01, F05, F11 |
| R-13 agent skill/handoff | P12 | A-01..A-09, SEC-T02 | F01, F03-F10, F12 |
| R-14 distribution/provenance | P06, P13, P14 | D, R-SEC01..03 | F01 |

The ledger also records decision status, packet dependencies, issue/PR evidence and
the immutable project invariants. A requirement is complete only after every primary
packet is complete and its release proof is attached. Packet completion cannot be
inferred from code presence or an assistant's status message.

P01 executable evidence is mapped case-by-case in the published
[v1 CLI contract](../contracts/cli-v1.md). That evidence qualifies the public boundary,
not the later media, storage, process, or worker implementations.

## R1 scoped mapping

R1 implementation is not active. The detailed mapping is frozen for P15 review in the
[industrial capability expansion](r1-industrial-capability-expansion.md); P15 must add
its own machine-readable ledger and independent fixture truth before P16 begins.

| Requirement | Primary packets | Test groups |
| --- | --- | --- |
| R-15 optional bounded enrichment | P15, P16, P20 | E-01..E-08, Q-07/Q-08 |
| R-16 source-grounded composition | P15, P17, P20 | RC-01..RC-08, Q-01/Q-05 |
| R-17 explicit managed catalogue | P15, P18, P20 | I-01..I-12, Q-05/Q-06/Q-09 |
| R-18 industrial job control | P15, P19, P20 | H-01..H-12, Q-03..Q-06 |
| R-19 production operations/security | P15, P19, P20 | H-08..H-12, Q-03..Q-10 |
| R-20 two-agent R0 parity | P15, P20 | A-01..A-09, Q-01/Q-02 |

P20 also reruns every applicable R0 test group. R1 cannot close a requirement by
testing only its new adapter while the underlying video investigation lifecycle is
broken.

## Scope guardrails

P03 completed in PR #42 (`3eef9b7`) under ADR 0010's narrower qualification profile.
It implements the P03 portions of R-07..R-11: private owned roots,
handle-relative metadata, stable locks, weighted admission, generation publication,
read/cleanup holds and process-crash recovery. S-03 at this packet proves stable
storage snapshots and detects identity/integrity changes; P04 implements actual media
source binding and staging under ADR 0012. P05 owns lifecycle deletion, while P10/P11/P14 retain
durable stage acknowledgement, the Ubuntu/ext4 OS/storage campaign and strict-worker
release proof. Explicit durable requests continue to fail before mutation.

R0 owns local media input, disposable/retained workspaces, bounded single-host worker
execution, evidence retrieval and native/npm distribution. It excludes OCR,
embeddings, model-generated frame captions, automatic stitching, cross-video catalogue,
SQLite, remote URLs, HTTP/MCP servers, tenancy and cloud selection. An excluded item
needs an explicit scope ADR and ledger change before implementation.

When a proposed change cannot cite an R0 requirement and packet, map it to the scoped
R1 document or the R2 backlog rather than expanding an active packet. R1 scope is not
permission to pull P15+ implementation ahead of P14. A security or correctness fix may
be accepted immediately but still needs a mapped threat/finding and regression proof.
