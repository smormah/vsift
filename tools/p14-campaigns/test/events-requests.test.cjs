'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const events = require('../lib/events.cjs');
const requests = require('../lib/requests.cjs');

const repository = path.join(__dirname, '..', '..', '..');
const frozenEvents = fs.readFileSync(path.join(repository, 'schemas', 'v1', 'examples', 'job-batch.events.jsonl'), 'utf8');

test('the frozen batch example is read: started, two lines, stopped, terminal', () => {
  const { events: list, problems } = events.parseEvents(frozenEvents);
  assert.deepEqual(problems, []);
  const batch = events.readBatch(list);
  assert.deepEqual(batch.problems, []);
  assert.equal(batch.started.concurrency, 2);
  assert.equal(batch.stopped, 'end_of_input');
  assert.equal(batch.ended, true);
  assert.equal(batch.outcomes.size, 2);
  assert.equal(events.disposition(batch.outcomes.get(1)), 'recorded');
  assert.equal(batch.peakRunning, 2);
  assert.equal(batch.results.get(2).replayed, false);
});

test('a stream that is not ready, out of order or too large is reported', () => {
  const bad = events.parseEvents('{"event":"progress","sequence":1}\n{"event":"progress","sequence":1}\nnot json\n');
  assert.match(bad.problems[0], /not JSON/);
  const batch = events.readBatch(bad.events);
  assert.ok(batch.problems.some((problem) => /not lifecycle started/.test(problem)));
  assert.ok(batch.problems.some((problem) => /does not follow/.test(problem)));
  const huge = events.parseEvents(`{"event":"x","pad":"${'a'.repeat(events.MAX_EVENT_BYTES)}"}\n`);
  assert.match(huge.problems[0], /above the 65536 byte bound/);
});

test('the runbook table of what the supervisor does with an outcome', () => {
  const d = (status, code = null) => events.disposition({ status, code, rejection: null });
  assert.equal(d('complete'), 'recorded');
  assert.equal(d('partial'), 'recorded');
  assert.equal(d('failed', 'INVALID_SOURCE'), 'dead_letter');
  assert.equal(d('failed', 'MISSING_CAPABILITY'), 'dead_letter');
  assert.equal(d('failed', 'IDEMPOTENCY_CONFLICT'), 'conflict');
  assert.equal(d('failed', 'BUSY'), 'redeliver');
  assert.equal(d('failed', 'DEADLINE_EXCEEDED'), 'redeliver');
  assert.equal(d('failed', 'STORAGE_IO'), 'redeliver');
  assert.equal(d('cancelled'), 'redeliver');
  assert.equal(d('rejected'), 'dead_letter');
  assert.equal(events.disposition(undefined), 'redeliver');
});

test('a planned line is held to what it was expected to come to', () => {
  const outcome = (status, code = null) => ({ status, code, rejection: null });
  assert.equal(events.meets({ expect: 'recorded' }, outcome('complete')).ok, true);
  assert.equal(events.meets({ expect: 'recorded' }, outcome('failed', 'BUSY')).ok, false);
  assert.equal(events.meets({ expect: 'replayed' }, outcome('complete'), { replayed: true }).ok, true);
  assert.equal(events.meets({ expect: 'replayed' }, outcome('complete'), { replayed: false }).ok, false);
  assert.equal(events.meets({ expect: 'rejected' }, outcome('rejected')).ok, true);
  assert.equal(events.meets({ expect: 'rejected' }, undefined).ok, false);
  assert.equal(events.meets({ expect: 'failed', codes: ['IDEMPOTENCY_CONFLICT'] }, outcome('failed', 'IDEMPOTENCY_CONFLICT')).ok, true);
  assert.equal(events.meets({ expect: 'failed', codes: ['INVALID_SOURCE'] }, outcome('failed', 'INTERNAL')).ok, false);
});

test('a leak is any forbidden string in the output', () => {
  assert.deepEqual(events.leaks('a /srv/vsift/inputs b canary-123', ['/srv/vsift', 'canary-123', 'nope']), ['/srv/vsift', 'canary-123']);
  assert.deepEqual(events.leaks('clean', ['', 'x']), []);
});

