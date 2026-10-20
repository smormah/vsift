#!/usr/bin/env node
'use strict';

// #363: the opt-in real-media campaign (docs/planning/real-media-set.md section 7).
// It drives the `vsift` it is given, by absolute path, over the approved items of
// the real-media set that real-media.cjs fetched, and checks properties of the
// answers, not words: no failure on valid speech, coverage that adds up, words not
// lost where chunks meet, repeats at seams, how far two chunk plans disagree,
// candidates, frames, a crop, an audio clip, the handoff check, and timings.
//
//   node real-media-campaign.cjs --vsift <exe> --media <dir> --work <dir>
//     --ffmpeg <exe> --ffprobe <exe> --whisper <exe> --model <file>
//     [--item RMnn]... [--manifest <file>] [--available-only]
//   node real-media-campaign.cjs --vsift <exe> --private-manifest <file> --work <dir>
//     --ffmpeg <exe> --ffprobe <exe> --whisper <exe> --model <file> [--item Pnn]...
//
// Every path is absolute. <dir> of --media is the destination real-media.cjs
// fetched into; each file is checked against its pinned size and SHA-256 before
// it is used. Under --work the run makes a new folder `run-<time>` and never
// deletes anything: per-user folders and a session root per item live there, and
// so do the audio clips kept for listening (`<item>/listen/`, copies of the
// item's audio under its licence: never commit or upload them). The report is
// `report.json` and `summary.md` in the run folder; the process exits 1 when an
// item fails a property check (advisory evidence: a finding becomes an issue).
// With --available-only (the hosted run) an approved item whose file is not
// there is reported "not yet mirrored" and is not a failure; a run in which no
// item could be investigated fails. An item not yet pinned in the manifest is
// held to the checksum its source publishes.
//
// A private manifest (section 13) lists the maintainer's own recordings, kept
// outside every repository with the files beside it. It runs only on a local
// machine (it is refused when CI or GITHUB_ACTIONS is set), and its items are
// reported by measurements and failure codes only: no title, name, file name or
// text reaches a report or a log line, and no listening clip is kept.
//
// Node.js 22, no dependency.

const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const media = require('./lib/real-media.cjs');
const campaign = require('./lib/real-media-campaign.cjs');

const STEP_TIMEOUT_MS = 4 * 60 * 60 * 1000;
const REPOSITORY = path.resolve(__dirname, '..', '..');

function parse(argv) {
  const options = { items: [], manifest: path.join(__dirname, 'real-media', 'manifest.json'), privateManifest: null, availableOnly: false };
  const names = { '--vsift': 'vsift', '--media': 'media', '--work': 'work', '--ffmpeg': 'ffmpeg', '--ffprobe': 'ffprobe',
    '--whisper': 'whisper', '--model': 'model', '--manifest': 'manifest', '--private-manifest': 'privateManifest' };
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    if (flag === '--available-only') {
      options.availableOnly = true;
      index -= 1;
      continue;
    }
    const value = argv[index + 1];
    if (value === undefined) throw new Error(`${flag} needs a value`);
    if (flag === '--item') options.items.push(value);
    else if (names[flag]) options[names[flag]] = value;
    else throw new Error(`unknown option ${flag}`);
  }
  const required = ['vsift', 'work', 'ffmpeg', 'ffprobe', 'whisper', 'model', ...(options.privateManifest ? ['privateManifest'] : ['media'])];
  for (const key of required) {
    if (!options[key] || !path.isAbsolute(options[key])) throw new Error(`--${key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)} must be an absolute path`);
  }
  return options;
}

function sha256(file) {
  return digestOf(file, 'sha256');
}

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

/** Runs `vsift` with the run's per-user folders and returns its answer, timed. */
function runner(vsift, user, log) {
  const env = { ...process.env, LOCALAPPDATA: user, APPDATA: path.join(user, 'roaming'), XDG_CONFIG_HOME: path.join(user, 'config'),
    XDG_CACHE_HOME: path.join(user, 'cache'), XDG_DATA_HOME: path.join(user, 'data'), HOME: user };
  return (step, args, { root = null, input = undefined } = {}) => {
    const full = [...(root ? ['--session-root', root] : []), ...args, '--json'];
    const started = process.hrtime.bigint();
    const result = childProcess.spawnSync(vsift, full, { encoding: 'utf8', env, input, maxBuffer: 1 << 29, timeout: STEP_TIMEOUT_MS, windowsHide: true });
    const seconds = Number(process.hrtime.bigint() - started) / 1e9;
    let json = null;
    try {
      json = JSON.parse(result.stdout);
    } catch {
      json = null;
    }
    log(`  ${step}: exit ${result.status} in ${seconds.toFixed(1)} s${json && json.error ? ` (${json.error.code})` : ''}`);
    return { step, exit: result.status, seconds, json };
  };
}

