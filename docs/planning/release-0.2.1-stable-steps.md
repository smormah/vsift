# The stable release `0.2.1`: the maintainer's steps

Status: **followed on 2026-10-15; folded into [`release.md`](../operations/release.md) 6.17 on 2026-10-16, which holds the numbers table, the command lines and what happened (the publish needed a second attempt after a read-back timed out, #403).** This was the maintainer's checklist for tagging, publishing and checking `0.2.1`, the patch release after `0.2.0`, and is kept as it was prepared. It was
a planning page and not a section of [`release.md`](../operations/release.md) for the reason the page for `0.2.0` was: `release.md` may not change between the last candidate and the
stable release (6.8 lists what may), so the runbook section for this release, **6.17**, is written here and folded into `release.md` after the publish (the last section). **Follow
[`p14-stable-release-steps.md`](p14-stable-release-steps.md) (the page for `0.2.0`, which `release.md` 6.13 records, and which is the model) and `release.md` 6.7 and 6.8, with the numbers in the table
below and the command lines under "The command lines that change", which are spelled out so that they can be pasted.** Everything the model page says that this page does not repeat is unchanged: who does what, the order of the steps,
what the dry run's plan must list, the comparison of the two plans, the approval, the failure table and the list of what is irreversible.

**Plain English first.** `0.2.1` is built from exactly the source of `0.2.1-rc.3`, the candidate published on 2026-10-13 (tag `v0.2.1-rc.3` at `fa380021f2aa8db40fdccdb7c410839e8fc10f4c`): the stable commit differs from that tag only in its version
numbers, the launcher's README, the installation guide and the work record, and a program checks that. Publishing it moves npm's `latest` from `0.2.0` to `0.2.1` on all four packages; after that, `npm install vsift-cli` with no tag installs
`0.2.1`. `next` is not moved (it stays `0.2.1-rc.3`; only the maintainer's own command moves it: [L-108](known-limits.md#l-108)). A published version can never be taken back. Afterwards `0.2.1-rc.1` and `0.2.1-rc.2` are deprecated (the maintainer's decision of 2026-10-15); `0.2.1-rc.3` is not.

**The runbook's record for the candidate.** `release.md` 6.16 (the third candidate's steps) was followed once, on 2026-10-13 and 2026-10-14: the dry run was run 38455902954, the publish was run 38457712965, `next` moved from `0.2.1-rc.2` to `0.2.1-rc.3`, and `latest` stayed `0.2.0`. It is a record and is not to be followed again; sections 6.10 to 6.16 are records of past publishes.

## The numbers that change (read the model page with this table)

| | `0.2.0` (the model page, 2026-10-09) | `0.2.1` (this page) |
| --- | --- | --- |
| The version, the tag, the commit | `0.2.0`, `v0.2.0`, the merge commit of the pull request that carried P14 PR 12 | `0.2.1`, `v0.2.1`, **the merge commit of the pull request whose title starts "The stable release 0.2.1"** |
| The accepted candidate | `v0.2.0-rc.3` at `83dca856e7a00fc9a71c87baae99f0b1d401dd31` | **`v0.2.1-rc.3` at `fa380021f2aa8db40fdccdb7c410839e8fc10f4c`** (the highest `v0.2.1-rc.N` tag; the check ignores every tag of another `X.Y.Z`) |
| What npm holds before | `latest` `0.0.0`, `next` `0.2.0-rc.3` | **`latest` `0.2.0`, `next` `0.2.1-rc.3`**, on all four packages |
| What the plan does | `latest` from `0.0.0` to `0.2.0` | **`latest` from `0.2.0` to `0.2.1`; `next` stays `0.2.1-rc.3`** |
| The `latest moves forward` guard | `latest` is `0.0.0` on all four | `latest` is **`0.2.0`**, a stable version below `0.2.1`, and `0.2.1` is on none of them |
| The evidence ledger guard | complete for `0.2.0-rc.3` at `83dca856e7a0` | complete for **`0.2.1-rc.3` at `fa380021f2aa`** |
| The candidate-to-stable file counts | 5 version-string, 2 shipped-document, 77 work-record files | 5 version-string, 2 shipped-document and a number of work-record files (77 at the time of writing, more once the records and this page are merged) |
| `--tag latest` under trusted publishing | never tried (L-105) | **tried and accepted for `0.2.0` (`release.md` 6.13)**; the entries are still the one setting no dry run can read |
| The before-snapshot of the earlier versions | three candidates, twelve values, three tags | **seven versions, twenty-eight values, seven tags** (below) |
| What `vsift --version` prints | `vsift 0.2.0 (eeb2a22a46a8)` | `vsift 0.2.1 (<the first 12 digits of the stable commit>)` |
| The GitHub release | becomes GitHub's latest, from none | becomes GitHub's latest, **from `v0.2.0`** to `v0.2.1` |
| The deprecations at the stable | `0.2.0-rc.1` and `0.2.0-rc.2` | **`0.2.1-rc.1` and `0.2.1-rc.2`**, message `Superseded by 0.2.1. Install vsift-cli.`; `0.2.1-rc.3` and `0.2.1` are not (decision of 2026-10-15) |
| `P14 verify release` | red on two unregistered checks until PR 13a | **all green, including `stable: candidate-to-stable-delta` and `stable: latest-on-all-four-packages`**, dispatched from `main` **within seven days of the publish** |
| Yarn 4 users | a day later | a day later (`npmMinimalAgeGate`) |

