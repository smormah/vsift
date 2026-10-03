'use strict';

// What a worker host is using while a batch runs (P14 RQ-09): the container's
// cgroup (memory, process count), and for each process in it the resident
// memory, the open descriptors and the threads. Everything is read from
// `/sys/fs/cgroup` and `/proc`, whose roots are arguments so the reading is
// tested against a fake tree.
//
// Reading another user's descriptors needs root, so the sampler runs as a
// separate process under `sudo` on the hosted runner (sampler.cjs).

const fs = require('node:fs');
const path = require('node:path');

/** Docker's container cgroups: the systemd driver's scopes and the cgroupfs driver's folders. */
function listScopes(cgroupRoot) {
  const found = [];
  const add = (directory, filter) => {
    let entries = [];
    try {
      entries = fs.readdirSync(directory, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      if (entry.isDirectory() && filter.test(entry.name)) {
        found.push({ id: entry.name.replace(/^docker-|\.scope$/g, ''), dir: path.join(directory, entry.name) });
      }
    }
  };
  add(path.join(cgroupRoot, 'system.slice'), /^docker-[0-9a-f]{64}\.scope$/);
  add(path.join(cgroupRoot, 'docker'), /^[0-9a-f]{64}$/);
  return found;
}

function readNumber(file) {
  try {
    const text = fs.readFileSync(file, 'utf8').trim();
    return /^\d+$/.test(text) ? Number(text) : null;
  } catch {
    return null;
  }
}

function readCpuMicroseconds(file) {
  try {
    const match = /^usage_usec (\d+)$/m.exec(fs.readFileSync(file, 'utf8'));
    return match ? Number(match[1]) : null;
  } catch {
    return null;
  }
}

function readPids(dir) {
  try {
    return fs.readFileSync(path.join(dir, 'cgroup.procs'), 'utf8').split('\n').filter(Boolean).map(Number);
  } catch {
    return [];
  }
}

/** One process, or null if it has gone. */
function readProcess(procRoot, pid) {
  let status;
  try {
    status = fs.readFileSync(path.join(procRoot, String(pid), 'status'), 'utf8');
  } catch {
    return null;
  }
  const field = (name) => new RegExp(`^${name}:\\s*(.*)$`, 'm').exec(status);
  const rss = field('VmRSS');
  let fds = null;
  try {
    fds = fs.readdirSync(path.join(procRoot, String(pid), 'fd')).length;
  } catch {
    fds = null;
  }
  return {
    pid,
    ppid: Number((field('PPid') || [])[1] || 0),
    comm: ((field('Name') || [])[1] || '').trim(),
    rssKiB: rss ? Number(/\d+/.exec(rss[1])[0]) : 0,
    threads: Number((field('Threads') || [])[1] || 0),
    fds,
  };
}

/**
 * The coordinator is the `vsift` process whose parent is no `vsift` process of
 * the scope; its descendants are every other process that has it as an
 * ancestor. Init and anything else outside that tree is not counted.
 */
function tree(processes, coordinatorName) {
  const byPid = new Map(processes.map((process_) => [process_.pid, process_]));
  const coordinator = processes.find((process_) => process_.comm === coordinatorName && !(byPid.get(process_.ppid) && byPid.get(process_.ppid).comm === coordinatorName)) || null;
  if (!coordinator) return { coordinator: null, descendants: [] };
  const descends = (process_) => {
    const seen = new Set();
    let current = byPid.get(process_.ppid);
    while (current && !seen.has(current.pid)) {
      if (current.pid === coordinator.pid) return true;
      seen.add(current.pid);
      current = byPid.get(current.ppid);
    }
    return false;
  };
  return { coordinator, descendants: processes.filter((process_) => process_ !== coordinator && descends(process_)) };
}

/**
 * One sample of every container.
 * @param {{ cgroupRoot: string, procRoot: string, coordinatorName?: string, now?: () => number }} options
 */
function sample(options) {
  const now = (options.now || Date.now)();
  const scopes = listScopes(options.cgroupRoot).map((scope) => {
    const processes = readPids(scope.dir).map((pid) => readProcess(options.procRoot, pid)).filter(Boolean);
    const { coordinator, descendants } = tree(processes, options.coordinatorName || 'vsift');
    return {
      id: scope.id.slice(0, 12),
      memoryBytes: readNumber(path.join(scope.dir, 'memory.current')),
      memoryPeakBytes: readNumber(path.join(scope.dir, 'memory.peak')),
      pids: readNumber(path.join(scope.dir, 'pids.current')),
      cpuMicroseconds: readCpuMicroseconds(path.join(scope.dir, 'cpu.stat')),
      coordinator: coordinator && { pid: coordinator.pid, rssKiB: coordinator.rssKiB, fds: coordinator.fds, threads: coordinator.threads },
      descendants: descendants.map((process_) => ({ pid: process_.pid, comm: process_.comm, rssKiB: process_.rssKiB })),
      processes: processes.length,
    };
  });
  return { t: now, scopes };
}

const MIB = 1024 * 1024;

/**
 * The k-th percentile (nearest rank) of `values`.
 * @param {number[]} values
 * @param {number} p between 0 and 100
 */
function percentile(values, p) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.max(1, Math.ceil((p / 100) * sorted.length));
  return sorted[rank - 1];
}

