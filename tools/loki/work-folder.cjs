#!/usr/bin/env node
'use strict';

// The overnight job's folders on LOKI (lib/work-folder.cjs says what and why).
//
//   node tools/loki/work-folder.cjs prepare
//       makes $RUNNER_TEMP/vsift-loki-<run id> and points TEMP, TMP and TMPDIR at
//       it for the job's later steps (through $GITHUB_ENV).
//   node tools/loki/work-folder.cjs clean
//       removes that folder and $GITHUB_WORKSPACE/target, each only when it is
//       positively identified, and names what the tests left in the temporary
//       folder as a warning.

const fs = require('node:fs');
const work = require('./lib/work-folder.cjs');

function required(name) {
  const value = process.env[name];
  if (!value) throw new work.WorkFolderError(`${name} is not set`);
  return value;
}

function main(command) {
  const root = required('RUNNER_TEMP');
  const runId = required('GITHUB_RUN_ID');
  if (command === 'prepare') {
    const lines = work.prepare(root, runId);
    fs.appendFileSync(required('GITHUB_ENV'), `${lines.join('\n')}\n`);
    console.log(`The job's temporary folder is ${work.temporaryFolder(root, runId)}.`);
    return;
  }
  if (command === 'clean') {
    const report = work.clean(root, runId, required('GITHUB_WORKSPACE'));
    for (const removed of report.removed) console.log(`removed ${removed}`);
    for (const skipped of report.skipped) console.log(`left alone ${skipped}`);
    if (report.leftovers.length > 0) {
      console.log(`::warning::The tests left ${report.leftovers.length} folders in the temporary folder (#423): ${report.leftovers.join(', ')}`);
    }
    return;
  }
  throw new work.WorkFolderError('usage: work-folder.cjs prepare | clean');
}

try {
  if (process.argv.length !== 3) throw new work.WorkFolderError('usage: work-folder.cjs prepare | clean');
  main(process.argv[2]);
} catch (error) {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
}
