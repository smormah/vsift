'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const sampler = require('../lib/cgroup-sampler.cjs');

const ID = 'a'.repeat(64);

/** Builds a fake /sys/fs/cgroup and /proc with one container of the given processes. */
function fixture(processes, { memory = 100 * 1024 * 1024, peak = 120 * 1024 * 1024, driver = 'systemd' } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift-sampler-'));
  const cgroup = path.join(root, 'sys');
  const proc = path.join(root, 'proc');
  const dir = driver === 'systemd' ? path.join(cgroup, 'system.slice', `docker-${ID}.scope`) : path.join(cgroup, 'docker', ID);
  fs.mkdirSync(dir, { recursive: true });
  fs.mkdirSync(proc, { recursive: true });
  fs.writeFileSync(path.join(dir, 'memory.current'), `${memory}\n`);
  fs.writeFileSync(path.join(dir, 'memory.peak'), `${peak}\n`);
  fs.writeFileSync(path.join(dir, 'pids.current'), `${processes.length}\n`);
  fs.writeFileSync(path.join(dir, 'cpu.stat'), 'usage_usec 4200\nuser_usec 4000\n');
  fs.writeFileSync(path.join(dir, 'cgroup.procs'), `${processes.map((entry) => entry.pid).join('\n')}\n`);
  for (const entry of processes) {
    fs.mkdirSync(path.join(proc, String(entry.pid), 'fd'), { recursive: true });
    for (let index = 0; index < (entry.fds || 0); index += 1) fs.writeFileSync(path.join(proc, String(entry.pid), 'fd', String(index)), '');
    fs.writeFileSync(
      path.join(proc, String(entry.pid), 'status'),
      `Name:\t${entry.comm}\nPPid:\t${entry.ppid}\nThreads:\t${entry.threads || 1}\nVmRSS:\t${entry.rssKiB || 0} kB\n`,
    );
  }
  return { root, cgroup, proc };
}

test('a container is read: memory, pids, the coordinator and what it started', () => {
  const { root, cgroup, proc } = fixture([
    { pid: 1, ppid: 0, comm: 'docker-init', rssKiB: 100, fds: 3 },
    { pid: 7, ppid: 1, comm: 'vsift', rssKiB: 51200, fds: 12, threads: 9 },
    { pid: 8, ppid: 7, comm: 'ffmpeg', rssKiB: 20480, fds: 5 },
    { pid: 9, ppid: 8, comm: 'ffmpeg-child', rssKiB: 10, fds: 4 },
  ]);
  const record = sampler.sample({ cgroupRoot: cgroup, procRoot: proc, now: () => 1234 });
  assert.equal(record.t, 1234);
  assert.equal(record.scopes.length, 1);
  const scope = record.scopes[0];
  assert.equal(scope.id, 'aaaaaaaaaaaa');
  assert.equal(scope.memoryBytes, 100 * 1024 * 1024);
  assert.equal(scope.memoryPeakBytes, 120 * 1024 * 1024);
  assert.equal(scope.pids, 4);
  assert.equal(scope.cpuMicroseconds, 4200);
  assert.deepEqual(scope.coordinator, { pid: 7, rssKiB: 51200, fds: 12, threads: 9 });
  assert.deepEqual(scope.descendants.map((entry) => entry.comm), ['ffmpeg', 'ffmpeg-child']);
  fs.rmSync(root, { recursive: true, force: true });
});

test('the cgroupfs driver\'s layout is read too, and a missing cgroup is an empty sample', () => {
  const { root, cgroup, proc } = fixture([{ pid: 7, ppid: 0, comm: 'vsift', rssKiB: 1, fds: 1 }], { driver: 'cgroupfs' });
  assert.equal(sampler.sample({ cgroupRoot: cgroup, procRoot: proc }).scopes.length, 1);
  assert.deepEqual(sampler.sample({ cgroupRoot: path.join(root, 'nothing'), procRoot: proc }).scopes, []);
  fs.rmSync(root, { recursive: true, force: true });
});

