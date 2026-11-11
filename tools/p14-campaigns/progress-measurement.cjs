#!/usr/bin/env node
'use strict';

// The hosted measurement behind ADR 0033 sections 6 and 9 (0.3.0 readiness
// item 5, brief W8): the quick/watched threshold and the progress estimate's
// error, on public real recordings. It is the `progress` mode of
// .github/workflows/p14-real-media.yml and runs nowhere else: nothing long runs
// on a development machine.
//
//   node progress-measurement.cjs --vsift <exe> --media <dir> --work <dir> --report <dir>
//     --ffmpeg <exe> --ffprobe <exe> --whisper <exe> --model <file>
//     --item RMnn [--item RMnn]... [--prefix-of RMnn] [--prefix-minutes 5,10,15] [--manifest <file>]
//
// For each whole item, and for each prefix of --prefix-of (cut here by FFmpeg),
// it makes two fresh sessions and runs `job start --mode quick` on one and
// `job start --mode watched` on the other, the order alternating from one
// medium to the next. A watched run is followed with `job status` every ten
// seconds. It writes `progress.json` (every figure, and every status sample)
// and `progress.md` (the tables) into --report. Public items only: it
// refuses --private-manifest (see lib/progress-measurement.cjs). The media and
// the sessions stay on the runner. Advisory evidence: the exit code is 1 only
// when a run did not seal (a tooling or product failure), never for a number.
//
// Node.js 22, no dependency.

const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const media = require('./lib/real-media.cjs');
const campaign = require('./lib/real-media-campaign.cjs');
const measure = require('./lib/progress-measurement.cjs');

const STEP_TIMEOUT_MS = 4 * 60 * 60 * 1000;

const clock = () => Number(process.hrtime.bigint()) / 1e9;
const sleep = (milliseconds) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);

function digestOf(file, algorithm) {
  const hash = crypto.createHash(algorithm);
  const descriptor = fs.openSync(file, 'r');
  try {
    const buffer = Buffer.alloc(1 << 20);
    let read;
    while ((read = fs.readSync(descriptor, buffer, 0, buffer.length, null)) > 0) hash.update(buffer.subarray(0, read));
  } finally {
    fs.closeSync(descriptor);
  }
  return hash.digest('hex');
}

/** `/proc/cpuinfo` on Linux, for the runner's CPU flags; `null` elsewhere or when it cannot be read. */
function readCpuinfo() {
  if (process.platform !== 'linux') return null;
  try {
    return fs.readFileSync('/proc/cpuinfo', 'utf8');
  } catch {
    return null;
  }
}

/** Runs `vsift` with the run's per-user folders and returns its answer, timed (the campaign's runner). */
function runner(vsift, user, log) {
  const env = { ...process.env, LOCALAPPDATA: user, APPDATA: path.join(user, 'roaming'), XDG_CONFIG_HOME: path.join(user, 'config'),
    XDG_CACHE_HOME: path.join(user, 'cache'), XDG_DATA_HOME: path.join(user, 'data'), HOME: user };
  return (step, args, { root = null, quiet = false } = {}) => {
    const full = [...(root ? ['--session-root', root] : []), ...args, '--json'];
    const started = process.hrtime.bigint();
    const result = childProcess.spawnSync(vsift, full, { encoding: 'utf8', env, maxBuffer: 1 << 29, timeout: STEP_TIMEOUT_MS, windowsHide: true });
    const seconds = Number(process.hrtime.bigint() - started) / 1e9;
    let json = null;
    try {
      json = JSON.parse(result.stdout);
    } catch {
      json = null;
    }
    if (!quiet) log(`  ${step}: exit ${result.status} in ${seconds.toFixed(1)} s${json && json.error ? ` (${json.error.code})` : ''}`);
    return { step, exit: result.status, seconds, json };
  };
}

const spawnTool = (program, args) => {
  const result = childProcess.spawnSync(program, args, { encoding: 'utf8', maxBuffer: 1 << 24, windowsHide: true });
  return { status: result.status, stdout: result.stdout || '' };
};

/** One recording's duration in seconds, read by FFprobe. */
function probeDuration(ffprobe, file) {
  const probed = spawnTool(ffprobe, ['-v', 'error', '-show_entries', 'format=duration', '-of', 'default=noprint_wrappers=1:nokey=1', file]);
  const duration = Number(probed.stdout.trim());
  if (probed.status !== 0 || !Number.isFinite(duration) || duration <= 0) throw new measure.ProgressMeasurementError('FFprobe could not read the recording\'s duration');
  return duration;
}

/** The revision a run sealed, read back whole: its block and every segment of it. */
function readRevision(at, session, revisionId, durationUs) {
  const all = campaign.allSegments(at, session, revisionId, durationUs);
  const revision = all.first && all.first.json && all.first.json.data && all.first.json.data.revision;
  if (all.failed || !revision) return null;
  const asr = revision.local_asr || {};
  return {
    revision_id: revision.revision_id,
    segment_count: revision.segment_count,
    threads: Number.isInteger(asr.threads) ? asr.threads : null,
    chunk_count: asr.chunk_count === undefined ? null : asr.chunk_count,
    segments: all.segments,
  };
}

