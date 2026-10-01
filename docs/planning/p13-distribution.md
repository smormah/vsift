# P13 distribution and managed-installation qualification record

Status: completed 2026-10-01 by the PR 12 change, on `main` at `011bc4d`, the commit the
0.1.0 pre-release was built from and tagged. **P13 is complete.** The 0.1.0 pre-release was
published to npm under the dist-tag `next` and to GitHub Releases on 2026-10-01; the section
"First publish" records how, including a first attempt that failed. Design:
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(Accepted 2026-10-01). Verification IDs: D-02..D-08, R-SEC01, R-SEC02 and SEC-T02 over
human output, serving R-03, R-13 and R-14 ([verification](verification.md)). Guides:
[`install.md`](../operations/install.md) (users) and
[`release.md`](../operations/release.md) (the maintainer's runbook). Attached stage of the
[end-to-end spine](e2e-test-spine.md): "Installed-user and managed-dependency run". Dates
are UTC unless a sentence says otherwise.

## Result in plain English

Everything P13 set out to build is merged, tested and published as a pre-release. `vsift`
prints readable text by default, says what to fix when a command line does not parse, and
checks an agent's draft report (`handoff check`). On Ubuntu 24.04 it can download, verify,
smoke-test and activate FFmpeg, whisper.cpp and the speech model (`setup install`), and
list, roll back, remove and diagnose them; that store survives kills at every step and, on
ext4, power loss. A release workflow builds byte-identical archives for Windows, macOS and
Linux, assembles the npm packages from them, and installs and runs them with npm, pnpm, Yarn
and Bun on all three systems from a local registry.

**What was published.** On 2026-10-01 the maintainer ran the Release workflow on the tag
`v0.1.0` (commit `011bc4d`, the release-prep change) and approved the protected `release`
environment. The workflow published `vsift-cli@0.1.0`, `@vsift/darwin-arm64@0.1.0`,
`@vsift/win32-x64@0.1.0` and `@vsift/linux-x64@0.1.0` to npm under `next`, with npm
provenance and Sigstore attestations, and created the GitHub pre-release `v0.1.0` with ten
assets. `latest` is still the empty `0.0.0` placeholder on all four packages, so a plain
`npm install vsift-cli` installs a package with no command, by design. **What that means:** a
pre-release exists for the next packet's qualification and for anyone who asks for `@next`.
**What it does not mean:** it is not a supported or stable release (that waits for P14),
the project has announced nothing, and no platform is "supported" yet
([L-035](known-limits.md#l-035)).

**The first attempt failed, and the record keeps it.** The first real run (created 20:37
UTC, approved about 21:34) stopped at its first `npm publish` with `ENEEDAUTH`, and nothing
was published. The
maintainer then reported that the connection between each npm package and the GitHub
workflow had never been set up, although an earlier report had said the trusted publishers
were done. A second run, after the connections were made, published everything. The account,
and what is and is not known about the cause, is under "First publish".

**What is weaker than it sounds**, stated once here and again where each claim is made:

- The npm matrix proves the packages install and run from a **local** registry on
  GitHub-hosted runners, which ship a Rust toolchain on `PATH` that no tested step invokes.
  The one install from the real registry (npm on the maintainer's Windows 11 development
  machine) is a single observation, not a clean-machine run; the clean-machine install with
  each package manager is the next packet's.
- The power-loss claim is for **Ubuntu 24.04 with ext4** only, from one hosted run on small
  stand-in versions, and its first run failed on a verifier defect that was then fixed.
- The offline install (`--artifact-dir`) has run only with stand-in artifacts, never with
  the real reviewed ones.
- The Windows and macOS prompts for an unsigned download are described from the operating
  systems' documentation; none has been observed on a VSift archive. One `npx vsift
  --version` ran on Windows 11 with no block or prompt, but that machine's Smart App Control
  state was not checked, so it does not settle [L-098](known-limits.md#l-098).
- The trusted-publisher settings are proved only by the second run working, not by reading
  them: npm shows them only to the package owner. The `release` environment and the tag
  ruleset were read back with `gh api` (below); "disallow bypass 2FA tokens" is the
  maintainer's report and cannot be read from outside.

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
6. **The 0.1.0 pre-release and its record** (release prep #247, the maintainer's publish of
   2026-10-01, PR 12): the tag `v0.1.0`, the dry run, the published packages and release,
   and this record's "First publish" section, the ledger and ADR 0023's acceptance.

Not in P13 (unchanged): the release qualification of the next packet, managed installation
on Windows and macOS, native installers, crates.io, the MCP adapter, SEC-T01's adversarial
evidence (L-068) and the P12 debt that is not the compact re-run.

## Pull requests

All are squash merges to protected `main`, listed in the order they merged. The merge
commits are from `gh pr view` on 2026-10-01.

| PR | Pull request | Merge commit | Merged |
| --- | --- | --- | --- |
| 0 | #226 ADR 0023 (then Proposed), the plan, ledger `in_progress` | `dbc60f7` | 2026-09-30 |
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
| 11 | #245 user guide, this record, closing sweep | `59f77a0` | 2026-10-01 |
| release prep | #247 release-notes wording, `0.1.0` changelog, setup state | `011bc4d`, the commit tagged `v0.1.0` and published | 2026-10-01 |
| 12 | the completion record and ledger follow-up (governance rule 9): the "First publish" section, the ledger, ADR 0023 Accepted, known limits and the memory files | this change; the ledger names `011bc4d` (see "First publish") | when this change merges |

## Decisions A to H and how each was met

| Decision | Outcome | Evidence and amendments |
| --- | --- | --- |
| A. Names | Launcher `vsift-cli` (command `vsift`) over `@vsift/win32-x64`, `@vsift/darwin-arm64`, `@vsift/linux-x64` | Two amendments on 2026-09-30. The scope: npm's "not available" for the organisation `vsift` was a repeated submission after the first had created it; `@shongo` was chosen briefly (#235, merged 14:32 UTC) and withdrawn about an hour later (#237). The launcher: npm refused the unscoped `vsift` as too similar to `sift` and `tsify` when the maintainer published a placeholder, so `vsift-cli` holds the name. [ADR 0009](../decisions/0009-package-identity-and-distribution.md) notes; the lesson there: a not-found lookup is not availability, only a publish proves a name. A third amendment on 2026-10-01: the three platform packages were each first held by a `0.0.0` placeholder the maintainer published with two-factor authentication (path A), so that their trusted publishers could be configured |
| B. What is published, and when | Nothing during the pull requests; one 0.x pre-release under `next` at completion; no crates.io | **Met.** Nothing was published during the pull requests except the maintainer's four `0.0.0` placeholders (`vsift-cli` on 2026-09-30; the three `@vsift/...` packages on 2026-10-01, path A). The one pre-release, 0.1.0, was published on 2026-10-01 by the second approved run, under `next`, with `latest` untouched; no crate was published. See "First publish" |
| C. Trust signals | Sigstore attestation and npm provenance only; no Authenticode, no notarization | **Met, and verified for 0.1.0**: every release file and every tarball has a Sigstore attestation that `gh attestation verify` accepted, and `npm audit signatures` verified the registry signatures and the provenance of all four packages (below). Still unsigned, as decided; what that means for users is in `install.md` section 4 ([L-098](known-limits.md#l-098)) |
| D. Platforms | Windows 11 x64 (MSVC, static C runtime), macOS 15 arm64, Linux x64 glibc built on Ubuntu 22.04 | The Release workflow builds each target twice and requires identical executables: passed on `main` at `951226f` ([run 36786019996](https://github.com/smormah/vsift/actions/runs/36786019996)) and `57f03fe` (run 36797351652). Windows needs `-C link-arg=/Brepro` (ADR 0023 PR 8 note) |
| E. Managed installation on Ubuntu 24.04 x64 only | Met | The two hosted managed-smoke runs below; Windows and macOS keep typed manual guidance (`unavailable_target`, engine tests and the P06 stage) |
| F. Order of work | P12 debt first, the compact tier re-run once after `handoff check` | #227 first; the re-run (#222) ran on `a0bfb06` and met the target: Sonnet 5.5 26 of 28, GPT-6-Sol 28 of 28 after the maintainer's `rg --files` reading (23 of 28 as run); L-085 closed ([P12 record](p12-agent-qualification.md)) |
| G. How a draft reaches `handoff check` | Standard input or `--file`; two literal skill forms | PR 5; the guard and the grader recognise exactly those forms ([ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md) notes) |
| H1. The validator answers | Met (PR 5) | `handoff_cli_contract`, `handoff_differential` |
| H2. Human output by default | Met (PR 2a, 2b) | Every command that completes has a renderer; one without is `INTERNAL` |
| H3. `DOWNLOAD_FAILED` | Met (PR 4) | Six reasons, exit 7; D-07 row |
| H4. In-place v1 edits before the first publication | Used in PRs 4, 5 and 6; closed by the publication of 0.1.0 on 2026-10-01 | [ADR 0008](../decisions/0008-cli-and-json-contract.md) notes; from 0.1.0 on, v1 changes are additive only |
| H5. Launcher checks the digest if under 50 ms | Met (PR 9) | Median 4.6 to 10.8 ms on the hosted runners (ADR 0023 PR 9 note, Release run 36772356382); the matrix fails above 50 ms |
| H6. Node.js 22, Bun 1.2 | Met | The matrix runs the minimums, 22.23.3 and 1.2.23 ([L-092](known-limits.md#l-092)) |
| H7. Skill inside the npm package and every archive | Met (PRs 8, 9) | Byte-identical, read back by `vsift-release verify` and `npm-verify` |
| H8. R-13 joins P13 | Met (PR 0) | Ledger and traceability |
| H9. Power-loss qualification of the managed store | Met, and strengthened on review to "a command that reported success survives a power loss" | [Run 36829198545](https://github.com/smormah/vsift/actions/runs/36829198545), below |
| H10. No named-agent re-run from a clean install | Kept | The spine's distribution checkpoint, agent part, is the next packet's. No agent has used the published package; the one install from the real registry (below) was a command-line check |

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
| Publish job tools | Node.js 24.21.0 with npm 11.19.0 (both approved runs set Node.js 24.21.0; the first run's log prints the npm version) |
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
| [36919612380](https://github.com/smormah/vsift/actions/runs/36919612380) | `Release`, dispatch on the tag `v0.1.0` (`011bc4d`) with `dry_run` set, 2026-10-01 20:10 | success (21 jobs) | The dry run: three builds, notices and SBOMs, packaging, the npm packages, all twelve `npm qualification` jobs and the plan; `attest` and `publish` skipped. The plan: version 0.1.0, dist-tag `next`, four `npm publish` commands in order (darwin-arm64, win32-x64, linux-x64, then `vsift-cli`) with `--provenance --ignore-scripts`, and a draft pre-release not marked latest with ten assets |
| [36922901956](https://github.com/smormah/vsift/actions/runs/36922901956) | `Release`, dispatch on the tag with `dry_run` cleared, created 2026-10-01 20:37 | **failure** | The first real dispatch. `attest` succeeded (20:48 to 20:49); `publish` waited for the maintainer's approval in the `release` environment, began at 21:34 and failed within seconds on its first command, `npm publish` of `@vsift/darwin-arm64`, with `ENEEDAUTH` (npm 11.19.0, Node.js 24.21.0). **Nothing was published and no GitHub release was created.** See "First publish" |
| [36931487439](https://github.com/smormah/vsift/actions/runs/36931487439) | `Release`, a new dispatch on the same tag with `dry_run` cleared, created 2026-10-01 21:52 | success (21 jobs, none skipped) | Everything was rebuilt; `attest` ran 22:01:01 to 22:01:11 and `publish` 22:01:51 to 22:04:03: four `npm publish` calls with provenance by trusted publishing (no token exists), the check that `next` is 0.1.0 and `latest` untouched, then the GitHub pre-release |

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
| **R-SEC01** fork pull requests cannot reach publish secrets | The governance lint, rules 1-7 (a test of the real `release.yml` and a mutation of it per rule, 28 for rule 7); `tools/vsift-release/src/publish.rs` tests the mode decision for every event, ref, repository and input; the settings read back with `gh api` on 2026-10-01 (environment `release`: one reviewer, administrator bypass off, deployment rule type tag `v*` only; ruleset `release tags` active on tags; fork approval `all_external_contributors`); the two dispatches on the tag | A pull request, a push or a fork cannot run `attest` or `publish`, which alone can write or request an OIDC token; actions are pinned; permissions are minimal. At run time, `publish` waited for the maintainer's approval in the `release` environment (the first run) and then published with no token anywhere: the second run succeeded by trusted publishing alone | That a fork's pull request was ever tried against the live settings (the lint and the mode tests cover it; nobody attacked it). npm's side cannot be read from outside: the trusted publishers are shown only to the package owner, and "disallow bypass 2FA tokens" is the maintainer's report |
| **R-SEC02** artifacts match the protected commit; signatures and provenance verified | `vsift-release npm` and `npm-verify`; the plan job's byte-for-byte checks on every run; the launcher's refusals (`npm/test/launcher.test.cjs`, every matrix job); for 0.1.0, `npm audit signatures` and `gh attestation verify` (below) | Tarballs are made only from canonical archives and equal their fresh assembly; the bytes qualified are the bytes the publish job published, by SHA-256; a changed, replaced or mismatched package is refused with 126. For 0.1.0: all four packages have npm provenance and verified registry signatures; all ten release files and all four tarballs verify against `smormah/vsift`'s `release.yml` at `refs/tags/v0.1.0` on GitHub-hosted runners only; the builds are reproducible across separate runs | That anyone but this one verification (one machine, one session) has checked; that the attestation proves the source is good (it proves where and how the files were built). The launcher check is not a defence against someone who can write to the install ([L-093](known-limits.md#l-093)) |
| **SEC-T02** over human output | `sec_t02_human_output.rs`, the `TerminalText` property tests, the renderers' unit tests, `evidence_cli_contract`, `job_run_cli_contract`, `job_batch_cli_contract`, golden snapshots | Every human output goes through one builder: no raw control, escape, link or hidden character; a hostile delivered path is inert, alone on its line and flagged; L-073 is closed | How a particular terminal draws the bytes (the tests inspect bytes, not a terminal emulator); progress is not rendered in human mode for the worker hosts ([L-017](known-limits.md#l-017)) |
| **R-13** agent skill and handoff | `handoff check` (`handoff_cli_contract`, `handoff_differential`, the skill guard), the compact re-run #222 | The grader and the command run one check; the compact tier meets 90% (93% and 100%, with the maintainer's reading for Sol) | A named-agent run from a clean install (the next packet); the review tier's A-09 blurred re-run ([L-095](known-limits.md#l-095), #224) |
| **R-14** install without Rust through npm, pnpm, Yarn and Bun | The twelve-job matrix (runs 36786019996, 36797351652 and, on the tag, 36919612380 and 36931487439); one install of `vsift-cli@next` from the real registry on Windows 11 with npm (below) | With scripts disabled, from tarballs identical to the ones the publish job published, the launcher and platform package install and run on Windows Server 2025, macOS 15 and Ubuntu 24.04 with each manager, global (Yarn: project) and one-shot; offline; with optional dependencies omitted (127, readable); uninstall is clean (Bun leaves a package, L-092). From the real registry: `npm install vsift-cli@next`, `npm audit signatures` and `npx vsift --version` worked on the maintainer's Windows 11 machine with no npm login, and printed `vsift 0.1.0 (011bc4da1af6)` | **A clean machine, or the real registry with pnpm, Yarn and Bun, or on macOS and Ubuntu.** The matrix's registry is a Verdaccio on loopback with no uplink; the runners have a Rust toolchain on `PATH` that no tested step invokes (the qualification driver does not remove it); Yarn's one-day gate is set to zero (so the gate's effect on the real `@next` was not observed); Node.js and Bun minimums only. The Windows 11 install was on a development machine, not a clean one. **No record shows a native archive extracted and run**: the release's archives were downloaded and their checksums and attestations verified, and each archive's executable is the byte-identical one the matrix ran from the platform package, but `install.md` section 3 has not been walked through to a running `vsift` |
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
| SEC-22 CI steals secrets or replaces a binary | Lint; two privileged jobs, dispatch-only on the tag; publish only qualified tarballs by digest; no long-lived token (none exists: the 0.1.0 publish used trusted publishing alone); the `release` environment with a reviewer and a tag rule, and the tag ruleset, read back with `gh api` | npm's trusted-publisher settings and "disallow bypass 2FA tokens" cannot be read from outside; the staged-publishing question before a stable release ([issue #246](https://github.com/smormah/vsift/issues/246)) |
| SEC-23 checksums from a compromised server | Trust anchor in reviewed source; launcher digest; Sigstore attestations on every release file and tarball, and npm provenance on all four packages, verified for 0.1.0 | The verification was one machine, one session; unsigned executables remain ([L-098](known-limits.md#l-098)) |

## Residual known limits

What matters at release; each entry in [known limits](known-limits.md) has the detail.

| ID | Limit | State |
| --- | --- | --- |
| L-036, L-096 | Nothing published; the attest and publish jobs had never run | **Closed** by the 0.1.0 publish on 2026-10-01 (evidence below); deleted from the register |
| L-097 | A part-way publish is public until a re-run completes it | Accepted. Not exercised: the first approved run stopped before any publish and the second published everything, so the skip-what-is-published path has never run |
| L-100 (new) | npm prints only `ENEEDAUTH`, with no reason, when a trusted publisher is missing or wrong | Accepted; mitigated by the runbook's preflight (`release.md` 6.2) |
| L-037 | Managed install on Ubuntu 24.04 x64 only; power-loss claim for ext4 only | Accepted by decision E |
| L-098 (new) | Unsigned executables meet SmartScreen and Gatekeeper; Smart App Control may block `vsift.exe` | To be tried on real machines before a stable release (one `npx vsift --version` on Windows 11 ran unblocked; Smart App Control's state there was not checked) |
| L-099 (new) | Managed install depends on the publishers' files and redirect hosts | Fails safe; the manual path is the fallback |
| L-087, L-088, L-090 | Rehash cost, `407` recognised by text, unprovable content left for the user | Monitoring or accepted |
| L-089 | SBOM `bom-ref` values name the runner's checkout path | Accepted |
| L-091 | Signal relay can repeat or miss one; a killed launcher cannot stop vsift | Accepted |
| L-092, L-093, L-094 | Matrix coverage (minimum runtimes, Yarn project install, Bun leftovers), digest check reach, Windows 260-character paths | Accepted |
| L-095 | Review-tier A-09 blurred re-run (#224) | Maintainer, before the next packet |
| L-035, L-042 | Platform evidence mostly Windows 11; real-tool paths run on demand | The next packet |
| L-016, L-017, L-086 | Path display residual, no progress in human worker output, `vsift-contract` cannot be packaged for crates.io | Accepted or deferred (no crate is published) |

## Maintainer-only steps

No setting was changed by an agent or a workflow. States as read on 2026-10-01 (`gh api`,
anonymous registry reads and the runs' logs); the runbook is
[`release.md`](../operations/release.md) section 6.

| Step | State |
| --- | --- |
| The maintainer's npm account (two-factor authentication, per ADR 0009's placeholder note); the organisation `vsift` (scope `@vsift`) | Done 2026-09-30 |
| `vsift-cli@0.0.0` placeholder (README and `package.json` only) | Done 2026-09-30, 21:59 UTC; `latest` |
| Fork pull requests: approval for all external contributors | Done 2026-10-01; read back with `gh api`: `all_external_contributors`, default token read-only, Actions cannot approve pull requests |
| Environment `release` (reviewer, no admin bypass, tag rule `v*`) | Done 2026-10-01; read back with `gh api`: one reviewer, self-review allowed, administrator bypass off (corrected after the first read showed it on), deployment rule type tag `v*` only, no secrets |
| Tag ruleset `v*` | Done 2026-10-01; read back with `gh api`: ruleset `release tags`, active, refs `refs/tags/v*`, rules creation, update, deletion and non-fast-forward, bypass only the Repository admin role |
| First publish of the three `@vsift/...` names (path A placeholders, or path B short-lived token) | Done 2026-10-01 (path A: placeholders `0.0.0` published by the maintainer with two-factor authentication: `@vsift/win32-x64` 17:44 UTC, `@vsift/linux-x64` 17:45, `@vsift/darwin-arm64` 17:46; each `latest`; confirmed by anonymous registry reads) |
| Trusted publishers on all four packages | **Reported done before the first attempt, and that report was wrong for it.** The maintainer's earlier report listed GitHub Actions, `smormah`, `vsift`, `release.yml`, environment `release`, label `release workflow`, allowed actions npm publish ticked and npm dist-tag unticked. After the first approved run failed with `ENEEDAUTH`, the maintainer reported that no connection between GitHub and npm had really been set up: the Trusted Publisher form's **Set up connection** step had not been completed on the packages. It was completed afterwards. Proved only by the second run's four publishes by trusted publishing (no token exists); npm shows these settings only to the package owner, so they cannot be read from outside ([issue #246](https://github.com/smormah/vsift/issues/246) tracks whether to move to staged publishing before a stable release) |
| Disallow tokens on the four packages | Done 2026-10-01 as reported by the maintainer (npm's option "Require two-factor authentication and disallow bypass 2fa tokens"); unverifiable from outside |
| `npm logout` on the machine used for the placeholders | Done 2026-10-01 as reported by the maintainer; the verification below ran with no npm login |
| Version, `CHANGELOG.md` release section, tag `v0.1.0` | Done 2026-10-01: the version is 0.1.0 in `Cargo.toml` and `npm/vsift-cli/package.json`; the `[0.1.0]` changelog section was cut in the release-prep change (#247, merged 19:28); the annotated tag `v0.1.0` points at `011bc4d` (read with `git ls-remote`), pushed before the 20:10 dry run |
| Dry run on the tag | Done 2026-10-01: run 36919612380, success; the plan matches what was published |
| Dispatch with `dry_run` cleared, and the approval | Done 2026-10-01 in two attempts: run 36922901956 failed (`ENEEDAUTH`, nothing published); run 36931487439 published |
| Verification of the published files | Done 2026-10-01 by a supervising agent session on the maintainer's Windows 11 machine (read-only; see "First publish") |

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
- **PR 10:** path A was chosen for the first publish (2026-10-01); `attest` runs without
  an approval (it did, in both approved runs); npm's staged publishing is not wired
  ([issue #246](https://github.com/smormah/vsift/issues/246), deferred on 2026-10-02 until after R1 or the public announcements);
  release immutability; the Release workflow as a required check; `SHA256SUMS` lists
  archives only. Settled in the release-prep change (#247): the release notes no longer say
  that installing through npm avoids every warning; they say that npm-installed files carry
  no download mark and that Windows Smart App Control can still block an unsigned program
  ([L-098](known-limits.md#l-098)).

## First publish

Written 2026-10-01 by PR 12. Every statement was read on that day from the workflow runs
and their logs (`gh run view`, `gh api`), the GitHub release (`gh release view`), the public
npm registry (anonymous `npm view`) or taken from the verification described below; a
statement that is only the maintainer's report says so. Anyone can repeat each check with
the commands in [`release.md`](../operations/release.md) section 6.4.

### What was released

| Item | Value |
| --- | --- |
| Version | 0.1.0, a 0.x pre-release (dist-tag `next`; `latest` is still `0.0.0` on all four packages) |
| Tag | `v0.1.0`, an annotated tag on `011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa`, PR #247's merge commit and `main`'s tip when the tag was made |
| Dry run | [36919612380](https://github.com/smormah/vsift/actions/runs/36919612380), success, 21 jobs; `attest` and `publish` skipped. Its plan: version 0.1.0, dist-tag `next`, four `npm publish` commands in order with `--provenance --ignore-scripts`, a draft pre-release not marked latest with ten assets |
| First real dispatch | [36922901956](https://github.com/smormah/vsift/actions/runs/36922901956): **failed**, nothing published |
| Second real dispatch | [36931487439](https://github.com/smormah/vsift/actions/runs/36931487439): success, a new dispatch on the same tag; everything rebuilt |

### The first attempt failed

The first run was created at 20:37. Its `attest` job succeeded (20:48 to 20:49). Its `publish`
job waited for the maintainer's approval in the `release` environment, began at 21:34 and
failed within seconds, on its first command, `npm publish` of `@vsift/darwin-arm64`. npm
(11.19.0 on Node.js 24.21.0) printed the package contents and then `npm error code
ENEEDAUTH`, "This command requires you to be logged in". **Nothing was published and no
GitHub release was created**: the registry's own record shows no 0.1.0 of any package
before the second run's publishes.

What is known, and what is not:

- npm's trusted-publishing exchange evidently failed. When it does, npm falls back to
  asking for a login, and at its default log level it prints no reason, so the log does not
  say which setting was wrong.
- The maintainer then reported that "we never really set up any sort of connections between
  GitHub and npm": the Trusted Publisher form's **Set up connection** step had not been
  completed on the packages. An earlier report that the trusted publishers were done had
  been wrong, and this record's earlier "done as reported by the maintainer" rows were wrong
  for the first attempt.
- After the connections were made, the second attempt succeeded.
- Not known: which field or step exactly caused the first failure. npm shows a package's
  trusted-publisher settings only to its owner, nobody outside can read them, and the first
  run's log is at npm's default level, without the verbose output that might name the
  reason. The cause is therefore the maintainer's report plus the fact that the second run
  worked, not something the logs show.

The first run's `attest` job had already attested that run's files. Because the builds are
reproducible, the second run's files are byte-identical, so each of the ten release assets
now has two attestations, one from each run, and both name the same bytes. The failure was
an unfinished setup step that no dry run can detect and the runbook did not make
checkable, not a fault found in the workflow; it is [L-100](known-limits.md#l-100), with a
preflight added to `release.md` 6.2.

### The second run

The `attest` job ran 22:01:01 to 22:01:11 and `publish` 22:01:51 to 22:04:03, each by npm
with provenance through trusted publishing (no token exists). The registry's publish times
and the Sigstore log entries npm printed:

| Package | Published (registry time) | Sigstore log index |
| --- | --- | --- |
| `@vsift/darwin-arm64@0.1.0` | 22:03:01 | 3042221095 |
| `@vsift/win32-x64@0.1.0` | 22:03:09 | 3042222924 |
| `@vsift/linux-x64@0.1.0` | 22:03:16 | 3042224909 |
| `vsift-cli@0.1.0` | 22:03:41 | 3042226758 |

npm printed each `+ package@version` line about a minute before the registry showed the
version ("being processed and may take a few minutes to become available"), so the registry
times above are later than the job's own log lines. This is the first real use of the
publish job's wait for the new `next` tag: it polled three times, 30 seconds apart, and
read `next` 0.1.0 and `latest` 0.0.0 on the fourth read (22:03:58). The GitHub release
`v0.1.0` was published at 22:04:01 as a pre-release, not a draft and not marked latest (the
"latest release" API answers 404), with ten assets: three `.tar.gz` archives, `SHA256SUMS`,
three `.cdx.json` SBOMs and three `THIRD-PARTY-NOTICES.txt` files. The first run's logged
digest for `@vsift/darwin-arm64` (SHA-1 `43210151b6918a18d667a2736162e2cb3c77cbf0`) equals
the registry's `dist.shasum` for the version the second run published.

### Verification

Run on 2026-10-01 by a supervising agent session on the maintainer's Windows 11 machine, with
no npm login, from empty temporary folders (since deleted). It is one machine and one session,
and the machine is a development machine, not a clean one.

| Check | Result |
| --- | --- |
| `npm view` of the four packages | Each has `latest` `0.0.0` and `next` `0.1.0` (re-read in PR 12); `vsift-cli@0.1.0` has the `vsift` command; all four versions carry an npm provenance statement (a SLSA provenance v1 predicate in `dist.attestations`) |
| `npm install vsift-cli@next`, then `npm audit signatures` | "4 packages have verified registry signatures" and "4 packages have verified attestations" |
| `npx vsift --version` | `vsift 0.1.0 (011bc4da1af6)`; it ran with no block or prompt. The machine's Smart App Control state was not checked, so this is one observation and does not settle [L-098](known-limits.md#l-098) |
| `gh release download v0.1.0`, then `sha256sum --check --strict SHA256SUMS` | All three archives OK |
| `gh attestation verify` with `--repo smormah/vsift --signer-workflow smormah/vsift/.github/workflows/release.yml --source-ref refs/tags/v0.1.0 --deny-self-hosted-runners` | 10 of 10 release files and 4 of 4 npm tarballs (fetched with `npm pack`) verified |
| Reproducibility across runs | The first 16 hex digits of each tarball's SHA-256 equal the dry-run plan's: `vsift-cli` `0492534f5300ed04`, `@vsift/darwin-arm64` `70377a8e329a0e10`, `@vsift/linux-x64` `1698b2e4342b07d1`, `@vsift/win32-x64` `63365812eaf75402` |
| GitHub release | A pre-release, not a draft, not marked latest, ten assets (read again in PR 12); GitHub lists two attestations for each of the ten assets |

### What this proves and does not prove

**Proves:**

- The workflow's real publishing path works: the protected environment's approval,
  trusted publishing with no stored token, npm provenance, Sigstore attestation and the
  GitHub pre-release, in the order the plan lists.
- What was published is what was qualified: the tarballs' digests equal the dry run's, the
  attestations tie every file to this repository's `release.yml` on the tag, and the builds
  are reproducible across separate runs.
- `latest` was left alone, and `vsift-cli@next` installs and runs on one Windows 11 machine
  from the real registry with no npm login.

**Does not prove:**

- A clean-machine install, or an install with pnpm, Yarn or Bun from the real registry, or
  on macOS or Ubuntu. Yarn's one-day gate on a real `@next` was not observed. What differed
  from the loopback matrix, in the one install done: nothing noticed, and nothing else was
  measured.
- That the native archive extracts and runs by `install.md` section 3; only its checksum
  and attestation were verified.
- Anything about Windows Smart App Control or the macOS prompts ([L-098](known-limits.md#l-098)).
- Why exactly the first attempt failed, or that every trusted-publisher field is right
  (only that they sufficed). "Disallow bypass 2FA tokens" is the maintainer's report.
- That a failed `publish` can be completed by **Re-run failed jobs** or by skipping the
  versions npm already holds ([L-097](known-limits.md#l-097)): no run has published part-way.
  Two checks suggest a re-run of the first run would have published the same files: the one
  tarball it logged is byte-identical to the one the second run published, and the second
  run's four tarballs match the dry run's plan by their first 16 hex digits.
- Any user's experience of this release: it was verified by one agent session, and no
  coding agent has used the published package (the next packet's checkpoint).

### What stays open

- [Issue #246](https://github.com/smormah/vsift/issues/246): whether to move npm publishing
  to staged publishing (a second approval on npmjs.com); deferred by the maintainer on
  2026-10-02 until after R1 or the public announcements.
- [L-100](known-limits.md#l-100): the missing reason when a trusted publisher is wrong; a
  preflight is in the runbook, and a workflow change (verbose npm logging or a hint) is
  possible later and is not made here.
- P14, the release qualification (a stable release, the clean-machine install with each
  package manager, the named-agent run from a clean install and the Smart App Control
  try-out), has not started and does not start automatically (governance rule 10).

### How the ledger records it

The ledger sets P13 `complete` with the merge commit `011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa`:
the last implementation change and the commit the release was built from and attested. It is
not this change's own merge commit, which a documentation, ledger and memory change cannot
know in advance; it is a deliberate choice for the maintainer's review. L-036 and L-096 are
closed by this publish and deleted from the known-limits register, as that file's closing rule
says.
