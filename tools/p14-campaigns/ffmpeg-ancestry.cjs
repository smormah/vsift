#!/usr/bin/env node
'use strict';

// Usage: node ffmpeg-ancestry.cjs --records <nvd.json> --snapshot <name>=<commit> [--snapshot ...] [--out <file>]
//
// P14 RQ-13, the FFmpeg row of the scan reading (docs/planning/p14-scan-
// reading-2026-10-02.md, addendum of 2026-10-04): for every public vulnerability record that names a
// fix commit of FFmpeg, says whether that fix is in each snapshot (a BtbN build is named by the
// upstream commit in its version string). It reads one file of National Vulnerability Database
// records, saved by the packet owner, and GitHub's commit and compare endpoints through
// `gh api`; it writes nothing but its result. Commit hashes are the only thing it sends.
//
// This is a reading of public records and of public history, not a test of a binary: nothing here runs
// FFmpeg or a reproducer.
//
// Why more than ancestry. A fix is in a snapshot when its commit is an ancestor of the snapshot's
// commit. FFmpeg also takes a fix into a release branch as a cherry-pick: a new commit with another
// hash whose message ends "(cherry picked from commit <master hash>)". The first reading of the
// reviewed snapshot (2026-10-02) used the ancestry test alone and counted all 17 such fixes as missing;
// all 17 were in it. So each fix is one of:
//   ancestor     the fix hash is an ancestor of the snapshot;
//   cherry-pick  a commit reachable from the snapshot but not from master names the fix in its
//                "cherry picked from" line. The two patches are then compared (the changed files and
//                the hunks, line numbers dropped): `patch` is "identical" or "differs"; a pick that
//                "differs" is a partial or adapted backport and must be read by hand;
//   reverted     the cherry-pick is named by a later "This reverts commit" line on the branch;
//   absent       none of the above. Not a finding by itself: a fix a person wrote by hand without a
//                trailer is "absent" here (a false negative), so read the record and the branch.
// The test can also be wrong the other way: a record that names the commit that introduced a bug
// instead of its fix (some do) is "ancestor" for the wrong reason, and a trailer is a statement, not
// proof; the patch comparison is what makes a cherry-pick evidence. A record that names no commit is
// "unreferenced" and has to be read by hand (its pull request or issue, the history of the file it names).

const fs = require('node:fs');
const { execFileSync } = require('node:child_process');

const REPOSITORY = 'FFmpeg/FFmpeg';
// The project's code host, its GitHub mirror and its gitweb all name a commit as ".../commit/<hex>".
const COMMIT_URL = /^https:\/\/(?:code\.ffmpeg\.org\/FFmpeg\/FFmpeg|github\.com\/FFmpeg\/FFmpeg|git\.ffmpeg\.org\/gitweb\/ffmpeg\.git)\/commit\/([0-9a-f]{7,40})(?![0-9a-f])/;
const CHERRY_PICK = /\(cherry picked from commit ([0-9a-f]{7,40})\)/g;
const REVERT = /This reverts commit ([0-9a-f]{7,40})/g;

/** The commit hashes (7 to 40 hex digits) a record's references name as commits of FFmpeg. */
function fixCommits(cve) {
  const found = [];
  for (const reference of cve.references || []) {
    const match = COMMIT_URL.exec(reference.url || '');
    if (match && !found.includes(match[1])) found.push(match[1]);
  }
  return found;
}

/** The commits a message says it was cherry-picked from. */
function cherryPicks(message) {
  return [...String(message).matchAll(CHERRY_PICK)].map((match) => match[1]);
}

/** The commits a message says it reverts. */
function reverts(message) {
  return [...String(message).matchAll(REVERT)].map((match) => match[1]);
}

/** Two hashes are the same commit when the shorter (at least 7 digits) is a prefix of the longer. */
function sameCommit(left, right) {
  const [short, long] = left.length <= right.length ? [left, right] : [right, left];
  return short.length >= 7 && long.startsWith(short);
}

/** Whether compare `<fix>...<snapshot>` says the fix is an ancestor of the snapshot. */
function isAncestor(compareStatus) {
  return compareStatus === 'ahead' || compareStatus === 'identical';
}

/**
 * "ancestor", "cherry-pick" (with the commit), "reverted" or "absent". `branchOnly` is the list from
 * `branchOnlyCommits`, in the order GitHub returns it (oldest first).
 */
function classify(fix, compareStatus, branchOnly) {
  if (isAncestor(compareStatus)) return { state: 'ancestor' };
  const pickIndex = branchOnly.findIndex((commit) => commit.picks.some((from) => sameCommit(from, fix)));
  if (pickIndex < 0) return { state: 'absent' };
  const pick = branchOnly[pickIndex];
  const reverter = branchOnly.slice(pickIndex + 1).find((commit) => commit.reverts.some((from) => sameCommit(from, pick.sha)));
  if (reverter) return { state: 'reverted', commit: pick.sha, date: pick.date, revertedBy: reverter.sha };
  return { state: 'cherry-pick', commit: pick.sha, date: pick.date };
}

/**
 * The changes of a commit as text that is equal for a fix and an unchanged cherry-pick of it: the files
 * in name order, each patch with its hunk header's line numbers dropped (the function name after the
 * header stays). A commit with no patch text (a binary file, a very large change) yields null.
 */
