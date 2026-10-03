#!/usr/bin/env node
'use strict';

// P14 RQ-08: repeats one group of race and stress tests many times on one
// system, keeps the output of every failed or hung repetition, and states the
// result against the rule (no failed repetition, none hung, at least the
// repetitions asked). It runs cargo test with explicit arguments, never a
// shell, and it writes only below --out.
//
//   node stress.cjs --suite locks --runs 200 --out <dir>
//        [--minutes 320] [--timeout-minutes 25] [--max-failures 10] [--commit <sha>]
//
// Never run it on a machine whose state matters: the suites start hundreds of
// processes, kill some of them and create and delete folders in the temporary
// directory (the workflow runs it on hosted runners).

const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {
  failedTests, testTotals, classify, tally, verdict, killTree, renderMarkdown,
} = require('./lib/repeat.cjs');

/** The suites: what each repeats, and the issue it watches. */
const SUITES = {
  locks: {
    description: 'Lock and lifecycle tests alongside process-spawning tests (issue #66)',
    cargo: ['-p', 'vsift-infrastructure', '--lib', '--test', 'p05_lifecycle'],
    testArgs: [],
  },
  admission: {
    description: 'Weighted admission and the cross-process coordination locks',
    cargo: ['-p', 'vsift-infrastructure', '--test', 'weighted_admission', '--test', 'storage_coordination'],
    testArgs: [],
  },
  supervisor: {
    description: 'The process supervisor (issue #128)',
    cargo: ['-p', 'vsift-infrastructure', '--test', 'process_supervisor'],
    testArgs: [],
  },
  roots: {
    description: 'Concurrent creation of one session root (issue #206)',
    cargo: ['-p', 'vsift-infrastructure', '--test', 'session_root_provisioning'],
    testArgs: [],
  },
  'supervisor-loaded': {
    description: 'The process supervisor (issue #128) while every CPU is kept busy: both failures were seen in a full workspace run, with other tests competing',
    cargo: ['-p', 'vsift-infrastructure', '--test', 'process_supervisor'],
    testArgs: [],
    burners: 2,
  },
  'roots-loaded': {
    description: 'Concurrent creation of one session root (issue #206) while every CPU is kept busy',
    cargo: ['-p', 'vsift-infrastructure', '--test', 'session_root_provisioning'],
    testArgs: [],
    burners: 2,
  },
  engine: {
    description: 'The engine worker, batch, jobs and lifecycle',
    cargo: [
      '-p', 'vsift', '--test', 'engine_worker', '--test', 'engine_batch', '--test', 'engine_jobs',
      '--test', 'engine_lifecycle',
    ],
    testArgs: [],
  },
  delivery: {
    description: 'The external-delivery simulation: workers killed at random, requests redelivered and duplicated',
    cargo: ['-p', 'vsift-cli', '--test', 'external_delivery_stress'],
    testArgs: ['--ignored', '--nocapture'],
  },
};

const KEPT_OUTPUT_BYTES = 4 * 1024 * 1024;

/**
 * Keeps the CPUs busy while a suite repeats: `perCpu` busy processes for each CPU,
 * stopped when the returned function is called. Both intermittent failures this
 * campaign watches (#128, #206) appeared in a full workspace test run, where
 * other test binaries competed for the machine.
 */
function startBurners(perCpu) {
  const children = [];
  for (let index = 0; index < perCpu * os.cpus().length; index += 1) {
    children.push(childProcess.spawn(process.execPath, ['-e', 'for (;;) {}'], { stdio: 'ignore' }));
  }
  return () => {
    for (const child of children) child.kill('SIGKILL');
  };
}

function parse(argv) {
  const options = { minutes: 320, timeoutMinutes: 25, maxFailures: 10, commit: 'unknown' };
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    switch (flag) {
      case '--suite': options.suite = value; break;
      case '--runs': options.runs = Number(value); break;
      case '--out': options.out = value; break;
      case '--minutes': options.minutes = Number(value); break;
      case '--timeout-minutes': options.timeoutMinutes = Number(value); break;
      case '--max-failures': options.maxFailures = Number(value); break;
      case '--commit': options.commit = value; break;
      default: throw new Error(`unknown option ${flag}`);
    }
  }
  if (!SUITES[options.suite]) throw new Error(`--suite must be one of ${Object.keys(SUITES).join(', ')}`);
  for (const key of ['runs', 'minutes', 'timeoutMinutes', 'maxFailures']) {
    if (!Number.isInteger(options[key]) || options[key] <= 0) throw new Error(`--${key} must be a positive integer`);
  }
  if (!options.out) throw new Error('--out is required');
  return options;
}

function cargo(args, options) {
  return childProcess.spawn('cargo', args, {
    cwd: options.cwd,
    detached: process.platform !== 'win32',
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, CARGO_TERM_COLOR: 'never', RUST_BACKTRACE: '1' },
  });
}

