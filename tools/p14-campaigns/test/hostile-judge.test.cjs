'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const j = require('../lib/hostile-judge.cjs');

const METRICS = (over = '') => `P14-METRICS exit=1 memory_peak=300000000 pids_peak=20 memory_events=low 0,high 0,max 0,oom 0,oom_kill ${over || 0},`;
const failed = (code) => JSON.stringify({ status: 'failed', error: { code } });
const run = (over = {}) => ({ code: 1, stdout: failed('INVALID_SOURCE'), stderr: `${METRICS()}\n`, durationMs: 2000, timedOut: false, oomKilled: false, network: 'none', ...over });
const spec = (over = {}) => ({ id: 'x', expect: 'failure', ...over });

test('tracked findings are told apart from new ones, and a fixed case is named', () => {
  const findings = [
    { id: 'fifo', op: 'ingest', findings: ['hang'] },
    { id: 'brand-new', op: 'ingest', findings: ['wrong code'] },
  ];
  const split = j.splitKnown(findings, { fifo: 264, 'symlink-to-canary': 265 }, ['symlink-to-canary', 'other']);
  assert.deepEqual(split.known.map((entry) => [entry.id, entry.issue]), [['fifo', 264]]);
  assert.deepEqual(split.fresh.map((entry) => entry.id), ['brand-new']);
  assert.deepEqual(split.fixed, ['symlink-to-canary']);
  assert.deepEqual(j.splitKnown([], {}, []), { known: [], fresh: [], fixed: [] });
});

test('the container\'s own metrics are read from the last line', () => {
  assert.deepEqual(j.readMetrics(`noise\n${METRICS(2)}\n`), { exit: 1, memoryPeak: 300000000, pidsPeak: 20, oomKills: 2 });
  assert.equal(j.readMetrics('nothing here'), null);
});

test('raw control characters are found, JSON-escaped ones are not', () => {
  assert.deepEqual(j.rawControls('plain text\nand a tab\t'), []);
  assert.deepEqual(j.rawControls('bad \u001b[2J'), ['0x1b']);
  assert.deepEqual(j.rawControls('c1 \u009b'), ['U+009b']);
  assert.deepEqual(j.rawControls('nul \u0000 and del \u007f').sort(), ['0x00', '0x7f']);
  assert.deepEqual(j.rawControls(JSON.stringify('esc \u001b done')), []);
});

test('a typed failure inside the bounds is what a hostile input must come to', () => {
  const verdict = j.judge({ spec: spec(), op: 'ingest', first: true, run: run(), canaries: ['CANARY'] });
  assert.equal(verdict.ok, true);
  assert.equal(verdict.outcome, 'failed INVALID_SOURCE');
});

test('an untyped code, a success where a refusal is due, and a wrong exit status are findings', () => {
  assert.match(j.judge({ spec: spec(), op: 'ingest', first: true, run: run({ stdout: failed('INTERNAL') }), canaries: [] }).findings[0], /INTERNAL, not one of/);
  assert.match(j.judge({ spec: spec(), op: 'ingest', first: true, run: run({ code: 0, stdout: JSON.stringify({ status: 'complete', data: {} }) }), canaries: [] }).findings[0], /must be refused/);
  assert.match(j.judge({ spec: spec(), op: 'ingest', first: true, run: run({ code: 0 }), canaries: [] }).findings.join(' '), /exited 0/);
  assert.equal(j.judge({ spec: spec({ expect: 'any' }), op: 'ingest', first: true, run: run({ code: 0, stdout: JSON.stringify({ status: 'complete', data: {} }) }), canaries: [] }).ok, true);
});

test('a case may allow more codes, and follow-up calls may add an invalid argument', () => {
  assert.equal(j.judge({ spec: spec({ codes: ['STORAGE_IO'] }), op: 'ingest', first: true, run: run({ stdout: failed('STORAGE_IO') }), canaries: [] }).ok, true);
  assert.equal(j.judge({ spec: spec({ expect: 'any' }), op: 'frame', first: false, run: run({ stdout: failed('INVALID_ARGUMENT') }), canaries: [] }).ok, true);
  assert.equal(j.judge({ spec: spec({ expect: 'any' }), op: 'ingest', first: true, run: run({ stdout: failed('INVALID_ARGUMENT') }), canaries: [] }).ok, false);
});