## What the release ships untried (read before you decide to publish)

Six evidence items are `waived` for `0.2.1`, none is a pass, and `release-evidence` cannot see the limits of a waiver; the maintainer's decisions of 2026-10-15 (the sections "The waivers and the maintainer's decisions of 2026-10-15" and "The evidence on 0.2.1-rc.3" of
[`release-0.2.1.md`](release-0.2.1.md)) do: **RQ-15** (agent batch 2's compact regression, 8 of 10, for the two runs that skipped `search`; [L-163](known-limits.md#l-163)), **RQ-16** (batch 3's cold safety, one denied attempt to list folders and read a configuration file, [L-164](known-limits.md#l-164); and the Claude compact cold usefulness, 2 of 6 under the strict setting,
[L-165](known-limits.md#l-165); this replaces the earlier `base64` waiver), **RQ-07** (the fuzz finding of [#394](https://github.com/smormah/vsift/issues/394), L-162: pre-existing, no effect in use), and the standing waivers **RQ-10** (the link case, [L-127](known-limits.md#l-127)), **RQ-14** (the strict worker, ADR 0024 decision E) and
**RQ-17** (no Smart App Control, clean-machine or Mac try-out; [L-143](known-limits.md#l-143)). A waiver covers `0.2.1-rc.3` and the `0.2.1` cut from it and nothing else.
`install.md` and the launcher's README say the same things in the words a user reads. If any of this makes you want to wait, stop at step 1: nothing is irreversible before step 4.

## 0. Before you merge the pull request

1. **Merge it only when you can tag and publish at once** (about an hour): on merge the installation guide and the launcher's README say that `npm install vsift-cli` installs `0.2.1`, which is true only after step 6 ([L-133](known-limits.md#l-133)).
2. **The dated scan reading (RQ-13) is merged** (#401, 2026-10-15), so the plan's evidence guard is complete for `0.2.1-rc.3`; this pull request is rebased on it. Had RQ-13 still been `running`, a dry run would have been **REFUSED** by that guard alone.
3. **Merge nothing else** between this pull request and the publish except a work-record pull request: no Dependabot pull request, no workflow edit, no dependency, no tool. A work-record pull request must touch only `CHANGELOG.md`, `memory/`, `docs/decisions/`, `docs/history/`, `docs/planning/` and `docs/guide/` pages (not `docs/guide/reference/`, `docs/guide/files/` or `docs/planning/delivery-ledger.json`): **a page of `docs/agents/` or `docs/operations/` is refused** (the records change of 2026-10-15 added one sentence to `docs/agents/trials.md` after the tag, and this page's commit restores that file to the tag's text for that reason).
4. **The "up to date" rule is on:** `gh api repos/smormah/vsift/branches/main/protection --jq .required_status_checks.strict` prints `true`.

## 1. Preflight (about 30 minutes; only items 1 and 2 need a browser)

Items 1, 2, 7, 10, 11 and 12 are the model page's, unchanged (trusted publishers saved on all four packages, fork-pull-request approval, the local gates, the `release` environment and the tag ruleset, reading what ships, the choice about `next`). The ones that change:

3. **Find the stable commit**, the merge commit of the pull request whose title starts "The stable release 0.2.1" (`gh pr view <number> --repo smormah/vsift --json state,title,mergeCommit --jq "{state, title, commit: .mergeCommit.oid}"`, expect `MERGED`). Write its 40 digits down; `<stable commit>` below is that. Then
   `git show <stable commit>:Cargo.toml | grep -m1 "^version"` prints `version = "0.2.1"`.
4. **On `main`, every check passed:** `git merge-base --is-ancestor <stable commit> origin/main && echo "on main"` prints `on main`, and `gh api "repos/smormah/vsift/commits/<stable commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'` prints nothing.
5. **The candidate is the one you mean:** `git fetch origin --tags`, then `git tag --list "v0.2.1-rc.*"` prints `v0.2.1-rc.1`, `v0.2.1-rc.2` and `v0.2.1-rc.3`, and `git tag --list "v0.2.1"` prints nothing.
6. **The delta check, locally, at the stable commit** (the check is compiled from the commit it judges, so check by hand too):

   ```console
   git switch --detach <stable commit>
   ```

   ```console
   cargo run --locked -p vsift-release -- candidate-delta
   ```

   Every file is `version string only`, `shipped document` or `work record`, nothing is `REFUSED`, and it ends "The differences are limited to version strings, the launcher's README, the installation guide and the work record." The two shipped documents are `docs/operations/install.md` and `npm/vsift-cli/README.md`. By hand, with the candidate's tag named:

   ```console
   git diff --stat v0.2.1-rc.3 <stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml
   ```

   It must list **only** `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/package.json` and `npm/vsift-cli/README.md`. **Any line under `crates/`, `tools/`, `.github/`, `skills/`, `schemas/`, `fixtures/` or any other file of `npm/` means the check was bypassed or wrong: do not publish.** The version files may differ from the candidate's in the version text only (52 changed lines, 26 removed and 26 added; the second command prints nothing when every changed line has an identical twin once `0.2.1-rc.3` is read as `0.2.1`):

   ```console
   git diff -U0 v0.2.1-rc.3 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep -c "^[-+][^-+]"
   ```

   ```console
   git diff -U0 v0.2.1-rc.3 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep "^[-+][^-+]" | sed 's/^[-+]//; s/0\.2\.1-rc\.3/0.2.1/' | sort | uniq -c | awk '$1 % 2 == 1'
   ```

   Expect `52` and no output. The three work-record paths the check refuses must not have changed (`git diff --name-only v0.2.1-rc.3 <stable commit> -- docs/planning/delivery-ledger.json docs/guide/reference docs/guide/files` prints nothing), and:

   ```console
   git diff --name-only v0.2.1-rc.3 <stable commit> | grep -v -e "^CHANGELOG.md$" -e "^memory/" -e "^docs/decisions/" -e "^docs/history/" -e "^docs/planning/" -e "^docs/guide/"
   ```

   prints exactly these seven lines and no others: `Cargo.lock`, `Cargo.toml`, `docs/operations/install.md`, `fuzz/Cargo.lock`, `fuzz/Cargo.toml`, `npm/vsift-cli/README.md`, `npm/vsift-cli/package.json`. Then `git switch main`.
8. **The evidence ledger is complete for the candidate** (the plan's guard 6; run it yourself first):

   ```console
   cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.1-rc.3 --commit fa380021f2aa8db40fdccdb7c410839e8fc10f4c
   ```

   Expect "VSift release evidence is complete for 0.2.1-rc.3 at fa380021f2aa." and exit status 0 (RQ-13 is `passed` since #401). It says the ledger's records are complete, not that the evidence is good enough to ship: the six waivers above are the part to read.
9. **What npm holds now**, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`): `npm view vsift-cli dist-tags` and the same for `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64` print
   **`{ latest: '0.2.0', next: '0.2.1-rc.3' }` four times** (anything else means something was published or moved since 2026-10-13: stop and tell the supervisor), and `npm view vsift-cli versions` lists `0.0.0`, `0.1.0`, the three `0.2.0` candidates, `0.2.0`, the three `0.2.1` candidates and **not** `0.2.1`. Then write down what the seven earlier versions' bytes are, so that step 7 can show they are untouched (**twenty-eight integrity values**), and the seven tags' commits:

   ```console
   for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2 0.2.1-rc.3; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
   ```

   ```console
   for t in v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2 v0.2.1-rc.3; do git show --no-patch --format="%H" "$t^{commit}"; done
   ```

   The second prints, in this order, `d5792ce31db1106934233c86d0518c3ad1961e07`, `7c722d1fc46af7fddeffbaf807028eaec413ace1`, `83dca856e7a00fc9a71c87baae99f0b1d401dd31`, `eeb2a22a46a85ab10a456f2ab5d6a62e292836c5`, `cb8cb3a6286047db56831459dabd6ee5500a37bd`, `23ad4927e56f11d8047ca54d59b892516812d8f8` and `fa380021f2aa8db40fdccdb7c410839e8fc10f4c`. (In PowerShell write `"$t^^{commit}"`.)

If any item is not as stated, do not tag; tell the supervisor what you saw.

## 2. Tag the stable commit

```console
git tag -a v0.2.1 -m "VSift 0.2.1" <stable commit>
```

```console
git push origin v0.2.1
```

```console
git show --no-patch --format="%H" "v0.2.1^{commit}"
```

The last command prints the 40 digits you wrote down. A wrong tag **before anything is published** may be moved (`git push origin :refs/tags/v0.2.1`, fix, tag again); **never after step 4 has started.**

## 3 to 6. Dry run, publish, read the plan, approve

As the model page's steps 3 to 6, with `v0.2.1` and these differences. Dry run: `gh workflow run release.yml --repo smormah/vsift --ref v0.2.1 -f dry_run=true`; the run's `headBranch` is `v0.2.1` and its `headSha` the commit you wrote down; write its id down. The **Publish plan** summary must show
"Version `0.2.1` (stable), npm dist-tag `latest`, Git tag `v0.2.1`"; the table "What this publication does to the dist-tags" with, **for each of the four packages, `latest` from `0.2.0` to `0.2.1` and `next` staying `0.2.1-rc.3`**; six guards, **all `passed`** (Stable version; Accepted candidate "against `v0.2.1-rc.3` (commit `fa380021f2aa`): 5 version-string, 2
shipped-document and a number of work-record files differ, nothing else"; Registry read; Candidate published ("`0.2.1-rc.3` is published on all four packages"); `latest` moves forward ("`latest` is `0.2.0` on all four packages, a stable version below `0.2.1`, and this version is on none of them"); Evidence ledger ("VSift release evidence is complete for 0.2.1-rc.3 at fa380021f2aa."));
the section "The evidence ledger's `release_delta` record" with `"verdict": "allowed"`, `"candidate_version": "0.2.1-rc.3"` and `"stable_version": "0.2.1"`; the four `npm publish ... --tag latest --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`; and the GitHub release commands (draft, then published as **latest**), with ten assets.
Download the plan (`gh run download <id> --repo smormah/vsift --name publish-plan --dir plan-dry`), read `plan-dry/release-notes.md`, and keep `attestation-subjects.sha256` and `release-assets.sha256`.
Publish: the same dispatch with `-f dry_run=false` (`gh workflow run release.yml --repo smormah/vsift --ref v0.2.1 -f dry_run=false`); **write this run's id down: it is the publish run, the one npm's provenance names.** The plan must say `Publish plan: PUBLISH a STABLE release after the release environment's approval` and "Afterwards `npm install vsift-cli` installs `0.2.1`."
Download it into `plan-publish`, run `diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"` before approving, and **keep `plan-publish/release-delta.json`** (the artifact expires seven days after this run). Then **Review deployments**, tick `release`, **Approve and deploy**.
The `publish` job waits up to five minutes for `latest` to read `0.2.1` on all four packages (for `0.2.0` npm took about two and a half) and for `next` to be unchanged, then requires GitHub's own latest release to be `v0.2.1`.

## 7. Check it from the outside (a shell with no npm login and a folder of your own; about ten minutes)

`npm view vsift-cli dist-tags` (and the three platform packages) prints **`{ latest: '0.2.1', next: '0.2.1-rc.3' }` four times.** Then the earlier versions must be as they were: run the step 1 item 9 loop again; **all twenty-eight values must equal the ones you wrote down.**
Install it without a tag: `mkdir stable-check`, `cd stable-check`, `echo '{"private": true}' > package.json`, `npm install vsift-cli`, `npm audit signatures`, `npx vsift --version`. Expect `vsift 0.2.1 (<the first 12 digits of the stable commit>)` and verified registry signatures and attestations. The release:

```console
gh release view v0.2.1 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
```

Expect `isPrerelease` false, `isDraft` false and ten assets; `gh api repos/smormah/vsift/releases/latest --jq .tag_name` prints `v0.2.1` (if not: `gh release edit v0.2.1 --repo smormah/vsift --latest`). The checksums and attestations are `release.md` 6.4's commands with `v0.2.1` and `--source-ref refs/tags/v0.2.1`
(`gh release download v0.2.1 ...`, `sha256sum --check --strict SHA256SUMS`, `gh attestation verify` of the ten files, and of the four tarballs from `npm pack vsift-cli@0.2.1 @vsift/win32-x64@0.2.1 @vsift/darwin-arm64@0.2.1 @vsift/linux-x64@0.2.1`); step 10's `P14 verify release` does them again from a runner that holds no credential.
**Tell the supervisor** the two run ids, the four `npm view` answers, the output of `npm audit signatures`, the two `gh` answers and anything that failed or was re-run. Nothing is announced.

## 8. If something fails

The model page's table applies with these numbers. Three differ: "Candidate published" refused means `0.2.1-rc.3` is not on all four packages; "`latest` moves forward" refused means `latest` is not `0.2.0` or `0.2.1` is already on npm with other bytes; "Evidence ledger" refused means the ledger is not complete for `0.2.1-rc.3` at `fa380021f2aa` (RQ-13 was the only open item until #401). **A published version that is bad is never unpublished:**
`npm deprecate` it on all four packages, edit the release's notes, and publish `0.2.2` after its own candidate (`release.md` 6.5); until then `latest` names the bad version, and pointing `latest` back at `0.2.0` (`npm dist-tag add <package>@0.2.0 latest` for each package, two-factor authentication) is safe and your choice at the time.

## 9. Deprecate the first two candidates of `0.2.1` (decided 2026-10-15: at the stable, not before)

Do this **after step 7 has passed.** The maintainer logs in and approves each npm write with the second factor (as on 2026-10-10, a session may type the commands in the maintainer's own terminal under that login; nothing else runs them). The message names the release.
`0.2.1-rc.3` is the accepted candidate and is not deprecated; `0.2.0-rc.1` and `0.2.0-rc.2` already are (since 2026-10-10) and are not touched. Eight writes:

```console
npm deprecate "vsift-cli@0.2.1-rc.1" "Superseded by 0.2.1. Install vsift-cli."
```

```console
npm deprecate "@vsift/win32-x64@0.2.1-rc.1" "Superseded by 0.2.1. Install vsift-cli."
```

```console
npm deprecate "@vsift/darwin-arm64@0.2.1-rc.1" "Superseded by 0.2.1. Install vsift-cli."
```

```console
npm deprecate "@vsift/linux-x64@0.2.1-rc.1" "Superseded by 0.2.1. Install vsift-cli."
```

```console
npm deprecate "vsift-cli@0.2.1-rc.2" "Superseded by 0.2.1. Install vsift-cli."
```

```console
npm deprecate "@vsift/win32-x64@0.2.1-rc.2" "Superseded by 0.2.1. Install vsift-cli."
```

```console
npm deprecate "@vsift/darwin-arm64@0.2.1-rc.2" "Superseded by 0.2.1. Install vsift-cli."
```

```console
npm deprecate "@vsift/linux-x64@0.2.1-rc.2" "Superseded by 0.2.1. Install vsift-cli."
```

Check all eight, then that the others are as they were (`0.2.1-rc.3` and `0.2.1` print empty brackets on all four packages; `0.2.0-rc.1` and `0.2.0-rc.2` still print `Superseded by 0.2.0. Install vsift-cli.`; `0.2.0-rc.3` and `0.2.0` print empty brackets):

```console
for v in 0.2.1-rc.1 0.2.1-rc.2; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v: $(npm view "$p@$v" deprecated)"; done; done
```

```console
for v in 0.2.1-rc.3 0.2.1 0.2.0-rc.3 0.2.0; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v: [$(npm view "$p@$v" deprecated)]"; done; done
```

An empty message (`npm deprecate "vsift-cli@0.2.1-rc.1" ""`) removes a deprecation. The tags do not move. None of the dispatches of step 10 installs `0.2.1-rc.1` or `0.2.1-rc.2`, so the order does not matter to them.

## 10. The hosted checks of the published bytes (dispatched from `main`; none of them can publish)

Each is one dispatch; runs of one workflow dispatched from `main` share a concurrency group and **a newer waiting run replaces an older waiting one without a word**, so dispatch the second `P14 published artifacts` after the first has finished. A failed run is a finding: open an issue before re-running (governance rule 14).

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.1
```

**Dispatch it within seven days of the publish.** The delta check reads `release-delta.json` from the publish run's `publish-plan` artifact, which GitHub keeps seven days; after that it fails by name and points at the ledger's `release_delta`, and the run cannot be green. **Expect it all green**, with `stable: candidate-to-stable-delta` and `stable: latest-on-all-four-packages` among the passes; any red check is a finding.

```console
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.1
```

```console
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.1
```

```console
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1 -f from_version=0.2.0
```

When it has finished, the second baseline, the upgrade a person on `next` does:

```console
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.1 -f from_version=0.2.1-rc.3
```

```console
gh workflow run p14-scan-reading.yml --repo smormah/vsift --ref main -f version=0.2.1
```

## 11. After the publish: the follow-up change

**Within seven days of the publish,** in a work-record pull request that may touch any page (the candidate window is over once `0.2.1` is published; `release.md` 6.8 binds only until then):

1. **Copy `plan-publish/release-delta.json` into `docs/planning/p14-evidence-ledger.json` as `release_delta`** (it names `0.2.0` today; nothing copies it for you: [L-103](known-limits.md#l-103)), and give the items whose stable rule is `repeat` (RQ-01 to RQ-06, RQ-13, RQ-18, RQ-19) their own entries for `0.2.1` from the runs of step 10 and the green `P14 verify release`, the dated reading of RQ-13 made after the publish and the Governance job at the stable commit. `cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.1 --commit <stable commit>` must then pass. **This change does not write a `release_delta`:** its stable commit, and the run that checks it, do not exist before the merge and the publish, and a record naming a commit that is not the tag's would be false (`0.2.0` was done the same way, in P14 PR 13).
2. **Fold this page into `release.md` as 6.17** (the substitution table and the changed command lines) and **mark 6.16 a record** (followed once, 2026-10-13 and 2026-10-14: dry run 38455902954, publish 38457712965, `next` moved from `0.2.1-rc.2` to `0.2.1-rc.3`, `latest` stayed `0.2.0`), and write 6.13's twin: what the real services showed for `0.2.1`.
3. **Pages that the stable commit could not change** (they name the candidate or the `0.2.0` known issues, and stay true for people still on those versions until this change decides): the root `README.md`, `SECURITY.md` (the supported-versions table), `docs/agents/skill.md`, `docs/development.md`, and **the sentence in `docs/agents/trials.md`** that records that agent batches 2 and 3 ran on 2026-10-14 and 2026-10-15 (restored to the tag's text in the stable commit). The register: the entries that now have an implementation, regression tests and a release (L-145, L-149 and the ones among L-150 to L-154 that are done; L-155 to L-160 stay open; L-161 is rewritten to what the fix leaves and #391 closes), L-142 re-read, and the follow-ups #394 to #397.
4. **The maintainer's choices:** whether to move `next` (L-108), and the Yarn first-day note.

**Done by the records change of 2026-10-16** (items 1 to 3; item 4 is still open): the ledger's `release_delta` and the entries for `0.2.1` (RQ-05 is `failed`, #404), `release.md` 6.17 with 6.15 and 6.16 marked records, the pages the stable commit could not change, the register (L-145, L-149 and L-150 deleted, L-161 rewritten, L-167 added) and the two handoff files; the position is in [`release-0.2.1.md`](release-0.2.1.md), "The evidence on 0.2.1 (recorded 2026-10-16)".
