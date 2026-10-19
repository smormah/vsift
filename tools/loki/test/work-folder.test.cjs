'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const work = require('../lib/work-folder.cjs');

/** A folder of the test's own, removed when the test ends. */
function scratch(t) {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'loki-work-test-'));
  t.after(() => fs.rmSync(folder, { recursive: true, force: true }));
  return folder;
}

/** A stand-in checkout of this repository with a Cargo target folder. */
function checkout(root) {
  const workspace = path.join(root, 'workspace');
  fs.mkdirSync(path.join(workspace, 'tools', 'loki', 'lib'), { recursive: true });
  fs.mkdirSync(path.join(workspace, 'target', 'debug'), { recursive: true });
  for (const file of ['Cargo.toml', 'rust-toolchain.toml', 'tools/loki/lib/work-folder.cjs', 'target/CACHEDIR.TAG', 'target/debug/x']) {
    fs.writeFileSync(path.join(workspace, file), '');
  }
  return workspace;
}

test('prepare makes the run\'s marked folder and points the temporary directory at it', (t) => {
  const root = scratch(t);
  const lines = work.prepare(root, '123');
  const folder = path.join(root, 'vsift-loki-123');
  assert.deepEqual(lines, [`TEMP=${folder}`, `TMP=${folder}`, `TMPDIR=${folder}`]);
  assert.equal(work.isOwned(folder, '123'), true);
  assert.deepEqual(work.prepare(root, '123'), lines, 'a re-run finds its own folder');
  fs.mkdirSync(path.join(root, 'vsift-loki-9'));
  assert.throws(() => work.prepare(root, '9'), /is not this run's/);
  assert.throws(() => work.prepare(root, '0'), /positive integer/);
  assert.throws(() => work.prepare(root, '1/../..'), /positive integer/);
  assert.throws(() => work.prepare('relative', '1'), /absolute/);
});

test('clean removes the run\'s folder and the target folder, and names what the tests left', (t) => {
  const root = scratch(t);
  const workspace = checkout(root);
  work.prepare(root, '7');
  const folder = path.join(root, 'vsift-loki-7');
  fs.mkdirSync(path.join(folder, 'vsift-session-x'));
  fs.mkdirSync(path.join(folder, 'other'));
  const report = work.clean(root, '7', workspace);
  assert.deepEqual(report.leftovers, ['vsift-session-x']);
  assert.deepEqual(report.removed, [folder, path.join(workspace, 'target')]);
  assert.equal(fs.existsSync(folder), false);
  assert.equal(fs.existsSync(path.join(workspace, 'target')), false);
  assert.equal(fs.existsSync(path.join(workspace, 'Cargo.toml')), true);
});

test('clean leaves alone anything it cannot identify as VSift\'s', (t) => {
  const root = scratch(t);
  // Another run's folder: unmarked, then with a marker that names another run.
  fs.mkdirSync(path.join(root, 'vsift-loki-8'));
  fs.writeFileSync(path.join(root, 'vsift-loki-8', 'keep'), '');
  let report = work.clean(root, '8', path.join(root, 'nowhere'));
  assert.deepEqual(report.removed, []);
  assert.equal(fs.existsSync(path.join(root, 'vsift-loki-8', 'keep')), true);
  fs.writeFileSync(path.join(root, 'vsift-loki-8', work.MARKER), '{"run":"80"}');
  assert.deepEqual(work.clean(root, '8', path.join(root, 'nowhere')).removed, []);

  // A target folder without Cargo's tag, or in a folder that is not this repository's checkout.
  const workspace = checkout(root);
  fs.rmSync(path.join(workspace, 'target', 'CACHEDIR.TAG'));
  assert.equal(work.isCargoTarget(workspace), false);
  fs.writeFileSync(path.join(workspace, 'target', 'CACHEDIR.TAG'), '');
  fs.rmSync(path.join(workspace, 'tools', 'loki', 'lib', 'work-folder.cjs'));
  assert.equal(work.isCargoTarget(workspace), false);
  assert.equal(work.isCargoTarget('relative'), false);
  report = work.clean(root, '8', workspace);
  assert.equal(fs.existsSync(path.join(workspace, 'target', 'debug', 'x')), true);
  assert.equal(report.skipped.length, 2);
});

test('a link in place of the run\'s folder is never followed', (t) => {
  const root = scratch(t);
  const outside = path.join(root, 'outside');
  fs.mkdirSync(outside);
  fs.writeFileSync(path.join(outside, work.MARKER), '{"run":"5"}');
  fs.writeFileSync(path.join(outside, 'precious'), '');
  const runner = path.join(root, 'runner-temp');
  fs.mkdirSync(runner);
  // A junction on Windows needs no privilege; elsewhere the type is ignored and it is a symbolic link.
  fs.symlinkSync(outside, path.join(runner, 'vsift-loki-5'), 'junction');
  const report = work.clean(runner, '5', path.join(root, 'nowhere'));
  assert.deepEqual(report.removed, []);
  assert.equal(fs.existsSync(path.join(outside, 'precious')), true);
  assert.throws(() => work.prepare(runner, '5'), /is not this run's/);
});