test('a hang, an out-of-memory, too much memory, too many processes and a network are findings', () => {
  const findings = (over) => j.judge({ spec: spec(), op: 'ingest', first: true, run: run(over), canaries: [] }).findings.join('; ');
  assert.match(findings({ timedOut: true }), /a hang/);
  assert.match(findings({ durationMs: 121_000 }), /took 121 s/);
  assert.match(findings({ oomKilled: true }), /ran out of memory/);
  assert.match(findings({ code: 137 }), /ran out of memory/);
  assert.match(findings({ stderr: `${METRICS(1)}\n` }), /ran out of memory/);
  assert.match(findings({ stderr: 'P14-METRICS exit=1 memory_peak=2000000000 pids_peak=20 memory_events=\n' }), /above the 1024 MiB limit/);
  assert.match(findings({ stderr: 'P14-METRICS exit=1 memory_peak=1 pids_peak=500 memory_events=\n' }), /500 processes/);
  assert.match(findings({ network: 'bridge' }), /network was bridge/);
});

test('output a terminal would act on, or a canary from outside the root, is a finding', () => {
  assert.match(j.judge({ spec: spec(), op: 'ingest', first: true, run: run({ stderr: `\u001b[31mred\n${METRICS()}\n` }), canaries: [] }).findings.join(';'), /raw control characters/);
  assert.match(j.judge({ spec: spec(), op: 'ingest', first: true, run: run({ stdout: JSON.stringify({ status: 'failed', error: { code: 'INVALID_SOURCE', message: 'CANARY-1' } }) }), canaries: ['CANARY-1'] }).findings.join(';'), /canary/);
});

test('the human output is judged only for what it carries', () => {
  const verdict = j.judge({ spec: spec({ expect: 'any' }), op: 'ingest_human', first: true, run: run({ stdout: 'The source is invalid.\n' }), canaries: [] });
  assert.equal(verdict.ok, true);
  assert.equal(j.judge({ spec: spec({ expect: 'any' }), op: 'ingest_human', first: true, run: run({ stdout: 'name \u001b[2J\n' }), canaries: [] }).ok, false);
});

test('an answer that is not JSON is a finding', () => {
  const verdict = j.judge({ spec: spec(), op: 'ingest', first: true, run: run({ stdout: 'panic: boom' }), canaries: [] });
  assert.equal(verdict.ok, false);
  assert.match(verdict.findings[0], /not JSON/);
});

test('a media file that must be refused is refused when some operation fails typed and nothing succeeds after', () => {
  const r = (outcome) => ({ outcome, ok: true });
  assert.equal(j.refusedSomewhere([r('complete'), r('failed INVALID_SOURCE')]), null);
  assert.equal(j.refusedSomewhere([r('failed INVALID_SOURCE')]), null);
  assert.match(j.refusedSomewhere([r('complete'), r('partial')]), /never refused/);
  assert.match(j.refusedSomewhere([r('complete'), r('failed INVALID_SOURCE'), r('complete')]), /succeeded after the refusal/);
});

test('a kill by the harness is a hang, not an out-of-memory', () => {
  const verdict = j.judge({ spec: { id: 'x', expect: 'failure' }, op: 'ingest', first: true, run: { code: 137, stdout: '', stderr: '', durationMs: 150000, timedOut: true, oomKilled: false, network: 'none' }, canaries: [] });
  assert.ok(verdict.findings.some((finding) => /a hang/.test(finding)));
  assert.ok(!verdict.findings.some((finding) => /out of memory/.test(finding)));
});

test('a job result carries its failure code in its data', () => {
  assert.equal(j.codeOf({ status: 'failed', data: { failure: { code: 'INVALID_ARGUMENT' } } }), 'INVALID_ARGUMENT');
  assert.equal(j.codeOf({ error: { code: 'RESOURCE_LIMIT' } }), 'RESOURCE_LIMIT');
  assert.equal(j.codeOf(null), null);
});
