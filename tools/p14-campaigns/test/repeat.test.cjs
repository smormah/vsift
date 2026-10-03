'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const {
  failedTests, testTotals, classify, tally, verdict, killTree, renderMarkdown,
} = require('../lib/repeat.cjs');

const FAILING = `running 3 tests
test lock::first ... ok
test lock::second ... FAILED
test lock::third ... ok

failures:

---- lock::second stdout ----
thread 'lock::second' panicked at src/lib.rs:1:1:
boom

failures:
    lock::second

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s

error: test failed, to rerun pass \`-p vsift-infrastructure --lib\`
`;

test('the failed tests of a run are named once each, in order', () => {
  assert.deepEqual(failedTests(FAILING), ['lock::second']);
  assert.deepEqual(failedTests('test a ... FAILED\ntest b ... FAILED\ntest a ... FAILED\n'), ['a', 'b']);
  assert.deepEqual(failedTests('test result: ok. 1 passed; 0 failed; 0 ignored\n'), []);
  assert.deepEqual(failedTests('failures:\n    x::y\n    z\n\nnext\n'), ['x::y', 'z']);
});

test('colour codes do not hide a failed test', () => {
  assert.deepEqual(failedTests('test \x1b[0mm::n ... FAILED\n'), ['m::n']);
  assert.deepEqual(failedTests('\x1b[0mfailures:\x1b[0m\n\x1b[31m    m::n\x1b[0m\n'), ['m::n']);
});

test('the totals add up every binary of a run', () => {
  const output = 'test result: ok. 10 passed; 0 failed; 2 ignored; 0 measured\ntest result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured\n';
  assert.deepEqual(testTotals(output), { passed: 13, failed: 1, ignored: 2 });
});

test('a repetition is classified by how it ended', () => {
  assert.equal(classify({ code: 0, timedOut: false, output: '' }), 'passed');
  assert.equal(classify({ code: 101, timedOut: false, output: FAILING }), 'failed');
  assert.equal(classify({ code: null, timedOut: true, output: '' }), 'timed_out');
  assert.equal(classify({ code: 101, timedOut: false, output: 'error[E0432]: could not compile `x`' }), 'build_failed');
  assert.equal(classify({ code: 1, timedOut: false, output: 'error: could not compile `x` (lib) due to 1 previous error' }), 'build_failed');
  assert.equal(classify({ code: 1, timedOut: false, output: 'error: linking with `link.exe` failed' }), 'build_failed');
});

test('the tally counts outcomes and failing tests', () => {
  const totals = tally([
    { rep: 1, outcome: 'passed', seconds: 10, tests: [] },
    { rep: 2, outcome: 'failed', seconds: 12, tests: ['a', 'b'] },
    { rep: 3, outcome: 'failed', seconds: 8, tests: ['a'] },
    { rep: 4, outcome: 'timed_out', seconds: 1500, tests: [] },
  ]);
  assert.equal(totals.completed, 4);
  assert.equal(totals.passed, 1);
  assert.equal(totals.failed, 2);
  assert.equal(totals.timedOut, 1);
  assert.deepEqual(totals.perTest, { a: 2, b: 1 });
  assert.equal(totals.secondsMin, 8);
  assert.equal(totals.secondsMax, 1500);
  assert.equal(totals.secondsTotal, 1530);
  assert.equal(tally([]).completed, 0);
});

test('the rule needs every asked repetition, none failed and none hung', () => {
  const clean = tally(Array.from({ length: 200 }, (_, index) => ({ rep: index + 1, outcome: 'passed', seconds: 1, tests: [] })));
  assert.deepEqual(verdict(clean, 200), { ok: true, reasons: [] });
  const short = verdict(clean, 300);
  assert.equal(short.ok, false);
  assert.match(short.reasons[0], /only 200 of 300/);
  const bad = verdict(tally([{ rep: 1, outcome: 'failed', seconds: 1, tests: ['x'] }, { rep: 2, outcome: 'timed_out', seconds: 1, tests: [] }]), 2);
  assert.equal(bad.ok, false);
  assert.match(bad.reasons.join(' '), /1 repetition\(s\) failed/);
  assert.match(bad.reasons.join(' '), /deadlock candidate/);
});

test('a hung repetition is stopped with the platform\'s own way of stopping a tree', () => {
  const calls = [];
  killTree(42, { platform: 'win32', spawnSync: (...args) => calls.push(args) });
  assert.deepEqual(calls[0].slice(0, 2), ['taskkill', ['/PID', '42', '/T', '/F']]);
  const signals = [];
  killTree(42, { platform: 'linux', kill: (pid, signal) => signals.push([pid, signal]) });
  assert.deepEqual(signals, [[-42, 'SIGKILL']]);
  const fallback = [];
  killTree(42, {
    platform: 'linux',
    kill: (pid, signal) => {
      fallback.push([pid, signal]);
      if (pid < 0) throw new Error('no such group');
    },
  });
  assert.deepEqual(fallback, [[-42, 'SIGKILL'], [42, 'SIGKILL']]);
  killTree(42, { platform: 'linux', kill: () => { throw new Error('gone'); } });
});

test('the summary names the suite, the counts and each failing test', () => {
  const totals = tally([{ rep: 1, outcome: 'failed', seconds: 3, tests: ['p::q'] }]);
  const text = renderMarkdown({
    suite: 'locks',
    host: { os: 'linux 6.8' },
    runsAsked: 5,
    totals,
    verdict: verdict(totals, 5),
    testsPerRepetition: 120,
    commit: 'abc',
    cargoVersion: 'cargo 1.98.1',
  });
  assert.match(text, /### locks on linux 6.8: FAILED/);
  assert.match(text, /\| 5 \| 1 \| 0 \| 1 \| 0 \|/);
  assert.match(text, /`p::q` \| 1/);
  assert.match(text, /Tests per repetition: 120/);
});