function main() {
  const options = measure.parseArguments(process.argv.slice(2));
  const log = (line) => process.stdout.write(`${line}\n`);
  const manifest = media.readManifest(path.resolve(options.manifest || path.join(__dirname, 'real-media', 'manifest.json')));
  const items = media.selectApproved(manifest, options.items);
  for (const item of items) measure.refusePrivate(item);

  const started = new Date();
  const runFolder = path.join(options.work, `progress-${started.toISOString().replace(/[:.]/g, '-')}`);
  fs.mkdirSync(runFolder, { recursive: false });
  fs.mkdirSync(options.report, { recursive: true });
  const user = path.join(runFolder, 'user');
  fs.mkdirSync(user);
  const versionLine = childProcess.spawnSync(options.vsift, ['--version'], { encoding: 'utf8' }).stdout.trim();
  const run = runner(options.vsift, user, log);
  log('setup');
  const setup = campaign.setup(run, options);
  if (!setup.passed) {
    process.stderr.write(`setup failed: ${setup.findings.join('; ')}\n`);
    return 2;
  }

  const files = new Map();
  for (const item of items) {
    const file = path.join(options.media, item.id, item.file_name);
    const exists = fs.existsSync(file);
    const state = campaign.mediaState(item, { exists, size: exists ? fs.statSync(file).size : null, digest: (algorithm) => digestOf(file, algorithm) });
    if (state !== 'ready') throw new measure.ProgressMeasurementError(`${item.id}: the media file is ${state}; the measurement needs every item it names, fetched and matching its pin`);
    files.set(item.id, file);
  }

  const plan = measure.planMedia(items, options.prefixOf, options.prefixMinutes);
  const prefixes = [];
  const summaries = [];
  const comparisons = [];
  let allSealed = true;
  for (const [index, entry] of plan.entries()) {
    let file = files.get(entry.item_id);
    if (entry.kind === 'prefix') {
      const destination = path.join(runFolder, 'prefixes', `${entry.label}${path.extname(file)}`);
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      const cut = measure.cutPrefix({ ffmpeg: options.ffmpeg, ffprobe: options.ffprobe, source: file, destination, seconds: entry.minutes * 60, spawn: spawnTool, remove: (target) => fs.rmSync(target, { force: true }) });
      prefixes.push({ label: entry.label, item_id: entry.item_id, minutes: entry.minutes, method: cut.method, duration_s: Number(cut.duration_s.toFixed(1)) });
      log(`${entry.label}: cut by ${cut.method}, ${cut.duration_s.toFixed(1)} s`);
      file = destination;
    }
    const durationUs = Math.round(probeDuration(options.ffprobe, file) * measure.SECOND);
    const results = {};
    for (const [position, mode] of entry.modes.entries()) {
      const folder = path.join(runFolder, entry.label, mode);
      results[mode] = measure.measureRun({ run, log, clock, sleep, readRevision, makeDirectory: (target) => fs.mkdirSync(target, { recursive: true }), entry, mode, order: position === 0 ? 'first' : 'second', folder, file, durationUs });
      summaries.push(results[mode].summary);
      if (results[mode].summary.ended !== 'succeeded') allSealed = false;
    }
    if (results.quick.segments && results.watched.segments) {
      comparisons.push({ label: entry.label, quick_revision_id: results.quick.segments.revision_id, watched_revision_id: results.watched.segments.revision_id,
        ...measure.compareRevisions(results.quick.segments, results.watched.segments) });
    }
    log(`${entry.label} done (${index + 1} of ${plan.length})`);
  }

  const record = {
    schema: 'vsift-progress-measurement/1',
    binary: { version: versionLine, sha256: digestOf(options.vsift, 'sha256') },
    platform: `${process.platform} ${process.arch}, ${os.cpus().length} logical CPUs`,
    runner: campaign.runnerIdentity({ cpus: os.cpus(), cpuinfo: readCpuinfo() }),
    started: started.toISOString(),
    finished: new Date().toISOString(),
    items: items.map((item) => ({ id: item.id, title: item.title, attribution: item.attribution, sha256: item.sha256 })),
    prefixes,
    runs: summaries,
    comparisons,
  };
  fs.writeFileSync(path.join(options.report, 'progress.json'), `${JSON.stringify(record, null, 2)}\n`);
  fs.writeFileSync(path.join(options.report, 'progress.md'), measure.markdown(record));
  log(`report: ${path.join(options.report, 'progress.json')}`);
  log(allSealed ? 'every run sealed' : 'at least one run did not seal (see progress.md)');
  return allSealed ? 0 : 1;
}

try {
  process.exitCode = main();
} catch (error) {
  process.stderr.write(`${error instanceof media.RealMediaError ? `${error.code}: ` : ''}${error && error.message}\n`);
  process.exitCode = 2;
}