/**
 * Whether a series grew all the way through, after a warm-up: the samples
 * after the first `warmup` share are cut into `windows` equal parts, and the
 * series is called monotonic growth when each part's largest value is above
 * the one before by more than `tolerance` (a share), and the last part is more
 * than `total` (a share) above the first. A flat or noisy series is not.
 * @param {number[]} values
 * @param {{ warmup?: number, windows?: number, tolerance?: number, total?: number, floor?: number }} [options]
 */
function monotonicGrowth(values, options = {}) {
  const { warmup = 0.25, windows = 4, tolerance = 0.02, total = 0.2, floor = 8 } = options;
  const kept = values.slice(Math.floor(values.length * warmup));
  if (kept.length < windows * 2) return { monotonic: false, reason: 'too few samples after the warm-up', maxima: [] };
  const size = Math.floor(kept.length / windows);
  const maxima = [];
  for (let index = 0; index < windows; index += 1) {
    maxima.push(Math.max(...kept.slice(index * size, index === windows - 1 ? kept.length : (index + 1) * size)));
  }
  const rising = maxima.every((value, index) => index === 0 || value > maxima[index - 1] * (1 + tolerance));
  const grown = maxima[windows - 1] > maxima[0] * (1 + total) && maxima[windows - 1] - maxima[0] > floor;
  return { monotonic: rising && grown, reason: rising && grown ? 'every window is above the one before' : 'not a steady rise', maxima };
}

/**
 * The resource curve of one run: the samples of one container, in time order.
 * @param {{ t: number, scope: object }[]} series
 */
function summariseRun(series) {
  const rss = series.filter((entry) => entry.scope.coordinator).map((entry) => entry.scope.coordinator.rssKiB / 1024);
  const fds = series.filter((entry) => entry.scope.coordinator && entry.scope.coordinator.fds !== null).map((entry) => entry.scope.coordinator.fds);
  const memory = series.map((entry) => entry.scope.memoryBytes).filter((value) => value !== null);
  const peaks = series.map((entry) => entry.scope.memoryPeakBytes).filter((value) => value !== null);
  const pids = series.map((entry) => entry.scope.pids).filter((value) => value !== null);
  const descendants = series.map((entry) => entry.scope.descendants.length);
  const seconds = series.length > 1 ? (series[series.length - 1].t - series[0].t) / 1000 : 0;
  return {
    samples: series.length,
    seconds: Math.round(seconds),
    coordinatorRssPeakMiB: rss.length ? Math.round(Math.max(...rss) * 10) / 10 : null,
    coordinatorRssGrowth: monotonicGrowth(rss),
    coordinatorFdsPeak: fds.length ? Math.max(...fds) : null,
    coordinatorFdsGrowth: monotonicGrowth(fds, { floor: 4 }),
    containerMemoryPeakMiB: Math.round((Math.max(0, ...memory, ...peaks)) / MIB),
    pidsPeak: pids.length ? Math.max(...pids) : null,
    descendantsPeak: descendants.length ? Math.max(...descendants) : 0,
  };
}

/**
 * The samples after `afterT + graceMs` that still show a descendant of the
 * coordinator: what must be empty ten seconds after a cancel.
 * @param {{ t: number, scope: object }[]} series
 */
function lingeringDescendants(series, afterT, graceMs) {
  return series.filter((entry) => entry.t > afterT + graceMs && entry.scope.coordinator && entry.scope.descendants.length > 0);
}

/** Groups a sampler's records by container, each in time order. */
function bySeries(records) {
  const groups = new Map();
  for (const record of records) {
    for (const scope of record.scopes) {
      if (!groups.has(scope.id)) groups.set(scope.id, []);
      groups.get(scope.id).push({ t: record.t, scope });
    }
  }
  return groups;
}

module.exports = {
  MIB, listScopes, readProcess, tree, sample, percentile, monotonicGrowth, summariseRun, lingeringDescendants, bySeries,
};
