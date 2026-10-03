'use strict';

// The pieces of the repetition runner (P14 RQ-08): what a failed cargo test
// run says, how the repetitions add up, and how a hung run is stopped. The
// runner itself is stress.cjs; everything here is pure or takes its process
// calls as arguments, so it is tested without running cargo.

const childProcess = require('node:child_process');

/**
 * The names of the tests a libtest run reports as failed, in order, without
 * repeats. Reads the "test <name> ... FAILED" lines (which every run prints)
 * and the "failures:" list.
 * @param {string} output
 * @returns {string[]}
 */
function failedTests(output) {
  const names = [];
  const add = (name) => {
    if (!names.includes(name)) names.push(name);
  };
  let listing = false;
  for (const raw of String(output).split(/\r?\n/)) {
    const line = raw.replace(/\x1b\[[0-9;]*m/g, '');
    const failed = /^test (\S+) \.\.\. FAILED\b/.exec(line);
    if (failed) add(failed[1]);
    // After a "failures:" line libtest lists the names, indented four spaces,
    // up to a blank line; the first "failures:" opens the output blocks.
    if (line === 'failures:') {
      listing = true;
      continue;
    }
    if (listing) {
      const named = /^ {4}(\S+)$/.exec(line);
      if (named) add(named[1]);
      else if (line.trim() === '') listing = false;
    }
  }
  return names;
}

/**
 * How many tests a cargo run says it ran, from the "test result:" lines of
 * every binary it executed.
 * @param {string} output
 * @returns {{ passed: number, failed: number, ignored: number }}
 */
function testTotals(output) {
  const totals = { passed: 0, failed: 0, ignored: 0 };
  for (const raw of String(output).split(/\r?\n/)) {
    const match = /^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored/.exec(raw.trim());
    if (match) {
      totals.passed += Number(match[1]);
      totals.failed += Number(match[2]);
      totals.ignored += Number(match[3]);
    }
  }
  return totals;
}

/**
 * Classifies one repetition from its exit status and output.
 * @param {{ code: number|null, timedOut: boolean, output: string }} run
 * @returns {'passed'|'failed'|'timed_out'|'build_failed'}
 */
function classify(run) {
  if (run.timedOut) return 'timed_out';
  if (run.code === 0) return 'passed';
  if (/^error(\[E\d+\])?: could not compile|^error: failed to run custom build|^error: linking with/m.test(run.output)) {
    return 'build_failed';
  }
  return 'failed';
}

/**
 * Adds the repetitions up.
 * @param {{ rep: number, outcome: string, seconds: number, tests: string[] }[]} repetitions
 */
function tally(repetitions) {
  const seconds = repetitions.map((rep) => rep.seconds).sort((a, b) => a - b);
  const perTest = {};
  for (const rep of repetitions) {
    for (const name of rep.tests) perTest[name] = (perTest[name] || 0) + 1;
  }
  const count = (outcome) => repetitions.filter((rep) => rep.outcome === outcome).length;
  return {
    completed: repetitions.length,
    passed: count('passed'),
    failed: count('failed'),
    timedOut: count('timed_out'),
    buildFailed: count('build_failed'),
    perTest,
    secondsMin: seconds.length ? seconds[0] : 0,
    secondsMedian: seconds.length ? seconds[Math.floor(seconds.length / 2)] : 0,
    secondsMax: seconds.length ? seconds[seconds.length - 1] : 0,
    secondsTotal: Math.round(seconds.reduce((sum, value) => sum + value, 0)),
  };
}

/**
 * The success rule of RQ-08 for one suite on one system: at least the asked
 * repetitions, none failed, none hung (a hung run is a deadlock candidate, and
 * is reported as one).
 * @param {ReturnType<typeof tally>} totals
 * @param {number} asked
 * @returns {{ ok: boolean, reasons: string[] }}
 */
function verdict(totals, asked) {
  const reasons = [];
  if (totals.failed > 0) reasons.push(`${totals.failed} repetition(s) failed`);
  if (totals.timedOut > 0) reasons.push(`${totals.timedOut} repetition(s) did not finish (a deadlock candidate)`);
  if (totals.buildFailed > 0) reasons.push('the tests did not build');
  if (totals.completed < asked) reasons.push(`only ${totals.completed} of ${asked} repetitions ran (the time budget ended the run)`);
  return { ok: reasons.length === 0, reasons };
}

/**
 * Stops a process and everything it started. A repetition that hangs holds
 * cargo, the test binaries and their children, and leaving any of them would
 * poison the next repetition.
 * @param {number} pid
 * @param {{ platform?: string, spawnSync?: typeof childProcess.spawnSync, kill?: (pid: number, signal: string) => void }} [deps]
 */
function killTree(pid, deps = {}) {
  const platform = deps.platform || process.platform;
  const run = deps.spawnSync || childProcess.spawnSync;
  const kill = deps.kill || process.kill;
  if (platform === 'win32') {
    run('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
    return;
  }
  try {
    kill(-pid, 'SIGKILL');
  } catch {
    try {
      kill(pid, 'SIGKILL');
    } catch {
      // Already gone.
    }
  }
}

/**
 * Markdown for the job summary.
 * @param {object} result the runner's result record
 */
function renderMarkdown(result) {
  const t = result.totals;
  const lines = [
    `### ${result.suite} on ${result.host.os}: ${result.verdict.ok ? 'passed' : 'FAILED'}`,
    '',
    `| Repetitions asked | completed | passed | failed | hung | total time | per repetition (min / median / max) |`,
    `| --- | --- | --- | --- | --- | --- | --- |`,
    `| ${result.runsAsked} | ${t.completed} | ${t.passed} | ${t.failed} | ${t.timedOut} | ${t.secondsTotal} s | ${t.secondsMin} / ${t.secondsMedian} / ${t.secondsMax} s |`,
    '',
  ];
  if (result.verdict.reasons.length > 0) {
    lines.push(`Reasons: ${result.verdict.reasons.join('; ')}.`, '');
  }
  const names = Object.keys(t.perTest).sort();
  if (names.length > 0) {
    lines.push('| Failing test | repetitions it failed |', '| --- | --- |');
    for (const name of names) lines.push(`| \`${name}\` | ${t.perTest[name]} |`);
    lines.push('');
  }
  const load = result.burners ? ` ${result.burners} busy processes kept every CPU occupied.` : '';
  lines.push(`Tests per repetition: ${result.testsPerRepetition}.${load} Commit ${result.commit}. ${result.cargoVersion}.`, '');
  return lines.join('\n');
}

module.exports = { failedTests, testTotals, classify, tally, verdict, killTree, renderMarkdown };
