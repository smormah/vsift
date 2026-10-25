# The stable release `0.2.3`: the maintainer's steps

Status: **prepared on 2026-10-25 by the pull request whose title starts "The stable release 0.2.3", into `release/0.2.x`. Not yet followed: `0.2.3` is not tagged or published.** This is the checklist for tagging, publishing and checking `0.2.3`, the patch release after `0.2.2` and the second stable release made on a release branch. It is a planning page and not a section of [`release.md`](../operations/release.md) because `release.md` may not change between the last candidate and the stable release (6.8); it is folded into `release.md` as **6.22** after the publish, in the records change to `main`, when 6.21 is marked a record. **Follow [`release-0.2.2-stable-steps.md`](release-0.2.2-stable-steps.md) (the page for `0.2.2`, which `release.md` 6.20 records, and which is the model), `release.md` 6.7, 6.8 and 6.18 step 5, with the numbers in the table below and the command lines on this page, which are spelled out so that they can be pasted (Git Bash; in PowerShell write `"$t^^{commit}"` for `"$t^{commit}"`).** Everything the model page says that this page does not repeat is unchanged: who does what, the order of the steps, what the dry run's plan must list, the comparison of the two plans, the approval, the failure table and the list of what is irreversible. Nothing here is run by a session except the checks that read: **tagging, dispatching the release workflow, approving the deployment and every npm write are the maintainer's** (the coordinator may type the commands in the maintainer's terminal under the maintainer's login, as on 2026-10-18).

