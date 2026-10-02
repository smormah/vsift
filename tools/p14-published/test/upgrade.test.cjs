'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const upgrade = require('../lib/upgrade.cjs');

test('the folders are the ones install.md section 8 lists, from the user state', () => {
  assert.deepEqual(upgrade.vsiftFolders('win32', { LOCALAPPDATA: 'C:\\Users\\x\\AppData\\Local' }), {
    config: path.join('C:\\Users\\x\\AppData\\Local', 'vsift'),
    sessions: path.join('C:\\Users\\x\\AppData\\Local', 'VSift-sessions'),
    managed: null,
  });
  assert.deepEqual(upgrade.vsiftFolders('darwin', { HOME: '/Users/x' }), {
    config: path.join('/Users/x', 'Library', 'Application Support', 'vsift'),
    sessions: path.join('/Users/x', 'Library', 'Caches', 'VSift-sessions'),
    managed: null,
  });
  const linux = upgrade.vsiftFolders('linux', { HOME: '/home/x' });
  assert.equal(linux.config, path.join('/home/x', '.config', 'vsift'));
  assert.equal(linux.sessions, path.join('/home/x', '.cache', 'vsift-sessions'));
  assert.equal(linux.managed, path.join('/home/x', '.local/share', 'vsift', 'managed-v1'));
  const xdg = upgrade.vsiftFolders('linux', { HOME: '/home/x', XDG_CONFIG_HOME: '/c', XDG_CACHE_HOME: '/k', XDG_DATA_HOME: '/d', ...{} });
  if (path.isAbsolute('/c')) {
    assert.deepEqual([xdg.config, xdg.sessions, xdg.managed], [path.join('/c', 'vsift'), path.join('/k', 'vsift-sessions'), path.join('/d', 'vsift', 'managed-v1')]);
  }
});

test('anything named for vsift under a root is found once, and an empty scope folder can be ignored', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 left '));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, 'a', 'vsift', 'deep'), { recursive: true });
  fs.writeFileSync(path.join(root, 'a', 'vsift', 'deep', 'vsift-file'), '');
  fs.mkdirSync(path.join(root, 'node_modules', '@vsift'), { recursive: true });
  fs.mkdirSync(path.join(root, 'b'), { recursive: true });
  fs.writeFileSync(path.join(root, 'b', 'VSift-sessions'), '');
  fs.writeFileSync(path.join(root, 'b', 'other'), '');
  const all = upgrade.findVsiftEntries(root).map((file) => path.relative(root, file)).sort();
  assert.deepEqual(all, [path.join('a', 'vsift'), path.join('b', 'VSift-sessions'), path.join('node_modules', '@vsift')]);
  const tidy = upgrade.findVsiftEntries(root, { ignoreEmpty: true }).map((file) => path.relative(root, file)).sort();
  assert.deepEqual(tidy, [path.join('a', 'vsift'), path.join('b', 'VSift-sessions')]);
  assert.deepEqual(upgrade.findVsiftEntries(path.join(root, 'missing')), []);
});

test('an older answer must still be inside a newer one: additive members are fine, nothing else changes', () => {
  const before = { status: 'complete', data: { items: [{ id: 'a', n: 1 }], cursor: null }, warnings: [] };
  assert.deepEqual(upgrade.subsetProblems(before, JSON.parse(JSON.stringify(before))), []);
  const grown = { status: 'complete', data: { items: [{ id: 'a', n: 1, extra: true }], cursor: null, more: 1 }, warnings: [], added: {} };
  assert.deepEqual(upgrade.subsetProblems(before, grown), []);
  assert.deepEqual(upgrade.subsetProblems(before, { ...grown, status: 'partial' }), ['/status: "complete" became "partial"']);
  assert.deepEqual(upgrade.subsetProblems(before, { status: 'complete', data: { items: [], cursor: null }, warnings: [] }), ['/data/items: an array of 1 became 0 items']);
  assert.deepEqual(upgrade.subsetProblems(before, { status: 'complete', data: { items: [{ id: 'a' }], cursor: null }, warnings: [] }), ['/data/items/0/n: removed']);
  assert.deepEqual(upgrade.subsetProblems(before, { status: 'complete', data: 'gone', warnings: [] }), ['/data: an object became "gone"']);
  assert.deepEqual(upgrade.subsetProblems(null, null), []);
  assert.equal(upgrade.subsetProblems(1, 2).length, 1);
});
