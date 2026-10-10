# P14 published-artifact qualification tools

Plain Node.js (CommonJS, no dependency) tools that the `P14 published artifacts`,
`P14 local upgrade` and `P14 verify release` workflows run on hosted runners (P14 PR 2; ADR 0024;
`docs/planning/p14-qualification.md` section 2). They install and run the **published** VSift:
nothing is built from source except in the local upgrade mode, nothing is published, and the only
credential in reach is the job's default read-only GitHub token, which only `gh` sees.

| Script | Evidence | What it does |
| --- | --- | --- |
| `resolve-release.cjs` | | Chooses the versions (default: the highest published; upgrade baseline 0.1.0) and the tags' commits; job outputs |
| `clean-install.cjs` | RQ-01 | One package manager on one system, from the real registry, in a scrubbed environment: global and one-shot installs, scripts disabled, optional dependencies omitted, hostile names and arguments through every shim, SEC-02 with planted tools, `npm audit signatures`, `gh attestation verify` |
| `archive.cjs` | RQ-02 | The three release archives: checksums, attestations, extraction, `--version` and `setup check --json` with no Node.js on `PATH`, the shipped skill against the tag's |
| `offline-install.cjs` | RQ-03 | The real reviewed artifacts fetched by the published binary's own plan, then `setup install --artifact-dir` in a container with no network; the tamper, missing-file and relative-folder refusals |
| `upgrade.cjs` | RQ-04 | Install the from-version, make sessions and a configuration, upgrade, compare, and walk `install.md` section 8. `--mode registry` (real registry) or `--mode local` (a loopback registry serving the pull request's own build) |
| `assemble-local-packages.cjs` | RQ-04 | The four packages of the local mode: the checkout's launcher, the published platform packages as a skeleton, the pull request's executables |
| `verify-release.cjs` | RQ-19 | The second verification of a publish: dist-tags, npm provenance, signatures, attestations, checksums, release flags and files; for a stable version also the candidate-to-stable delta (read from the publish run's `publish-plan` artifact, which GitHub keeps seven days) and `latest` on all four packages |

`lib/` holds the shared pieces, each with a test: `scrub.cjs` (the scrubbed `PATH`), `hostile.cjs`
(the hostile names and arguments), `sec02.cjs` (planted tools), `verify.cjs` (the rules of RQ-19 and
where a stable version's two extra checks register), `release.cjs`, `download.cjs`, `upgrade.cjs`,
`archive.cjs`, `managers.cjs`, `local-registry.cjs`, `common.cjs`.

```console
node --test "tools/p14-published/test/*.test.cjs"
```

The pins (Node.js, Bun, pnpm, Yarn, Verdaccio, the actions, the Ubuntu image) are the Release
workflow's; `test/pins.test.cjs` holds them equal and holds every P14 workflow and tool to "cannot
publish". `invoke.ps1` is a copy of `npm/qualification/invoke.ps1`, kept here so that a change to
the tools does not start the Release workflow.