function main() {
  const options = parse(process.argv.slice(2));
  const log = (line) => process.stdout.write(`${line}\n`);
  let selected;
  let fileOf;
  if (options.privateManifest) {
    media.refuseHostedPrivateRun(process.env);
    const manifest = media.readPrivateManifest(options.privateManifest, REPOSITORY);
    selected = media.selectApproved(manifest, options.items);
    const folder = path.dirname(fs.realpathSync.native(options.privateManifest));
    fileOf = (item) => path.join(folder, item.file_name);
  } else {
    const manifest = media.readManifest(path.resolve(options.manifest));
    selected = media.selectApproved(manifest, options.items);
    fileOf = (item) => path.join(options.media, item.id, item.file_name);
  }
  const started = new Date();
  const runFolder = path.join(options.work, `run-${started.toISOString().replace(/[:.]/g, '-')}`);
  fs.mkdirSync(runFolder, { recursive: false });
  const user = path.join(runFolder, 'user');
  fs.mkdirSync(user);
  const versionLine = childProcess.spawnSync(options.vsift, ['--version'], { encoding: 'utf8' }).stdout.trim();
  const run = runner(options.vsift, user, log);
  log('setup');
  const result = {
    binary: { version: versionLine, sha256: sha256(options.vsift) },
    platform: `${process.platform} ${process.arch}, ${os.cpus().length} logical CPUs`,
    started: started.toISOString(),
    setup: campaign.setup(run, options),
    items: [],
  };
  let keptCrops = 0;
  for (const item of selected) {
    const file = fileOf(item);
    log(item.private ? `${item.id} (private)` : `${item.id}: ${item.title}`);
    const folder = path.join(runFolder, item.id);
    fs.mkdirSync(folder);
    const exists = fs.existsSync(file);
    const state = campaign.mediaState(item, { exists, size: exists ? fs.statSync(file).size : null, digest: (algorithm) => digestOf(file, algorithm) });
    if (state === 'missing' && options.availableOnly && !item.private) {
      result.items.push({ id: item.id, title: item.title, not_yet_mirrored: true, verdict: { passed: true, findings: [] } });
      log(`  ${item.id}: not yet mirrored (not a failure)`);
      continue;
    }
    if (state !== 'ready') {
      const finding = item.private ? 'the private file is missing or differs from its manifest entry' : 'the media file is missing or differs from its pin: fetch it with real-media.cjs';
      result.items.push(item.private
        ? { id: item.id, private: true, verdict: { passed: false, failed_checks: [{ check: 'media', findings: 1 }] }, failure_codes: [], measurements: {} }
        : { id: item.id, title: item.title, attribution: item.attribution, measurements: {}, checks: {}, verdict: { passed: false, findings: [finding] } });
      log(`  ${item.id}: FAIL: ${finding}`);
      continue;
    }
    // Crops kept for a person (#173): at most KEPT_CROPS_PER_ITEM of an item, KEPT_CROPS_PER_RUN in the run.
    const keepCrops = Math.max(0, campaign.KEPT_CROPS_PER_RUN - keptCrops);
    const report = campaign.investigate(item, file, run, { folder, keepCrops });
    keptCrops += ((report.measurements.small_text && report.measurements.small_text.kept_crops) || []).length;
    if (item.private) {
      const kept = campaign.privateReport(report);
      result.items.push(kept);
      log(`  ${item.id}: ${kept.verdict.passed ? 'pass' : `FAIL in ${kept.verdict.failed_checks.map((entry) => entry.check).join(', ')}`}`);
    } else {
      report.sha256 = item.sha256;
      report.verdict = require('./lib/real-media-checks.cjs').itemVerdict(report.checks);
      result.items.push(report);
      log(`  ${item.id}: ${report.verdict.passed ? 'pass' : `FAIL: ${report.verdict.findings.join('; ')}`}`);
    }
  }
  result.finished = new Date().toISOString();
  const ran = result.items.filter((item) => !item.not_yet_mirrored);
  // A run in which nothing could be investigated proves nothing: it fails.
  result.passed = result.setup.passed && ran.length > 0 && ran.every((item) => item.verdict.passed);
  fs.writeFileSync(path.join(runFolder, 'report.json'), `${JSON.stringify(result, null, 2)}\n`);
  fs.writeFileSync(path.join(runFolder, 'summary.md'), campaign.markdown(result));
  log(`report: ${path.join(runFolder, 'report.json')}`);
  log(result.passed ? 'every item passed its property checks' : 'at least one property check failed (advisory: see summary.md)');
  return result.passed ? 0 : 1;
}

try {
  process.exitCode = main();
} catch (error) {
  process.stderr.write(`${error instanceof media.RealMediaError ? `${error.code}: ` : ''}${error && error.message}\n`);
  process.exitCode = 2;
}