**Plain English first.** `0.2.3` is built from exactly the source of `0.2.3-rc.1`, the candidate published on 2026-10-24 (tag `v0.2.3-rc.1` at `09e40398894fddebe8aa94cf3d871eab5b8ef7ec` on `release/0.2.x`): the stable commit differs from that tag only in its version numbers, the launcher's README, the installation guide and the work record, and a program checks that. Publishing it moves npm's `latest` from `0.2.2` to `0.2.3` on all four packages; after that, `npm install vsift-cli` with no tag installs `0.2.3`. `next` is not moved (it stays `0.2.3-rc.1`; only the maintainer's own command moves it: [L-108](known-limits.md#l-108)). A published version can never be taken back. **The maintainer decided on 2026-10-25 to deprecate only `0.2.3-rc.1`** ([`release-0.2.3.md`](release-0.2.3.md), "The maintainer's decisions of 2026-10-25"); step 9 holds the commands, with the message `Superseded by 0.2.3. Install vsift-cli.`, **for the maintainer to confirm at the gate** (the exact text is the coordinator's proposal and follows 0.2.2's). `0.2.2` is a superseded stable release and is not deprecated, as `0.2.1` was not.

**The runbook's record for the candidate.** `release.md` 6.21 was followed on 2026-10-22 to 2026-10-24: the nightlies of the commit passed (runs 39837839311 and 39892011366), the dry run was 40037368499, the publish 40038576713 (first attempt), `next` moved from `0.2.2-rc.1` to `0.2.3-rc.1` and `latest` stayed `0.2.2`. Its evidence is recorded in [`release-0.2.3.md`](release-0.2.3.md), "The evidence on 0.2.3-rc.1" (#525), and the ledger is complete for it.

## The numbers that change (read the model page with this table)

| | `0.2.2` (the model page, 2026-10-18) | `0.2.3` (this page) |
| --- | --- | --- |
| The version, the tag, the commit | `0.2.2`, `v0.2.2`, the merge commit on `release/0.2.x` of "The stable release 0.2.2" (#456) | `0.2.3`, `v0.2.3` (annotated, message `VSift 0.2.3`), **the merge commit on `release/0.2.x` of the pull request whose title starts "The stable release 0.2.3"** |
| Where the commit and the tag are | `release/0.2.x` | `release/0.2.x`, **the branch made again at the cut of `0.2.3-rc.1`** (the `guard` job refuses a tag its branch does not contain; a stable version needs no nightly of its own, 6.18) |
| The accepted candidate | `v0.2.2-rc.1` at `43a033dbf813` | **`v0.2.3-rc.1` at `09e40398894fddebe8aa94cf3d871eab5b8ef7ec`** (the only, so the highest, `v0.2.3-rc.N` tag; the check ignores every tag of another `X.Y.Z`) |
| What npm holds before | `latest` `0.2.1`, `next` `0.2.2-rc.1` | **`latest` `0.2.2`, `next` `0.2.3-rc.1`**, on all four packages (read 2026-10-25, below) |
| What the plan does | `latest` from `0.2.1` to `0.2.2`; `next` stays | **`latest` from `0.2.2` to `0.2.3`; `next` stays `0.2.3-rc.1`** |
| The `latest moves forward` guard | `latest` is `0.2.1` on all four | `latest` is **`0.2.2`**, a stable version below `0.2.3`, and `0.2.3` is on none of them |
| The evidence ledger guard | complete for `0.2.2-rc.1` at `43a033dbf813` | complete for **`0.2.3-rc.1` at `09e40398894f`** (passes since #525; read again on 2026-10-25) |
| The candidate-to-stable file counts | 5 version-string, 2 shipped-document, 81 work-record files | 5 version-string, 2 shipped-document and **11 work-record files** (at the head of the pull request; more if a work-record pull request merges before the tag) |
| The version-line count of the by-hand check (step 1 item 6) | `52` | **`58`** (29 changed lines, in pairs: the workspace has a seventh crate, `vsift-windows-security`) |
| The before-snapshot of the earlier versions | 45 integrity values and 10 tags | **every published version: 53 integrity values and 12 tags** (below) |
| What `vsift --version` prints | `vsift 0.2.2 (0dadc69df4f5)` | `vsift 0.2.3 (<the first 12 digits of the stable commit>)` |
| The GitHub release | becomes GitHub's latest, from `v0.2.1` | becomes GitHub's latest, **from `v0.2.2`** to `v0.2.3` |
| The deprecations at the stable | `0.2.1-rc.3` and `0.2.2-rc.1` (decided and done 2026-10-18) | **only `0.2.3-rc.1`** (the maintainer's decision of 2026-10-25; step 9) |
| The hosted checks after the publish | dispatched on the tag | dispatched on the tag, **except `P14 journeys`, which is dispatched on `main` with the newer test driver** (step 10, #520); `P14 verify release` **within seven days of the publish** |

## What the release ships untried (read before you decide to publish)

Five of the twenty evidence items are not passes for `0.2.3`; none is a pass and `release-evidence` cannot see the limits of a waiver ([L-179](known-limits.md#l-179)); the maintainer's decisions of 2026-10-25 can ([`release-0.2.3.md`](release-0.2.3.md)): **RQ-08** is `waived`, scoped to #519's single repetition ([L-231](known-limits.md#l-231)); **RQ-15 and RQ-16** are `waived` for `0.2.3` (the skill is byte-identical to the one the earlier batches tried, and the batches run at the `0.3.0` cut); and the standing waivers **RQ-14** (the strict worker, until #188 in R1) and **RQ-17** (no Smart App Control, clean-machine or Mac try-out, until the try-out with LOKI in cycle 3; [L-143](known-limits.md#l-143)) carry. The known limits of this release, which the changelog and `install.md` say in plain words: **L-230** (the noisy-speech accuracy set is not run on hosted macOS, #520; its entry is on `main`, not on this branch), **L-231** (#519: one stress repetition in 20,100 rejected correctly but took longer than the test's bound under load on Windows) and **L-232** (#526: the Node.js versions pinned for the tests and the publish job are behind the Node.js security releases of 2026-10-20; tooling only; the pins move on `main` and on `release/0.2.x` with `0.2.4`, not before this publish). The launcher's README and `install.md` say the same things in the words a user reads. If any of this makes you want to wait, stop at step 1: nothing is irreversible before step 4. (The `release` workflow runs on the pinned Node.js of L-232 for the publish job; that was accepted for `0.2.3`.)

## 0. Before you merge the pull request

1. **The records of the candidate are merged** (#525, `552e1281e17795d5335993802aea718d1dd3b889`), so the plan's evidence guard is complete for `0.2.3-rc.1`; this pull request is cut from that commit or a later one on `release/0.2.x`.
2. **Merge nothing else into `release/0.2.x`** between this pull request and the publish except a work-record pull request (`CHANGELOG.md`, `memory/`, `docs/decisions/`, `docs/history/`, `docs/planning/` and `docs/guide/` pages, not `docs/guide/reference/`, `docs/guide/files/` or `docs/planning/delivery-ledger.json`): **a page of `docs/agents/` or `docs/operations/`, `SECURITY.md` or the root README is refused** (6.8). `main` is not frozen and keeps taking merges; nothing of it reaches the branch. **The hot fix `0.2.4` is made on this branch after `0.2.3` is published, not before.**
3. **The window this opens is the branch's** ([L-133](known-limits.md#l-133)): on merge the installation guide and the launcher's README on `release/0.2.x` say that `npm install vsift-cli` installs `0.2.3`, which is true only after step 6; `main`'s copies keep `0.2.2`'s wording until the records change after the publish. Merge when you can tag and publish within the day.

## 1. Preflight (about 30 minutes; only items 1 and 2 need a browser)

Items 1, 2, 7, 10, 11 and 12 are the model page's, unchanged (trusted publishers saved on all four packages, fork-pull-request approval, the local gates, the `release` environment, the tag ruleset and **the release-branch ruleset** (6.2 step 8), reading what ships, the choice about `next`). The ones that change:

3. **Find the stable commit**, the merge commit of the pull request whose title starts "The stable release 0.2.3". Write its 40 digits down; `<stable commit>` below is that:

   ```console
   gh pr view <number> --repo smormah/vsift --json state,baseRefName,title,mergeCommit --jq "{state, baseRefName, title, commit: .mergeCommit.oid}"
   ```

   Expect `MERGED` and `release/0.2.x`. Then `git fetch origin --tags` and `git show <stable commit>:Cargo.toml | grep -m1 "^version"` prints `version = "0.2.3"`.
4. **On `release/0.2.x`, every check passed:** `git merge-base --is-ancestor <stable commit> origin/release/0.2.x && echo "on the release branch"` prints `on the release branch`, `git merge-base --is-ancestor v0.2.3-rc.1 <stable commit> && echo "after the candidate"` prints `after the candidate`, and

   ```console
   gh api "repos/smormah/vsift/commits/<stable commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   prints nothing (the push to `release/0.2.x` runs `CI`; wait for its `CI result`).
5. **The candidate is the one you mean:** `git tag --list "v0.2.3-rc.*"` prints `v0.2.3-rc.1` only, and `git tag --list "v0.2.3"` prints nothing.
6. **The delta check, locally, at the stable commit** (the check is compiled from the commit it judges, so check by hand too):

   ```console
   git switch --detach <stable commit>
   cargo run --locked -p vsift-release -- candidate-delta
   ```

   Every file is `version string only`, `shipped document` or `work record`, nothing is `REFUSED`, and it ends "The differences are limited to version strings, the launcher's README, the installation guide and the work record." By hand:

   ```console
   git diff --stat v0.2.3-rc.1 <stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml
   ```

   It must list **only** `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/package.json` and `npm/vsift-cli/README.md`. **Any line under `crates/`, `tools/`, `.github/`, `skills/`, `schemas/`, `fixtures/` or any other file of `npm/` means the check was bypassed or wrong: do not publish.** The version files may differ in the version text only:

   ```console
   git diff -U0 v0.2.3-rc.1 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep -c "^[-+][^-+]"
   git diff -U0 v0.2.3-rc.1 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep "^[-+][^-+]" | sed 's/^[-+]//; s/0\.2\.3-rc\.1/0.2.3/' | sort | uniq -c | awk '$1 % 2 == 1'
   git diff --name-only v0.2.3-rc.1 <stable commit> -- docs/planning/delivery-ledger.json docs/guide/reference docs/guide/files
   git diff --name-only v0.2.3-rc.1 <stable commit> | grep -v -e "^CHANGELOG.md$" -e "^memory/" -e "^docs/decisions/" -e "^docs/history/" -e "^docs/planning/" -e "^docs/guide/"
   ```

   Expect `58`, then nothing, then nothing, then exactly these seven lines: `Cargo.lock`, `Cargo.toml`, `docs/operations/install.md`, `fuzz/Cargo.lock`, `fuzz/Cargo.toml`, `npm/vsift-cli/README.md`, `npm/vsift-cli/package.json`. Then `git switch -` (back to your branch).
7. (The model page's, unchanged.)
8. **The evidence ledger is complete for the candidate** (the plan's guard 6; run it yourself first):

   ```console
   cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.3-rc.1 --commit 09e40398894fddebe8aa94cf3d871eab5b8ef7ec
   ```

   Expect "VSift release evidence is complete for 0.2.3-rc.1 at 09e40398894f." and exit status 0. It says the ledger's records are complete, not that the evidence is good enough to ship: the five items above are the part to read. (`--complete-for 0.2.3` fails until the records change after the publish copies the run's `release-delta.json` into the ledger: the carried items need that record, [L-103](known-limits.md#l-103).)
9. **What npm holds now**, from a shell with no npm login (an empty `NPM_CONFIG_USERCONFIG` and `NPM_CONFIG_GLOBALCONFIG`, two different empty files): `npm view vsift-cli dist-tags` and the same for `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64` print **`{ latest: '0.2.2', next: '0.2.3-rc.1' }` four times** (anything else means something was published or moved since 2026-10-25: stop and tell the coordinator), and `npm view vsift-cli@0.2.3 version` answers 404. Then write down every published version's bytes, so that step 7 can show they are untouched, and the tags' commits:

   ```console
   for v in 0.0.0-stage 0.0.0 0.1.0 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2 0.2.1-rc.3 0.2.1 0.2.2-rc.1 0.2.2 0.2.3-rc.1; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do i="$(npm view "$p@$v" dist.integrity 2>/dev/null)"; [ -n "$i" ] && echo "$p@$v $i"; done; done
   for t in v0.1.0 v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2 v0.2.1-rc.3 v0.2.1 v0.2.2-rc.1 v0.2.2 v0.2.3-rc.1; do git show --no-patch --format="$t %H" "$t^{commit}"; done
   ```

   The first prints 53 lines (`0.0.0-stage` exists for `@vsift/darwin-arm64` only); the 45 values of `0.0.0-stage` to `0.2.2-rc.1` are those of [`release-0.2.2-stable-steps.md`](release-0.2.2-stable-steps.md) (13 of them) and `release.md` 6.19 (the 32 of `0.2.0-rc.1` to `0.2.1`), and the eight new ones were read on 2026-10-25:

   ```text
   vsift-cli@0.2.2 sha512-qEBm0Erv2TItjBttBft1FqUTwQHqQjQOO0BdDxlZYnxOzYbzejqGpUg+SE+AVJHSfjZJdpMRLX+m6cS/kFJb+A==
   @vsift/win32-x64@0.2.2 sha512-J0HhLLsT8mM4oKlVGFNWGUKYALerQONmdIL5pChLRZqaUT5wJj3BUxoliSCHCSBow01k1n0xq45JEjDceJb5LQ==
   @vsift/darwin-arm64@0.2.2 sha512-8Cl9Vzh0+rKE/Oo6E0zTSx1pbUXGbn6315YxuMTOxmudFH/zJMtgQMexNwt6El5GKazik91THi5rtu+UuiyV0g==
   @vsift/linux-x64@0.2.2 sha512-ahqEQuxuguH/Cd+EpNYN6AfERDYklzVcGnOs3fHHV3hZLWlkDQj2DQJ12hgpbnpoUOhqAM4CmJqnSdVz5O4BaA==
   vsift-cli@0.2.3-rc.1 sha512-HzDC3OK5fmyo87Zq/iO5okkbIQkwoMzT508/lJ+GknyWdihrzc5FE8oiXl+J9jJ0KA3VEw3LJTMzZYPwcaHt8g==
   @vsift/win32-x64@0.2.3-rc.1 sha512-QfXArpFy60mgsONnm2bxcsmUuDZgaBqYpKthDVRGQnWn56sdxf2Q6Hv9F5c0PurPxJ34qe+Upee1O/QR2Z119A==
   @vsift/darwin-arm64@0.2.3-rc.1 sha512-TjxGYf0rfJSJ/SikoNXTR6jPrPdcRRruG2JZ2KVQScbby+BjJuEmonUBnFhfKKp9QrWaNt5qn2WlfBaRGfOmIA==
   @vsift/linux-x64@0.2.3-rc.1 sha512-KbBLL58Bu1bsX2v55mGMtw3sXAzXlUY9N2ux0H8cp/vYH9kW67XZFOTh6V0/RDAo32jBXU766dJHNbxKVJV8CA==
   ```

   The second prints the twelve annotated tags with `011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa` (`v0.1.0`), the commits of `v0.2.0-rc.1` to `v0.2.1` (6.19), `43a033dbf81346b967b293a150e13306828cac50` (`v0.2.2-rc.1`), `0dadc69df4f5c5c3264be72f04be26a600620fe1` (`v0.2.2`) and `09e40398894fddebe8aa94cf3d871eab5b8ef7ec` (`v0.2.3-rc.1`).

If any item is not as stated, do not tag; tell the coordinator what you saw.

## 2. Tag the stable commit (the maintainer's own git identity as tagger)

```console
git tag -a v0.2.3 -m "VSift 0.2.3" <stable commit>
git push origin v0.2.3
git show --no-patch --format="%H" "v0.2.3^{commit}"
git cat-file -t v0.2.3
git fetch origin && git merge-base --is-ancestor <stable commit> origin/release/0.2.x && echo "on the release branch"
```

The third command prints the 40 digits you wrote down and the fourth `tag`. A wrong tag **before anything is published** may be moved (`git push origin :refs/tags/v0.2.3`, fix, tag again; the `guard` and the publish's re-check require the tag to name the run's commit); **never after step 4 has started.**

## 3 to 6. Dry run, publish, read the plan, approve

As the model page's steps 3 to 6, with `v0.2.3` and these differences.

**Dry run:**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.3 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --limit 3 --json databaseId,headBranch,headSha,event,status,conclusion,url
```

Pick the run whose `headBranch` is `v0.2.3` and whose `headSha` is the commit you wrote down; write its id down. The `guard` job answers within a minute (the tag of the workspace version, the tag naming the commit, the commit on `release/0.2.x` with the five rule types; no nightly is asked of a stable version). The **Publish plan** summary must show "Version `0.2.3` (stable), npm dist-tag `latest`, Git tag `v0.2.3`"; the table "What this publication does to the dist-tags" with, **for each of the four packages, `latest` from `0.2.2` to `0.2.3` and `next` staying `0.2.3-rc.1`**; six guards, **all `passed`** (Stable version; Accepted candidate "against `v0.2.3-rc.1` (commit `09e40398894f`): 5 version-string, 2 shipped-document and 11 work-record files differ, nothing else" (11 at the head of the pull request); Registry read; Candidate published ("`0.2.3-rc.1` is published on all four packages"); `latest` moves forward ("`latest` is `0.2.2` on all four packages, a stable version below `0.2.3`, and this version is on none of them"); Evidence ledger ("VSift release evidence is complete for 0.2.3-rc.1 at 09e40398894f.")); the section "The evidence ledger's `release_delta` record" with `"verdict": "allowed"`, `"candidate_version": "0.2.3-rc.1"` and `"stable_version": "0.2.3"`; the four `npm publish ... --tag latest --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`; and the GitHub release commands (draft, then published as **latest**), with ten assets. Download the plan and read the notes:

```console
gh run download <dry-run id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

**Publish:**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.3 -f dry_run=false
gh run list --repo smormah/vsift --workflow release.yml --limit 3 --json databaseId,headBranch,headSha,event,status,conclusion,url
gh run download <publish id> --repo smormah/vsift --name publish-plan --dir plan-publish
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

**Write the publish run's id down: it is the run npm's provenance names.** Its plan must say `Publish plan: PUBLISH a STABLE release after the release environment's approval` and "Afterwards `npm install vsift-cli` installs `0.2.3`." **Keep `plan-publish/release-delta.json`** (the artifact expires seven days after the run; the records change copies it into the ledger). Then **Review deployments**, tick `release`, **Approve and deploy** (the maintainer; the coordinator never approves). After the approval the `publish` job asks the `guard`'s questions again (a moved tag or a branch that lost the commit or a rule refuses it), publishes the four packages with `--tag latest`, reads `latest` back every 30 seconds for up to 20 minutes with `next` unchanged (a wrong answer fails at once), creates the GitHub release as a draft and publishes it as latest, and requires GitHub's own latest release to be `v0.2.3`. If the read-back runs out of time with the packages published, **Re-run failed jobs** on the same run finishes it (6.5), approved again.

## 7. Check it from the outside (a shell with no npm login and a folder of your own; about ten minutes)

`npm view vsift-cli dist-tags` (and the three platform packages) prints **`{ latest: '0.2.3', next: '0.2.3-rc.1' }` four times.** The step 1 item 9 loop prints the same 53 values and the same twelve tag commits (and the new tag `v0.2.3`, which is not in the loop). Install it without a tag:

```console
mkdir stable-check && cd stable-check && echo '{"private": true}' > package.json
npm install vsift-cli
npm audit signatures
npx vsift --version
gh release view v0.2.3 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `vsift 0.2.3 (<the first 12 digits of the stable commit>)`, verified registry signatures and attestations, `isPrerelease` false, `isDraft` false and ten assets, and `v0.2.3` (if not: `gh release edit v0.2.3 --repo smormah/vsift --latest`). The checksums and attestations are `release.md` 6.4's commands with `v0.2.3`, `--source-ref refs/tags/v0.2.3`, `--signer-workflow smormah/vsift/.github/workflows/release.yml` and `--deny-self-hosted-runners`. **Tell the coordinator** the two run ids, the four `npm view` answers, the output of `npm audit signatures`, the two `gh` answers and anything that failed or was re-run. Nothing is announced.

## 8. If something fails

The model page's table applies with these numbers. Three differ: "Candidate published" refused means `0.2.3-rc.1` is not on all four packages; "`latest` moves forward" refused means `latest` is not `0.2.2` or `0.2.3` is already on npm with other bytes; "Evidence ledger" refused means the ledger at the stable commit is not complete for `0.2.3-rc.1` at `09e40398894f`. The `guard` refusing the dispatch names its reason (the wrong ref, a tag off `release/0.2.x`, a branch without its rule types). **A published version that is bad is never unpublished:** `npm deprecate` it on all four packages, edit the release's notes, and publish `0.2.4` (the hot fix is already planned on this branch) after its own candidate (`release.md` 6.5); until then `latest` names the bad version, and pointing `latest` back at `0.2.2` (`npm dist-tag add <package>@0.2.2 latest` for each package, two-factor authentication) is safe and your choice at the time.

## 9. Deprecations (only what the maintainer decides: `0.2.3-rc.1`, decided 2026-10-25; confirm the text)

After step 7 has passed. The maintainer logs in and approves each npm write with the second factor; a session may type the commands in the maintainer's own terminal under that login, and nothing else runs them (`npm whoami` first: a lapsed login is found here otherwise). **Four writes, for `0.2.3-rc.1` only:**

```console
npm deprecate "vsift-cli@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
npm deprecate "@vsift/win32-x64@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
npm deprecate "@vsift/darwin-arm64@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
npm deprecate "@vsift/linux-x64@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
```

Read back with `npm view "<package>@0.2.3-rc.1" deprecated` for each package (it prints the message); `0.2.3` and every other version print what they printed before (empty, or their earlier message; `0.2.2` prints nothing, as it is not deprecated). An empty message removes a deprecation. The tags do not move, and `next` still names `0.2.3-rc.1`, now with npm's deprecation warning (the same state `0.2.2-rc.1` has been in since 2026-10-18, [L-108](known-limits.md#l-108)).

## 10. The hosted checks of the published bytes (none of them can publish)

Each is one dispatch; runs of one workflow on one ref share a concurrency group and **a newer waiting run replaces an older waiting one without a word**, so dispatch the second `P14 published artifacts` after the first has finished. A failed run is a finding: open an issue before re-running (governance rule 14). Dispatched on the tag `v0.2.3`:

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref v0.2.3 -f version=0.2.3
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref v0.2.3 -f published_version=0.2.3
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.3 -f version=0.2.3 -f from_version=0.2.2
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.3 -f version=0.2.3 -f from_version=0.2.3-rc.1
gh workflow run p14-scan-reading.yml --repo smormah/vsift --ref v0.2.3 -f version=0.2.3
```

**`P14 journeys` is dispatched on `main`, not on the tag,** because the tag's test driver is the candidate's and runs the noisy-speech set on hosted macOS, which does not fit that runner (#520, L-230):

```console
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.3 -f tests_from=workflow-ref
```

This is the route the maintainer decided on 2026-10-25 for the candidate (run 40154146940: the published binary under test, the test driver from `main`, all three systems passed, the noisy-speech set skipped on macOS and gated on Ubuntu and Windows by `P07 local ASR`); **using it for the stable's own bytes is this page's proposal, for the maintainer to confirm.** `main` has moved on since that run, so a failure in a checkpoint that `0.3.0` work changed is a finding of the test driver and not of `0.2.3`; say so in the issue, and the fallback is the dispatch on the tag with `-f asr_gates=false` (no macOS accuracy gate at all, which is weaker). **Expect it all green**, `P14 verify release` with `stable: candidate-to-stable-delta` and `stable: latest-on-all-four-packages` among the passes and **within seven days of the publish** (its delta check reads `release-delta.json` from the publish run's artifact).

## 11. After the publish: the records change to `main` (6.18 step 5)

**Within seven days of the publish,** in a pull request **to `main`** that may touch any page (6.8 binds the release branch only until the publish):

1. **Copy `plan-publish/release-delta.json` into the ledger as `release_delta`** (it names `0.2.2` today; nothing copies it for you: [L-103](known-limits.md#l-103)), and give the items whose stable rule is `repeat` (RQ-01 to RQ-06, RQ-13, RQ-18, RQ-19) their own entries for `0.2.3` from the runs of step 10, the Governance job at the stable commit and a dated reading of RQ-13. The items waived for `0.2.3` (RQ-08, RQ-15, RQ-16) and the carried waivers (RQ-14, RQ-17) keep their texts. `release-evidence --complete-for 0.2.3 --commit <stable commit>` must then pass.
2. **Bring the release branch's work record and the stable commit's shipped documents to `main`** (the ledger, the planning pages, the changelog, the register, the batch records, `docs/operations/install.md`, `npm/vsift-cli/README.md`); the `Release window` workflow runs on that pull request by itself (it triggers on the ledger) and must find every window commit's lines on `main`.
3. **Fold this page into `release.md` as 6.22** and **mark 6.21 a record** (followed 2026-10-22 to 2026-10-24: nightlies 39837839311 and 39892011366, dry run 40037368499, publish 40038576713; `next` moved from `0.2.2-rc.1` to `0.2.3-rc.1`), with what the real services showed for `0.2.3`.
4. **Pages the release branch could not change:** the root `README.md`, `SECURITY.md` (the supported-versions table: `0.2.3` is the newest patch), `docs/agents/skill.md` and `docs/agents/trials.md`, `docs/development.md`. The register: L-230 (on `main`), L-231 and L-232 say which release carries them; L-137 is deleted if the maintainer agrees that the hostile-media campaign (with the malformed-model case, #399) and the dated scan reading, both passed for the candidate, settle it.
5. **The maintainer's choices:** the deprecation of step 9, moving `next` (L-108), and the hot fix `0.2.4` (the FFmpeg update, #351 and #272, and the Node.js pins of L-232) made on `release/0.2.x` after this publish.