function normalisedPatch(commit) {
  const files = [...(commit.files || [])].sort((left, right) => left.filename.localeCompare(right.filename));
  if (files.length === 0 || files.some((file) => typeof file.patch !== 'string')) return null;
  return files.map((file) => `${file.filename}\n${file.patch.replace(/^@@[^@]*@@/gm, '@@')}`).join('\n');
}

/** "identical", "differs" or "unknown" (a patch could not be read). */
function comparePatches(fix, pick) {
  const left = normalisedPatch(fix);
  const right = normalisedPatch(pick);
  if (left === null || right === null) return 'unknown';
  return left === right ? 'identical' : 'differs';
}

/** `gh api <path>` as parsed JSON; the one place that starts a process (no shell, fixed executable). */
function ghApi(path) {
  const output = execFileSync('gh', ['api', path], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  return JSON.parse(output);
}

/** The commits reachable from `snapshot` but not from master, with the commits each picks or reverts. */
function branchOnlyCommits(api, snapshot) {
  const commits = [];
  for (let page = 1; ; page += 1) {
    const data = api(`repos/${REPOSITORY}/compare/master...${snapshot}?per_page=100&page=${page}`);
    for (const commit of data.commits) {
      commits.push({
        sha: commit.sha,
        date: commit.commit.committer.date.slice(0, 10),
        picks: cherryPicks(commit.commit.message),
        reverts: reverts(commit.commit.message),
      });
    }
    if (data.commits.length < 100) return commits;
  }
}

/** Reads every record of a saved NVD response or of an array of records. */
function loadRecords(file) {
  const parsed = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (Array.isArray(parsed)) return parsed.map((entry) => entry.cve || entry);
  if (parsed.vulnerabilities) return parsed.vulnerabilities.map((entry) => entry.cve);
  return Object.values(parsed).map((entry) => entry.cve || entry);
}

/** The result for every record against every snapshot. `api` is injected so a test needs no network. */
function readRecords(records, snapshots, api) {
  const branches = Object.fromEntries(Object.entries(snapshots).map(([name, commit]) => [name, branchOnlyCommits(api, commit)]));
  return records.map((cve) => {
    const fixes = fixCommits(cve).map((hash) => {
      let commit;
      try {
        commit = api(`repos/${REPOSITORY}/commits/${hash}`);
      } catch {
        // A hash the GitHub mirror does not hold (a pull request's head commit, rewritten when it was
        // merged): reported, not guessed at; the record's pull request names the merge commit.
        return { hash, onMirror: false, snapshots: {} };
      }
      const perSnapshot = {};
      for (const [name, snapshot] of Object.entries(snapshots)) {
        const compare = api(`repos/${REPOSITORY}/compare/${commit.sha}...${snapshot}?per_page=1`);
        const result = classify(commit.sha, compare.status, branches[name]);
        if (result.state === 'cherry-pick') result.patch = comparePatches(commit, api(`repos/${REPOSITORY}/commits/${result.commit}`));
        perSnapshot[name] = result;
      }
      return { hash: commit.sha, onMirror: true, date: commit.commit.committer.date.slice(0, 10), snapshots: perSnapshot };
    });
    return { id: cve.id, published: String(cve.published).slice(0, 10), fixes, unreferenced: fixes.length === 0 };
  });
}

function describe(result) {
  return result.state === 'cherry-pick' ? `cherry-pick(patch=${result.patch})` : result.state;
}

function summary(results, names) {
  return results.map((record) => {
    if (record.unreferenced) return `${record.id} unreferenced (read it by hand)`;
    return `${record.id} ${record.fixes.map((fix) => (fix.onMirror ? `${fix.hash.slice(0, 10)} ${names.map((name) => `${name}=${describe(fix.snapshots[name])}`).join(' ')}` : `${fix.hash.slice(0, 10)} not on the mirror`)).join('; ')}`;
  });
}

function main(argv) {
  const options = { snapshot: [] };
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index].replace(/^--/, '');
    if (key === 'snapshot') options.snapshot.push(argv[index + 1]);
    else options[key] = argv[index + 1];
  }
  const snapshots = {};
  for (const entry of options.snapshot) {
    const [name, commit] = entry.split('=');
    if (!name || !/^[0-9a-f]{7,40}$/.test(commit || '')) {
      process.stderr.write('a --snapshot is <name>=<commit hash>\n');
      return 2;
    }
    snapshots[name] = commit;
  }
  if (!options.records || Object.keys(snapshots).length === 0) {
    process.stderr.write('usage: ffmpeg-ancestry.cjs --records <nvd.json> --snapshot <name>=<commit> [--snapshot ...] [--out <file>]\n');
    return 2;
  }
  const results = readRecords(loadRecords(options.records), snapshots, ghApi);
  if (options.out) fs.writeFileSync(options.out, `${JSON.stringify({ snapshots, results }, null, 2)}\n`);
  process.stdout.write(`${summary(results, Object.keys(snapshots)).join('\n')}\n`);
  return 0;
}

if (require.main === module) process.exitCode = main(process.argv.slice(2));

module.exports = { fixCommits, cherryPicks, reverts, sameCommit, isAncestor, classify, normalisedPatch, comparePatches, branchOnlyCommits, loadRecords, readRecords, summary };
