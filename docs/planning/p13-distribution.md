# P13 distribution and managed-installation qualification record

Status: recorded 2026-10-01 on branch `p13-pr11-docs`, on `main` at `6de55da`, which holds
P13 PRs 0-10 and the PR 7 follow-up. **The first publish is pending.** Everything below is
complete except the section "First publish", which the ledger follow-up (PR 12) completes
after the maintainer has published the 0.x pre-release. Design:
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(Proposed until P13 completes). Verification IDs: D-02..D-08, R-SEC01, R-SEC02 and SEC-T02
over human output, serving R-03, R-13 and R-14 ([verification](verification.md)). Guides:
[`install.md`](../operations/install.md) (users) and
[`release.md`](../operations/release.md) (the maintainer's runbook). Attached stage of the
[end-to-end spine](e2e-test-spine.md): "Installed-user and managed-dependency run". Dates
are UTC unless a sentence says otherwise.

## Result in plain English

Everything P13 sets out to build is merged and tested. `vsift` prints readable text by
default, says what to fix when a command line does not parse, and checks an agent's draft
report (`handoff check`). On Ubuntu 24.04 it can download, verify, smoke-test and activate
FFmpeg, whisper.cpp and the speech model (`setup install`), and list, roll back, remove and
diagnose them; that store survives kills at every step and, on ext4, power loss. A release
workflow builds byte-identical archives for Windows, macOS and Linux, assembles the npm
packages from them, and installs and runs them with npm, pnpm, Yarn and Bun on all three
systems from a local registry. The attestation, publishing and release steps are wired and
dry-run whenever the Release workflow runs.

**Nothing has been published except an empty placeholder.** The one thing no test could
exercise, the attestation and publishing path against GitHub and npm, has not run, and the
maintainer's release settings (the `release` environment, the tag ruleset, trusted
publishers) are not made. P13 is therefore **not complete**. It completes when the
maintainer has made those settings, published the 0.x pre-release and verified it, and the
follow-up PR 12 has recorded that here and in the ledger.

**What is weaker than it sounds**, stated once here and again where each claim is made:

- The npm matrix proves the packages install and run from a **local** registry on
  GitHub-hosted runners, which ship a Rust toolchain on `PATH` that no tested step invokes.
  It is not a clean-machine install from the real registry; that is the next packet's.
- The power-loss claim is for **Ubuntu 24.04 with ext4** only, from one hosted run on small
  stand-in versions, and its first run failed on a verifier defect that was then fixed.
- The offline install (`--artifact-dir`) has run only with stand-in artifacts, never with
  the real reviewed ones.
