'use strict';

// Fetch the files a published binary's own plan names, when that plan names an address that moved
// (L-254), so that an old release can be the starting point of a scenario although it cannot
// download them itself. Used by the `P13 managed smoke` upgrade job; RQ-04's upgrade.cjs does the
// same inside its own run (lib/plan-artifacts.cjs explains the rule).
//
//   node tools/p14-published/fetch-plan-artifacts.cjs --binary <absolute path of the published vsift> \
//     --folder <empty folder to fill>
//
// Standard output is exactly one line, `VSIFT_E2E_ARTIFACT_DIR=<folder>`, when the plan names a moved
// address (the job appends it to GITHUB_ENV), and nothing when it does not. What happened goes to
// standard error. The binary runs `setup plan`, which only reads, in an empty temporary home.

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { QualificationError, describeStatus, expect, parseArguments, parseJson, run } = require('./lib/common.cjs');
const { fetchPlanArtifacts, plansAMovedAddress } = require('./lib/plan-artifacts.cjs');

async function main() {
  const options = parseArguments(process.argv.slice(2), { required: ['binary', 'folder'] });
  expect(path.isAbsolute(options.binary) && fs.existsSync(options.binary), `${options.binary} is not an existing absolute path`);
  const folder = path.resolve(options.folder);
  expect(!fs.existsSync(folder) || fs.readdirSync(folder).length === 0, `${folder} is not empty`);
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift plan '));
  try {
    const env = {
      PATH: '',
      HOME: home,
      XDG_CONFIG_HOME: path.join(home, 'config'),
      XDG_DATA_HOME: path.join(home, 'data'),
      XDG_CACHE_HOME: path.join(home, 'cache'),
      XDG_STATE_HOME: path.join(home, 'state'),
    };
    const result = run(options.binary, ['setup', 'plan', '--profile', 'desktop', '--json'], env, { cwd: home });
    const plan = parseJson(result);
    expect(result.status === 0 && plan.command === 'setup.plan', `${describeStatus(result)}: ${result.stdout.slice(0, 300)}`);
    const actions = plan.data.actions;
    if (!plansAMovedAddress(actions)) {
      process.stderr.write('the plan names no address that moved; nothing to fetch\n');
      return;
    }
    const notes = await fetchPlanArtifacts({ actions, folder });
    process.stderr.write(`${notes.join('\n')}\n`);
    process.stdout.write(`VSIFT_E2E_ARTIFACT_DIR=${folder}\n`);
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
}

main().catch((error) => {
  process.stderr.write(`${error instanceof QualificationError ? error.message : error.stack}\n`);
  process.exitCode = 1;
});
