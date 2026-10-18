# The stable release `0.2.2`: the maintainer's steps

Status: **prepared on 2026-10-18 by the pull request whose title starts "The stable release 0.2.2" (#456), into `release/0.2.x`, and followed once the same day: `0.2.2` is published (tag `v0.2.2` at `0dadc69df4f5`, publish run 39127747535, first attempt). The steps that were followed and what the real services showed are [`release.md`](../operations/release.md) 6.20, into which this page is folded; the page is kept as it was prepared, and the deprecations of step 9 (decided on 2026-10-18: `0.2.1-rc.3` and `0.2.2-rc.1`) were done the same day.** This is the maintainer's checklist for tagging, publishing and checking `0.2.2`, the patch release after `0.2.1`, and the first stable release made on a release branch. It is a planning page and not a section of [`release.md`](../operations/release.md) because `release.md` may not change between the last candidate and the stable release (6.8); it is folded into `release.md` as **6.20** after the publish, in the records change to `main`, when 6.19 is marked a record. **Follow [`release-0.2.1-stable-steps.md`](release-0.2.1-stable-steps.md) (the page for `0.2.1`, which `release.md` 6.17 records, and which is the model), `release.md` 6.7, 6.8 and 6.18 step 5, with the numbers in the table below and the command lines on this page, which are spelled out so that they can be pasted (Git Bash; in PowerShell write `"$t^^{commit}"` for `"$t^{commit}"`).** Everything the model page says that this page does not repeat is unchanged: who does what, the order of the steps, what the dry run's plan must list, the comparison of the two plans, the approval, the failure table and the list of what is irreversible.

**Plain English first.** `0.2.2` is built from exactly the source of `0.2.2-rc.1`, the candidate published on 2026-10-18 (tag `v0.2.2-rc.1` at `43a033dbf81346b967b293a150e13306828cac50` on `release/0.2.x`): the stable commit differs from that tag only in its version numbers, the launcher's README, the installation guide and the work record, and a program checks that. Publishing it moves npm's `latest` from `0.2.1` to `0.2.2` on all four packages; after that, `npm install vsift-cli` with no tag installs `0.2.2`. `next` is not moved (it stays `0.2.2-rc.1`; only the maintainer's own command moves it: [L-108](known-limits.md#l-108)). A published version can never be taken back. **Which versions are deprecated at this release is not decided** (open in [`release-0.2.2.md`](release-0.2.2.md): whether `0.2.1-rc.3`, and `0.2.2-rc.1`, are); step 9 holds the commands for whatever the maintainer decides.

**The runbook's record for the candidate.** `release.md` 6.19 was followed once, on 2026-10-17 and 2026-10-18: the nightly was run 39058441764, the dry run 39077367812, the publish 39078284454 (first attempt), `next` moved from `0.2.1-rc.3` to `0.2.2-rc.1`, and `latest` stayed `0.2.1`. Its evidence is recorded in [`release-0.2.2.md`](release-0.2.2.md), "The evidence on 0.2.2-rc.1" (#455).

## The numbers that change (read the model page with this table)

| | `0.2.1` (the model page, 2026-10-15) | `0.2.2` (this page) |
| --- | --- | --- |
| The version, the tag, the commit | `0.2.1`, `v0.2.1`, the merge commit on `main` of "The stable release 0.2.1" | `0.2.2`, `v0.2.2` (annotated, message `VSift 0.2.2`), **the merge commit on `release/0.2.x` of the pull request whose title starts "The stable release 0.2.2"** |
| Where the commit and the tag are | `main` | **`release/0.2.x`** (the `guard` job refuses a tag its branch does not contain; a stable version needs no nightly of its own, 6.18) |
| The accepted candidate | `v0.2.1-rc.3` at `fa380021f2aa` | **`v0.2.2-rc.1` at `43a033dbf81346b967b293a150e13306828cac50`** (the only, so the highest, `v0.2.2-rc.N` tag; the check ignores every tag of another `X.Y.Z`) |
| What npm holds before | `latest` `0.2.0`, `next` `0.2.1-rc.3` | **`latest` `0.2.1`, `next` `0.2.2-rc.1`**, on all four packages (read 2026-10-18, below) |
| What the plan does | `latest` from `0.2.0` to `0.2.1` | **`latest` from `0.2.1` to `0.2.2`; `next` stays `0.2.2-rc.1`** |
| The `latest moves forward` guard | `latest` is `0.2.0` on all four | `latest` is **`0.2.1`**, a stable version below `0.2.2`, and `0.2.2` is on none of them |
| The evidence ledger guard | complete for `0.2.1-rc.3` at `fa380021f2aa` | complete for **`0.2.2-rc.1` at `43a033dbf813`** (passes since #455) |
| The candidate-to-stable file counts | 5 version-string, 2 shipped-document, 77 work-record files | 5 version-string, 2 shipped-document and 81 work-record files (at the head of the pull request: the records of the candidate, #455, and this commit; more if a work-record pull request merges before the tag) |
| The read-back after the publish | five minutes (it ran out, #403) | **up to 20 minutes** (since #422; `next` read back in about two minutes for the candidate) |
| The before-snapshot of the earlier versions | seven versions, twenty-eight values, seven tags | **every published version: 45 integrity values and 10 tags** (below) |
| What `vsift --version` prints | `vsift 0.2.1 (e9ab6e626733)` | `vsift 0.2.2 (<the first 12 digits of the stable commit>)` |
| The GitHub release | becomes GitHub's latest, from `v0.2.0` | becomes GitHub's latest, **from `v0.2.1`** to `v0.2.2` |
| The deprecations at the stable | `0.2.1-rc.1` and `0.2.1-rc.2` | **not decided** (step 9) |
| The hosted checks after the publish | dispatched from `main` | **dispatched on the tag `v0.2.2`** (as for `0.2.2-rc.1`, 6.19); `P14 verify release` **within seven days of the publish** |

## What the release ships untried (read before you decide to publish)

Three evidence items are `waived` for `0.2.2`, none is a pass, and `release-evidence` cannot see the limits of a waiver ([L-179](known-limits.md#l-179)); the maintainer's decisions of 2026-10-18 ([`release-0.2.2.md`](release-0.2.2.md), "The maintainer's decisions of 2026-10-18") do: **RQ-16** (the usefulness part: without the skill a compact Claude model was useful in 4 of 6 cold runs and 7 of 12 with six reserve runs; [#454](https://github.com/smormah/vsift/issues/454), [L-178](known-limits.md#l-178)), and the standing waivers **RQ-14** (the strict worker, ADR 0024 decision E, until #188 in R1) and **RQ-17** (no Smart App Control, clean-machine or Mac try-out, until the try-out with LOKI in cycle 3; [L-143](known-limits.md#l-143)). Two of the skill's three changes were not exercised by any trial scenario, and #404's fix passed once on hosted Windows. `install.md` and the launcher's README say the same things in the words a user reads. If any of this makes you want to wait, stop at step 1: nothing is irreversible before step 4.

## 0. Before you merge the pull request

1. **The records of the candidate are merged** (#455, `0f97a9b`), so the plan's evidence guard is complete for `0.2.2-rc.1`; this pull request is cut from that commit.
2. **Merge nothing else into `release/0.2.x`** between this pull request and the publish except a work-record pull request (`CHANGELOG.md`, `memory/`, `docs/decisions/`, `docs/history/`, `docs/planning/` and `docs/guide/` pages, not `docs/guide/reference/`, `docs/guide/files/` or `docs/planning/delivery-ledger.json`): **a page of `docs/agents/` or `docs/operations/`, `SECURITY.md` or the root README is refused** (6.8). `main` is not frozen and keeps taking merges; nothing of it reaches the branch.
3. **The window this opens is the branch's** ([L-133](known-limits.md#l-133)): on merge the installation guide and the launcher's README on `release/0.2.x` say that `npm install vsift-cli` installs `0.2.2`, which is true only after step 6; `main`'s copies keep `0.2.1`'s wording until the records change after the publish. Merge when you can tag and publish within the day.

## 1. Preflight (about 30 minutes; only items 1 and 2 need a browser)

Items 1, 2, 7, 10, 11 and 12 are the model page's, unchanged (trusted publishers saved on all four packages, fork-pull-request approval, the local gates, the `release` environment, the tag ruleset and **the release-branch ruleset** (6.2 step 8), reading what ships, the choice about `next`). The ones that change:

3. **Find the stable commit**, the merge commit of the pull request whose title starts "The stable release 0.2.2". Write its 40 digits down; `<stable commit>` below is that:

   ```console
   gh pr view <number> --repo smormah/vsift --json state,baseRefName,title,mergeCommit --jq "{state, baseRefName, title, commit: .mergeCommit.oid}"
   ```

   Expect `MERGED` and `release/0.2.x`. Then `git fetch origin --tags` and `git show <stable commit>:Cargo.toml | grep -m1 "^version"` prints `version = "0.2.2"`.
4. **On `release/0.2.x`, every check passed:** `git merge-base --is-ancestor <stable commit> origin/release/0.2.x && echo "on the release branch"` prints `on the release branch`, `git merge-base --is-ancestor v0.2.2-rc.1 <stable commit> && echo "after the candidate"` prints `after the candidate`, and

   ```console
   gh api "repos/smormah/vsift/commits/<stable commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   prints nothing (the push to `release/0.2.x` runs `CI`; wait for its `CI result`).
5. **The candidate is the one you mean:** `git tag --list "v0.2.2-rc.*"` prints `v0.2.2-rc.1` only, and `git tag --list "v0.2.2"` prints nothing.
6. **The delta check, locally, at the stable commit** (the check is compiled from the commit it judges, so check by hand too):

   ```console
   git switch --detach <stable commit>
   cargo run --locked -p vsift-release -- candidate-delta
   ```

   Every file is `version string only`, `shipped document` or `work record`, nothing is `REFUSED`, and it ends "The differences are limited to version strings, the launcher's README, the installation guide and the work record." By hand:

   ```console
   git diff --stat v0.2.2-rc.1 <stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml
   ```

   It must list **only** `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/package.json` and `npm/vsift-cli/README.md`. **Any line under `crates/`, `tools/`, `.github/`, `skills/`, `schemas/`, `fixtures/` or any other file of `npm/` means the check was bypassed or wrong: do not publish.** The version files may differ in the version text only:

   ```console
   git diff -U0 v0.2.2-rc.1 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep -c "^[-+][^-+]"
   git diff -U0 v0.2.2-rc.1 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep "^[-+][^-+]" | sed 's/^[-+]//; s/0\.2\.2-rc\.1/0.2.2/' | sort | uniq -c | awk '$1 % 2 == 1'
   git diff --name-only v0.2.2-rc.1 <stable commit> -- docs/planning/delivery-ledger.json docs/guide/reference docs/guide/files
   git diff --name-only v0.2.2-rc.1 <stable commit> | grep -v -e "^CHANGELOG.md$" -e "^memory/" -e "^docs/decisions/" -e "^docs/history/" -e "^docs/planning/" -e "^docs/guide/"
   ```

   Expect `52`, then nothing, then nothing, then exactly these seven lines: `Cargo.lock`, `Cargo.toml`, `docs/operations/install.md`, `fuzz/Cargo.lock`, `fuzz/Cargo.toml`, `npm/vsift-cli/README.md`, `npm/vsift-cli/package.json`. Then `git switch -` (back to your branch).
7. (The model page's, unchanged.)
8. **The evidence ledger is complete for the candidate** (the plan's guard 6; run it yourself first):

   ```console
   cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.2-rc.1 --commit 43a033dbf81346b967b293a150e13306828cac50
   ```

   Expect "VSift release evidence is complete for 0.2.2-rc.1 at 43a033dbf813." and exit status 0. It says the ledger's records are complete, not that the evidence is good enough to ship: the three waivers above are the part to read. (`--complete-for 0.2.2` fails until the records change after the publish copies the run's `release-delta.json` into the ledger: the carried items need that record, [L-103](known-limits.md#l-103).)
9. **What npm holds now**, from a shell with no npm login (an empty `NPM_CONFIG_USERCONFIG` and `NPM_CONFIG_GLOBALCONFIG`, two different empty files): `npm view vsift-cli dist-tags` and the same for `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64` print **`{ latest: '0.2.1', next: '0.2.2-rc.1' }` four times** (anything else means something was published or moved since 2026-10-18: stop and tell the supervisor), and `npm view vsift-cli@0.2.2 version` answers 404. Then write down every published version's bytes, so that step 7 can show they are untouched, and the tags' commits:

   ```console
   for v in 0.0.0-stage 0.0.0 0.1.0 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2 0.2.1-rc.3 0.2.1 0.2.2-rc.1; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do i="$(npm view "$p@$v" dist.integrity 2>/dev/null)"; [ -n "$i" ] && echo "$p@$v $i"; done; done
   for t in v0.1.0 v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2 v0.2.1-rc.3 v0.2.1 v0.2.2-rc.1; do git show --no-patch --format="$t %H" "$t^{commit}"; done
   ```

   The first prints 45 lines (`0.0.0-stage` exists for `@vsift/darwin-arm64` only); the 32 values of `0.2.0-rc.1` to `0.2.1` are those of `release.md` 6.19, and the others read on 2026-10-18 were:

   ```text
   @vsift/darwin-arm64@0.0.0-stage sha512-4RHZ8TFzdl8IdydZh8uTFuJOIpf57sXwdGXeYm4rQAebKzUqrrhry+p6aqoOP1Uhag8j1ouCLJ3eH9HZ1UGUkQ==
   vsift-cli@0.0.0 sha512-ek3RCw6PpTFxwtPfnl5TZcIhDawhCS5pCfCy+ujR33naOxe4NVnu2TzrtVqS/1sp47zbWKnHphHmXixuUmVUpQ==
   @vsift/win32-x64@0.0.0 sha512-2UOTc+KVJws+by7uSf4xvA73Ov91kBNJbf26aqozxItgKO1WOC7ZqqI2sGQCfL5P+qG8UpyLs7fwaAiYHosdEQ==
   @vsift/darwin-arm64@0.0.0 sha512-4DTi2hVvTgPkufMYD+Gc/vqzt6Hho/iULXGF/XMQQ3RWXdTOeyA5oXIvY3qPwftbnoE9XNowJGvc2JN5cPoykg==
   @vsift/linux-x64@0.0.0 sha512-Tk5p31GRqyW9SCiAKjOrrrCv4SR5TX5yWPRX8rFdXXhSOyDUML+NEVVo+yYXzUoCst6EAm/VBsQowD+0ue1+IA==
   vsift-cli@0.1.0 sha512-uPlZJGDcG/9cT2vYqCm688TBuLtdw0e5xPI+0HVLi8UdAsq/w+vGyVYIIHn0YhEcUaOTCQ5BfEZdrZow5WSanw==
   @vsift/win32-x64@0.1.0 sha512-EXZojaYMqbrbRRzgRy5AJ0NBZWvIfNHbOm/5QNopLbKwIYZ0U5/9mdmh2DAI1Qvk09qVMxhxw4t7UBRtz2W5CA==
   @vsift/darwin-arm64@0.1.0 sha512-pVKeq9Vk4FY62paXA9BkDX+bFZpX2052IckmBmxaeNks49KiAOuvsz3awqNE2nCmgqQFwo1y07ewexRu11nbMg==
   @vsift/linux-x64@0.1.0 sha512-aktF6eCGgH2D0cG2lwUCKc46FbEyTEcNV7OaXNS2sT0geLf891iFEbGJYlnYFJncuhHL7VcgmrEkedYEV3jgMw==
   vsift-cli@0.2.2-rc.1 sha512-8hIA1viKicezxvuuCLAwFp36U4JTdUANHCVQ5Oji5lsyAPQ0G8e3DbViqBxXF/va0FShr5XkQqb54tAZqDD7Mw==
   @vsift/win32-x64@0.2.2-rc.1 sha512-z9dT1uPain5dsuRCbxacEU2FlLLsL+GGpX9FrjW9o1dU94m5luyk1wOBPzjKny5QaaBWK2xX/0WbL7mhCu9vgg==
   @vsift/darwin-arm64@0.2.2-rc.1 sha512-b7BsalVHW60y5o3baDsW2LPutEURtuLnHVi85OjM9wC//+6opRaqyDWBq0j1K8S9FPsI5WNs2kZy0lfPqLjiuQ==
   @vsift/linux-x64@0.2.2-rc.1 sha512-DPM+KiqkNdQtrVSopWKSchhAyc7iuGHp3ZpX7hO1iITlxl3gffIObfOrMSbXc9/Con80ZofH/mUmwstb5laUIQ==
   ```

   The second prints the ten annotated tags with `011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa` (`v0.1.0`), the eight commits of 6.19 (`v0.2.0-rc.1` to `v0.2.1`) and `43a033dbf81346b967b293a150e13306828cac50` (`v0.2.2-rc.1`).

If any item is not as stated, do not tag; tell the supervisor what you saw.

## 2. Tag the stable commit (the maintainer's own git identity as tagger)

```console
git tag -a v0.2.2 -m "VSift 0.2.2" <stable commit>
git push origin v0.2.2
git show --no-patch --format="%H" "v0.2.2^{commit}"
git cat-file -t v0.2.2
git fetch origin && git merge-base --is-ancestor <stable commit> origin/release/0.2.x && echo "on the release branch"
```

The third command prints the 40 digits you wrote down and the fourth `tag`. A wrong tag **before anything is published** may be moved (`git push origin :refs/tags/v0.2.2`, fix, tag again; the `guard` and the publish's re-check require the tag to name the run's commit); **never after step 4 has started.**

## 3 to 6. Dry run, publish, read the plan, approve

As the model page's steps 3 to 6, with `v0.2.2` and these differences.

**Dry run:**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.2 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --limit 3 --json databaseId,headBranch,headSha,event,status,conclusion,url
```

Pick the run whose `headBranch` is `v0.2.2` and whose `headSha` is the commit you wrote down; write its id down. The `guard` job answers within a minute (the tag of the workspace version, the tag naming the commit, the commit on `release/0.2.x` with the five rule types; no nightly is asked of a stable version). The **Publish plan** summary must show "Version `0.2.2` (stable), npm dist-tag `latest`, Git tag `v0.2.2`"; the table "What this publication does to the dist-tags" with, **for each of the four packages, `latest` from `0.2.1` to `0.2.2` and `next` staying `0.2.2-rc.1`**; six guards, **all `passed`** (Stable version; Accepted candidate "against `v0.2.2-rc.1` (commit `43a033dbf813`): 5 version-string, 2 shipped-document and 81 work-record files differ, nothing else" (81 at the head of the pull request); Registry read; Candidate published ("`0.2.2-rc.1` is published on all four packages"); `latest` moves forward ("`latest` is `0.2.1` on all four packages, a stable version below `0.2.2`, and this version is on none of them"); Evidence ledger ("VSift release evidence is complete for 0.2.2-rc.1 at 43a033dbf813.")); the section "The evidence ledger's `release_delta` record" with `"verdict": "allowed"`, `"candidate_version": "0.2.2-rc.1"` and `"stable_version": "0.2.2"`; the four `npm publish ... --tag latest --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`; and the GitHub release commands (draft, then published as **latest**), with ten assets. Download the plan and read the notes:

```console
gh run download <dry-run id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

**Publish:**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.2 -f dry_run=false
gh run list --repo smormah/vsift --workflow release.yml --limit 3 --json databaseId,headBranch,headSha,event,status,conclusion,url
gh run download <publish id> --repo smormah/vsift --name publish-plan --dir plan-publish
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

**Write the publish run's id down: it is the run npm's provenance names.** Its plan must say `Publish plan: PUBLISH a STABLE release after the release environment's approval` and "Afterwards `npm install vsift-cli` installs `0.2.2`." **Keep `plan-publish/release-delta.json`** (the artifact expires seven days after the run; the records change copies it into the ledger). Then **Review deployments**, tick `release`, **Approve and deploy** (the maintainer; the supervisor never approves). After the approval the `publish` job asks the `guard`'s questions again (a moved tag or a branch that lost the commit or a rule refuses it), publishes the four packages with `--tag latest`, reads `latest` back every 30 seconds for up to 20 minutes with `next` unchanged (a wrong answer fails at once), creates the GitHub release as a draft and publishes it as latest, and requires GitHub's own latest release to be `v0.2.2`. If the read-back runs out of time with the packages published, **Re-run failed jobs** on the same run finishes it (6.5), approved again.

## 7. Check it from the outside (a shell with no npm login and a folder of your own; about ten minutes)

`npm view vsift-cli dist-tags` (and the three platform packages) prints **`{ latest: '0.2.2', next: '0.2.2-rc.1' }` four times.** The step 1 item 9 loop prints the same 45 values and the same ten tag commits (and the new tag `v0.2.2`, which is not in the loop). Install it without a tag:

```console
mkdir stable-check && cd stable-check && echo '{"private": true}' > package.json
npm install vsift-cli
npm audit signatures
npx vsift --version
gh release view v0.2.2 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `vsift 0.2.2 (<the first 12 digits of the stable commit>)`, verified registry signatures and attestations, `isPrerelease` false, `isDraft` false and ten assets, and `v0.2.2` (if not: `gh release edit v0.2.2 --repo smormah/vsift --latest`). The checksums and attestations are `release.md` 6.4's commands with `v0.2.2`, `--source-ref refs/tags/v0.2.2`, `--signer-workflow smormah/vsift/.github/workflows/release.yml` and `--deny-self-hosted-runners`. **Tell the supervisor** the two run ids, the four `npm view` answers, the output of `npm audit signatures`, the two `gh` answers and anything that failed or was re-run. Nothing is announced.

## 8. If something fails

The model page's table applies with these numbers. Three differ: "Candidate published" refused means `0.2.2-rc.1` is not on all four packages; "`latest` moves forward" refused means `latest` is not `0.2.1` or `0.2.2` is already on npm with other bytes; "Evidence ledger" refused means the ledger at the stable commit is not complete for `0.2.2-rc.1` at `43a033dbf813`. The `guard` refusing the dispatch names its reason (the wrong ref, a tag off `release/0.2.x`, a branch without its rule types). **A published version that is bad is never unpublished:** `npm deprecate` it on all four packages, edit the release's notes, and publish `0.2.3` after its own candidate (`release.md` 6.5); until then `latest` names the bad version, and pointing `latest` back at `0.2.1` (`npm dist-tag add <package>@0.2.1 latest` for each package, two-factor authentication) is safe and your choice at the time.

## 9. Deprecations (only what the maintainer decides; none is decided)

After step 7 has passed, and only for the versions the maintainer names. The maintainer logs in and approves each npm write with the second factor; a session may type the commands in the maintainer's own terminal under that login, and nothing else runs them. For each chosen `<version>` and each of the four packages (four writes a version):

```console
npm deprecate "vsift-cli@<version>" "Superseded by 0.2.2. Install vsift-cli."
npm deprecate "@vsift/win32-x64@<version>" "Superseded by 0.2.2. Install vsift-cli."
npm deprecate "@vsift/darwin-arm64@<version>" "Superseded by 0.2.2. Install vsift-cli."
npm deprecate "@vsift/linux-x64@<version>" "Superseded by 0.2.2. Install vsift-cli."
```

Read back with `npm view "<package>@<version>" deprecated`; `0.2.2` and every version not chosen print what they printed before (empty, or their earlier message). An empty message removes a deprecation. The tags do not move.

## 10. The hosted checks of the published bytes (dispatched on the tag; none of them can publish)

Each is one dispatch; runs of one workflow on one ref share a concurrency group and **a newer waiting run replaces an older waiting one without a word**, so dispatch the second `P14 published artifacts` after the first has finished. A failed run is a finding: open an issue before re-running (governance rule 14).

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref v0.2.2 -f version=0.2.2
gh workflow run p14-journeys.yml --repo smormah/vsift --ref v0.2.2 -f version=0.2.2
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref v0.2.2 -f published_version=0.2.2
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.2 -f version=0.2.2 -f from_version=0.2.1
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.2 -f version=0.2.2 -f from_version=0.2.2-rc.1
gh workflow run p14-scan-reading.yml --repo smormah/vsift --ref v0.2.2 -f version=0.2.2
```

**Dispatch `P14 verify release` within seven days of the publish:** its delta check reads `release-delta.json` from the publish run's artifact. **Expect it all green**, with `stable: candidate-to-stable-delta` and `stable: latest-on-all-four-packages` among the passes. **`P14 journeys` is RQ-05 for `0.2.2` and the last evidence for #404:** its Windows job's `p11_shutdown_and_redelivery` stage must pass.

## 11. After the publish: the records change to `main` (6.18 step 5)

**Within seven days of the publish,** in a pull request **to `main`** that may touch any page (6.8 binds the release branch only until the publish):

1. **Copy `plan-publish/release-delta.json` into the ledger as `release_delta`** (it names `0.2.1` today; nothing copies it for you: [L-103](known-limits.md#l-103)), and give the items whose stable rule is `repeat` (RQ-01 to RQ-06, RQ-13, RQ-18, RQ-19) their own entries for `0.2.2` from the runs of step 10, the Governance job at the stable commit and a dated reading of RQ-13. `release-evidence --complete-for 0.2.2 --commit <stable commit>` must then pass.
2. **Bring the release branch's work record and the stable commit's shipped documents to `main`** (the ledger, the planning pages, the changelog, the register, the batch records, `docs/operations/install.md`, `npm/vsift-cli/README.md`); the `Release window` workflow runs on that pull request by itself (it triggers on the ledger) and must find every window commit's lines on `main`.
3. **Fold this page into `release.md` as 6.20** and **mark 6.19 a record** (followed once, 2026-10-17 and 2026-10-18: nightly 39058441764, dry run 39077367812, publish 39078284454; `next` moved from `0.2.1-rc.3` to `0.2.2-rc.1`), with what the real services showed for `0.2.2`.
4. **Pages the release branch could not change:** the root `README.md`, `SECURITY.md` (the supported-versions table: `0.2.2` is the newest patch), `docs/agents/skill.md` and `docs/agents/trials.md` (the batches ran on the candidate), `docs/development.md`. The register: close L-167 and #404 with the journeys of step 10, delete L-162 (#394 shipped with its fuzz evidence), and re-read L-151, L-152, L-163 and L-164 with the issues' own criteria.
5. **The maintainer's choices:** the deprecations of step 9, moving `next` (L-108), and restating #399's acceptance for `0.2.2`.
