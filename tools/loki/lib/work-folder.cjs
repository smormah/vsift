'use strict';

// The overnight job's own folders on LOKI (ADR 0025) and how they are cleaned.
//
// LOKI's runner is persistent, so what a job leaves stays for the next one and
// for the maintainer. Every folder the tests make in the temporary directory is
// put inside one folder of the job's own (`prepare` points TEMP, TMP and TMPDIR at
// it), and `clean` removes exactly that folder and the checkout's Cargo target
// folder, each only after it is positively identified as VSift's (AGENTS.md: the
// clean-up is restricted to positively identified VSift-owned folders). Nothing
// else is ever deleted: not the runner's folders, not the user's profile, never a
// path a variable could point elsewhere without the checks failing.

const fs = require('node:fs');
const path = require('node:path');

/** The name of the job's temporary folder: this prefix and the run's id. */
const PREFIX = 'vsift-loki-';

/** The marker that identifies the folder as this run's. */
const MARKER = '.vsift-loki-owned';

/** Names the tests' own folders carry, which a clean run leaves none of (#423). */
const LEFTOVER = /^(vsift|guide-|scratch-)/;

class WorkFolderError extends Error {
  constructor(message) {
    super(message);
    this.name = 'WorkFolderError';
  }
}

function runIdOf(text) {
  if (!/^[1-9][0-9]{0,19}$/.test(String(text))) throw new WorkFolderError('the run id must be a positive integer');
  return String(text);
}

/**
 * The job's temporary folder: a direct child of the runner's temporary folder.
 * @param {string} root the runner's temporary folder (RUNNER_TEMP)
 * @param {string} runId
 */
function temporaryFolder(root, runId) {
  if (!path.isAbsolute(root)) throw new WorkFolderError('the runner temporary folder must be absolute');
  return path.join(root, `${PREFIX}${runIdOf(runId)}`);
}

/** Whether `candidate` is an existing real folder, not a link or junction. */
function isRealDirectory(candidate, fileSystem = fs) {
  try {
    return fileSystem.lstatSync(candidate).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Makes the job's temporary folder with its marker and returns the lines that
 * point the job's temporary directory at it (for $GITHUB_ENV).
 * @param {string} root
 * @param {string} runId
 */
function prepare(root, runId, fileSystem = fs) {
  const folder = temporaryFolder(root, runId);
  if (!isRealDirectory(root, fileSystem)) throw new WorkFolderError('the runner temporary folder is not a real folder');
  if (fileSystem.existsSync(folder)) {
    // A re-run of the same run may find its own folder; anything else is refused.
    if (!isOwned(folder, runId, fileSystem)) throw new WorkFolderError(`${folder} exists and is not this run's`);
  } else {
    fileSystem.mkdirSync(folder);
    fileSystem.writeFileSync(path.join(folder, MARKER), `${JSON.stringify({ run: runIdOf(runId) })}\n`);
  }
  return ['TEMP', 'TMP', 'TMPDIR'].map((name) => `${name}=${folder}`);
}

function isOwned(folder, runId, fileSystem = fs) {
  if (!isRealDirectory(folder, fileSystem)) return false;
  try {
    const marker = JSON.parse(fileSystem.readFileSync(path.join(folder, MARKER), 'utf8'));
    return marker && marker.run === runIdOf(runId);
  } catch {
    return false;
  }
}

/**
 * Whether `target` is the checkout's Cargo target folder: a real folder named
 * `target` directly inside a checkout of this repository (its Cargo.toml and
 * rust-toolchain.toml beside it), holding the CACHEDIR.TAG Cargo writes.
 */
function isCargoTarget(workspace, fileSystem = fs) {
  if (!path.isAbsolute(workspace)) return false;
  const target = path.join(workspace, 'target');
  return isRealDirectory(target, fileSystem)
    && fileSystem.existsSync(path.join(workspace, 'Cargo.toml'))
    && fileSystem.existsSync(path.join(workspace, 'rust-toolchain.toml'))
    && fileSystem.existsSync(path.join(workspace, 'tools', 'loki', 'lib', 'work-folder.cjs'))
    && fileSystem.existsSync(path.join(target, 'CACHEDIR.TAG'));
}

/**
 * Removes the job's temporary folder and the checkout's target folder, each only
 * when it is positively identified, and reports what the tests left behind.
 * @returns {{ removed: string[], skipped: string[], leftovers: string[] }}
 */
function clean(root, runId, workspace, fileSystem = fs) {
  const report = { removed: [], skipped: [], leftovers: [] };
  const folder = temporaryFolder(root, runId);
  if (isOwned(folder, runId, fileSystem)) {
    report.leftovers = fileSystem.readdirSync(folder).filter((name) => LEFTOVER.test(name)).sort();
    fileSystem.rmSync(folder, { recursive: true, maxRetries: 5, retryDelay: 1000 });
    report.removed.push(folder);
  } else {
    report.skipped.push(`${folder}: not this run's folder (missing, a link, or no marker)`);
  }
  if (isCargoTarget(workspace, fileSystem)) {
    const target = path.join(workspace, 'target');
    fileSystem.rmSync(target, { recursive: true, maxRetries: 5, retryDelay: 1000 });
    report.removed.push(target);
  } else {
    report.skipped.push(`${path.join(String(workspace), 'target')}: not this checkout's Cargo target folder`);
  }
  return report;
}

module.exports = {
  PREFIX, MARKER, WorkFolderError, temporaryFolder, prepare, clean, isOwned, isCargoTarget,
};
