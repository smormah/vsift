#!/usr/bin/env node
'use strict';

// P14 RQ-08: repeats one group of race and stress tests many times on one
// system, keeps the output of every failed or hung repetition, and states the
// result against the rule (no failed repetition, none hung, at least the
// repetitions asked). It runs cargo and the test binaries with explicit
// arguments, never a shell, and it writes only below --out.
//
//   node stress.cjs --suite locks --runs 200 --out <dir>
//        [--minutes 320] [--timeout-minutes 25] [--max-failures 10] [--commit <sha>]
//        [--until runs|budget]
//   node stress.cjs --suite repro --runs 1000 --out <dir>
//        --package <package> --target lib|<test binary> --filter <test path>
//        [--match exact|substring] [--ignored no|only|include] [--copies 1-8] [--burners 0-4]
//
// --until runs (the default) stops after --runs repetitions; --until budget keeps
// repeating until --minutes have passed, and --runs is then the least that must
// complete (the overnight lane on LOKI fills its night this way).
//
// The `repro` suite runs one named test (#439, #443): it builds the test binary
// once and then runs it directly, --copies of it at once, so the copies do not
// wait for each other on cargo's build lock.
//
// Never run it on a machine whose state matters: the suites start hundreds of
// processes, kill some of them and create and delete folders in the temporary
// directory (the workflows run it on hosted runners and on LOKI).

const childProcess = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {
  failedTests, testTotals, classify, tally, verdict, killTree, renderMarkdown,
} = require('./lib/repeat.cjs');
const repro = require('./lib/repro.cjs');

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
  'batch-277': {
    description: 'The opt-in batch test of #277: 40 rounds of 20 requests at concurrency 4 leave no initializing session',
    cargo: ['-p', 'vsift', '--test', 'engine_batch'],
    testArgs: ['--ignored', '--exact', 'batches_at_concurrency_four_leave_no_initializing_session'],
  },
};

/** The suite whose test is named by the caller. */
const REPRO = 'repro';

/** When a run stops: after --runs repetitions, or when --minutes have passed. */
const UNTIL = Object.freeze(['runs', 'budget']);

const KEPT_OUTPUT_BYTES = 4 * 1024 * 1024;
const KEPT_STDOUT_BYTES = 32 * 1024 * 1024;

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

/** Flags that take a value, and the option each sets. */
const FLAGS = {
  '--suite': 'suite',
  '--runs': 'runs',
  '--out': 'out',
  '--minutes': 'minutes',
  '--timeout-minutes': 'timeoutMinutes',
  '--max-failures': 'maxFailures',
  '--commit': 'commit',
  '--until': 'until',
  '--package': 'package',
  '--target': 'target',
  '--filter': 'filter',
  '--match': 'match',
  '--ignored': 'ignored',
  '--copies': 'copies',
  '--burners': 'burners',
};
const NUMERIC = new Set(['runs', 'minutes', 'timeoutMinutes', 'maxFailures', 'copies', 'burners']);
const REPRO_ONLY = ['package', 'target', 'filter', 'match', 'ignored', 'copies', 'burners'];

/**
 * Reads the command line. Every flag takes a value; an unknown flag, a missing
 * value or a repro option outside the repro suite is refused.
 * @param {string[]} argv
 */
function parse(argv) {
  const options = { minutes: 320, timeoutMinutes: 25, maxFailures: 10, commit: 'unknown', until: 'runs' };
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const key = FLAGS[flag];
    if (!key) throw new Error(`unknown option ${flag}`);
    const value = argv[index + 1];
    if (value === undefined) throw new Error(`${flag} needs a value`);
    options[key] = NUMERIC.has(key) ? Number(value) : value;
  }
  if (options.suite !== REPRO && !Object.hasOwn(SUITES, options.suite)) {
    throw new Error(`--suite must be one of ${[...Object.keys(SUITES), REPRO].join(', ')}`);
  }
  for (const key of ['runs', 'minutes', 'timeoutMinutes', 'maxFailures']) {
    if (!Number.isInteger(options[key]) || options[key] <= 0) throw new Error(`--${key} must be a positive integer`);
  }
  if (!UNTIL.includes(options.until)) throw new Error(`--until must be one of ${UNTIL.join(', ')}`);
  if (!options.out) throw new Error('--out is required');
  if (options.suite === REPRO) {
    options.repro = repro.validateRepro(options);
  } else {
    const stray = REPRO_ONLY.filter((key) => options[key] !== undefined);
    if (stray.length > 0) throw new Error(`only the repro suite takes ${stray.map((key) => `--${key}`).join(', ')}`);
  }
  for (const key of REPRO_ONLY) delete options[key];
  return options;
}

/**
 * Runs one process to the end or the timeout; keeps the tail of its combined
 * output, and its standard output apart (cargo's JSON messages).
 */
function runProcess(command, args, timeoutMs, cwd, extraEnv = {}) {
  return new Promise((resolve) => {
    const begun = Date.now();
    const child = childProcess.spawn(command, args, {
      cwd,
      detached: process.platform !== 'win32',
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, CARGO_TERM_COLOR: 'never', RUST_BACKTRACE: '1', ...extraEnv },
    });
    let output = '';
    let stdout = '';
    let timedOut = false;
    const keep = (chunk) => {
      output += chunk.toString('utf8');
      if (output.length > KEPT_OUTPUT_BYTES) output = output.slice(output.length - KEPT_OUTPUT_BYTES);
    };
    child.stdout.on('data', (chunk) => {
      keep(chunk);
      if (stdout.length < KEPT_STDOUT_BYTES) stdout += chunk.toString('utf8');
    });
    child.stderr.on('data', keep);
    const timer = setTimeout(() => {
      timedOut = true;
      killTree(child.pid);
    }, timeoutMs);
    const seconds = () => Math.round((Date.now() - begun) / 100) / 10;
    child.on('error', (error) => {
      clearTimeout(timer);
      resolve({ code: null, timedOut, output: `${output}\nspawn error: ${error.message}`, stdout, seconds: seconds() });
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      resolve({ code, timedOut, output, stdout, seconds: seconds() });
    });
  });
}

