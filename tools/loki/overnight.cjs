#!/usr/bin/env node
'use strict';

// The overnight lane on LOKI (ADR 0025; docs/operations/loki-runner.md).
//
//   node tools/loki/overnight.cjs plan
//       reads EVENT, SCHEDULE, SUITES, MINUTES and the REPRO_* inputs from the
//       environment and writes `run` and `plan` to $GITHUB_OUTPUT (the hosted
//       planning job; nothing runs on LOKI when `run` is false).
//   node tools/loki/overnight.cjs run --out <folder> --commit <sha>
//       checks the plan in $PLAN again and runs each suite in turn with the
//       repetition runner, writing <folder>/<suite>/ and <folder>/summary.md;
//       exits 1 if any suite failed.
//
// It runs node and the runner with explicit arguments, never a shell. The suites
// load the machine for hours: run it only on LOKI or a disposable machine.

const childProcess = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const overnight = require('./lib/overnight.cjs');

const REPOSITORY = path.resolve(__dirname, '..', '..');

function options(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!['--out', '--commit'].includes(flag) || value === undefined) throw new overnight.PlanError(`unknown option ${flag}`);
    parsed[flag.slice(2)] = value;
  }
  if (!parsed.out || !path.isAbsolute(parsed.out)) throw new overnight.PlanError('--out must be an absolute folder');
  if (!/^[0-9a-f]{40}$/.test(parsed.commit || '')) throw new overnight.PlanError('--commit must be a full commit SHA');
  return parsed;
}

function writePlan() {
  const decided = overnight.plan(process.env, new Date());
  console.log(decided.reason);
  for (const suite of decided.suites) console.log(`- ${suite.name}${suite.minutes ? `: ${suite.minutes} min` : ''}`);
  const output = process.env.GITHUB_OUTPUT;
  if (!output) throw new overnight.PlanError('GITHUB_OUTPUT is not set');
  fs.appendFileSync(output, `run=${decided.run}\nplan=${JSON.stringify(decided.suites)}\n`);
}

function runSuite(args) {
  return new Promise((resolve) => {
    const child = childProcess.spawn(process.execPath, args, { cwd: REPOSITORY, stdio: 'inherit' });
    child.on('error', (error) => {
      console.error(`could not start the runner: ${error.message}`);
      resolve(2);
    });
    child.on('close', (code) => resolve(code === null ? 2 : code));
  });
}

async function run(argv) {
  const where = { ...options(argv), repository: REPOSITORY };
  let suites;
  try {
    suites = overnight.validatePlan(JSON.parse(process.env.PLAN || 'null'));
  } catch (error) {
    throw new overnight.PlanError(`the plan cannot be run: ${error.message}`);
  }
  fs.mkdirSync(where.out, { recursive: true });
  const results = [];
  for (const suite of suites) {
    if (suite.slot) {
      console.log(`::warning::${suite.name}: not run; the real-media campaign is not built yet (#363)`);
      results.push({ name: suite.name, outcome: 'not_run' });
      continue;
    }
    console.log(`::group::${suite.name} (${suite.minutes} minutes)`);
    const code = await runSuite(overnight.stressArguments(suite, where));
    console.log('::endgroup::');
    const summaryFile = path.join(where.out, suite.name, 'summary.md');
    results.push({
      name: suite.name,
      outcome: code === 0 ? 'passed' : 'failed',
      minutes: suite.minutes,
      summary: fs.existsSync(summaryFile) ? fs.readFileSync(summaryFile, 'utf8') : `${suite.name}: no summary (exit ${code})`,
    });
  }
  fs.writeFileSync(path.join(where.out, 'overnight.json'), `${JSON.stringify({ commit: where.commit, results: results.map(({ summary, ...rest }) => rest) }, null, 2)}\n`);
  const markdown = overnight.renderSummary(results, where.commit);
  fs.writeFileSync(path.join(where.out, 'summary.md'), markdown);
  process.stdout.write(markdown);
  return results.some((result) => result.outcome === 'failed') ? 1 : 0;
}

async function main(argv) {
  const [command, ...rest] = argv;
  if (command === 'plan' && rest.length === 0) {
    writePlan();
    return 0;
  }
  if (command === 'run') return run(rest);
  throw new overnight.PlanError('usage: overnight.cjs plan | run --out <folder> --commit <sha>');
}

main(process.argv.slice(2)).then(
  (code) => {
    process.exitCode = code;
  },
  (error) => {
    console.error(`::error::${error.message}`);
    process.exitCode = 2;
  },
);
