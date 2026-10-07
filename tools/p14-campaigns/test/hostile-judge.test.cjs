'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const j = require('../lib/hostile-judge.cjs');

const METRICS = (over = '') => `P14-METRICS exit=1 memory_peak=300000000 pids_peak=20 memory_events=low 0,high 0,max 0,oom 0,oom_kill ${over || 0},`;
const failed = (code) => JSON.stringify({ status: 'failed', error: { code } });
const run = (over = {}) => ({ code: 1, stdout: failed('INVALID_SOURCE'), stderr: `${METRICS()}\n`, durationMs: 2000, timedOut: false, oomKilled: false, network: 'none', ...over });
const spec = (over = {}) => ({ id: 'x', expect: 'failure', ...over });

const TRACKED = { link: { issue: 265, op: 'ingest', outcome: 'failed STORAGE_IO' }, pipe: { issue: 264, op: 'ingest', outcome: 'no JSON (exit 137)' } };
const wrongCode = (code) => `failed with ${code}, not one of INVALID_SOURCE, RESOURCE_LIMIT, DEADLINE_EXCEEDED`;

test('tracked findings are told apart from new ones, and a fixed case is named', () => {
  const findings = [
    { id: 'link', op: 'ingest', outcome: 'failed STORAGE_IO', findings: [wrongCode('STORAGE_IO')] },
    { id: 'brand-new', op: 'ingest', outcome: 'failed INTERNAL', findings: [wrongCode('INTERNAL')] },
  ];
  const split = j.splitKnown(findings, TRACKED, ['pipe', 'other']);
  assert.deepEqual(split.known.map((entry) => [entry.id, entry.issue]), [['link', 265]]);
  assert.deepEqual(split.fresh.map((entry) => entry.id), ['brand-new']);
  assert.deepEqual(split.fixed, ['pipe']);
  assert.deepEqual(j.splitKnown([], {}, []), { known: [], fresh: [], fixed: [] });
});

test('a tracked case that answers anything but what was filed is a new finding', () => {
  // #310, L-134: the no-room case was tracked by its id alone, so its INTEGRITY_FAILURE (the tool had named a folder
  // VSift refuses) was counted as the filed finding and no run failed for it.
  const filed = { id: 'link', op: 'ingest', outcome: 'failed STORAGE_IO', findings: [wrongCode('STORAGE_IO')] };
  assert.equal(j.trackedIssue(filed, TRACKED), 265);
  for (const [what, other] of [
    ['another outcome', { ...filed, outcome: 'failed INTEGRITY_FAILURE', findings: [wrongCode('INTEGRITY_FAILURE')] }],
    ['another operation', { ...filed, op: 'job_name' }],
    ['something beside the wrong code', { ...filed, findings: [...filed.findings, 'raw control characters in the output (0x1b)'] }],
    ['a case that is not tracked', { ...filed, id: 'other' }],
  ]) {
    assert.equal(j.trackedIssue(other, TRACKED), null, what);
    const split = j.splitKnown([other], TRACKED, []);
    assert.deepEqual([split.known.length, split.fresh.length], [0, 1], what);
  }
  assert.equal(j.trackedIssue({ ...filed, id: 'toString' }, TRACKED), null, 'an inherited property is not a tracked case');
});

test('a pinned answer is the only one an operation may give', () => {
  const pinned = spec({ expect: 'failure', codes: ['INVALID_SOURCE', 'RESOURCE_LIMIT', 'DEADLINE_EXCEEDED', 'STORAGE_IO'], answers: { ingest: { codes: ['STORAGE_IO'], remediation: 'The folder has no room' }, job_small: { codes: ['RESOURCE_LIMIT'] } } });
  const noRoom = (summary) => JSON.stringify({ status: 'failed', error: { code: 'STORAGE_IO', remediation: summary === null ? [] : [{ summary }] } });
  const ingest = (stdout, over = {}) => j.judge({ spec: pinned, op: 'ingest', first: true, run: run({ stdout, ...over }), canaries: [] });

  assert.equal(ingest(noRoom('The folder has no room for a copy. Free some.')).ok, true);
  // The answer the mis-built case gave: typed and inside its bound, and not what the case is about.
  assert.match(ingest(failed('INTEGRITY_FAILURE')).findings[0], /failed with INTEGRITY_FAILURE, not one of STORAGE_IO$/);
  // A code the case's wider list or the plan allows is still not the pinned one.
  assert.match(ingest(failed('INVALID_SOURCE')).findings[0], /failed with INVALID_SOURCE, not one of STORAGE_IO$/);
  // The right code for another reason: a disk that failed answers STORAGE_IO too.
  assert.match(ingest(noRoom('The path you gave is a link.')).findings[0], /its remediation does not begin "The folder has no room": "The path you gave is a link\."/);
  assert.match(ingest(noRoom(null)).findings[0], /its remediation does not begin/);
  assert.match(ingest(failed('STORAGE_IO')).findings[0], /its remediation does not begin/);
  assert.match(ingest(JSON.stringify({ status: 'complete', data: {} }), { code: 0 }).findings[0], /was accepted \(complete\) but must fail with STORAGE_IO/);

  const worker = (code) => j.judge({ spec: pinned, op: 'job_small', first: true, run: run({ stdout: JSON.stringify({ status: 'failed', data: { failure: { code } } }) }), canaries: [] });
  assert.equal(worker('RESOURCE_LIMIT').ok, true);
  assert.equal(worker('STORAGE_IO').ok, false);
  // An operation the case does not pin keeps the case's own codes, and a follow-up call its extra one.
  assert.equal(j.judge({ spec: pinned, op: 'frame', first: false, run: run({ stdout: failed('INVALID_ARGUMENT') }), canaries: [] }).ok, true);
  assert.equal(j.judge({ spec: spec({ answers: { ingest: { codes: ['INVALID_SOURCE'] } } }), op: 'frame', first: false, run: run({ stdout: failed('RESOURCE_LIMIT') }), canaries: [] }).ok, true);
  // A pinned operation does not get the follow-up code.
  assert.equal(j.judge({ spec: spec({ answers: { frame: { codes: ['INVALID_SOURCE'] } } }), op: 'frame', first: false, run: run({ stdout: failed('INVALID_ARGUMENT') }), canaries: [] }).ok, false);
});

test('the first remediation of a failed command is read, and nothing else is taken for one', () => {
  assert.equal(j.remediationOf({ error: { code: 'STORAGE_IO', remediation: [{ summary: 'first' }, { summary: 'second' }] } }), 'first');
  for (const json of [null, {}, { error: {} }, { error: { remediation: [] } }, { error: { remediation: 'text' } }, { error: { remediation: [{ summary: 7 }] } }, { data: { failure: { code: 'RESOURCE_LIMIT' } } }]) {
    assert.equal(j.remediationOf(json), '');
  }
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
