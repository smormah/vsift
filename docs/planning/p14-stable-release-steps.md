# The stable release `0.2.0`: the maintainer's steps (P14 PR 12)

Status: **prepared 2026-10-09 (P14 PR 12); nothing here has been run.** This is the maintainer's checklist for tagging,
publishing and checking `0.2.0`. It is a planning page, not a section of [`release.md`](../operations/release.md), because
`release.md` may not change between the third candidate and the stable release (6.8 lists what may). The rule is
`release.md` 6.7; this page is 6.7 written out with this release's real numbers, the way 6.10 to 6.12 are for the
candidates. Where this page and 6.7 or 6.12 differ, this page says so, and it is the later word. Folding it into
`release.md` is work after the publish (the last section).

**Who does what.** Every command that publishes, tags, deprecates or touches npm is yours: the supervisor never runs one, and
no step needs a secret except your npm login and second factor (step 9) and your approval of the `release`
environment (step 6). The supervisor dispatches nothing before you say go; the read-only workflows of step 10 are
dispatched from `main` and cannot publish.

**Plain English first.** `0.2.0` is built from exactly the source of `0.2.0-rc.3`: the stable commit differs from the tag
`v0.2.0-rc.3` only in its version numbers, the launcher's README, the installation guide and the work record, and a program
checks that (step 1, item 6). Publishing it moves npm's `latest` for the first time, from the empty `0.0.0`
placeholder to `0.2.0`, on all four packages. After that, `npm install vsift-cli` with no tag installs VSift. A published
version can never be taken back.

## What differs from the third candidate's runbook (`release.md` 6.12)

| | The third candidate (6.12, 2026-10-08) | The release (this page) |
| --- | --- | --- |
| What npm holds before | `next` `0.2.0-rc.2`, `latest` `0.0.0` | `next` **`0.2.0-rc.3`**, `latest` `0.0.0`, on all four packages |
| What the plan does | moves `next`, leaves `latest` | moves **`latest` from `0.0.0` to `0.2.0`** on all four packages and leaves `next` at `0.2.0-rc.3` |
| The publish command | `npm publish ... --tag next` | `npm publish ... --tag latest`, which **has never gone through trusted publishing** (L-105) |
| The GitHub release | a pre-release, not marked latest | **not** a pre-release, **marked latest** (also never tried: L-105) |
| Guards in the plan | two | six, one of them the evidence ledger for `0.2.0-rc.3` |
| The tag and the commit it names | `v0.2.0-rc.3` at `83dca856e7a0` | `v0.2.0` at the merge commit of the pull request that carries P14 PR 12 |
| What `vsift --version` prints | `vsift 0.2.0-rc.3 (83dca856e7a0)` | `vsift 0.2.0 (<the first 12 digits of the stable commit>)` |
| What `P14 verify release` does | all green | **red on exactly two named checks until P14 PR 13a registers them** (that was the design, not a finding); **all green after it, within seven days of the publish** (steps 10 and 11) |
| Deprecating the earlier candidates | optional, after the verification runs | **decided 2026-10-09: part of this release, at the stable, not before** (step 9); it replaces 6.12 step 7 |
| Who gets it by default | `@next` users | everyone who installs `vsift-cli` with no tag; **Yarn 4 users a day later** (`npmMinimalAgeGate`) |

## What the release ships untried (read before you decide to publish)

Four evidence items are `waived`, none is a pass, and `release-evidence` cannot see the limits of a waiver; the texts do.

