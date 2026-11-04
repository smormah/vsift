# The stable release `0.2.4`: the maintainer's steps

Status: **prepared on 2026-11-04 by the pull request whose title starts "The stable release 0.2.4", into `release/0.2.x`. Not yet followed: `0.2.4` is not tagged or published.** This is the checklist for tagging, publishing and checking `0.2.4`, the hot fix of `0.2.3` and the third stable release made on a release branch. It is a planning page and not a section of [`release.md`](../operations/release.md) because `release.md` may not change between the last candidate and the stable release (6.8); it is folded into `release.md` after the publish, in the records change to `main`. **Follow [`release-0.2.3-stable-steps.md`](release-0.2.3-stable-steps.md) (the page for `0.2.3`, which is the model for this one, with [`release-0.2.2-stable-steps.md`](release-0.2.2-stable-steps.md) behind it and `release.md` 6.20), `release.md` 6.7, 6.8, 6.18 step 5 and 6.22, with the numbers in the table below and the command lines on this page, which are spelled out so that they can be pasted (Git Bash; in PowerShell write `"$t^^{commit}"` for `"$t^{commit}"`).** Everything the model page says that this page does not repeat is unchanged: who does what, the order of the steps, what the dry run's plan must list, the comparison of the two plans, the approval, the failure table and the list of what is irreversible. Nothing here is run by a session except the checks that read: **tagging, dispatching the release workflow, approving the deployment and every npm write are the maintainer's** (the coordinator may type the commands in the maintainer's terminal under the maintainer's login, as on 2026-10-18).

