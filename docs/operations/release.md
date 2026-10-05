# Building, checking and publishing releases

Status: maintainer runbook for the native release archives, the npm packages and their
publication, 2026-10-02 (P13 PRs 8, 9 and 10,
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
sections 1 and 2, decisions B, C, D and H5; P14 PR 8,
[ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md) decisions A and B,
the stable release and the release candidate). **0.1.0 was published with it on 2026-10-01**
(section 6; the record is [`p13-distribution.md`](../planning/p13-distribution.md), "First
publish"). **The stable path (section 6.7) has been built and tested but has never run
against the real services: no stable version is published, and `latest` is still the empty
`0.0.0` placeholder on all four packages.** The exact steps for the first release candidate,
`0.2.0-rc.1`, are section 6.10. The workflow described here builds, checks and
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

**When it runs.** On pull requests to `main` and pushes to `main` that change anything
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
cleared anywhere else fails the plan job rather than quietly doing a dry run. `publish`
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
     (`sort -V`); the `publish` job does not record the dist-tags before publishing, does
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
8. in **every** workflow (P14 PR 8): runs a command that moves a dist-tag (`npm` or
   `pnpm` followed by `dist-tag` or `dist-tags`, whatever sits between, quoting and spacing
   included; `yarn npm tag`; a call of the registry's `.../dist-tags/...` endpoint), or
   publishes a package (`npm`, `pnpm`, `yarn`, `bun` or `cargo` followed by `publish`)
   anywhere but the release workflow's `publish` job. Reads such as `npm view <package>
   dist-tags` are not refused. `latest` therefore moves only by publishing a stable
   version.

The lint reads the YAML tree, so flow mappings and aliases are seen, and it refuses
merge keys (`<<`). Comments are not part of the tree. Its tests are in
`tools/vsift-governance/src/workflows.rs` (rule 8 included) and, for rule 7,
`workflows/publish.rs`, which checks the real `release.yml` and a mutation of it for
every rule: 65 deliberately broken copies, 28 for the P13 rules and 37 for the channels,
the stable path's inputs and the evidence step, each of which the lint must name.

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
| `plan` | every run, after all twelve `npm-qualify` jobs | `contents: read` | Requires the downloaded archives to equal `package`'s `SHA256SUMS` output and the tarballs to equal `npm-package`'s `tarball-sums` output, by SHA-256. Reads npm's public metadata of the four packages with anonymous GETs (`https://registry.npmjs.org/<package>`, status and body kept in the runner's temporary folder; of the body only the dist-tags and each version's integrity are used, and nothing of it is printed or uploaded). Runs `vsift-release publish-plan`, which reads every archive back, re-assembles the npm packages and requires each tarball to be its package byte for byte, decides the mode and the channel (below), checks the guards of the channel and writes the plan: `publish-plan.md` (added to the job summary, even when the plan is refused), `publish-plan.json`, the release notes, the SBOM and notices of each target as separate release assets, and the digest lists of the release assets and of every file to attest. Checks out the full history, which the candidate comparison of a stable plan needs. Before the plan it names the accepted candidate (`vsift-release candidate-delta --github-output`) and, when there is one, runs the evidence ledger's completeness check for it (`vsift-governance release-evidence --complete-for <candidate> --commit <candidate commit>`), keeping the check's exit status and output in the runner's temporary folder for the plan to read (`--evidence`); the plan also receives the run's id and the date (`--run-id`, `--date`), which a stable plan that passes records in `release-delta.json` (6.7). Outputs `mode` (`dry-run` or `publish`), `version`, `tag`, `channel` (`prerelease` for a pre-release; for a stable version, the channel `stable`) and the plan's own digests. |
| `attest` | only when publishing | `id-token: write`, `attestations: write` | Downloads the archives, tarballs and plan, requires each to match the earlier jobs' outputs, and creates a Sigstore build-provenance attestation (`actions/attest-build-provenance` v4.2.2, pinned by commit) for every archive, `SHA256SUMS`, SBOM, notices file and npm tarball, named in the plan's `attestation-subjects.sha256`. |
| `publish` | only when publishing, after `attest`, in the `release` environment | `id-token: write`, `contents: write` | Waits for the maintainer's approval. Checks every file again, records the four packages' dist-tags, then with the npm of Node.js 24.21.0 (npm 11.19.0; trusted publishing needs 11.5.1 or later) runs, in this order, `npm publish ./npm-packages/<tarball> --access public --provenance --ignore-scripts` with `--tag next` (channel `prerelease`) or `--tag latest` (channel `stable`) for `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64` and then `vsift-cli`. Each channel has its own step, written out in full and run only for its channel; each checks the version's shape itself first, and the step of the stable release also requires every package's `latest` to be a stable version at or below the one published. It then reads both dist-tags of every package back: a pre-release moved `next` and left `latest` where it was, a stable version moved `latest` and left `next`. Last, `gh release create v<version> --verify-tag --draft` with the archives, `SHA256SUMS`, SBOMs and notices and the plan's notes (`--prerelease --latest=false` for a pre-release), and `gh release edit` publishes the draft (`--latest` for the stable release, which is then confirmed to be GitHub's latest release). |

**The mode and the channel.** `vsift-release publish-plan` answers `publish` only for a
`workflow_dispatch` with `dry_run` cleared, in `smormah/vsift`, on the ref
`refs/tags/v<workspace version>`. Pull requests and pushes are dry runs, and so is a
dispatch with `dry_run` set. A dispatch with `dry_run` cleared anywhere else fails the plan
job and names the reason. The channel comes from the version (section 1): with a
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

### 6.3 Publishing a release candidate (or another pre-release)

This is the procedure of 0.1.0 (2026-10-01) and the one for `0.2.0-rc.1` (P14 PR 10). It
moves `next` and nothing else; `latest` stays where it is. The stable version has its own
procedure (6.7), which starts from a published and accepted candidate.

1. **Choose the version and bump it.** The workspace version in `Cargo.toml` is the
   published version, and nothing else decides it. Bump it in the files that hold it, and
   in no other file (6.8 lists the same files, and the work record and documents in which a stable commit
   may also differ from its candidate): `Cargo.toml` (`[workspace.package] version` and
   the `version` of the five workspace dependencies), `Cargo.lock`, `fuzz/Cargo.toml`
   (four dependencies), `fuzz/Cargo.lock`, and `npm/vsift-cli/package.json` (its own
   `version` and the exact version of its three optional dependencies); update
   `CHANGELOG.md`'s release section. A candidate is `X.Y.Z-rc.N` with a positive `N`
   (`0.2.0-rc.1`, then `-rc.2` only if findings need it). Merge the bump to `main`. From
   the first candidate's tag on, only fixes for findings may change the code (6.8).

   **The user guide goes with the first bump to a new release** (`0.1.0` to `0.2.0-rc.1`),
   and only then. The guide names a release, not a candidate, and its two checks compare
   the release part of the version (`0.2.0`): so the bump to `0.2.0-rc.1` re-runs the
   guide's examples, regenerates its two reference pages and moves the version marker in
   `docs/guide/index.md` (the `Guide` workflow fails the bump until it does; steps in
   [Development](../development.md#the-user-guide-and-its-checks)), while `-rc.2` and
   the final `0.2.0` change nothing under `docs/guide/`. That is what keeps the commit that
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
npm view vsift-cli dist-tags        # latest: '0.0.0' (unchanged), next: '0.2.0-rc.1'
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
archives, `SHA256SUMS`, three SBOMs and three notices files. For the **stable** release
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
anything. A pull request that changes the workflow or `tools/p14-published/` runs it too. A
stable version fails it by name for now: the two checks it adds (the delta between the
candidate and the stable release, which reads the `release-delta.json` of 6.7 from the Release
run's `publish-plan` artifact, and `latest` on all four packages) are registered in
`tools/p14-published/lib/verify.cjs` before the stable publish, and a verification that skipped
them would be green about the wrong thing. Run 36969577300 verified 0.1.0: ten of ten files and four of four tarballs
attested, the provenance of all four packages read, and `npm audit signatures` verified the two
packages a Linux runner installs (the launcher and `@vsift/linux-x64`, npm 10.9.9; the count
npm reports depends on its version, which is why the provenance check reads all four packages
itself).

A version that `npm publish` has just reported can take a minute or more to appear in
`npm view` ("Your package is being processed and may take a few minutes to become
available"); on 2026-10-01 the publish job's check for `next` polled three times, 30
seconds apart, before it saw 0.1.0, and it waits up to five minutes. Nothing is wrong if
your own `npm view` is empty for a moment.

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

**This path has never run against the real services** (known limit
[L-105](../planning/known-limits.md#l-105)). It is built so that a mistake stops before
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
| `latest` moves forward | on every package `latest` is now a stable version below this one (or already this version with the same bytes: a re-run completing a partial publish); this version is on none of them under another tag or with other bytes | `guards.rs`; the publish job's own check just before publishing (lint: `sort -V` line) |
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
nothing copies it for you ([L-103](../planning/known-limits.md#l-103)).

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
   **npm publish** allowed, **npm dist-tag** not). `--tag latest` is part of `npm publish`,
   but it has never gone through trusted publishing (only `--tag next` has), so this is the
   one setting a first stable publish can still find wrong ([L-100](../planning/known-limits.md#l-100),
   [L-105](../planning/known-limits.md#l-105)).
2. **The `release` environment and the tag ruleset** are as 6.2 steps 3 and 4 say (read-only
   `gh api repos/smormah/vsift/environments/release` and `.../rulesets`), and fork-pull-request
   approval is "all external contributors".
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
   document or a work record (6.8), and nothing is marked REFUSED.
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
   five minutes for `latest` to read back as this version on all four packages and for
   `next` to be unchanged; creates the GitHub release as a draft, publishes it marked
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
   (download the run's `publish-plan` artifact within its 7 days).

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

### 6.8 The candidate rule and the candidate-to-stable-version check (P14 PR 8; the lists settled in PR 10b)

**The rule** (ADR 0024 decision B). From the cut of a release candidate only fixes for
findings may change the code: no features, no refactors, no unrelated cleanup. A fix that
changes code means a new candidate (`rc.2`) and the qualification of what it touches; the
stable release is built on the last candidate. **The check** (decision A) is the
mechanical form: the stable commit must differ from its accepted candidate only in the stable
release's own version strings, the documents the release publishes and the work record.

- **The accepted candidate** is the highest-numbered `v<X.Y.Z>-rc.<N>` tag for the stable
  release's own `X.Y.Z`; any other tag is ignored. Only you can create a `v*` tag (the tag ruleset).
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
- **Everything else is refused**, and so is a listed path that is added (a version-string
  file or a shipped document), deleted, linked, re-moded or renamed rather than edited. So are a
  candidate that is not an ancestor of the stable commit and a stable commit that is the
  candidate's own. In particular, **from the moment the candidate's tag exists until the stable
  release is published, nothing may be merged that touches the code, a dependency or a
  workflow:** no Dependabot pull request (they change `Cargo.toml`, `Cargo.lock` or a workflow),
  no change to a qualification tool or workflow, to the README, to `SECURITY.md` or to any
  other operator document. If one is needed, the cure is a second candidate (`rc.2`) with the
  change, not a longer list. Every other repository page changes after the stable release is
  published, in the ledger follow-up (P14 PR 13), not in the stable commit.
- The lists are constants in `tools/vsift-release/src/candidate.rs`, and a test spells them
  out and a second test reads this section and requires every entry to be named in it, so
  changing one is visible in a diff and in the documentation. A third test applies the check to
  one path of every area that must stay refused (the crates and the managed catalogue, the
  schemas, the fixtures, the skill, the trial harness, the scenarios, the settings, the
  release tool, every other tool, every workflow, the launcher's code, the other documents, the
  siblings of a record directory) and to every allowed kind, and a fourth breaks a copy of the
  lists in many ways (a record directory that covers `crates/`, a missing trailing slash, an
  emptied protected list, the skill added as a shipped document) and requires the test to
  notice. They are code, and code is frozen at the cut: change them before the candidate is
  cut or cut another candidate.
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
  candidate to be the accepted one.

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
whatever the check answers (56 checks in all). Run it with `bash tools/vsift-release/tests/publish-steps.sh
.github/workflows/release.yml` (Linux, or Git Bash on Windows); the Rust test
`publish_steps` runs it on Linux in CI. No real service is contacted. It cannot prove that
npm's trusted publishing accepts `--tag latest`, or that `gh release edit --latest` marks a
draft latest on GitHub: those are what the first stable publish tests
([L-105](../planning/known-limits.md#l-105)).

### 6.10 The first release candidate, `0.2.0-rc.1`: the exact steps (P14 PR 10c)

For the person doing it once, in order. It is 6.3 and 6.4 with the real commands, the output to expect and what to do when it
differs. It takes about an hour, most of it waiting for builds and the registry. **A candidate is never announced** and moves
`next` only: `latest` stays the empty `0.0.0` placeholder on all four packages at every step. Times are the maintainer's.

**What you are about to do cannot be undone.** A version published to npm can never be published again, even if it is removed;
a tag you push is a record that the ruleset keeps from moving once anything was published from it. If a step below fails
before `npm publish` has run, nothing is lost and you can repeat it. After it, only the "If something fails" list applies.

**0. Before you start (about ten minutes, nothing here changes anything).**

1. Merge nothing else. From the moment the tag exists until the stable release is published, **nothing may change on `main`
   but the work record, the installation guide and the launcher's README** (6.8 lists them exactly): no Dependabot pull
   request, no workflow edit, no dependency bump, no change to a tool or to any operator document. The stable release is
   refused otherwise, and the cure is a second candidate. Dependabot opens new pull requests on Mondays: leave them open.
2. The "Require branches to be up to date before merging" rule on `main`, which was turned off to speed merging, is **back on**
   (Settings, Branches, the rule for `main`). Read-only check: `gh api repos/smormah/vsift/branches/main/protection --jq
   .required_status_checks.strict` prints `true`.
3. `main` is at the commit you mean to release: pull requests 10a and 10b are merged and nothing is open that you expect to merge.

   ```console
   git fetch origin --tags
   git switch main
   git pull --ff-only
   git log -1 --format="%H %s"
   grep -m1 "^version" Cargo.toml
   ```

   Expect the 10b merge commit and `version = "0.2.0-rc.1"`. Write the full 40-digit commit down: it is what you tag.
4. The checks that bind this commit pass on your machine (about a minute each, after the first build):

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
5. The trusted publishers are still saved on npmjs.com for all four packages (6.2 step 6: a saved entry listed, owner `smormah`,
   repository `vsift`, workflow `release.yml`, environment `release`, **npm publish** allowed). This is the one setting no dry
   run can check (L-100); `--tag next` has gone through it for 0.1.0, so it is expected to hold.
6. The `release` environment and the tag ruleset exist as 6.2 steps 3 and 4 say (read-only):

   ```console
   gh api repos/smormah/vsift/environments/release --jq '{reviewers: [.protection_rules[]? | .reviewers[]? | .type], branch_policy: .deployment_branch_policy, can_admins_bypass: .can_admins_bypass}'
   gh api repos/smormah/vsift/rulesets --jq '.[] | {name, target, enforcement}'
   ```

   Expect `{"branch_policy":{"custom_branch_policies":true,"protected_branches":false},"can_admins_bypass":false,"reviewers":["User"]}`
   (a required reviewer, administrators cannot bypass, custom policies for tags only) and `{"enforcement":"active","name":"release
   tags","target":"tag"}`. These were the answers on 2026-10-05.
7. What npm holds now, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`):

   ```console
   npm view vsift-cli dist-tags
   npm view @vsift/win32-x64 dist-tags
   npm view @vsift/darwin-arm64 dist-tags
   npm view @vsift/linux-x64 dist-tags
   ```

   Expect `{ latest: '0.0.0', next: '0.1.0' }` four times. Remember it: step 5 compares.

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
gh run list --repo smormah/vsift --workflow release.yml --limit 1
gh run watch <the run id> --repo smormah/vsift
```

It takes about 30 to 40 minutes: 21 jobs, `attest` and `publish` skipped. Open the run's **Publish plan** summary and read it
top to bottom. Expect:

- the heading `Publish plan: dry run, nothing is published` and "This run was dispatched with `dry_run` set";
- `` `latest` is not touched. This release candidate is published under `next` only ``;
- "Version `0.2.0-rc.1` (release candidate), npm dist-tag `next`, Git tag `v0.2.0-rc.1`", the commit you wrote down and "The plan
  is enforced" (a dispatch on the tag is the rehearsal of a publish);
- the table "What this publication does to the dist-tags": for each of the four packages `next` from `0.1.0` to `0.2.0-rc.1`
  and `latest` staying `0.0.0`;
- the guard **Registry**: passed ("this version is not `latest` and is not on npm with other bytes on any package");
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
issue before re-running** (governance rule 14). Do not wait for them to start the hour of step 4 over.

**6. Afterwards.** Tell the supervisor: the run ids of the dry run and the publish, the four `npm view` answers, the output of
`npm audit signatures`, and anything that failed or was re-run. Nothing is announced. **Batches 2 and 3 of the agent trials
start only on your explicit go** (`docs/agents/trials.md`, "The P14 batches"); their freeze is already committed
(`docs/planning/p14-agent-trials/batch-2/freeze.json` and `batch-3/`) and `prepare` refuses a trial if the skill, grader,
scenarios or settings differ from it.

**If something fails.**

| When | What it means | What to do |
| --- | --- | --- |
| Step 0 check fails | the commit is not what was reviewed | do not tag; tell the supervisor |
| Step 2 fails or is refused | nothing was attested or published | read "Refused because:", fix it, move the tag if the commit changes (nothing was published from it) |
| Step 3: `publish` fails with `ENEEDAUTH` before any `+ package@version` line | a trusted publisher is not saved or is wrong | 6.5 first bullet of `ENEEDAUTH`; nothing was published |
| Step 3: `publish` fails after some packages | a partial publish ([L-097](../planning/known-limits.md#l-097)) | **Re-run failed jobs** on the same run within seven days and approve again: versions npm holds with the same bytes are skipped |
| Step 4: `next` is not `0.2.0-rc.1` after ten minutes | the registry is slow, or the publish stopped | look at the `publish` job's log before doing anything |
| Step 4: `latest` is not `0.0.0` | the workflow is wrong | 6.5 "`latest` moved when it should not have": `npm dist-tag add <package>@0.0.0 latest` for each package that moved, then open an issue; **do not dispatch again** |
| A published version is bad | the candidate itself is wrong | **never unpublish** (a version number can never be used again, and the attestations name its bytes): `npm deprecate vsift-cli@0.2.0-rc.1 "<what is wrong; use 0.2.0-rc.2>"` on all four packages, edit the GitHub release's notes to say so, and publish `0.2.0-rc.2` by this section again (a second candidate is planned for if findings need it; a third is your call) |

**What is irreversible, in one list.** The four published versions and their provenance records on npm's transparency log; the
Sigstore attestations of the ten files and four tarballs; the tag once anything was published from it; the release page's
existence (its notes may be edited). **What is reversible:** `next` (`npm dist-tag add vsift-cli@0.1.0 next`, for each package,
two-factor authentication; the workflow never does it), the notes, a deprecation (`npm deprecate <package>@<version> ""` removes
it).