| Item | What was waived | Where the text is |
| --- | --- | --- |
| RQ-10, malicious media | the link case only: `ingest` of a symbolic link answers `STORAGE_IO` where the rule names `INVALID_SOURCE` (#265, L-127). **Carried to the stable on 2026-10-10, for the link case alone** (it named `0.2.0-rc.3` only when the release was published; open point 1 of the last section) | plan 29.5 and 29.10 |
| RQ-14, SEC-T01 | the strict worker is not claimed to contain a hostile decoder (decision E option 4) | ADR 0024 amendment of 2026-10-03 |
| RQ-16, the cold agent | one cold run in 18 read the clip `vsift audio` had named, with `base64`, inside the container (#340, L-142); carried to the stable on 2026-10-09 | plan 29.9 and 29.10 |
| RQ-17, the try-outs | no Smart App Control or SmartScreen try-out, no true clean-machine install, no Mac Gatekeeper try-out, no person has run VSift on a Mac (L-143, L-098); carried to the stable on 2026-10-09 | plan 29.10 |

`install.md` and the launcher's README say the same things in the words a user reads. If any of this makes you want to
wait, stop at step 1: nothing is irreversible before step 4.

## 0. Before you merge the pull request

1. **Merge it only when you can tag and publish at once** (about an hour). On merge, the installation guide and the launcher's
   README say that `npm install vsift-cli` installs `0.2.0`, which is true only after step 6 ([L-133](known-limits.md#l-133)).
2. **Merge nothing else** between this pull request and the publish except a work-record pull request: no Dependabot pull
   request (they open on Mondays; leave them), no workflow edit, no dependency, no tool. A code change that lands in between is
   not in the tag but is in the commit the check judges, and the check refuses it (`release.md` 6.8).
3. **The "up to date" rule is on:**

   ```console
   gh api repos/smormah/vsift/branches/main/protection --jq .required_status_checks.strict
   ```

   Expect `true`.

## 1. Preflight (about 30 minutes; only item 1 and item 2 need a browser)

Write the stable commit's 40 digits down once you have it (item 3) and use it everywhere `<stable commit>` appears.

1. **Trusted publishers are still saved on npmjs.com for all four packages** (`vsift-cli`, `@vsift/win32-x64`,
   `@vsift/darwin-arm64`, `@vsift/linux-x64`): each package's Settings, Trusted Publisher, a *saved* entry listed (not the empty
   form) with owner `smormah`, repository `vsift`, workflow `release.yml`, environment `release`, **npm publish** allowed,
   **npm dist-tag** not allowed. This is the one setting only you can read, and no dry run can see it (L-100). Every earlier
   publish, with `--tag next`, went through it; **`--tag latest` never has**, so this is where a first stable publish can still
   stop (`ENEEDAUTH`, with nothing published; `release.md` 6.5).
2. **Fork pull requests** need approval from all external contributors (Settings, Actions, General), as `release.md` 6.2 step 2
   says.
3. **Find the stable commit**, the merge commit of the pull request that carries PR 12 (the supervisor opens it and tells you its number):

   ```console
   git fetch origin --tags
   ```

   ```console
   git switch main
   ```

   ```console
   git pull --ff-only
   ```

   ```console
   gh pr view <the pull request number> --repo smormah/vsift --json state,title,mergeCommit --jq "{state, title, commit: .mergeCommit.oid}"
   ```

   Expect `state` `MERGED`. If `main` has moved on only by work-record pull requests, the commit to tag is still that merge commit,
   not `main`'s head. Then:

   ```console
   git show <stable commit>:Cargo.toml | grep -m1 "^version"
   ```

   Expect `version = "0.2.0"`.
4. **The commit is on `main` and every check of it passed:**

   ```console
   git merge-base --is-ancestor <stable commit> origin/main && echo "on main"
   ```

   ```console
   gh api "repos/smormah/vsift/commits/<stable commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   Expect `on main` and **no output at all** from the second command. A line such as `Quality (windows-latest): in_progress null`
   means wait; a failed check means do not tag.
5. **The candidate is the one you mean:**

   ```console
   git tag --list "v0.2.0-rc.*"
   ```

   Expect `v0.2.0-rc.1`, `v0.2.0-rc.2` and `v0.2.0-rc.3`. The highest number is the candidate the check compares with, so a later
   candidate you did not accept would have to be dealt with first. Also:

   ```console
   git tag --list "v0.2.0"
   ```

   Expect no output.
6. **The delta check, locally, at the stable commit.** The check is compiled from the commit it judges, so run it, then check by hand:

   ```console
   git switch --detach <stable commit>
   ```

   ```console
   cargo run --locked -p vsift-release -- candidate-delta
   ```

   Every file it lists is `version string only`, `shipped document` or `work record`, and nothing is marked `REFUSED` (at the time of writing: 5 version-string
   files, 2 shipped documents and 77 work-record files); it ends
   "The differences are limited to version strings, the launcher's README, the installation guide and the work record." Now by hand,
   with the candidate's tag named (not `rc.2`: that would show the third candidate's own changes and prove nothing):

   ```console
   git diff --stat v0.2.0-rc.3 <stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml
   ```

   It must list **only** `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/package.json` and
   `npm/vsift-cli/README.md`. **Any line under `crates/`, `tools/`, `.github/`, `skills/`, `schemas/`, `fixtures/` or any other
   file of `npm/` means the check was bypassed or wrong: do not publish.** Each version file must differ from the candidate's in the version
   text and nothing else. The second command prints nothing when every changed line of the five files has an identical twin once `0.2.0-rc.3` is read as
   `0.2.0` (there are 52 changed lines, 26 removed and 26 added; the first command counts them):

   ```console
   git diff -U0 v0.2.0-rc.3 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep -c "^[-+][^-+]"
   ```

   ```console
   git diff -U0 v0.2.0-rc.3 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep "^[-+][^-+]" | sed 's/^[-+]//; s/0\.2\.0-rc\.3/0.2.0/' | sort | uniq -c | awk '$1 % 2 == 1'
   ```

   Expect `52` from the first and no output from the second.

   Last, the three work-record paths the check refuses must not have changed, and nothing outside the allowed kinds may appear:

   ```console
   git diff --name-only v0.2.0-rc.3 <stable commit> -- docs/planning/delivery-ledger.json docs/guide/reference docs/guide/files
   ```

   Expect no output (`docs/planning/delivery-ledger.json` changes only in PR 13).

   ```console
   git diff --name-only v0.2.0-rc.3 <stable commit> | grep -v -e "^CHANGELOG.md$" -e "^memory/" -e "^docs/decisions/" -e "^docs/history/" -e "^docs/planning/" -e "^docs/guide/"
   ```

   Expect exactly these seven lines and no others: `Cargo.lock`, `Cargo.toml`, `docs/operations/install.md`, `fuzz/Cargo.lock`,
   `fuzz/Cargo.toml`, `npm/vsift-cli/README.md`, `npm/vsift-cli/package.json`. Then go back:

   ```console
   git switch main
   ```
7. **The local gates at the stable commit** (a minute each after the first build; run them from the detached commit of item 6 if you
   prefer, then switch back):

   ```console
   cargo run --locked -p vsift-governance -- check
   ```

   ```console
   cargo run --locked -p vsift-governance -- public-claims
   ```

   ```console
   cargo run --locked -p vsift-governance -- release-evidence
   ```

   Expect "VSift delivery ledger is valid.", "VSift public claims agree with the evidence ledger ..." and "VSift release evidence ledger
   is valid."
8. **The evidence ledger is complete for the candidate** (this is the plan's guard 6; run it yourself first):

   ```console
   cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.0-rc.3 --commit 83dca856e7a00fc9a71c87baae99f0b1d401dd31
   ```

   Expect "VSift release evidence is complete for 0.2.0-rc.3 at 83dca856e7a0." and exit status 0. It says the ledger's records are complete,
   not that the evidence is good enough to ship: the four waivers of the table above are the part to read.
9. **What npm holds now**, from a shell with no npm login (`npm config get //registry.npmjs.org/:_authToken` prints `undefined`):

   ```console
   npm view vsift-cli dist-tags
   ```

   ```console
   npm view @vsift/win32-x64 dist-tags
   ```

   ```console
   npm view @vsift/darwin-arm64 dist-tags
   ```

   ```console
   npm view @vsift/linux-x64 dist-tags
   ```

   Expect **`{ latest: '0.0.0', next: '0.2.0-rc.3' }` four times.** Anything else means something was published or moved since
   2026-10-08: stop and tell the supervisor. Then confirm the release is not on npm yet:

   ```console
   npm view vsift-cli versions
   ```

   It lists `0.0.0`, `0.1.0`, the three candidates and **not** `0.2.0`. Last, write down what the three candidates' bytes are, so that
   step 7 can show they are untouched (twelve integrity values), and the three tags' commits:

   ```console
   for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
   ```

   ```console
   for t in v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3; do git show --no-patch --format="%H" "$t^{commit}"; done
   ```

   The second prints `d5792ce31db1106934233c86d0518c3ad1961e07`, `7c722d1fc46af7fddeffbaf807028eaec413ace1` and
   `83dca856e7a00fc9a71c87baae99f0b1d401dd31`. (In PowerShell write `"$t^^{commit}"`.)
10. **The `release` environment and the tag ruleset** are as `release.md` 6.2 steps 3 and 4 say (read-only):

    ```console
    gh api repos/smormah/vsift/environments/release --jq '{reviewers: [.protection_rules[]? | .reviewers[]? | .type], branch_policy: .deployment_branch_policy, can_admins_bypass: .can_admins_bypass}'
    ```

    ```console
    gh api repos/smormah/vsift/rulesets --jq '.[] | {name, target, enforcement}'
    ```

    Expect `{"branch_policy":{"custom_branch_policies":true,"protected_branches":false},"can_admins_bypass":false,"reviewers":["User"]}` and
    `{"enforcement":"active","name":"release tags","target":"tag"}`.
11. **Read what ships.** Both reach the public when `latest` moves: [`npm/vsift-cli/README.md`](../../npm/vsift-cli/README.md) (it is the
    npm page of the package) and [`install.md`](../operations/install.md) (the release notes link to it at the tag). Check that they say
    what you want said about `latest`, `@next`, the Yarn one-day hold and the things that were not tried. The dry run's
    `release-notes.md` (step 3) is the third document: it is code, frozen at the candidate.
12. **Decide about `next`, if you want to.** The workflow never moves it (`release.md` 6.7, [L-108](known-limits.md#l-108)), so after the
    publish `vsift-cli@next` still installs `0.2.0-rc.3` and `npm install vsift-cli` installs `0.2.0`. Moving it is your choice, later, with
    two-factor authentication; both documents are written to be true either way. Nothing below moves it.

If any item is not as stated, do not tag; tell the supervisor what you saw.

## 2. Tag the stable commit

You are the only person the tag ruleset lets create a `v*` tag.

```console
git tag -a v0.2.0 -m "VSift 0.2.0" <stable commit>
```

```console
git push origin v0.2.0
```

```console
git show --no-patch --format="%H" "v0.2.0^{commit}"
```

The last command prints the 40 digits you wrote down. A wrong tag **before anything is published** may be moved (you can bypass the
ruleset): `git push origin :refs/tags/v0.2.0`, fix, tag again. **Never move or delete it after step 4 has started** (the attestations name it from then on).

## 3. Dry run on the tag (nothing is published)

The dry run is *enforced* for a stable version: it fails whenever the real run would. Use "Use workflow from" **Tags: v0.2.0** in the
Actions tab with **dry_run** ticked, or:

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.0 -f dry_run=true
```

```console
gh run list --repo smormah/vsift --workflow release.yml --event workflow_dispatch --limit 3 --json databaseId,headBranch,headSha,status,conclusion
```

Pick the run by what it says: its `headBranch` is `v0.2.0` and its `headSha` the commit you wrote down. **Write the run id down.**

```console
gh run watch <the dry run's id> --repo smormah/vsift
```

It takes about 30 to 40 minutes (`attest` and `publish` are skipped). Open the run's **Publish plan** summary and read it from the top. It
must show:

- the heading `Publish plan: dry run, nothing is published`, "This run was dispatched with `dry_run` set." and "This version is stable: a
  real publication of this plan would move npm's `latest` dist-tag on all four packages.";
- "Version `0.2.0` (stable), npm dist-tag `latest`, Git tag `v0.2.0`", the commit you wrote down, `workflow_dispatch` on `refs/tags/v0.2.0`
  and "The plan is enforced";
- the table "What this publication does to the dist-tags": **for each of the four packages `latest` from `0.0.0` to `0.2.0`, and `next`
  staying `0.2.0-rc.3`**;
- six guards, **all `passed`**: Stable version; Accepted candidate ("against `v0.2.0-rc.3` (commit `83dca856e7a0`): 5 version-string, 2
  shipped-document and a number of work-record files differ, nothing else"); Registry read; Candidate published (`0.2.0-rc.3` is
  published on all four packages); `latest` moves forward (`latest` is `0.0.0` on all four, a stable version below `0.2.0`, and this
  version is on none of them); Evidence ledger ("VSift release evidence is complete for 0.2.0-rc.3 at 83dca856e7a0.");
- the section "The evidence ledger's `release_delta` record" with `"verdict": "allowed"`, `"candidate_version": "0.2.0-rc.3"` and
  `"stable_version": "0.2.0"`;
- four `npm publish ... --tag latest --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`,
  `@vsift/linux-x64`, `vsift-cli`, and the GitHub release commands (created as a draft, then published as **latest**), with ten assets.

If it says **REFUSED**, read "Refused because:" and do not publish. A refusal of "Accepted candidate" means the check found a file outside
the allowed lists: it is not a thing to work around (a fourth candidate may be needed). Then download the plan and keep it:

```console
gh run download <the dry run's id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

Read `plan-dry/release-notes.md` (it is what the GitHub release will say: "VSift 0.2.0 is published to npm under the dist-tag `latest`, so
`npm install vsift-cli` installs it, and it is the latest GitHub release", then "What this release promises", the Yarn one-day note and
the SmartScreen, Gatekeeper and Smart App Control paragraph). **Keep `plan-dry/attestation-subjects.sha256` and
`plan-dry/release-assets.sha256`**: step 4 compares them. The run's files expire after seven days.

## 4. Publish: dispatch with `dry_run` cleared

The same dispatch on the same tag with **dry_run cleared** (untick it, or `-f dry_run=false`):

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.0 -f dry_run=false
```

```console
gh run list --repo smormah/vsift --workflow release.yml --event workflow_dispatch --limit 3 --json databaseId,headBranch,headSha,status,conclusion
```

**Write this run's id down: it is the publish run**, the one npm's provenance will name. The plan job must say
`Publish plan: PUBLISH a STABLE release after the release environment's approval` and "This publication moves npm's `latest` dist-tag on
all four packages. Afterwards `npm install vsift-cli` installs `0.2.0`."; `attest` runs (about a minute; **its attestations are public and
permanent from the moment it finishes**, before the approval; repeating is harmless because the builds are reproducible); `publish` stops at
**Waiting for review**. Download this run's plan:

```console
gh run download <the publish run's id> --repo smormah/vsift --name publish-plan --dir plan-publish
```

**Compare it with the dry run's, before you approve:**

```console
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

**Keep `plan-publish/release-delta.json`**: PR 13 copies it into the ledger and the two stable checks read it, and the artifact expires seven
days after this run.

## 5. Read the plan one last time

The dist-tag table (`latest` from `0.0.0` to `0.2.0` on all four, `next` unchanged), the four `npm publish ... --tag latest` commands in
order, `gh release edit ... --latest`. This is the last point at which stopping costs nothing but time.

## 6. Approve

**Review deployments**, tick `release`, **Approve and deploy** (within a week). From the first `npm publish` on, only the failure table below
applies.

What the `publish` job does, and where it stops: it records the dist-tags; checks the version's shape and that every `latest` is a stable
version at or below this one; publishes the three platform packages and then `vsift-cli` with `--tag latest`; waits up to five minutes for
`latest` to read back as `0.2.0` on all four packages and for `next` to be unchanged; creates the GitHub release as a draft, publishes it marked
latest, and requires GitHub's own latest release to be `v0.2.0`. npm may say a package "is being processed and may take a few minutes to become
available": that is normal.

## 7. Check it from the outside (a shell with no npm login and a folder of your own; about ten minutes)

```console
npm view vsift-cli dist-tags
```

```console
npm view @vsift/win32-x64 dist-tags
```

```console
npm view @vsift/darwin-arm64 dist-tags
```

```console
npm view @vsift/linux-x64 dist-tags
```

Expect **`{ latest: '0.2.0', next: '0.2.0-rc.3' }` four times.** `next` must be unchanged. Then the earlier candidates must be as they were (the
twelve values of step 1, item 9):

```console
for v in 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v $(npm view "$p@$v" dist.integrity)"; done; done
```

Now install it **without a tag**, the way a user does:

```console
mkdir stable-check
```

```console
cd stable-check
```

```console
echo '{"private": true}' > package.json
```

```console
npm install vsift-cli
```

```console
npm audit signatures
```

```console
npx vsift --version
```

Expect `vsift 0.2.0 (<the first 12 digits of the stable commit>)`, and from `npm audit signatures` verified registry signatures and verified
attestations (on Windows 4 and 4; on Linux the two packages it installs). Then the GitHub release:

```console
gh release view v0.2.0 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
```

Expect `isPrerelease` **false**, `isDraft` false and ten assets.

```console
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `v0.2.0`. If it prints anything else, the release is not marked latest: `gh release edit v0.2.0 --repo smormah/vsift --latest`, then ask again.

The checksums and attestations of the ten files and four tarballs are `release.md` 6.4's commands with the version and `--source-ref
refs/tags/v0.2.0`; step 10's `P14 verify release` does them again from a runner that holds no credential, so doing both is optional. By hand:

```console
gh release download v0.2.0 --repo smormah/vsift --dir release
```

```console
(cd release && sha256sum --check --strict SHA256SUMS)
```

```console
for file in release/*; do gh attestation verify "$file" --repo smormah/vsift --signer-workflow smormah/vsift/.github/workflows/release.yml --source-ref refs/tags/v0.2.0 --deny-self-hosted-runners; done
```

```console
mkdir tarballs
```

```console
npm pack vsift-cli@0.2.0 @vsift/win32-x64@0.2.0 @vsift/darwin-arm64@0.2.0 @vsift/linux-x64@0.2.0 --pack-destination tarballs
```

```console
for file in tarballs/*.tgz; do gh attestation verify "$file" --repo smormah/vsift --signer-workflow smormah/vsift/.github/workflows/release.yml --source-ref refs/tags/v0.2.0 --deny-self-hosted-runners; done
```

The release page and npmjs.com should show each version as built and signed on GitHub Actions.

**Tell the supervisor** now, even before the rest: the dry run's id and the publish run's id, the four `npm view` answers, the output of
`npm audit signatures`, the answers of the two `gh` commands, and anything that failed or was re-run. Nothing is announced.

## 8. If something fails

| When | What it means | What to do |
| --- | --- | --- |
| Step 1 fails (a check, a setting, a registry answer) | the commit, the settings or npm are not what was reviewed | do not tag; tell the supervisor |
| Step 3 fails or is **REFUSED** | nothing was attested or published | read "Refused because:" and fix the cause (`release.md` 6.5); a wrong tag may be moved before anything is published. "Accepted candidate" refused: a file outside the allowed lists; "Candidate published" refused: `0.2.0-rc.3` is not on all four packages; "`latest` moves forward" refused: `latest` is not `0.0.0` or `0.2.0` is already on npm with other bytes; "Evidence ledger" refused: the ledger is not complete for `0.2.0-rc.3` at this commit |
| Step 4: the two `diff`s differ | the bytes are not the dry run's | do not approve; open an issue; the run stays unapproved and expires |
| Step 6: `publish` fails with `ENEEDAUTH` before any `+ package@version` line | a trusted publisher is not saved or is wrong, or `--tag latest` is not accepted under it (L-105 item 1) | `release.md` 6.5; nothing was published; check the four entries of step 1 item 1 |
| Step 6: `publish` fails after some packages | a partial publish ([L-097](known-limits.md#l-097)) | **Re-run failed jobs** on the same run within seven days and approve again: versions npm holds with the same bytes are skipped. `latest` has already moved on the packages published first, the launcher goes last, so `npm install vsift-cli` still gets the old `latest` until it finishes |
| Step 6: the job fails at its last step (the release is not GitHub's latest) | npm is already correct (L-105 item 2) | `gh release edit v0.2.0 --repo smormah/vsift --latest`; do not re-run the job |
| Step 6: the read-back of `latest` is slow | npm shows a new version after a minute or more (L-105 item 3) | wait; the job waits up to five minutes; **Re-run failed jobs** if it gave up |
| Step 7: `latest` is not `0.2.0` on one package after ten minutes | the registry is slow, or the publish stopped | read the `publish` job's log before doing anything |
| Step 7: `next` is not `0.2.0-rc.3`, or a candidate's integrity changed | impossible by the workflow's rules and npm's | stop and tell the supervisor |
| A published version is bad | the release itself is wrong | **never unpublish**: `npm deprecate` it on all four packages, edit the release's notes, and publish `0.2.1` after its own candidate (`release.md` 6.5). Until then `latest` still names the bad version; pointing it back to the empty `0.0.0` placeholder is safe and useless, and your choice at the time |

**What is irreversible, in one list.** The four published `0.2.0` versions and their provenance records on npm's transparency log; the Sigstore
attestations of the ten files and four tarballs (they exist from the end of `attest`, before the approval); the tag once anything was published from
it; whatever anyone installed while `latest` named `0.2.0`. **What is reversible:** `latest` itself (`npm dist-tag add <package>@0.0.0 latest` for each package, two-factor
authentication; the workflow never does it), `next` (your choice), the notes, a deprecation (`npm deprecate <package>@<version> ""` removes it).
**Policy, not impossibility:** GitHub lets you delete a release page or move a tag, and `release.md` says never (6.5).

## 9. Deprecate the first two candidates (decided 2026-10-09: at the stable, not before)

Do this **after step 7 has passed**, with your npm login and second factor. The supervisor never runs these. The message is one line and names the release.
`0.2.0-rc.3` is not covered by the decision and is not deprecated here (`next` names it, and npm prints a warning on every install of a deprecated version).

```console
npm deprecate "vsift-cli@0.2.0-rc.1" "Superseded by 0.2.0. Install vsift-cli."
```

```console
npm deprecate "@vsift/win32-x64@0.2.0-rc.1" "Superseded by 0.2.0. Install vsift-cli."
```

```console
npm deprecate "@vsift/darwin-arm64@0.2.0-rc.1" "Superseded by 0.2.0. Install vsift-cli."
```

```console
npm deprecate "@vsift/linux-x64@0.2.0-rc.1" "Superseded by 0.2.0. Install vsift-cli."
```

```console
npm deprecate "vsift-cli@0.2.0-rc.2" "Superseded by 0.2.0. Install vsift-cli."
```

```console
npm deprecate "@vsift/win32-x64@0.2.0-rc.2" "Superseded by 0.2.0. Install vsift-cli."
```

```console
npm deprecate "@vsift/darwin-arm64@0.2.0-rc.2" "Superseded by 0.2.0. Install vsift-cli."
```

```console
npm deprecate "@vsift/linux-x64@0.2.0-rc.2" "Superseded by 0.2.0. Install vsift-cli."
```

Check all eight (each prints its version and the message; a version with no message is not deprecated), and that the release and the third candidate are not:

```console
for v in 0.2.0-rc.1 0.2.0-rc.2; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do echo "$p@$v: $(npm view "$p@$v" deprecated)"; done; done
```

```console
for v in 0.2.0-rc.3 0.2.0; do echo "vsift-cli@$v: [$(npm view "vsift-cli@$v" deprecated)]"; done
```

The second prints two empty brackets. An empty message (`npm deprecate "vsift-cli@0.2.0-rc.1" ""`) removes a deprecation. None of the dispatches in step 10 installs
`0.2.0-rc.1` or `0.2.0-rc.2` (their baselines are `0.1.0` and `0.2.0-rc.3`), so the order does not matter to them. You may also edit the earlier release
pages' notes to say that they are superseded by `v0.2.0` (`gh release edit v0.2.0-rc.3 --repo smormah/vsift --notes-file <the file>`); that is not a condition of anything.

**Done on 2026-10-10.** Before it, `latest` was `0.2.0` and `next` was `0.2.0-rc.3` on all four packages and no version was deprecated. After it, the first check prints the message
`Superseded by 0.2.0. Install vsift-cli.` for all eight (`0.2.0-rc.1` and `0.2.0-rc.2` on each of the four packages), and `0.2.0-rc.3` and `0.2.0` print an empty answer on all four packages
(the second check, widened from `vsift-cli` to the four). The tags did not move. On the maintainer's instruction a session typed the commands in the maintainer's own terminal, under the maintainer's npm login, and the maintainer approved each of the eight writes with the second factor;
that is an exception to "the supervisor never runs these" above, made by the maintainer for this step. The earlier release pages' notes were not edited.

## 10. The hosted checks of the published bytes (dispatched from `main`; none of them can publish)

These are the evidence of PR 13 (the ledger's `repeat` items RQ-01 to RQ-06 and RQ-19). Each is one dispatch. Runs of one workflow dispatched from `main`
share a concurrency group, one runs and one may wait, and **a newer waiting run replaces an older waiting run without a word**, so dispatch the second
`P14 published artifacts` only after the first has finished. A failed run is a finding: open an issue before re-running (governance rule 14).

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref main -f version=0.2.0
```

**Expect this run to be all green, from P14 PR 13a on** (the pull request that registered the two stable checks; `release.md` 6.4): `stable: candidate-to-stable-delta`
and `stable: latest-on-all-four-packages` must be among the passes, with every other check of the run (the four packages' dist-tags and provenance, `npm audit
signatures`, the attestations of ten files and four tarballs, the checksums, the release's flags and its ten file names). Any red check is a finding.
**Dispatch it within seven days of the publish** (the delta check reads `release-delta.json` from the publish run's `publish-plan` artifact, which GitHub keeps until
2026-10-16 14:54 UTC for the publish run 37946261087): after that the delta check fails by name, says so, and points at the ledger's `release_delta`, and the run cannot
be green. Before PR 13a the run was red on exactly those two checks, each "not registered yet", which was the design (a stable version is not verified without them).

```console
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.0
```

```console
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref main -f published_version=0.2.0
```

```console
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.0 -f from_version=0.1.0
```

Wait for it to finish (`gh run watch <its id> --repo smormah/vsift`), then the second baseline, the upgrade a person on `next` really does:

```console
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref main -f version=0.2.0 -f from_version=0.2.0-rc.3
```

`P14 published artifacts` runs the twelve clean installs, the three archives, the offline install and the upgrade on three systems; its upgrade installs
`from_version` by its exact version. `0.2.0-rc.1` and `0.2.0-rc.2` are not baselines (the second candidate stores what the third stores). The weekly runs of
`P14 journeys` and `P13 managed smoke` test the highest published version, which is `0.2.0` from now on.

## 11. After the publish: PR 13 and the follow-ups

**Within seven days of the publish** (the publish run's `publish-plan` artifact, kept in `plan-publish/`, expires then; `P14 verify release` fetches the record from it):

1. **Register the two stable checks** in `STABLE_CHECKS` (`tools/p14-published/lib/verify.cjs`): `candidate-to-stable-delta`, which reads `release-delta.json` from the
   Release run npm's provenance names (`context.releaseRunId`), and `latest-on-all-four-packages` (with the release marked latest). **Done: P14 PR 13a (2026-10-10),**
   a small pull request that changed `tools/` and its tests (`tools/p14-published/test/verify.test.cjs` expected the registry to be empty; it now expects the two names and
   holds both checks to the values of the real publish), allowed once the stable is published (`release.md` 6.8 binds only until then). Then dispatch `P14 verify release`
   for `0.2.0` again, **within seven days of the publish** (step 10): **this** run, all green, is the RQ-19 evidence for the release.
2. **Copy `plan-publish/release-delta.json` into `docs/planning/p14-evidence-ledger.json`** as `release_delta` (nothing copies it for you: L-103).

**PR 13, the ledger follow-up** (governance rule 9; the supervisor builds it, you merge it; plan 9 and `implementation-work-packets.md`):

1. **The ledger gets its own entries for `0.2.0`** at the stable commit for the items whose stable gate is `repeat`: RQ-01 to RQ-04 and RQ-19 (the
   runs of step 10 and the green `P14 verify release`), RQ-05 (`P14 journeys`), RQ-06 (`P13 managed smoke`), **RQ-13** (a dated scan reading again before the
   stable: `gh workflow run p14-scan-reading.yml --repo smormah/vsift --ref main -f version=0.2.0` and the by-hand reading, as for `0.2.0-rc.3`) and **RQ-18** (the Governance
   job at the stable commit, its push to `main`). RQ-07 to RQ-12 and RQ-14 to RQ-17 are `carry` items: the candidate's evidence stands with the recorded `release_delta`.
2. **`cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.0 --commit <stable commit>` passes** (exit 0).
3. **The repository-only pages flip to the release's instructions** and the claims rung moves as plan section 9 says (`after_p14`), after the register pass: the list below.
4. **The delivery ledger gets P14 `complete`** with the stable release commit and a verification summary, ADR 0024 becomes Accepted, the register is swept (L-105 records
   what the first `latest` publish did; L-108, L-133 and L-103 are brought up to date), and the work record states R0 complete and the neutral checkpoint for using the
   published CLI ourselves (plan section 12).

**Pages outside 6.8's lists that name the candidate or the `@next` install, so the stable commit could not change them** (PR 13, or a pull request just after the publish, does it):
the root `README.md` (the install block at lines 134 to 137 says `@next`; the status paragraph at line 204 still says 0.1.0 is the pre-release and 0.2.0-rc.3 a candidate under
qualification) and its graphic `docs/assets/readme/roadmap.svg` (L-121); `docs/agents/skill.md` (line 43 installs with `@next`; around line 296 "batch 3 has run on no candidate");
`docs/development.md` (the example `release-evidence --complete-for 0.2.0-rc.3` at line 574, and the notes on the candidate window); `docs/operations/release.md` (6.12 step 7 says deprecation
is optional and before the stable: it is decided, at the stable, and this page's step 9 is its text); `SECURITY.md` (the supported-versions table lists `0.2.0-rc.N` and `0.1.0` and says
"no stable release"). **Pages that the lists allow and that were left alone on purpose** (a work-record or guide page that is not a document the release ships): `docs/guide/limits.md` (line 5,
"VSift is a pre-release"), `docs/planning/support-and-resource-profiles.md` (the sentence at line 106 about the third candidate), `docs/planning/delivery-governance.md` (line 38) and
`docs/planning/rq-17-tryout-sheet.md` (lines 53, 104, 121 and 170 install with `@next` and call a plain `vsift-cli` the placeholder). `docs/planning/public-claims.json` keeps its `candidate`
rung until PR 13, so CL-201 to CL-209 stay unused.

**Open for you, none decided here:**

1. **Does the RQ-10 waiver carry to the stable? Decided 2026-10-10: yes, for the link case alone and not for another candidate** (plan 29.5, the update of 2026-10-10). It named
   `0.2.0-rc.3` only; on 2026-10-09 you carried RQ-16's and RQ-17's waivers to the stable by name and did not mention RQ-10's. The stable is the same source. The completeness check
   passes either way (a waived item is complete for any version), so this was a wording decision for the ledger entry and the plan.
2. **Whether to move `next`** (step 1 item 12, L-108).
3. **What to do about Yarn on the first day** beyond the note in `install.md`, the launcher's README and the release notes (nothing was tried: Yarn's behaviour for an untagged
   `yarn add vsift-cli` inside the one-day hold is not known; with an exact version it said "quarantined" on 0.1.0).
4. **#340** (the audio clip a cold agent cannot use) and the other findings listed in the work record: a change to the CLI's text is a new candidate or a `0.2.x`.
