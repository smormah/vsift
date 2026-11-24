'use strict';

// The one shared wait of the post-publish checks (#618): returns only when the public
// registry serves the tarballs of all four packages at a version, or fails after at most
// 15 minutes naming what is still missing.
//
//   node tools/p14-published/wait-for-registry.cjs --version <published version>
//
// Every job that packs or installs the published version runs this first (`P14 verify
// release`, `P14 published artifacts`, `P14 journeys`). It reads four public URLs with a
// HEAD request, sends no credential and nothing that names a person or a machine, and
// changes nothing. The decisions are in lib/registry-wait.cjs, which has the tests.

const fs = require('node:fs');
const { setTimeout: delay } = require('node:timers/promises');

const { QualificationError, parseArguments } = require('./lib/common.cjs');
const { headStatus, waitForRegistry } = require('./lib/registry-wait.cjs');

async function main() {
  const options = parseArguments(process.argv.slice(2), { required: ['version'] });
  const log = (line) => process.stdout.write(`${line}\n`);
  const { attempts, waitedMs } = await waitForRegistry({
    version: options.version,
    probe: (url) => headStatus(url),
    sleep: (milliseconds) => delay(milliseconds),
    now: () => Date.now(),
    log,
  });
  if (process.env.GITHUB_STEP_SUMMARY) {
    const served = attempts === 1 ? 'at the first request' : `after ${Math.round(waitedMs / 1000)} s (${attempts} attempts)`;
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### Registry\n\nnpm served all four tarballs of \`${options.version}\` ${served}.\n\n`);
  }
}

main().catch((error) => {
  process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
  process.exitCode = 1;
});
