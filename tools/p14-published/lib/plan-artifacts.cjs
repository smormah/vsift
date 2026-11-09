'use strict';

// The files an OLD published binary needs, fetched for it (L-254).
//
// A published VSift pins the exact address of every managed artifact and refuses any redirect it
// has not reviewed. When the FFmpeg builds repository moved to a new owner (2026-11-07) every
// published version that pins the old address began to refuse its own download, and a published
// binary cannot be changed. The two scenarios that use an old release only as a means (RQ-04's
// baseline and `P13 managed smoke`'s upgrade-from-a-release) therefore hand it the files instead:
// each file the baseline's own plan names is fetched from the plan's address, following the move
// (the redirect lands on the repository's new address), and checked against the size and SHA-256
// that PLAN carries before it is kept. The old binary then installs from the folder
// (`setup install --artifact-dir`), which applies its own size and SHA-256 check again.
//
// What this does not touch: the redirect policy of any VSift build. A test of "does the published
// binary download from the publishers" is a different question, and it stays red until a release
// pins the new address (`P13 managed smoke` with `published_version`, the weekly schedule).
//
// Requests carry only the neutral identifier of lib/common.cjs: no person, machine or credential.

const fs = require('node:fs');
const path = require('node:path');

const { expect } = require('./common.cjs');
const { artifactName, downloadVerified } = require('./download.cjs');

/**
 * Address prefixes that a repository move left redirecting, and that published versions still pin
 * (L-254, docs/planning/known-limits.md#l-254). Add a prefix here only after reading the move.
 */
const MOVED_PREFIXES = Object.freeze(['https://github.com/BtbN/FFmpeg-Builds/']);

/**
 * Whether any action of a `setup plan` names an address that moved, so that its binary cannot
 * download it by itself. A plan that names none needs nothing: its binary downloads as it always did.
 */
function plansAMovedAddress(actions) {
  expect(Array.isArray(actions), 'the plan has no actions');
  return actions.some((action) => MOVED_PREFIXES.some((prefix) => String(action.source_url).startsWith(prefix)));
}

/**
 * Fetches every file the plan names into `folder`, each one verified against the plan's size and
 * SHA-256 (and refused otherwise), and returns one note per file.
 */
async function fetchPlanArtifacts({ actions, folder, fetchImpl = fetch }) {
  expect(Array.isArray(actions) && actions.length > 0, 'the plan has no actions to fetch');
  fs.mkdirSync(folder, { recursive: true });
  const notes = [];
  for (const action of actions) {
    const name = artifactName(action.source_url);
    const { finalUrl } = await downloadVerified({
      url: action.source_url,
      bytes: action.bytes,
      sha256: action.sha256,
      destination: path.join(folder, name),
      fetchImpl,
    });
    notes.push(`${name} ${action.bytes} bytes via ${new URL(finalUrl).hostname}`);
  }
  return notes;
}

module.exports = { MOVED_PREFIXES, fetchPlanArtifacts, plansAMovedAddress };
