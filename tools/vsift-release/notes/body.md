## Install

The npm package is `vsift-cli`; the command it installs is `vsift`. It needs Node.js 22 or later, or Bun 1.2 or later.

```console
npm install --global {installed}
pnpm add --global {installed}
bun add --global {installed}
```

Yarn 4 holds back a version for a day after it is published (`npmMinimalAgeGate`). Wait a day, or list `vsift-cli` and `@vsift/*` under `npmPreapprovedPackages` in the project's `.yarnrc.yml`.

The executables are built for the three R0 targets: Windows 11 x64, macOS 15 on Apple silicon, and Linux x64 with glibc 2.35 or later and OpenSSL 3.

## Native archives

Each `vsift-{version}-<target>.tar.gz` holds the `vsift` executable, the licences, `THIRD-PARTY-NOTICES`, a CycloneDX SBOM and the agent skill; each target's SBOM and notices are also attached on their own. Check a download against `SHA256SUMS` (`sha256sum --check --ignore-missing SHA256SUMS`, or `shasum -a 256 --check --ignore-missing SHA256SUMS` on macOS). The executables are not code-signed or notarized, so Windows SmartScreen and macOS Gatekeeper may warn about one downloaded directly. Files installed through npm do not carry the download mark that triggers those two warnings, but Windows Smart App Control, where it is turned on, can block an unsigned program however it was installed; see the installation guide.

## Verify the provenance

Every archive, `SHA256SUMS`, SBOM, notices file and npm tarball has a Sigstore build-provenance attestation from the Release workflow:

```console
gh attestation verify <file> --repo {repository} \
  --signer-workflow {repository}/{workflow} \
  --source-ref refs/tags/{tag} --deny-self-hosted-runners
```

The npm packages also carry npm provenance: `npm audit signatures` in a project that installed `{installed}` checks their registry signatures and provenance attestations.

Installation guide: https://github.com/{repository}/blob/{tag}/docs/operations/install.md
