# Building, checking and publishing releases

Status: maintainer runbook for the native release archives, the npm packages and their
publication, 2026-10-02 (P13 PRs 8, 9 and 10,
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
sections 1 and 2, decisions B, C, D and H5; P14 PR 8,
[ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md) decisions A and B,
the stable release and the release candidate). **0.1.0 was published with it on 2026-10-01**
(section 6; the record is [`p13-distribution.md`](../planning/p13-distribution.md), "First
publish"). **The stable path (section 6.7) ran against the real services for the first time
on 2026-10-09: the stable release `0.2.0` is published under `latest` on all four packages
(publish run 37946261087), and `next` still names the third candidate, `0.2.0-rc.3`. What
happened, and what it showed, is section 6.13. It ran a second time on 2026-10-15 for the stable release `0.2.1`
(publish run 38758081349: `latest` moved from `0.2.0` to `0.2.1` on all four packages and `next` stays `0.2.1-rc.3`); the steps
that were followed and what the services showed, among them a read-back that timed out after the packages were published, are section 6.17. It ran a third time on 2026-10-18 for the stable release `0.2.2`, the first published from a release branch
(publish run 39127747535, first attempt: `latest` moved from `0.2.1` to `0.2.2` on all four packages and `next` stays `0.2.2-rc.1`); the steps and what the services showed are section 6.20.** The exact steps for the first release
candidate, `0.2.0-rc.1`, are section 6.10 (published on 2026-10-05); those for the second,
`0.2.0-rc.2`, are section 6.11 (published on 2026-10-07); those for the third, `0.2.0-rc.3`,
are section 6.12 (published on 2026-10-08); the checklist of the stable release `0.2.0` was
[`p14-stable-release-steps.md`](../planning/p14-stable-release-steps.md). The exact steps for the first
candidate of the patch release `0.2.1`, `0.2.1-rc.1`, are section 6.14 (published on 2026-10-10 and superseded): the first
candidate published while a stable version is `latest`, so every expectation about `latest` differs from 6.10 to 6.12.
The exact steps for the second candidate, `0.2.1-rc.2`, are section 6.15 (published on 2026-10-11 and superseded once its successor is): the first candidate
published while another candidate of the same release is on `next`, which it supersedes.
The exact steps for the third candidate, `0.2.1-rc.3`, are section 6.16 (published on 2026-10-13 and accepted as the candidate of the stable release: 6.15's steps with the numbers that
change, written compactly, because `0.2.1-rc.3` is `0.2.1-rc.2` plus one fix).
Sections 6.10 to 6.16 are the record of those publishes, not instructions for a version that is
already published. **For the next stable version, 6.20 (with 6.7 and 6.18) is the model; for the next candidate, 6.19 is. The first candidate of `0.2.2`, `0.2.2-rc.1`, is section 6.19 (cut 2026-10-17 and published on 2026-10-18 from `release/0.2.x`, the first candidate made on a release branch; a record since 2026-10-18), and the stable release `0.2.2` is section 6.20 (published 2026-10-18 from the same branch; a record).** **From 2026-10-16 releases are timed by the weekly window of 6.18** (the cut at Friday or Saturday midnight in the maintainer's local time, from a commit whose nightly passed, the long hosted runs after it, the publish on Monday or Tuesday; hot fixes outside it). **Since 2026-10-17 release candidates and the stable release are made on a release branch, `release/<MAJOR>.<MINOR>.x`, and `main` keeps taking merges during the window** (6.8 and 6.18); the workflow's first job, `guard`, refuses a dispatch on the wrong ref and a candidate whose commit has no passed nightly in seconds, before anything is built (section 1, 6.18). The workflow described here builds, checks and
packages the archives, assembles the npm packages and runs their qualification (section 5),
and on every run writes the publish plan and shows it (a dry run). It publishes only when the maintainer
dispatches it on a release tag with `dry_run` cleared and then approves the protected
`release` environment (section 6). The version decides what a publication does: a version
with a pre-release suffix (`0.2.0-rc.1`) goes under `next`; a version without one (`0.2.0`)
is a stable version, goes under `latest` and moves it on all four packages. The
maintainer's one-time setup, the candidate procedure and the stable release procedure are
section 6. The installation guide
for users is [`install.md`](install.md).

## 1. What the release workflow does

`.github/workflows/release.yml` (**Release**) has three archive jobs, the two npm jobs
of section 5 and the three publishing jobs of section 6:

| Job | Runner | What it does |
| --- | --- | --- |
| `guard` | `ubuntu-24.04` | Runs first, and every other job needs it. For a dispatch it reads the workspace version from `Cargo.toml` and refuses, in seconds, a ref the mode may not use, a release tag that is not on its release branch, and a pre-release (a candidate) whose commit has no passed Nightly run (6.18 has the rules and why). A pull request or push passes it at once. `actions: read` and `contents: read`; no secret, no build. |
| `inventory` | `ubuntu-24.04` | Installs the pinned cargo-about 0.9.2 and cargo-cyclonedx 0.5.9, fetches the locked sources and writes, for each target, `THIRD-PARTY-NOTICES` and a CycloneDX 1.5 SBOM of the `vsift-cli` crate's dependency graph for that target. cargo-about runs `--offline`, so licence texts come from the locked crate sources only. |
| `build` | `windows-2025`, `macos-15`, `ubuntu-22.04` | Builds `vsift` for its target twice on the same runner, with every release output removed in between, and requires the two executables to be byte-identical; requires `vsift --version` to print `vsift <version> (<first 12 digits of the commit>)`; records the Linux and macOS runtime dependencies and refuses a Windows executable that imports the dynamic C runtime. |
| `package` | `ubuntu-24.04` | Packages each target with `tools/vsift-release`, twice, and requires the two archives to be identical; reads each archive back and checks it against its inputs; writes `SHA256SUMS` and checks it with `sha256sum --check --strict`; keeps the archives and `SHA256SUMS` as the run's `release-dry-run` artifact for 7 days. |

The targets are the three R0 targets of ADR 0023 decision D:

| Target | Built on | Toolchain notes |
| --- | --- | --- |
| `x86_64-pc-windows-msvc` | Windows Server 2025 | MSVC with a static C runtime (`-C target-feature=+crt-static`); `-C link-arg=/Brepro`, without which the linker writes a build time and a random PDB identity into every executable |
| `aarch64-apple-darwin` | macOS 15, Apple silicon | |
| `x86_64-unknown-linux-gnu` | Ubuntu 22.04 | glibc 2.35 or later and OpenSSL 3 (`libssl.so.3`) at run time, because the transport uses the system TLS library (ADR 0023, Consequences) |

The build command is always the same, and the governance check refuses any other:

```console
cargo build --release --locked -p vsift-cli --bin vsift --target <target>
```

No feature is selected, so no development-only feature (`fault-injection`,
`durability-campaign`, `install-test-hooks`) can reach an archive, and only the `vsift`
executable is built: test and qualification binaries such as the smoke-test stand-in
belong to other packages and are never compiled here.

**When it runs.** On pull requests to `main` or a release branch (`release/**`) and pushes to them that change anything
an archive or npm package is made from (`crates/`, `skills/vsift/`, the licences,
`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `tools/vsift-release/`, `npm/`,
`tools/send-console-ctrl.ps1`, which the Windows qualification uses, or the workflow
itself), and on manual dispatch (with the input `dry_run`, set by default). Never on a
tag push or a GitHub release event: a release is a manual dispatch on its tag.

**Why a run does not publish unless the maintainer means it to.** Every job but
`attest` and `publish` has `contents: read` only (`permissions: {}` at the top), checks
out without keeping credentials and uses no secret. `attest` and `publish` run only when
all of these hold, checked twice (the jobs' conditions and the plan job's decision):
the run is a manual dispatch, `dry_run` is cleared, the repository is `smormah/vsift`
(not a fork), and the ref is the tag `v<version>` of the workspace version. A pull
request, a push or a fork therefore never publishes, and a dispatch with `dry_run`
cleared anywhere else is refused by the `guard` job in seconds, and by the plan job again, rather than quietly doing a dry run. `publish`
then waits for the maintainer's approval of the `release` environment. Everything a dry
run produces is the run's own artifacts, which expire after 7 days. The governance
workflow lint (section 3) holds the workflow to all of this.

**Which channel a version gets.** The workspace version alone decides, by the plan job
(`vsift-release publish-plan`, `tools/vsift-release/src/publish.rs`); nothing the dispatching
person types can change it.

| Version | Kind | npm | GitHub release | Release notes say |
| --- | --- | --- | --- | --- |
| `0.2.0-rc.1`, `0.2.0-rc.12` (`rc.` and a positive number) | release candidate | `--tag next`; `latest` untouched | pre-release, not marked latest | "a release candidate under qualification", not announced, no claim of support |
| `0.3.0-beta.1`, `1.0.0-rc.0` (any other suffix) | pre-release | `--tag next`; `latest` untouched | pre-release, not marked latest | "a pre-release" |
| `0.2.0`, `1.0.0`, `0.1.0` (no suffix; a 0.x version too) | **stable version** | `--tag latest`; **`latest` moves on all four packages**; `next` untouched | the release marked latest | what the stable release promises (the CLI grammar, exit codes and v1 JSON, additively) and nothing more |
| `0.0.0` | the placeholder every package already holds | refused in every mode | refused | refused |

Anything npm would rewrite or read as another version is refused: build metadata (`+...`),
a leading `v`, leading zeros, an empty identifier, whitespace. A pre-release is *never*
published under `latest`, and a stable version is *never* published under `next`. `0.1.0`
was published before this rule, as a pre-release under `next`; by shape it is a stable
version, so a plan for it is refused by the registry guards (npm already holds it under another tag).
While the workspace version is `0.1.0`, every pull request's dry run therefore shows the
stable plan as "would be refused" (no candidate, `0.1.0` already on npm): that is the plan
working, not a failure, and it ends when the version becomes a candidate.

## 2. The archives

Each archive is `vsift-<version>-<target>.tar.gz`, a gzip-compressed tar file on every
target (Windows 11 opens `.tar.gz` in File Explorer, and `tar -xf` extracts it). It
holds one directory of the same name:

```text
vsift-0.1.0-x86_64-unknown-linux-gnu/
  LICENSE
  LICENSE-APACHE
  LICENSE-MIT
  THIRD-PARTY-NOTICES
  skills/vsift/...          the agent skill, byte-identical to the repository's
  vsift                     vsift.exe on Windows
  vsift.cdx.json            CycloneDX 1.5 SBOM for this target
```

The packager (`tools/vsift-release`, never shipped) refuses an executable that is not
named `vsift`/`vsift.exe` or is not of the target's format and architecture (PE32+
x86-64, 64-bit Mach-O arm64, 64-bit ELF x86-64); notices that do not come from the
reviewed template; an SBOM that is not CycloneDX JSON describing `vsift-cli`; and a
skill entry that is a link or leaves the skill directory. An archive is
deterministic: entries in path order, every time stamp the source commit's time, owner
and group 0 without names, modes 0755 and 0644, and a gzip header without a name or
time. `verify` reads an archive back and requires exactly the packaged entries, modes
and bytes, and nothing else, and requires the archive's bytes to equal a fresh
packaging of the same inputs and commit time (which fixes owners, times and
compression too).

`SHA256SUMS` lists the three archives in the format `sha256sum --check` and
`shasum -a 256 --check` read. A checksum from the same server as the archive is not
proof of origin (threat SEC-23); the Sigstore attestation of PR 10 is.

## 3. The governance workflow lint

`cargo run --locked -p vsift-governance -- check` (the CI **Governance** job) parses
every file in `.github/workflows/` and fails when a workflow:

1. uses an action or reusable workflow not pinned to a full 40-character commit SHA (a
   local `./` action or a `docker://…@sha256:` image digest is accepted);
2. is triggered by `pull_request_target`;
3. has no top-level `permissions`, or grants a `write` scope there, or uses
   `read-all`/`write-all` anywhere (a job names the write scopes it needs itself);
4. requests `id-token: write` anywhere but the release workflow's `attest` and
   `publish` jobs;
5. interpolates a `${{ }}` expression into a `run` script from anything but the
   `runner`, `matrix`, `strategy` and `job` contexts and a closed list of `github`
   fields (`sha`, `run_id`, `run_number`, `run_attempt`, `repository`,
   `repository_id`, `workspace`, `server_url`, `event_name`); anything else reaches the
   script through `env` and a quoted variable;
6. for `release.yml` only: runs a `cargo build` other than the command above, passes
   any feature, package-set or profile selection to any cargo command but `cargo
   install` of a pinned tool, or names a development feature, the smoke-test stand-in,
   the crash-campaign tool, a `CARGO_PROFILE_` override or `debug-assertions` in any
   key or value;
7. for `release.yml` only (P13 PR 10), breaks a publishing rule:
   - it is triggered by anything but `pull_request`, `push` to branches (never tags) and
     `workflow_dispatch`, whose one input `dry_run` is a boolean defaulting to `true`;
   - it has no `plan`, `attest` or `publish` job, or `attest` or `publish` can run
     without every one of `github.event_name == 'workflow_dispatch'`,
     `github.event.inputs.dry_run == 'false'`, `github.repository == 'smormah/vsift'`,
     `startsWith(github.ref, 'refs/tags/v')` and `needs.plan.outputs.mode == 'publish'`,
     or its condition holds `||`, `!`, `always()`, `failure()` or `cancelled()`;
   - a job other than `attest` (`id-token`, `attestations`) or `publish` (`id-token`,
     `contents`) writes anything;
   - `publish` does not run in the `release` environment, another job names an
     environment, or a publish could be cancelled part-way (`publish` needs
     `cancel-in-progress: false`; the run's own setting may cancel only non-dispatch runs);
   - `npm publish` runs outside `publish`, or without `--provenance`, or without an
     explicit `--tag next` or `--tag latest` (never npm's default, a variable or
     `--tag=...`); `gh release` runs outside `publish`, runs a subcommand but `create`,
     `edit` or `view`, creates a release without `--verify-tag` and `--draft`, edits one
     with a flag but `--repo`, `--draft=false` and `--latest`; `gh api` runs other than
     `gh api repos/smormah/vsift/releases/latest --jq <filter>`; any job runs
     `npm unpublish`, `npm deprecate`, `gh release delete`, `git tag` or `git push`;
   - (P14 PR 8, the channels) the `plan` job does not export `channel`; a step that runs
     `npm publish` or `gh release create` or `edit` serves both channels, or does not run
     only under `needs.plan.outputs.channel == 'prerelease'` for `--tag next` and a
     `--prerelease --latest=false` release, or only under the channel `stable` for `--tag
     latest`, a release created without `--prerelease` and an edit with `--latest`, or its
     condition holds `||` or another bypass; an `npm publish` step does not open with the
     shape check of its channel (`[[ "${VERSION}" =~ ... ]]`: a suffix for `next`, none
     for `latest`); the stable release is marked latest when the draft is created rather
     than by the edit that publishes it; the `npm publish` step of the stable release does not
     first require every package's `latest` to be at or below the version published
     (`sort -V`), or (since 2026-10-17) does not first read `latest` again (`timeout 20 npm view
     ... --prefer-online`) and refuse unless it is still the plan's value or this version; the `publish` job does not record the dist-tags before publishing, does
     not read both tags back after each channel's publish, or does not confirm that
     GitHub's latest release is the stable tag;
   - (P14 PR 8, the stable path's inputs) the `plan` job does not check out with
     `fetch-depth: 0`, or does not run `vsift-release publish-plan` with `--registry`,
     `--evidence`, `--run-id` and `--date`, or does not run `vsift-release candidate-delta
     --github-output` (which names the accepted candidate) or `vsift-governance
     release-evidence --complete-for <candidate> --commit <candidate commit>` (the evidence
     ledger's completeness check, RQ-20); a `curl` takes any flag but the reviewed read-only ones (`--silent`, `--show-error`,
     `--proto`, `--max-time`, `--retry`, `--output`, `--write-out`) or any URL but
     `https://registry.npmjs.org/...`; `attest` or `publish` runs `curl` or `wget`;
   - `npm-package` does not export `tarball-sums`, or `npm-qualify`, `plan`, `attest` or
     `publish` does not check its tarballs against that output with
     `sha256sum --check --strict`; `attest` or `publish` uses an action other than its
     few reviewed ones, downloads anything but this run's `release-dry-run`,
     `npm-packages` or `publish-plan` artifact (no pattern, run id, repository or token),
     or runs `cargo`, `vsift-release`, `npm pack`, `npm install`, `npm ci`, `npx`, `pnpm`
     or `yarn`;
   - the workflow names any secret but `NPM_BOOTSTRAP_TOKEN`, or names that one outside
     `publish`;
   - (2026-10-17, the guard, #337 and governance rule 15) there is no job `guard`, or a job
     other than `guard` has no `needs` (so it could start before the guard accepted the run),
     or a job that waits for the guard has `always()`, `failure()` or `cancelled()` in its
     condition, or `continue-on-error` (so it could run after a refusal), or `guard` has
     `needs`, an `if` or `continue-on-error` on itself or a step, any permission but exactly
     `actions: read` and `contents: read`, an action other than `actions/checkout`, or a
     command that builds or installs (`cargo`, `npm install`, `curl` and the rest of the
     privileged jobs' list); or no step of `guard` is given `EVENT_NAME`, `GIT_REF`,
     `REPOSITORY`, `DRY_RUN_INPUT` and `SOURCE_COMMIT` from exactly `github.event_name`,
     `github.ref`, `github.repository`, `github.event.inputs.dry_run` and `github.sha` while it
     reads every Nightly run of exactly that commit in any status
     (`.../actions/workflows/nightly.yml/runs?head_sha=${SOURCE_COMMIT}&per_page=100`), each
     run's jobs, the release branch's head (`.../git/ref/heads/${release_branch}`), the
     comparison of the commit with that head's own commit (`.../compare/${SOURCE_COMMIT}...${head}`,
     so a tag named like the branch cannot stand in for it), the commit the tag names
     (`.../git/ref/tags/${tag}`, an annotated tag peeled through `.../git/tags/<object>`) and the
     rules that apply to the branch (`.../rules/branches/${release_branch}`), and refuses with
     `[ "${tagged}" = "${SOURCE_COMMIT}" ] || refuse`; `gh api` in `guard` reads only those paths of
     `smormah/vsift`, with `--jq` and nothing else;
   - (2026-10-17, the read-back window, #403) the `plan` job does not export `tags-before` (the
     dist-tags it read, before anything was published), or the `publish` job records its "before"
     values from anything but that output or with an `npm view` of its own; or a channel's
     read-back step does not poll in a loop bounded both by rounds and by a deadline
     (`deadline=$((SECONDS + D))`, `for attempt in $(seq 1 N); do`, `if [ "${settled}" = yes ] || [
     "${SECONDS}" -ge "${deadline}" ]; then` and `sleep S`), runs an `npm view` without `timeout
     <seconds>` in front of it, waits less than 15 minutes for a registry that answers at once, or
     can take, with every read's cut-off counted for the four packages, more than the `publish`
     job's `timeout-minutes` less 15; does not keep waiting (`settled=no`) while the tag that moves
     does not name the version or the tag that stays gave no answer; does not fail at once, inside
     the loop, when the tag that moves reads neither the version nor its value before or the tag
     that stays reads anything but its value before; or does not fail when the loop ends unsettled;
   - (2026-10-17, the re-check after the approval) the `publish` job has no step, before
     every step that publishes or releases and without `if` or `continue-on-error`, that is
     given the guard's `env` and `VERSION` from `needs.plan.outputs.version`, reads the
     guard's paths, refuses with `[ "${GIT_REF}" = "refs/tags/${tag}" ] || refuse` and `[ "${tagged}" =
     "${SOURCE_COMMIT}" ] || refuse`, and carries the guard's `nightly_passed`, `on_release_branch`,
     `tag_commit` and `release_branch_protected` functions character for character; `gh api` in
     `publish` reads only the latest release and the guard's paths;
8. in **every** workflow (P14 PR 8): runs a command that moves a dist-tag (`npm` or
   `pnpm` followed by `dist-tag` or `dist-tags`, whatever sits between, quoting and spacing
   included; `yarn npm tag`; a call of the registry's `.../dist-tags/...` endpoint), or
   publishes a package (`npm`, `pnpm`, `yarn`, `bun` or `cargo` followed by `publish`)
   anywhere but the release workflow's `publish` job. Reads such as `npm view <package>
   dist-tags` are not refused. `latest` therefore moves only by publishing a stable
   version;
9. in **every** workflow (2026-10-19, [ADR 0025](../decisions/0025-self-hosted-runner-policy.md),
   the self-hosted runner policy): names a `runs-on` label that is neither a GitHub-hosted image
   nor the `loki` runner's, or one the lint cannot read (an expression other than a literal
   matrix value, outside the one listed job, `ci.yml`'s `quality`), or a runner group; reaches the
   self-hosted runner without naming `loki`; or has a job on `loki` and is `release.yml`, is
   triggered by anything but `schedule` and `workflow_dispatch`, holds any write scope but a
   GitHub-hosted job's `issues: write`, runs a job in an environment, names a secret but
   `secrets.GITHUB_TOKEN` or passes secrets to a reusable workflow, or its `loki` job has
   permissions other than exactly `contents: read`, names a secret or `github.token`, checks out
   without `persist-credentials: false` or has no literal `timeout-minutes` of 1 to 720. The
   release workflow therefore never runs on a self-hosted runner, as `--deny-self-hosted-runners`
   assumes when its attestations are verified.

The lint reads the YAML tree, so flow mappings and aliases are seen, and it refuses
merge keys (`<<`). Comments are not part of the tree. Its tests are in
`tools/vsift-governance/src/workflows.rs` (rule 8 included), for rule 9
`workflows/runners/tests.rs` (27 broken copies of the real `loki-overnight.yml`, the overnight job
moved into a copy of the real `release.yml`, and synthetic workflows for matrices, computed runners,
reusable workflows and the hosted labels), and, for rule 7, `workflows/publish.rs`, which checks the real `release.yml` and a mutation of it for
every rule: 127 deliberately broken copies, 28 for the P13 rules, 37 for the channels,
the stable path's inputs and the evidence step, 28 for the guard and the read-back
window, 12 for the re-check after the approval, 19 for the holes an independent
review found in them and 3 for the stable publish's fresh read of `latest` (2026-10-17),
each of which the lint must name.

*Why the dispatch input is compared as a string:* GitHub compares values of different
types loosely, turning `null` and `false` both into 0, so `inputs.dry_run == false` is
also true on a push or pull request, which has no inputs at all. The workflow compares
`github.event.inputs.dry_run == 'false'` (a string, and empty without a dispatch) and
checks the event separately; the lint requires exactly that form.

## 4. Running it and checking a result

Run it from the Actions tab (**Release**, *Run workflow*) or let a pull request run it.
To check a run's archives on your machine, download the `release-dry-run` artifact and:

```console
sha256sum --check --strict SHA256SUMS        # Linux
shasum -a 256 --check SHA256SUMS             # macOS
tar -xzf vsift-<version>-<target>.tar.gz
vsift-<version>-<target>/vsift --version      # names the commit it was built from
```

To package locally from a checkout, with an executable, notices and SBOM made as the
workflow makes them:

```console
cargo run --locked -p vsift-release -- package --target <target> --binary <path to vsift> \
  --notices <THIRD-PARTY-NOTICES> --sbom <vsift.cdx.json> \
  --source-date-epoch "$(git log -1 --format=%ct)" --out-dir <existing directory>
```

A dry-run archive is unsigned and unattested. Do not distribute one.

## 5. The npm packages and their qualification (P13 PR 9)

The same workflow turns the archives into the four npm packages and runs their
qualification. Both jobs run after `package`, and neither can publish: they have `contents: read`, no OIDC
token and no secret, and the only registry they write to runs on the job's own loopback
address and is gone when the job ends.

| Job | Runner | What it does |
| --- | --- | --- |
| `npm-package` | `ubuntu-24.04` | `vsift-release npm` reads the three archives back (each must be canonical) and writes the package folders; `npm pack` packs each twice and the two tarballs must be identical; `vsift-release npm-verify` checks every tarball against a fresh assembly; keeps the four tarballs as the run's `npm-packages` artifact for 7 days. |
| `npm-qualify` | `windows-2025`, `macos-15`, `ubuntu-24.04`, each with npm, pnpm, Yarn and Bun (12 jobs) | Installs the pinned Verdaccio and package manager from the public registry (read-only, no credentials), then runs the driver in `npm/qualification/`, which publishes the tarballs to Verdaccio on `127.0.0.1:4873` and checks the package manager against it (ADR 0023, PR 9 note). The job summary lists every check. |

**The packages.**

| Package | Holds |
| --- | --- |
| `vsift-cli` (the `vsift` command) | `bin/vsift.cjs`, `lib/launcher.cjs`, `package.json` and `README.md` from `npm/vsift-cli/`; `platform-digests.json` (each executable's size and SHA-256, computed from the archives); `LICENSE`, `LICENSE-APACHE`, `LICENSE-MIT`; `skills/vsift/` |
| `@vsift/win32-x64`, `@vsift/darwin-arm64`, `@vsift/linux-x64` | the target's `vsift` or `vsift.exe` (mode 0755), its `THIRD-PARTY-NOTICES`, the licence files, a README and a manifest with `os`, `cpu`, `preferUnplugged` and `publishConfig.access: public` |

The launcher lists the platform packages as `optionalDependencies` at its own exact
version. No package has a lifecycle script, a `gypfile` or a `binding.gyp`, and no
manifest names a person: `vsift-governance check` holds every `package.json` under `npm/`
to that, and `npm-verify` holds the packed tarballs to it too. The version of every
package is the workspace version; `npm/vsift-cli/package.json` must carry it, with each
optional dependency at the same version (a release-tool test and the governance check
fail otherwise). To release a new version, change the workspace version and those four
places in `npm/vsift-cli/package.json` together.

**Pinned tools** (workflow `env`): Node.js 22.23.3 (with the npm it ships), Bun 1.2.23,
pnpm 12.8.1, Yarn 4.18.1, Verdaccio 6.10.4. Update them in one reviewed change.

**Reproducing it locally.** On Linux or macOS (the tarballs must be packed where Unix
modes exist), with the three archives of one run in `dist/`:

```console
mkdir -p npm-dist/packages npm-dist/tarballs
cargo run --locked -p vsift-release -- npm --archive dist/<archive> --archive dist/<archive> \
  --archive dist/<archive> --out-dir npm-dist/packages
(cd npm-dist/packages/vsift-cli && npm pack --ignore-scripts --pack-destination ../../tarballs)
# ... and the same for vsift-darwin-arm64, vsift-linux-x64 and vsift-win32-x64
cargo run --locked -p vsift-release -- npm-verify --archive ... --tarball npm-dist/tarballs/<tarball> ...
npm install --prefix /tmp/tools --ignore-scripts verdaccio@6.10.4
node npm/qualification/qualify.cjs --manager npm --packages npm-dist/tarballs \
  --verdaccio /tmp/tools/node_modules/verdaccio/bin/verdaccio \
  --version-line "vsift 0.1.0 (<12 digits>)" --work /tmp/qualification
```

The driver refuses any registry but `http://127.0.0.1:4873/`, removes every
`npm_config_`, `YARN_`, `BUN_CONFIG_`, `PNPM_` and token variable before it runs a
package manager, and uses a throwaway Verdaccio user whose token is written only to a
scratch npmrc scoped to that address. Never point it at a real registry, and never add
`npm login`, `npm adduser` or a publish to the public registry to this workflow: PR 10's
protected `publish` job is the only publishing path.

**Names and dist-tags, for PR 10's publish step and release notes.** The launcher package
is `vsift-cli` (npm refused the unscoped `vsift` as too similar to `sift` and `tsify`;
ADR 0009 note of 2026-09-30); the command it installs is `vsift`; the platform packages
are `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64`. `vsift-cli@0.0.0`,
the maintainer's placeholder, is `latest` until a stable version is published; a
pre-release is published with `--tag next`, so its release notes tell users to install
`vsift-cli@next`, and a stable version is published with `--tag latest`, so its notes tell
them to install `vsift-cli`. Yarn 4.18 holds every new version back for a day
(`npmMinimalAgeGate`), for `vsift-cli` and `@vsift/*` alike (the stable release included:
users of `latest` on Yarn get it a day after it is published): the release notes say so,
with the `npmPreapprovedPackages` workaround of `install.md`.

## 6. Attestation and publishing (P13 PR 10, P14 PR 8): the maintainer's runbook

**Status: sections 6.2 to 6.4 were completed on 2026-10-01 for 0.1.0** (the record
[`p13-distribution.md`](../planning/p13-distribution.md) says how each was checked): the
one-time setup, the tag `v0.1.0`, a dry run on it, two real dispatches (the first failed
with `ENEEDAUTH` before anything was published, the second published; see 6.2's preflight
and 6.5) and the verification. They are written below as the steps for the next release.
**Sections 6.7 to 6.9 (P14 PR 8) are the stable path, the rule that holds a stable version
to its candidate and the test of the publishing shell; they have not run against the real
services.** The release candidate `0.2.0-rc.1` (P14 PR 10) is their first real use, with
section 6.3, and the stable release `0.2.0` (PR 12) the second.
Every setting below is the maintainer's to make (ADR 0023, "Maintainer-only actions"); no
agent or workflow changes them. Nothing here needs an e-mail address or other personal
detail beyond the public GitHub owner `smormah` and repository `vsift`, and no step asks
for one.

### 6.1 The three publishing jobs

| Job | Runs | Permissions | What it does |
| --- | --- | --- | --- |
| `plan` | every run, after all twelve `npm-qualify` jobs | `contents: read` | Requires the downloaded archives to equal `package`'s `SHA256SUMS` output and the tarballs to equal `npm-package`'s `tarball-sums` output, by SHA-256. Reads npm's public metadata of the four packages with anonymous GETs (`https://registry.npmjs.org/<package>`, status and body kept in the runner's temporary folder; of the body only the dist-tags and each version's integrity are used, and nothing of it is printed or uploaded). Runs `vsift-release publish-plan`, which reads every archive back, re-assembles the npm packages and requires each tarball to be its package byte for byte, decides the mode and the channel (below), checks the guards of the channel and writes the plan: `publish-plan.md` (added to the job summary, even when the plan is refused), `publish-plan.json`, the release notes, the SBOM and notices of each target as separate release assets, and the digest lists of the release assets and of every file to attest. Checks out the full history, which the candidate comparison of a stable plan needs. Before the plan it names the accepted candidate (`vsift-release candidate-delta --github-output`) and, when there is one, runs the evidence ledger's completeness check for it (`vsift-governance release-evidence --complete-for <candidate> --commit <candidate commit>`), keeping the check's exit status and output in the runner's temporary folder for the plan to read (`--evidence`); the plan also receives the run's id and the date (`--run-id`, `--date`), which a stable plan that passes records in `release-delta.json` (6.7). Outputs `mode` (`dry-run` or `publish`), `version`, `tag`, `channel` (`prerelease` for a pre-release; for a stable version, the channel `stable`), `tags-before` (since 2026-10-17: the four packages' dist-tags as the plan read them, `none` for a tag that is not set and `unknown` for what could not be read or is not a version, which the publish job compares with; a re-run of failed jobs keeps it) and the plan's own digests. |
| `attest` | only when publishing | `id-token: write`, `attestations: write` | Downloads the archives, tarballs and plan, requires each to match the earlier jobs' outputs, and creates a Sigstore build-provenance attestation (`actions/attest-build-provenance` v4.2.2, pinned by commit) for every archive, `SHA256SUMS`, SBOM, notices file and npm tarball, named in the plan's `attestation-subjects.sha256`. |
| `publish` | only when publishing, after `attest`, in the `release` environment | `id-token: write`, `contents: write`, and (since 2026-10-17) `actions: read` | Waits for the maintainer's approval. Checks every file again, records the four packages' dist-tags **from the plan's `tags-before` output** (never read again here, 6.4), **re-checks the `guard` job's conditions after the approval** (the ref is the planned version's tag in `smormah/vsift`, the tag still names the commit this run built, that commit is still on its release branch, the branch's rulesets still hold the five rule types of 6.2 step 8, and for a candidate the latest Nightly run that tested exactly that commit still passed with none still running; the guard's own functions, which the lint compares; 6.18), then with the npm of Node.js 24.21.0 (npm 11.19.0; trusted publishing needs 11.5.1 or later) runs, in this order, `npm publish ./npm-packages/<tarball> --access public --provenance --ignore-scripts` with `--tag next` (channel `prerelease`) or `--tag latest` (channel `stable`) for `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64` and then `vsift-cli`. Each channel has its own step, written out in full and run only for its channel; each checks the version's shape itself first, and the step of the stable release also requires every package's `latest` to be a stable version at or below the one published, and (since 2026-10-17) reads `latest` again first and refuses unless it is still what the plan read or already this version: two publishes of a stable version waiting for approval would otherwise both pass on their plans' values, and the lower one, approved second, would move `latest` backwards. It then reads both dist-tags of every package back, every 30 seconds until 20 minutes have passed, past npm's own cache, each read cut off after 20 seconds: a pre-release moved `next` and left `latest` where it was, a stable version moved `latest` and left `next`; a tag that still reads its value from before is waited for, and any other value fails the step at once (6.4, #403). Last, `gh release create v<version> --verify-tag --draft` with the archives, `SHA256SUMS`, SBOMs and notices and the plan's notes (`--prerelease --latest=false` for a pre-release), and `gh release edit` publishes the draft (`--latest` for the stable release, which is then confirmed to be GitHub's latest release). |

**The mode and the channel.** `vsift-release publish-plan` answers `publish` only for a
`workflow_dispatch` with `dry_run` cleared, in `smormah/vsift`, on the ref
`refs/tags/v<workspace version>`. Pull requests and pushes are dry runs, and so is a
dispatch with `dry_run` set. A dispatch with `dry_run` cleared anywhere else is refused by the
`guard` job before anything is built (6.18, "Which refs may run each mode"), and the plan job
names the reason too if it is ever reached. The channel comes from the version (section 1): with a
pre-release suffix it is `prerelease`, without one it is the channel `stable`.

**What a plan enforces.** A stable plan lists guards (section 6.7) and a failed guard stops
the run only where the plan is *enforced*: a publish, and a dispatch on the version's own
tag even with `dry_run` set, which is the rehearsal of a publish and so fails whenever the
real run would. Any other run (a pull request, a push, a dispatch elsewhere) shows the same
findings under "A real publication of this plan would be refused, because:" and carries on.
A refused plan still writes its explanation to the job summary and writes no attestation
list, release notes or command.

**What is published is what the npm jobs checked.** `npm-package` packs the tarballs once and
exports their SHA-256 list as a job output, which no later job can change. Every
`npm-qualify` job requires the tarballs it installs to match that list, and so do
`plan`, `attest` and `publish`, which download only this run's artifacts. `attest` and
`publish` check out no code and build, pack or install nothing; they run a few lines of
shell and two reviewed actions. `tools/vsift-release`'s tests
`the_release_workflow_runs_the_planned_commands` and
`the_two_publishing_steps_differ_only_in_their_guards_and_their_dist_tag` hold those lines
to the plan's commands word for word and the two channels' steps to each other, and the
lint (section 3, rule 7) holds the rest.

**What the dry run shows.** Open a Release run and its **Publish plan** job summary: the
mode and why, whether `latest` moves (in bold, at the top), the version, dist-tag and Git
tag, a table of what the publication does to each package's dist-tags (from what to what,
and what stays), the guards, and three tables, each file with its SHA-256: the files to
attest, the four `npm publish` commands in order, and the GitHub release commands with
their assets. The same plan is the run's `publish-plan` artifact.

### 6.2 One-time setup, in this order

1. **Merge the pull requests the release needs** (for 0.1.0, P13's PRs 0-11 and the
   release-prep change; PR 12, the ledger follow-up, came after the publish, ADR 0023
   decision B). Read the guide users will follow, [`install.md`](install.md), before you
   publish.
2. **Fork pull requests** (Settings, Actions, General, "Approval for running fork pull
   request workflows from contributors"): choose **Require approval for all external
   contributors** (today it is "first-time contributors"). Under "Workflow permissions"
   keep **Read repository contents and packages permissions** and leave **Allow GitHub
   Actions to create and approve pull requests** cleared.
3. **The `release` environment** (Settings, Environments, New environment, name
   `release`):
   - **Required reviewers:** yourself. Leave **Prevent self-review** cleared, or you
     could not approve your own dispatch.
   - **Allow administrators to bypass configured protection rules:** clear it, so the
     approval is always asked for.
   - **Deployment branches and tags:** **Selected branches and tags**, then one rule of
     type **Tag** with the pattern `v*`, and no branch rule. The `publish` job can then
     run only on a release tag.
   - **Environment secrets:** none, unless you choose path B in step 5.
4. **The tag ruleset** (Settings, Rules, Rulesets, New ruleset, **New tag ruleset**):
   name `release tags`, enforcement **Active**, target tags **Include by pattern** `v*`,
   rules **Restrict creations**, **Restrict updates**, **Restrict deletions** and **Block
   force pushes**, and in the bypass list the **Repository admin** role (you). Only you
   can then create, move or delete a `v*` tag, and the workflow never creates one.
5. **The first publish of `@vsift/win32-x64`, `@vsift/darwin-arm64` and
   `@vsift/linux-x64`** (done on 2026-10-01 by path A; a later release needs nothing
   here). npm lets you configure a trusted publisher only on a package that exists, and
   these three did not yet (`vsift-cli` did: the `0.0.0` placeholder). ADR 0023 allows two
   ways; choose one:
   - **Path A (recommended; no token ever exists): placeholders published by you with
     two-factor authentication**, as you did for `vsift-cli`. For each of the three names,
     in an empty folder, write the two files below by hand (do not run `npm init`, which
     copies your npm profile's name into `author`), then run `npm publish --access public`
     from that folder and confirm with your second factor.

     `package.json` (for `@vsift/win32-x64`; change the name for the other two):

     ```json
     {
       "name": "@vsift/win32-x64",
       "version": "0.0.0",
       "description": "Placeholder that holds the name until VSift's first release. Install vsift-cli instead.",
       "license": "MIT OR Apache-2.0",
       "homepage": "https://github.com/smormah/vsift",
       "repository": { "type": "git", "url": "git+https://github.com/smormah/vsift.git" }
     }
     ```

     `README.md`: one line saying the package is a placeholder and to install
     `vsift-cli`. No code, no binary, no scripts, no `author`. Each placeholder stays its
     package's `latest`; the release goes under `next`, and `vsift-cli` selects the
     platform packages by exact version, so nobody installs a placeholder by accident.
     This publishes three more placeholders, which ADR 0009's note of 2026-09-30 allowed
     only for the launcher: record the choice there.
   - **Path B: a short-lived token stored only in the `release` environment.** Create an
     npm granular access token with the shortest expiry npm offers, read and write access
     to the `@vsift` scope only, and no other permission; add it to the `release`
     environment (not to the repository) as the secret `NPM_BOOTSTRAP_TOKEN`. npm uses
     trusted publishing when it can and falls back to this token for the three packages
     that have no trusted publisher yet; the token only ever reaches npm through an
     environment variable. The first release then publishes all four packages with
     provenance. **Immediately afterwards** revoke the token on npmjs.com, delete the
     environment secret, and do step 6 for the three new packages.
6. **Trusted publishers, one per package** (npmjs.com, the package, Settings, Trusted
   Publisher, GitHub Actions), for `vsift-cli`, `@vsift/win32-x64`, `@vsift/darwin-arm64`
   and `@vsift/linux-x64`, each exactly:

   | Field | Value |
   | --- | --- |
   | Organization or user | `smormah` |
   | Repository | `vsift` |
   | Workflow filename | `release.yml` |
   | Environment name | `release` |
   | Allowed actions | tick **npm publish**; leave **npm dist-tag** cleared |

   npm cannot change a trusted publisher once created; to fix a field, delete it and add
   it again. Configurations created after 2026-09-03 allow only `npm stage publish`
   unless **npm publish** is ticked, and the workflow publishes directly.

   **Preflight, before the first real dispatch of every release.** Filling in the form is
   not enough. Clicking **Set up connection** and confirming with your second factor is
   what saves the entry, and no dry run can tell whether that happened, because the
   exchange with npm exists only in the `publish` job. So, on npmjs.com, open each of the
   four packages' Settings page and confirm that a **saved Trusted Publisher entry is
   listed** (not the empty form) with the values of the table and **npm publish** allowed.
   npm shows these settings only to the package owner, so nothing else can check them for
   you. The first real dispatch of 2026-10-01 failed with `ENEEDAUTH` (section 6.5), and the
   maintainer then reported that the connections had not been set up, although an earlier
   report said they were
   ([L-100](../planning/known-limits.md#l-100)).
7. **Lock token publishing** (each package, Settings, Publishing access): choose
   **Require two-factor authentication and disallow tokens** once its trusted publisher
   exists (after path B's release, for the three platform packages). Trusted publishing
   keeps working; a stolen token no longer can publish. Keep two-factor authentication
   required for the `vsift` organisation's members.
8. **The release-branch rulesets** (added 2026-10-17, before the first cut on a release branch;
   **applied by the maintainer on 2026-10-17** as two rulesets, "release branches: creation" and
   "release branches: changes"; Settings, Rules, Rulesets, New ruleset, **New branch ruleset**): name `release branches`,
   enforcement **Active**, target branches **Include by pattern** `release/*`, rules **Restrict
   creations**, **Restrict deletions**, **Require a pull request before merging** (squash, as on
   `main`), **Require status checks to pass** with `CI result`, `Documentation` and `Governance`
   (the CI workflow runs on pull requests to `release/**`; the Release dry run runs there too,
   and stays optional as on `main`), and **Block force pushes**. **The bypass lists, as applied:**
   "release branches: creation" lets the **Repository admin** role (you) create and delete a
   release branch, so only you do, as only you create a `v*` tag; "release branches: changes" lets
   you bypass it **for pull requests only**, so nobody, you included, pushes to a release branch
   directly. **What the supervisor saw on 2026-10-17** with a throwaway `release/0.0.x`: creating the
   branch by push works **once the commit has a passed `CI result`** (it was refused while `main`'s
   CI was still running on it); a direct push to the branch afterwards is refused with "Changes must
   be made through a pull request"; deleting the branch works. **The `guard` job and the publish job's re-check read the rules that apply
   to the release branch** (`gh api repos/smormah/vsift/rules/branches/release/<MAJOR>.<MINOR>.x`)
   and refuse a candidate or the stable release unless the rule types `creation`, `deletion`,
   `non_fast_forward`, `pull_request` and `required_status_checks` are all there; they check the
   types, not the rules' settings (which checks are required, who may bypass), so read those
   yourself ([L-173](../planning/known-limits.md#l-173)). The `release` environment keeps its one rule
   (tags `v*`): publishing is still from a tag, never from the branch.

### 6.3 Publishing a release candidate (or another pre-release)

**Since 2026-10-17 the candidate is tagged on its release branch** (6.8, 6.18 step 2): the
first candidate's bump (step 1) still merges to `main`, and the cut creates
`release/<MAJOR>.<MINOR>.x` at a commit of `main` that carries it and whose nightly passed; a
later candidate's fix and bump merge into the release branch by pull request, and the nightly is
dispatched on the branch before its head is tagged. Step 2 tags that commit; steps 3 and 4
dispatch on the tag as before, and the `guard` job refuses, in seconds, a tag that is not on the
release branch or whose commit has no passed nightly. The text below is the procedure as it ran
for the candidates of `0.2.0` and `0.2.1`, on `main`.

This is the procedure of 0.1.0 (2026-10-01) and the one for `0.2.0-rc.1`, `0.2.0-rc.2` and
`0.2.0-rc.3` (P14 PR 10, three times: 6.10, 6.11 and 6.12 are these steps with the real commands) and for
`0.2.1-rc.1`, `0.2.1-rc.2` and `0.2.1-rc.3` (6.14, 6.15 and 6.16). It moves `next` and nothing else; `latest` stays where it is (the empty `0.0.0` placeholder
until 2026-10-09, `0.2.0` since). The stable version has its own
procedure (6.7), which starts from a published and accepted candidate.

1. **Choose the version and bump it.** The workspace version in `Cargo.toml` is the
   published version, and nothing else decides it. Bump it in the files that hold it, and
   in no other file (6.8 lists the same files, and the work record and documents in which a stable commit
   may also differ from its candidate): `Cargo.toml` (`[workspace.package] version` and
   the `version` of the five workspace dependencies), `Cargo.lock`, `fuzz/Cargo.toml`
   (four dependencies), `fuzz/Cargo.lock`, and `npm/vsift-cli/package.json` (its own
   `version` and the exact version of its three optional dependencies); update
   `CHANGELOG.md`'s release section. A candidate is `X.Y.Z-rc.N` with a positive `N`
   (`0.2.0-rc.1`, then `0.2.0-rc.2`, which two findings of the first required, then `0.2.0-rc.3`,
   which a failed agent-trial gate of the second required; a fourth only if findings need it). Merge the bump to `main`. From the first candidate's tag on, only fixes
   for findings may change the code (6.8).

   **The user guide goes with the first bump to a new release** (`0.1.0` to `0.2.0-rc.1`, and
   again `0.2.0` to `0.2.1-rc.1`, which moved the marker to `0.2.1`), and only then. The guide names a release, not a candidate, and its two checks compare
   the release part of the version (`0.2.0`): so the bump to `0.2.0-rc.1` re-runs the
   guide's examples, regenerates its two reference pages and moves the version marker in
   `docs/guide/index.md` (the `Guide` workflow fails the bump until it does; steps in
   [Development](../development.md#the-user-guide-and-its-checks)), while the bumps to `-rc.2`,
   `-rc.3` and the final `0.2.0` change no generated page and no marker under `docs/guide/` (only the
   sentence that names the candidate moves, which is prose; a fix that changes `--help`, a remediation or a
   limit regenerates the pages in its own pull request, as two fixes of the third candidate did). That is what keeps the commit that
   publishes `0.2.0` inside the list of 6.8.
2. **Create the tag** on the merged commit of `main` and push it:

   ```console
   git tag -a v0.2.0-rc.1 -m "VSift 0.2.0-rc.1" <commit>
   git push origin v0.2.0-rc.1
   ```

3. **Dry run on the tag.** Actions, **Release**, **Run workflow**, "Use workflow from"
   **Tags: v0.2.0-rc.1**, leave **dry_run** ticked. When it finishes, read the **Publish
   plan** summary: mode `dry-run`, "`latest` is not touched", the version (a release
   candidate), `next`, a table showing `next` moving from its present value to the new
   version and `latest` staying, the guards, and the digests.
4. **Publish.** Run it again on the same tag with **dry_run cleared**. The plan job must
   say **PUBLISH a release candidate**, `attest` runs, and `publish` waits with "Waiting
   for review". Check the plan summary once more, then **Review deployments**, tick
   `release`, **Approve and deploy**. The run waits in the environment as long as you take
   (a wait of about 45 minutes did no harm on 2026-10-01), but a run waits for approval for
   at most 30 days and its files, the artifacts every job downloads, expire after 7: approve
   within a week, or the run cannot publish and you dispatch again.
5. **Verify** (section 6.4). For 0.1.0 the ledger follow-up (P13 PR 12) recorded the runs
   in the qualification record's "First publish" section
   ([`p13-distribution.md`](../planning/p13-distribution.md)), marked ADR 0023 Accepted and
   set P13 complete; for a later release, bring the run's link, the output of the section
   6.4 commands and anything that failed or was re-run to the record that release belongs
   to (for the P14 candidate, its evidence-ledger entries). A candidate is never announced.

### 6.4 Verifying provenance and attestations after publishing

From a machine with no npm credentials (an empty user and global configuration, as in
ADR 0009's name checks), for a release candidate or other pre-release (the commands are
those of 0.1.0; use the version you published):

```console
npm view vsift-cli dist-tags        # latest: unchanged ('0.0.0' while no stable version existed, '0.2.0' since 2026-10-09), next: '0.2.0-rc.1'
npm view vsift-cli@next dist.attestations
mkdir check && cd check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures                # registry signatures and provenance attestations verified
npx vsift --version                 # vsift 0.2.0-rc.1 (<the tag's first 12 commit digits>)
```

npmjs.com shows each version as "Built and signed on GitHub Actions", linking the
workflow run. For the GitHub release and the tarballs (`gh` signed in):

```console
gh release download v0.2.0-rc.1 --repo smormah/vsift --dir release
cd release && sha256sum --check --strict SHA256SUMS
for file in *; do
  gh attestation verify "$file" --repo smormah/vsift \
    --signer-workflow smormah/vsift/.github/workflows/release.yml \
    --source-ref refs/tags/v0.2.0-rc.1 --deny-self-hosted-runners
done
npm pack vsift-cli@0.2.0-rc.1 @vsift/win32-x64@0.2.0-rc.1 @vsift/darwin-arm64@0.2.0-rc.1 @vsift/linux-x64@0.2.0-rc.1
# ... and the same gh attestation verify for each .tgz
```

The release must be a pre-release that is not marked latest, with ten assets: three
archives, `SHA256SUMS`, three SBOMs and three notices files. GitHub's own latest release
(`gh api repos/smormah/vsift/releases/latest --jq .tag_name`) must not change: it was a 404 while every
release was a pre-release, and is the stable release's tag (`v0.2.0`) since 2026-10-09. For the **stable** release
the checks differ in four places, listed in 6.7.

**The same checks, from a runner that holds no publishing credential.** The workflow
`P14 verify release` (`.github/workflows/p14-verify-release.yml`, P14 PR 2, evidence item RQ-19)
does all of the above from a hosted runner after each publish: dispatch it from the Actions tab
with the published `version` (empty means the highest on the registry). It reads the four
packages' dist-tags and npm provenance (the attestation must name `release.yml`, the tag, the
tag's commit and a GitHub-hosted builder), runs `npm audit signatures`, runs `gh attestation
verify` on all ten release files and all four tarballs, checks the checksums and GitHub's own
digest of each file, and checks the release's flags and its ten file names. It has
`contents: read` and `attestations: read`, no secret and no OIDC token, and cannot change
anything. A pull request that changes the workflow or `tools/p14-published/` runs it too.

**A stable version is asked for two more checks**, which `tools/p14-published/lib/verify.cjs`
registers by name. A pre-release is asked for neither, and a stable version whose required
check is not registered fails by name, because a verification that skipped them would be
green about the wrong thing.

- **The delta check** reads `release-delta.json` (6.7) from the `publish-plan` artifact of
  the Release run that npm's provenance names, and requires: the record's stable version is
  the one verified, and its stable commit is the commit of the tag, of the GitHub release's
  tag and of npm's provenance on all four packages; its candidate is the highest
  `v<X.Y.Z>-rc.<N>` tag of the same `X.Y.Z` (6.8) and its candidate commit that tag's commit;
  its verdict is `allowed`; and the run it names is that run. It does not compare the files
  again: the plan job did that, from the full history, and an enforced plan that failed it
  published nothing. It reads the artifact with the job's own token (`contents: read` and
  `attestations: read` were enough to list the run's artifacts and download this one, in the
  pull request run 38009140561). **The artifact lives seven days.** After that the check fails, never
  passes, and says where the same record was copied: the evidence ledger's `release_delta`
  (6.7). So the verification of the stable version is dispatched within seven days of its
  publish, and a run after that cannot be green.
- **The `latest` check** requires `latest` to be the version on all four packages (and the
  version to be published), and the GitHub release of its tag to be not a draft, not a
  pre-release and the release GitHub calls the latest.

They were registered **after the stable tag, not before** (P14 PR 13a): a change to `tools/`
between the candidate and the stable commit is refused by 6.8, and the workflow is dispatched
from `main` (`gh workflow run p14-verify-release.yml --ref main -f version=<the stable
version>`), so the code that runs is `main`'s at that moment, not the tag's. Their
tests (`tools/p14-published/test/verify.test.cjs`) run on the values of the real 0.2.0
publish. Run 36969577300 verified 0.1.0: ten of ten files and four of four tarballs
attested, the provenance of all four packages read, and `npm audit signatures` verified the two
packages a Linux runner installs (the launcher and `@vsift/linux-x64`, npm 10.9.9; the count
npm reports depends on its version, which is why the provenance check reads all four packages
itself).

A version that `npm publish` has just reported can take minutes to appear in
`npm view` ("Your package is being processed and may take a few minutes to become
available"): on 2026-10-01 the publish job's check for `next` polled three times, 30
seconds apart, before it saw 0.1.0; for `0.2.0` `latest` took about two and a half minutes;
for `0.2.1` one package still read its old `latest` after five minutes and read right when the
job was re-run, about fourteen minutes after the publish (#403). **Since 2026-10-17 the job reads
the tags every 30 seconds until 20 minutes have passed** (at most 40 rounds, asking npm past its
own cache with `--prefer-online`). Each read is cut off after 20 seconds (`timeout 20 npm view
... --fetch-retries=1`), so a stuck connection is no answer rather than a wait of npm's own
retries (about 15 minutes by its defaults); with every cut-off counted the step ends within about
23 minutes, and the job's timeout is 45. The values "before the publish" are **the plan job's
read** (its `tags-before` output), not a read of the publish job's own: a **Re-run failed jobs**
keeps the plan's outputs, so after a partial publish a stale copy of the old tag is still waited
for, never taken for a wrong answer. A tag that still reads its value from before the publish,
or gives no answer, is waited for; **any other value fails the step at once**, and so does any
move of the tag that must stay. Only a registry that has not settled fails the step on time, and
its message says that npm accepted every publication and that **Re-run failed jobs** finishes the
release (6.5).
The window is a reading of three publishes, not a bound npm states
([L-172](../planning/known-limits.md#l-172)). Nothing is wrong if your own `npm view` is empty
for a moment.

How 0.1.0 verified, for comparison: `npm audit signatures` reported 4 packages with
verified registry signatures and 4 with verified attestations; `gh attestation verify`
accepted 10 of 10 release files and 4 of 4 npm tarballs; the checksums of the three archives
matched.

### 6.5 If something goes wrong

- **The plan job fails on the tag:** nothing was attested or published. Fix the cause.
  If the fix needs a new commit, the tag may be moved (you can bypass the ruleset) as
  long as nothing was published from it; once anything was, never move a tag: publish a
  new version. For a stable version the job summary says **REFUSED** and "Refused
  because:" names each guard that failed (6.7); the commands below it are not shown,
  because none may run. A dry run on the tag fails for the same reasons the real run
  would, so a refusal there is the place to fix it.
- **`npm error code ENEEDAUTH` ("This command requires you to be logged in") in
  `publish`.** npm could not exchange the run's identity token for permission to publish
  and fell back to asking for a login, which a workflow cannot give. At its default log
  level npm prints no reason ([L-100](../planning/known-limits.md#l-100)). If this was the
  first `npm publish` of the run, nothing was published: confirm with an anonymous `npm
  view <package> versions`. Check, in this order, for each of the four packages: a
  **saved** Trusted Publisher entry exists (the preflight in 6.2; this is what stopped the
  first real run on 2026-10-01); each of its fields matches the table in 6.2 (owner
  `smormah`, repository `vsift`, workflow `release.yml`, environment `release`); **npm
  publish** is allowed; the package is the one named in the error. npm cannot edit an
  entry: delete it and add it again. Then finish the release either with **Re-run failed
  jobs** on the same run (approve the environment again; only the `publish` job runs
  again) or with a new dispatch on the same tag with `dry_run` cleared. Both are safe
  because the builds are reproducible: the second run of 2026-10-01 rebuilt everything and
  its four tarballs matched the dry run's plan, and the one tarball the failed run logged
  is the one that was finally published. Only a new dispatch has been used so far. A new
  dispatch attests again, so each file then has two attestations naming the same bytes;
  either way the 7-day limit on the run's files applies to a re-run.
- **The read-back did not settle within 20 minutes** (the step's message says "npm accepted
  every publication above"): the packages are published. Read the four packages' dist-tags
  with `npm view` from a shell with no npm login; when they show what the step waited for, use
  **Re-run failed jobs** on the same run and approve again: the versions npm holds with the same
  bytes are skipped, the read-back passes and the GitHub release is created (this is what
  finished `0.2.1`, 6.17). The re-run compares with the plan's read of the tags from before the
  publish, so a copy of the old tag that npm still serves is waited for, not taken for a wrong
  answer. If a tag shows anything else, the step would have failed at once instead; see
  "`latest` moved when it should not have" below.
- **A publish fails part-way** ([L-097](../planning/known-limits.md#l-097)): use **Re-run
  failed jobs** on the same run within 7 days, and approve again. Versions npm already
  holds with the same bytes are skipped; the job stops if npm holds other bytes for one.
  For a stable version, `latest` has by then moved on the packages published so far; the
  platform packages go first and `vsift-cli`, the one users install, last, so until the
  re-run finishes `npm install vsift-cli` still gets the old `latest`. The re-run accepts
  `latest` already being this version (the guard in the job allows "at or below"), and a
  version npm holds with other bytes stops it.
- **`latest` moved when it should not have** (the read-back after a pre-release's publish
  fails with "latest" differing from what was recorded; a pre-release is published with
  `--tag next` only, so this means the workflow is wrong, and the lint, the plan and the
  shell checks each should have stopped it): point `latest` back yourself, for each
  package that moved, with `npm dist-tag add <package>@<the previous latest> latest`
  (the previous value is in the job log under "Record the dist-tags before publishing";
  two-factor authentication), do not dispatch again, and open an issue. The workflow never
  runs `npm dist-tag`, and the lint (section 3, rule 8) refuses it in every workflow.
- **A draft release was left behind** by a failed run: delete the draft on GitHub, then
  re-run.
- **The stable release is not GitHub's latest** (the last step of the stable `publish` job
  fails after the release was published): npm is already correct. Mark it yourself with
  `gh release edit v<version> --repo smormah/vsift --latest`, then confirm with `gh api
  repos/smormah/vsift/releases/latest --jq .tag_name`. Do not re-run the job (the release
  exists).
- **A published version is bad** (a release candidate or the stable release): never unpublish it (npm
  refuses to reuse the version number, and the attestations name its bytes). Deprecate it
  on all four packages yourself (`npm deprecate <package>@<version> "<what is wrong and
  which version replaces it>"`, two-factor authentication), edit the GitHub release's notes
  to say so (do not delete the release or the tag), and publish a fixed version. A fix
  of the stable release is a new stable version (`0.2.1`), which follows its own candidate (`0.2.1-rc.1`):
  the same procedure, nothing skipped. Until it is published, `latest` still names the bad
  stable version. If it is dangerous, point `latest` back with `npm dist-tag add <package>@<good
  version> latest` for each package (two-factor authentication); for the first stable version the
  only earlier `latest` is the empty `0.0.0` placeholder, a package with no command, which
  is safe and useless: that is your choice, made at the time. Record what happened in the
  evidence ledger and the known-limits register.

### 6.6 Decisions left to the maintainer

- Settled (2026-10-01): path A for the first publish of the three platform packages
  (step 5): `0.0.0` placeholders, no token ever existed.
- Whether `attest` should also wait for the `release` environment: today it runs without
  an approval once you dispatch the tag with `dry_run` cleared, so that the approval you
  give is for publishing alone (in both real runs of 2026-10-01 it ran first and succeeded);
  gating it too means approving twice per release.
- npm's staged publishing (a trusted publisher allowed only `npm stage publish`, each
  version then approved on npmjs.com with two-factor authentication) as a second gate
  after the environment's approval: not wired; it would change the publish command. To
  be decided later: the maintainer deferred it on 2026-10-02 until after R1 or the public
  announcements ([issue #246](https://github.com/smormah/vsift/issues/246)); until then the
  workflow publishes directly, behind the `release` environment's approval.
- GitHub's release immutability (Settings, General, Releases): compatible with the
  draft-then-publish flow above.
- Whether the Release workflow becomes a required check (main's required checks today are
  Quality, Documentation, dependency policy and review, Rust analysis and Governance). A
  pull request that changes `release.yml` or `tools/vsift-release/` runs it, and its plan
  job is where a change to the stable path shows its dry run; making it required would
  also block a pull request on an infrastructure failure, so it stays optional until you
  decide.
- Settled (2026-10-01): the generated release notes (rendered by
  `tools/vsift-release/src/notes.rs` from the Markdown templates in
  `tools/vsift-release/notes/` since P14 PR 8; `publish.rs` before that) no longer say that installing through npm avoids the
  warnings outright. They say that files installed through npm do not carry the download
  mark that triggers SmartScreen and Gatekeeper, and that Windows Smart App Control, where
  it is on, can still block an unsigned program however it was installed, with a pointer
  to the installation guide ([L-098](../planning/known-limits.md#l-098), `install.md`
  section 4). The paragraph is the same in the candidate's, the pre-release's and the
  stable release's notes. Since P14 PR 8 the templates and this runbook are scanned by the
  public-claims check (`docs/planning/public-claims.json`), and the notes name the three
  machines as the R0 targets the executables are built for rather than with the word the
  claims ladder reserves for the release matrix
  (L-102, closed 2026-10-02): a wording change is a change to a scanned
  document.
- Settled (2026-10-02, ADR 0024 decisions A, B and C): R0 ships as `0.2.0` on `latest`,
  after a published release candidate `0.2.0-rc.N` under `next`, and without signing
  unless the try-outs show a block with no way through. Not decided: whether `next` should
  follow the stable release (6.7: it does not; it is yours to move).

### 6.7 The stable release (P14 PR 8; first used for `0.2.0`, P14 PR 12)

**This path ran against the real services for the first time on 2026-10-09, for `0.2.0`
(section 6.13; known limit [L-105](../planning/known-limits.md#l-105)).** It is built so that a mistake stops before
anything is published, and so that the one thing the workflow cannot take back, moving
`latest`, needs a published and accepted release candidate, a plan that passed every guard,
a dispatch on the tag, and your approval. It is the same workflow, dispatch, environment
and approval as 6.3; what differs is the channel (`--tag latest`, the release marked
latest), the guards, and what you check before and after.

**What makes a version a stable version.** It has no pre-release suffix (section 1): the version in
`Cargo.toml` at the tag decides, and nothing you type does. A stable version is published
only from a tag `v<version>` that equals that version, only by a dispatch with `dry_run`
cleared, only in `smormah/vsift`, only after you approve the `release` environment.

**The guards.** Every stable plan lists them, in this order (the plan's "Guards" table):

| Guard | What it checks | Held by |
| --- | --- | --- |
| Stable version | the version has no suffix, so it takes the channel `stable` and the dist-tag `latest`; never `next` | `publish.rs` (`Channel`, tests of every version kind); the lint's channel rules (the step's `if`, its shape check) |
| Accepted candidate | the highest `v<X.Y.Z>-rc.<N>` tag is an ancestor of this commit and differs from it only in version strings, the launcher's README, the installation guide and the work record (6.8) | `candidate.rs`; the plan job's full-history checkout (lint) |
| Registry read | npm answered for all four packages (a missing package or an unreadable answer fails it) | `registry.rs`; the registry step and `--registry` (lint) |
| Candidate published | that candidate version is on npm for all four packages | `guards.rs` |
| `latest` moves forward | on every package `latest` is now a stable version below this one (or already this version with the same bytes: a re-run completing a partial publish); this version is on none of them under another tag or with other bytes | `guards.rs`; the publish job's own check just before publishing, on the plan's values and on `latest` read again then, which must still be the plan's value or this version (lint: `sort -V` line and the fresh read) |
| Evidence ledger | the ledger is complete for the candidate (`vsift-governance release-evidence --complete-for <candidate> --commit <candidate commit>`, RQ-20): the plan shows its first lines, and an answer that is missing counts as a failure | `evidence.rs`, `guards.rs`; the plan job's candidate and evidence steps and the plan's `--evidence` argument (lint) |

A failed guard refuses an *enforced* plan (a publish, or a dispatch on the tag with
`dry_run` set); any other run shows it and carries on (section 6.1).

**What the plan shows** (fixture values from the tool's tests; yours has the real versions,
commit and digests). The part to read before approving, for a stable plan that passes:

```text
> This publication moves npm's `latest` dist-tag on all four packages. Afterwards `npm
> install vsift-cli` installs `0.2.0`. A published version cannot be unpublished: ...

| Package | Moves `latest` from | to | `next` stays |
| `@vsift/darwin-arm64` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |
| `@vsift/win32-x64` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |
| `@vsift/linux-x64` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |
| `vsift-cli` | `0.0.0` | `0.2.0` | `0.2.0-rc.1` |

| Guard | Result | What was seen |
| Stable version | passed | `0.2.0` has no pre-release suffix, so it is published under `latest` ... |
| Accepted candidate | passed | against `v0.2.0-rc.1` (commit `aaaaaaaaaaaa`): 1 version-string, 2 shipped-document and 6 work-record files differ, nothing else |
| Registry read | passed | npm's public metadata of all 4 packages was read |
| Candidate published | passed | `0.2.0-rc.1` is published on all four packages |
| `latest` moves forward | passed | `latest` is `0.0.0` on all four packages, a stable version below `0.2.0`, and this version is on none of them |
| Evidence ledger | passed | VSift release evidence is complete for 0.2.0-rc.1 at aaaaaaaaaaaa. |
```

An enforced stable plan that passes also writes `release-delta.json` into the plan artifact:
the comparison of the candidate with the stable release, in the shape of the evidence
ledger's `release_delta` record (the versions and commits of both, the verdict, this run's id and the date). Copy it
into `docs/planning/p14-evidence-ledger.json` after the publish (the follow-up, P14 PR 13);
nothing copies it for you ([L-103](../planning/known-limits.md#l-103)). `P14 verify release`
reads the same file from the run's artifact (6.4), but only while GitHub keeps it, seven days,
so the copy in the ledger is the only record after that.

A plan that fails a guard starts with `## Publish plan: REFUSED, nothing is published`, then
"Refused because:" and one line per failed guard, for example:

```text
- Accepted candidate: `crates/vsift-cli/src/main.rs` may not differ between the candidate and the stable release: only version strings, the launcher's README, the installation guide and the work record may
- `latest` moves forward: `0.1.0` is already on npm, and not with these bytes (on all four packages)
```

On a pull request the same findings appear under "A real publication of this plan would be
refused, because:" and the run carries on.

**Preflight, before the dry run.** The list for every real dispatch of a stable version; do
not skip an item because the candidate's publish went well.

1. **Trusted publishers are still saved** on all four packages (6.2 step 6: npmjs.com, each
   package's Settings, a saved Trusted Publisher entry listed with the values of the table,
   **npm publish** allowed, **npm dist-tag** not). `--tag latest` is part of `npm publish`; it
   went through trusted publishing for `0.2.0` (6.13), so these entries are known to accept
   it, but they stay the one setting a stable publish depends on that no dry run can read
   ([L-100](../planning/known-limits.md#l-100), [L-105](../planning/known-limits.md#l-105)).
2. **The `release` environment, the tag ruleset and (since 2026-10-17) the release-branch ruleset**
   are as 6.2 steps 3, 4 and 8 say (read-only `gh api repos/smormah/vsift/environments/release` and
   `.../rulesets`), and fork-pull-request approval is "all external contributors".
3. **What npm holds now**, from a machine with no npm credentials: `npm view vsift-cli
   dist-tags` and the same for `@vsift/win32-x64`, `@vsift/darwin-arm64` and
   `@vsift/linux-x64` show `latest: '0.0.0'` (or the previous stable version) and `next` at the
   candidate; `npm view <package> versions` lists the candidate and not the stable version.
4. **The candidate is the one you mean.** `git tag --list 'v<X.Y.Z>-rc.*'` (after `git
   fetch --tags`) ends with the candidate you accepted: the highest number is the accepted
   one, and a later candidate you decided not to accept must be dealt with before (it
   cannot be deleted without bypassing the ruleset).
5. **The delta check, locally, at the stable commit** (6.8): `cargo run --locked -p
   vsift-release -- candidate-delta`. Every file it lists is a version string, a shipped
   document or a work record (6.8), and nothing is marked REFUSED. **Then check it by hand,
   because the check is compiled from the commit it judges** (the plan job and this command
   build `vsift-release` from the checkout under test, so a stable commit that edited
   `candidate.rs` would pass its own edit):

   ```console
   git diff --stat v0.2.0-rc.3 <the stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml
   ```

   Name **the last candidate's tag**, the highest `v0.2.0-rc.<N>`: `git tag --list 'v0.2.0-rc.*'`
   shows them. With the first three tagged it is `v0.2.0-rc.3`; a diff against `v0.2.0-rc.2` would
   show the third candidate's own changes (the skill, the short-audio floor, the copy's remediation,
   the tests and the campaign tool) and prove nothing, and a diff against `v0.2.0-rc.1` the second's
   two fixes as well. **Until `v0.2.0-rc.3` exists the highest tag is `v0.2.0-rc.2`:** this command
   and the check are for the stable commit, which comes after the third candidate is tagged.

   It must list **only** `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`,
   `npm/vsift-cli/package.json` and `npm/vsift-cli/README.md`, each with a few changed lines
   (read the version-string files' diff: only the version text). **Any line under `crates/`,
   `tools/`, `.github/`, `skills/`, `schemas/`, `fixtures/` or any other file of `npm/` means
   the check was bypassed or wrong: do not publish.**
6. **The evidence ledger** is complete for the candidate. The plan job runs P14 PR 1's check
   (`vsift-governance release-evidence --complete-for <candidate> --commit <candidate
   commit>`) and the plan refuses the run if it fails, so a pass is a precondition rather
   than something to remember; still read the check's output in the plan summary and the
   waivers it records, and confirm that the maintainer's try-outs have a recorded
   observation and that the hosted qualification (the published-artifact workflows)
   passed on the candidate: the check says the ledger's records are complete, not that
   the evidence is good enough to ship ([L-103](../planning/known-limits.md#l-103)).
7. **What ships is what you want to ship.** Read the launcher's `README.md` and the release
   notes the dry run produces (`publish-plan/release-notes.md` in the run's `publish-plan`
   artifact): both reach the public at the moment `latest` moves, and the notes are code,
   frozen at the candidate. Read `install.md` too: it is the guide users will follow.

**The procedure.**

1. **Tag** the stable commit (the merge commit of P14 PR 12) and push the tag, as in 6.3
   step 2 with `v0.2.0`.
2. **Dry run on the tag** (6.3 step 3 with the stable tag, **dry_run** ticked). It is
   *enforced*: it fails whenever the real run would. Read the **Publish plan** summary: the
   first lines must say "A real publication of this plan would move npm's `latest`" (a dry
   run) and the table "What this publication does to the dist-tags" must show, for each of
   the four packages, `latest` from `0.0.0` to `0.2.0` and `next` staying at the candidate;
   every guard must be "passed", the evidence ledger's included. If it says
   **REFUSED**, read "Refused because:", fix the cause (6.5), and repeat.
3. **Publish.** Dispatch again on the same tag with **dry_run cleared**. The plan must say
   **PUBLISH** and name the stable release (in capitals), `attest` runs and `publish` waits for the environment.
   Read the summary one last time (the dist-tag table; the four `npm publish ... --tag
   latest` commands in order; `gh release edit ... --latest`), then **Review deployments**,
   tick `release`, **Approve and deploy**. Approve within a week (6.3 step 4).
4. **What the `publish` job does** (and where it stops): it records the dist-tags; checks
   the version's shape and that every `latest` is a stable version at or below this one;
   publishes the three platform packages and then `vsift-cli` with `--tag latest`; waits up to
   20 minutes (since 2026-10-17; five before) for `latest` to read back as this version on all
   four packages with `next` unchanged, and fails at once on any other answer (6.4); creates the GitHub release as a draft, publishes it marked
   latest, and requires GitHub's own latest release to be this tag.

**After the publish** (the checks of 6.4 differ in four places):

1. `npm view vsift-cli dist-tags` (and the three platform packages) shows `latest:
   '0.2.0'` and `next` unchanged; npmjs.com shows each version as built and signed on
   GitHub Actions.
2. On a machine with no npm credentials, **without a tag**: `npm install vsift-cli` in an
   empty project installs `0.2.0` (`npx vsift --version` prints `vsift 0.2.0 (<the tag's
   first 12 commit digits>)`), and `npm audit signatures` verifies registry signatures and
   attestations.
3. The GitHub release is **not** a pre-release and is GitHub's latest: `gh release view
   v0.2.0 --repo smormah/vsift --json isPrerelease,isDraft` says false and false, and `gh
   api repos/smormah/vsift/releases/latest --jq .tag_name` prints `v0.2.0`.
4. The checksums and `gh attestation verify` of the ten files and four tarballs (6.4 with
   `--source-ref refs/tags/v0.2.0`).
5. Re-run the hosted qualification on the stable release's bytes (ADR 0024 decision A) and put the
   run's link, the commands' output and anything that failed or was re-run in the evidence
   ledger, with the plan's `release-delta.json` as the `release_delta` of the carried items
   (download the run's `publish-plan` artifact within its 7 days). `P14 verify release` for the
   stable version, dispatched from `main` within those seven days, must be all green, and its
   delta check and its `latest` check (6.4) must be among the passes.

**What changes for users.** From the moment the four packages are published, `npm install
vsift-cli`, `npx vsift-cli`, `pnpm dlx vsift-cli`, `bunx vsift-cli` and `yarn dlx` without a
tag install the stable version; before, they installed the empty `0.0.0` placeholder, a
package with no command. Yarn 4 holds every new version back for a day
(`npmMinimalAgeGate`), so Yarn users of `latest` get it a day after the publish; the
workaround is the `npmPreapprovedPackages` setting of `install.md`, and the release notes
say so. `next` is not touched, so it keeps naming the candidate, an older build than the
stable release, until the next pre-release moves it; moving it yourself (`npm dist-tag add
vsift-cli@0.2.0 next` for each package, with two-factor authentication) is your choice, and
the workflow never does it ([L-108](../planning/known-limits.md#l-108)).

**Never,** whatever goes wrong: unpublish a version, move or delete a `v*` tag, or delete the
GitHub release (6.5 says what to do instead).

### 6.8 The candidate rule, the release branch and the candidate-to-stable-version check (P14 PR 8; the lists settled in PR 10b; the release branch since 2026-10-17)

**The rule** (ADR 0024 decision B, amended by its note of 2026-10-17). **Each release's candidates
and the stable release are made on its line's release branch, `release/<MAJOR>.<MINOR>.x`** (`release/0.2.x`
for `0.2.2`, `0.2.3` and every other `0.2` release), which the weekly cut creates from a commit of `main`
whose nightly passed (6.18). From the cut of a release candidate only fixes for findings may change the
code **on the release branch**: no features, no refactors, no unrelated cleanup. A fix that changes code
means a new candidate (`rc.2`, cut on 2026-10-06 for two findings of the first: 6.11; `rc.3`, cut on
2026-10-08 for a failed agent-trial gate of the second, which a change to the skill answers: 6.12), made
on the release branch from a commit whose own nightly passed, and the qualification of what it touches;
the stable release is built on the last candidate, from the release branch. **`main` is not frozen:** it
keeps taking merges throughout the window, because nothing merged to it reaches the release branch
except by a pull request to that branch. (Until 2026-10-17 the candidates were tagged on `main`, and
`main` itself was frozen from a candidate's tag to the stable release; 6.10 to 6.17 are records of that
way.) **The check** (decision A) is the mechanical form: the stable commit must differ from its accepted
candidate only in the stable release's own version strings, the documents the release publishes and the
work record. The release branch does not change the check: both commits are on the branch, and the
candidate is an ancestor of the stable commit there.

- **The accepted candidate** is the highest-numbered `v<X.Y.Z>-rc.<N>` tag for the stable
  release's own `X.Y.Z`; any other tag is ignored. Only you can create a `v*` tag (the tag ruleset).
  **With `v0.2.0-rc.1`, `v0.2.0-rc.2` and `v0.2.0-rc.3` tagged it is `v0.2.0-rc.3`:** the stable
  release is compared with the third candidate and the earlier two are never looked at (a test builds
  two candidates one fix apart and requires the highest to be the one compared:
  `the_highest_candidate_is_the_accepted_one`; the choice is by number, so it holds for three as for
  two). The earlier candidates stay published and
  tagged and take no part in the check. Until the maintainer creates `v0.2.0-rc.3` the highest tag is
  `v0.2.0-rc.2`, which matters to nobody: the check runs only for a stable version. Since 2026-10-17
  every candidate tag and the stable tag are on the release branch: the `guard` job refuses a
  dispatch of a tag whose commit the branch does not contain (6.18).
- **Version-string files** (the stable release's content must equal the candidate's with *every*
  occurrence of the candidate's version text replaced by the stable release's, byte for byte):
  `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`,
  `npm/vsift-cli/package.json`. A listed file you leave unchanged is not a difference. Why:
  the stable release's own version is the one thing it differs in by definition, and a
  manifest or lockfile that changes anything else (a dependency, a feature) is a code change.
- **Shipped documents** (any change allowed, as an edit of an ordinary file): the launcher
  package's README, `npm/vsift-cli/README.md`, which ships in the npm package and whose install
  instructions differ between a candidate and the stable release (allowed since PR 8); and the
  installation guide, `docs/operations/install.md`, which the release notes link to **at the
  release's own tag** (`blob/<tag>/docs/operations/install.md`), so the guide at the stable
  release's tag must tell a reader to install `vsift-cli`, not `vsift-cli@next` (added in PR 10b).
  Both are scanned by the claims check, so neither can say more than the claims rung allows. The
  skill (`skills/vsift/`) is *not* in the list: its bytes are what the named-client trials ran
  against, and a change after the candidate would ship a skill they never saw. The release notes
  and the platform packages' README are generated from this repository's code, which is frozen
  at the candidate too.
- **The work record** (an edit of an ordinary file, or the addition of one; never a deletion):
  `CHANGELOG.md`, and every file below `memory/`, `docs/decisions/`, `docs/history/`,
  `docs/planning/` and `docs/guide/`, **except** three things, which stay refused:
  `docs/planning/delivery-ledger.json` (it fixes the packet's objective and changes only in the
  completion follow-up, P14 PR 13), `docs/guide/reference/` (the two generated reference pages,
  which the `Guide` workflow regenerates from the binary and the schemas and compares) and
  `docs/guide/files/` (the practice files its examples run on). Why: none of it ships in an
  archive or a package, no build and no test of the program reads it as input, and the
  repository's own rules change it in every pull request (the changelog and the two handoff
  files in the same change; a finding adds a known limit; PR 11 records the candidate's evidence
  in `docs/planning/p14-evidence-ledger.json`, which the stable plan's evidence guard reads **at
  the stable commit**, so the stable release could not be built without it). `CHANGELOG.md` was
  a version-string file until PR 10b; it moved here because every pull request adds to it.
  **The candidate's evidence is therefore recorded on the release branch** (a records pull request
  to the branch), since the stable commit is made there.
- **Everything else is refused**, and so is a listed path that is added (a version-string
  file or a shipped document), deleted, linked, re-moded or renamed rather than edited. So are a
  candidate that is not an ancestor of the stable commit and a stable commit that is the
  candidate's own. In particular, **from the moment the candidate's tag exists until the stable
  release is published, nothing may be merged into the release branch that touches the code, a
  dependency or a workflow:** no Dependabot pull request (they change `Cargo.toml`, `Cargo.lock` or
  a workflow), no change to a qualification tool or workflow, to the README, to `SECURITY.md` or to
  any other operator document. If one is needed, the cure is another candidate (as `rc.2` and `rc.3`
  were) with the change, not a longer list. All of these may merge to `main` at any time; they reach
  a release branch at the next cut. Every other repository page that names the release is corrected
  after the stable release is published, in the records change on `main` (as P14 PR 13 and the records
  of 2026-10-16 did), not in the stable commit.
- **The release branch** (since 2026-10-17; the steps are 6.18):
  - **It is created by the cut, by the maintainer,** at a commit of `main` that already carries the
    first candidate's version and whose latest Nightly run passed; that commit is tagged `v<X.Y.Z>-rc.1`.
    If a previous window's branch of the same line is still there, the stable release of that window is
    published and its records are on `main`, it is deleted first: the tags keep every commit of it.
  - **It takes changes only by pull request** (the branch ruleset of 6.2 step 8): a window fix, the
    version bump of the next candidate or of the stable release, the candidate's evidence and other
    work record, and the shipped documents of the stable commit. Its pull requests run `CI` and the
    Release dry run (both workflows trigger on `release/**`).
  - **A window fix is brought to `main` too,** by its own pull request against `main` with the same
    change, so `main` never lacks a fix that shipped. **The `Release window` workflow checks it**
    (`python3 tools/release_window.py`, since 2026-10-17; 6.18 step 4 has when it runs): for every commit
    of a release branch since its cut it asks whether the change is on `main`, **by content and not by
    commit**, because with squash merges nothing on the branch is an ancestor of `main`, the counterpart is
    another pull request with another number, and a patch-id fails as soon as `main` has moved on. For each
    file the commit changed, every significant line it added must be in `main`'s file, every significant
    line it removed must be gone from it, a deleted file must be gone and a changed binary file identical;
    a version string is compared without its version. Run by hand:
    `git fetch origin && python3 tools/release_window.py [--records]` (it reads `origin/main` and every
    `origin/release/*`; `--main <ref>` and `--release <ref>` choose others). It exits 1 and names the
    commit, the file and the lines when a fix is missing; **no ruleset requires it**, so a deliberate
    difference is recorded in the pull request that finds it. The release branch's work record reaches `main` in
    the records change after the publish, **and so do the stable commit's shipped documents**
    (`docs/operations/install.md` and `npm/vsift-cli/README.md`, which tell a reader to install
    `vsift-cli` rather than `vsift-cli@next`): they are made on the release branch, and without that
    change `main`'s installation guide would keep the candidate's wording. Its version strings do not
    need to: the next cut's bump sets `main`'s version.
  - **What the check does not compare line by line (#458, L-173; since 2026-10-18)**, because the records
    change rewrites the work record as current state instead of copying it, and each of these is named in
    the report as "excused": (1) **a line a window commit added that the branch's own head no longer has**
    (a later window commit replaced it; that commit is compared on its own, so the branch's net change is
    still held to `main`); (2) **`memory/TODO.md` and `memory/project_current_status.md`**, which every
    change rewrites, when `main`'s last change to the file is not older than the release commit (a
    newer rewrite; if `main` has not rewritten them since, they are compared line by line); (3) **in
    `CHANGELOG.md` and the pages directly in `docs/planning/` (the register among them), a line the
    release commit replaced in place when `main` no longer has the line it replaced, and a new line
    whose first 60 characters `main` has in a line the cut did not have (the same sentence, reworded)**:
    a date, a count, a status or a "not published" sentence that both sides rewrote is not reported, a new
    register entry or bullet that `main` lacks is, and so is a replaced line that `main` kept as the cut had
    it. Code, configuration, version files, the two shipped documents, the ledger and every other file are
    compared exactly as before, and the first excuse is the only one that reaches them. None of the three
    can hide a missing code fix: a fix's code is in files none of them applies to (or in a commit
    whose later replacement is itself compared), and a fix's own record, its changelog bullet and its
    register entry, stay compared.
  - **A commit made on the release branch has no scheduled nightly** (the schedule runs on `main`):
    before such a commit is tagged as a candidate, the Nightly workflow is dispatched on the branch
    (`gh workflow run nightly.yml --repo smormah/vsift --ref release/<MAJOR>.<MINOR>.x`) and must pass on
    the branch's head; the `guard` job refuses the candidate otherwise. The stable commit needs no
    nightly of its own: it differs from the accepted candidate only as this section allows.
  - **A hot fix of a published stable version** re-creates the branch from that version's tag if it was
    deleted (`git push origin v<X.Y.Z>^{commit}:refs/heads/release/<MAJOR>.<MINOR>.x`); the fix, its
    candidate and the stable release then follow the same rules. **For a version tagged before
    2026-10-17** (`0.2.1` and earlier) the branch made from its tag has the workflows of then: a `ci.yml`
    that does not run on `release/**` (so `CI result` never reports and the ruleset blocks every pull
    request), no `nightly.yml` (so no nightly can be dispatched there) and a `release.yml` without the
    guard. **The first pull request to such a branch therefore brings the current `ci.yml`,
    `nightly.yml` and the workflows it calls, `release.yml`, and the tools they run (`tools/ci_lanes.py`,
    `tools/vsift-release/`, `tools/vsift-governance/`) from `main`**, before any fix and before the first
    candidate's tag. That is a stated exception to the rule that the release branch takes no workflow
    change: the rule binds from a candidate's tag, and this change comes before it, so the candidate
    and the stable release are both made with the current workflows; its diff is read like any other
    workflow change.
- The lists are constants in `tools/vsift-release/src/candidate.rs`, and a test spells them
  out and a second test reads this section and requires every entry to be named in it, so
  changing one is visible in a diff and in the documentation. A third test applies the check to
  one path of every area that must stay refused (the crates and the managed catalogue, the
  schemas, the fixtures, the skill, the trial harness, the scenarios, the settings, the
  release tool, every other tool, every workflow, the launcher's code, the other documents, the
  siblings of a record directory) and to every allowed kind, and a fourth breaks a copy of the
  lists in many ways (a record directory that covers `crates/`, a missing trailing slash, an
  emptied protected list, the skill added as a shipped document) and requires the test to
  notice. They are code, and code is frozen at the cut on the release branch: change them on `main`
  before the cut, or cut another candidate.
- Run it by hand with `cargo run --locked -p vsift-release -- candidate-delta
  [--stable-commit <sha>]` from a clone that has the tags (`git fetch --tags`): it prints
  each changed path with its verdict and fails if any is refused. It compares the working
  tree's version (the tool's own workspace version) with its candidate, so run it at the
  stable commit. The plan job does the same comparison inside `publish-plan` for every stable
  version, at the commit it is running on.
- **What it proves, and what it does not** ([L-107](../planning/known-limits.md#l-107)): it
  compares paths and bytes, not meaning. It cannot say that the launcher's README or the
  installation guide is right, that a work record is true, that the candidate's evidence was
  complete, or that a version string was the only thing a change meant; it takes the highest
  candidate to be the accepted one. Nor does the release branch prove that a window fix reached
  `main`: that is a pull request someone opens, and the `Release window` check above reports its absence when it is
  run, by lines and not by meaning ([L-173](../planning/known-limits.md#l-173)).

### 6.9 Testing the publishing shell (P14 PR 8)

The privileged `publish` job is a few dozen lines of shell, so that no Rust is compiled
where an OIDC token exists. They are tested by running them. `tools/vsift-release/tests/
publish-steps.sh` extracts each publishing step's script from `release.yml` itself, and the
plan job's registry step, and runs them with stub `npm`, `gh`, `curl` and `sleep` commands
that keep their state in a temporary folder. It checks, among other things, that a
pre-release moves `next` and leaves `latest`; that a stable version moves `latest` on all
four packages, leaves `next`, publishes with `--tag latest` four times and never with `--tag
next`; that each step refuses the other channel's version before anything is published;
that `latest` ahead of the version, a pre-release as `latest` or no `latest` refuses the
stable version (and that `0.9.0` is below `0.10.0`); that a re-run after a partial stable publish
publishes only what is missing; that other bytes for a published version stop the publish;
that a stable version which moved `next` fails its verification; that the stable release must be
GitHub's latest; that the registry step records one status per package and never fails
the job, whatever curl does; and that the candidate step names a candidate only when there is
one and the evidence step records the check's exit status and output without failing the job,
whatever the check answers (56 checks until 2026-10-17). **Since 2026-10-17 it also runs the
`guard` job's script** (with a stub `gh` that answers the Nightly runs, their jobs and the
comparison with the release branch from fixtures, applying the script's own `--jq` filters when
`jq` is installed, as on GitHub's Ubuntu runners): every ref each mode may and may not use, a
publish on `main` refused without a single API read (#337), a candidate refused off the release
branch, with no nightly, after a failed nightly, after a failed nightly followed by a run that ran
nothing, with a skipped check, with only a push-triggered run, or when GitHub cannot be read, and
accepted after a passed scheduled or dispatched run; a stable version not asked for a nightly; and
**the read-back** (#403): a package whose tag reads its old value for twelve minutes passes, a
registry that never settles fails after exactly 40 reads 30 seconds apart with a message that the
packages were published, and a wrong value of either tag fails at once, without a single wait
After the independent review of the same day it also checks the tag (moved, annotated and peeled,
unreadable), the comparison with the branch head's own commit (the stub answers only reads of the
exact commit, branch head, tag and query, so a read of the branch by name or of another commit
fails), a Nightly run still in progress, the branch's rule types, the publish job's re-check of all
of it after the approval, the "before" values taken from the plan's output (a re-run that reads a
stale old tag waits; a missing, unreadable or reordered package stops the publish), a read that hangs
(cut off after 20 seconds), a candidate read-back that never moves or never answers, and (after the
re-review) a lower stable version refused when a higher one moved `latest` during the wait (181 checks
in all; without `jq` the filters themselves are not run, and the script says so). Run it with `bash tools/vsift-release/tests/publish-steps.sh
.github/workflows/release.yml` (Linux, or Git Bash on Windows); the Rust test
`publish_steps` runs it on Linux in CI. No real service is contacted. It cannot prove that
npm's trusted publishing accepts `--tag latest`, or that `gh release edit --latest` marks a
draft latest on GitHub: those were what the first stable publish tested, and both held
(section 6.13, [L-105](../planning/known-limits.md#l-105)).

### 6.10 The first release candidate, `0.2.0-rc.1`: the exact steps (P14 PR 10c)

**A record (2026-10-10).** Published on 2026-10-05, when `latest` was the empty `0.0.0` placeholder; kept as what was done. For a candidate published now, with `0.2.0` as `latest`, follow 6.16.

**This is the record of the first candidate, which the maintainer published on 2026-10-05
(tag `v0.2.0-rc.1` at `d5792ce31db1106934233c86d0518c3ad1961e07`; the dry run was run 37299364837 and the
publish run 37323324151). The second candidate is 6.11 and the third is 6.12, which repeat these steps
with their own numbers; read 6.12 to publish.** It is kept as written because 6.11 says what differs from it.

For the person doing it once, in order. It is 6.3 and 6.4 with the real commands, the output to expect and what to do when it
differs. It takes about an hour, most of it waiting for builds and the registry. **A candidate is never announced** and moves
`next` only: `latest` stays the empty `0.0.0` placeholder on all four packages at every step. Times are the maintainer's.

**What you are about to do cannot be undone.** A version published to npm can never be published again, even if it is removed,
and a tag may be moved (you can bypass the ruleset) only until anything is published from it. If a step below fails before
`npm publish` has run, nothing is lost and you can repeat it, with one thing to know: **the `attest` job runs before the
`release` environment's approval and creates public, permanent Sigstore attestations** of the run's files. Repeating is harmless
(the builds are reproducible, so a second attestation names the same bytes, as on 2026-10-01: 6.5), but they exist from the
moment `attest` finishes. After `npm publish`, only the "If something fails" list applies. The commands
are written for Git Bash (or any POSIX shell); `gh` and `npm` commands work in PowerShell too, and `^{commit}` needs quotes there.

**0. Before you start (about fifteen minutes; only the settings step changes anything).**

1. **Merge pull request #308 (the cut and this runbook) only when you can tag and publish at once.** On merge the README, the
   installation guide and the launcher's README say a release candidate is under qualification, which is true only once it is
   published (known limit [L-133](../planning/known-limits.md#l-133)); steps 0 to 4 fit in about an hour. Merge nothing else:
   from the moment the tag exists until the stable release is published, **nothing may change on `main` but the work record, the
   installation guide and the launcher's README** (6.8 lists them exactly): no Dependabot pull request, no workflow edit, no
   dependency bump, no change to a tool or to any operator document. The stable release is refused otherwise, and the cure is a
   second candidate. Dependabot opens new pull requests on Mondays: leave them open.
2. **Turn the "Require branches to be up to date before merging" rule on `main` back on before you tag** (it was turned off to
   speed merging): Settings, Branches, the rule for `main`, or `gh api -X PATCH
   repos/smormah/vsift/branches/main/protection/required_status_checks -F strict=true` (administrator rights). Check it, read-only:
   `gh api repos/smormah/vsift/branches/main/protection --jq .required_status_checks.strict` prints `true`.
3. `main` is at the commit you mean to release: pull requests 10a and 10b are merged and nothing is open that you expect to merge.

   ```console
   git fetch origin --tags
   git switch main
   git pull --ff-only
   git log -1 --format="%H %s"
   grep -m1 "^version" Cargo.toml
   ```

   Expect the 10b merge commit and `version = "0.2.0-rc.1"`. Write the full 40-digit commit down: it is what you tag.
4. **The commit you tag is on `main` and every check of it passed.** The Release workflow builds and plans but runs no tests of its
   own, and no guard checks that a tagged commit was merged or tested, so you check it:

   ```console
   git merge-base --is-ancestor <the 40-digit commit> origin/main && echo "on main"
   gh api "repos/smormah/vsift/commits/<the 40-digit commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   Expect `on main`, and **no output at all** from the second command: every check run of that commit is completed and succeeded
   (or was skipped on purpose: `Attest build provenance`, `Publish to npm ...` and the fuzz matrix show as skipped on a pull
   request). Each line it prints is a check that failed or has not finished (for example `Quality (windows-latest): in_progress
   null`): wait, or do not tag. The same command on the 10a merge commit printed nothing on 2026-10-05, with 36 check runs.
5. The checks that bind this commit pass on your machine (about a minute each, after the first build):

   ```console
   cargo run --locked -p vsift-governance -- check
   cargo run --locked -p vsift-governance -- public-claims
   cargo run --locked -p vsift-governance -- release-evidence
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-2/freeze.json
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-3/freeze.json
   ```

   Expect "VSift delivery ledger is valid.", "VSift public claims agree with the evidence ledger (this proves recorded evidence
   and absent banned words, not that a sentence is true).", "VSift release evidence ledger is valid." and `nothing frozen has
   changed` twice. Do **not** run `release-evidence --complete-for 0.2.0-rc.1` now: it fails on purpose, because every evidence
   item is still stale for the candidate (the qualification of PR 11 records it). Any other answer: stop and tell the
   supervisor; do not tag.
6. The trusted publishers are still saved on npmjs.com for all four packages (6.2 step 6: a saved entry listed, owner `smormah`,
   repository `vsift`, workflow `release.yml`, environment `release`, **npm publish** allowed). This is the one setting no dry
   run can check (L-100); `--tag next` has gone through it for 0.1.0, so it is expected to hold.
7. The `release` environment and the tag ruleset exist as 6.2 steps 3 and 4 say (read-only):

   ```console
   gh api repos/smormah/vsift/environments/release --jq '{reviewers: [.protection_rules[]? | .reviewers[]? | .type], branch_policy: .deployment_branch_policy, can_admins_bypass: .can_admins_bypass}'
   gh api repos/smormah/vsift/rulesets --jq '.[] | {name, target, enforcement}'
   ```

   Expect `{"branch_policy":{"custom_branch_policies":true,"protected_branches":false},"can_admins_bypass":false,"reviewers":["User"]}`
   (a required reviewer, administrators cannot bypass, custom policies for tags only) and `{"enforcement":"active","name":"release
   tags","target":"tag"}`. These were the answers on 2026-10-05.
8. What npm holds now, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`):

   ```console
   npm view vsift-cli dist-tags
   npm view @vsift/win32-x64 dist-tags
   npm view @vsift/darwin-arm64 dist-tags
   npm view @vsift/linux-x64 dist-tags
   ```

   Expect `{ latest: '0.0.0', next: '0.1.0' }` four times. Remember it: step 4 compares.

**1. Tag the commit** (you are the only person the ruleset lets create a `v*` tag):

```console
git tag -a v0.2.0-rc.1 -m "VSift 0.2.0-rc.1" <the 40-digit commit>
git push origin v0.2.0-rc.1
git show --no-patch --format="%H" v0.2.0-rc.1^{commit}
```

The last line prints the commit you wrote down. A wrong tag **before anything is published** may be moved (you can bypass the
ruleset): delete it with `git push origin :refs/tags/v0.2.0-rc.1`, fix, push again. **Never move or delete it after step 3.**

**2. Dry run on the tag.** Actions, **Release**, **Run workflow**, "Use workflow from" **Tags: v0.2.0-rc.1**, leave **dry_run**
ticked, **Run workflow**. Or:

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.0-rc.1 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --event workflow_dispatch --limit 3 --json databaseId,headBranch,headSha,status,conclusion
gh run watch <the run id> --repo smormah/vsift
```

Pick the run by what it says, not by position: its `headBranch` must be `v0.2.0-rc.1` and its `headSha` the commit you wrote
down (the newest run listed can be a pull request's or another dispatch). It takes about 30 to 40 minutes: 21 jobs, `attest`
and `publish` skipped. Open the run's **Publish plan** summary and read it
top to bottom. Expect:

- the heading `Publish plan: dry run, nothing is published` and "This run was dispatched with `dry_run` set";
- `` `latest` is not touched. This release candidate is published under `next` only ``;
- "Version `0.2.0-rc.1` (release candidate), npm dist-tag `next`, Git tag `v0.2.0-rc.1`", the commit you wrote down and "The plan
  is enforced" (a dispatch on the tag is the rehearsal of a publish);
- the table "What this publication does to the dist-tags": for each of the four packages `next` from `0.1.0` to `0.2.0-rc.1`
  and `latest` staying `0.0.0`;
- two guards, both **passed**: **Pre-release version** ("`0.2.0-rc.1` has a pre-release suffix, so it is published under `next` and
  `latest` is not touched") and **Registry** ("this version is not `latest` and is not on npm with other bytes on any package");
  the run of pull request #308 showed exactly this plan, with "report-only" in place of "enforced";
- four `npm publish ... --tag next --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`,
  `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`, and a GitHub release created as a draft **pre-release not marked latest**
  with ten assets.

Download the run's `publish-plan` artifact and read `release-notes.md`: it is what the GitHub release will say ("VSift 0.2.0-rc.1
is a release candidate. It is under qualification, it is not announced, and it is no statement of support or stability ...").
If anything differs, or the run fails, nothing was published: fix it (6.5, "The plan job fails on the tag"), and repeat from
step 1 if the commit has to change.

**3. Publish.** The same dispatch with **dry_run cleared** (untick it; with `gh`: `-f dry_run=false`) on the same tag. The plan
job must say `Publish plan: PUBLISH a release candidate after the release environment's approval`; `attest` runs (about a
minute); `publish` stops at **Waiting for review**. Read the plan once more, then **Review deployments**, tick `release`,
**Approve and deploy**. Approve within a week: the run's files expire after seven days (6.3 step 4).

The `publish` job then publishes the four packages (about three minutes), waits up to five minutes for `next` to read back as
`0.2.0-rc.1` on all four (on 0.1.0 it took three polls, 30 seconds apart), checks that `latest` is where it was, and creates
and publishes the GitHub pre-release. A message from npm that a package "is being processed and may take a few minutes to
become available" is normal.

**4. Check it from the outside** (a shell with no npm login and a folder of your own; about ten minutes). Expect, in this order:

```console
npm view vsift-cli dist-tags
npm view @vsift/win32-x64 dist-tags
npm view @vsift/darwin-arm64 dist-tags
npm view @vsift/linux-x64 dist-tags
```

`{ latest: '0.0.0', next: '0.2.0-rc.1' }` four times. **`latest` must still be `0.0.0` and `next` must be `0.2.0-rc.1`.**
Anything else on `latest` is the emergency of 6.5 ("`latest` moved when it should not have").

```console
mkdir rc-check && cd rc-check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures
npx vsift --version
```

Expect `npm audit signatures` to report that the packages have verified registry signatures and verified attestations (on
Windows 4 and 4; on Linux the two packages it installs), and `vsift 0.2.0-rc.1 (<the first 12 digits of your commit>)`.

```console
gh release view v0.2.0-rc.1 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `isPrerelease` true, `isDraft` false, ten assets; and **`gh: Not Found (HTTP 404)`** from the second command (no release
is marked latest: 0.1.0 is a pre-release too). If it prints `v0.2.0-rc.1`, the release is wrongly marked latest: `gh release edit
v0.2.0-rc.1 --repo smormah/vsift --latest=false`.

The checksums and attestations of the ten files and the four tarballs are 6.4's commands with the version and `--source-ref
refs/tags/v0.2.0-rc.1`; the next step does them again from a runner that holds no credential, so doing both is optional.

**5. The second verification and the hosted qualification** (nothing here can publish; each is one dispatch, run from `main`):

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.1
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.1 -f from_version=0.1.0
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.1
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.0-rc.1
```

`P14 verify release` (RQ-19) takes a few minutes and must be all green: the provenance of four packages, `npm audit
signatures`, `gh attestation verify` of ten files and four tarballs, the checksums, and the release's flags. The others are
the evidence of P14 PR 11 (RQ-01 to RQ-06): their results go into the ledger there, and **a failed run is a finding: open an
issue before re-running** (governance rule 14). You do not have to wait for the other three before you finish step 4.

**6. Afterwards.** Tell the supervisor: the run ids of the dry run and the publish, the four `npm view` answers, the output of
`npm audit signatures`, and anything that failed or was re-run. Nothing is announced. **Batches 2 and 3 of the agent trials
start only on your explicit go** (`docs/agents/trials.md`, "The P14 batches"); their freeze is already committed
(`docs/planning/p14-agent-trials/batch-2/freeze.json` and `batch-3/`) and `prepare` refuses a trial if the skill, grader,
scenarios or settings differ from it.

**If something fails.**

| When | What it means | What to do |
| --- | --- | --- |
| Step 0 check fails (a failing check run, not on `main`, a governance or freeze answer) | the commit is not what was reviewed | do not tag; tell the supervisor |
| Step 2 fails or is refused | nothing was attested or published | read "Refused because:", fix it, move the tag if the commit changes (nothing was published from it) |
| Step 3: `publish` fails with `ENEEDAUTH` before any `+ package@version` line | a trusted publisher is not saved or is wrong | 6.5 first bullet of `ENEEDAUTH`; nothing was published |
| Step 3: `publish` fails after some packages | a partial publish ([L-097](../planning/known-limits.md#l-097)) | **Re-run failed jobs** on the same run within seven days and approve again: versions npm holds with the same bytes are skipped |
| Step 4: `next` is not `0.2.0-rc.1` after ten minutes | the registry is slow, or the publish stopped | look at the `publish` job's log before doing anything |
| Step 4: `latest` is not `0.0.0` | the workflow is wrong | 6.5 "`latest` moved when it should not have": `npm dist-tag add <package>@0.0.0 latest` for each package that moved, then open an issue; **do not dispatch again** |
| A published version is bad | the candidate itself is wrong | **never unpublish** (a version number can never be used again, and the attestations name its bytes): `npm deprecate vsift-cli@0.2.0-rc.1 "<what is wrong; use 0.2.0-rc.2>"` on all four packages, edit the GitHub release's notes to say so, and publish the next candidate by this section again (two findings of the first candidate led to this: the second candidate is 6.11) |

**What is irreversible, in one list.** The four published versions and their provenance records on npm's transparency log; the
Sigstore attestations of the ten files and four tarballs (they exist from the end of `attest`, before the approval); the tag
once anything was published from it. **Policy, not impossibility:** GitHub lets you delete the release page or move the tag,
and this runbook says never (6.5), because the page, the tag and the attestations name each other. **What is reversible:** `next` (`npm dist-tag add vsift-cli@0.1.0 next`, for each package,
two-factor authentication; the workflow never does it), the notes, a deprecation (`npm deprecate <package>@<version> ""` removes
it).

### 6.11 The second release candidate, `0.2.0-rc.2`: the exact steps (P14 PR 10, repeated)

**A record (2026-10-10).** Published on 2026-10-07, when `latest` was the empty `0.0.0` placeholder; kept as what was done. For a candidate published now, with `0.2.0` as `latest`, follow 6.16.

**This is the record of the second candidate, which the maintainer published on 2026-10-07 by these steps (tag `v0.2.0-rc.2` at
`7c722d1fc46af7fddeffbaf807028eaec413ace1`; the packages at 09:43 UTC from publish run 37592020376; `P14 verify release` run
37602927119 passed). The third candidate is 6.12, which repeats these steps with its own numbers; read that one to publish.** It is
kept as written, in the future tense of the day before the publish, because 6.12 says what differs from it.

**Why there is a second candidate.** The hosted campaigns of P14 PR 11 on the first candidate found two defects in the program. On
Windows a reader that overlaps a writer's rename could be told that stored data is damaged ([#314](https://github.com/smormah/vsift/issues/314),
fixed by #318), and a video over the 20 GiB limit answered `STORAGE_IO` where 0.1.0 said `INVALID_SOURCE` (#310, fixed by #319). On
2026-10-06 the maintainer decided on a second candidate: **`0.2.0-rc.1` plus exactly those two fixes and the version bump, and nothing
else** (no skill, grader, scenario, setting, tool or FFmpeg catalogue change, no Dependabot pull request). It is published as 6.10
published the first, and this section is that one with what differs. **`0.2.0-rc.1` stays published and tagged: it is superseded, not
withdrawn,** and no step below touches it. A candidate is never announced and moves `next` only: `latest` stays the empty `0.0.0`
placeholder on all four packages at every step. It takes about an hour, most of it waiting for builds and the registry. Times are the
maintainer's. The commands are written for Git Bash (or any POSIX shell: the `for` loops are shell); the `gh` and `npm` commands work in
PowerShell too, and `^{commit}` needs quotes there.

**What differs from 6.10, in one table.**

| | First candidate (6.10, 2026-10-05) | Second candidate (this section) |
| --- | --- | --- |
| What npm holds before the publish | `next` `0.1.0`, `latest` `0.0.0` | `next` **`0.2.0-rc.1`**, `latest` `0.0.0`, on all four packages |
| What the dry run's plan does to `next` | moves it `0.1.0` to `0.2.0-rc.1` | moves it **`0.2.0-rc.1` to `0.2.0-rc.2`**; `latest` stays `0.0.0` |
| The tag and the commit it names | `v0.2.0-rc.1` at the merge of #308 | `v0.2.0-rc.2` at the merge commit of the pull request titled "P14 PR 10 repeated: the second release candidate `0.2.0-rc.2` is cut" |
| What `vsift --version` prints | `vsift 0.2.0-rc.1 (d5792ce31db1)` | `vsift 0.2.0-rc.2 (<the first 12 digits of that commit>)` |
| The upgrade check (`from_version`) | `0.1.0` | `0.1.0` **and `0.2.0-rc.1`** (the upgrade a person on `next` really does) |
| The candidate the stable release is compared with | none yet | **`v0.2.0-rc.2`**, the highest candidate tag (6.8) |
| `release-evidence --complete-for` | failed on every item | fails on the items the fixes touch until PR 11 is repeated (plan section 25): **do not run it as a gate now** |
| After the publish | nothing | optionally deprecate the first candidate (step 7) |

**0. Before you start (about fifteen minutes; only the settings step can change anything).**

1. **Merge the pull request only when you can tag and publish at once.** On merge the README, the installation guide and the launcher's
   README name `0.2.0-rc.2` as the release candidate now in qualification, which is true only once it is published (known limit
   [L-133](../planning/known-limits.md#l-133)); the installation guide says that until then `@next` installs `0.2.0-rc.1`. Merge nothing
   else: from the moment the tag exists until the stable release is published **nothing may change on `main` but the work record, the
   installation guide and the launcher's README** (6.8 lists them exactly), and the same holds from this merge to the tag: a code
   change that lands in between is not in the tag but is in the commit the stable release is built from, and the check refuses it. No
   Dependabot pull request, no workflow
   edit, no dependency bump, no change to a tool or to an operator document. The cure for any of them is a third candidate. Dependabot
   opens new pull requests on Mondays: leave them open.
2. **The "Require branches to be up to date before merging" rule on `main` is on before you tag.** It may have been turned off to speed
   merging (it was before the first candidate). Read it: `gh api repos/smormah/vsift/branches/main/protection --jq
   .required_status_checks.strict` prints `true`. If it prints `false`, turn it on (Settings, Branches, the rule for `main`, or `gh api -X
   PATCH repos/smormah/vsift/branches/main/protection/required_status_checks -F strict=true`, administrator rights).
3. **The commit you tag** is the merge commit of that pull request. Find it and write the full 40 digits down:

   ```console
   git fetch origin --tags
   git switch main
   git pull --ff-only
   gh pr list --repo smormah/vsift --state merged --search "second release candidate" --json number,title,mergeCommit --jq ".[] | {number, title, commit: .mergeCommit.oid}"
   grep -m1 "^version" Cargo.toml
   ```

   Expect `version = "0.2.0-rc.2"`. The search may list other pull requests that use those words: take the one whose title starts with
   `P14 PR 10 repeated`. If `main` has moved on since only by work-record pull requests, the commit to tag is still that pull
   request's merge commit, not `main`'s head (a later work-record commit is allowed in the stable release; a
   code change is not).
4. **The commit is on `main` and every check of it passed.** The Release workflow builds and plans but runs no tests, and nothing else
   checks that a tagged commit was merged or tested:

   ```console
   git merge-base --is-ancestor <the 40-digit commit> origin/main && echo "on main"
   gh api "repos/smormah/vsift/commits/<the 40-digit commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   Expect `on main` and **no output at all** from the second command (a check that failed or has not finished prints a line, such as
   `Quality (windows-latest): in_progress null`: wait, or do not tag; `Attest build provenance`, `Publish to npm ...` and the fuzz matrix
   show as skipped on a pull request, which is fine).
5. **The checks that bind this commit pass on your machine** (about a minute each, after the first build):

   ```console
   cargo run --locked -p vsift-governance -- check
   cargo run --locked -p vsift-governance -- public-claims
   cargo run --locked -p vsift-governance -- release-evidence
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-2/freeze.json
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-3/freeze.json
   ```

   Expect "VSift delivery ledger is valid.", "VSift public claims agree with the evidence ledger (this proves recorded evidence and
   absent banned words, not that a sentence is true).", "VSift release evidence ledger is valid." and `nothing frozen has changed`
   twice. Do **not** run `release-evidence --complete-for 0.2.0-rc.2` as a gate: it fails on purpose until PR 11 is repeated (plan
   section 25.2 lists the items and why). Any other answer: stop and tell the supervisor; do not tag.
6. **The trusted publishers are still saved** on npmjs.com for all four packages (6.2 step 6: owner `smormah`, repository `vsift`,
   workflow `release.yml`, environment `release`, **npm publish** allowed). No dry run can check this setting (L-100); the first
   candidate's publish on 2026-10-05, with `--tag next`, went through it, so it is expected to hold. **The `release` environment and
   the tag ruleset** exist as 6.2 steps 3 and 4 say (read-only):

   ```console
   gh api repos/smormah/vsift/environments/release --jq '{reviewers: [.protection_rules[]? | .reviewers[]? | .type], branch_policy: .deployment_branch_policy, can_admins_bypass: .can_admins_bypass}'
   gh api repos/smormah/vsift/rulesets --jq '.[] | {name, target, enforcement}'
   ```

   Expect `{"branch_policy":{"custom_branch_policies":true,"protected_branches":false},"can_admins_bypass":false,"reviewers":["User"]}` and
   `{"enforcement":"active","name":"release tags","target":"tag"}`.
7. **What npm holds now**, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`):

   ```console
   npm view vsift-cli dist-tags
   npm view @vsift/win32-x64 dist-tags
   npm view @vsift/darwin-arm64 dist-tags
   npm view @vsift/linux-x64 dist-tags
   ```

   Expect **`{ latest: '0.0.0', next: '0.2.0-rc.1' }` four times**. Anything else (`next` on another version, `latest` not `0.0.0`) means
   something was published or moved since 2026-10-05: stop and tell the supervisor. Then write down what the first candidate's bytes are,
   so that step 4 can show they are untouched:

   ```console
   for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do npm view "$p@0.2.0-rc.1" dist.integrity; done
   git show --no-patch --format="%H" v0.2.0-rc.1^{commit}
   ```

   The last line must print `d5792ce31db1106934233c86d0518c3ad1961e07`.

**1. Tag the commit** (you are the only person the ruleset lets create a `v*` tag):

```console
git tag -a v0.2.0-rc.2 -m "VSift 0.2.0-rc.2" <the 40-digit commit>
git push origin v0.2.0-rc.2
git show --no-patch --format="%H" v0.2.0-rc.2^{commit}
```

The last line prints the commit you wrote down. A wrong tag **before anything is published** may be moved (you can bypass the ruleset):
delete it with `git push origin :refs/tags/v0.2.0-rc.2`, fix, push again. **Never move or delete it after step 3.**

**2. Dry run on the tag, and read its plan before you publish.** This is the lesson of the first candidate's run: the dry run builds the
real bytes and shows exactly what a publish will do, and for `0.2.0-rc.1` the dry run (run 37299364837, 10:52 UTC) and the publish run
(run 37323324151, 14:16 UTC) wrote **byte-identical** checksum files, so what you read in the dry run is what the publish attests and
publishes. Actions, **Release**, **Run workflow**, "Use workflow from" **Tags: v0.2.0-rc.2**, leave **dry_run** ticked, **Run workflow**. Or:

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.0-rc.2 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --event workflow_dispatch --limit 3 --json databaseId,headBranch,headSha,status,conclusion
gh run watch <the run id> --repo smormah/vsift
```

Pick the run by what it says, not by position: its `headBranch` must be `v0.2.0-rc.2` and its `headSha` the commit you wrote down. It
takes about 30 to 40 minutes: 21 jobs, `attest` and `publish` skipped. Open the run's **Publish plan** summary and read it top to bottom.
Expect:

- the heading `Publish plan: dry run, nothing is published` and "This run was dispatched with `dry_run` set";
- `` `latest` is not touched. This release candidate is published under `next` only ``;
- "Version `0.2.0-rc.2` (release candidate), npm dist-tag `next`, Git tag `v0.2.0-rc.2`", the commit you wrote down, `workflow_dispatch`
  on `refs/tags/v0.2.0-rc.2` and "The plan is enforced";
- the table "What this publication does to the dist-tags": for each of the four packages **`next` from `0.2.0-rc.1` to `0.2.0-rc.2`**
  and `latest` staying `0.0.0`;
- two guards, both **passed**: **Pre-release version** and **Registry** ("this version is not `latest` and is not on npm with other
  bytes on any package");
- four `npm publish ... --tag next --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`,
  `@vsift/linux-x64`, `vsift-cli`, and a GitHub release created as a draft **pre-release not marked latest** with ten assets.

Then download the plan and keep it:

```console
gh run download <the dry run's id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

Read `plan-dry/release-notes.md`: it is what the GitHub release will say ("VSift 0.2.0-rc.2 is a release candidate. It is under
qualification, it is not announced, and it is no statement of support or stability ..."), and its last line links the installation guide at
the tag. **Keep `plan-dry/attestation-subjects.sha256` and `plan-dry/release-assets.sha256`:** step 3 compares them with the publish
run's. If anything differs from this list, or the run fails, nothing was published: fix it (6.5, "The plan job fails on the tag"), and
repeat from step 1 if the commit has to change.

**3. Publish.** The same dispatch with **dry_run cleared** (untick it; with `gh`: `-f dry_run=false`) on the same tag. The plan job must
say `Publish plan: PUBLISH a release candidate after the release environment's approval`; `attest` runs (about a minute; **its
attestations are public and permanent from the moment it finishes**, before the approval: repeating is harmless because the builds are
reproducible, 6.5); `publish` stops at **Waiting for review**. Download this run's `publish-plan` the same way (`--dir plan-publish`) and
compare, **before you approve**:

```console
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

Then **Review deployments**, tick `release`, **Approve and deploy**. Approve within a week: the run's files expire after seven days (6.3
step 4). The `publish` job publishes the four packages (about three minutes), waits up to five minutes for `next` to read back as
`0.2.0-rc.2` on all four, checks that `latest` is where it was, and creates and publishes the GitHub pre-release. A message from npm that
a package "is being processed and may take a few minutes to become available" is normal. **After `npm publish` has run for the first
package, only the failure table below applies.**

**4. Check it from the outside** (a shell with no npm login and a folder of your own; about ten minutes). Expect, in this order:

```console
npm view vsift-cli dist-tags
npm view @vsift/win32-x64 dist-tags
npm view @vsift/darwin-arm64 dist-tags
npm view @vsift/linux-x64 dist-tags
```

`{ latest: '0.0.0', next: '0.2.0-rc.2' }` four times. **`latest` must still be `0.0.0` and `next` must be `0.2.0-rc.2`.** Anything else
on `latest` is the emergency of 6.5 ("`latest` moved when it should not have"). The first candidate must be as it was:

```console
for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do npm view "$p@0.2.0-rc.1" dist.integrity; done
```

The four values equal the ones you wrote down in step 0. Then:

```console
mkdir rc-check && cd rc-check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures
npx vsift --version
```

Expect `npm audit signatures` to report verified registry signatures and verified attestations (on Windows 4 and 4; on Linux the two
packages it installs), and `vsift 0.2.0-rc.2 (<the first 12 digits of your commit>)`.

```console
gh release view v0.2.0-rc.2 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `isPrerelease` true, `isDraft` false, ten assets; and **`gh: Not Found (HTTP 404)`** from the second command (no release is marked
latest: the first candidate and 0.1.0 are pre-releases too). If it prints `v0.2.0-rc.2`, the release is wrongly marked latest: `gh release
edit v0.2.0-rc.2 --repo smormah/vsift --latest=false`. The checksums and attestations of the ten files and the four tarballs are 6.4's
commands with the version and `--source-ref refs/tags/v0.2.0-rc.2`; step 5 does them again from a runner that holds no credential, so
doing both is optional.

**5. The second verification and the hosted checks of the published bytes** (nothing here can publish; each is one dispatch, from `main`):

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.2
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.2 -f from_version=0.1.0
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.2 -f from_version=0.2.0-rc.1
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.2
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.0-rc.2
```

`P14 verify release` (RQ-19) takes a few minutes and must be all green: the provenance of four packages, `npm audit signatures`, `gh
attestation verify` of ten files and four tarballs, the checksums, and the release's flags. The two `P14 published artifacts` runs are
one workflow with two upgrade baselines: **0.1.0 to `0.2.0-rc.2`** is what the first candidate's run did, and **`0.2.0-rc.1` to
`0.2.0-rc.2`** is the upgrade a person on `next` really does. The workflow accepts any published version as `from_version` and takes its
tag's commit, so the second dispatch is expected to work, but it has not been run: if it fails in the `resolve` job and not in a check, that
is the dispatch and not the candidate, so open an issue before re-running. **The two runs share a concurrency group and the second waits
for the first; do not dispatch a third while one is waiting, because a third replaces the waiting one.** These are the evidence of P14 PR 11
repeated (RQ-01 to RQ-06 and RQ-19): their results go into the ledger there, and **a failed run is a finding: open an issue before
re-running** (governance rule 14). You do not have to wait for the other four before you finish step 4. The rest of the repeat (the
source-built campaigns at the tag, agent-trial batches 2 and 3, the try-outs) is plan section 25.3; the supervisor dispatches and the
maintainer starts those, each on an explicit go.

**6. Afterwards.** Tell the supervisor: the run ids of the dry run and the publish, the four `npm view` answers, the output of `npm audit
signatures`, the result of the `diff` of step 3 and anything that failed or was re-run. Nothing is announced. **Batches 2 and 3 of the
agent trials start only on your explicit go** (`docs/agents/trials.md`, "The P14 batches"). Their freeze is committed for this candidate
(`docs/planning/p14-agent-trials/batch-2/freeze.json` and `batch-3/freeze.json`: the same digests as the first candidate's, because nothing
frozen changed) and `prepare` refuses a trial if the skill, grader, scenarios or settings differ from it. `batch-2/` holds only that file:
the first candidate's batch-2 records, summary and state are in `batch-2-rc.1/`, and a batch runs against one version, so do not copy
their state files back.

**7. Optional, once step 5's runs have finished: deprecate the first candidate.** `next` already points at the second, so a new install
gets it; deprecating `0.2.0-rc.1` makes npm print a warning to anyone who still pins it. It is reversible and it is your choice, and it
needs your npm login with two-factor authentication. **Do it after step 5's runs finish, not before:** the upgrade job installs
`0.2.0-rc.1` by its exact version and a deprecated version prints a warning on install.

```console
npm deprecate "vsift-cli@0.2.0-rc.1" "Superseded by 0.2.0-rc.2. Install vsift-cli@next."
npm deprecate "@vsift/win32-x64@0.2.0-rc.1" "Superseded by 0.2.0-rc.2. Install vsift-cli@next."
npm deprecate "@vsift/darwin-arm64@0.2.0-rc.1" "Superseded by 0.2.0-rc.2. Install vsift-cli@next."
npm deprecate "@vsift/linux-x64@0.2.0-rc.1" "Superseded by 0.2.0-rc.2. Install vsift-cli@next."
```

An empty message (`npm deprecate "vsift-cli@0.2.0-rc.1" ""`) removes the deprecation. You may also edit the first release page's notes to say
that it is superseded by `v0.2.0-rc.2` (`gh release edit v0.2.0-rc.1 --repo smormah/vsift --notes-file <the file>`); neither is a
condition of anything.

**If something fails.**

| When | What it means | What to do |
| --- | --- | --- |
| Step 0 fails (a failing check run, not on `main`, a governance or freeze answer, `next` or `latest` not as stated) | the commit or the registry is not what was reviewed | do not tag; tell the supervisor |
| Step 2 fails, is refused, or the plan differs from the list | nothing was attested or published | read "Refused because:", fix it, move the tag if the commit changes (nothing was published from it) |
| Step 2: the plan's `next` moves from something other than `0.2.0-rc.1` | someone published or moved something | do not publish; tell the supervisor |
| Step 2: the **Registry** guard fails ("`0.2.0-rc.2` is already on npm, and not with these bytes") | the version was published before with other bytes: the number is used for good | do not publish; the next candidate is `0.2.0-rc.3` and needs its own cut |
| Step 3: the two `diff`s differ | the bytes are not the dry run's | do not approve; open an issue; the run stays unapproved and expires |
| Step 3: `publish` fails with `ENEEDAUTH` before any `+ package@version` line | a trusted publisher is not saved or is wrong | 6.5 first bullet of `ENEEDAUTH`; nothing was published |
| Step 3: `publish` fails after some packages | a partial publish ([L-097](../planning/known-limits.md#l-097)) | **Re-run failed jobs** on the same run within seven days and approve again: versions npm holds with the same bytes are skipped |
| Step 4: `next` is not `0.2.0-rc.2` after ten minutes | the registry is slow, or the publish stopped | look at the `publish` job's log before doing anything |
| Step 4: `latest` is not `0.0.0` | the workflow is wrong | 6.5 "`latest` moved when it should not have": `npm dist-tag add <package>@0.0.0 latest` for each package that moved, then open an issue; **do not dispatch again** |
| Step 4: the first candidate's integrity values changed | impossible by npm's rules; it would mean the reading was of another package | stop and tell the supervisor |
| A published version is bad | the candidate itself is wrong | **never unpublish**: `npm deprecate` it on all four packages, edit the release's notes, and cut `0.2.0-rc.3` by this section again |

**What is irreversible, in one list.** The four published `0.2.0-rc.2` versions and their provenance records on npm's transparency log; the
Sigstore attestations of the ten files and four tarballs (they exist from the end of `attest`, before the approval); the tag once anything
was published from it; and, from 2026-10-05, the four `0.2.0-rc.1` versions, which this section does not change. **Policy, not
impossibility:** GitHub lets you delete a release page or move a tag, and this runbook says never (6.5), because the page, the tag and
the attestations name each other. **What is reversible:** `next` (`npm dist-tag add vsift-cli@0.2.0-rc.1 next`, for each package,
two-factor authentication; the workflow never does it), the notes, and a deprecation (`npm deprecate <package>@<version> ""` removes it).

### 6.12 The third release candidate, `0.2.0-rc.3`: the exact steps (P14 PR 10, repeated again)

**A record (2026-10-10).** Published on 2026-10-08, when `latest` was the empty `0.0.0` placeholder; kept as what was done. For a candidate published now, with `0.2.0` as `latest` (so `latest` stays `0.2.0`, and GitHub's latest release stays `v0.2.0` instead of being a 404), follow 6.16.

**Why there is a third candidate.** Agent-trial batch 2 ran on the published `0.2.0-rc.2` on 2026-10-07, and Claude Opus 5.5's review tier
missed two gates (the blurred banner 1 of 3; the citation check 4 of 6), as it had on the first candidate. The evidence item RQ-15 is
`failed` for `0.2.0-rc.2`. The maintainer decided the same day not to waive the gates and not to exclude the model, but to change the skill
and cut a third candidate (plan sections 27 and 28). The skill ships in the package, so a changed skill is a new candidate by the rules of 6.8.
**`0.2.0-rc.3` is `0.2.0-rc.2` plus exactly this, and the version bump:**

| Change | Pull request | What it is |
| --- | --- | --- |
| The skill's two evidence rules | this cut | an unreadable region proves nothing about its content in either direction; a claim states only what its own citations show or say (`skills/vsift/SKILL.md`, `references/handoff.md`) |
| A floor for short audio, and a range too short to be a sample | #330 (#322, #332) | audio under 100 ms is recorded as a gap and never given to the recogniser; `audio` of a range of 31 microseconds or less is refused as `INVALID_ARGUMENT` |
| A copy that runs out of time says so | #331 (#325) | the code stays `INVALID_SOURCE`; the answer gains a remediation |
| Three test and tool fixes | #333 (#321, #310, #327) | the supervisor test's marker race, the malicious-media campaign's no-room case, a trial-harness test; no product code |

Nothing else: no FFmpeg or whisper.cpp re-pin (after the stable release, L-132 and L-137) and no Dependabot pull request. It is published as 6.11
published the second, and this section is that one with its own numbers. **`0.2.0-rc.1` and `0.2.0-rc.2` stay published and tagged: they are
superseded, not withdrawn,** and no step below touches them. A candidate is never announced and moves `next` only: `latest` stays the empty
`0.0.0` placeholder on all four packages at every step. It takes about an hour, most of it waiting for builds and the registry. Times are the
maintainer's. The commands are written for Git Bash (or any POSIX shell: the `for` loops are shell); the `gh` and `npm` commands work in
PowerShell too, and `^{commit}` needs quotes there.

**What was learned from the second candidate's publish (2026-10-07).** It went as 6.11 is written: the packages were published at 09:43 UTC
by publish run 37592020376, which is the run npm's provenance names, at the tag's commit `7c722d1fc46af7fddeffbaf807028eaec413ace1`; `P14
verify release` (run 37602927119) passed its 20 checks minutes later; the upgrade from `0.2.0-rc.1`, which no run had tried before, worked
(`P14 published artifacts` run 37611381117), so `from_version` does accept any published version. The supervisor also checked the result
from outside with an empty npm configuration (no login, no token) and saw what step 4 expects. Nothing failed and nothing was re-run, so
this section changes no step: it changes the numbers, adds one upgrade baseline to choose from, and says how to dispatch runs that share a
concurrency group. One thing 6.11 did not have to say: **the dry run's id of the second candidate was not written down in the repository;
write down both run ids this time (step 6).**

**What differs from 6.11, in one table.**

| | Second candidate (6.11, 2026-10-07) | Third candidate (this section) |
| --- | --- | --- |
| What npm holds before the publish | `next` `0.2.0-rc.1`, `latest` `0.0.0` | `next` **`0.2.0-rc.2`**, `latest` `0.0.0`, on all four packages |
| What the dry run's plan does to `next` | moves it `0.2.0-rc.1` to `0.2.0-rc.2` | moves it **`0.2.0-rc.2` to `0.2.0-rc.3`**; `latest` stays `0.0.0` |
| The tag and the commit it names | `v0.2.0-rc.2` at `7c722d1fc46a`, the merge of #320 | `v0.2.0-rc.3` at the merge commit of the pull request whose title starts "P14 PR 10 repeated again" |
| What `vsift --version` prints | `vsift 0.2.0-rc.2 (7c722d1fc46a)` | `vsift 0.2.0-rc.3 (<the first 12 digits of that commit>)` |
| What changed since the candidate before | two fixes under `crates/` | the skill, two changes of the program's answers, a remediation, tests and a campaign tool (the table above) |
| The agent-trial freeze | the first candidate's digests (nothing frozen changed) | **a new freeze**: the `skill` digest differs on purpose, the other six are unchanged (step 6) |
| The earlier candidates' bytes to write down | the first's | **the first's and the second's** (step 0.7) |
| The upgrade check (`from_version`) | `0.1.0` and `0.2.0-rc.1` | **`0.1.0` and `0.2.0-rc.2`** (the upgrade a person on `next` really does); `0.2.0-rc.1` is optional (step 5) |
| How the upgrade runs are dispatched | two, the second waits | **one after another:** never more than one waiting (step 5) |
| The candidate the stable release is compared with | `v0.2.0-rc.2` | **`v0.2.0-rc.3`**, the highest candidate tag (6.8) |
| `release-evidence --complete-for` | failed on 17 of 20 items | fails on 16 of the 20 items until PR 11 is repeated (plan section 28.3): **do not run it as a gate now** |
| After the publish | optionally deprecate the first candidate | nothing: the maintainer decided on 2026-10-09 to deprecate `0.2.0-rc.1` and `0.2.0-rc.2` at the stable release, not before (step 7) |

**0. Before you start (about fifteen minutes; only the settings step can change anything).**

1. **Merge the pull request only when you can tag and publish at once.** On merge the README, the installation guide and the launcher's
   README name `0.2.0-rc.3` as the release candidate now in qualification, which is true only once it is published (known limit
   [L-133](../planning/known-limits.md#l-133)); the installation guide says that until then `@next` installs `0.2.0-rc.2`. Merge nothing
   else: from the moment the tag exists until the stable release is published **nothing may change on `main` but the work record, the
   installation guide and the launcher's README** (6.8 lists them exactly), and the same holds from this merge to the tag: a code
   change that lands in between is not in the tag but is in the commit the stable release is built from, and the check refuses it. No
   Dependabot pull request, no workflow edit, no dependency bump, no change to a tool or to an operator document. The cure for any of
   them is a fourth candidate. Dependabot opens new pull requests on Mondays: leave them open.
2. **The "Require branches to be up to date before merging" rule on `main` is on before you tag.** Read it: `gh api
   repos/smormah/vsift/branches/main/protection --jq .required_status_checks.strict` prints `true` (it was on when this cut was
   prepared). If it prints `false`, turn it on (Settings, Branches, the rule for `main`, or `gh api -X PATCH
   repos/smormah/vsift/branches/main/protection/required_status_checks -F strict=true`, administrator rights).
3. **The commit you tag** is the merge commit of that pull request. Find it and write the full 40 digits down:

   ```console
   git fetch origin --tags
   git switch main
   git pull --ff-only
   gh pr list --repo smormah/vsift --state merged --search "third release candidate" --json number,title,mergeCommit --jq ".[] | {number, title, commit: .mergeCommit.oid}"
   grep -m1 "^version" Cargo.toml
   ```

   Expect `version = "0.2.0-rc.3"`. The search may list other pull requests that use those words: take the one whose title starts with
   `P14 PR 10 repeated again`. If `main` has moved on since only by work-record pull requests, the commit to tag is still that pull
   request's merge commit, not `main`'s head (a later work-record commit is allowed in the stable release; a code change is not).
4. **The commit is on `main` and every check of it passed.** The Release workflow builds and plans but runs no tests, and nothing else
   checks that a tagged commit was merged or tested:

   ```console
   git merge-base --is-ancestor <the 40-digit commit> origin/main && echo "on main"
   gh api "repos/smormah/vsift/commits/<the 40-digit commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   Expect `on main` and **no output at all** from the second command (a check that failed or has not finished prints a line, such as
   `Quality (windows-latest): in_progress null`: wait, or do not tag; `Attest build provenance`, `Publish to npm ...` and the fuzz matrix
   show as skipped on a pull request, which is fine).
5. **The checks that bind this commit pass on your machine** (about a minute each, after the first build):

   ```console
   cargo run --locked -p vsift-governance -- check
   cargo run --locked -p vsift-governance -- public-claims
   cargo run --locked -p vsift-governance -- release-evidence
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-2/freeze.json
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-3/freeze.json
   ```

   Expect "VSift delivery ledger is valid.", "VSift public claims agree with the evidence ledger (this proves recorded evidence and
   absent banned words, not that a sentence is true).", "VSift release evidence ledger is valid." and `nothing frozen has changed`
   twice. Do **not** run `release-evidence --complete-for 0.2.0-rc.3` as a gate: it fails on purpose until PR 11 is repeated (plan
   section 28.3 lists the items and why). Any other answer: stop and tell the supervisor; do not tag.
6. **The trusted publishers are still saved** on npmjs.com for all four packages (6.2 step 6: owner `smormah`, repository `vsift`,
   workflow `release.yml`, environment `release`, **npm publish** allowed). No dry run can check this setting (L-100); the first two
   candidates' publishes, with `--tag next`, went through it, so it is expected to hold. **The `release` environment and the tag
   ruleset** exist as 6.2 steps 3 and 4 say (read-only):

   ```console
   gh api repos/smormah/vsift/environments/release --jq '{reviewers: [.protection_rules[]? | .reviewers[]? | .type], branch_policy: .deployment_branch_policy, can_admins_bypass: .can_admins_bypass}'
   gh api repos/smormah/vsift/rulesets --jq '.[] | {name, target, enforcement}'
   ```

   Expect `{"branch_policy":{"custom_branch_policies":true,"protected_branches":false},"can_admins_bypass":false,"reviewers":["User"]}` and
   `{"enforcement":"active","name":"release tags","target":"tag"}`.
7. **What npm holds now**, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`):

   ```console
   npm view vsift-cli dist-tags
   npm view @vsift/win32-x64 dist-tags
   npm view @vsift/darwin-arm64 dist-tags
   npm view @vsift/linux-x64 dist-tags
   ```

   Expect **`{ latest: '0.0.0', next: '0.2.0-rc.2' }` four times**. Anything else (`next` on another version, `latest` not `0.0.0`) means
   something was published or moved since 2026-10-07: stop and tell the supervisor. Then write down what the earlier candidates' bytes
   are, so that step 4 can show they are untouched:

   ```console
   for v in 0.2.0-rc.1 0.2.0-rc.2; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
   git show --no-patch --format="%H" v0.2.0-rc.1^{commit}
   git show --no-patch --format="%H" v0.2.0-rc.2^{commit}
   ```

   Eight integrity values, and the last two lines must print `d5792ce31db1106934233c86d0518c3ad1961e07` and
   `7c722d1fc46af7fddeffbaf807028eaec413ace1`.

**1. Tag the commit** (you are the only person the ruleset lets create a `v*` tag):

```console
git tag -a v0.2.0-rc.3 -m "VSift 0.2.0-rc.3" <the 40-digit commit>
git push origin v0.2.0-rc.3
git show --no-patch --format="%H" v0.2.0-rc.3^{commit}
```

The last line prints the commit you wrote down. A wrong tag **before anything is published** may be moved (you can bypass the ruleset):
delete it with `git push origin :refs/tags/v0.2.0-rc.3`, fix, push again. **Never move or delete it after step 3.**

**2. Dry run on the tag, and read its plan before you publish.** The dry run builds the real bytes and shows exactly what a publish will
do. For `0.2.0-rc.1` the dry run (run 37299364837) and the publish run (run 37323324151) wrote **byte-identical** checksum files, and the
second candidate's publish went the same way: what you read in the dry run is what the publish attests and publishes, and step 3
compares the two before you approve. Actions, **Release**, **Run workflow**, "Use workflow from" **Tags: v0.2.0-rc.3**, leave
**dry_run** ticked, **Run workflow**. Or:

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.0-rc.3 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --event workflow_dispatch --limit 3 --json databaseId,headBranch,headSha,status,conclusion
gh run watch <the run id> --repo smormah/vsift
```

Pick the run by what it says, not by position: its `headBranch` must be `v0.2.0-rc.3` and its `headSha` the commit you wrote down. It
takes about 30 to 40 minutes: 21 jobs, `attest` and `publish` skipped. Open the run's **Publish plan** summary and read it top to bottom.
Expect:

- the heading `Publish plan: dry run, nothing is published` and "This run was dispatched with `dry_run` set";
- `` `latest` is not touched. This release candidate is published under `next` only ``;
- "Version `0.2.0-rc.3` (release candidate), npm dist-tag `next`, Git tag `v0.2.0-rc.3`", the commit you wrote down, `workflow_dispatch`
  on `refs/tags/v0.2.0-rc.3` and "The plan is enforced";
- the table "What this publication does to the dist-tags": for each of the four packages **`next` from `0.2.0-rc.2` to `0.2.0-rc.3`**
  and `latest` staying `0.0.0`;
- two guards, both **passed**: **Pre-release version** and **Registry** ("this version is not `latest` and is not on npm with other
  bytes on any package");
- four `npm publish ... --tag next --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`,
  `@vsift/linux-x64`, `vsift-cli`, and a GitHub release created as a draft **pre-release not marked latest** with ten assets.

Then download the plan and keep it:

```console
gh run download <the dry run's id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

Read `plan-dry/release-notes.md`: it is what the GitHub release will say ("VSift 0.2.0-rc.3 is a release candidate. It is under
qualification, it is not announced, and it is no statement of support or stability ..."), and its last line links the installation guide at
the tag. **Keep `plan-dry/attestation-subjects.sha256` and `plan-dry/release-assets.sha256`:** step 3 compares them with the publish
run's. If anything differs from this list, or the run fails, nothing was published: fix it (6.5, "The plan job fails on the tag"), and
repeat from step 1 if the commit has to change.

**3. Publish.** The same dispatch with **dry_run cleared** (untick it; with `gh`: `-f dry_run=false`) on the same tag. The plan job must
say `Publish plan: PUBLISH a release candidate after the release environment's approval`; `attest` runs (about a minute; **its
attestations are public and permanent from the moment it finishes**, before the approval: repeating is harmless because the builds are
reproducible, 6.5); `publish` stops at **Waiting for review**. Download this run's `publish-plan` the same way (`--dir plan-publish`) and
compare, **before you approve**:

```console
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

Then **Review deployments**, tick `release`, **Approve and deploy**. Approve within a week: the run's files expire after seven days (6.3
step 4). The `publish` job publishes the four packages (about three minutes), waits up to five minutes for `next` to read back as
`0.2.0-rc.3` on all four, checks that `latest` is where it was, and creates and publishes the GitHub pre-release. A message from npm that
a package "is being processed and may take a few minutes to become available" is normal. **After `npm publish` has run for the first
package, only the failure table below applies.**

**4. Check it from the outside** (a shell with no npm login and a folder of your own; about ten minutes). Expect, in this order:

```console
npm view vsift-cli dist-tags
npm view @vsift/win32-x64 dist-tags
npm view @vsift/darwin-arm64 dist-tags
npm view @vsift/linux-x64 dist-tags
```

`{ latest: '0.0.0', next: '0.2.0-rc.3' }` four times. **`latest` must still be `0.0.0` and `next` must be `0.2.0-rc.3`.** Anything else
on `latest` is the emergency of 6.5 ("`latest` moved when it should not have"). The earlier candidates must be as they were:

```console
for v in 0.2.0-rc.1 0.2.0-rc.2; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
```

The eight values equal the ones you wrote down in step 0. Then:

```console
mkdir rc-check && cd rc-check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures
npx vsift --version
```

Expect `npm audit signatures` to report verified registry signatures and verified attestations (on Windows 4 and 4; on Linux the two
packages it installs), and `vsift 0.2.0-rc.3 (<the first 12 digits of your commit>)`.

```console
gh release view v0.2.0-rc.3 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `isPrerelease` true, `isDraft` false, ten assets; and **`gh: Not Found (HTTP 404)`** from the second command (no release is marked
latest: the earlier candidates and 0.1.0 are pre-releases too). If it prints `v0.2.0-rc.3`, the release is wrongly marked latest: `gh release
edit v0.2.0-rc.3 --repo smormah/vsift --latest=false`. The checksums and attestations of the ten files and the four tarballs are 6.4's
commands with the version and `--source-ref refs/tags/v0.2.0-rc.3`; step 5 does them again from a runner that holds no credential, so
doing both is optional.

**5. The second verification and the hosted checks of the published bytes** (nothing here can publish; each is one dispatch, from `main`).
Three of the four workflows are dispatched once:

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.3
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.3
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.0-rc.3
```

`P14 published artifacts` is dispatched once for each upgrade baseline, **one after another**:

```console
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.3 -f from_version=0.1.0
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.0-rc.3 -f from_version=0.2.0-rc.2
```

`P14 verify release` (RQ-19) takes a few minutes and must be all green: the provenance of four packages, `npm audit signatures`, `gh
attestation verify` of ten files and four tarballs, the checksums, and the release's flags. **Which upgrades matter, read from the
workflow:** its `upgrade` job installs the published `from_version`, makes a session, a retained bundle and a registered configuration,
upgrades to `version` over the real registry and reads them back, on three systems; the other 20 jobs (the tools' own tests, the version
choice, twelve clean installs, three archives, the offline install) do not depend on `from_version` and simply run again. Two baselines are needed: **0.1.0 to `0.2.0-rc.3`**, the baseline
every candidate has been measured from and the only published version that is not a candidate, and **`0.2.0-rc.2` to `0.2.0-rc.3`**, the
upgrade a person on `next` really does. **`0.2.0-rc.1` to `0.2.0-rc.3` is optional** (a third dispatch with `-f
from_version=0.2.0-rc.1`; each run was 34 job-minutes on the second candidate): it covers a person who pinned the first candidate,
which was `next` for two days and was never announced. The first candidate stores what the second stores (the two fixes between them
changed no stored format), so it is expected to add little. Say whether you ran it. **Runs of one workflow dispatched from `main` share a concurrency group** (`p14-published-artifacts-refs/heads/main`,
never cancelled): one runs, one may wait, and **a newer waiting run replaces an older waiting run without a word**. So dispatch the
second only after the first has started, and a third only after the first has finished; the simplest rule is to wait for each to finish. The same holds for any workflow you dispatch twice from `main`. The workflow accepts any published version as
`from_version` and takes its tag's commit; that was first tried on the second candidate and worked. These are the evidence of P14 PR 11
repeated again (RQ-01 to RQ-06 and RQ-19): their results go into the ledger there, and **a failed run is a finding: open an issue before
re-running** (governance rule 14). You do not have to wait for the other runs before you finish step 4. The rest of the repeat (the
source-built campaigns at the tag, agent-trial batches 2 and 3, the try-outs) is plan section 28.4; the supervisor dispatches and the
maintainer starts those, each on an explicit go.

**6. Afterwards.** Tell the supervisor: **the run ids of the dry run and of the publish**, the four `npm view` answers, the output of
`npm audit signatures`, the result of the `diff` of step 3, which upgrade baselines you dispatched, and anything that failed or was
re-run. Nothing is announced. **Batches 2 and 3 of the agent trials start only on your explicit go** (`docs/agents/trials.md`, "The P14
batches"; the checklist is [`p14-batch-2-3-checklist.md`](../planning/p14-batch-2-3-checklist.md)). **Their freeze is new for this
candidate** (`docs/planning/p14-agent-trials/batch-2/freeze.json` and `batch-3/freeze.json`): the skill changed on purpose, so the `skill`
digest and the whole-freeze digest differ from the first two candidates', and the grader, the scenarios, the cold scenarios, the hold-outs,
the settings and the corpus truth are byte-identical to theirs. `prepare` refuses a trial if anything frozen differs from the file.
`batch-2/` holds only that file: the first candidate's batch-2 records are in `batch-2-rc.1/` and the second's in `batch-2-rc.2/`, and a
batch runs against one version, so do not copy their state files back. Batch 3 still takes `-AllowGraderChange`, for the reason the
checklist gives (the grader changed between batch 1 and batch 2, on 2026-10-04; it has not changed since).

**7. Deprecating the earlier candidates: decided on 2026-10-09 (at the stable release, not before) and done on 2026-10-10.** This step was
written as optional and before the stable release; the maintainer decided otherwise. `0.2.0-rc.1` and `0.2.0-rc.2` were deprecated on all
four packages after the stable release `0.2.0` was published and checked, with the message `Superseded by 0.2.0. Install vsift-cli.`
(eight `npm deprecate` commands, the maintainer's login and second factor; read back with `npm view <package>@<version> deprecated`), and
`0.2.0-rc.3` and `0.2.0` were not deprecated. Section 6.13 and step 9 of
[`p14-stable-release-steps.md`](../planning/p14-stable-release-steps.md) have the commands and the result. An empty message
(`npm deprecate "vsift-cli@0.2.0-rc.2" ""`) removes a deprecation. Do not deprecate a candidate before the runs that install it by its exact
version have finished: the upgrade job installs a version by number, and a deprecated version prints a warning on install.

**If something fails.**

| When | What it means | What to do |
| --- | --- | --- |
| Step 0 fails (a failing check run, not on `main`, a governance or freeze answer, `next` or `latest` not as stated) | the commit or the registry is not what was reviewed | do not tag; tell the supervisor |
| Step 2 fails, is refused, or the plan differs from the list | nothing was attested or published | read "Refused because:", fix it, move the tag if the commit changes (nothing was published from it) |
| Step 2: the plan's `next` moves from something other than `0.2.0-rc.2` | someone published or moved something | do not publish; tell the supervisor |
| Step 2: the **Registry** guard fails ("`0.2.0-rc.3` is already on npm, and not with these bytes") | the version was published before with other bytes: the number is used for good | do not publish; the next candidate is `0.2.0-rc.4` and needs its own cut |
| Step 3: the two `diff`s differ | the bytes are not the dry run's | do not approve; open an issue; the run stays unapproved and expires |
| Step 3: `publish` fails with `ENEEDAUTH` before any `+ package@version` line | a trusted publisher is not saved or is wrong | 6.5 first bullet of `ENEEDAUTH`; nothing was published |
| Step 3: `publish` fails after some packages | a partial publish ([L-097](../planning/known-limits.md#l-097)) | **Re-run failed jobs** on the same run within seven days and approve again: versions npm holds with the same bytes are skipped |
| Step 4: `next` is not `0.2.0-rc.3` after ten minutes | the registry is slow, or the publish stopped | look at the `publish` job's log before doing anything |
| Step 4: `latest` is not `0.0.0` | the workflow is wrong | 6.5 "`latest` moved when it should not have": `npm dist-tag add <package>@0.0.0 latest` for each package that moved, then open an issue; **do not dispatch again** |
| Step 4: an earlier candidate's integrity value changed | impossible by npm's rules; it would mean the reading was of another package | stop and tell the supervisor |
| Step 5: an upgrade run you dispatched is gone from the list | a later dispatch replaced it while it waited | dispatch it again, after the running one has finished; nothing was lost but time |
| A published version is bad | the candidate itself is wrong | **never unpublish**: `npm deprecate` it on all four packages, edit the release's notes, and cut `0.2.0-rc.4` by this section again |

**What is irreversible, in one list.** The four published `0.2.0-rc.3` versions and their provenance records on npm's transparency log; the
Sigstore attestations of the ten files and four tarballs (they exist from the end of `attest`, before the approval); the tag once anything
was published from it; and the eight versions of `0.2.0-rc.1` (2026-10-05) and `0.2.0-rc.2` (2026-10-07), which this section does not
change. **Policy, not impossibility:** GitHub lets you delete a release page or move a tag, and this runbook says never (6.5), because the
page, the tag and the attestations name each other. **What is reversible:** `next` (`npm dist-tag add vsift-cli@0.2.0-rc.2 next`, for each
package, two-factor authentication; the workflow never does it), the notes, and a deprecation (`npm deprecate <package>@<version> ""`
removes it).

### 6.13 The stable release `0.2.0`: what happened (P14 PR 12 and PR 13, 2026-10-09 and 2026-10-10)

The maintainer's steps for this release were not a section of this runbook but a planning page,
[`p14-stable-release-steps.md`](../planning/p14-stable-release-steps.md), because this file could not change between the third candidate
and the tag (6.8). That page is 6.7 written out with this release's numbers; it is the model for the next stable version, and this
section records what happened when it was followed and what the real services showed. **`0.2.0` is built from the same source as
`0.2.0-rc.3`: the stable commit differs from the tag `v0.2.0-rc.3` only in five version-string files, the two shipped documents and the work
record (6.8).** The tag is `v0.2.0` at `eeb2a22a46a85ab10a456f2ab5d6a62e292836c5` (the merge of #344).

| Step | What happened |
| --- | --- |
| Dry run on the tag | `workflow_dispatch` on `v0.2.0` with `dry_run` set, 2026-10-09 14:23 UTC, run 37943856631, green in about 19 minutes: `attest` and `publish` skipped, all six guards `passed`. |
| The real dispatch | The same dispatch with `dry_run` cleared, 14:43 UTC, run 37946261087, which is the run npm's provenance names. The plan and `attest` finished at 14:54. `attestation-subjects.sha256` and `release-assets.sha256` were identical to the dry run's, and the plan's `release-delta.json` was kept (it is the ledger's `release_delta`). |
| The six guards | Stable version; accepted candidate (against `v0.2.0-rc.3` at `83dca856e7a0`: 5 version-string, 2 shipped-document and 77 work-record files differ, nothing else); registry read; candidate published on all four packages; `latest` moves forward (`0.0.0` on all four); evidence ledger (complete for `0.2.0-rc.3`). All `passed`. |
| The approval | The maintainer approved the `release` environment; the `publish` job started at 16:57 UTC and ended at 17:00 UTC. |
| `--tag latest` under trusted publishing | **It was accepted.** The four `npm publish ... --tag latest --provenance --ignore-scripts` commands ran in order (`@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`) in 22 seconds, each signed a provenance statement and each printed `+ package@0.2.0`; no `ENEEDAUTH`, and no bootstrap token existed. |
| npm's read-back of `latest` | **It took about two and a half minutes**: the job polled five times, 30 seconds apart (it allows ten), before `latest` read `0.2.0` on all four packages. `next` read `0.2.0-rc.3` on all four throughout. |
| The GitHub release | Created as a draft with ten assets and published at 17:00:34 UTC with `--latest`: not a draft, not a pre-release, and GitHub's own latest release read `v0.2.0` at the first attempt. |

**Checked from outside, after the publish:** `latest` is `0.2.0` and `next` is `0.2.0-rc.3` on all four packages;
a clean `npm install vsift-cli` with no tag installs `0.2.0` and `vsift --version` prints `vsift 0.2.0 (eeb2a22a46a8)`; `npm audit
signatures` reports 4 verified registry signatures and 4 verified attestations; `gh attestation verify` passes for 14 of 14 files (the ten
release files and the four npm tarballs, with `--source-ref refs/tags/v0.2.0`); the registry's tarballs equal the plan's checksums; the
integrity values of `0.2.0-rc.1`, `0.2.0-rc.2` and `0.2.0-rc.3` (twelve values) were unchanged. **The hosted checks were then run on `0.2.0`'s own
bytes** (2026-10-09): `P14 published artifacts` twice (from `0.1.0` and from `0.2.0-rc.3`, runs 37999883603 and 38000209110), `P14 journeys`
(37999886382), `P13 managed smoke` (37999889647) and `P14 scan reading` (38007675152, read on 2026-10-10: `p14-scan-reading-2026-10-10.md`).
**`P14 verify release`'s first run on `0.2.0` (37999880619) was red on exactly the two checks a stable version adds, "not registered yet",
by design** (20 other checks passed); P14 PR 13a (#348) registered them in `tools/p14-published/lib/verify.cjs`, and the run that records
RQ-19 for `0.2.0` is `P14 verify release` run 38013937541 (2026-10-10, from `main` at `6ca2f3c`: 22 checks passed, the two that a stable version adds among them). **The two checks read `release-delta.json` from the publish run's `publish-plan` artifact, which GitHub
keeps seven days (until 2026-10-16 for this release), so the verification of a stable version is dispatched within seven days of its publish**;
after that the delta check fails by name and points at the evidence ledger's `release_delta`, where the record was copied on 2026-10-10.

**Deprecation.** On 2026-10-10 `0.2.0-rc.1` and `0.2.0-rc.2` were deprecated on all four packages with the message `Superseded by 0.2.0.
Install vsift-cli.` (6.12 step 7; the maintainer's login and second factor; read back with `npm view <package>@<version> deprecated`), and
`0.2.0-rc.3` and `0.2.0` were not. **`next` was not moved** and the workflow never moves it ([L-108](../planning/known-limits.md#l-108)):
`vsift-cli@next` still installs `0.2.0-rc.3`, and whether to move it is the maintainer's open choice.

**What was not tried, and is not shown.** Yarn's behaviour for an untagged `yarn add vsift-cli` inside its one-day hold (the hosted jobs
installed `vsift-cli@0.2.0` by exact version after the documented exemption, and Yarn did hold the 5.6-hour-old version back); the weekly
`P07 local ASR` gates, `P14 load` and `P14 runbook walk` on the stable release's own tag (they are the candidate's, carried); and everything the
stable release ships untried (the maintainer's try-outs, RQ-17, [L-143](../planning/known-limits.md#l-143)). Nothing here announces anything.

### 6.14 The release candidate `0.2.1-rc.1`: the exact steps (the patch release's first candidate, cut 2026-10-10)

**A record (2026-10-11).** Published on 2026-10-10 (tag `v0.2.1-rc.1` at `cb8cb3a6286047db56831459dabd6ee5500a37bd`, Release run 38084137607; `P14 verify release` run 38086846526 passed: `next 0.2.1-rc.1, latest 0.2.0`, GitHub's latest release `v0.2.0`)
and **superseded on 2026-10-11 by `0.2.1-rc.2`**, which the maintainer decided to cut to carry the fix of #359 and the larger findings; kept as what was done. For a candidate published now, with `0.2.1-rc.2` on `next`, follow 6.16.

**Status when written: cut, not tagged, not published.** This section was written before the candidate existed on npm; nothing in it had been run. It is 6.12 with the numbers of
this candidate and with the one thing that is new: **it is the first candidate published while a stable version exists, so `latest` is `0.2.0`, not the empty
`0.0.0` placeholder** (`0.0.0` was `latest` for every earlier candidate). A candidate still moves `next` and nothing else, so **`latest` must stay `0.2.0` on all
four packages at every step, and GitHub's own "latest release" must stay `v0.2.0`**; every place below that read "`0.0.0`" or "404" in 6.12 reads differently here.
Sections 6.10 to 6.12 are records of past publishes; do not follow them for this candidate.

**Why there is a candidate.** The first real recording tried on `0.2.0` found a high-severity defect the day after the release
([#353](https://github.com/smormah/vsift/issues/353), L-145). The maintainer decided on 2026-10-10 that the patch release `0.2.1`
carries it with everything else that was waiting, and that a changed skill is a new candidate by the rules of 6.8. **`0.2.1-rc.1` is `0.2.0` plus exactly this and the version bump**
(the plan for the release is [`release-0.2.1.md`](../planning/release-0.2.1.md)):

| Change | Pull request | What it is |
| --- | --- | --- |
| A transcription chunk that cannot be used is a recorded gap | #358 (#353) | `transcript retranscribe` and `job resume` answer `partial` and list what they did not transcribe; the quarter rule tolerates one rejected segment below four; the failure says what failed; optional new stored member `carried_untranscribed` |
| The audio clip's text, and a `BUSY` with a retry hint | #357 (#340, #342) | `audio`'s help and readable result say an agent cannot listen to a clip; an evidence command's `BUSY` carries `retry_after_ms` 2000 and a remediation |
| A flaky Windows test of the trial harness | #356 (#345) | test only; the harness is unchanged |
| The agent skill | this cut (#349 and the three above) | the wording "video walkthrough", partial transcriptions read as gaps, a `BUSY` retried once, the audio clip's text (`skills/vsift`, `docs/agents/skill.md`) |

Nothing else: no FFmpeg or whisper.cpp re-pin ([#351](https://github.com/smormah/vsift/issues/351), L-132, L-137, L-144), no dependency change (`Cargo.lock` differs from `0.2.0`'s only in the
version text of the workspace crates) and none of #337, #325, #334 or #363. Against the tag `v0.2.0`: 54 files under `crates/` (31 source, 23 test and snapshot files; +6,403, -429), ten under `tools/` (the second
verification's two checks for a stable version (P14 PR 13a) and their tests, the tests this cut adds, the freeze test's pin, and one test file of the trial harness, #345), nine under `schemas/v1/` (descriptions, three examples and one new member),
the skill, one workflow (`.github/workflows/p14-verify-release.yml`, a hosted verification workflow, not the Release workflow) and the five version-string files. **Nothing under `tools/vsift-release/`,
`tools/vsift-governance/`, `tools/vsift-agent-trials/src`, `fixtures/`, `deny.toml`, `rust-toolchain.toml` or `.github/workflows/release.yml` differs.**

**`0.2.0`, `0.2.0-rc.1`, `0.2.0-rc.2` and `0.2.0-rc.3` stay published and tagged and no step below touches them** (`0.2.0-rc.1` and `0.2.0-rc.2` are deprecated since 2026-10-10; `0.2.0` and `0.2.0-rc.3` are not).
A candidate is never announced. It takes about an hour, most of it waiting for builds and the registry. The commands are for Git Bash (or any POSIX shell: the `for` loops are shell); the `gh` and `npm`
commands work in PowerShell too, and `^{commit}` needs quotes there.

**Who does what.** **The supervisor** (the session that prepares and verifies the release) creates and pushes the tag, dispatches the dry run and the publishing run, compares the two plans, checks the result from
outside and dispatches the verification workflows; the `release tags` ruleset lets only the repository's administrator create a `v*` tag, so the tag is pushed from that account's login (6.2 step 4). **The
maintainer** merges the pull request, confirms the one setting no tool can read (the trusted publishers, step 0.6), **approves the `release` environment's deployment** (step 3, the only approval the publish needs),
and decides everything marked "open" in [`release-0.2.1.md`](../planning/release-0.2.1.md): the agent-trial batches, the waivers, the deprecations. Nothing here is announced.

**What differs from 6.12, in one table.**

| | Third candidate (6.12, 2026-10-08) | This candidate (6.14) |
| --- | --- | --- |
| What npm holds before the publish | `next` `0.2.0-rc.2`, **`latest` `0.0.0`** | `next` **`0.2.0-rc.3`**, **`latest` `0.2.0`**, on all four packages |
| What the dry run's plan does to `next` and `latest` | moves `next` `0.2.0-rc.2` to `0.2.0-rc.3`; `latest` stays `0.0.0` | moves `next` **`0.2.0-rc.3` to `0.2.1-rc.1`**; `latest` stays **`0.2.0`** |
| What a plain `npm install vsift-cli` installs, before and after | the empty `0.0.0` placeholder | **`0.2.0`**, before and after: a candidate never changes it |
| What GitHub calls the latest release (`releases/latest`) | none: `gh: Not Found (HTTP 404)` | **`v0.2.0`**, before and after |
| The tag and the commit it names | `v0.2.0-rc.3` at the merge commit of the pull request titled "P14 PR 10 repeated again" | `v0.2.1-rc.1` at the merge commit of the pull request whose title starts "P14-style cut: the release candidate 0.2.1-rc.1" |
| What `vsift --version` prints | `vsift 0.2.0-rc.3 (<12 digits>)` | `vsift 0.2.1-rc.1 (<the first 12 digits of that commit>)` |
| The earlier bytes to write down | the first two candidates' (eight integrity values) | **all four earlier versions'** (`0.2.0-rc.1`, `0.2.0-rc.2`, `0.2.0-rc.3` and `0.2.0`: sixteen values) and their four tag commits |
| The agent-trial freeze | new (the skill's two rules) | **new again** (the skill changed): `skill` `2f8686b7...`, whole freeze `dbc4c22c...`; the other six digests are the third candidate's (step 6) |
| The upgrade baselines (`from_version`) | `0.1.0` and `0.2.0-rc.2` | **`0.2.0` first** (the upgrade a person on `latest` does), **then `0.1.0`**; `0.2.0-rc.3` (a person on `next`) is optional |
| Who runs the steps | the maintainer | the supervisor, with the maintainer's approval of the `release` environment (above) |
| The candidate the stable release is compared with | `v0.2.0-rc.3` | **`v0.2.1-rc.1`**, the highest `v0.2.1-rc.N` tag (6.8) |
| `release-evidence --complete-for` | fails on 16 of the 20 items | fails on **most items** (step 0.5): **do not run it as a gate** |

**0. Before the tag (about fifteen minutes; only item 6 can change anything, and the maintainer does it).**

1. **Merge the pull request only when the supervisor can tag and publish at once.** Repository pages name `0.2.1-rc.1` before it is on npm (the guide's first page, the changelog, the skill guide, the trial documents, the plan
   and this runbook); the README, the installation guide, the launcher's README and the security policy are worded so that they are true both before and after the publish (the installation guide's and the launcher's
   README's "it names `0.2.0-rc.3` ... or a later candidate or release" is true whether `next` names the third candidate or this one), which is why this cut opens a smaller window than
   [L-133](../planning/known-limits.md#l-133) describes. Merge nothing else: from the moment the tag exists until the stable release is published **nothing may change on `main` but the work record, the
   installation guide and the launcher's README** (6.8 lists them exactly), and the same holds from this merge to the tag. No Dependabot pull request, no workflow edit, no dependency bump, no change to a
   tool or to an operator document: the cure for any of them is a second candidate (`0.2.1-rc.2`). Dependabot opens new pull requests on Mondays: leave them open.
2. **The "Require branches to be up to date before merging" rule on `main` is on before the tag.** `gh api repos/smormah/vsift/branches/main/protection --jq .required_status_checks.strict` prints `true`.
3. **The commit to tag** is the merge commit of that pull request. Find it and write the full 40 digits down:

   ```console
   git fetch origin --tags
   gh pr list --repo smormah/vsift --state merged --search "release candidate 0.2.1-rc.1" --json number,title,mergeCommit --jq ".[] | {number, title, commit: .mergeCommit.oid}"
   git show <the 40-digit commit>:Cargo.toml | grep -m1 "^version"
   ```

   Expect `version = "0.2.1-rc.1"`. The search may list other pull requests that use those words: take the one whose title starts with `P14-style cut: the release candidate 0.2.1-rc.1`. If `main` has moved on since
   only by work-record pull requests, the commit to tag is still that pull request's merge commit, not `main`'s head.
4. **The commit is on `main` and every check of it passed** (the Release workflow builds and plans but runs no tests, and nothing else checks that a tagged commit was merged or tested):

   ```console
   git merge-base --is-ancestor <the 40-digit commit> origin/main && echo "on main"
   gh api "repos/smormah/vsift/commits/<the 40-digit commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   Expect `on main` and **no output at all** from the second command (`Attest build provenance`, `Publish to npm ...` and the fuzz matrix show as skipped on a pull request, which is fine).
5. **The checks that bind this commit pass**, from a checkout of it (about a minute each, after the first build):

   ```console
   cargo run --locked -p vsift-governance -- check
   cargo run --locked -p vsift-governance -- public-claims
   cargo run --locked -p vsift-governance -- release-evidence
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-2/freeze.json
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-3/freeze.json
   ```

   Expect "VSift delivery ledger is valid.", "VSift public claims agree with the evidence ledger (this proves recorded evidence and absent banned words, not that a sentence is true).", "VSift release
   evidence ledger is valid." and `nothing frozen has changed` twice. Do **not** run `release-evidence --complete-for 0.2.1-rc.1` as a gate: the ledger holds nothing for this candidate, so it names most of the 20
   items on purpose ([`release-0.2.1.md`](../planning/release-0.2.1.md) lists them). Any other answer: do not tag.
6. **The trusted publishers are still saved** on npmjs.com for all four packages (6.2 step 6: owner `smormah`, repository `vsift`, workflow `release.yml`, environment `release`, **npm publish** allowed). **The
   maintainer confirms this**: the setting is visible only when signed in, and no dry run can check it (L-100). Three candidates (`--tag next`) and the stable release (`--tag latest`) went through it, so it is
   expected to hold. The supervisor reads the `release` environment and the tag ruleset (read-only):

   ```console
   gh api repos/smormah/vsift/environments/release --jq '{reviewers: [.protection_rules[]? | .reviewers[]? | .type], branch_policy: .deployment_branch_policy, can_admins_bypass: .can_admins_bypass}'
   gh api repos/smormah/vsift/rulesets --jq '.[] | {name, target, enforcement}'
   ```

   Expect `{"branch_policy":{"custom_branch_policies":true,"protected_branches":false},"can_admins_bypass":false,"reviewers":["User"]}` and `{"enforcement":"active","name":"release tags","target":"tag"}`.
7. **What npm and GitHub hold now**, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`):

   ```console
   npm view vsift-cli dist-tags
   npm view @vsift/win32-x64 dist-tags
   npm view @vsift/darwin-arm64 dist-tags
   npm view @vsift/linux-x64 dist-tags
   npm view vsift-cli version
   gh api repos/smormah/vsift/releases/latest --jq .tag_name
   ```

   Expect **`{ latest: '0.2.0', next: '0.2.0-rc.3' }` four times**, `0.2.0` from the fifth command (what a plain install gets) and `v0.2.0` from the last. Anything else (`next` on another version, `latest`
   not `0.2.0`) means something was published or moved since 2026-10-10: stop. Then write down what the earlier versions' bytes and tags are, so that step 4 can show they are untouched:

   ```console
   for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
   for t in v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0; do git show --no-patch --format="$t %H" "$t^{commit}"; done
   ```

   Sixteen integrity values (a deprecated version still prints its integrity; `0.2.0-rc.1` and `0.2.0-rc.2` are deprecated), and the four commits must be `d5792ce31db1106934233c86d0518c3ad1961e07`, `7c722d1fc46af7fddeffbaf807028eaec413ace1`,
   `83dca856e7a00fc9a71c87baae99f0b1d401dd31` and `eeb2a22a46a85ab10a456f2ab5d6a62e292836c5`.

**1. Tag the commit** (the supervisor, from the administrator's login):

```console
git tag -a v0.2.1-rc.1 -m "VSift 0.2.1-rc.1" <the 40-digit commit>
git push origin v0.2.1-rc.1
git show --no-patch --format="%H" v0.2.1-rc.1^{commit}
```

The last line prints the commit written down. A wrong tag **before anything is published** may be moved (the administrator can bypass the ruleset): `git push origin :refs/tags/v0.2.1-rc.1`, fix, push again.
**Never move or delete it after step 3.**

**2. Dry run on the tag, and read its plan before anything is published** (the supervisor). The dry run builds the real bytes and shows exactly what a publish will do; for `0.2.0-rc.1`, the second candidate and `0.2.0`
the dry run and the publish run wrote byte-identical checksum files, and step 3 compares them. Actions, **Release**, **Run workflow**, "Use workflow from" **Tags: v0.2.1-rc.1**, **dry_run** ticked. Or:

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.1-rc.1 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --event workflow_dispatch --limit 3 --json databaseId,headBranch,headSha,status,conclusion
gh run watch <the run id> --repo smormah/vsift
```

Pick the run by what it says, not by position: its `headBranch` must be `v0.2.1-rc.1` and its `headSha` the commit written down. It takes about 20 to 40 minutes: 21 jobs, `attest` and `publish`
skipped. Open the run's **Publish plan** summary and read it top to bottom. Expect:

- the heading `Publish plan: dry run, nothing is published` and "This run was dispatched with `dry_run` set";
- `` `latest` is not touched. This release candidate is published under `next` only ``;
- "Version `0.2.1-rc.1` (release candidate), npm dist-tag `next`, Git tag `v0.2.1-rc.1`", the commit written down, `workflow_dispatch` on `refs/tags/v0.2.1-rc.1` and "The plan is enforced";
- the table "What this publication does to the dist-tags": for each of the four packages **`next` from `0.2.0-rc.3` to `0.2.1-rc.1`** and **`latest` staying `0.2.0`** (**not `0.0.0`**; a table that shows `not read`
  means the plan could not read the registry, and the guard below is then advisory only: stop and find out why before the approval);
- two guards, both **passed**: **Pre-release version** and **Registry** ("this version is not `latest` and is not on npm with other bytes on any package");
- four `npm publish ... --tag next --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`, and a GitHub release created as a draft
  **pre-release not marked latest** with ten assets.

Then download the plan and keep it, and **write down the dry run's id** (it is part of the record):

```console
gh run download <the dry run's id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

Read `plan-dry/release-notes.md`: it is what the GitHub release will say ("VSift 0.2.1-rc.1 is a release candidate. It is under qualification, it is not announced, and it is no statement of support or
stability ..."), and its last line links the installation guide at the tag. **Keep `plan-dry/attestation-subjects.sha256` and `plan-dry/release-assets.sha256`.** If anything differs from this list, or the run
fails, nothing was published: fix it (6.5, "The plan job fails on the tag"), and repeat from step 1 if the commit has to change.

**3. Publish** (the supervisor dispatches; the maintainer approves). The same dispatch with **dry_run cleared** (`-f dry_run=false`) on the same tag. The plan job must say `Publish plan: PUBLISH a release
candidate after the release environment's approval`; `attest` runs (about a minute; **its attestations are public and permanent from the moment it finishes**, before the approval: repeating is harmless because
the builds are reproducible, 6.5); `publish` stops at **Waiting for review**. Download this run's `publish-plan` (`--dir plan-publish`), **write down this run's id too**, and compare **before the approval**:

```console
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

Then **the maintainer** opens **Review deployments**, ticks `release` and chooses **Approve and deploy**, within a week (the run's files expire after seven days, 6.3 step 4). The `publish` job publishes the four
packages (about three minutes), waits up to five minutes for `next` to read back as `0.2.1-rc.1` on all four, **checks that `latest` is where it was (`0.2.0`)**, and creates and publishes the GitHub
pre-release. A message from npm that a package "is being processed and may take a few minutes to become available" is normal. **After `npm publish` has run for the first package, only the failure table below applies.**

**4. Check it from the outside** (the supervisor; a shell with no npm login and a folder of its own; about ten minutes). Expect, in this order:

```console
npm view vsift-cli dist-tags
npm view @vsift/win32-x64 dist-tags
npm view @vsift/darwin-arm64 dist-tags
npm view @vsift/linux-x64 dist-tags
npm view vsift-cli version
```

`{ latest: '0.2.0', next: '0.2.1-rc.1' }` four times, and `0.2.0` from the last command. **`latest` must still be `0.2.0` and `next` must be `0.2.1-rc.1`.** Anything else on `latest` is the emergency of 6.5
("`latest` moved when it should not have"). The earlier versions must be as they were:

```console
for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
```

The sixteen values equal the ones written down in step 0.7. Then:

```console
mkdir rc-check && cd rc-check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures
npx vsift --version
```

Expect `npm audit signatures` to report verified registry signatures and verified attestations (on Windows 4 and 4; on Linux the two packages it installs), and `vsift 0.2.1-rc.1 (<the first 12 digits of the commit>)`.
A second folder with `npm install vsift-cli` and **no tag** must install `0.2.0` (`npx vsift --version` prints `vsift 0.2.0 (eeb2a22a46a8)`): a candidate does not reach a user who asks for no tag.

```console
gh release view v0.2.1-rc.1 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `isPrerelease` true, `isDraft` false, ten assets; and **`v0.2.0`** from the second command (the release GitHub calls the latest is the one of the stable version, and it was `gh: Not Found (HTTP 404)` for the earlier
candidates only because no stable version existed). If it prints `v0.2.1-rc.1`, the candidate is wrongly marked latest: `gh release edit v0.2.1-rc.1 --repo smormah/vsift --latest=false`, then
`gh release edit v0.2.0 --repo smormah/vsift --latest` and read the second command again. The checksums and attestations of the ten files and four tarballs are 6.4's commands with the version and
`--source-ref refs/tags/v0.2.1-rc.1`; step 5 does them again from a runner that holds no credential, so doing both is optional.

**5. The second verification and the hosted checks of the published bytes** (the supervisor; nothing here can publish; each is one dispatch, from `main`). Three of the four workflows are dispatched once:

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.1
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.1
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.1-rc.1
```

`P14 published artifacts` is dispatched once for each upgrade baseline, **one after another** (the second only after the first has finished: runs of one workflow from `main` share a concurrency group, one runs,
one may wait, and a newer waiting run replaces an older one without a word):

```console
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.1 -f from_version=0.2.0
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.1 -f from_version=0.1.0
```

`P14 verify release` (RQ-19) takes a few minutes and must be all green: the provenance of four packages, `npm audit signatures`, `gh attestation verify` of ten files and four tarballs, the checksums, and the
release's flags. **For this candidate it reads the dist-tags as they are:** each package's detail line must say `next 0.2.1-rc.1, latest 0.2.0`, and the release-flags check must say `pre-release true, GitHub's latest
release v0.2.0` (the tool takes the registry's own `latest`, not the placeholder; `tools/p14-published/test/verify.test.cjs` holds both for this case). A pre-release is asked for neither of the two checks of a stable version. **Which upgrades
matter:** `0.2.0` to `0.2.1-rc.1` is the upgrade a person on `latest` does (it reads back a session, a retained bundle and a configuration that `0.2.0` wrote; it produces no transcription gap, so it says
nothing about the rollback caveat of #353); `0.1.0` to `0.2.1-rc.1` is the baseline every candidate has been measured from. **`0.2.0-rc.3` to `0.2.1-rc.1` is optional** (the same program as `0.2.0`, so it is expected to add nothing; say whether it was run). The
rest of the evidence (the source-built campaigns at the tag, agent-trial batches 2 and 3, the scan reading, the try-outs) is in [`release-0.2.1.md`](../planning/release-0.2.1.md); the supervisor dispatches the
hosted ones and the maintainer starts the others, each on an explicit go. **A failed run is a finding: open an issue before re-running** (governance rule 14).

**6. Afterwards.** The supervisor tells the maintainer: **the run ids of the dry run and of the publish**, the `npm view` answers, the output of `npm audit signatures`, the result of the `diff` of step 3, which upgrade
baselines were dispatched, and anything that failed or was re-run. Nothing is announced. **Batches 2 and 3 of the agent trials start only on the maintainer's explicit go** (`docs/agents/trials.md`, "The P14
batches"; the checklist is [`p14-batch-2-3-checklist.md`](../planning/p14-batch-2-3-checklist.md)) and only after the candidate is published. **Their freeze is new for this candidate**
(`docs/planning/p14-agent-trials/batch-2/freeze.json` and `batch-3/freeze.json`): the skill changed on purpose, so the `skill` digest (`2f8686b7...`) and the whole-freeze digest (`dbc4c22c...`) differ from the third
candidate's, and the grader, the scenarios, the cold scenarios, the hold-outs, the settings and the corpus truth are byte-identical to theirs. `prepare` refuses a trial if anything frozen differs from the file. `batch-2/`
and `batch-3/` hold only that file: the third candidate's records are in `batch-2-rc.3/` and `batch-3-rc.3/` (and the first two candidates' batch 2 in `batch-2-rc.1/` and `batch-2-rc.2/`), and a batch runs against one version,
so do not copy their state files back. Batch 3 still takes `-AllowGraderChange`, for the reason the checklist gives.

**7. Deprecation: nothing is planned, and it is the maintainer's open decision.** For `0.2.0` the maintainer decided on 2026-10-09 to deprecate the first two candidates at the stable release, not before, and did so on
2026-10-10 (6.13). Whether `0.2.1-rc.1` (and `0.2.0-rc.3`) are deprecated when `0.2.1` is published is not decided; do not deprecate a candidate before the runs that install it by its exact version have finished
(a deprecated version prints a warning on install, and the upgrade job installs by number). An empty message (`npm deprecate "vsift-cli@0.2.1-rc.1" ""`) removes a deprecation.

**If something fails.**

| When | What it means | What to do |
| --- | --- | --- |
| Step 0 fails (a failing check run, not on `main`, a governance or freeze answer, `next` or `latest` not as stated) | the commit or the registry is not what was reviewed | do not tag; tell the maintainer |
| Step 2 fails, is refused, or the plan differs from the list | nothing was attested or published | read "Refused because:", fix it, move the tag if the commit changes (nothing was published from it) |
| Step 2: the plan's `next` moves from something other than `0.2.0-rc.3`, or its `latest` is not `0.2.0` | someone published or moved something | do not publish; tell the maintainer |
| Step 2: the **Registry** guard fails ("`0.2.1-rc.1` is already on npm, and not with these bytes") | the version was published before with other bytes: the number is used for good | do not publish; the next candidate is `0.2.1-rc.2` and needs its own cut |
| Step 3: the two `diff`s differ | the bytes are not the dry run's | do not approve; open an issue; the run stays unapproved and expires |
| Step 3: `publish` fails with `ENEEDAUTH` before any `+ package@version` line | a trusted publisher is not saved or is wrong | 6.5 first bullet of `ENEEDAUTH`; nothing was published |
| Step 3: `publish` fails after some packages | a partial publish ([L-097](../planning/known-limits.md#l-097)) | **Re-run failed jobs** on the same run within seven days and approve again: versions npm holds with the same bytes are skipped |
| Step 4: `next` is not `0.2.1-rc.1` after ten minutes | the registry is slow, or the publish stopped | look at the `publish` job's log before doing anything |
| Step 4: `latest` is not `0.2.0` | the workflow is wrong | 6.5 "`latest` moved when it should not have": `npm dist-tag add <package>@0.2.0 latest` for each package that moved (the maintainer's login and second factor), then open an issue; **do not dispatch again** |
| Step 4: GitHub's latest release is not `v0.2.0` | the release was marked latest | the two `gh release edit` commands of step 4 |
| Step 4: an earlier version's integrity value changed | impossible by npm's rules; it would mean the reading was of another package | stop |
| Step 5: an upgrade run you dispatched is gone from the list | a later dispatch replaced it while it waited | dispatch it again, after the running one has finished; nothing was lost but time |
| A published version is bad | the candidate itself is wrong | **never unpublish**: `npm deprecate` it on all four packages, edit the release's notes, and cut `0.2.1-rc.2` by this section again |

**What is irreversible, in one list.** The four published `0.2.1-rc.1` versions and their provenance records on npm's transparency log; the Sigstore attestations of the ten files and four tarballs (they exist from
the end of `attest`, before the approval); the tag once anything was published from it; and the sixteen versions of `0.2.0-rc.1` to `0.2.0`, which this section does not change. **Policy, not
impossibility:** GitHub lets you delete a release page or move a tag, and this runbook says never (6.5), because the page, the tag and the attestations name each other. **What is reversible:** `next`
(`npm dist-tag add vsift-cli@0.2.0-rc.3 next` for each package, with the maintainer's login and second factor; the workflow never does it), the notes, and a deprecation.

### 6.15 The release candidate `0.2.1-rc.2`: the exact steps (the patch release's second candidate, cut 2026-10-11)

**A record (2026-10-13).** Followed once, on 2026-10-11: the tag `v0.2.1-rc.2` is the commit `23ad4927e56f11d8047ca54d59b892516812d8f8` (the merge of #385); the dry run was run 38134946959 and the publishing run 38135772111; `next` moved from `0.2.1-rc.1` to `0.2.1-rc.2` on all
four packages, and `latest` stayed `0.2.0` (GitHub's latest release stayed `v0.2.0`). The publish and the checks from outside matched this section. Five hosted checks were run on the published bytes the same day, all green, and none is recorded in the ledger or carries to a later
version: `P14 verify release` (run 38139064324), `P14 journeys` (38139202117), `P13 managed smoke` (38139337654) and `P14 published artifacts` from `0.2.0` (38139472845) and from `0.1.0` (38139753709). **Superseded by `0.2.1-rc.3`** (6.16), which the maintainer decided on 2026-10-13 to cut for the
`setup check` defect of #391; **`0.2.1-rc.2` was deprecated on npm on 2026-10-15** (\"Superseded by 0.2.1. Install vsift-cli.\"). Kept as what was done. It is a record and is not to be followed again.

**Status when written: cut, not tagged, not published.** This section was written before the candidate existed on npm; nothing in it had been run. It is 6.14 with the numbers of this candidate, and with
what is different because **`0.2.1-rc.1` is already published and is superseded by this candidate**: before the publish `next` names `0.2.1-rc.1`, not `0.2.0-rc.3`, and `latest` is `0.2.0`
on all four packages and stays there. A candidate still moves `next` and nothing else, so **`latest` must stay `0.2.0` on all four packages at every step, and GitHub's own "latest release" must stay `v0.2.0`**.
6.14 is the record of the first candidate's publish and was followed once; this section was the one to follow for the second. What that run showed about 6.14's steps is not recorded here: whoever ran it should check this section against what they did before it is used.
**One thing in 6.14 was wrong and is corrected here:** its step 0.3 told the supervisor to take the pull request "whose title starts with `P14-style cut: ...`", but the cut's pull request was titled "The release candidate 0.2.1-rc.1: ..."
(the squash commit's subject is that title; step 0.3 below searches for the title the cut's pull request has and checks the version in `Cargo.toml` of the commit it finds). **One number changes:** the sixteen earlier integrity values
and four earlier tag commits that 6.14 asked for are twenty values and five commits here, because `0.2.1-rc.1` joins them. Sections 6.10 to 6.14 are records of past publishes; do not follow them for this candidate.

**Why there is a second candidate.** The maintainer's live testing on two real recordings, the day after `0.2.0` and while `0.2.1-rc.1` waited for its checks, found a second serious transcription defect, a silent one: merging the overlapping
30-second pieces can drop speech at a seam while the result still says the whole range was transcribed ([#359](https://github.com/smormah/vsift/issues/359), L-149). The maintainer decided on
2026-10-11 to put that fix and the larger findings of the testing into `0.2.1`, that the version number stays `0.2.1`, and that `0.2.1-rc.1` is superseded, so its hosted evidence round was stopped and none of it is recorded.
**`0.2.1-rc.2` is `0.2.1-rc.1` plus exactly this and the version bump** (the plan for the release is [`release-0.2.1.md`](../planning/release-0.2.1.md)):

| Change | Pull request | What it is |
| --- | --- | --- |
| Merging chunks can no longer drop speech at a seam; what no segment covers is reported | #377 (#359) | a guard on the union of the chunks' segments (half a second); `local_asr.unplaced_ranges` lists where words that no segment holds may lie; `search` and `transcript get` call such a stretch not transcribed; optional new stored member, and **a revision that holds it is read as damaged by `0.2.0`, `0.2.1-rc.1` and earlier** ([L-160](../planning/known-limits.md#l-160)); limits [L-155](../planning/known-limits.md#l-155) to [L-159](../planning/known-limits.md#l-159) |
| `ingest` of a path that names no file, a folder or a file that may not be read says so; a superseded segment id still resolves | #378 (#369, #372) | three fixed remediation texts for `STORAGE_IO`, the code and exit status unchanged; a test, no behaviour change, for #372 |
| A brief view of a read, and the coverage of `transcript get` | #379 (#365, #367, #373) | `--view full\|brief` on `transcript get`, `search` and `candidates` (the default is the answer every release gave); `transcript_coverage` always on `transcript get`, which is `partial` over a part nobody transcribed, as `search` is |
| The agent skill, second change | this cut | the brief view, the coverage of a read, five-minute `candidates` ranges, `BUSY` while a transcription runs, the newest ids after a range retranscription, a long report checked in two parts, `session list` paging, and whole recordings out of scope (`skills/vsift`, `docs/agents/skill.md`) |
| The work record | #375, #376 and this cut | register entries L-149 to L-160, the design of the larger findings, the plan, the changelog |

Nothing else: no FFmpeg or whisper.cpp re-pin ([#351](https://github.com/smormah/vsift/issues/351), L-132, L-137, L-144), no dependency change (`Cargo.lock` differs from `0.2.1-rc.1`'s only in the version text of the eight workspace crates), no
change to any of #337, #325, #334 or #363, and nothing for #366 (a report that cannot be given on the command line), #367 beyond the brief view (no filter, progress events or parallel windows), #368 (out of scope for this line), #370 or #371
beyond the skill's words. Against the tag `v0.2.1-rc.1` (counted from the cut's tree): **62 files under `crates/` (32 source, 30 test and snapshot files; +9,707, -352), 23 under `schemas/v1/`** (new members, examples and descriptions), the skill
(six files and its fuzz seed copy), one fuzz seed and the fuzz replay test, one test of the trial harness (the freeze's pin), the guide, the work record and the five version-string files. Against `v0.2.0`: **89 files under `crates/` (42 source, 47 test and snapshot files; +16,116, -787), ten under
`tools/`, 29 under `schemas/v1/`**, the skill, one workflow (`.github/workflows/p14-verify-release.yml`, a hosted verification workflow, not the Release workflow) and the five version-string files. **Nothing under `tools/vsift-release/`, `tools/vsift-governance/`,
`tools/vsift-agent-trials/src`, `fixtures/`, `deny.toml`, `rust-toolchain.toml` or `.github/workflows/release.yml` differs from `v0.2.0` or from `v0.2.1-rc.1`** (checked with `git diff --numstat` for this cut: no line).

**What this candidate carries over from `0.2.1-rc.1`, `0.2.0` and its candidates: nothing as evidence.** The ledger holds nothing for any `0.2.1` candidate. `vsift-governance release-evidence --complete-for 0.2.1-rc.2` (run for this cut, **not a gate**) names 15 of the 20
items, **RQ-01 to RQ-09, RQ-11, RQ-12, RQ-13, RQ-15, RQ-18 and RQ-19**: RQ-01 to RQ-06, RQ-13, RQ-18 and RQ-19 are `passed` for `0.2.0`, RQ-07 to RQ-09, RQ-11, RQ-12 and RQ-15 for `0.2.0-rc.3`, and between 90 and 314
files in each item's scope changed since (the staleness rule of the ledger: an entry counts for a later version only if nothing in its item's scope changed). **It does not name RQ-10, RQ-14, RQ-16, RQ-17 or RQ-20**: the four waived items carry
no `applies_to`, so the check passes them for any version without a decision, and the decisions are the maintainer's and still open ([`release-0.2.1.md`](../planning/release-0.2.1.md), "The waivers"). `vsift-release candidate-delta` was not run for this cut:
it compares a **stable** commit with its accepted candidate and refuses a pre-release tree ("`0.2.1-rc.2` has a pre-release suffix, so it has no release candidate to compare with"); the counts above are `git diff` counts. **A candidate writes no `release_delta`.** The
plan for the stable release (`0.2.1`) writes it (6.7, 6.8), comparing the stable commit with the highest `v0.2.1-rc.N` tag, which is `v0.2.1-rc.2` once this candidate is tagged, and the ledger copies it in so that this candidate's evidence counts for the release.
That is why the evidence below is recorded for `0.2.1-rc.2`'s own bytes and commit, once each.

**`0.2.0`, `0.2.0-rc.1`, `0.2.0-rc.2`, `0.2.0-rc.3` and `0.2.1-rc.1` stay published and tagged and no step below touches them** (`0.2.0-rc.1` and `0.2.0-rc.2` are deprecated since 2026-10-10; `0.2.0`, `0.2.0-rc.3` and `0.2.1-rc.1` are not).
A candidate is never announced. It takes about an hour, most of it waiting for builds and the registry. The commands are for Git Bash (or any POSIX shell: the `for` loops are shell); the `gh` and `npm`
commands work in PowerShell too, and `^{commit}` needs quotes there.

**Who does what.** **The supervisor** (the session that prepares and verifies the release) creates and pushes the tag, dispatches the dry run and the publishing run, compares the two plans, checks the result from
outside and dispatches the verification workflows; the `release tags` ruleset lets only the repository's administrator create a `v*` tag, so the tag is pushed from that account's login (6.2 step 4). **The
maintainer** merges the pull request, confirms the one setting no tool can read (the trusted publishers, step 0.6), **approves the `release` environment's deployment** (step 3, the only approval the publish needs),
and decides everything marked "open" in [`release-0.2.1.md`](../planning/release-0.2.1.md): the agent-trial batches, the four waivers, the deprecations. Nothing here is announced.

**What differs from 6.14, in one table.**

| | First candidate (6.14, 2026-10-10) | This candidate (6.15) |
| --- | --- | --- |
| What npm holds before the publish | `next` `0.2.0-rc.3`, `latest` `0.2.0` | `next` **`0.2.1-rc.1`**, `latest` `0.2.0`, on all four packages |
| What the dry run's plan does to `next` and `latest` | moves `next` `0.2.0-rc.3` to `0.2.1-rc.1`; `latest` stays `0.2.0` | moves `next` **`0.2.1-rc.1` to `0.2.1-rc.2`**; `latest` stays `0.2.0` |
| What a plain `npm install vsift-cli` installs, before and after | `0.2.0` | **`0.2.0`**, before and after: a candidate never changes it |
| What GitHub calls the latest release (`releases/latest`) | `v0.2.0` | **`v0.2.0`**, before and after |
| The tag and the commit it names | `v0.2.1-rc.1` at the merge commit of "The release candidate 0.2.1-rc.1: ..." | `v0.2.1-rc.2` at the merge commit of the pull request whose title starts "The release candidate 0.2.1-rc.2" |
| What `vsift --version` prints | `vsift 0.2.1-rc.1 (<12 digits>)` | `vsift 0.2.1-rc.2 (<the first 12 digits of that commit>)` |
| The earlier bytes to write down | four earlier versions' (sixteen values) and their four tag commits | **five earlier versions'** (`0.2.0-rc.1`, `0.2.0-rc.2`, `0.2.0-rc.3`, `0.2.0` and **`0.2.1-rc.1`**: twenty values) and their **five** tag commits |
| The agent-trial freeze | new (the skill changed) | **new again** (the skill changed again): `skill` `256b4840...`, whole freeze `a48567ac...`; the other six digests are the third and fourth candidates' (step 6) |
| The upgrade baselines (`from_version`) | `0.2.0` first, then `0.1.0`; `0.2.0-rc.3` optional | **`0.2.0` first** (the upgrade a person on `latest` does), **then `0.1.0`**; `0.2.1-rc.1` (a person on `next`) and `0.2.0-rc.3` are optional |
| What this candidate supersedes | nothing: `next` held `0.2.0-rc.3`, the stable release's own source | **`0.2.1-rc.1`** (it has L-149 and the first skill): it stays published and under `next`'s history, and nothing deprecates it here (step 7) |
| The candidate the stable release is compared with | `v0.2.1-rc.1` | **`v0.2.1-rc.2`**, the highest `v0.2.1-rc.N` tag (6.8) |
| `release-evidence --complete-for` | fails on most items | fails on **15 of the 20** (above): **do not run it as a gate** |

**0. Before the tag (about fifteen minutes; only item 6 can change anything, and the maintainer does it).**

1. **Merge the pull request only when the supervisor can tag and publish at once.** Repository pages name `0.2.1-rc.2` before it is on npm (the guide's first page, the changelog, the skill guide, the trial documents, the plan, the register's entries L-108 and L-133,
   the work record and this runbook); the README, the installation guide, the launcher's README and the security policy are worded so that they are true both before and after the publish (their "`0.2.1-rc.1`" is a statement about which
   versions have the known issue, and stays true), which is why this cut opens the same small window as the first candidate's ([L-133](../planning/known-limits.md#l-133)). Merge nothing else: from the moment the tag exists until the stable release is published **nothing may change on `main` but the work record, the
   installation guide and the launcher's README** (6.8 lists them exactly), and the same holds from this merge to the tag. No Dependabot pull request, no workflow edit, no dependency bump, no change to a
   tool or to an operator document: the cure for any of them is a third candidate (`0.2.1-rc.3`). Dependabot opens new pull requests on Mondays: leave them open.
2. **The "Require branches to be up to date before merging" rule on `main` is on before the tag.** `gh api repos/smormah/vsift/branches/main/protection --jq .required_status_checks.strict` prints `true`.
3. **The commit to tag** is the merge commit of that pull request. Find it and write the full 40 digits down:

   ```console
   git fetch origin --tags
   gh pr list --repo smormah/vsift --state merged --search "release candidate 0.2.1-rc.2" --json number,title,mergeCommit --jq ".[] | {number, title, commit: .mergeCommit.oid}"
   git show <the 40-digit commit>:Cargo.toml | grep -m1 "^version"
   ```

   Expect `version = "0.2.1-rc.2"`. The search may list other pull requests that use those words (the record-only ones that name the candidate): take the one whose title starts with `The release candidate 0.2.1-rc.2` and whose commit shows that version. If `main` has moved on since
   only by work-record pull requests, the commit to tag is still that pull request's merge commit, not `main`'s head.
4. **The commit is on `main` and every check of it passed** (the Release workflow builds and plans but runs no tests, and nothing else checks that a tagged commit was merged or tested):

   ```console
   git merge-base --is-ancestor <the 40-digit commit> origin/main && echo "on main"
   gh api "repos/smormah/vsift/commits/<the 40-digit commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   Expect `on main` and **no output at all** from the second command (`Attest build provenance`, `Publish to npm ...` and the fuzz matrix show as skipped on a pull request, which is fine).
5. **The checks that bind this commit pass**, from a checkout of it (about a minute each, after the first build):

   ```console
   cargo run --locked -p vsift-governance -- check
   cargo run --locked -p vsift-governance -- public-claims
   cargo run --locked -p vsift-governance -- release-evidence
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-2/freeze.json
   cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-3/freeze.json
   ```

   Expect "VSift delivery ledger is valid.", "VSift public claims agree with the evidence ledger (this proves recorded evidence and absent banned words, not that a sentence is true).", "VSift release
   evidence ledger is valid." and `nothing frozen has changed` twice. Do **not** run `release-evidence --complete-for 0.2.1-rc.2` as a gate: the ledger holds nothing for this candidate, so it names 15 of the 20
   items on purpose (above). Any other answer: do not tag.
6. **The trusted publishers are still saved** on npmjs.com for all four packages (6.2 step 6: owner `smormah`, repository `vsift`, workflow `release.yml`, environment `release`, **npm publish** allowed). **The
   maintainer confirms this**: the setting is visible only when signed in, and no dry run can check it (L-100). Four candidates (`--tag next`) and the stable release (`--tag latest`) went through it, so it is
   expected to hold. The supervisor reads the `release` environment and the tag ruleset (read-only):

   ```console
   gh api repos/smormah/vsift/environments/release --jq '{reviewers: [.protection_rules[]? | .reviewers[]? | .type], branch_policy: .deployment_branch_policy, can_admins_bypass: .can_admins_bypass}'
   gh api repos/smormah/vsift/rulesets --jq '.[] | {name, target, enforcement}'
   ```

   Expect `{"branch_policy":{"custom_branch_policies":true,"protected_branches":false},"can_admins_bypass":false,"reviewers":["User"]}` and `{"enforcement":"active","name":"release tags","target":"tag"}`.
7. **What npm and GitHub hold now**, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`):

   ```console
   npm view vsift-cli dist-tags
   npm view @vsift/win32-x64 dist-tags
   npm view @vsift/darwin-arm64 dist-tags
   npm view @vsift/linux-x64 dist-tags
   npm view vsift-cli version
   gh api repos/smormah/vsift/releases/latest --jq .tag_name
   ```

   Expect **`{ latest: '0.2.0', next: '0.2.1-rc.1' }` four times**, `0.2.0` from the fifth command (what a plain install gets) and `v0.2.0` from the last. Anything else (`next` on another version, `latest`
   not `0.2.0`) means something was published or moved since 2026-10-10: stop. Then write down what the earlier versions' bytes and tags are, so that step 4 can show they are untouched. **This is the before-snapshot: keep its output.**

   ```console
   for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
   for t in v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1; do git show --no-patch --format="$t %H" "$t^{commit}"; done
   ```

   **Twenty** integrity values (a deprecated version still prints its integrity; `0.2.0-rc.1` and `0.2.0-rc.2` are deprecated), and the five commits must be `d5792ce31db1106934233c86d0518c3ad1961e07`, `7c722d1fc46af7fddeffbaf807028eaec413ace1`,
   `83dca856e7a00fc9a71c87baae99f0b1d401dd31`, `eeb2a22a46a85ab10a456f2ab5d6a62e292836c5` and `cb8cb3a6286047db56831459dabd6ee5500a37bd`.

**1. Tag the commit** (the supervisor, from the administrator's login):

```console
git tag -a v0.2.1-rc.2 -m "VSift 0.2.1-rc.2" <the 40-digit commit>
git push origin v0.2.1-rc.2
git show --no-patch --format="%H" v0.2.1-rc.2^{commit}
```

The last line prints the commit written down. A wrong tag **before anything is published** may be moved (the administrator can bypass the ruleset): `git push origin :refs/tags/v0.2.1-rc.2`, fix, push again.
**Never move or delete it after step 3.**

**2. Dry run on the tag, and read its plan before anything is published** (the supervisor). The dry run builds the real bytes and shows exactly what a publish will do; for `0.2.0-rc.1`, the second candidate and `0.2.0`
the dry run and the publish run wrote byte-identical checksum files, and step 3 compares them. Actions, **Release**, **Run workflow**, "Use workflow from" **Tags: v0.2.1-rc.2**, **dry_run** ticked. Or:

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.1-rc.2 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --event workflow_dispatch --limit 3 --json databaseId,headBranch,headSha,status,conclusion
gh run watch <the run id> --repo smormah/vsift
```

Pick the run by what it says, not by position: its `headBranch` must be `v0.2.1-rc.2` and its `headSha` the commit written down. It takes about 20 to 40 minutes: 21 jobs, `attest` and `publish`
skipped. Open the run's **Publish plan** summary and read it top to bottom. Expect:

- the heading `Publish plan: dry run, nothing is published` and "This run was dispatched with `dry_run` set";
- `` `latest` is not touched. This release candidate is published under `next` only ``;
- "Version `0.2.1-rc.2` (release candidate), npm dist-tag `next`, Git tag `v0.2.1-rc.2`", the commit written down, `workflow_dispatch` on `refs/tags/v0.2.1-rc.2` and "The plan is enforced";
- the table "What this publication does to the dist-tags": for each of the four packages **`next` from `0.2.1-rc.1` to `0.2.1-rc.2`** and **`latest` staying `0.2.0`** (a table that shows `not read`
  means the plan could not read the registry, and the guard below is then advisory only: stop and find out why before the approval);
- two guards, both **passed**: **Pre-release version** and **Registry** ("this version is not `latest` and is not on npm with other bytes on any package");
- four `npm publish ... --tag next --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`, and a GitHub release created as a draft
  **pre-release not marked latest** with ten assets.

Then download the plan and keep it, and **write down the dry run's id** (it is part of the record):

```console
gh run download <the dry run's id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

Read `plan-dry/release-notes.md`: it is what the GitHub release will say ("VSift 0.2.1-rc.2 is a release candidate. It is under qualification, it is not announced, and it is no statement of support or
stability ..."), and its last line links the installation guide at the tag. **Keep `plan-dry/attestation-subjects.sha256` and `plan-dry/release-assets.sha256`.** If anything differs from this list, or the run
fails, nothing was published: fix it (6.5, "The plan job fails on the tag"), and repeat from step 1 if the commit has to change.

**3. Publish** (the supervisor dispatches; the maintainer approves). The same dispatch with **dry_run cleared** (`-f dry_run=false`) on the same tag, **never on `main`**. The plan job must say `Publish plan: PUBLISH a release
candidate after the release environment's approval`; `attest` runs (about a minute; **its attestations are public and permanent from the moment it finishes**, before the approval: repeating is harmless because
the builds are reproducible, 6.5); `publish` stops at **Waiting for review**. Download this run's `publish-plan` (`--dir plan-publish`), **write down this run's id too**, and compare **before the approval**:

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.1-rc.2 -f dry_run=false
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

Then **the maintainer** opens **Review deployments**, ticks `release` and chooses **Approve and deploy**, within a week (the run's files expire after seven days, 6.3 step 4). The `publish` job publishes the four
packages (about three minutes), waits up to five minutes for `next` to read back as `0.2.1-rc.2` on all four, **checks that `latest` is where it was (`0.2.0`)**, and creates and publishes the GitHub
pre-release. A message from npm that a package "is being processed and may take a few minutes to become available" is normal. **After `npm publish` has run for the first package, only the failure table below applies.**

**4. Check it from the outside** (the supervisor; a shell with no npm login and a folder of its own; about ten minutes). Expect, in this order:

```console
npm view vsift-cli dist-tags
npm view @vsift/win32-x64 dist-tags
npm view @vsift/darwin-arm64 dist-tags
npm view @vsift/linux-x64 dist-tags
npm view vsift-cli version
```

`{ latest: '0.2.0', next: '0.2.1-rc.2' }` four times, and `0.2.0` from the last command. **`latest` must still be `0.2.0` and `next` must be `0.2.1-rc.2`.** Anything else on `latest` is the emergency of 6.5
("`latest` moved when it should not have"). The earlier versions must be as they were, **including the first candidate of this release**:

```console
for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
```

The twenty values equal the ones written down in step 0.7 (`diff` the two outputs). Then:

```console
mkdir rc-check && cd rc-check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures
npx vsift --version
```

Expect `npm audit signatures` to report verified registry signatures and verified attestations (on Windows 4 and 4; on Linux the two packages it installs), and `vsift 0.2.1-rc.2 (<the first 12 digits of the commit>)`.
**The installed skill is this cut's:** from a checkout of the tag, `diff -r skills/vsift rc-check/node_modules/vsift-cli/skills/vsift && echo "the installed skill is the tag's"` prints the second part and nothing else
(`P14 published artifacts` makes the same comparison, in step 5). A second folder with `npm install vsift-cli` and **no tag** must install `0.2.0` (`npx vsift --version` prints `vsift 0.2.0 (eeb2a22a46a8)`): a candidate does not reach a user who asks for no tag.

```console
gh release view v0.2.1-rc.2 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `isPrerelease` true, `isDraft` false, ten assets; and **`v0.2.0`** from the second command (the release GitHub calls the latest is the one of the stable version). If it prints `v0.2.1-rc.2`, the candidate is wrongly marked latest:
`gh release edit v0.2.1-rc.2 --repo smormah/vsift --latest=false`, then `gh release edit v0.2.0 --repo smormah/vsift --latest` and read the second command again. The attestations and checksums of the ten files and four tarballs are 6.4's commands with the version and
`--source-ref refs/tags/v0.2.1-rc.2`, **with `--signer-workflow smormah/vsift/.github/workflows/release.yml` and `--deny-self-hosted-runners`** on every `gh attestation verify`; **the checksums are compared with the plan's**: the SHA-256 of each tarball that `npm pack` of the four
packages at this version writes equals its line in `plan-publish/attestation-subjects.sha256`, and the ten release files pass `sha256sum --check --strict SHA256SUMS` and equal `plan-publish/release-assets.sha256`. Step 5 does them again from a runner that holds no credential, so doing both is optional.

**5. The second verification and the hosted checks of the published bytes** (the supervisor; nothing here can publish; each is one dispatch, from `main`, **once**, on this candidate: nothing from `0.2.1-rc.1`'s stopped round is reused or recorded). Three of the four workflows are dispatched once:

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.2
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.2
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.1-rc.2
```

`P14 published artifacts` is dispatched once for each upgrade baseline, **one after another** (the second only after the first has finished: runs of one workflow from `main` share a concurrency group, one runs,
one may wait, and a newer waiting run replaces an older one without a word):

```console
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.2 -f from_version=0.2.0
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.2 -f from_version=0.1.0
```

`P14 verify release` (RQ-19) takes a few minutes and must be all green: the provenance of four packages, `npm audit signatures`, `gh attestation verify` of ten files and four tarballs, the checksums, and the
release's flags. **For this candidate it reads the dist-tags as they are:** each package's detail line must say `next 0.2.1-rc.2, latest 0.2.0`, and the release-flags check must say `pre-release true, GitHub's latest
release v0.2.0` (the tool takes the registry's own `latest`; `tools/p14-published/test/verify.test.cjs` holds both for a candidate published while a stable version is `latest`). A pre-release is asked for neither of the two checks of a stable version. **Which upgrades
matter:** `0.2.0` to `0.2.1-rc.2` is the upgrade a person on `latest` does (it reads back a session, a retained bundle and a configuration that `0.2.0` wrote); `0.1.0` to `0.2.1-rc.2` is the baseline every candidate has been measured from.
**`0.2.1-rc.1` to `0.2.1-rc.2` is optional** (a person on `next`; it is the one upgrade that crosses the first candidate's skill and the `unplaced_ranges` member, but a session written by rc.1 holds neither, so it is expected to add nothing) and **`0.2.0-rc.3` to `0.2.1-rc.2`** likewise: say whether either was run.
**No upgrade run tests going back**: a revision that holds `unplaced_ranges` (or a gap, since `0.2.1-rc.1`) is read as damaged by an older version (L-160), and nothing in the hosted set tries it.

The rest of the evidence the plan lists is dispatched by the supervisor, **once each, one after another and staggered** (a shortage of hosted runners shows as queueing, #316), with the parameters of the runs for `0.2.0`'s second and third candidates so that the figures compare ([`release-0.2.1.md`](../planning/release-0.2.1.md), "The evidence the candidate needs"):
`P14 compatibility`, `Fuzz` (31 targets) and `P14 stress` at the tag; `P07 local ASR`, `P13 managed power loss` and `P10 durability campaign` at the tag; `P14 load`, `P14 malicious media`, `P14 runbook walk` and `P14 scan reading` from `main` with `version=0.2.1-rc.2`.
The Governance job of the CI run at the candidate's commit is RQ-18's, and it reads RQ-19's status, not its version ([L-133](../planning/known-limits.md#l-133)). **A failed run is a finding: open an issue before re-running** (governance rule 14). Their results
are recorded in the ledger in a work-record pull request (the order of steps in the plan, step 6), from what each run computed, never earlier.

**6. Afterwards.** The supervisor tells the maintainer: **the run ids of the dry run and of the publish**, the `npm view` answers, the output of `npm audit signatures`, the result of the `diff` of step 3 and the twenty-value comparison of step 4, which upgrade
baselines were dispatched, and anything that failed or was re-run. Nothing is announced. **Batches 2 and 3 of the agent trials start only on the maintainer's explicit go** (`docs/agents/trials.md`, "The P14
batches"; the checklist is [`p14-batch-2-3-checklist.md`](../planning/p14-batch-2-3-checklist.md)), one at a time, and only after the candidate is published. **Their freeze is new for this candidate**
(`docs/planning/p14-agent-trials/batch-2/freeze.json` and `batch-3/freeze.json`): the skill changed on purpose, so the `skill` digest (`256b4840...`) and the whole-freeze digest (`a48567ac...`) differ from the fourth
candidate's (`2f8686b7...` and `dbc4c22c...`, which no batch ran under) and the third's, and the grader, the scenarios, the cold scenarios, the hold-outs, the settings and the corpus truth are byte-identical to theirs. `prepare` refuses a trial if anything frozen differs from the file. `batch-2/`
and `batch-3/` hold only that file: the third candidate's records are in `batch-2-rc.3/` and `batch-3-rc.3/` (and the first two candidates' batch 2 in `batch-2-rc.1/` and `batch-2-rc.2/`), and a batch runs against one version,
so do not copy their state files back. Batch 3 still takes `-AllowGraderChange`, for the reason the checklist gives. **The skill's changes are untried and most have no scenario** (`docs/agents/skill.md`, "Untried"): a result of batch 2 says
how the three tiers read the new text on the old scenarios, and nothing about a whole-recording question, a report over 5,500 characters or a `BUSY` frame call.

**7. What this candidate supersedes, and deprecation: nothing is planned, and it is the maintainer's open decision.** `0.2.1-rc.1` stays published, tagged and installable by its exact number; `next` names `0.2.1-rc.2` from the publish on and the history of `next` keeps `0.2.1-rc.1`.
It has L-149 (silent loss of speech at a seam) and the first skill, which is why it is superseded, **not** unsafe to leave: nobody is directed to it (`latest` is `0.2.0`, and no page says to install it). For `0.2.0` the maintainer decided on 2026-10-09 to deprecate the first two candidates at the stable release,
not before, and did so on 2026-10-10 with the message `Superseded by 0.2.0. Install vsift-cli.` (6.12 step 7, 6.13). **Whether `0.2.1-rc.1`, and `0.2.0-rc.3` with it, are deprecated when `0.2.1` is published (or now) is not decided.** Do not deprecate a candidate before the runs
that install it by its exact version have finished (a deprecated version prints a warning on install, and the upgrade job installs by number): that includes the optional upgrade from `0.2.1-rc.1`. An empty message (`npm deprecate "vsift-cli@0.2.1-rc.1" ""`) removes a deprecation. `next` is moved to `0.2.1` after the stable
release only by the maintainer's own command (the workflow never does it; [L-108](../planning/known-limits.md#l-108)).

**If something fails.**

| When | What it means | What to do |
| --- | --- | --- |
| Step 0 fails (a failing check run, not on `main`, a governance or freeze answer, `next` or `latest` not as stated) | the commit or the registry is not what was reviewed | do not tag; tell the maintainer |
| Step 2 fails, is refused, or the plan differs from the list | nothing was attested or published | read "Refused because:", fix it, move the tag if the commit changes (nothing was published from it) |
| Step 2: the plan's `next` moves from something other than `0.2.1-rc.1`, or its `latest` is not `0.2.0` | someone published or moved something | do not publish; tell the maintainer |
| Step 2: the **Registry** guard fails ("`0.2.1-rc.2` is already on npm, and not with these bytes") | the version was published before with other bytes: the number is used for good | do not publish; the next candidate is `0.2.1-rc.3` and needs its own cut |
| Step 3: the two `diff`s differ | the bytes are not the dry run's | do not approve; open an issue; the run stays unapproved and expires |
| Step 3: `publish` fails with `ENEEDAUTH` before any `+ package@version` line | a trusted publisher is not saved or is wrong | 6.5 first bullet of `ENEEDAUTH`; nothing was published |
| Step 3: `publish` fails after some packages | a partial publish ([L-097](../planning/known-limits.md#l-097)) | **Re-run failed jobs** on the same run within seven days and approve again: versions npm holds with the same bytes are skipped |
| Step 4: `next` is not `0.2.1-rc.2` after ten minutes | the registry is slow, or the publish stopped | look at the `publish` job's log before doing anything |
| Step 4: `latest` is not `0.2.0` | the workflow is wrong | 6.5 "`latest` moved when it should not have": `npm dist-tag add <package>@0.2.0 latest` for each package that moved (the maintainer's login and second factor), then open an issue; **do not dispatch again** |
| Step 4: GitHub's latest release is not `v0.2.0` | the release was marked latest | the two `gh release edit` commands of step 4 |
| Step 4: an earlier version's integrity value changed | impossible by npm's rules; it would mean the reading was of another package | stop |
| Step 4: the installed skill differs from the tag's | the package was assembled from other bytes than the tag's | stop; open an issue; do not approve any batch |
| Step 5: an upgrade run you dispatched is gone from the list | a later dispatch replaced it while it waited | dispatch it again, after the running one has finished; nothing was lost but time |
| A published version is bad | the candidate itself is wrong | **never unpublish**: `npm deprecate` it on all four packages, edit the release's notes, and cut `0.2.1-rc.3` by this section again |

**What is irreversible, in one list.** The four published `0.2.1-rc.2` versions and their provenance records on npm's transparency log; the Sigstore attestations of the ten files and four tarballs (they exist from
the end of `attest`, before the approval); the tag once anything was published from it; and the twenty versions of `0.2.0-rc.1` to `0.2.1-rc.1`, which this section does not change. **Policy, not
impossibility:** GitHub lets you delete a release page or move a tag, and this runbook says never (6.5), because the page, the tag and the attestations name each other. **What is reversible:** `next`
(`npm dist-tag add vsift-cli@0.2.1-rc.1 next` for each package, with the maintainer's login and second factor; the workflow never does it), the notes, and a deprecation.

### 6.16 The release candidate `0.2.1-rc.3`: the exact steps (the patch release's third candidate, cut 2026-10-13)

**A record (2026-10-16).** Followed once, on 2026-10-13 and 2026-10-14: the tag `v0.2.1-rc.3` is the commit `fa380021f2aa8db40fdccdb7c410839e8fc10f4c` (the merge of #393); the dry run was run 38455902954 and the publishing run 38457712965; `next` moved from `0.2.1-rc.2` to `0.2.1-rc.3` on all four packages, and `latest` stayed `0.2.0` (GitHub's latest release stayed `v0.2.0`). `P14 verify release` (run 38482041370) passed. The hosted checks and agent batches 2 and 3 were run on its bytes and the evidence is recorded in the ledger ([`release-0.2.1.md`](../planning/release-0.2.1.md), "The evidence on 0.2.1-rc.3"). It is the accepted candidate of the stable release `0.2.1` (6.17), it is still `next`, and it is not deprecated. What else the run showed about its steps (a step that was wrong or is missing) was not recorded. It is a record and is not to be followed again.

**Status when written: cut, not tagged, not published.** This section was written before the candidate existed on npm; nothing in it had been run. **How to use it: follow 6.15's steps 0 to 7, its "If something fails" table and its list of what is irreversible, with the numbers in the table below and the
command lines under "The command lines that change", which are spelled out so that they can be pasted.** Everything 6.15 says that this section does not repeat is unchanged: who does what, the checks of steps 0.2, 0.4 and 0.6, the order of the steps, what the dry run's plan must list, the comparison of the two plans, the
approval and the failure table. 6.15 is the record of the second candidate (its head says how it went). **`0.2.1-rc.2` was already published and is superseded by this candidate:** before the publish `next` names `0.2.1-rc.2`, and `latest` is `0.2.0` on all four packages. A candidate still moves `next` and nothing else, so
**`latest` must stay `0.2.0` on all four packages at every step, and GitHub's own "latest release" must stay `v0.2.0`**. Nothing is announced. Sections 6.10 to 6.15 are records of past publishes; do not follow them for this candidate.

**Why there is a third candidate.** A second Windows 11 machine, running `0.2.1-rc.2` from npm, showed that the first `vsift setup check` after a restart can report an installed, healthy FFmpeg and FFprobe as `[timeout]`, call the setup `blocked` and give the advice for a missing tool ([#391](https://github.com/smormah/vsift/issues/391),
[L-161](../planning/known-limits.md#l-161), medium); every published version has it. The maintainer decided on 2026-10-13 to fix it in `0.2.1`, by a third candidate. **`0.2.1-rc.3` is `0.2.1-rc.2` plus exactly this fix and the version bump** (the plan for the release is [`release-0.2.1.md`](../planning/release-0.2.1.md)):

| Change | Pull request | What it is |
| --- | --- | --- |
| `setup check`: a tool that did not answer in time is not called missing, and the default deadline is 30 seconds | #392 (#391, L-161), merged to `main` as `2e31d88` | the three tool checks share one deadline, 5 seconds before and 30 now (`--timeout-seconds` still takes 1 to 60); a timed-out tool is told what happened and what helps, never "install or locate"; `setup plan` proposes no managed action only because of a timeout; no schema changed; two new example files |
| The version, the pages that name the candidate and the work record | this cut | the version text in the five files, the guide's first page, this runbook, the plan, the changelog, the register's L-108, L-133 and L-161, the trial documents' sentence that the freeze is kept |

Against the tag `v0.2.1-rc.2` (counted from the cut's tree): **13 source files under `crates/` and one test manifest (+1,135, -81), 11 test and snapshot files (+1,384, -35), three files under `schemas/v1/`** (two new examples and the README), the guide, the contract, ADR 0014, the installation guide, the work record and the five version-string files.
**Nothing under `skills/`, `fuzz/seeds/`, `tools/` (so none of the trial harness or its test), `fixtures/`, `.github/`, `deny.toml` or `rust-toolchain.toml` differs from `v0.2.1-rc.2`** (checked with `git diff --stat` for this cut: no line), and `Cargo.lock` and `fuzz/Cargo.lock` differ only in the version text of the workspace crates.

**What this candidate carries over from `0.2.1-rc.2` and every earlier version: nothing as evidence.** The five hosted checks run on `0.2.1-rc.2` on 2026-10-11 (6.15's head) were never recorded and do not count for a program whose `setup check` code differs; the ledger holds nothing for any `0.2.1` candidate, and the agent-trial batches 2 and 3 have not run on any of them. `vsift-governance release-evidence --complete-for 0.2.1-rc.3` (run for this cut, **not a gate**) names the same 15 of the 20 items
as for rc.2 (RQ-01 to RQ-09, RQ-11, RQ-12, RQ-13, RQ-15, RQ-18 and RQ-19, each `passed` for `0.2.0` or `0.2.0-rc.3` and with 105 to 150 files of its scope changed since, RQ-13 and RQ-18 over 330 and over 160) and, as before, does not name the four waived items or RQ-20. `vsift-release candidate-delta` cannot be run for a candidate. The evidence is recorded for `0.2.1-rc.3`'s own bytes and commit, once each.

**What is not in this candidate.** Nothing else of `main` (`main` at `2e31d88`, the merge of #392, is `0.2.1-rc.2` plus that fix); no change to the skill, and so no new agent-trial freeze; no FFmpeg or whisper.cpp re-pin ([#351](https://github.com/smormah/vsift/issues/351)), no dependency change, nothing for the limits that #391's fix leaves ([L-161](../planning/known-limits.md#l-161) lists them), and no fix for any other finding. **#391 is not closed by publishing:** it closes
when the published candidate has been run on the machine that showed it (step 6).

**The numbers that change (read 6.15 with this table).**

| | 6.15 (second candidate, 2026-10-11) | This candidate (6.16) |
| --- | --- | --- |
| The version, the tag | `0.2.1-rc.2`, `v0.2.1-rc.2` | **`0.2.1-rc.3`, `v0.2.1-rc.3`** |
| The commit to tag | the merge commit of the pull request whose title starts "The release candidate 0.2.1-rc.2" | the merge commit of the pull request whose title starts **"The release candidate 0.2.1-rc.3"** |
| What npm holds before the publish | `next` `0.2.1-rc.1`, `latest` `0.2.0` | `next` **`0.2.1-rc.2`**, `latest` `0.2.0`, on all four packages |
| What the dry run's plan does to `next` and `latest` | moves `next` `0.2.1-rc.1` to `0.2.1-rc.2` | moves `next` **`0.2.1-rc.2` to `0.2.1-rc.3`**; `latest` stays `0.2.0` |
| What a plain `npm install vsift-cli` installs, and GitHub's latest release | `0.2.0` and `v0.2.0`, before and after | the same: `0.2.0` and `v0.2.0`, before and after |
| What `vsift --version` prints | `vsift 0.2.1-rc.2 (<12 digits>)` | `vsift 0.2.1-rc.3 (<the first 12 digits of that commit>)` |
| The earlier bytes to write down | five versions' (twenty values) and five tag commits | **six** versions' (`0.2.0-rc.1`, `0.2.0-rc.2`, `0.2.0-rc.3`, `0.2.0`, `0.2.1-rc.1` and **`0.2.1-rc.2`**: **twenty-four** values) and **six** tag commits (the five of 6.15 step 0.7, plus `v0.2.1-rc.2` = `23ad4927e56f11d8047ca54d59b892516812d8f8`) |
| The agent-trial freeze | new (the skill changed): `skill` `256b4840...`, whole freeze `a48567ac...` | **rc.2's, unchanged** (the skill is the same text): `batch-2/freeze.json` and `batch-3/freeze.json` byte for byte, both `freeze check` runs say `nothing frozen has changed`; the batches run against `0.2.1-rc.3` (`-Version 0.2.1-rc.3`) |
| The upgrade baselines (`from_version`) | `0.2.0` first, then `0.1.0`; `0.2.1-rc.1` and `0.2.0-rc.3` optional | **`0.2.0` first, then `0.1.0`**; `0.2.1-rc.2`, `0.2.1-rc.1` and `0.2.0-rc.3` optional |
| What this candidate supersedes | `0.2.1-rc.1` | **`0.2.1-rc.2`, and with it `0.2.1-rc.1`**: both stay published and installable by number; nothing is deprecated here (step 7) |
| The candidate the stable release is compared with | `v0.2.1-rc.2` | **`v0.2.1-rc.3`**, the highest `v0.2.1-rc.N` tag (6.8) |
| `release-evidence --complete-for` | fails on 15 of the 20 | the same 15 of 20: **do not run it as a gate** |
| The next candidate, if this one is bad | `0.2.1-rc.3` | `0.2.1-rc.4`, by this section again |

**The command lines that change** (the rest of 6.15's steps, with no version in them, are as written there).

**Step 0.1, the pages that name `0.2.1-rc.3` before it is on npm:** the guide's first page, the changelog, the skill guide, the trial documents, the plan, the register's entries L-108, L-133 and L-161, the work record and this runbook. The README, the installation guide (its known-issue note says the fix is in the third candidate, which is true both before and after the publish), the launcher's README and the security policy are
worded so that they are true both ways, as for 6.15. Merge nothing else, and from this merge to the tag, and from the tag until the stable release is published, **nothing may change on `main` but the work record, the installation guide and the launcher's README** (6.8): the cure for any other change is a fourth candidate (`0.2.1-rc.4`).

**Step 0.3, the commit to tag:**

```console
git fetch origin --tags
gh pr list --repo smormah/vsift --state merged --search "release candidate 0.2.1-rc.3" --json number,title,mergeCommit --jq ".[] | {number, title, commit: .mergeCommit.oid}"
git show <the 40-digit commit>:Cargo.toml | grep -m1 "^version"
```

Expect `version = "0.2.1-rc.3"`. Take the pull request whose title starts with `The release candidate 0.2.1-rc.3` and whose commit shows that version (the search also lists other pull requests that use those words, among them #392, the fix, merged as `2e31d88`). Step 0.4 (`git merge-base --is-ancestor`, the check runs) and step 0.5 are as written in 6.15, and step 0.5's two `freeze check` runs say `nothing frozen has changed`.

**Step 0.7, what npm and GitHub hold now** (a shell with no npm login): the four `npm view ... dist-tags` commands, `npm view vsift-cli version` and the `releases/latest` command of 6.15 are as written there. Expect **`{ latest: '0.2.0', next: '0.2.1-rc.2' }` four times**, `0.2.0` from the fifth command and `v0.2.0` from the last; anything else means something was published or moved since 2026-10-11: stop. The before-snapshot, whose output is kept:

```console
for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
for t in v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2; do git show --no-patch --format="$t %H" "$t^{commit}"; done
```

**Twenty-four** integrity values (`0.2.0-rc.1` and `0.2.0-rc.2` are deprecated and still print theirs), and the six commits must be `d5792ce31db1106934233c86d0518c3ad1961e07`, `7c722d1fc46af7fddeffbaf807028eaec413ace1`, `83dca856e7a00fc9a71c87baae99f0b1d401dd31`, `eeb2a22a46a85ab10a456f2ab5d6a62e292836c5`,
`cb8cb3a6286047db56831459dabd6ee5500a37bd` and `23ad4927e56f11d8047ca54d59b892516812d8f8`.

**Step 1, tag the commit** (the supervisor, from the administrator's login):

```console
git tag -a v0.2.1-rc.3 -m "VSift 0.2.1-rc.3" <the 40-digit commit>
git push origin v0.2.1-rc.3
git show --no-patch --format="%H" v0.2.1-rc.3^{commit}
```

The last line prints the commit written down. A wrong tag before anything is published may be moved (`git push origin :refs/tags/v0.2.1-rc.3`, fix, push again); never after step 3.

**Step 2, the dry run, and step 3, the publish** (the same two dispatches, on the tag, never on `main`; write down both run ids; the maintainer approves the `release` environment):

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.1-rc.3 -f dry_run=true
gh workflow run release.yml --repo smormah/vsift --ref v0.2.1-rc.3 -f dry_run=false
```

Pick each run by what it says: its `headBranch` must be `v0.2.1-rc.3` and its `headSha` the commit written down. The dry run's **Publish plan** must read "Version `0.2.1-rc.3` (release candidate), npm dist-tag `next`, Git tag `v0.2.1-rc.3`", the commit written down, `workflow_dispatch` on `refs/tags/v0.2.1-rc.3`, and, for each of the four packages, **`next` from `0.2.1-rc.2` to `0.2.1-rc.3` and `latest` staying `0.2.0`**; `plan-dry/release-notes.md`
says "VSift 0.2.1-rc.3 is a release candidate ..."; the two `diff`s of 6.15 step 3 (`plan-dry` against `plan-publish`) print "the same bytes as the dry run" before the approval. The `publish` job waits up to five minutes for `next` to read back as `0.2.1-rc.3` on all four and checks that `latest` is `0.2.0`.

**Step 4, check it from outside** (a shell with no npm login and a folder of its own). The five `npm view` commands of 6.15 step 4 print **`{ latest: '0.2.0', next: '0.2.1-rc.3' }` four times and `0.2.0`**. The earlier versions must be as they were, **including the second candidate**:

```console
for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
mkdir rc-check && cd rc-check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures
npx vsift --version
gh release view v0.2.1-rc.3 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

The twenty-four values equal those of step 0.7 (`diff` the two outputs). `npm audit signatures` reports verified signatures and attestations; `npx vsift --version` prints `vsift 0.2.1-rc.3 (<the first 12 digits of the commit>)`; the installed skill equals the tag's (from a checkout of the tag, `diff -r skills/vsift rc-check/node_modules/vsift-cli/skills/vsift && echo "the installed skill is the tag's"` prints the second part and nothing else); an `npm install vsift-cli` with no tag in a second folder installs `0.2.0` (`vsift 0.2.0 (eeb2a22a46a8)`). `gh release view` prints `isPrerelease` true, `isDraft` false and ten assets, and the last command **`v0.2.0`**; if it prints `v0.2.1-rc.3`: `gh release edit v0.2.1-rc.3 --repo smormah/vsift --latest=false`, then
`gh release edit v0.2.0 --repo smormah/vsift --latest`, and read it again. Every `gh attestation verify` is 6.4's command with the version, `--source-ref refs/tags/v0.2.1-rc.3`, `--signer-workflow smormah/vsift/.github/workflows/release.yml` and `--deny-self-hosted-runners`; the tarballs' and the ten files' checksums equal the plan's (6.15 step 4).

**Step 5, the hosted checks of the published bytes** (the supervisor; each is one dispatch, from `main`, once, on this candidate; nothing from `0.2.1-rc.1`'s or `0.2.1-rc.2`'s runs is reused):

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.3
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.3
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.1-rc.3
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.3 -f from_version=0.2.0
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1-rc.3 -f from_version=0.1.0
```

`P14 published artifacts` is dispatched **one after another** (the second after the first has finished: runs of one workflow from `main` share a concurrency group, and a newer waiting run replaces an older one without a word). `P14 verify release` (RQ-19) must be all green, and each package's detail line must say `next 0.2.1-rc.3, latest 0.2.0` and the release-flags check `pre-release true,
GitHub's latest release v0.2.0`. `0.2.0` to `0.2.1-rc.3` is the upgrade a person on `latest` does and `0.1.0` to `0.2.1-rc.3` the baseline every candidate has been measured from. **`0.2.1-rc.2` to `0.2.1-rc.3`, `0.2.1-rc.1` to `0.2.1-rc.3` and `0.2.0-rc.3` to `0.2.1-rc.3` are optional** (a person on `next`); say whether any was run. `P14 journeys` and `P14 published artifacts` run `setup check` on fresh hosted runners, the nearest thing to a
cold start that is run: they do not show the fix, because nobody has seen them fail on it. The rest of the evidence is dispatched once each, one after another and staggered, with the parameters of the runs for `0.2.0`'s candidates: `P14 compatibility`, `Fuzz` (31 targets) and `P14 stress` at the tag; `P07 local ASR`, `P13 managed power loss` and `P10 durability campaign` at the tag; `P14 load`, `P14 malicious media`, `P14 runbook walk` and
`P14 scan reading` from `main` with `version=0.2.1-rc.3`. The Governance job of the CI run at the candidate's commit is RQ-18's. **A failed run is a finding: open an issue before re-running** (governance rule 14). Results are recorded in the ledger in a work-record pull request, from what each run computed, never earlier.

**Step 6, afterwards.** The supervisor tells the maintainer the run ids of the dry run and the publish, the `npm view` answers, the output of `npm audit signatures`, the result of the `diff` of step 3 and the twenty-four-value comparison, which upgrade baselines were dispatched, and anything that failed or was re-run. Nothing is announced. **Batches 2 and 3 of the agent trials start only on the
maintainer's explicit go**, one at a time, after the candidate is published, against the registry's `0.2.1-rc.3` (`-Version 0.2.1-rc.3` in [`p14-batch-2-3-checklist.md`](../planning/p14-batch-2-3-checklist.md)), **under the freeze written for `0.2.1-rc.2`, which this candidate keeps** (`skill` `256b4840...`, whole freeze `a48567ac...`; `prepare` refuses a trial if anything frozen differs from the file); `batch-2/` and `batch-3/` hold only that file, batch 3 still
takes `-AllowGraderChange`, and the skill's changes are as untried as they were (6.15 step 6). **The check that settles #391 is the maintainer's:** on the machine that showed it, with the published candidate, the first `vsift setup check` after a start. Until it has been run the issue stays open.

**Step 7, what this candidate supersedes: `0.2.1-rc.2` and `0.2.1-rc.1`, and deprecation: nothing is planned, and it is the maintainer's open decision.** Both stay published, tagged and installable by their exact numbers, and `next` names `0.2.1-rc.3` from the publish on; they have L-161 (and `0.2.1-rc.1` also L-149), which is why they are superseded, **not** unsafe to leave: nobody is directed to them (`latest` is `0.2.0`). Do not deprecate a candidate before the runs that
install it by its exact version have finished, which includes the optional upgrades from `0.2.1-rc.2` and `0.2.1-rc.1`; an empty message (`npm deprecate "vsift-cli@0.2.1-rc.2" ""`) removes a deprecation. `next` is moved to `0.2.1` after the stable release only by the maintainer's own command (the workflow never does it; [L-108](../planning/known-limits.md#l-108)).

**If something fails**, 6.15's table applies with these readings: step 2's plan must move `next` from **`0.2.1-rc.2`** (anything else: do not publish, tell the maintainer); the **Registry** guard failing ("`0.2.1-rc.3` is already on npm, and not with these bytes") means the number is used for good and the next candidate is `0.2.1-rc.4`, which needs its own cut; a bad published version is deprecated on all four packages (never unpublished) and `0.2.1-rc.4` is cut by this section again.

**What is irreversible:** the four published `0.2.1-rc.3` versions and their provenance records on npm's transparency log; the Sigstore attestations of the ten files and four tarballs (they exist from the end of `attest`, before the approval); the tag once anything was published from it; and the twenty-four versions of `0.2.0-rc.1` to `0.2.1-rc.2`, which this section does not change. **What is reversible:** `next`
(`npm dist-tag add vsift-cli@0.2.1-rc.2 next` for each package, with the maintainer's login and second factor; the workflow never does it), the notes, and a deprecation.

### 6.17 The stable release `0.2.1`: the numbers, the command lines and what happened (the patch release, 2026-10-15)

**A record (2026-10-16), and the second model for a stable version.** The maintainer's steps for this release were a planning page, [`release-0.2.1-stable-steps.md`](../planning/release-0.2.1-stable-steps.md), because this file could not change between the third candidate and the tag (6.8). That page followed [`p14-stable-release-steps.md`](../planning/p14-stable-release-steps.md) (the page for `0.2.0`, which 6.13 records) and 6.7 with the numbers in the table below and the command lines under "The command lines that changed". This section folds in that table and those lines and records what happened when the page was followed and what the real services showed. **`0.2.1` is built from the same source as `0.2.1-rc.3`:** the stable commit differs from the tag `v0.2.1-rc.3` only in five version-string files, the two shipped documents and the work record (6.8; the plan's guard counted 5, 2 and 77 files). The tag is `v0.2.1` at `e9ab6e62673328a1645254a4a32e552a2a2f61bf` (the merge of #402). **For the next stable version,** write a planning page from this section and 6.7 before the candidate window closes, with that version's numbers, and fold it in here after the publish. Sections 6.10 to 6.16 are records of candidates; the steps below were followed once, on 2026-10-15.

**The numbers that changed from the model page (read 6.7 and the model page with this table).**

| | `0.2.0` (the model page, 2026-10-09) | `0.2.1` (this release) |
| --- | --- | --- |
| The version, the tag, the commit | `0.2.0`, `v0.2.0`, the merge commit of the pull request that carried P14 PR 12 | `0.2.1`, `v0.2.1`, the merge commit of the pull request whose title starts "The stable release 0.2.1" |
| The accepted candidate | `v0.2.0-rc.3` at `83dca856e7a00fc9a71c87baae99f0b1d401dd31` | `v0.2.1-rc.3` at `fa380021f2aa8db40fdccdb7c410839e8fc10f4c` (the highest `v0.2.1-rc.N` tag; the check ignores every tag of another `X.Y.Z`) |
| What npm holds before | `latest` `0.0.0`, `next` `0.2.0-rc.3` | `latest` `0.2.0`, `next` `0.2.1-rc.3`, on all four packages |
| What the plan does | `latest` from `0.0.0` to `0.2.0` | `latest` from `0.2.0` to `0.2.1`; `next` stays `0.2.1-rc.3` (only the maintainer's own command moves it, [L-108](../planning/known-limits.md#l-108)) |
| The `latest moves forward` guard | `latest` is `0.0.0` on all four | `latest` is `0.2.0`, a version below `0.2.1`, and `0.2.1` is on none of them |
| The evidence ledger guard | complete for `0.2.0-rc.3` at `83dca856e7a0` | complete for `0.2.1-rc.3` at `fa380021f2aa` |
| Candidate-to-release file counts | 5 version-string, 2 shipped-document, 77 work-record files | 5 version-string, 2 shipped-document, 77 work-record files |
| `--tag latest` under trusted publishing | never tried (L-105) | tried and accepted for `0.2.0` (6.13) and again for `0.2.1` |
| The before-snapshot of the earlier versions | three candidates, twelve values, three tags | seven versions, twenty-eight values, seven tags |
| What `vsift --version` prints | `vsift 0.2.0 (eeb2a22a46a8)` | `vsift 0.2.1 (e9ab6e626733)` |
| The GitHub release | becomes GitHub's latest, from none | becomes GitHub's latest, from `v0.2.0` to `v0.2.1` |
| The deprecations afterwards | `0.2.0-rc.1` and `0.2.0-rc.2` | `0.2.1-rc.1` and `0.2.1-rc.2`, message `Superseded by 0.2.1. Install vsift-cli.`; `0.2.1-rc.3` and `0.2.1` are not (decision of 2026-10-15) |
| `P14 verify release` | red on two unregistered checks until P14 PR 13a | all green, including the delta check and the check of `latest` on all four packages, dispatched from `main` within seven days of the publish |
| Yarn 4 users | a day later | a day later (`npmMinimalAgeGate`) |

**The command lines that changed** (`<stable commit>` is the 40 digits of the merge commit; the rest of the model page's steps, with no version in them, are as written there).

1. **Find the commit** (before the tag): `gh pr view <number> --repo smormah/vsift --json state,title,mergeCommit --jq "{state, title, commit: .mergeCommit.oid}"` (expect `MERGED`), then `git show <stable commit>:Cargo.toml | grep -m1 "^version"` prints `version = "0.2.1"`. On `main`, every check passed: `git merge-base --is-ancestor <stable commit> origin/main && echo "on main"` and `gh api "repos/smormah/vsift/commits/<stable commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'` prints nothing. The up-to-date rule is on: `gh api repos/smormah/vsift/branches/main/protection --jq .required_status_checks.strict` prints `true`.
2. **The delta check, by hand, at the commit** (the check is compiled from the commit it judges): `git switch --detach <stable commit>`, then `cargo run --locked -p vsift-release -- candidate-delta` (every file `version string only`, `shipped document` or `work record`, nothing `REFUSED`), then `git diff --stat v0.2.1-rc.3 <stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml` lists only `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/package.json` and `npm/vsift-cli/README.md`, and the version files differ in the version text only: `git diff -U0 v0.2.1-rc.3 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep -c "^[-+][^-+]"` prints `52` and the same pipe through `grep "^[-+][^-+]" | sed 's/^[-+]//; s/0\.2\.1-rc\.3/0.2.1/' | sort | uniq -c | awk '$1 % 2 == 1'` prints nothing. Then `git switch main`. Outside the allowed work-record paths only seven files may differ: `git diff --name-only v0.2.1-rc.3 <stable commit> | grep -v -e "^CHANGELOG.md$" -e "^memory/" -e "^docs/decisions/" -e "^docs/history/" -e "^docs/planning/" -e "^docs/guide/"` prints `Cargo.lock`, `Cargo.toml`, `docs/operations/install.md`, `fuzz/Cargo.lock`, `fuzz/Cargo.toml`, `npm/vsift-cli/README.md` and `npm/vsift-cli/package.json`.
3. **The evidence ledger is complete for the candidate** (the plan's guard): `cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.1-rc.3 --commit fa380021f2aa8db40fdccdb7c410839e8fc10f4c` prints "VSift release evidence is complete for 0.2.1-rc.3 at fa380021f2aa." (RQ-13 was the last open item, until #401).
4. **What npm holds now,** from a shell with no npm login: `npm view vsift-cli dist-tags` and the same for the three platform packages print `{ latest: '0.2.0', next: '0.2.1-rc.3' }` four times, and `npm view vsift-cli versions` does not list `0.2.1`. Write down the twenty-eight integrity values and the seven tag commits: `for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2 0.2.1-rc.3; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done` and `for t in v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2 v0.2.1-rc.3; do git show --no-patch --format="%H" "$t^{commit}"; done` (in PowerShell write `"$t^^{commit}"`); the commits are `d5792ce31db1106934233c86d0518c3ad1961e07`, `7c722d1fc46af7fddeffbaf807028eaec413ace1`, `83dca856e7a00fc9a71c87baae99f0b1d401dd31`, `eeb2a22a46a85ab10a456f2ab5d6a62e292836c5`, `cb8cb3a6286047db56831459dabd6ee5500a37bd`, `23ad4927e56f11d8047ca54d59b892516812d8f8` and `fa380021f2aa8db40fdccdb7c410839e8fc10f4c`.
5. **Tag, dry run, publish:** `git tag -a v0.2.1 -m "VSift 0.2.1" <stable commit>`, `git push origin v0.2.1`, `git show --no-patch --format="%H" "v0.2.1^{commit}"` (the 40 digits written down); a wrong tag may be moved only before anything is published. Dry run: `gh workflow run release.yml --repo smormah/vsift --ref v0.2.1 -f dry_run=true`; publish: the same with `-f dry_run=false`. Pick each run by its `headBranch` (`v0.2.1`) and `headSha`. The plan must list the six guards, all `passed` (the stable version; the accepted candidate against `v0.2.1-rc.3` with 5, 2 and 77 files; the registry read; the candidate published; `latest` moving forward from `0.2.0`; the evidence ledger complete for `0.2.1-rc.3`), the table of dist-tags (`latest` from `0.2.0` to `0.2.1` and `next` staying `0.2.1-rc.3`, on each of the four packages), the `release_delta` record with `"verdict": "allowed"`, the four `npm publish ... --tag latest --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`, and the GitHub release commands (a draft, then published as latest, ten assets). Download the dry run's plan (`gh run download <id> --repo smormah/vsift --name publish-plan --dir plan-dry`), then the publish's (`plan-publish`), compare `attestation-subjects.sha256` and `release-assets.sha256`, and **keep `plan-publish/release-delta.json`: the artifact expires seven days after the run.** Then the maintainer approves the `release` environment.
6. **Check it from the outside** (no npm login): the four `npm view ... dist-tags` print `{ latest: '0.2.1', next: '0.2.1-rc.3' }`; the loop of item 4 prints the same twenty-eight values; `mkdir check-0.2.1`, `cd check-0.2.1`, `echo '{"private": true}' > package.json`, `npm install vsift-cli`, `npm audit signatures`, `npx vsift --version` prints `vsift 0.2.1 (e9ab6e626733)`; `gh release view v0.2.1 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"` prints false, false and 10; `gh api repos/smormah/vsift/releases/latest --jq .tag_name` prints `v0.2.1`; the checksums and attestations are 6.4's commands with `v0.2.1` and `--source-ref refs/tags/v0.2.1`.
7. **Deprecate the first two candidates of the release** (after item 6 passed; the maintainer's login and second factor for each write; eight writes): for each of `vsift-cli`, `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64` and each of `0.2.1-rc.1` and `0.2.1-rc.2`, `npm deprecate "<package>@<version>" "Superseded by 0.2.1. Install vsift-cli."`. Read back with `npm view "<package>@<version>" deprecated`; `0.2.1-rc.3`, `0.2.1`, `0.2.0-rc.3` and `0.2.0` print empty brackets and `0.2.0-rc.1` and `0.2.0-rc.2` still print their earlier message. An empty message removes a deprecation. The tags do not move.
8. **The hosted checks of the published bytes** (dispatched from `main`, none can publish; dispatch the second `P14 published artifacts` after the first has finished, because a newer waiting run replaces an older one): `gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.1` **within seven days of the publish** (the delta check reads the publish run's artifact, which GitHub keeps seven days; after that it fails by name and points at the ledger's `release_delta`), `p14-journeys.yml` with `-f version=0.2.1`, `p13-managed-smoke.yml` with `-f published_version=0.2.1`, `p14-published-artifacts.yml` with `-f version=0.2.1 -f from_version=0.2.0` and then with `-f from_version=0.2.1-rc.3`, and `p14-scan-reading.yml` with `-f version=0.2.1`. A failed run is a finding: open an issue before re-running (governance rule 14).
9. **After the publish, in a work-record pull request that may touch any page:** copy `plan-publish/release-delta.json` into the ledger's `release_delta` and give the items whose release rule is `repeat` (RQ-01 to RQ-06, RQ-13, RQ-18, RQ-19) their own entries for `0.2.1`; `release-evidence --complete-for 0.2.1 --commit <stable commit>` is then the check; fold the planning page into this section; mark the candidate's section a record; correct the pages the stable commit could not change (the root README, `SECURITY.md`, the skill guide, the development guide, the sentence in `docs/agents/trials.md`) and the register; rewrite the two handoff files. **Done by the records change of 2026-10-16** ([`release-0.2.1.md`](../planning/release-0.2.1.md), "The evidence on 0.2.1 (recorded 2026-10-16)").

**What happened.**

| Step | What happened |
| --- | --- |
| Dry run on the tag | `workflow_dispatch` on `v0.2.1` with `dry_run` set, 2026-10-15 at 14:59 UTC, run 38756334727, success in about ten minutes: `attest` and `publish` skipped. |
| The real dispatch | The same dispatch with `dry_run` cleared, 15:11 UTC, run 38758081349, which is the run npm's provenance names (its attempt 1). The build, packaging, npm qualification, plan and `attest` jobs finished by 15:22 UTC, and the plan's `release-delta.json` was kept: it is the ledger's `release_delta`. |
| The six guards | The stable version; the accepted candidate (against `v0.2.1-rc.3` at `fa380021f2aa`: 5 version-string, 2 shipped-document and 77 work-record files differ, nothing else); the registry read; the candidate published on all four packages; `latest` moves forward (`0.2.0` on all four); the evidence ledger (complete for `0.2.1-rc.3`). All `passed`. |
| The approval | The maintainer approved the `release` environment, and the `publish` job started at 15:32 UTC. |
| `--tag latest` under trusted publishing | **It was accepted again.** The four `npm publish ... --tag latest --provenance --ignore-scripts` commands published all four packages. |
| npm's read-back of `latest` | **It took longer than the job allows.** The job polls every 30 seconds for up to five minutes: three of the four packages read `latest` as `0.2.1` from about the fifth attempt, and `@vsift/darwin-arm64` still read `0.2.0` after ten attempts (15:33 to 15:38 UTC). The job failed there (15:38 UTC); the GitHub release had not been created and no draft was left. A few minutes later npm's public metadata read `latest 0.2.1` and `next 0.2.1-rc.3` on all four packages ([#403](https://github.com/smormah/vsift/issues/403); for `0.2.0` the same wait took about two and a half minutes). |
| The re-run | As 6.5 says for a publish that fails part-way: **Re-run failed jobs** on the same run, approved again (attempt 2, 15:44 UTC): the versions npm already held with the same bytes were skipped, the read-back passed at once, and the GitHub release was created as a draft with ten assets and published as latest (the job ran 15:47 to 15:48 UTC, success). npm's provenance statements name attempt 1, the attempt that published the packages, and `P14 verify release` accepts that. |

**Checked from outside, after the publish (2026-10-15):** `latest` is `0.2.1` and `next` is `0.2.1-rc.3` on all four packages; the 28 integrity values of the seven earlier versions and the seven tag commits are unchanged; `npm audit signatures` reports 4 verified registry signatures and 4 verified attestations; `gh attestation verify` passes for the ten release files and the four registry tarballs, which equal the plan's checksums; a plain `npm install vsift-cli` installs `0.2.1` and `vsift --version` prints `vsift 0.2.1 (e9ab6e626733)`; GitHub's latest release is `v0.2.1`. **The hosted checks were then run on `0.2.1`'s own bytes** (all dispatched from `main` at the stable commit): `P14 verify release` (38764125726, 22 checks passed, the two that a stable version adds among them: the delta check and `latest` on all four packages), `P13 managed smoke` (38764760524), `P14 published artifacts` from `0.2.0` (38765075608) and from `0.2.1-rc.3` (38765697122, dispatched after the first had finished) and `P14 scan reading` (38777879949) passed; **`P14 journeys` (38764446399) failed on hosted Windows twice, in the worker's shutdown stage, and passed on Ubuntu and macOS** ([#404](https://github.com/smormah/vsift/issues/404), known limit [L-167](../planning/known-limits.md#l-167); the maintainer decided on 2026-10-16 that it is a known issue of `0.2.1` and the first fix of `0.2.2`). The record is [`release-0.2.1.md`](../planning/release-0.2.1.md), "The evidence on 0.2.1 (recorded 2026-10-16)".

**Deprecation.** On 2026-10-15 `0.2.1-rc.1` and `0.2.1-rc.2` were deprecated on all four packages with the message `Superseded by 0.2.1. Install vsift-cli.`; `0.2.1-rc.3`, `0.2.1` and `0.2.0` were not. **`next` was not moved** and the workflow never moves it ([L-108](../planning/known-limits.md#l-108)): `vsift-cli@next` installs `0.2.1-rc.3`, a pre-release of the same source, and whether to move it by hand is the maintainer's open choice.

**What this run showed about the steps, for the next stable version.** (1) **The read-back window is too short for the real service** (#403): the packages can be published and the job can still fail, and then 6.5's re-run is the whole remedy. Until the window is lengthened (iteration B of [`backlog-iterations.md`](../planning/backlog-iterations.md), which is cycle 1, `0.2.2`, since 2026-10-16), expect that, check `npm view` before doing anything else, and re-run the failed job rather than the whole workflow. (Note, 2026-10-17: the window is now 20 minutes and a wrong answer fails at once, 6.4; the remedy for a read-back that still runs out is the same, 6.5.) (2) The page said that for `0.2.0` the read-back took about two and a half minutes; for `0.2.1` one package took more than five. (3) **A failed `P14 journeys` run on one system is a finding** whatever the other systems say: governance rule 14 asked for an issue before the one re-run, and the failure is recorded in the evidence ledger as it is (the item is `failed`; whether to waive it is the maintainer's). (4) No other problem with the steps was reported: the guards, the approval, the delta record, the checks from outside and the deprecations are as the model page has them.

**What was not tried, and is not shown.** Yarn's behaviour for an untagged `yarn add vsift-cli` inside its one-day gate (the hosted jobs installed the exact version after the documented exemption); the campaigns on the release's own tag (they are the candidate's, carried by the recorded delta); and everything the release ships untried (the maintainer's try-outs, RQ-17, [L-143](../planning/known-limits.md#l-143)). Nothing here announces anything.

### 6.18 The weekly release window (decided 2026-10-16; release branches and the workflow's checks since 2026-10-17)

**An operating procedure, not a record.** The maintainer decided on 2026-10-16 how releases are timed and
tested, and on 2026-10-17 that release candidates and the stable release are made on a release branch so that `main`
keeps taking merges during the window. The policy and the reasons are in
[`delivery-governance.md`](../planning/delivery-governance.md) ("The test and release model"), and the stories it
serves are in [`backlog-iterations.md`](../planning/backlog-iterations.md). This section is what a person does. The
steps of 6.3 (a candidate) and 6.7 (the stable version), the check of 6.8 and the stable plan's six guards are
unchanged; what changed is **where** the tags are made (the release branch, 6.8) and that **the Release workflow
now refuses, in seconds, a dispatch on the wrong ref and a candidate whose commit has no passed nightly** (the
`guard` job, below). The maintainer's approval of every `release` environment gate and of every npm write is
unchanged, and so is the rule that only the maintainer creates a `v*` tag.

**The rhythm.**

| When | What |
| --- | --- |
| Monday to Friday | Development on `main`. Each pull request passes the gate (target 5 minutes, hard limit 10). The nightly runs on `main` when something merged. **During a window `main` keeps taking merges**; the window's own changes go to the release branch by pull request. |
| **Friday or Saturday midnight** (the maintainer's local time) | **The cut** (step 2): the release branch from a commit whose nightly passed, and the first candidate's tag on it. |
| Saturday to Monday | The long hosted runs (2 to 3 hours each), on the candidate's tag, for what the week's changes touched (step 3). |
| **Monday or Tuesday** | **The target for the stable release and its publish**, on the maintainer's approval (step 5). |

**Which refs may run each mode** (the `guard` job, the Release workflow's first job; every other job needs it, so a
refused dispatch builds nothing). The workspace version at the dispatched commit decides the names.

| Mode | May run on | Also required | Refused otherwise, before any build |
| --- | --- | --- | --- |
| Pull request, push | the pull request's merge ref, a push to `main` or `release/**` | nothing more | (never refused by the guard) |
| **Dry run** (dispatch, `dry_run` set) | `main`; `release/<MAJOR>.<MINOR>.x` of the workspace version (a candidate version there gets a line in the summary saying whether a candidate tagged on this commit would meet the nightly precondition); the tag `v<version>` | on the tag, everything a publish needs (the rehearsal fails whenever the real run would) | any other branch or tag, another line's release branch, a fork's tag |
| **Candidate** (dispatch, `dry_run` cleared, a version with a suffix) | the tag `v<version>` only, in `smormah/vsift` | the tag still names the run's commit; that commit is on `release/<MAJOR>.<MINOR>.x`, whose rulesets hold the five rule types of 6.2 step 8; **the nightly precondition** (step 1) | `main`, a release branch or any other ref ("Dispatch it on the tag, or leave dry_run set"; #337) |
| **Stable version** (dispatch, `dry_run` cleared, no suffix) | the tag `v<version>` only, in `smormah/vsift` | as for a candidate but the nightly; the stable plan's six guards (6.7) | as for a candidate |

**After the approval, the `publish` job asks the same questions again** before its first `npm publish`
(the maintainer's decision of 2026-10-17): the approval can come days after the dispatch, so a candidate whose
nightly was re-run and failed (or is still running) in the meantime, a tag that was moved to another commit, a
release branch that no longer holds the commit or that lost one of its rules is refused there, and nothing is
published. The tag check matters for re-runs too: **Re-run all jobs** of an old run, after its tag was moved, would
otherwise build the old commit and attach it to the moved tag; both the guard and the re-check refuse it. It uses the guard's own two functions,
character for character (the lint compares them), and reads GitHub only. The plan job's own check (a publish
only on `refs/tags/v<version>`, 6.1) stays as a further line. In GitHub's
"Run workflow" form, **"Use workflow from" defaults to `main`: choose the tag (Tags) before you clear `dry_run`**;
with `gh`, pass `--ref v<version>`. A wrong choice now costs seconds, not the twelve minutes of builds it cost on
2026-10-08 (#337).

1. **The nightly precondition (governance rule 15), enforced by the `guard` job since 2026-10-17.** A candidate (any
   version with a suffix) is published, and its tag rehearsed, only when **the latest Nightly run that tested exactly
   the tag's commit passed**:
   - **Exactly that commit**, not a later one that contains it: a later commit's pass does not show that this one
     passes, because the later commit may be the very fix of this one's failure (the stop-the-line fix is such a
     commit), and a commit of the release branch is never an ancestor of `main` when pull requests are squashed.
     Not an earlier one either. Only a run whose head is the commit tested the bytes the candidate is built from.
   - **The latest such run decides,** by start time: a pass followed by a failed re-run is refused, and a failure
     followed by a passed re-run or a passed dispatch is accepted (governance rule 14 asks for the issue first).
     **While any Nightly run of that commit is still queued or running, the candidate is refused** (wait for it),
     so an older run cannot decide while a newer one is under way.
   - **A run that ran nothing is passed over, not counted.** A scheduled run that finds nothing new on `main` (its
     first job, "Anything new on main", says so) skips every check and still concludes `success`; after a failed
     night, the next night's run on the same commit is such a run. A run in which only one job ran is therefore
     ignored, and the run before it decides.
   - **Every job of the deciding run but `Report a failure` must have concluded `success`;** a skipped check is not a
     passed one. Only `schedule` and `workflow_dispatch` runs of `nightly.yml` count (a branch experiment that gave
     the workflow a push trigger, as on 2026-10-16, does not).
   - **A commit no nightly tested** (a commit made on the release branch, or a `main` commit merged after the night's
     run) gets one by dispatch: `gh workflow run nightly.yml --repo smormah/vsift --ref <the branch whose head is the
     commit>` (about 20 minutes; on a branch other than `main` it prints its failure report instead of filing one).
     Check it before tagging: `gh run list --repo smormah/vsift --workflow nightly.yml --commit <sha> --json
     databaseId,event,conclusion,startedAt,url`, or dispatch a dry run on the release branch, whose summary says
     whether the precondition would be met.
   - If the latest nightly of `main` failed, development has stopped: its issue is fixed first, and there is no cut
     from a red commit. A stable version is not asked for a nightly of its own: its code is its accepted candidate's,
     which met the precondition when it was published, and the plan compares the two (6.8).
2. **The cut** (the maintainer's, by hand; no workflow creates a branch or a tag):
   1. **Before the night's nightly**, the first candidate's version bump is merged to `main` by 6.3 step 1 (the
      files of 6.8, the changelog, and with the first bump to a new release the guide's marker). `main` then
      carries `X.Y.Z-rc.1` until the next cut's bump.
   2. **Choose the commit:** the `main` commit of the latest Nightly run that passed and that carries the bump:
      `gh run list --repo smormah/vsift --workflow nightly.yml --branch main --limit 5 --json
      headSha,event,conclusion,startedAt,url`, then check that its checks ran (step 1). If the bump merged after the
      night's run, dispatch the nightly on `main` and wait for it.
   3. **Create the release branch at that commit:** `git push origin <commit>:refs/heads/release/0.2.x` (for the
      `0.2` line). **Cut only from a commit whose `CI` has finished with a passed `CI result`**: the ruleset refuses
      the branch's creation while it is still running (seen on 2026-10-17); a commit whose nightly passed has long
      finished its `CI`. After the cut, expect every direct push to the branch to be refused ("Changes must be made
      through a pull request"): window fixes, bumps and records reach it by pull request only. If a previous window's `release/0.2.x` is still there, the stable release of that window is published
      and its records are on `main`: delete it first (`git push origin --delete release/0.2.x`); its tags keep its commits.
      If the stable release of that window is not published, the window has slipped (step 6) and there is no new cut.
      **Before deleting the old branch, run the `Release window` workflow** (`gh workflow run release-window.yml
      --repo smormah/vsift --ref main -f records=true`, or `python3 tools/release_window.py --records` from a fresh
      clone): the branch is the last place that holds a fix `main` may lack, and the records change should
      already have shown none.
   4. **Tag the commit** (6.3 step 2): `git tag -a v0.2.2-rc.1 -m "VSift 0.2.2-rc.1" <commit>` and `git push origin
      v0.2.2-rc.1`.
   5. **Dry run on the tag, then publish** (6.3 steps 3 and 4): `gh workflow run release.yml --repo smormah/vsift
      --ref v0.2.2-rc.1 -f dry_run=true`. The `guard` job answers within a minute: the ref, the branch and the
      nightly. Then the publish with `-f dry_run=false` and the maintainer's approval of the `release` environment.
      The candidate goes to `next`. **A "Re-run failed jobs" of a candidate's publish asks the nightly again** (the
      publish job's re-check runs in every attempt): if a Nightly run of that commit failed between the attempts,
      the re-run is refused, even when some or all of the packages are already on `next`. Then do not try to force
      it: the published versions stay as they are (never unpublished), and the recovery is to fix forward with the
      next candidate (`rc.N+1`), whose own nightly must pass.

   **Why the cut is not a scheduled job.** A job that creates the branch would need `contents: write` in a workflow
   that today writes only in its two approved jobs, and a way past the branch ruleset; it would choose the commit by
   the same rule the `guard` job checks at the tag; and the tag, the dispatch and the approval stay the maintainer's
   anyway. It would save one `git push`, so the cut stays a runbook step.
3. **After the cut, the long hosted runs.** Dispatch, on the candidate's tag, the long runs for the items whose scope
   this week's changes touched. The evidence ledger's scopes decide, and they are coarse: `crates` is in the scope of
   most items, RQ-01 to RQ-12 and RQ-14 to RQ-17. The list is in [`p14-qualification.md`](../planning/p14-qualification.md)
   and the ledger, and `release-evidence --complete-for <candidate> --commit <sha>` names what is missing. Record the
   run links in the release's planning page and the ledger **by a records pull request to the release branch** (the
   stable plan reads the ledger at the stable commit, 6.8). **RQ-04's local upgrade runs here, on the candidate's tag:**
   since #416 `P14 local upgrade` no longer runs on pull requests (only in the nightly, under the `full-ci` label or by
   dispatch), so its evidence for a release comes from `gh workflow run p14-local-upgrade.yml --repo smormah/vsift --ref
   v<candidate>` once the candidate's tag exists (the run must be on the tag; `-f from_version=<published version>` is
   optional and defaults to `0.1.0`). Record its link with the other RQ-04 runs.
4. **A failure in the window.** Open (or find) the issue first, then fix it with a **targeted fix and a rerun of only
   the failing workflow**, never the whole round. The fix is a pull request **into the release branch**, and the same
   change goes to `main` by its own pull request. If it changes code, 6.8 applies: it means a new candidate. Merge its
   version bump (`rc.N+1`) into the release branch by pull request, dispatch the nightly on the branch (`gh workflow
   run nightly.yml --repo smormah/vsift --ref release/0.2.x`) and wait for it to pass on the branch's head, then tag
   that head and dispatch as in step 2.5. **Pull requests to a release branch get the scanning of those to `main`** (since
   2026-10-17, L-173): `CI`, the Release dry run, CodeQL's `Analyze Rust` (it always reports, and analyses when the
   change touches what CodeQL reads, #416), `Security` (the cargo-deny policy and the dependency review) and the
   P14 workflows whose paths the change touches. Their pushes and schedules stay on `main`. **Then dispatch, on the
   new candidate's tag, the long workflows that cover what the fix touches** (step 3): a pull request runs them only
   for the paths each one names. **The fix's counterpart pull request to `main` is opened with it**, and the `Release
   window` check (`python3 tools/release_window.py`, or the workflow on demand) lists any commit of the branch that
   `main` does not have yet; it is a report, so a counterpart that is still open shows as missing until it merges.
   A finding that is
   not a regression of this week may be accepted by the maintainer as a known issue of the release, as #404 was for
   `0.2.1`. That is the maintainer's decision, recorded in the ledger, and never the supervisor's.
5. **The stable release and its publish, Monday or Tuesday.** When the accepted candidate's evidence is complete
   (`release-evidence --complete-for`), make the stable commit **on the release branch** (a pull request with the
   version strings and the shipped documents of 6.8, and nothing else outside the work record), and take the stable
   release by 6.7 and the model pages 6.13 and 6.17 (the delta check, the tag on that commit, the dry run, the
   publish). The `guard` job checks that the tag is on the release branch. **The publish waits for the maintainer's
   approval of the `release` environment; the supervisor never approves it and never runs an npm write.** The
   read-back of the dist-tags waits up to 20 minutes and fails at once on a wrong answer (6.4); if it runs out of time,
   6.5 says how to finish. Then check the published bytes from outside (6.4) and run the quick checks: `P14 verify
   release` (within seven days of the publish), `P14 journeys`, `P13 managed smoke`, and `P14 published artifacts`
   from the previous release and from the candidate. **The records change after the publish is a pull request to
   `main`** that brings the release branch's work record (the ledger, the planning page, the changelog) **and the
   stable commit's shipped documents** (`docs/operations/install.md`, `npm/vsift-cli/README.md`, so `main`'s
   installation guide says what the stable release says), and checks that every window fix is on `main`: **the `Release window` workflow runs on that pull request by itself**
   (it triggers on a change to `docs/planning/p14-evidence-ledger.json`, which every records change makes) and, as the
   merge ref is compared, requires the work record and the shipped documents of the branch to be on `main` as well as
   its fixes (`--records`); it fails with the commit, the file and the lines `main` lacks (the records change
   rewrites the handoff files, the register's date and counts and the plan's status as current state, and the
   check does not report those: 6.8 lists what it excuses, and the report names each). Bring them, or record
   in the pull request why `main` differs, before merging; no ruleset requires the check. After it merges, the release
   branch may stay until the next cut deletes it.
6. **If the window slips,** the cut waits for the next Friday or Saturday. A candidate already published stays on
   `next`, and its release branch stays. Only a red nightly stops development; a late publish does not.

**A hot fix** is a fix too urgent to wait for the window. It goes through outside the window, on the release branch
of the version it fixes (re-created from that version's tag if it was deleted, 6.8; for a version tagged before
2026-10-17 the first pull request brings the current workflows and their tools onto it, 6.8): the fix by pull request into the
branch (and the same change to `main`), the full check on all three systems on the branch's head, which is the
nightly dispatched on the branch (about 30 minutes) and which the candidate needs anyway, a candidate and the stable
release by 6.3 and 6.7 (the stable plan needs a published accepted candidate, and the completeness check still
applies: an item whose scope the fix touches and that needs a long run needs that run or a recorded waiver by the
maintainer), the publish on the maintainer's approval, and then the quick checks of the published bytes listed in
step 5. The long runs and the agent batches are not run before a hot fix; they run in the next window, and anything
they find is an issue first.

**The monthly round, or with a major change.** About once a month, and when a major change lands (a rewrite of the skill, a re-pin of FFmpeg or whisper.cpp, a change to the way VSift calls
a tool), the full-cost items run in the window of the candidate that carries the change: agent batches 2 and 3 (the agent-trial runbook, [`trials.md`](../agents/trials.md)),
the dependency-quality checks (`P07 local ASR` accuracy, `P14 malicious media` and the dated scan reading) and the full durability campaign. They are **not run weekly**. In an ordinary
week, a change to the skill's wording gets the uncounted skill smoke (about nine Claude runs on a local build, about $6 and 15 minutes) instead. It is not evidence for RQ-15 or RQ-16, so a candidate whose skill
changed still needs those items passed, carried or waived before the stable release: the ledger's rule is unchanged. **One exception to read with it (2026-10-17, [`p14-qualification.md`](../planning/p14-qualification.md) section 32):** RQ-10's waiver ends with `0.2.1`, so the first candidate after it runs `P14 malicious media` on its own bytes and RQ-10 is recorded `passed` for it, although that run is otherwise a monthly one; the ledger entry stays `waived` until then only because the completeness check cannot end a waiver by date, and the campaign's judge already pins the link's `STORAGE_IO` and its remediation (the same change).

**The weekly drift runs stay** (`P14 journeys`, `P13 managed smoke`, `P07 local ASR`, `Fuzz` and the durability campaigns, on their schedules). A failure of one opens an issue by itself. That needs a workflow change;
until it lands, read the Actions page each Monday and file the issue by hand.

**What this does not settle.** The coarse scopes of the ledger against "limited to what the week touched" (listed for
the maintainer in [`backlog-iterations.md`](../planning/backlog-iterations.md)). And what the release branch leaves to
people ([L-173](../planning/known-limits.md#l-173)): a window fix reaches `main` only by a pull request someone opens;
the `Release window` check that a window fix is on `main` is a report that runs on the records change and on demand,
not a rule, and compares lines, not the counterpart's correctness (a fix that `main` reworked is reported as
missing); and the guard and the re-check read the branch's rule types, not their settings. (The guard's questions are asked twice: at the dispatch,
and again by the `publish` job after the approval.)

### 6.19 The release candidate `0.2.2-rc.1`: the cut, the numbers and the command lines (the first candidate of `0.2.2`, and the first on a release branch; cut 2026-10-17)

**A record (2026-10-18).** Followed once, on 2026-10-17 and 2026-10-18: the tag `v0.2.2-rc.1` is the commit `43a033dbf81346b967b293a150e13306828cac50` (the merge of #453), at which the maintainer created `release/0.2.x`; the nightly was run 39058441764, the dry run 39077367812 and the publish 39078284454 (its first attempt); `next` moved from `0.2.1-rc.3` to `0.2.2-rc.1` on all four packages, and `latest` stayed `0.2.1` (GitHub's latest release stayed `v0.2.1`). `P14 verify release` (run 39081996813) passed. The hosted checks, the long runs and agent batches 2 and 3 were run on its bytes or its tag and the evidence is recorded in the ledger ([`release-0.2.2.md`](../planning/release-0.2.2.md), "The evidence on 0.2.2-rc.1", #455). It is the accepted candidate of the stable release `0.2.2` (6.20) and it is still `next`; it was deprecated on 2026-10-18, after the stable release's publish (6.20). **What the run showed about the steps:** the branch ruleset refuses to create `release/0.2.x` at a commit whose `CI result` has not passed (step 0 says so); the first click on the approval did not register and the second did; the `guard` job passed in ten seconds; and the 20-minute read-back read `next` back at its fifth read, about two minutes after the last publish. It is a record and is not to be followed again.

**Status when written: cut, not branched, not tagged, not published.** This section was written before the candidate existed anywhere but in the pull request that cuts it; nothing in it had been run. It is **6.18's steps 1 to 3 (the nightly precondition, the cut, the long runs) with this candidate's numbers**, plus 6.16's checks from outside, and the command lines are spelled out so that they can be pasted (Git Bash; in PowerShell write `"$t^^{commit}"` for `"$t^{commit}"`). 6.3 (a candidate), 6.4 (the checks of a publish), 6.5 (if something goes wrong) and 6.8 (the release branch) are unchanged. **It is the first candidate of a new release made on a release branch, so it is also the first time `release/0.2.x` is created, the `guard` job decides a real dispatch, and the 20-minute read-back meets the registry.** The plan for the release is [`release-0.2.2.md`](../planning/release-0.2.2.md). Nothing is announced.

**What the candidate is.** `0.2.2-rc.1` is `main` at the merge of the pull request whose title starts "The release candidate 0.2.2-rc.1", which is `main` at `f50ae6b7` (the merge of #452) plus the version bump and the work record. It carries cycle 1 of [`backlog-iterations.md`](../planning/backlog-iterations.md): the fix of #404 first, and #361, #362, #334, #394, #266 on Windows, #325's first step, #264, #232, #370, #371, #366, #367, #395, #396, #419, #433 and #436 (the plan's table names each pull request), with a new Windows-only dependency (`fs4` 1.1.0) and the release workflow of 6.18. The skill is the text of #367 and the agent-trial freeze is the one #435 committed.

**The numbers (read 6.16 and 6.18 with this table).**

| | 6.16 (`0.2.1-rc.3`, 2026-10-13, on `main`) | This candidate (6.19) |
| --- | --- | --- |
| The version, the tag | `0.2.1-rc.3`, `v0.2.1-rc.3` | **`0.2.2-rc.1`, `v0.2.2-rc.1`** (annotated, message `VSift 0.2.2-rc.1`, as every earlier tag) |
| The commit to tag | the merge commit of the pull request "The release candidate 0.2.1-rc.3", on `main` | the merge commit of the pull request whose title starts **"The release candidate 0.2.2-rc.1"**, which is also the head of **`release/0.2.x`** when it is created |
| Where the tag must be | on `main` | **on `release/0.2.x`** (the `guard` refuses a tag its branch does not contain); the branch does not exist before step 3 |
| The nightly | not asked | **the latest Nightly run that tested exactly that commit passed** (scheduled or dispatched; every job but `Report a failure` succeeded; none of that commit still running) |
| What npm holds before the publish | `next` `0.2.1-rc.2`, `latest` `0.2.0` | **`next` `0.2.1-rc.3`, `latest` `0.2.1`**, on all four packages (read on 2026-10-17, below); `0.2.2-rc.1` is on none (404) |
| What the dry run's plan does | `next` from `0.2.1-rc.2` to `0.2.1-rc.3`; `latest` stays `0.2.0` | **`next` from `0.2.1-rc.3` to `0.2.2-rc.1`; `latest` stays `0.2.1`**; GitHub's latest release stays `v0.2.1` |
| What `vsift --version` prints | `vsift 0.2.1-rc.3 (<12 digits>)` | `vsift 0.2.2-rc.1 (<the first 12 digits of the commit>)` |
| The earlier bytes to write down | six versions (24 values) and six tag commits | **eight versions (`0.2.0-rc.1` to `0.2.1`: 32 values) and eight tag commits** (below) |
| The agent-trial freeze | rc.2's, kept | **#435's, kept byte for byte** (`skill` `bd205d6c...`, whole freeze `10180cd88ec994dc48079f3aed46e5343a43b79264056e7b6af206c4b85f74c0`); both `freeze check` runs say `nothing frozen has changed`; the `0.2.1-rc.3` batches' records moved to `batch-2-0.2.1-rc.3/` and `batch-3-0.2.1-rc.3/` in the cut |
| The upgrade baselines (`from_version`) | `0.2.0`, then `0.1.0` | **`0.2.1` first, then `0.2.1-rc.3`**; `0.2.0` and `0.1.0` optional; `P14 local upgrade` on the tag (default `0.1.0`) |
| What this candidate supersedes | `0.2.1-rc.2` | **nothing**: it is the first candidate of `0.2.2`; `0.2.1-rc.3` stops being `next` and is not deprecated here |
| `release-evidence --complete-for` | 15 of 20 | **17 of 20 at the cut** (below); RQ-07, RQ-10, RQ-15 and RQ-16 are `planned`, RQ-14 and RQ-17 `waived` and not named |
| The next candidate, if this one is bad | `0.2.1-rc.4` | `0.2.2-rc.2`: a fix and its bump by pull request into `release/0.2.x`, the nightly dispatched on the branch, its head tagged (6.18 step 4) |

**What npm and git held on 2026-10-17** (read for this section from a shell with an empty npm configuration and no login: `NPM_CONFIG_USERCONFIG` and `NPM_CONFIG_GLOBALCONFIG` each set to an empty file of its own, since npm refuses one file for both). The four `npm view <package> dist-tags --json` printed `{"next":"0.2.1-rc.3","latest":"0.2.1"}` four times, `npm view vsift-cli version` printed `0.2.1`, and `npm view vsift-cli@0.2.2-rc.1 version` answered 404. The 32 integrity values of the earlier versions:

```text
vsift-cli@0.2.0-rc.1 sha512-idqlqhE7sqArRRaqWV29mCFRi/kvhKrJM/HU6VTNpjtQzlzxhMMUdfc2Fl9UQnIDEv4GN61zf+PjUnJYMyPkyQ==
@vsift/win32-x64@0.2.0-rc.1 sha512-8sC7cxFqadbUdJcmByco4Epkvbcayy82TS9msLlxepzNEKK4bFVYeDjEJX2iZFAWTCn7Hcduv4JVJKL5eA3mIA==
@vsift/darwin-arm64@0.2.0-rc.1 sha512-PRptfKfSPiZRRP7KHwQLvjcYIAJ78fxyU2XkIUF7AFv+5NmhXY98lse0DfMoQMRy8R/e2mmPXZXBXTYe2I8+LQ==
@vsift/linux-x64@0.2.0-rc.1 sha512-FkNl2trWj6OAb9gFtcaEzM6+et5A6uTEVKDl5D4PHQiOE2qj4hGH2ypjA7gLD6/i4RAma81uW+nciL7KCXLbJg==
vsift-cli@0.2.0-rc.2 sha512-rYHQFWRGAVsk/iyTh5BmVRSbOwad2znKarClVbdOWK0rVBOjLJw/AvY+tfYp8nyCY+ctvyMZWi7t5aZ2i9OVoA==
@vsift/win32-x64@0.2.0-rc.2 sha512-vlAIEZB0aZgAOyctelJEJGWEWSakbHHvzQi/PY6RXV8XsdVWFOumaswl6HVsFnw5dFkT8t5viO/Yic8fDEozVg==
@vsift/darwin-arm64@0.2.0-rc.2 sha512-mYxX8AZ6g4OI0eaZ78fvaHMuGuk4gYKRZaNp+lW0CMSIz0mR+0tIBAq9oVNKoiidEPCxUV5SkRbhweW16yQxEw==
@vsift/linux-x64@0.2.0-rc.2 sha512-AoYn5O3oIv4hY1xQIA3UcMDGlSP2ps/8DFlCsR/d7rDaA3M5+ZgO9aMe4El8xumV6MAJs1RSca+dvw8mI1Id1g==
vsift-cli@0.2.0-rc.3 sha512-QcksJKa3u84/UehUDVg91nmJFhIziZ90tiJI17btMFFlooN2BEInIG6B3mszVh3HH4xPXFePkwAzfpU86B6p6g==
@vsift/win32-x64@0.2.0-rc.3 sha512-tEDJ2G0KEwrThKkL+MATAqLPhYd2ny05l1EGfqczEY/lXM5m/ngleaJCl8TFAJr0ROWAFJx+Nx5MhGVnOSNwbw==
@vsift/darwin-arm64@0.2.0-rc.3 sha512-q2M5yETdU4z66ZVoOnJP4XIx6s0YzPIuA7A7MjWdqHuSktCnEwyIEMCtmwqk/8u8x5t37l98j2AZsQ3uOUAcxg==
@vsift/linux-x64@0.2.0-rc.3 sha512-KLC9ObwsN7R+nHbiIj/wvFGBCi8rmGbD2sIhjI6Qd7nGnHwlIoWJ8Cv2ppXYVBHBlDsfwHw83T5DIpf6QQa46A==
vsift-cli@0.2.0 sha512-LSM0T0NKuxernIgAmj4Tpot3ErMT0MNy6AtYGtDEiWBFFNA0Gzxxx+QvjD+FcKWoUn6Bo1Lx4C0YzYzUaCMbHw==
@vsift/win32-x64@0.2.0 sha512-Zw7b+Jt/Ip/eqKFtvMnxKpJ0IRT9FaWQaQqMQ42BUifICnLY7YBzM+h8jtFSKiAL3Gt2GWdGYulsaNc2iN8mZA==
@vsift/darwin-arm64@0.2.0 sha512-4IX8Ho7rrnXLP1y1iTZ7qbX8skN8Oc0rGxwJxmkFYfzHNtqb/PtEeYcSIWxmho1mi+O8VllvBJzYnPxoF6TJEg==
@vsift/linux-x64@0.2.0 sha512-Qnn2RLwi+S9UbKzsGlX495BB0bta6z7Ev9Yi5r9VulLwQb0UKZ0yJHwmMbfXc7ZaNi6JZEjx2MwM4EkBTKIDbQ==
vsift-cli@0.2.1-rc.1 sha512-g2XEp+oeI+xmWjeTTrJux3AEGcojY1VN49sLeMUZvQBOB9NRM+hAraXQibam86bB+UnQb1TlWEZYHKduCk8eWQ==
@vsift/win32-x64@0.2.1-rc.1 sha512-hk/49T5fJqA0Q8ne6c6zXkRO11pX8UXlFjtIIlPDutgGbd/XfzaCBlKdo7yrZQBbmeIFjC5XNrdFDNabw8khHw==
@vsift/darwin-arm64@0.2.1-rc.1 sha512-61oE6nevQ/w6NTVDGm2LxfVRJ9+xhc1y3z0jckyxI/cKMdbOaWX/NI7tM1z3lBeuQ5reZ49D60BIkuKoozwOfg==
@vsift/linux-x64@0.2.1-rc.1 sha512-dkgPCogtjqhdKznac6smAAQmKoqp6xW32H8cmpOHlPGSLh0Bo0nUQ/m7xhms746zmL4mUNnU+ngK5QlIMKqFZw==
vsift-cli@0.2.1-rc.2 sha512-uSQNWy5eImAO0Qo20yD1Xj5IWAjn6MOH/zEViE/4WWQIG3lPPF8nTchbe+mlfGrzkTe9OWBQe8nwWVK+Gep41w==
@vsift/win32-x64@0.2.1-rc.2 sha512-5d2je0HlXSawzLtJBz/JmwC3SzSf1eWPvkPqrdUHnl0H/OdQbyWFxNcLWvwLhIEDXqJkVg1CjRYDzU1kSsUq7Q==
@vsift/darwin-arm64@0.2.1-rc.2 sha512-ZzmcQj152w8FwBZRdWicjpyZpdL/RmRWVOZV+puFnASQ59xoopZPs8ECiNeMI39/vlxvMDuxYJRgVLQ5mXjS0g==
@vsift/linux-x64@0.2.1-rc.2 sha512-WWlxZ3mWJwSHEjYAdv+EJUlqrQi8sabjNd01K0yoqloQCzqsyVOn7bym8VcD8PY4K+2pmWg5SkSRH5fH921luw==
vsift-cli@0.2.1-rc.3 sha512-pdNSwRnlPKo/Lwi/0d/Y6nCAOAu1oPy6iAbEr8mEoSMFz0a37tOjC/3+xegyBbobaof1kGqdJI0akUNHTfAv+A==
@vsift/win32-x64@0.2.1-rc.3 sha512-Y/YYp1kwa3m8SSgz1kcIYRKyKIsvzvYvSbqgFjmxgK6T+p4OwoI2A+B9Zybzk3W2oz2LwrfBIp2puNneUguhfQ==
@vsift/darwin-arm64@0.2.1-rc.3 sha512-DoaxYjC547Dfu0ONzHhyf8GhYhIhDweDCFZz1bCYwuui1d1nYzaA+0mByTZClnmMdZ9kmTNZmtyB4CIlnl0ETQ==
@vsift/linux-x64@0.2.1-rc.3 sha512-X16IPGZ7y0mkbWOlIPgfPPAW4laa/o5NHRen3v2xTf5MViD66Q1yrpxhfDIYm4G/Fm7DA1vze2Vduz1bcUZkAA==
vsift-cli@0.2.1 sha512-by2+8Cp4ITlvgvaxDDSJkwxEkOTvr6kQPVyLDYbnSptLBjxC1ogus/EEuLBtPxOUT1kMYqtUzjeYlSt7DbD6AA==
@vsift/win32-x64@0.2.1 sha512-aajNOxD0kWeApuHJ7HIMyqzKOtAKWC/eX7Aa8S4U/glP7lDkKYuawd0fkbuDmeiI3FZZxMg9rCj7/XaY3BMSUA==
@vsift/darwin-arm64@0.2.1 sha512-SqSgtq09WniVTBDHNCJvSZPQ4axJAfuW5DDG/O3N1n/mIwqq2oLyJCBv0NlngZVWnd7EsUE/adxnL/FWxi9vaA==
@vsift/linux-x64@0.2.1 sha512-bHXKLWx+1HdtRmv2gmU816XKqm5+zGp2xawA8Qj/e2IE9pnXOz1XsDfM9D7pQcRDAlhGpwynlrw+zPDJXECMIQ==
```

The eight tags are annotated (`git cat-file -t` says `tag`, each with the message `VSift <version>`) and name `d5792ce31db1106934233c86d0518c3ad1961e07` (`v0.2.0-rc.1`), `7c722d1fc46af7fddeffbaf807028eaec413ace1` (`v0.2.0-rc.2`), `83dca856e7a00fc9a71c87baae99f0b1d401dd31` (`v0.2.0-rc.3`), `eeb2a22a46a85ab10a456f2ab5d6a62e292836c5` (`v0.2.0`), `cb8cb3a6286047db56831459dabd6ee5500a37bd` (`v0.2.1-rc.1`), `23ad4927e56f11d8047ca54d59b892516812d8f8` (`v0.2.1-rc.2`), `fa380021f2aa8db40fdccdb7c410839e8fc10f4c` (`v0.2.1-rc.3`) and `e9ab6e62673328a1645254a4a32e552a2a2f61bf` (`v0.2.1`). No `release/*` branch and no `v0.2.2*` tag existed.

**`release-evidence --complete-for 0.2.2-rc.1` at the cut** (a reading, not a gate; it fails until the runs of step 7 are recorded) names 17 items: RQ-01 to RQ-04, RQ-06, RQ-13, RQ-18 and RQ-19 (`passed` for `0.2.1`, with 53 to 254 files of their scopes changed since), RQ-08, RQ-09, RQ-11 and RQ-12 (`passed` for `0.2.1-rc.3`, 126 to 128 files changed), RQ-05 (`failed`, #404) and RQ-07, RQ-10, RQ-15 and RQ-16 (`planned` by this cut: their waivers covered `0.2.1-rc.3` and `0.2.1` only, and a waived item would pass the check silently). It does not name RQ-14 and RQ-17 (standing waivers: the maintainer decides whether they carry to `0.2.2`) or RQ-20.

**Step 0, before the cut (the supervisor).** The pull request is merged (by the supervisor, on green) and nothing else is cut with it. Then, in a fresh clone or a clean worktree that is not the maintainer's:

```console
git fetch origin --tags
gh pr list --repo smormah/vsift --state merged --search "The release candidate 0.2.2-rc.1" --json number,title,mergeCommit --jq ".[] | {number, title, commit: .mergeCommit.oid}"
git show <the 40-digit commit>:Cargo.toml | grep -m1 "^version"
git merge-base --is-ancestor <the 40-digit commit> origin/main && echo "on main"
gh api "repos/smormah/vsift/commits/<the 40-digit commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.name == "CI result") | "\(.name): \(.status) \(.conclusion)"'
git ls-remote origin "refs/heads/release/*" "refs/tags/v0.2.2*"
for b in 2 3; do cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-$b/freeze.json; done
```

Expect `version = "0.2.2-rc.1"`, `on main`, `CI result: completed success` (wait for it if it is still running: **the branch ruleset refuses to create a branch at a commit whose `CI result` has not passed**, seen on 2026-10-17), nothing from `ls-remote` (no release branch and no `v0.2.2` tag yet) and `nothing frozen has changed` twice. Take the pull request whose title starts with "The release candidate 0.2.2-rc.1" and whose commit shows that version.

**Step 1, the nightly of exactly that commit (6.18 step 1).** If `git rev-parse origin/main` still prints the commit, dispatch the nightly on `main`; if `main` has moved on, do step 3 first and dispatch it on `release/0.2.x` (`--ref release/0.2.x`), whose head is the commit (a later commit's pass does not count):

```console
git rev-parse origin/main
gh workflow run nightly.yml --repo smormah/vsift --ref main
gh run list --repo smormah/vsift --workflow nightly.yml --commit <the 40-digit commit> --json databaseId,event,status,conclusion,startedAt,url
gh run watch <the run id> --repo smormah/vsift --exit-status
```

The run takes about 20 minutes and must end `success` with every job but `Report a failure` successful (a dispatched run always runs its checks). On a branch other than `main` a failure prints its report instead of filing an issue. A failure stops the cut: file the issue (governance rule 14), fix it on `main` by pull request, and cut from the fixed commit (no new bump is needed: `main` still carries `0.2.2-rc.1`). A scheduled nightly (02:41 UTC) counts too when it tests that commit.

**Step 2, choose the commit (6.18 step 2.2).** It is the merge commit of step 0, whose nightly passed in step 1. Check it once more: `gh run list --repo smormah/vsift --workflow nightly.yml --commit <the 40-digit commit> --json event,status,conclusion --jq ".[]"` lists a `success` as the newest run and nothing `in_progress` or `queued`.

**Step 3, create the release branch (6.18 step 2.3; the maintainer's administrator login, as for the tags):**

```console
git push origin <the 40-digit commit>:refs/heads/release/0.2.x
git ls-remote origin refs/heads/release/0.2.x
```

The second line prints the commit and `refs/heads/release/0.2.x`. From now on every direct push to the branch is refused ("Changes must be made through a pull request"). No earlier `release/0.2.x` exists, so there is nothing to delete and no `Release window` run is needed before this cut.

**Step 4, tag the commit (6.18 step 2.4; 6.3 step 2):**

```console
git tag -a v0.2.2-rc.1 -m "VSift 0.2.2-rc.1" <the 40-digit commit>
git push origin v0.2.2-rc.1
git show --no-patch --format="%H" "v0.2.2-rc.1^{commit}"
git fetch origin && git merge-base --is-ancestor <the 40-digit commit> origin/release/0.2.x && echo "on the release branch"
```

The third line prints the commit written down. A wrong tag before anything is published may be moved (`git push origin :refs/tags/v0.2.2-rc.1`, fix, push again; the `guard` and the publish's re-check require the tag to name the run's commit); never after step 6.

**Step 5, the dry run on the tag (6.18 step 2.5):**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --limit 3 --json databaseId,headBranch,headSha,event,status,conclusion,url
```

Pick the run whose `headBranch` is `v0.2.2-rc.1` and whose `headSha` is the commit. The `guard` job answers within a minute: the ref (the tag of the workspace version), the tag naming the commit, the commit on `release/0.2.x` with the five rule types, and the nightly of step 1. Its **Publish plan** must read "Version `0.2.2-rc.1` (release candidate), npm dist-tag `next`, Git tag `v0.2.2-rc.1`", the commit, `workflow_dispatch` on `refs/tags/v0.2.2-rc.1`, and for each of the four packages **`next` from `0.2.1-rc.3` to `0.2.2-rc.1` and `latest` staying `0.2.1`**; `plan-dry/release-notes.md` says "VSift 0.2.2-rc.1 is a release candidate ...". Download the plan (`gh run download <id> --repo smormah/vsift --name publish-plan --dir plan-dry`). Anything else: do not publish; tell the maintainer.

**Step 6, the publish (the maintainer approves):**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f dry_run=false
```

Pick the run as in step 5, download its plan into `plan-publish` and compare: `diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"`. Then **the maintainer** reviews the deployment and approves `release`; the supervisor never does. After the approval the `publish` job asks the `guard`'s questions again (a nightly of the commit that failed or still runs in the meantime, a moved tag or a branch that lost the commit or a rule refuses it), publishes the four packages under `next` and reads `next` back every 30 seconds for up to 20 minutes (a wrong answer fails at once). If it runs out of time with the packages published, **Re-run failed jobs** finishes it (6.5); that re-run asks the nightly again.

**Step 7, check it from outside, then the hosted runs.** From a shell with no npm login and a folder of its own (an empty `NPM_CONFIG_USERCONFIG` and `NPM_CONFIG_GLOBALCONFIG`, two different empty files): the four `npm view <package> dist-tags` print **`{ latest: '0.2.1', next: '0.2.2-rc.1' }` four times** and `npm view vsift-cli version` prints `0.2.1`; the 32 values above are unchanged:

```console
for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2 0.2.1-rc.3 0.2.1; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
for t in v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2 v0.2.1-rc.3 v0.2.1; do git show --no-patch --format="$t %H" "$t^{commit}"; done
mkdir rc-check && cd rc-check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures
npx vsift --version
gh release view v0.2.2-rc.1 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

`npm audit signatures` reports verified signatures and attestations; `npx vsift --version` prints `vsift 0.2.2-rc.1 (<the first 12 digits of the commit>)`; the installed skill equals the tag's (`diff -r skills/vsift rc-check/node_modules/vsift-cli/skills/vsift` from a checkout of the tag prints nothing); `gh release view` prints `isPrerelease` true, `isDraft` false and ten assets, and the last command **`v0.2.1`** (if it prints `v0.2.2-rc.1`: `gh release edit v0.2.2-rc.1 --repo smormah/vsift --latest=false`, then `gh release edit v0.2.1 --repo smormah/vsift --latest`). Every `gh attestation verify` is 6.4's with `--source-ref refs/tags/v0.2.2-rc.1`, `--signer-workflow smormah/vsift/.github/workflows/release.yml` and `--deny-self-hosted-runners`.

**The hosted runs** (the supervisor; each one dispatch, **on the tag** (6.18 step 3), once, on this candidate's published bytes or its source; nothing from `0.2.1-rc.3`'s or `0.2.1`'s runs is reused). The quick checks first, right after the publish:

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1
gh workflow run p14-journeys.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f published_version=0.2.2-rc.1
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1 -f from_version=0.2.1
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1 -f from_version=0.2.1-rc.3
```

Dispatch the second `P14 published artifacts` only after the first has finished (runs of one workflow on one ref share a concurrency group, and a newer waiting run replaces an older one without a word). `P14 verify release` (RQ-19) must say `next 0.2.2-rc.1, latest 0.2.1` for each package and `pre-release true, GitHub's latest release v0.2.1`. **`P14 journeys` (RQ-05) is the evidence for #404:** its Windows job's `p11_shutdown_and_redelivery` stage must pass. Then the long runs (6.18 step 3), one after another and staggered:

```console
gh workflow run p14-compatibility.yml --repo smormah/vsift --ref v0.2.2-rc.1
gh workflow run p14-local-upgrade.yml --repo smormah/vsift --ref v0.2.2-rc.1
gh workflow run p07-local-asr.yml --repo smormah/vsift --ref v0.2.2-rc.1
gh workflow run fuzz.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f seconds=3600
gh workflow run p14-stress.yml --repo smormah/vsift --ref v0.2.2-rc.1
gh workflow run p13-managed-power-loss.yml --repo smormah/vsift --ref v0.2.2-rc.1
gh workflow run p10-durability-campaign.yml --repo smormah/vsift --ref v0.2.2-rc.1
gh workflow run p14-load.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1
gh workflow run p14-malicious-media.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1
gh workflow run p14-runbook-walk.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1
gh workflow run p14-scan-reading.yml --repo smormah/vsift --ref v0.2.2-rc.1 -f version=0.2.2-rc.1
```

`Fuzz` runs 3,600 seconds a target, the budget of the earlier candidates; `P14 stress`, `P14 load`, `P13 managed power loss` and `P10 durability campaign` run with their workflows' defaults, as the earlier candidates' runs did (compare each with its ledger note for `0.2.1-rc.3` before dispatching). **`P14 malicious media` is RQ-10's first run under the rule of plan section 32** (the link's documented `STORAGE_IO` with the remediation that links are not followed passes; the judge pins it): a run with no finding records RQ-10 `passed` for this candidate. RQ-13 also needs a dated reading by hand (`fs4` 1.1.0 is new, and `process-wrap` gained its `creation-flags` feature). The Governance job of the `CI` run at the commit is RQ-18's. **A failed run is a finding: open an issue before re-running** (governance rule 14); a fix is a pull request into `release/0.2.x` with its counterpart to `main`, and it means `0.2.2-rc.2` (6.18 step 4).

**Step 8, agent-trial batches 2 and 3: the maintainer decided to run them on this candidate.** After the publish, on the maintainer's explicit go for each, one at a time, from a clean checkout at the commit (or any later commit of the branch: the freeze binds digests), against the registry's `0.2.2-rc.1`: the commands of [`p14-batch-2-3-checklist.md`](../planning/p14-batch-2-3-checklist.md) with `-Version 0.2.2-rc.1` (batch 3 with `-AllowGraderChange`), under the freeze committed by #435. `batch-2/` and `batch-3/` hold only that freeze; the campaign script refuses a state file recorded for another version, which is why the cut moved the `0.2.1-rc.3` records aside (a rename could not be made later: on the release branch a work-record path may be edited or added, never deleted, 6.8).

**Step 9, the records.** A records pull request **to `release/0.2.x`** records each run in the ledger from what it computed (RQ-01 to RQ-13, RQ-15, RQ-16, RQ-18, RQ-19), the dated scan reading, and the batches' records and readings, never earlier; the plan page gets a section "The evidence on 0.2.2-rc.1". The maintainer decides RQ-14 and RQ-17 and any finding. Then the stable commit `0.2.2` on the branch (6.18 step 5, with 6.7 and 6.17 as the model, written as its own planning page before the window closes), and after its publish the records change to `main`, which brings the branch's work record and the shipped documents and on which the `Release window` workflow runs by itself.

**If something fails.** 6.15's table applies with these readings: the `guard` refusing a dispatch names its reason (the wrong ref, a tag off the branch, a branch without its rule types, or the nightly: wait for a running one, or dispatch one and let it pass); step 5's plan must move `next` from **`0.2.1-rc.3`** (anything else: do not publish); the **Registry** guard failing ("`0.2.2-rc.1` is already on npm, and not with these bytes") means the number is used for good and the next candidate is `0.2.2-rc.2`; a bad published version is deprecated on all four packages (never unpublished) and `0.2.2-rc.2` is cut on the branch by 6.18 step 4.

**What is irreversible:** the four published `0.2.2-rc.1` versions and their provenance records; the Sigstore attestations of the ten files and four tarballs (they exist from the end of `attest`, before the approval); the tag once anything was published from it; and the 32 versions of `0.2.0-rc.1` to `0.2.1`, which this section does not change. **What is reversible:** `next` (`npm dist-tag add vsift-cli@0.2.1-rc.3 next` for each package, with the maintainer's login and second factor; the workflow never does it), the notes, and a deprecation.

### 6.20 The stable release `0.2.2`: the numbers, the command lines and what happened (the patch release, and the first stable version made on a release branch, 2026-10-18)

**A record (2026-10-18), and the model for the next stable version.** The maintainer's steps for this release were a planning page, [`release-0.2.2-stable-steps.md`](../planning/release-0.2.2-stable-steps.md), because this file could not change between the last candidate and the stable release (6.8). That page followed [`release-0.2.1-stable-steps.md`](../planning/release-0.2.1-stable-steps.md) (the page for `0.2.1`, which 6.17 records), 6.7, 6.8 and 6.18 step 5, with the numbers in the table below and the command lines under "The command lines that changed". This section folds in that table and those lines and records what happened when the page was followed and what the real services showed. **`0.2.2` is built from the same source as `0.2.2-rc.1`:** the stable commit differs from the tag `v0.2.2-rc.1` only in five version-string files, the two shipped documents and the work record (6.8; the plan's guard counted 5, 2 and 81 files). The stable commit is the merge of #456 into `release/0.2.x`, and the tag is `v0.2.2` (annotated, `VSift 0.2.2`) at `0dadc69df4f5c5c3264be72f04be26a600620fe1`. **For the next stable version,** write a planning page from this section, 6.7 and 6.18 before the window closes, with that version's numbers, and fold it in here after the publish. 6.19 is the record of the candidate; the steps below were followed once, on 2026-10-18.

**The numbers that changed from the model page (read 6.7, 6.17 and the model page with this table).**

| | `0.2.1` (the model page, 2026-10-15) | `0.2.2` (this release) |
| --- | --- | --- |
| The version, the tag, the commit | `0.2.1`, `v0.2.1`, the merge commit on `main` of "The stable release 0.2.1" | `0.2.2`, `v0.2.2` (annotated, message `VSift 0.2.2`), the merge commit on `release/0.2.x` of the pull request whose title starts "The stable release 0.2.2" (#456) |
| Where the commit and the tag are | `main` | `release/0.2.x` (the `guard` job refuses a tag its branch does not contain; a stable version needs no nightly of its own, 6.18) |
| The accepted candidate | `v0.2.1-rc.3` at `fa380021f2aa` | `v0.2.2-rc.1` at `43a033dbf81346b967b293a150e13306828cac50` (the only, so the highest, `v0.2.2-rc.N` tag; the check ignores every tag of another `X.Y.Z`) |
| What npm holds before | `latest` `0.2.0`, `next` `0.2.1-rc.3` | `latest` `0.2.1`, `next` `0.2.2-rc.1`, on all four packages |
| What the plan does | `latest` from `0.2.0` to `0.2.1` | `latest` from `0.2.1` to `0.2.2`; `next` stays `0.2.2-rc.1` (only the maintainer's own command moves it, [L-108](../planning/known-limits.md#l-108)) |
| The `latest moves forward` guard | `latest` is `0.2.0` on all four | `latest` is `0.2.1`, a stable version below `0.2.2`, and `0.2.2` is on none of them |
| The evidence ledger guard | complete for `0.2.1-rc.3` at `fa380021f2aa` | complete for `0.2.2-rc.1` at `43a033dbf813` (since #455) |
| Candidate-to-stable-version file counts | 5 version-string, 2 shipped-document, 77 work-record files | 5 version-string, 2 shipped-document, 81 work-record files |
| The read-back after the publish | five minutes (it ran out, #403) | up to 20 minutes (since #422) |
| The before-snapshot of the earlier versions | seven versions, twenty-eight values, seven tags | every published version: 45 integrity values and 10 tags |
| What `vsift --version` prints | `vsift 0.2.1 (e9ab6e626733)` | `vsift 0.2.2 (0dadc69df4f5)` |
| The GitHub release | becomes GitHub's latest, from `v0.2.0` | becomes GitHub's latest, from `v0.2.1` |
| The deprecations afterwards | `0.2.1-rc.1` and `0.2.1-rc.2` | `0.2.1-rc.3` and `0.2.2-rc.1`, message `Superseded by 0.2.2. Install vsift-cli.` (decided and done 2026-10-18, below) |
| The hosted checks after the publish | dispatched from `main` | dispatched on the tag `v0.2.2` (as for `0.2.2-rc.1`, 6.19); `P14 verify release` within seven days of the publish |
| Yarn 4 users | a day later | a day later (`npmMinimalAgeGate`) |

**The command lines that changed** (`<stable commit>` is the 40 digits of the merge commit; Git Bash, and in PowerShell write `"$t^^{commit}"` for `"$t^{commit}"`; the rest of the model page's steps, with no version in them, are as written there).

1. **Find the commit** (before the tag): `gh pr view <number> --repo smormah/vsift --json state,baseRefName,title,mergeCommit --jq "{state, baseRefName, title, commit: .mergeCommit.oid}"` (expect `MERGED` and `release/0.2.x`), then `git fetch origin --tags` and `git show <stable commit>:Cargo.toml | grep -m1 "^version"` prints `version = "0.2.2"`. On the release branch, after the candidate, every check passed: `git merge-base --is-ancestor <stable commit> origin/release/0.2.x && echo "on the release branch"`, `git merge-base --is-ancestor v0.2.2-rc.1 <stable commit> && echo "after the candidate"`, and `gh api "repos/smormah/vsift/commits/<stable commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'` prints nothing (the push to `release/0.2.x` runs `CI`). `git tag --list "v0.2.2-rc.*"` prints `v0.2.2-rc.1` only and `git tag --list "v0.2.2"` nothing.
2. **The delta check, by hand, at the commit** (the check is compiled from the commit it judges): `git switch --detach <stable commit>`, then `cargo run --locked -p vsift-release -- candidate-delta` (every file `version string only`, `shipped document` or `work record`, nothing `REFUSED`), then `git diff --stat v0.2.2-rc.1 <stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml` lists only `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/package.json` and `npm/vsift-cli/README.md`; `git diff -U0 v0.2.2-rc.1 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep -c "^[-+][^-+]"` prints `52` and the same pipe through `grep "^[-+][^-+]" | sed 's/^[-+]//; s/0\.2\.2-rc\.1/0.2.2/' | sort | uniq -c | awk '$1 % 2 == 1'` prints nothing; `git diff --name-only v0.2.2-rc.1 <stable commit> -- docs/planning/delivery-ledger.json docs/guide/reference docs/guide/files` prints nothing; and `git diff --name-only v0.2.2-rc.1 <stable commit> | grep -v -e "^CHANGELOG.md$" -e "^memory/" -e "^docs/decisions/" -e "^docs/history/" -e "^docs/planning/" -e "^docs/guide/"` prints exactly `Cargo.lock`, `Cargo.toml`, `docs/operations/install.md`, `fuzz/Cargo.lock`, `fuzz/Cargo.toml`, `npm/vsift-cli/README.md` and `npm/vsift-cli/package.json`. Then `git switch -`.
3. **The evidence ledger is complete for the candidate** (the plan's guard): `cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.2-rc.1 --commit 43a033dbf81346b967b293a150e13306828cac50` prints "VSift release evidence is complete for 0.2.2-rc.1 at 43a033dbf813." (since #455). It says the records are complete, not that the evidence is good enough: three items are `waived` for `0.2.2` (RQ-14, RQ-16 and RQ-17, [L-179](../planning/known-limits.md#l-179)).
4. **What npm holds now,** from a shell with no npm login (an empty `NPM_CONFIG_USERCONFIG` and `NPM_CONFIG_GLOBALCONFIG`, two different empty files): the four `npm view <package> dist-tags` print `{ latest: '0.2.1', next: '0.2.2-rc.1' }` and `npm view vsift-cli@0.2.2 version` answers 404. Write down every published version's bytes and the tags' commits: `for v in 0.0.0-stage 0.0.0 0.1.0 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2 0.2.1-rc.3 0.2.1 0.2.2-rc.1; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do i="$(npm view "$p@$v" dist.integrity 2>/dev/null)"; [ -n "$i" ] && echo "$p@$v $i"; done; done` prints 45 lines (`0.0.0-stage` exists for `@vsift/darwin-arm64` only; the 32 values of `0.2.0-rc.1` to `0.2.1` are those of 6.19, the other 13 are on the planning page), and `for t in v0.1.0 v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2 v0.2.1-rc.3 v0.2.1 v0.2.2-rc.1; do git show --no-patch --format="$t %H" "$t^{commit}"; done` prints `011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa` (`v0.1.0`), the eight commits of 6.19 and `43a033dbf81346b967b293a150e13306828cac50` (`v0.2.2-rc.1`).
5. **Tag, dry run, publish:** `git tag -a v0.2.2 -m "VSift 0.2.2" <stable commit>`, `git push origin v0.2.2`, `git show --no-patch --format="%H" "v0.2.2^{commit}"` (the 40 digits written down) and `git cat-file -t v0.2.2` (`tag`); a wrong tag may be moved only before anything is published. Dry run: `gh workflow run release.yml --repo smormah/vsift --ref v0.2.2 -f dry_run=true`; publish: the same with `-f dry_run=false`. Pick each run by its `headBranch` (`v0.2.2`) and `headSha`. The plan must list the six guards, all `passed` (the stable version; the accepted candidate against `v0.2.2-rc.1` with 5, 2 and 81 files; the registry read; the candidate published on all four packages; `latest` moving forward from `0.2.1`; the evidence ledger complete for `0.2.2-rc.1`), the table of dist-tags (`latest` from `0.2.1` to `0.2.2` and `next` staying `0.2.2-rc.1`, on each of the four packages), the `release_delta` record with `"verdict": "allowed"`, `"candidate_version": "0.2.2-rc.1"` and `"stable_version": "0.2.2"`, the four `npm publish ... --tag latest --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`, and the GitHub release commands (a draft, then published as latest, ten assets). Download the dry run's plan (`gh run download <id> --repo smormah/vsift --name publish-plan --dir plan-dry`), then the publish's (`plan-publish`), compare `attestation-subjects.sha256` and `release-assets.sha256`, and **keep `plan-publish/release-delta.json`: the artifact expires seven days after the run.** Then the maintainer approves the `release` environment.
6. **Check it from the outside** (no npm login, a folder of its own): the four `npm view ... dist-tags` print `{ latest: '0.2.2', next: '0.2.2-rc.1' }`; the loops of item 4 print the same 45 values and ten tag commits; `mkdir check-0.2.2 && cd check-0.2.2 && echo '{"private": true}' > package.json`, `npm install vsift-cli`, `npm audit signatures`, `npx vsift --version` prints `vsift 0.2.2 (0dadc69df4f5)`; `gh release view v0.2.2 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"` prints false, false and 10; `gh api repos/smormah/vsift/releases/latest --jq .tag_name` prints `v0.2.2`; the checksums and attestations are 6.4's commands with `v0.2.2`, `--source-ref refs/tags/v0.2.2`, `--signer-workflow smormah/vsift/.github/workflows/release.yml` and `--deny-self-hosted-runners`.
7. **Deprecate the versions the maintainer names** (after item 6 passed; the maintainer's login and second factor for each write; four writes a version): for each of `vsift-cli`, `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64`, `npm deprecate "<package>@<version>" "Superseded by 0.2.2. Install vsift-cli."`, and read back with `npm view "<package>@<version>" deprecated`; `0.2.2` and every version not named print what they printed before. An empty message removes a deprecation. The tags do not move.
8. **The hosted checks of the published bytes, on the tag** (none can publish; dispatch the second `P14 published artifacts` after the first has finished, because a newer waiting run replaces an older one): `gh workflow run p14-verify-release.yml --repo smormah/vsift --ref v0.2.2 -f version=0.2.2` **within seven days of the publish** (its delta check reads the publish run's artifact), `p14-journeys.yml` with `-f version=0.2.2`, `p13-managed-smoke.yml` with `-f published_version=0.2.2`, `p14-published-artifacts.yml` with `-f version=0.2.2 -f from_version=0.2.1` and then with `-f from_version=0.2.2-rc.1`, and `p14-scan-reading.yml` with `-f version=0.2.2`, each with `--ref v0.2.2`. A failed run is a finding: open an issue before re-running (governance rule 14).
9. **After the publish, in a pull request to `main` that may touch any page** (6.18 step 5; 6.8 binds the release branch only until the publish): copy `plan-publish/release-delta.json` into the ledger's `release_delta`, give the items whose rule for a stable version is `repeat` (RQ-01 to RQ-06, RQ-13, RQ-18, RQ-19) their own entries for `0.2.2`, and check with `release-evidence --complete-for 0.2.2 --commit <stable commit>`; bring the release branch's work record and the stable commit's shipped documents to `main` (the `Release window` workflow runs on that pull request by itself); fold the planning page in here; mark the candidate's section a record; correct the pages the release branch could not change (the root README, `SECURITY.md`, the skill guide, the trials guide, the development guide) and the register; rewrite the two handoff files. **Done by the records change of 2026-10-18** ([`release-0.2.2.md`](../planning/release-0.2.2.md), "The evidence on 0.2.2 (recorded 2026-10-18)").

**What happened.**

| Step | What happened |
| --- | --- |
| The stable commit | #456 merged into `release/0.2.x` as `0dadc69df4f5`, after the candidate's records (#455, `0f97a9b`); the `CI` run of the push (39125745195) passed, its Governance job included. |
| The tag | `v0.2.2`, annotated, at `0dadc69df4f5c5c3264be72f04be26a600620fe1`, on `release/0.2.x`. |
| Dry run on the tag | `workflow_dispatch` on `v0.2.2` with `dry_run` set, 2026-10-18 at 12:16 UTC, run 39126632838, success in eleven minutes: `attest` and `publish` skipped. |
| The real dispatch | The same dispatch with `dry_run` cleared, 12:33 UTC, run 39127747535, which is the run npm's provenance names (its attempt 1). The `guard` job passed in eight seconds (the tag of the workspace version, the tag naming the run's commit, the commit on `release/0.2.x` with its rule types; no nightly is asked of a stable version). The build, packaging, npm qualification, plan and `attest` jobs finished by 12:45 UTC, and the plan's `release-delta.json` was kept: it is the ledger's `release_delta`. |
| The six guards | The stable version; the accepted candidate (against `v0.2.2-rc.1` at `43a033dbf813`: 5 version-string, 2 shipped-document and 81 work-record files differ, nothing else); the registry read; the candidate published on all four packages; `latest` moves forward (`0.2.1` on all four); the evidence ledger (complete for `0.2.2-rc.1`). All `passed`. |
| The approval | The maintainer approved the `release` environment, and the `publish` job started at 14:59 UTC. Its re-check after the approval found that `v0.2.2` (on `release/0.2.x`) still met every condition. |
| `--tag latest` under trusted publishing | Accepted, as for `0.2.0` and `0.2.1`: the four packages were published from 14:59:36 to 15:00:01 UTC. |
| npm's read-back of `latest` | **Within the window, on the first attempt.** The job reads both dist-tags every 30 seconds for up to 20 minutes (since #422): `latest` read `0.2.2` on all four packages at the fourth read (15:01:40 UTC), about a minute and forty seconds after the last publish, with `next` `0.2.2-rc.1` unchanged. The GitHub release was created as a draft with ten assets and published as latest; the run ended `success` at 15:01:48 UTC, with no re-run. |

**Checked from outside, after the publish (2026-10-18):** `latest` is `0.2.2` and `next` is `0.2.2-rc.1` on all four packages; the 45 integrity values and the 10 tag commits of the earlier versions are unchanged; the GitHub release `v0.2.2` is not a pre-release, has ten assets and is GitHub's latest release; a plain `npm install vsift-cli` installs `0.2.2` and `vsift --version` prints `vsift 0.2.2 (0dadc69df4f5)`, with a verified registry signature and attestation; `gh attestation verify` passed on 3 of 3 archives. **The hosted checks were then run on `0.2.2`'s own bytes, all dispatched on the tag `v0.2.2`:** `P14 verify release` (39139002524, 22 checks passed, the two that a stable version adds among them), `P13 managed smoke` (39139007438), `P14 published artifacts` from `0.2.1` (39139010023) and from `0.2.2-rc.1` (39139382067, dispatched after the first had finished), `P14 scan reading` (39139013207) and `P14 journeys` (39139004783: every checkpoint passed on Ubuntu, Windows and macOS, the Windows shutdown stage of #404 included). The record is [`release-0.2.2.md`](../planning/release-0.2.2.md), "The evidence on 0.2.2 (recorded 2026-10-18)".

**Deprecation.** On 2026-10-18 `0.2.1-rc.3` and `0.2.2-rc.1` were deprecated on all four packages with the message `Superseded by 0.2.2. Install vsift-cli.` (item 7, eight writes): the maintainer decided it and logged in to npm, the supervisor typed the commands in the maintainer's terminal, and the maintainer approved each write with the second factor. The first attempt waited for that login: it had lapsed (point 4 below). Read back from outside the same day: both versions print the message on all four packages, and `0.2.2`, `0.2.1` and `0.2.0` print nothing. **`next` was not moved,** by the maintainer's decision, and the workflow never moves it ([L-108](../planning/known-limits.md#l-108)): `vsift-cli@next` installs `0.2.2-rc.1`, a pre-release of the same source, now with npm's deprecation warning. **#404 and #403 were closed by the supervisor the same day**, on the journeys of the candidate and of `0.2.2` (runs 39081998702 and 39139004783) and on the two bounded read-backs (publish runs 39078284454 and 39127747535).

**What this run showed about the steps, for the next stable version.** (1) **The 20-minute read-back of #403 held on its second real use** and on its first for `latest`: about a minute and forty seconds, where one package of `0.2.1` took more than five. (2) **The first stable version published from a release branch needed nothing the planning page did not say:** the `guard`, the re-check after the approval and the delta check accepted a tag on `release/0.2.x` with the candidate's tag as its ancestor. (3) **The records change after the publish is the first real test of the `Release window` check's content comparison** (6.18 step 5, [L-173](../planning/known-limits.md#l-173)): with `--records` it reports as missing from `main` the work-record lines that the records change rewrites as current state, and lines that one window commit added and a later window commit replaced; the pull request records why `main` differs. (4) An npm login that has lapsed is found at the deprecation step, after the publish; it delays the deprecations and nothing else, and `npm whoami` in the maintainer's terminal before item 7 finds it first. (5) No other problem with the steps was reported.

**What was not tried, and is not shown.** Yarn's behaviour for an untagged `yarn add vsift-cli` inside its one-day gate (the hosted jobs installed the exact version after the documented exemption); the long runs and the agent batches on the release's own tag (they are the candidate's, carried by the recorded delta); the upgrade from `0.1.0` and `P14 compatibility` on the stable tag (the candidate's runs are recorded); and everything the release ships untried: the adversarial evidence of the worker host's isolation (RQ-14), the cold-agent usefulness of a compact Claude model (RQ-16, #454) and the maintainer's try-outs (RQ-17, [L-143](../planning/known-limits.md#l-143)), each `waived` for `0.2.2` and none a pass. Nothing here announces anything.