- The Windows and macOS prompts for an unsigned download are described from the operating
  systems' documentation; none has been observed on a VSift archive, and Windows Smart App
  Control may block `vsift.exe` outright ([L-098](known-limits.md#l-098)).
- The repository settings that make R-SEC01 true at run time do not exist yet.

## Scope delivered

1. **Native archives and the release pipeline** (PR 8, 10): `release.yml`; reproducible
   `.tar.gz` archives for `x86_64-pc-windows-msvc`, `aarch64-apple-darwin` and
   `x86_64-unknown-linux-gnu` with licences, `THIRD-PARTY-NOTICES`, a CycloneDX SBOM, the
   skill and `SHA256SUMS`; `vsift --version` names the commit; the governance lint of every
   workflow (R-SEC01); the `plan`, `attest` and `publish` jobs, `dry_run` by default.
2. **The npm launcher** (PR 9): `vsift-cli` (plain CommonJS, no install scripts) over
   `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64`, qualified on a
   loopback Verdaccio with four package managers on three systems.
3. **Managed installation on Ubuntu 24.04 x64** (PRs 3, 4, 6, 7): smoke executor, guarded
   install transaction, managed lookup tier, `setup install`, `--artifact-dir`,
   `DOWNLOAD_FAILED`, `setup list/rollback/remove/repair`, bounded cleanup, stale-stage
   sweep, kill and power-loss qualification.
4. **Human-readable output** (PR 2a, 2b) with the SEC-T02 rerun over it; **typed parse
   remediation** (PR 1, closes L-071); **`vsift handoff check`** (PR 5, #213).
5. **Documentation** (PR 11): ADR 0023 and its notes, `install.md`, `release.md`, this
   record, the spine's P13 stage, verification, threat model and known limits.

Not in P13 (unchanged): the release qualification of the next packet, managed installation
on Windows and macOS, native installers, crates.io, the MCP adapter, SEC-T01's adversarial
evidence (L-068) and the P12 debt that is not the compact re-run.

## Pull requests

All are squash merges to protected `main`, listed in the order they merged. The merge
commits are from `gh pr view` on 2026-10-01.

| PR | Pull request | Merge commit | Merged |
| --- | --- | --- | --- |
| 0 | #226 ADR 0023 (Proposed), the plan, ledger `in_progress` | `dbc60f7` | 2026-09-30 |
| P12 debt | #227 skill wording and loop-period grader fixes (#218-#221, #224) | `27dadbe` | 2026-09-30 |
| 1 | #228 typed parse remediation, terminal-safe diagnostics (L-071) | `88ef9bf` | 2026-09-30 |
| 2a | #229 human output part 1, `TerminalText`, SEC-T02 rerun | `bffc6bb` | 2026-09-30 |
| 3 | #230 managed-install smoke executor and failure cleanup | `e22ee59` | 2026-09-30 |
| 2b | #231 human output part 2, path display, SEC-T02 rerun complete (L-073) | `02df4eb` | 2026-09-30 |
| 5 | #233 `vsift handoff check`, shared validator, skill and grader input exception | `1a9d027` | 2026-09-30 |
| names | #235 scope `@shongo` and a `vsift` placeholder (withdrawn the same day) | `68e577a` | 2026-09-30 |
| 4 | #234 install transaction, managed lookup tier, `DOWNLOAD_FAILED`, plan intent vs observed state | `d43a518` | 2026-09-30 |
| names | #237 scope is `@vsift` after all | `6637229` | 2026-09-30 |
| P12 debt | #238 trial grader: `commands_only` always allows `handoff check` | `a0bfb06` | 2026-09-30 |
| 8 | #236 `release.yml`, reproducible archives, governance workflow lint | `772ead2` | 2026-09-30 |
| 6 | #239 `setup list/rollback/remove/repair`, bounded cleanup, stale-stage sweep | `02a8f76` | 2026-09-30 |
| 9 | #240 npm packages (`vsift-cli`, `@vsift/*`) and the Verdaccio matrix | `951226f` | 2026-09-30 |
| P12 debt | #242 compact-tier re-run #222 meets the target; `rg --files` reading; records | `57796a0` | 2026-09-30 |
| 7 | #241 kill tests, directory flushes, power-loss campaign, install E2E stage | `01656d6` | 2026-09-30 |
| 10 | #243 release plan, Sigstore attestation, gated npm and GitHub publish wiring | `57f03fe` | 2026-10-01 |
| 7 follow-up | #244 power-loss checker accepts the in-flight command's state | `6de55da` | 2026-10-01 |
| 11 | #245 this pull request: documentation, this record, closing sweep | recorded by PR 12 | |
| 12 | the ledger follow-up with the merge commit (governance rule 9) | pending | |

## Decisions A to H and how each was met

| Decision | Outcome | Evidence and amendments |
| --- | --- | --- |
| A. Names | Launcher `vsift-cli` (command `vsift`) over `@vsift/win32-x64`, `@vsift/darwin-arm64`, `@vsift/linux-x64` | Two amendments on 2026-09-30. The scope: npm's "not available" for the organisation `vsift` was a repeated submission after the first had created it; `@shongo` was chosen briefly (#235, merged 14:32 UTC) and withdrawn about an hour later (#237). The launcher: npm refused the unscoped `vsift` as too similar to `sift` and `tsify` when the maintainer published a placeholder, so `vsift-cli` holds the name. [ADR 0009](../decisions/0009-package-identity-and-distribution.md) notes; the lesson there: a not-found lookup is not availability, only a publish proves a name |
| B. What is published, and when | Nothing during the pull requests; one 0.x pre-release under `next` at completion; no crates.io | Held so far. The only publishes are the maintainer's four `0.0.0` placeholders: `vsift-cli` on 2026-09-30 and the three `@vsift/...` packages on 2026-10-01. An anonymous registry read on 2026-10-01 shows `vsift-cli` with that one version, published 2026-09-30 at 21:59, as `latest`, and no `bin`; the three `@vsift/...` packages were published as `0.0.0` placeholders on 2026-10-01 (see the maintainer steps below). The pre-release is pending |
| C. Trust signals | Sigstore attestation and npm provenance only; no Authenticode, no notarization | Wired (PR 10), never run ([L-096](known-limits.md#l-096)). What an unsigned download means for users is in `install.md` section 4 ([L-098](known-limits.md#l-098)) |
| D. Platforms | Windows 11 x64 (MSVC, static C runtime), macOS 15 arm64, Linux x64 glibc built on Ubuntu 22.04 | The Release workflow builds each target twice and requires identical executables: passed on `main` at `951226f` ([run 36786019996](https://github.com/smormah/vsift/actions/runs/36786019996)) and `57f03fe` (run 36797351652). Windows needs `-C link-arg=/Brepro` (ADR 0023 PR 8 note) |
| E. Managed installation on Ubuntu 24.04 x64 only | Met | The two hosted managed-smoke runs below; Windows and macOS keep typed manual guidance (`unavailable_target`, engine tests and the P06 stage) |
| F. Order of work | P12 debt first, the compact tier re-run once after `handoff check` | #227 first; the re-run (#222) ran on `a0bfb06` and met the target: Sonnet 5.5 26 of 28, GPT-6-Sol 28 of 28 after the maintainer's `rg --files` reading (23 of 28 as run); L-085 closed ([P12 record](p12-agent-qualification.md)) |
| G. How a draft reaches `handoff check` | Standard input or `--file`; two literal skill forms | PR 5; the guard and the grader recognise exactly those forms ([ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md) notes) |
| H1. The validator answers | Met (PR 5) | `handoff_cli_contract`, `handoff_differential` |
| H2. Human output by default | Met (PR 2a, 2b) | Every command that completes has a renderer; one without is `INTERNAL` |
| H3. `DOWNLOAD_FAILED` | Met (PR 4) | Six reasons, exit 7; D-07 row |
| H4. In-place v1 edits before the first publication | Used in PRs 4, 5 and 6 | [ADR 0008](../decisions/0008-cli-and-json-contract.md) notes; after the first publish v1 changes are additive only |
| H5. Launcher checks the digest if under 50 ms | Met (PR 9) | Median 4.6 to 10.8 ms on the hosted runners (ADR 0023 PR 9 note, Release run 36772356382); the matrix fails above 50 ms |
| H6. Node.js 22, Bun 1.2 | Met | The matrix runs the minimums, 22.23.3 and 1.2.23 ([L-092](known-limits.md#l-092)) |
| H7. Skill inside the npm package and every archive | Met (PRs 8, 9) | Byte-identical, read back by `vsift-release verify` and `npm-verify` |
| H8. R-13 joins P13 | Met (PR 0) | Ledger and traceability |
| H9. Power-loss qualification of the managed store | Met, and strengthened on review to "a command that reported success survives a power loss" | [Run 36829198545](https://github.com/smormah/vsift/actions/runs/36829198545), below |
| H10. No named-agent re-run from a clean install | Kept | The spine's distribution checkpoint, agent part, is the next packet's |

## Method and environments

Evidence comes at the lowest layer that can show it:

1. **Contract, application and infrastructure tests** on every pull request, on Ubuntu,
   macOS and Windows, with local HTTP, TLS and proxy servers and stand-in artifacts.
2. **The kill matrix** (`vsift-infrastructure/tests/p13_install_transaction/kill.rs`, every
   CI OS): every arrival of every one of the 22 `managed-*` fault points on Linux, the
   first arrival of each elsewhere, plus kills through the operating system.
3. **Hosted real-network jobs on Ubuntu 24.04**, by manual dispatch only: workflow `P13
   managed smoke` (three jobs) and `P13 managed power loss`.
4. **The Release workflow** on every pull request and push that changes an archive or npm
   input (and by dispatch): builds, archives, the twelve-job npm matrix and the publish
   plan.

| Item | Value |
| --- | --- |
| Hosted runners | `ubuntu-24.04` (image 20260920.314.1 in the runs read), `windows-2025`, `macos-15` |
| Rust toolchain | 1.98.1, the repository's pinned toolchain, which `rustup` installs on the runner; hosted runners have `rustup` and `cargo` on `PATH` |
| npm matrix tools | Node.js 22.23.3 with its npm, Bun 1.2.23, pnpm 12.8.1, Yarn 4.18.1, Verdaccio 6.10.4 |
| Publish job tools (not yet run) | Node.js 24.21.0 with npm 11.19.0 |
| Release tools | cargo-about 0.9.2, cargo-cyclonedx 0.5.9 |
| Product evidence of earlier packets | Windows 11 (L-035); nothing in P13 was measured on a developer machine and cited here |

## Hosted runs

Each run was read with `gh run view` on 2026-10-01. The two runs on `01656d6` were created
at 23:50 UTC on 2026-09-30; the ADR notes and the verification rows date them 2026-10-01
(local time).

| Run | Workflow and commit | Result | What it shows |
| --- | --- | --- | --- |
| [36734316384](https://github.com/smormah/vsift/actions/runs/36734316384) | `P13 managed smoke`, `main` at `d43a518`, 2026-09-30 | success (2 jobs) | The real reviewed artifacts downloaded, verified, smoked and activated (install 21.5 s, rerun 0.3 s); the smoke's negative control (a changed banner) discarded all three stages; L-087's timings: warm `setup check` 0.65 s, first `frame get` 0.29 s, warm `frame get` 0.17 s |
| [36793180858](https://github.com/smormah/vsift/actions/runs/36793180858) | `P13 managed smoke`, `main` at `01656d6`, 2026-09-30 | success (3 jobs) | The above again, and the `install-e2e` job: `p13_install: passed` in 47.7 s |
| [36793177930](https://github.com/smormah/vsift/actions/runs/36793177930) | `P13 managed power loss`, `main` at `01656d6`, 2026-09-30 | **failure** | Positive: 1,812 points, 134 acknowledgements, 240 points and 53 acknowledgements reported lost, no damage, clean `e2fsck`. Negative control: 36 lost, 458 torn points. The cause was the checker (below) |
| [36829198545](https://github.com/smormah/vsift/actions/runs/36829198545) | `P13 managed power loss`, `main` at `6de55da`, 2026-10-01 | success | Positive `points=1812 acks=134 lost_acks=0 damaged_points=0 fsck_failures=0 mount_failures=0 torn_points=0`; negative control `points=507 acks=50 lost_acks=36 damaged_points=0 torn_points=458` (it must fail, and does) |
| [36786019996](https://github.com/smormah/vsift/actions/runs/36786019996) | `Release`, push to `main` at `951226f` (PR 9's merge), 2026-09-30 | success | All twelve `npm qualification` jobs (three systems by four package managers) |
| [36797351652](https://github.com/smormah/vsift/actions/runs/36797351652) | `Release`, push to `main` at `57f03fe` (PR 10's merge), 2026-10-01 | success | The three builds, packaging, the twelve qualification jobs and the publish plan job in dry-run mode ("started by `push`, which never publishes"); `attest` and `publish` skipped |
| [36794417916](https://github.com/smormah/vsift/actions/runs/36794417916) | `Release`, PR 10's branch at `2bb5f47`, 2026-10-01 | success | The same on the pull request |
| [36772356382](https://github.com/smormah/vsift/actions/runs/36772356382) | `Release`, PR 9's branch at `2f0d65c`, 2026-09-30 | success | The first all-green matrix. Earlier runs of that branch failed and found real defects (Yarn's one-day gate, Bun on Windows, `bunx --bun`; ADR 0023 PR 9 note): the matrix can fail |

## Evidence per requirement

"Proves" is what the evidence shows; "Does not prove" is what a reader might wrongly take
from it. Test files are in the rows of [verification](verification.md) sections 2 and 7.

| ID | Evidence | Proves | Does not prove |
| --- | --- | --- | --- |
| **D-02** checksum, manifest, stale authorization | `p13_setup_install_cli` (every CI OS), `vsift-infrastructure` and `vsift-application` `p13_install_transaction`, the two hosted smoke runs | Only the compiled catalogue is a trust anchor; a plan is accepted only when it equals a freshly rebuilt one and the digest matches; every byte is checked by exact size and SHA-256 before extraction; altered bytes are never activated; the real artifacts pass on a hosted runner | A signature (none exists: checksums are pinned in reviewed source), a publisher compromised before review, or that the publishers keep the files ([L-099](known-limits.md#l-099)) |
| **D-03** network drop, resume, disk full | `vsift-infrastructure/tests/p13_install_transaction.rs` against local servers, every CI OS | A dropped body is `DOWNLOAD_FAILED` (`offline`), leaves no stage, and the rerun restarts at byte zero with no `Range` header; `206` is refused; a full disk (injected) keeps the previous version | Behaviour on a genuinely flaky network: the hosted runs had a good one |
| **D-04** archive traversal, bombs, substitution | The P06 bounded archive readers, PR 3's smoke and layout checks, the kill matrix; the real archives pass on hosted runs | Bounded, flat, private staging; an executable is reviewed in its native format before it runs; cleanup removes only what it proves its own | Resistance to a hostile archive from the wild: the fixtures are synthetic, and a hostile archive would also have to carry the pinned SHA-256 |
| **D-05** concurrent installs, interrupted activation, rollback under a job | Kill matrix (22 points, six scenarios, OS kills); `every_commit_step_is_flushed_before_the_next`; the power-loss run 36829198545 | A kill at any step leaves a store every command can read, `setup repair` describes exactly, and a rerun completes; every changed folder is flushed before a command returns; on Ubuntu 24.04 with ext4, no replayed power loss lost an acknowledged command or damaged the store, and the negative control (no flushes) did lose acknowledgements, so the campaign can see what it claims to prevent | Any other filesystem or disk: dm-log-writes on one hosted runner, ext4's ordered journal, small one-file stand-in versions and a workload of 150 managed commands, not 232 MB of FFmpeg; Windows has no directory flush and no managed install; macOS runs the same `fsync` unqualified; real disks that ignore flushes ([L-056](known-limits.md#l-056)). **The verifier was changed after the first run failed** (ADR 0023 PR 7 addendum): the change is narrow, tested, and re-classifying the first run's own report under it gives 0 lost in the positive run and 36 in the negative; but the passing run is the second |
| **D-06** missing executable, wrong architecture, smoke failure | `vsift-application/tests/d06_smoke_cleanup.rs`, `vsift-infrastructure/tests/p13_smoke_executor.rs`, the hosted smoke with its negative control | No activation without a passing smoke; every failed stage is discarded or retained and reported | A smoke fixture that matches every real workload: it is a banner, one short clip and one short speech fixture |
| **D-07** TLS failure, proxy auth, redirects, offline import, unavailable target | `p13_install_transaction` (local TLS, proxy and redirect servers), `p13_setup_install_cli`, engine tests for unavailable targets, the real publisher routes on hosted runs | Each `DOWNLOAD_FAILED` reason is produced; proxy credentials never appear in any output mode; an offline folder is verified like a download; other targets get typed manual guidance | Real corporate proxies and TLS interception; **the offline install with the real reviewed artifacts, which has never run** (only stand-ins, and, through the binary, the missing-artifact refusal); that a publisher keeps its redirect hosts ([L-099](known-limits.md#l-099), [L-088](known-limits.md#l-088)) |
| **D-08** uninstall active, unused, external | `managed_store_lifecycle/tests.rs`, `p13_setup_lifecycle_cli`, the kill matrix, the E2E stage (`setup remove whisper_model`, then reinstall) | Removal never touches the selected or a held version, never follows a link, never addresses a tool you registered; a corrupted version is removable | That the user's own deletions or an unusual disk failure leave nothing behind ([L-090](known-limits.md#l-090)) |
| **R-SEC01** fork pull requests cannot reach publish secrets | The governance lint, rules 1-7 (a test of the real `release.yml` and a mutation of it per rule, 28 for rule 7); `tools/vsift-release/src/publish.rs` tests the mode decision for every event, ref, repository and input | A pull request, a push or a fork cannot run `attest` or `publish`, which alone can write or request an OIDC token; actions are pinned; permissions are minimal; the only secret is an optional environment secret | **GitHub's run-time enforcement.** Read on 2026-10-01: no `release` environment, no tag ruleset, fork approval is still "first-time contributors", no tag or release exists. The lint reads the YAML; the settings are the maintainer's ([L-096](known-limits.md#l-096)) |
| **R-SEC02** artifacts match the protected commit; signatures and provenance verified | `vsift-release npm` and `npm-verify`; the plan job's byte-for-byte checks on every run; the launcher's refusals (`npm/test/launcher.test.cjs`, every matrix job) | Tarballs are made only from canonical archives and equal their fresh assembly; the bytes qualified are the bytes the publish job would publish, by SHA-256; a changed, replaced or mismatched package is refused with 126 | **Any attestation or npm provenance: none exists yet.** `gh attestation verify` and `npm audit signatures` have never been run on a VSift file. The launcher check is not a defence against someone who can write to the install ([L-093](known-limits.md#l-093)) |
| **SEC-T02** over human output | `sec_t02_human_output.rs`, the `TerminalText` property tests, the renderers' unit tests, `evidence_cli_contract`, `job_run_cli_contract`, `job_batch_cli_contract`, golden snapshots | Every human output goes through one builder: no raw control, escape, link or hidden character; a hostile delivered path is inert, alone on its line and flagged; L-073 is closed | How a particular terminal draws the bytes (the tests inspect bytes, not a terminal emulator); progress is not rendered in human mode for the worker hosts ([L-017](known-limits.md#l-017)) |
| **R-13** agent skill and handoff | `handoff check` (`handoff_cli_contract`, `handoff_differential`, the skill guard), the compact re-run #222 | The grader and the command run one check; the compact tier meets 90% (93% and 100%, with the maintainer's reading for Sol) | A named-agent run from a clean install (the next packet); the review tier's A-09 blurred re-run ([L-095](known-limits.md#l-095), #224) |
| **R-14** install without Rust through npm, pnpm, Yarn and Bun | The twelve-job matrix (runs 36786019996 and 36797351652) | With scripts disabled, from tarballs identical to the ones the publish job would publish, the launcher and platform package install and run on Windows Server 2025, macOS 15 and Ubuntu 24.04 with each manager, global (Yarn: project) and one-shot; offline; with optional dependencies omitted (127, readable); uninstall is clean (Bun leaves a package, L-092) | **A clean machine, or the real registry.** The registry is a Verdaccio on loopback with no uplink; the runners have a Rust toolchain on `PATH` that no tested step invokes (the qualification driver does not remove it); Yarn's one-day gate is set to zero; Node.js and Bun minimums only; Windows 11 itself is not a runner. **No job downloads, extracts and runs a native archive**: its executable is byte-identical to the one the matrix ran from the platform package and `vsift-release verify` reads the archive back, but `install.md` section 3 is walked by hand only at the first publish |
| **R-03** dependency lifecycle, one managed target | The D rows above, the install E2E stage | On Ubuntu 24.04 x64 the real artifacts install, are used (the A-08 local-ASR journey on the managed tools alone), are killed and rerun, removed and reinstalled | Other Ubuntu versions or Arm; managed installation anywhere else (decision E); a user's real proxy |

## Threats

The per-threat controls and tests are in the threat model's
[P13 notes](security-threat-model.md#p13-notes) and its final-state table.

| Threat | Final P13 state | Still open |
| --- | --- | --- |
| SEC-12 malicious update | Reviewed catalogue only; SHA-256 before extraction; smoke before activation; no update under a job | No signature check exists; publishers' continued hosting ([L-099](known-limits.md#l-099)) |
| SEC-13 archive traversal, bombs | Bounded readers, flat private staging, native-format check, ownership-proving cleanup | Hostile archives from the wild (D-04) |
| SEC-14 resume, redirects, proxy credentials | No resume, reviewed redirect route, six typed reasons, no credential in any output | Real proxies (D-07); `407` by text ([L-088](known-limits.md#l-088)) |
| SEC-15 rollback or removal under a job | Use locks; removal only of unselected, unheld versions; kill and power-loss evidence | ext4 only (D-05) |
| SEC-22 CI steals secrets or replaces a binary | Lint; two privileged jobs, dispatch-only on the tag; publish only qualified tarballs by digest; no long-lived token | The environment, ruleset and trusted publishers do not exist yet |
| SEC-23 checksums from a compromised server | Trust anchor in reviewed source; launcher digest; attestation designed to prove origin | No attestation exists until the first publish |

## Residual known limits

What matters at release; each entry in [known limits](known-limits.md) has the detail.

| ID | Limit | State |
| --- | --- | --- |
| L-036 | Nothing published | Ends with the first publish |
| L-096, L-097 | The attest and publish jobs have never run; a part-way publish is public until a re-run | The first run is the evidence; re-run within 7 days |
| L-037 | Managed install on Ubuntu 24.04 x64 only; power-loss claim for ext4 only | Accepted by decision E |
| L-098 (new) | Unsigned executables meet SmartScreen and Gatekeeper; Smart App Control may block `vsift.exe` | To be tried on real machines before a stable release |
| L-099 (new) | Managed install depends on the publishers' files and redirect hosts | Fails safe; the manual path is the fallback |
| L-087, L-088, L-090 | Rehash cost, `407` recognised by text, unprovable content left for the user | Monitoring or accepted |
| L-089 | SBOM `bom-ref` values name the runner's checkout path | Accepted |
| L-091 | Signal relay can repeat or miss one; a killed launcher cannot stop vsift | Accepted |
| L-092, L-093, L-094 | Matrix coverage (minimum runtimes, Yarn project install, Bun leftovers), digest check reach, Windows 260-character paths | Accepted |
| L-095 | Review-tier A-09 blurred re-run (#224) | Maintainer, before the next packet |
| L-035, L-042 | Platform evidence mostly Windows 11; real-tool paths run on demand | The next packet |
| L-016, L-017, L-086 | Path display residual, no progress in human worker output, `vsift-contract` cannot be packaged for crates.io | Accepted or deferred (no crate is published) |

## Maintainer-only steps

None is done by an agent or a workflow. States as read on 2026-10-01 (`gh api` and an
anonymous registry read); the runbook is [`release.md`](../operations/release.md) section 6.

| Step | State |
| --- | --- |
| The maintainer's npm account (two-factor authentication, per ADR 0009's placeholder note); the organisation `vsift` (scope `@vsift`) | Done 2026-09-30 |
| `vsift-cli@0.0.0` placeholder (README and `package.json` only) | Done 2026-09-30, 21:59 UTC; `latest` |
| Fork pull requests: approval for all external contributors | Done 2026-10-01; read back with `gh api`: `all_external_contributors`, default token read-only, Actions cannot approve pull requests |
| Environment `release` (reviewer, no admin bypass, tag rule `v*`) | Done 2026-10-01; read back with `gh api`: one reviewer, self-review allowed, administrator bypass off (corrected after the first read showed it on), deployment rule type tag `v*` only, no secrets |
| Tag ruleset `v*` | Done 2026-10-01; read back with `gh api`: ruleset `release tags`, active, refs `refs/tags/v*`, rules creation, update, deletion and non-fast-forward, bypass only the Repository admin role |
| First publish of the three `@vsift/...` names (path A placeholders, or path B short-lived token) | Done 2026-10-01 (path A: placeholders `0.0.0` published by the maintainer with two-factor authentication: `@vsift/win32-x64` 17:44 UTC, `@vsift/linux-x64` 17:45, `@vsift/darwin-arm64` 17:46; each `latest`; confirmed by anonymous registry reads) |
| Trusted publishers on all four packages | Done 2026-10-01 as reported by the maintainer: GitHub Actions, `smormah`, `vsift`, `release.yml`, environment `release`, label `release workflow`, allowed actions npm publish ticked and npm dist-tag unticked. npm shows these settings only to the package owner, so they are unverified until the first publish ([issue #246](https://github.com/smormah/vsift/issues/246) tracks whether to move to staged publishing before a stable release) |
| Disallow tokens on the four packages | Done 2026-10-01 as reported by the maintainer (npm's option "Require two-factor authentication and disallow bypass 2fa tokens"); unverified from outside |
| Version, `CHANGELOG.md` release section, tag `v0.1.0` | The version is 0.1.0 in `Cargo.toml` and `npm/vsift-cli/package.json`, and the changelog's `[0.1.0]` section is cut in the release-prep change; the tag is pending (no tags) |
| Dry run on the tag, dispatch with `dry_run` cleared, approval | Pending |
| Verification of the published files and the record of it here | Pending |

Releasing is not "done" by merging: GitHub's required checks cover Quality, Documentation,
dependency policy and review, Rust analysis and Governance; the Release workflow, with its
twelve-job matrix, is not a required check (a maintainer decision, `release.md` 6.6).

## Readings awaiting the maintainer

Choices the implementation pull requests made and flagged for review, each recorded in the
ADR 0023 note of its pull request; none blocks completion.

- **PR 2a, 2b:** paths shown whole with a flag; the worker hosts render only their final
  result (L-017); the `\\?\` note.
- **PR 4:** a digested plan intent beside observed state; development builds resolve no
  publisher host; a failed smoke is `MISSING_CAPABILITY`; the `BUSY` retry hint of 30 s.
- **PR 5:** the `handoff-check-data` schema, a line per pointer, skill forms without
  `--session`.
- **PR 6:** removal proves ownership, not integrity; `setup list` and `setup repair` are
  `free` for the skill; `repair` drops `--profile`; pointer format v2.
- **PR 7:** directory flushes; an empty store folder is adopted.
- **PR 8:** `.tar.gz` for Windows; a twelve-digit commit; two extra lint rules.
- **PR 9:** exits 126 and 127; the signal rules; no SBOM in the platform packages; twelve
  more Release jobs per archive or npm change; Yarn through a project install.
- **PR 10:** path A or B for the first publish; `attest` runs without an approval; npm's
  staged publishing is not wired; release immutability; the release notes' wording (see the
  note below); the Release workflow as a required check; `SHA256SUMS` lists archives only.
- **Found while writing this record:** the generated release notes say installing through
  npm avoids SmartScreen and Gatekeeper warnings. That holds for the download mark; it may
  not hold for Smart App Control ([L-098](known-limits.md#l-098)).

## First publish

**Pending.** It is not written yet because it has not happened. The ledger follow-up (PR
12) completes this section after the maintainer publishes the 0.x pre-release, and does
these in the same change: records the evidence below, marks ADR 0023 Accepted, sets P13
`complete` in the ledger with this pull request's merge commit, closes L-036 and L-096 (or
rewrites them to what remains) and reconciles the register at the packet's close.

To be recorded, from `release.md` section 6.4:

- the version, the tag and its commit, the dispatch and the approved run's link, and the
  dry run's plan summary;
- `npm view` of the four packages (versions, `dist-tags`: `latest` still `0.0.0`, `next`
  the new version);
- `npm audit signatures` and the provenance shown on npmjs.com;
- `gh attestation verify` for each of the ten release assets and the four tarballs;
- the GitHub release: a pre-release, not marked latest, with ten assets;
- an install of `vsift-cli@next` with each package manager from the real registry, and what
  differed from the loopback matrix (Yarn's one-day gate, name and scope rules, size);
- any failure and the re-run that finished it ([L-097](known-limits.md#l-097)), and whether
  trusted publishing worked first time or path B's token was used.