/**
 * Builds the suite once and returns how to run one repetition, or the failed
 * build's output.
 */
async function prepare(options, repoRoot) {
  const timeoutMs = options.timeoutMinutes * 60 * 1000;
  if (options.repro) {
    const build = await runProcess('cargo', repro.buildArguments(options.repro), 90 * 60 * 1000, repoRoot);
    if (build.code !== 0) return { failed: build };
    const binary = repro.testExecutable(build.stdout, options.repro);
    const manifestDir = binary.manifestPath ? path.dirname(binary.manifestPath) : repoRoot;
    const args = repro.testArguments(options.repro);
    return {
      description: repro.describe(options.repro),
      burners: options.repro.burners,
      copies: options.repro.copies,
      // cargo runs a test binary from its package's folder and tells it where that is.
      once: () => runProcess(binary.executable, args, timeoutMs, manifestDir, { CARGO_MANIFEST_DIR: manifestDir }),
    };
  }
  const suite = SUITES[options.suite];
  const build = await runProcess('cargo', ['test', '--locked', '--no-run', ...suite.cargo], 90 * 60 * 1000, repoRoot);
  if (build.code !== 0) return { failed: build };
  const testArgs = suite.testArgs.length ? ['--', ...suite.testArgs] : [];
  return {
    description: suite.description,
    burners: suite.burners || 0,
    copies: 1,
    once: () => runProcess('cargo', ['test', '--locked', '--no-fail-fast', ...suite.cargo, ...testArgs], timeoutMs, repoRoot),
  };
}

async function main() {
  const options = parse(process.argv.slice(2));
  const repoRoot = process.cwd();
  fs.mkdirSync(options.out, { recursive: true });
  const started = new Date();
  const version = childProcess.spawnSync('cargo', ['--version'], { encoding: 'utf8' });
  const cargoVersion = (version.stdout || 'cargo (unknown version)').trim();

  console.log(`${options.suite}: building the tests once`);
  const plan = await prepare(options, repoRoot);
  if (plan.failed) {
    process.stdout.write(plan.failed.output.split('\n').slice(-60).join('\n'));
    console.error('the tests did not build; no repetition ran');
    process.exitCode = 2;
    return;
  }
  console.log(`${options.suite}: ${plan.description}`);

  const stopBurners = plan.burners ? startBurners(plan.burners) : () => {};
  const repetitions = [];
  const deadline = Date.now() + options.minutes * 60 * 1000;
  const wanted = options.until === 'runs' ? options.runs : Number.POSITIVE_INFINITY;
  let testsPerRepetition = 0;
  let rep = 0;
  while (rep < wanted) {
    if (Date.now() >= deadline) {
      console.log(`the time budget of ${options.minutes} minutes ended the run after ${rep} repetitions`);
      break;
    }
    const batch = Math.min(plan.copies, wanted - rep);
    const runs = await Promise.all(Array.from({ length: batch }, () => plan.once()));
    for (const run of runs) {
      rep += 1;
      const outcome = classify(run);
      const tests = outcome === 'passed' ? [] : failedTests(run.output);
      const totals = testTotals(run.output);
      testsPerRepetition = Math.max(testsPerRepetition, totals.passed + totals.failed);
      repetitions.push({ rep, outcome, seconds: run.seconds, tests });
      if (outcome !== 'passed') {
        const file = path.join(options.out, `repetition-${String(rep).padStart(4, '0')}-${outcome}.log`);
        fs.writeFileSync(file, run.output);
        console.log(`repetition ${rep}: ${outcome.toUpperCase()} in ${run.seconds} s (${tests.join(', ') || 'no test named'}); output kept in ${path.basename(file)}`);
      } else if (rep === 1 || rep % 10 === 0) {
        console.log(`repetition ${rep}: passed in ${run.seconds} s`);
      }
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
    description: plan.description,
    host: {
      os: `${os.platform()} ${os.release()}`,
      cpus: os.cpus().length,
      memoryMiB: Math.round(os.totalmem() / (1024 * 1024)),
    },
    commit: options.commit,
    cargoVersion,
    burners: plan.burners ? plan.burners * os.cpus().length : 0,
    copies: plan.copies,
    until: options.until,
    test: options.repro || null,
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
  // A filter that names no test passes every run with nothing tested; for a
  // repro that is a mistake in the dispatch, never a rate.
  if (options.repro && totals.completed > 0 && testsPerRepetition === 0) {
    result.verdict = { ok: false, reasons: [...result.verdict.reasons, 'the filter matched no test'] };
  }
  fs.writeFileSync(path.join(options.out, 'result.json'), `${JSON.stringify(result, null, 2)}\n`);
  const markdown = renderMarkdown(result);
  fs.writeFileSync(path.join(options.out, 'summary.md'), markdown);
  process.stdout.write(`\n${markdown}\n`);
  process.exitCode = result.verdict.ok ? 0 : 1;
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 2;
  });
}

module.exports = { SUITES, REPRO, UNTIL, parse };
