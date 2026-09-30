# Building and checking release archives

Status: maintainer runbook for the native release archives and the npm packages,
2026-09-30 (P13 PRs 8 and 9,
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
sections 1 and 2, decisions D and H5). **Nothing is published yet.** The workflow
described here builds, checks and packages the archives, assembles and qualifies the npm
packages (section 5), and keeps everything only as artifacts of its own run. Attestation
and publishing (the protected `release` environment, Sigstore build provenance, npm
trusted publishing) are P13 PR 10; this runbook gains that section then. The
installation guide for users is [`install.md`](install.md).

## 1. What the release workflow does

`.github/workflows/release.yml` (**Release**) has three archive jobs, and the two npm
jobs of section 5:

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
itself), and on manual dispatch. Never on a tag or a GitHub release.

**Why it cannot publish.** The workflow grants no job a write scope or an OIDC token
(`permissions: {}` at the top, `contents: read` per job), checks out without keeping
credentials, uses no secret, and has no step that talks to a registry or creates a
release. Its output is the run's own artifacts, which expire after 7 days. The
governance workflow lint (section 3) keeps it that way until PR 10 adds the `attest`
and `publish` jobs, the only jobs the lint will allow an OIDC token.

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
   key or value.

The lint reads the YAML tree, so flow mappings and aliases are seen, and it refuses
merge keys (`<<`). Comments are not part of the tree. Its tests are in
`tools/vsift-governance/src/workflows.rs`.

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
