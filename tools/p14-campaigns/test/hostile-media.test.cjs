'use strict';

// What the malicious-media campaign asks VSift, as far as it can be checked
// without a container: where each operation's session root is, what the size
// cases require, and which findings are filed. (#310, known limit L-134: the
// no-room case named the small filesystem's mount point as the session root,
// a folder VSift refuses, and so never reached the room check.)

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const media = require('../hostile-media.cjs');
const hostile = require('../lib/hostile-cases.cjs');
const judge = require('../lib/hostile-judge.cjs');

const repository = path.join(__dirname, '..', '..', '..');
const base = fs.readFileSync(path.join(repository, 'fixtures', 'corpus', 'generated', 'F01.mp4'));
const all = [...hostile.cases({ fixture: () => base }), ...hostile.nameCases({ fixture: () => base })];
const find = (id) => all.find((entry) => entry.id === id);

const METRICS = 'P14-METRICS exit=7 memory_peak=7000000 pids_peak=6 memory_events=low 0,high 0,max 0,oom 0,oom_kill 0,';
const answered = (json, seconds = 0.1) => ({ code: 7, stdout: JSON.stringify(json), stderr: `${METRICS}\n`, durationMs: seconds * 1000, timedOut: false, oomKilled: false, network: 'none' });
const cliFailure = (code, summary) => ({ status: 'failed', error: { code, remediation: summary ? [{ summary }] : [] } });
const jobFailure = (code) => ({ status: 'failed', data: { failure: { code } } });
const verdict = (spec, op, run) => judge.judge({ spec, op, first: true, run, canaries: [] });

/** The `--session-root` values of a command (the worker case runs two commands in one shell line). */
function rootsOf(command) {
  const words = command[0] === 'sh' ? command[2].split(' ') : command;
  return words.flatMap((word, index) => (word === '--session-root' ? [words[index + 1]] : []));
}

test('loading the campaign for its tests does not start it', () => {
  // A direct run without --out exits 2; loading it must leave the process alone.
  assert.equal(process.exitCode, undefined);
  assert.equal(typeof media.sessionRoot, 'function');
});

test('a case with its own small filesystem roots its sessions in folders inside the mount, never at the mount point', () => {
  const small = all.filter((entry) => entry.tmpfs);
  assert.deepEqual(small.map((entry) => entry.id), ['sparse-no-room']);
  for (const spec of small) {
    assert.deepEqual(media.smallFilesystem(spec), ['--tmpfs', `${media.SMALL_MOUNT}:rw,size=${spec.tmpfs}m,uid=10001,gid=10001,mode=0700`]);
    const used = new Set();
    for (const op of spec.ops) {
      const roots = rootsOf(media.commandFor(op, spec, null));
      assert.ok(roots.length > 0, `${spec.id} ${op} names a session root`);
      for (const root of roots) {
        assert.equal(root, media.sessionRoot(spec, op), `${spec.id} ${op}`);
        assert.notEqual(root, media.SMALL_MOUNT, `${spec.id} ${op}: the mount point exists already and VSift did not create it, so VSift refuses it before it looks at the source`);
        const inside = path.posix.relative(media.SMALL_MOUNT, root);
        assert.ok(inside !== '' && !inside.startsWith('..') && !inside.includes('/'), `${spec.id} ${op}: ${root} is a folder directly inside the small filesystem`);
        used.add(root);
      }
    }
    // A desktop root and a worker workspace are different kinds of root: each operation kind gets its own.
    assert.equal(used.size, 2, spec.id);
  }
});

test('every other case keeps the shared roots', () => {
  const plain = find('sparse-30gib');
  assert.deepEqual(media.smallFilesystem(plain), []);
  assert.deepEqual(rootsOf(media.commandFor('ingest', plain, null)), [media.WORKSPACE]);
  assert.deepEqual(rootsOf(media.commandFor('ingest_human', plain, null)), [media.WORKSPACE]);
  assert.deepEqual(rootsOf(media.commandFor('frame', plain, 'ses_0123456789abcdef')), [media.WORKSPACE]);
  assert.deepEqual(rootsOf(media.commandFor('job_name', plain, null)), [media.WORKER_WORKSPACE]);
  assert.throws(() => media.commandFor('nothing', plain, null), /unknown operation/);
});

test('the no-room case requires the answers of the room check and of the reserve', () => {
  const spec = find('sparse-no-room');
  assert.equal(spec.expect, 'failure');
  assert.deepEqual(spec.ops, ['ingest', 'job_small']);
  const noRoom = `${hostile.NO_ROOM_REMEDIATION_START} of this video. Nothing was committed.`;

  // What the corrected case answered on the published 0.2.0-rc.1 (run 37361623352): refused at once, before the copy.
  assert.equal(verdict(spec, 'ingest', answered(cliFailure('STORAGE_IO', noRoom))).ok, true);
  assert.equal(verdict(spec, 'job_small', answered(jobFailure('RESOURCE_LIMIT'), 0.6)).ok, true);

  // What the mis-built case answered on 0.1.0 and both candidates (run 37613284274): a refused folder, after five seconds.
  const refusedFolder = verdict(spec, 'ingest', answered(cliFailure('INTEGRITY_FAILURE', 'The VSift session_root folder holds no VSift ownership marker'), 5.2));
  assert.equal(refusedFolder.ok, false);
  assert.equal(refusedFolder.outcome, 'failed INTEGRITY_FAILURE');
  const split = judge.splitKnown([{ id: spec.id, op: 'ingest', outcome: refusedFolder.outcome, findings: refusedFolder.findings }], media.TRACKED, []);
  assert.deepEqual([split.known.length, split.fresh.length], [0, 1], 'it fails the run: nothing tracks it any more');

  // The code without the remediation is a disk that failed, or a link: not this case.
  assert.equal(verdict(spec, 'ingest', answered(cliFailure('STORAGE_IO', null))).ok, false);
  // A copy that was let through, or a worker that answered like the desktop path, is a finding too.
  assert.equal(verdict(spec, 'ingest', { ...answered({ status: 'complete', data: { session_id: 'ses_0123456789abcdef' } }), code: 0 }).ok, false);
  assert.equal(verdict(spec, 'job_small', answered(jobFailure('STORAGE_IO'))).ok, false);
});