**Plain English first.** `0.2.4` is built from exactly the source of `0.2.4-rc.1`, the candidate published on 2026-11-04 (tag `v0.2.4-rc.1` at `d44d71532967bb512bc92d328995e1c9e6f40a05` on `release/0.2.x`): the stable commit differs from that tag only in its version numbers, the launcher's README, the installation guide and the work record, and a program checks that. What the release does is re-pin the managed FFmpeg that a new Ubuntu install gets (to BtbN's month-end build of 2026-10-31, `n9.0.2-39`); it is a hot fix, so no long runs were made before its publish (`release.md` 6.18) and the maintainer waived them for `0.2.4`. Publishing it moves npm's `latest` from `0.2.3` to `0.2.4` on all four packages; after that, `npm install vsift-cli` with no tag installs `0.2.4`. `next` is not moved (it stays `0.2.4-rc.1`; only the maintainer's own command moves it: [L-108](known-limits.md#l-108)). A published version can never be taken back. **The deprecations follow the maintainer's decision of 2026-10-25, which was never carried out for `0.2.3-rc.1`:** step 9 holds the commands, for the maintainer to confirm as they approve each npm write. Both candidates are deprecated: `0.2.4-rc.1` with `Superseded by 0.2.4. Install vsift-cli.`, and `0.2.3-rc.1` with the message decided then, `Superseded by 0.2.3. Install vsift-cli.` `0.2.3` is a superseded stable release and is not deprecated, as `0.2.2` and `0.2.1` were not.

**The runbook's record for the candidate.** `release.md` 6.22 was followed on 2026-11-04: the cut (#575) and the nightly dispatched on the branch at its commit (run 41816481791), the dry run (41818685278), the publish (41822398268, first attempt; the maintainer approved), `next` moved from `0.2.3-rc.1` to `0.2.4-rc.1` and `latest` stayed `0.2.3`. Its evidence is recorded in [`release-0.2.4.md`](release-0.2.4.md), "The evidence on 0.2.4-rc.1" (#577), and the ledger is complete for it.

## The numbers that change (read the model page with this table)

| | `0.2.3` (the model page, 2026-10-25) | `0.2.4` (this page) |
| --- | --- | --- |
| The version, the tag, the commit | `0.2.3`, `v0.2.3`, the merge commit of "The stable release 0.2.3" (#528, `fda35b68a960`) | `0.2.4`, `v0.2.4` (annotated, message `VSift 0.2.4`), **the merge commit on `release/0.2.x` of the pull request whose title starts "The stable release 0.2.4"** (`<stable commit>`, written down at step 1 item 3) |
| Where the commit and the tag are | `release/0.2.x`, made again at the cut of `0.2.3-rc.1` | `release/0.2.x`, **the same branch, not made again** (a hot fix is made on the branch of the version it fixes, 6.18); the `guard` job refuses a tag its branch does not contain; a stable version needs no nightly of its own |
| The accepted candidate | `v0.2.3-rc.1` at `09e40398894f` | **`v0.2.4-rc.1` at `d44d71532967bb512bc92d328995e1c9e6f40a05`** (the only, so the highest, `v0.2.4-rc.N` tag; the check ignores every tag of another `X.Y.Z`) |
| What npm holds before | `latest` `0.2.2`, `next` `0.2.3-rc.1` | **`latest` `0.2.3`, `next` `0.2.4-rc.1`**, on all four packages (read 2026-11-04, below) |
| What the plan does | `latest` from `0.2.2` to `0.2.3`; `next` stays | **`latest` from `0.2.3` to `0.2.4`; `next` stays `0.2.4-rc.1`** |
| The `latest moves forward` guard | `latest` is `0.2.2` on all four | `latest` is **`0.2.3`**, a stable version below `0.2.4`, and `0.2.4` is on none of them |
| The evidence ledger guard | complete for `0.2.3-rc.1` at `09e40398894f` | complete for **`0.2.4-rc.1` at `d44d71532967`** (passes since #577; read again on 2026-11-04) |
| The candidate-to-stable file counts | 5 version-string, 2 shipped-document and 11 work-record files | 5 version-string, 2 shipped-document and **11 work-record files** (at the head of the pull request, with the records of #577 in the count: the changelog, the two memory files, ADR 0024, the guide's first page, the backlog, the register, the ledger, the dated scan reading, the plan page and this page; more if a work-record pull request merges before the tag) |
| The version-line count of the by-hand check (step 1 item 6) | `58` | **`58`** (29 changed lines, in pairs; the workspace has the same seven crates) |
| The before-snapshot of the earlier versions | 53 integrity values and 12 tags | **every published version: `61` integrity values and 14 tags** (below; the 53 and the eight new ones of `0.2.3` and `0.2.4-rc.1`) |
| What `vsift --version` prints | `vsift 0.2.3 (<12 digits>)` | `vsift 0.2.4 (<the first 12 digits of the stable commit>)` |
| The GitHub release | becomes GitHub's latest, from `v0.2.2` | becomes GitHub's latest, **from `v0.2.3`** to `v0.2.4` |
| The deprecations at the stable | only `0.2.3-rc.1` (decided 2026-10-25, **not carried out**) | **both candidates (step 9):** `0.2.4-rc.1` ("Superseded by 0.2.4. Install vsift-cli.") and `0.2.3-rc.1` ("Superseded by 0.2.3. Install vsift-cli.", the decision of 2026-10-25); the maintainer confirms them as they approve each npm write |
| The hosted checks after the publish | on the tag, except `P14 journeys` on `main`; `P14 verify release` within seven days | **the same routes, with `P14 scan reading` on the tag as well** (step 10; `P14 malicious media` and `P07 local ASR` are not repeated: a hot fix makes no long runs before its publish, and the stable's source is the candidate's) |

## What the release ships untried (read before you decide to publish)

Nine of the twenty evidence items are `waived`, so not passes, for `0.2.4`, by the maintainer's decisions of 2026-11-04 ([`release-0.2.4.md`](release-0.2.4.md), "The maintainer's decisions of 2026-11-04"), and `release-evidence` cannot see the limits of a waiver ([L-179](known-limits.md#l-179)): **RQ-07** (the fuzzing hour, although the two raised archive bounds sit in the code it fuzzes; the scheduled `Fuzz` on `main` on Monday 9 November covers the same bounds, #573), **RQ-08** (stress; the re-pin does not touch it; #519 stays a known issue, [L-231](known-limits.md#l-231)), **RQ-09** (load), **RQ-11** (power loss and the durability campaign), **RQ-12** (the runbook walk), **RQ-15 and RQ-16** (the agent-trial batches: a hot fix runs none and the skill is unchanged), and the standing waivers **RQ-14** (the strict worker, until #188 in R1) and **RQ-17** (no Smart App Control, clean-machine or Mac try-out; [L-143](known-limits.md#l-143)). RQ-04's `P14 compatibility` and `P14 local upgrade` were waived as dispatches on the tag, inside an item that is `passed` on runs of all its three parts. Each waiver names `0.2.4` and nothing later; the next cut sets each item back to `planned`. **What the release does not fix:** [#351](https://github.com/smormah/vsift/issues/351) (an MP4 with very many `pssh` boxes, CVE-2026-107678; VSift's 15-second time bound ends it, [L-144](known-limits.md#l-144)), [#272](https://github.com/smormah/vsift/issues/272), and five FFmpeg records of 2026-11-01 that the pinned build does not carry ([L-227](known-limits.md#l-227)). **The re-pin is for Ubuntu only:** the Windows catalogue is `main`'s alone (#573). The changelog, the launcher's README and `install.md` say the same things in the words a user reads. If any of this makes you want to wait, stop at step 1: nothing is irreversible before step 4.

## 0. Before you merge the pull request

1. **The records of the candidate are merged** (#577, `4af1fb488ebe089946ee3483ec6195f761e59f66`), so the plan's evidence guard is complete for `0.2.4-rc.1`; this pull request is cut from that commit or a later one on `release/0.2.x`.
2. **Merge nothing else into `release/0.2.x`** between this pull request and the publish except a work-record pull request (`CHANGELOG.md`, `memory/`, `docs/decisions/`, `docs/history/`, `docs/planning/` and `docs/guide/` pages, not `docs/guide/reference/`, `docs/guide/files/` or `docs/planning/delivery-ledger.json`): **a page of `docs/agents/` or `docs/operations/`, `SECURITY.md` or the root README is refused** (6.8). `main` is not frozen and keeps taking merges (it carries `0.3.0` work); nothing of it reaches the branch.
3. **The window this opens is the branch's** ([L-133](known-limits.md#l-133)): on merge the installation guide and the launcher's README on `release/0.2.x` say that `npm install vsift-cli` installs `0.2.4`, which is true only after step 6; `main`'s copies keep their wording until the records change after the publish. Merge when you can tag and publish within the day.

## 1. Preflight (about 30 minutes; only items 1 and 2 need a browser)

Items 1, 2, 7, 10, 11 and 12 are the model page's, unchanged (trusted publishers saved on all four packages, fork-pull-request approval, the local gates, the `release` environment, the tag ruleset and the release-branch ruleset (6.2 step 8), reading what ships, the choice about `next`). The ones that change:

3. **Find the stable commit**, the merge commit of the pull request whose title starts "The stable release 0.2.4". Write its 40 digits down; `<stable commit>` below is that:

   ```console
   gh pr view <number> --repo smormah/vsift --json state,baseRefName,title,mergeCommit --jq "{state, baseRefName, title, commit: .mergeCommit.oid}"
   ```

   Expect `MERGED` and `release/0.2.x`. Then `git fetch origin --tags` and `git show <stable commit>:Cargo.toml | grep -m1 "^version"` prints `version = "0.2.4"`.
4. **On `release/0.2.x`, every check passed:** `git merge-base --is-ancestor <stable commit> origin/release/0.2.x && echo "on the release branch"` prints `on the release branch`, `git merge-base --is-ancestor v0.2.4-rc.1 <stable commit> && echo "after the candidate"` prints `after the candidate`, and

   ```console
   gh api "repos/smormah/vsift/commits/<stable commit>/check-runs?per_page=100" --paginate --jq '.check_runs[] | select(.status != "completed" or (.conclusion | IN("success", "skipped", "neutral") | not)) | "\(.name): \(.status) \(.conclusion)"'
   ```

   prints nothing (the push to `release/0.2.x` runs `CI`; wait for its `CI result`).
5. **The candidate is the one you mean:** `git tag --list "v0.2.4-rc.*"` prints `v0.2.4-rc.1` only, and `git tag --list "v0.2.4"` prints nothing.
6. **The delta check, locally, at the stable commit** (the check is compiled from the commit it judges, so check by hand too):

   ```console
   git switch --detach <stable commit>
   cargo run --locked -p vsift-release -- candidate-delta
   ```

   Every file is `version string only`, `shipped document` or `work record`, nothing is `REFUSED`, and it ends "The differences are limited to version strings, the launcher's README, the installation guide and the work record." By hand:

   ```console
   git diff --stat v0.2.4-rc.1 <stable commit> -- crates tools .github skills schemas fixtures fuzz npm Cargo.toml Cargo.lock rust-toolchain.toml deny.toml
   ```

   It must list **only** `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, `npm/vsift-cli/package.json` and `npm/vsift-cli/README.md`. **Any line under `crates/`, `tools/`, `.github/`, `skills/`, `schemas/`, `fixtures/` or any other file of `npm/` means the check was bypassed or wrong: do not publish.** The version files may differ in the version text only:

   ```console
   git diff -U0 v0.2.4-rc.1 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep -c "^[-+][^-+]"
   git diff -U0 v0.2.4-rc.1 <stable commit> -- Cargo.toml Cargo.lock fuzz/Cargo.toml fuzz/Cargo.lock npm/vsift-cli/package.json | grep "^[-+][^-+]" | sed 's/^[-+]//; s/0\.2\.4-rc\.1/0.2.4/' | sort | uniq -c | awk '$1 % 2 == 1'
   git diff --name-only v0.2.4-rc.1 <stable commit> -- docs/planning/delivery-ledger.json docs/guide/reference docs/guide/files
   git diff --name-only v0.2.4-rc.1 <stable commit> | grep -v -e "^CHANGELOG.md$" -e "^memory/" -e "^docs/decisions/" -e "^docs/history/" -e "^docs/planning/" -e "^docs/guide/"
   ```

   Expect `58`, then nothing, then nothing, then exactly these seven lines: `Cargo.lock`, `Cargo.toml`, `docs/operations/install.md`, `fuzz/Cargo.lock`, `fuzz/Cargo.toml`, `npm/vsift-cli/README.md`, `npm/vsift-cli/package.json`. Then `git switch -` (back to your branch).
7. (The model page's, unchanged.)
8. **The evidence ledger is complete for the candidate** (the plan's guard 6; run it yourself first):

   ```console
   cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.4-rc.1 --commit d44d71532967bb512bc92d328995e1c9e6f40a05
   ```

   Expect "VSift release evidence is complete for 0.2.4-rc.1 at d44d71532967." and exit status 0. It says the ledger's records are complete, not that the evidence is good enough to ship: the nine waived items above are the part to read. (`--complete-for 0.2.4` fails until the records change after the publish copies the run's `release-delta.json` into the ledger and records the runs of step 10: [L-103](known-limits.md#l-103).)
9. **What npm holds now**, from a shell with no npm login (an empty `NPM_CONFIG_USERCONFIG` and `NPM_CONFIG_GLOBALCONFIG`, two different empty files): `npm view vsift-cli dist-tags` and the same for `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64` print **`{ next: '0.2.4-rc.1', latest: '0.2.3' }` four times** (anything else means something was published or moved since 2026-11-04: stop and tell the coordinator), and `npm view vsift-cli@0.2.4 version` answers 404. Then write down every published version's bytes, so that step 7 can show they are untouched, and the tags' commits:

   ```console
   for v in 0.0.0-stage 0.0.0 0.1.0 0.2.0-rc.1 0.2.0-rc.2 0.2.0-rc.3 0.2.0 0.2.1-rc.1 0.2.1-rc.2 0.2.1-rc.3 0.2.1 0.2.2-rc.1 0.2.2 0.2.3-rc.1 0.2.3 0.2.4-rc.1; do for p in vsift-cli @vsift/win32-x64 @vsift/darwin-arm64 @vsift/linux-x64; do i="$(npm view "$p@$v" dist.integrity 2>/dev/null)"; [ -n "$i" ] && echo "$p@$v $i"; done; done
   for t in v0.1.0 v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.3 v0.2.0 v0.2.1-rc.1 v0.2.1-rc.2 v0.2.1-rc.3 v0.2.1 v0.2.2-rc.1 v0.2.2 v0.2.3-rc.1 v0.2.3 v0.2.4-rc.1; do git show --no-patch --format="$t %H" "$t^{commit}"; done
   ```

   The first prints `61` lines (`0.0.0-stage` exists for `@vsift/darwin-arm64` only; the loop takes a few minutes); the 53 values of `0.0.0-stage` to `0.2.3-rc.1` are those of [`release-0.2.3-stable-steps.md`](release-0.2.3-stable-steps.md) (45 and eight), and the eight new ones were read on 2026-11-04:

   ```text
   vsift-cli@0.2.3 sha512-CzBcW3UrrEXy1xAgZz0b3t3fA1X76CZntdFCPNZsgpIof/iqC98ZpYNf42zdI0fotk/kb2sF3/UYqo9VZYd7Rw==
@vsift/win32-x64@0.2.3 sha512-wWhiWGgezIyQHMc2X+uFGowCjtHKoVekwAepNQLkbKQxmGx0K4/pYB/SduK01vfaElrJ3PDoa7yV4jqLYwyCQQ==
@vsift/darwin-arm64@0.2.3 sha512-wzJPXzYuxYn+nK2j4rLeOAlavYmo+ZaCyGbNS+Hjnd0jTpBM/OZYbNYt9nlCtYNa402rSgjtirGrSeM19LqgJw==
@vsift/linux-x64@0.2.3 sha512-So6qA7s/IHWoL1UeH39n8ivoQJLrMxkQVrHs8QuNMRkIwudOihdWNTj4r9o+WHF+oWK17P0Uog5AFW1pB/APRg==
vsift-cli@0.2.4-rc.1 sha512-SFVbtP9LPPsnuKUjl5BXiXY0sYSaNgK6VuL6zXLIZHT0qJWqARSgMDwBP7+AzCzIKIVeni8ZxpkcsxRdPZy+6Q==
@vsift/win32-x64@0.2.4-rc.1 sha512-Z93WtnpEddDonlPzU9KttOhR24Qny3RhtETuBfgy99posdrB5hUprRu6yfhE5PLsTJ3MoYtGiwOZLaya2fLN8Q==
@vsift/darwin-arm64@0.2.4-rc.1 sha512-1spIWiOAQifL3PFysk5XqbmkrcFQYf7r4zN2hgMEJ43iKFjoLFIFYl2kG2QQPYfa5oorXyBUl7DCejUMKP0i/g==
@vsift/linux-x64@0.2.4-rc.1 sha512-AZVMjnwUI9LCk4X4fbZBNXtTElnG3oeuUAtlbhF53bV36vgLNyO80qiFms+xzKC63NSAJQqdUl4dbjYY+tFcOA==
   ```

   The second prints the fourteen annotated tags' commits: `011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa` (`v0.1.0`), those of `v0.2.0-rc.1` to `v0.2.1` (6.19), `43a033dbf81346b967b293a150e13306828cac50` (`v0.2.2-rc.1`), `0dadc69df4f5c5c3264be72f04be26a600620fe1` (`v0.2.2`), `09e40398894fddebe8aa94cf3d871eab5b8ef7ec` (`v0.2.3-rc.1`), `fda35b68a9608042c380cfe055d76323bd4ef954` (`v0.2.3`) and `d44d71532967bb512bc92d328995e1c9e6f40a05` (`v0.2.4-rc.1`; the tag's own object is `038b15456ffad41e30309832f0209206a33f7295`, not listed).

   **The hot fix's one extra read:** `gh api repos/smormah/vsift/releases/latest --jq .tag_name` prints `v0.2.3`.

If any item is not as stated, do not tag; tell the coordinator what you saw.

## 2. Tag the stable commit (the maintainer's own git identity as tagger)

```console
git tag -a v0.2.4 -m "VSift 0.2.4" <stable commit>
git push origin v0.2.4
git show --no-patch --format="%H" "v0.2.4^{commit}"
git cat-file -t v0.2.4
git fetch origin && git merge-base --is-ancestor <stable commit> origin/release/0.2.x && echo "on the release branch"
```

The third command prints the 40 digits you wrote down and the fourth `tag`. A wrong tag **before anything is published** may be moved (`git push origin :refs/tags/v0.2.4`, fix, tag again; the `guard` and the publish's re-check require the tag to name the run's commit); **never after step 4 has started.**

## 3 to 6. Dry run, publish, read the plan, approve

As the model page's steps 3 to 6, with `v0.2.4` and these differences.

**Dry run:**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.4 -f dry_run=true
gh run list --repo smormah/vsift --workflow release.yml --limit 3 --json databaseId,headBranch,headSha,event,status,conclusion,url
```

Pick the run whose `headBranch` is `v0.2.4` and whose `headSha` is the commit you wrote down; write its id down: `<dry-run id>`. The `guard` job answers within a minute (the tag of the workspace version, the tag naming the commit, the commit on `release/0.2.x` with the five rule types; no nightly is asked of a stable version). The **Publish plan** summary must show "Version `0.2.4` (stable), npm dist-tag `latest`, Git tag `v0.2.4`"; the table "What this publication does to the dist-tags" with, **for each of the four packages, `latest` from `0.2.3` to `0.2.4` and `next` staying `0.2.4-rc.1`**; six guards, **all `passed`** (Stable version; Accepted candidate "against `v0.2.4-rc.1` (commit `d44d71532967`): 5 version-string, 2 shipped-document and 11 work-record files differ, nothing else"; Registry read; Candidate published ("`0.2.4-rc.1` is published on all four packages"); `latest` moves forward ("`latest` is `0.2.3` on all four packages, a stable version below `0.2.4`, and this version is on none of them"); Evidence ledger ("VSift release evidence is complete for 0.2.4-rc.1 at d44d71532967.")); the section "The evidence ledger's `release_delta` record" with `"verdict": "allowed"`, `"candidate_version": "0.2.4-rc.1"` and `"stable_version": "0.2.4"`; the four `npm publish ... --tag latest --provenance --ignore-scripts` commands in the order `@vsift/darwin-arm64`, `@vsift/win32-x64`, `@vsift/linux-x64`, `vsift-cli`; and the GitHub release commands (draft, then published as **latest**), with ten assets. Download the plan and read the notes:

```console
gh run download <dry-run id> --repo smormah/vsift --name publish-plan --dir plan-dry
```

**Publish:**

```console
gh workflow run release.yml --repo smormah/vsift --ref v0.2.4 -f dry_run=false
gh run list --repo smormah/vsift --workflow release.yml --limit 3 --json databaseId,headBranch,headSha,event,status,conclusion,url
gh run download <publish id> --repo smormah/vsift --name publish-plan --dir plan-publish
diff plan-dry/attestation-subjects.sha256 plan-publish/attestation-subjects.sha256 && diff plan-dry/release-assets.sha256 plan-publish/release-assets.sha256 && echo "the same bytes as the dry run"
```

**Write the publish run's id down: it is the run npm's provenance names.** Its plan must say `Publish plan: PUBLISH a STABLE release after the release environment's approval` and "Afterwards `npm install vsift-cli` installs `0.2.4`." **Keep `plan-publish/release-delta.json`** (the artifact expires seven days after the run; the records change copies it into the ledger). Then **Review deployments**, tick `release`, **Approve and deploy** (the maintainer; the coordinator never approves). After the approval the `publish` job asks the `guard`'s questions again (a moved tag or a branch that lost the commit or a rule refuses it), publishes the four packages with `--tag latest`, reads `latest` back every 30 seconds for up to 20 minutes with `next` unchanged (a wrong answer fails at once), creates the GitHub release as a draft and publishes it as latest, and requires GitHub's own latest release to be `v0.2.4`. If the read-back runs out of time with the packages published, **Re-run failed jobs** on the same run finishes it (6.5), approved again.

The runs of the candidate for comparison (read 2026-11-04): dry run 41818685278 and publish 41822398268; the publish read `next` back about three minutes after its approval.

## 7. Check it from the outside (a shell with no npm login and a folder of your own; about ten minutes)

`npm view vsift-cli dist-tags` (and the three platform packages) prints **`{ next: '0.2.4-rc.1', latest: '0.2.4' }` four times.** The step 1 item 9 loop prints the same `61` values and the same fourteen tag commits (and the new tag `v0.2.4`, which is not in the loop). Install it without a tag:

```console
mkdir stable-check && cd stable-check && echo '{"private": true}' > package.json
npm install vsift-cli
npm audit signatures
npx vsift --version
gh release view v0.2.4 --repo smormah/vsift --json isPrerelease,isDraft,assets --jq "{isPrerelease, isDraft, assets: (.assets | length)}"
gh api repos/smormah/vsift/releases/latest --jq .tag_name
```

Expect `vsift 0.2.4 (<the first 12 digits of the stable commit>)`, verified registry signatures and attestations, `isPrerelease` false, `isDraft` false and ten assets, and `v0.2.4` (if not: `gh release edit v0.2.4 --repo smormah/vsift --latest`). The installed skill equals the tag's (`diff -r skills/vsift stable-check/node_modules/vsift-cli/skills/vsift` from a checkout of the tag prints nothing). The checksums and attestations are `release.md` 6.4's commands with `v0.2.4`, `--source-ref refs/tags/v0.2.4`, `--signer-workflow smormah/vsift/.github/workflows/release.yml` and `--deny-self-hosted-runners`. **Tell the coordinator** the two run ids, the four `npm view` answers, the output of `npm audit signatures`, the two `gh` answers and anything that failed or was re-run. Nothing is announced.

## 8. If something fails

The model page's table applies with these numbers. Three differ: "Candidate published" refused means `0.2.4-rc.1` is not on all four packages; "`latest` moves forward" refused means `latest` is not `0.2.3` or `0.2.4` is already on npm with other bytes; "Evidence ledger" refused means the ledger at the stable commit is not complete for `0.2.4-rc.1` at `d44d71532967`. The `guard` refusing the dispatch names its reason (the wrong ref, a tag off `release/0.2.x`, a branch without its rule types). **A published version that is bad is never unpublished:** `npm deprecate` it on all four packages, edit the release's notes, and publish `0.2.5` after its own candidate (`release.md` 6.5; the branch is the line's, so the candidate is made on it); until then `latest` names the bad version, and pointing `latest` back at `0.2.3` (`npm dist-tag add <package>@0.2.3 latest` for each package, two-factor authentication) is safe and your choice at the time.

## 9. Deprecations (both candidates; the maintainer confirms them as they approve each write)

**The ruling.** The maintainer decided on 2026-10-25 that `0.2.3-rc.1` is deprecated at the stable, with the message `Superseded by 0.2.3. Install vsift-cli.` ([`release-0.2.3-stable-steps.md`](release-0.2.3-stable-steps.md), step 9); it was never carried out (read on 2026-11-04: `npm view vsift-cli@0.2.3-rc.1 deprecated` prints nothing; `0.2.2-rc.1` prints "Superseded by 0.2.2. Install vsift-cli."). So the default is **both**: `0.2.4-rc.1` with `Superseded by 0.2.4. Install vsift-cli.` and `0.2.3-rc.1` with `Superseded by 0.2.3. Install vsift-cli.` **A superseded stable release (`0.2.3`) is not deprecated.**

> The maintainer's confirmation: `<both as written / changes>` (given to the coordinator on `<date>`).

The shipped documents say the same: the launcher's README and `install.md` name `0.2.3-rc.1` and `0.2.4-rc.1` as deprecated once `0.2.4` is published.

After step 7 has passed. The maintainer logs in and approves each npm write with the second factor; a session may type the commands in the maintainer's own terminal under that login, and nothing else runs them (`npm whoami` first: a lapsed login is found here otherwise). **Eight writes, four for `0.2.4-rc.1` and four for `0.2.3-rc.1`:**

```console
npm deprecate "vsift-cli@0.2.4-rc.1" "Superseded by 0.2.4. Install vsift-cli."
npm deprecate "@vsift/win32-x64@0.2.4-rc.1" "Superseded by 0.2.4. Install vsift-cli."
npm deprecate "@vsift/darwin-arm64@0.2.4-rc.1" "Superseded by 0.2.4. Install vsift-cli."
npm deprecate "@vsift/linux-x64@0.2.4-rc.1" "Superseded by 0.2.4. Install vsift-cli."
```

and the four of `0.2.3-rc.1`, with the message decided on 2026-10-25:

```console
npm deprecate "vsift-cli@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
npm deprecate "@vsift/win32-x64@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
npm deprecate "@vsift/darwin-arm64@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
npm deprecate "@vsift/linux-x64@0.2.3-rc.1" "Superseded by 0.2.3. Install vsift-cli."
```

Read back with `npm view "<package>@<version>" deprecated` for each package and version (it prints the message); `0.2.4`, `0.2.3` and every other version print what they printed before (empty, or their earlier message). An empty message removes a deprecation. The tags do not move, and `next` still names `0.2.4-rc.1`, now with npm's deprecation warning (the state `0.2.2-rc.1` has been in since 2026-10-18, [L-108](known-limits.md#l-108)).

## 10. The hosted checks of the published bytes (none of them can publish)

Each is one dispatch; runs of one workflow on one ref share a concurrency group and **a newer waiting run replaces an older waiting one without a word**, so dispatch the second `P14 published artifacts` after the first has finished. A failed run is a finding: open an issue before re-running (governance rule 14). Dispatched on the tag `v0.2.4`:

```console
gh workflow run p14-verify-release.yml --repo smormah/vsift --ref v0.2.4 -f version=0.2.4
gh workflow run p13-managed-smoke.yml --repo smormah/vsift --ref v0.2.4 -f published_version=0.2.4
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.4 -f version=0.2.4 -f from_version=0.2.3
gh workflow run p14-published-artifacts.yml --repo smormah/vsift --ref v0.2.4 -f version=0.2.4 -f from_version=0.2.4-rc.1
gh workflow run p14-scan-reading.yml --repo smormah/vsift --ref v0.2.4 -f version=0.2.4
```

(`P13 managed smoke`'s upgrade job starts from its default, `0.2.2`, as for the candidate: `0.2.3` pins the same FFmpeg build as `0.2.2`, so the supersede notice and the move to `n9.0.2-39` are the same, and the run stays comparable with the candidate's.)

**`P14 journeys` is dispatched on `main`, not on the tag,** because the tag's test driver is the candidate's and runs the noisy-speech set on hosted macOS, which does not fit that runner (#520, #576, L-230):

```console
gh workflow run p14-journeys.yml --repo smormah/vsift --ref main -f version=0.2.4 -f tests_from=workflow-ref
```

This is the route the maintainer decided on 2026-11-04 for the candidate (run 41837876046: the published binary under test, the test driver from `main`, all three systems passed, the noisy-speech set skipped on macOS and gated on Ubuntu and Windows by `P07 local ASR`); **using it for the stable's own bytes is this page's proposal, for the maintainer to confirm, as it was for `0.2.3`.** `main` has moved on since that run (it carries `0.3.0` work), so a failure in a checkpoint that `0.3.0` work changed is a finding of the test driver and not of `0.2.4`; say so in the issue, and the fallback is the dispatch on the tag with `-f asr_gates=false` (no macOS accuracy gate at all, which is weaker). **Expect it all green**, `P14 verify release` with `stable: candidate-to-stable-delta` and `stable: latest-on-all-four-packages` among the passes and **within seven days of the publish** (its delta check reads `release-delta.json` from the publish run's artifact).

**`P14 malicious media` and `P07 local ASR` are not repeated for the stable:** RQ-10 is not a repeat item, and RQ-05's stable evidence is the `P14 journeys` run above, as for `0.2.3`. **`P14 scan reading` is dispatched on the tag, as `0.2.3` did** (RQ-13's stable rule is `repeat`, and its producer asks for a reading again before the stable); see the list above. Its short dated reading compares the run with the accepted reading of 2026-11-04 and names anything new.

## 11. After the publish: the records change to `main` (6.18 step 5)

**Within seven days of the publish,** in a pull request **to `main`** that may touch any page (6.8 binds the release branch only until the publish):

1. **Copy `plan-publish/release-delta.json` into the ledger as `release_delta`** (it names `0.2.3` today; nothing copies it for you: [L-103](known-limits.md#l-103)), and give the items whose stable rule is `repeat` (RQ-01 to RQ-06, RQ-13, RQ-18, RQ-19) their own entries for `0.2.4` from the runs of step 10, the Governance job at the stable commit and a dated reading of RQ-13. The items waived for `0.2.4` (RQ-07, RQ-08, RQ-09, RQ-11, RQ-12, RQ-15, RQ-16) and the carried waivers (RQ-14, RQ-17) keep their texts. `release-evidence --complete-for 0.2.4 --commit <stable commit>` must then pass.
2. **Bring the release branch's work record and the stable commit's shipped documents to `main`** (the ledger and the plan page of `0.2.3` and `0.2.4`, the changelog, the register, `docs/operations/install.md`, `npm/vsift-cli/README.md`); the `Release window` workflow runs on that pull request by itself (it triggers on the ledger) and must find every window commit's lines on `main`.
3. **Fold this page into `release.md`** (the next number after 6.22) and **mark 6.22 a record** (followed on 2026-11-04: nightly 41816481791, dry run 41818685278, publish 41822398268; `next` moved from `0.2.3-rc.1` to `0.2.4-rc.1`), with what the real services showed for `0.2.4`.
4. **Pages the release branch could not change:** the root `README.md`, `SECURITY.md` (the supported-versions table: `0.2.4` is the newest patch), `docs/agents/skill.md` and `docs/agents/trials.md`, `docs/development.md`.
5. **The maintainer's choices:** the deprecations of step 9, moving `next` (L-108), and the next window's long runs (the waived items, #351's re-check at the November month-end build for `0.3.0`).