/** Runs cargo to the end or the timeout; keeps the tail of its output. */
function runCargo(args, timeoutMs, cwd) {
  return new Promise((resolve) => {
    const child = cargo(args, { cwd });
    let output = '';
    let timedOut = false;
    const keep = (chunk) => {
      output += chunk.toString('utf8');
      if (output.length > KEPT_OUTPUT_BYTES) output = output.slice(output.length - KEPT_OUTPUT_BYTES);
    };
    child.stdout.on('data', keep);
    child.stderr.on('data', keep);
    const timer = setTimeout(() => {
      timedOut = true;
      killTree(child.pid);
    }, timeoutMs);
    child.on('error', (error) => {
      clearTimeout(timer);
      resolve({ code: null, timedOut, output: `${output}\nspawn error: ${error.message}` });
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      resolve({ code, timedOut, output });
    });
  });
}

async function main() {
  const options = parse(process.argv.slice(2));
  const suite = SUITES[options.suite];
  const repoRoot = process.cwd();
  fs.mkdirSync(options.out, { recursive: true });
  const started = new Date();
  const version = childProcess.spawnSync('cargo', ['--version'], { encoding: 'utf8' });
  const cargoVersion = (version.stdout || 'cargo (unknown version)').trim();

  console.log(`${options.suite}: ${suite.description}`);
  console.log(`building the tests once (${suite.cargo.join(' ')})`);
  const build = await runCargo(['test', '--locked', '--no-run', ...suite.cargo], 90 * 60 * 1000, repoRoot);
  if (build.code !== 0) {
    process.stdout.write(build.output.split('\n').slice(-60).join('\n'));
    console.error('the tests did not build; no repetition ran');
    process.exitCode = 2;
    return;
  }

  const stopBurners = suite.burners ? startBurners(suite.burners) : () => {};
  const repetitions = [];
  const deadline = Date.now() + options.minutes * 60 * 1000;
  let testsPerRepetition = 0;
  for (let rep = 1; rep <= options.runs; rep += 1) {
    if (Date.now() >= deadline) {
      console.log(`the time budget of ${options.minutes} minutes ended the run before repetition ${rep}`);
      break;
    }
    const begun = Date.now();
    const run = await runCargo(
      ['test', '--locked', '--no-fail-fast', ...suite.cargo, ...(suite.testArgs.length ? ['--', ...suite.testArgs] : [])],
      options.timeoutMinutes * 60 * 1000,
      repoRoot,
    );
    const seconds = Math.round((Date.now() - begun) / 100) / 10;
    const outcome = classify(run);
    const tests = outcome === 'passed' ? [] : failedTests(run.output);
    const totals = testTotals(run.output);
    testsPerRepetition = Math.max(testsPerRepetition, totals.passed + totals.failed);
    repetitions.push({ rep, outcome, seconds, tests });
    if (outcome !== 'passed') {
      const file = path.join(options.out, `repetition-${String(rep).padStart(4, '0')}-${outcome}.log`);
      fs.writeFileSync(file, run.output);
      console.log(`repetition ${rep}/${options.runs}: ${outcome.toUpperCase()} in ${seconds} s (${tests.join(', ') || 'no test named'}); output kept in ${path.basename(file)}`);
    } else if (rep === 1 || rep % 10 === 0) {
      console.log(`repetition ${rep}/${options.runs}: passed in ${seconds} s`);
    }
    if (repetitions.filter((entry) => entry.outcome !== 'passed').length >= options.maxFailures) {
      console.log(`stopped after ${options.maxFailures} failed repetitions`);
      break;
    }
  }

  stopBurners();
  const totals = tally(repetitions);
  const result = {
    suite: options.suite,
    description: suite.description,
    host: {
      os: `${os.platform()} ${os.release()}`,
      cpus: os.cpus().length,
      memoryMiB: Math.round(os.totalmem() / (1024 * 1024)),
    },
    commit: options.commit,
    cargoVersion,
    burners: suite.burners ? suite.burners * os.cpus().length : 0,
    runsAsked: options.runs,
    minutesBudget: options.minutes,
    timeoutMinutes: options.timeoutMinutes,
    testsPerRepetition,
    started: started.toISOString(),
    finished: new Date().toISOString(),
    totals,
    repetitions,
    verdict: verdict(totals, options.runs),
  };
  fs.writeFileSync(path.join(options.out, 'result.json'), `${JSON.stringify(result, null, 2)}\n`);
  const markdown = renderMarkdown(result);
  fs.writeFileSync(path.join(options.out, 'summary.md'), markdown);
  process.stdout.write(`\n${markdown}\n`);
  process.exitCode = result.verdict.ok ? 0 : 1;
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 2;
});