test('a source over the limit is invalid whatever the free space, and the worker may meet its reserve first', () => {
  const spec = find('sparse-30gib');
  assert.equal(spec.expect, 'failure');
  assert.equal(spec.tmpfs, undefined);
  assert.equal(verdict(spec, 'ingest', answered(cliFailure('INVALID_SOURCE'))).ok, true);
  // #310: the answer of 0.2.0-rc.1, where the room check ran before the size limit.
  assert.equal(verdict(spec, 'ingest', answered(cliFailure('STORAGE_IO', `${hostile.NO_ROOM_REMEDIATION_START} of this video.`))).ok, false);
  assert.equal(verdict(spec, 'ingest', answered(cliFailure('RESOURCE_LIMIT'))).ok, false);
  assert.equal(verdict(spec, 'job_name', answered(jobFailure('RESOURCE_LIMIT'), 0.6)).ok, true);
  assert.equal(verdict(spec, 'job_name', answered(jobFailure('INVALID_SOURCE'))).ok, true);
  assert.equal(verdict(spec, 'job_name', answered(jobFailure('STORAGE_IO'))).ok, false);
});

test('the remediation the no-room case requires is how the product begins it', () => {
  const source = fs.readFileSync(path.join(repository, 'crates', 'vsift-contract', 'src', 'storage.rs'), 'utf8');
  const constant = /pub const SOURCE_NO_ROOM_REMEDIATION: &str = "([^"\\]*)";/.exec(source);
  assert.ok(constant, 'SOURCE_NO_ROOM_REMEDIATION is a plain string constant in vsift-contract');
  assert.ok(constant[1].startsWith(hostile.NO_ROOM_REMEDIATION_START), constant[1]);
  assert.ok(hostile.NO_ROOM_REMEDIATION_START.length >= 40, 'long enough to tell it from any other remediation');
});

test('only filed findings are tracked, each for a case, an operation and an outcome the campaign has', () => {
  assert.deepEqual(media.TRACKED, { 'symlink-to-canary': { issue: 265, op: 'ingest', outcome: 'failed STORAGE_IO' } });
  for (const [id, filed] of Object.entries(media.TRACKED)) {
    const spec = find(id);
    assert.ok(spec, `${id} is a case`);
    assert.ok(spec.ops.includes(filed.op), `${id} runs ${filed.op}`);
    assert.match(filed.outcome, /^failed [A-Z_]+$/);
    assert.ok(Number.isInteger(filed.issue) && filed.issue > 0);
    // A tracked answer is a finding by the case's own rule; if the rule allowed it there would be nothing to track.
    const code = filed.outcome.split(' ')[1];
    const found = verdict(spec, filed.op, answered(filed.op === 'ingest' ? cliFailure(code) : jobFailure(code)));
    assert.equal(found.ok, false, id);
    assert.equal(judge.trackedIssue({ id, op: filed.op, outcome: found.outcome, findings: found.findings }, media.TRACKED), filed.issue);
  }
});

test('the summary says which findings are filed and which are new', () => {
  const linkFinding = { id: 'symlink-to-canary', op: 'ingest', outcome: 'failed STORAGE_IO', findings: ['failed with STORAGE_IO, not one of INVALID_SOURCE, RESOURCE_LIMIT, DEADLINE_EXCEEDED, INVALID_ARGUMENT'] };
  const roomFinding = { id: 'sparse-no-room', op: 'ingest', outcome: 'failed INTEGRITY_FAILURE', findings: ['failed with INTEGRITY_FAILURE, not one of STORAGE_IO'] };
  const report = (findingsList) => {
    const split = judge.splitKnown(findingsList, media.TRACKED, []);
    return { ok: false, known: split.known, fresh: split.fresh, fixed: split.fixed, containmentOk: true, cases: 96, operations: 251, findings: findingsList.length, slowestSeconds: 5.2, peakMiB: 809, results: [], findingsList, containment: [] };
  };
  const tracked = media.markdown(report([linkFinding]));
  assert.match(tracked, /FINDINGS, all tracked \(#265\)/);
  assert.match(tracked, /`symlink-to-canary` ingest: .* \(tracked: #265\)/);
  const fresh = media.markdown(report([linkFinding, roomFinding]));
  assert.match(fresh, /NEW FINDINGS/);
  assert.match(fresh, /`sparse-no-room` ingest: failed with INTEGRITY_FAILURE, not one of STORAGE_IO \(NEW\)/);
  assert.match(fresh, /`symlink-to-canary` ingest: .* \(tracked: #265\)/);
});