test('operation ids are valid, stable and distinct', () => {
  const first = requests.operationId('seed', 1);
  assert.match(first, /^op_[0-9a-f]{32}$/);
  assert.equal(first, requests.operationId('seed', 1));
  assert.notEqual(first, requests.operationId('seed', 2));
  assert.notEqual(first, requests.operationId('other', 1));
});

test('the generator is deterministic', () => {
  const a = new requests.Random('x');
  const b = new requests.Random('x');
  assert.deepEqual([a.int(1000), a.int(1000), a.int(1000)], [b.int(1000), b.int(1000), b.int(1000)]);
  assert.notDeepEqual(new requests.Random('y').int(1e9), new requests.Random('z').int(1e9));
});

test('the soak mix has the plan\'s kinds and every well-formed line is a v1 request', () => {
  const mix = requests.soakMix('p14', 1000);
  assert.equal(mix.length, 1000);
  const kinds = new Set(mix.map((line) => line.kind));
  for (const kind of ['ingest_candidates', 'ingest_open', 'ingest_candidates_retain', 'recognise', 'supplied_transcript', 'malformed', 'duplicate', 'conflict', 'missing_source']) {
    assert.ok(kinds.has(kind), `no ${kind} line`);
  }
  assert.deepEqual(requests.soakMix('p14', 1000), mix);
  for (const line of mix) {
    if (line.kind === 'malformed') continue;
    const body = JSON.parse(line.text);
    assert.equal(body.schema_version, '1');
    assert.match(body.operation_id, /^op_[0-9a-f]{32}$/);
    assert.ok(Buffer.byteLength(line.text) <= 65536);
    assert.ok(body.steps.length <= 8);
    if (line.kind === 'duplicate') assert.ok(mix.some((other) => other.id === line.id && other.kind !== 'duplicate' && other.kind !== 'conflict'));
  }
  // A duplicate has the bytes of the request it repeats; a conflict has another body under the same id.
  const original = mix.find((line) => line.kind === 'ingest_candidates');
  assert.ok(original);
  const conflict = mix.find((line) => line.kind === 'conflict');
  assert.ok(conflict);
  const first = mix.find((line) => line.id === conflict.id && line.kind !== 'conflict' && line.kind !== 'duplicate');
  assert.notEqual(first.text, conflict.text);
});

test('the ladder asks the same requests of every rung, one in six recognising speech', () => {
  const set = requests.ladderRequests('ladder', 24);
  assert.equal(set.length, 24);
  assert.equal(set.filter((line) => line.kind === 'recognise').length, 4);
  assert.deepEqual(requests.ladderRequests('ladder', 24), set);
  assert.equal(new Set(set.map((line) => line.id)).size, 24);
});

test('a batch file is the lines in order, newline ended', () => {
  const text = requests.batchFile([{ text: 'a' }, { text: 'b' }]);
  assert.equal(text, 'a\nb\n');
});

test('a late duplicate or conflict that ran as a new request is outside the dedupe window, nothing else is', () => {
  const outcome = (status, code = null) => ({ status, code, rejection: null });
  const duplicate = { expect: 'replayed' };
  const conflict = { expect: 'failed', codes: ['IDEMPOTENCY_CONFLICT'] };
  assert.equal(events.outsideWindow(duplicate, outcome('complete'), { replayed: false }), true);
  assert.equal(events.outsideWindow(duplicate, outcome('complete'), { replayed: true }), false);
  assert.equal(events.outsideWindow(duplicate, outcome('failed', 'RESOURCE_LIMIT'), undefined), false);
  assert.equal(events.outsideWindow(conflict, outcome('complete'), { replayed: false }), true);
  assert.equal(events.outsideWindow(conflict, outcome('failed', 'IDEMPOTENCY_CONFLICT'), undefined), false);
  assert.equal(events.outsideWindow({ expect: 'recorded' }, outcome('complete'), { replayed: false }), false);
});

test('bundle names fit the grammar', () => {
  const name = requests.bundleName(requests.operationId('s', 3));
  assert.match(name, /^[a-z0-9][a-z0-9_-]{0,63}$/);
});