test('a vsift started by the coordinator is a descendant, not a second coordinator', () => {
  const processes = [
    { pid: 7, ppid: 1, comm: 'vsift' },
    { pid: 8, ppid: 7, comm: 'vsift' },
    { pid: 9, ppid: 8, comm: 'ffprobe' },
  ];
  const { coordinator, descendants } = sampler.tree(processes, 'vsift');
  assert.equal(coordinator.pid, 7);
  assert.deepEqual(descendants.map((entry) => entry.pid), [8, 9]);
  assert.equal(sampler.tree([{ pid: 1, ppid: 0, comm: 'sh' }], 'vsift').coordinator, null);
});

test('a process that has gone is not read', () => {
  assert.equal(sampler.readProcess(os.tmpdir(), 99999999), null);
});

test('percentiles use the nearest rank', () => {
  assert.equal(sampler.percentile([5, 1, 3, 2, 4], 50), 3);
  assert.equal(sampler.percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 95), 10);
  assert.equal(sampler.percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20], 95), 19);
  assert.equal(sampler.percentile([], 50), null);
});

test('steady growth after the warm-up is flagged; a plateau and noise are not', () => {
  const rising = Array.from({ length: 80 }, (_, index) => 100 + index * 2);
  assert.equal(sampler.monotonicGrowth(rising).monotonic, true);
  const plateau = Array.from({ length: 80 }, (_, index) => (index < 10 ? 50 + index * 5 : 100));
  assert.equal(sampler.monotonicGrowth(plateau).monotonic, false);
  const noisy = Array.from({ length: 80 }, (_, index) => 100 + (index % 7));
  assert.equal(sampler.monotonicGrowth(noisy).monotonic, false);
  assert.equal(sampler.monotonicGrowth([1, 2, 3]).monotonic, false);
  // A rise inside the floor does not count: a few descriptors more is not a leak.
  const small = Array.from({ length: 80 }, (_, index) => 10 + Math.floor(index / 40));
  assert.equal(sampler.monotonicGrowth(small, { floor: 4 }).monotonic, false);
});

test('a run is summarised and what outlives a cancel is found', () => {
  const entry = (t, rssKiB, descendants, fds = 10) => ({
    t,
    scope: { coordinator: { pid: 7, rssKiB, fds, threads: 4 }, descendants: descendants.map((comm) => ({ pid: 8, comm, rssKiB: 1 })), memoryBytes: 200 * sampler.MIB, memoryPeakBytes: 300 * sampler.MIB, pids: 5 },
  });
  const series = [entry(0, 51200, []), entry(1000, 61440, ['ffmpeg']), entry(2000, 71680, ['ffmpeg']), entry(15000, 71680, ['ffmpeg']), entry(16000, 71680, [])];
  const summary = sampler.summariseRun(series);
  assert.equal(summary.seconds, 16);
  assert.equal(summary.coordinatorRssPeakMiB, 70);
  assert.equal(summary.containerMemoryPeakMiB, 300);
  assert.equal(summary.pidsPeak, 5);
  assert.equal(summary.descendantsPeak, 1);
  assert.equal(summary.coordinatorFdsPeak, 10);
  assert.deepEqual(sampler.lingeringDescendants(series, 2000, 10000).map((item) => item.t), [15000]);
  assert.deepEqual(sampler.lingeringDescendants(series, 5000, 10000), []);
});

test('records are grouped by container in time order', () => {
  const groups = sampler.bySeries([
    { t: 1, scopes: [{ id: 'a' }, { id: 'b' }] },
    { t: 2, scopes: [{ id: 'a' }] },
  ]);
  assert.deepEqual(groups.get('a').map((item) => item.t), [1, 2]);
  assert.deepEqual(groups.get('b').map((item) => item.t), [1]);
});
