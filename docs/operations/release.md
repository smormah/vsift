# Building, checking and publishing releases

Status: maintainer runbook for the native release archives, the npm packages and their
publication, 2026-10-01 (P13 PRs 8, 9 and 10,
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
sections 1 and 2, decisions B, C, D and H5). **0.1.0 was published with it on 2026-10-01**
(section 6; the record is [`p13-distribution.md`](../planning/p13-distribution.md), "First
publish"). The workflow described here builds, checks and packages the archives, assembles
and qualifies the npm packages (section 5), and on every run writes the publish plan and
shows it (a dry run). It publishes only when the maintainer dispatches it on a release tag
with `dry_run` cleared and then approves the protected `release` environment (section 6).
The maintainer's one-time setup and the publish are section 6 too. The installation guide
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
   - `npm publish` runs outside `publish`, or without `--provenance` and `--tag next`, or
     names `latest`; `gh release` runs outside `publish`, or creates a release without
     `--verify-tag`, `--prerelease` and `--latest=false`; any job runs `npm dist-tag`,
     `npm unpublish`, `npm deprecate`, `gh release delete`, `git tag` or `git push`;
   - `npm-package` does not export `tarball-sums`, or `npm-qualify`, `plan`, `attest` or
     `publish` does not check its tarballs against that output with
     `sha256sum --check --strict`; `attest` or `publish` uses an action other than its
     few reviewed ones, downloads anything but this run's `release-dry-run`,
     `npm-packages` or `publish-plan` artifact (no pattern, run id, repository or token),
     or runs `cargo`, `vsift-release`, `npm pack`, `npm install`, `npm ci`, `npx`, `pnpm`
     or `yarn`;
   - the workflow names any secret but `NPM_BOOTSTRAP_TOKEN`, or names that one outside
     `publish`.

The lint reads the YAML tree, so flow mappings and aliases are seen, and it refuses
merge keys (`<<`). Comments are not part of the tree. Its tests are in
`tools/vsift-governance/src/workflows.rs` and, for rule 7, `workflows/publish.rs`, which
checks the real `release.yml` and a mutation of it for every rule.

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

The same workflow turns the archives into the four npm packages and qualifies them. Both
jobs run after `package`, and neither can publish: they have `contents: read`, no OIDC
token and no secret, and the only registry they write to runs on the job's own loopback
address and is gone when the job ends.

| Job | Runner | What it does |
| --- | --- | --- |
| `npm-package` | `ubuntu-24.04` | `vsift-release npm` reads the three archives back (each must be canonical) and writes the package folders; `npm pack` packs each twice and the two tarballs must be identical; `vsift-release npm-verify` checks every tarball against a fresh assembly; keeps the four tarballs as the run's `npm-packages` artifact for 7 days. |
| `npm-qualify` | `windows-2025`, `macos-15`, `ubuntu-24.04`, each with npm, pnpm, Yarn and Bun (12 jobs) | Installs the pinned Verdaccio and package manager from the public registry (read-only, no credentials), then runs `npm/qualification/qualify.cjs`, which publishes the tarballs to Verdaccio on `127.0.0.1:4873` and qualifies the package manager against it (ADR 0023, PR 9 note). The job summary lists every check. |

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
the maintainer's placeholder, is `latest` and stays `latest` until a stable release; the
0.x pre-release is published with `--tag next`, so release notes tell users to install
`vsift-cli@next`. Yarn 4.18 holds every new version back for a day
(`npmMinimalAgeGate`), for `vsift-cli` and `@vsift/*` alike: say so in the release notes,
with the `npmPreapprovedPackages` workaround of `install.md`.

## 6. Attestation and publishing (P13 PR 10): the maintainer's runbook

**Status: sections 6.2 to 6.4 were completed on 2026-10-01 for 0.1.0** (the record
[`p13-distribution.md`](../planning/p13-distribution.md) says how each was checked): the
one-time setup, the tag `v0.1.0`, a dry run on it, two real dispatches (the first failed
with `ENEEDAUTH` before anything was published, the second published; see 6.2's preflight
and 6.5) and the verification. They are written below as the steps for the next release.
Every setting below is the maintainer's to make (ADR 0023, "Maintainer-only actions"); no
agent or workflow changes them. Nothing here needs an e-mail address or other personal
detail beyond the public GitHub owner `smormah` and repository `vsift`, and no step asks
for one.

### 6.1 The three publishing jobs

| Job | Runs | Permissions | What it does |
| --- | --- | --- | --- |
| `plan` | every run, after all twelve `npm-qualify` jobs | `contents: read` | Requires the downloaded archives to equal `package`'s `SHA256SUMS` output and the tarballs to equal `npm-package`'s `tarball-sums` output, by SHA-256. Runs `vsift-release publish-plan`, which reads every archive back, re-assembles the npm packages and requires each tarball to be its package byte for byte, decides the mode (below) and writes the plan: `publish-plan.md` (added to the job summary), `publish-plan.json`, the release notes, the SBOM and notices of each target as separate release assets, and the digest lists of the release assets and of every file to attest. Outputs `mode` (`dry-run` or `publish`), `version`, `tag` and the plan's own digests. |
| `attest` | only when publishing | `id-token: write`, `attestations: write` | Downloads the archives, tarballs and plan, requires each to match the earlier jobs' outputs, and creates a Sigstore build-provenance attestation (`actions/attest-build-provenance` v4.2.2, pinned by commit) for every archive, `SHA256SUMS`, SBOM, notices file and npm tarball, named in the plan's `attestation-subjects.sha256`. |
| `publish` | only when publishing, after `attest`, in the `release` environment | `id-token: write`, `contents: write` | Waits for the maintainer's approval. Checks every file again, then with the npm of Node.js 24.21.0 (npm 11.19.0; trusted publishing needs 11.5.1 or later) runs, in this order, `npm publish ./npm-packages/<tarball> --tag next --access public --provenance --ignore-scripts` for `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64` and then `vsift-cli`; requires `vsift-cli`'s `next` to be the new version and `latest` not to be; then `gh release create v<version> --verify-tag --draft --prerelease --latest=false` with the archives, `SHA256SUMS`, SBOMs and notices and the plan's notes, and publishes the draft. |

**The mode.** `vsift-release publish-plan` answers `publish` only for a
`workflow_dispatch` with `dry_run` cleared, in `smormah/vsift`, on the ref
`refs/tags/v<workspace version>`, for a 0.x version or a pre-release. Pull requests and
pushes are dry runs, and so is a dispatch with `dry_run` set. A dispatch with `dry_run`
cleared anywhere else fails the plan job and names the reason. A stable version
(1.0.0 or later without a pre-release part) is refused in every mode: its dist-tag
(`latest`) and release are P14's to plan, and this workflow never moves `latest`.

**What is published is what was qualified.** `npm-package` packs the tarballs once and
exports their SHA-256 list as a job output, which no later job can change. Every
`npm-qualify` job requires the tarballs it installs to match that list, and so do
`plan`, `attest` and `publish`, which download only this run's artifacts. `attest` and
`publish` check out no code and build, pack or install nothing; they run a few lines of
shell and two reviewed actions. `tools/vsift-release`'s test
`the_release_workflow_runs_the_planned_commands` holds those lines to the plan's
commands word for word, and the lint (section 3, rule 7) holds the rest.

**What the dry run shows.** Open a Release run and its **Publish plan** job summary: the
mode and why, the version, dist-tag and Git tag, and three tables, each file with its
SHA-256: the files to attest, the four `npm publish` commands in order, and the GitHub
release command with its assets. The same plan is the run's `publish-plan` artifact.

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

### 6.3 Publishing the pre-release

1. **Choose the version.** The workspace version in `Cargo.toml` is the published
   version (0.1.0 today); `npm/vsift-cli/package.json` and its three optional
   dependencies must carry the same (section 5). It must be a 0.x version or a
   pre-release. Update `CHANGELOG.md`'s release section and merge.
2. **Create the tag** on the merged commit of `main` and push it:

   ```console
   git tag -a v0.1.0 -m "VSift 0.1.0" <commit>
   git push origin v0.1.0
   ```

3. **Dry run on the tag.** Actions, **Release**, **Run workflow**, "Use workflow from"
   **Tags: v0.1.0**, leave **dry_run** ticked. When it finishes, read the **Publish
   plan** summary: mode `dry-run`, the version, `next`, and the digests.
4. **Publish.** Run it again on the same tag with **dry_run cleared**. The plan job must
   say **PUBLISH**, `attest` runs, and `publish` waits with "Waiting for review". Check
   the plan summary once more, then **Review deployments**, tick `release`, **Approve
   and deploy**. The run waits in the environment as long as you take (a wait of about 45
   minutes did no harm on 2026-10-01), but a run waits for approval for at most 30 days
   and its files, the artifacts every job downloads, expire after 7: approve within a week,
   or the run cannot publish and you dispatch again.
5. **Verify** (section 6.4). For 0.1.0 the ledger follow-up (P13 PR 12) recorded the runs
   in the qualification record's "First publish" section
   ([`p13-distribution.md`](../planning/p13-distribution.md)), marked ADR 0023 Accepted and
   set P13 complete; for a later release, bring the run's link, the output of the section
   6.4 commands and anything that failed or was re-run to the record that release belongs
   to.

### 6.4 Verifying provenance and attestations after publishing

From a machine with no npm credentials (an empty user and global configuration, as in
ADR 0009's name checks):

```console
npm view vsift-cli dist-tags        # latest: '0.0.0', next: '0.1.0'
npm view vsift-cli@next dist.attestations
mkdir check && cd check && echo '{"private": true}' > package.json
npm install vsift-cli@next
npm audit signatures                # registry signatures and provenance attestations verified
npx vsift --version                 # vsift 0.1.0 (<the tag's first 12 commit digits>)
```

npmjs.com shows each version as "Built and signed on GitHub Actions", linking the
workflow run. For the GitHub release and the tarballs (`gh` signed in):

```console
gh release download v0.1.0 --repo smormah/vsift --dir release
cd release && sha256sum --check --strict SHA256SUMS
for file in *; do
  gh attestation verify "$file" --repo smormah/vsift \
    --signer-workflow smormah/vsift/.github/workflows/release.yml \
    --source-ref refs/tags/v0.1.0 --deny-self-hosted-runners
done
npm pack vsift-cli@0.1.0 @vsift/win32-x64@0.1.0 @vsift/darwin-arm64@0.1.0 @vsift/linux-x64@0.1.0
# ... and the same gh attestation verify for each .tgz
```

The release must be a pre-release that is not marked latest, with ten assets: three
archives, `SHA256SUMS`, three SBOMs and three notices files.

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
  new version.
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
- **`latest` moved** (the job says so and stops before the GitHub release): point it
  back yourself with `npm dist-tag add vsift-cli@0.0.0 latest` (two-factor
  authentication), then re-run the failed job. The workflow never runs `npm dist-tag`.
- **A draft release was left behind** by a failed run: delete the draft on GitHub, then
  re-run.
- **A published version is bad:** do not unpublish it (npm refuses to reuse the version
  number); deprecate it with `npm deprecate` yourself and publish a fixed version.

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
  Quality, Documentation, dependency policy and review, Rust analysis and Governance).
- Settled (2026-10-01): the generated release notes (`tools/vsift-release/src/publish.rs`)
  no longer say that installing through npm avoids the warnings outright. They say that
  files installed through npm do not carry the download mark that triggers SmartScreen
  and Gatekeeper, and that Windows Smart App Control, where it is on, can still block an
  unsigned program however it was installed, with a pointer to the installation guide
  ([L-098](../planning/known-limits.md#l-098), `install.md` section 4).
