'use strict';

// Tests of the hostile names and arguments (lib/hostile.cjs).

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const hostile = require('../lib/hostile.cjs');

test('every platform has the classes the plan names: quotes, newlines, leading dashes, spaces and Unicode', () => {
  const labels = (platform) => hostile.hostileFileNames(platform).map(([label]) => label).join(' | ');
  for (const platform of ['linux', 'darwin']) {
    const all = labels(platform);
    for (const word of ['single quote', 'double quote', 'newline', 'leading dash', 'spaces', 'accents and CJK']) {
      assert.ok(all.includes(word), `${platform}: ${word}`);
    }
  }
  const windows = labels('win32');
  for (const word of ['single quote', 'leading dash', 'spaces', 'accents and CJK']) {
    assert.ok(windows.includes(word), `win32: ${word}`);
  }
  assert.ok(!windows.includes('double quote') && !windows.includes('newline'), 'Windows cannot name a file with a quote or a newline');
});

test('names are unique, never a path and valid on the platform that creates them', () => {
  for (const platform of ['linux', 'darwin', 'win32']) {
    const names = hostile.hostileFileNames(platform).map(([, name]) => name);
    assert.equal(new Set(names).size, names.length, platform);
    for (const name of names) {
      assert.ok(!name.includes('/') && name !== '.' && name !== '..', `${platform}: ${JSON.stringify(name)}`);
      assert.ok(name.endsWith('.md'));
      if (platform === 'win32') {
        assert.ok(!/[<>:"/\\|?*\u0000-\u001f]/.test(name), `${platform}: ${JSON.stringify(name)}`);
      }
    }
  }
});

test('the names can be created and read back on this machine', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 hostile ü '));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  for (const [label, name] of hostile.hostileFileNames(process.platform)) {
    fs.writeFileSync(path.join(directory, name), hostile.DRAFT);
    assert.equal(fs.readFileSync(path.join(directory, name), 'utf8'), hostile.DRAFT, label);
  }
});

test('every injecting argument would create the marker file if a shell ran it', () => {
  for (const platform of ['linux', 'win32']) {
    const arguments_ = hostile.hostileArguments(platform);
    assert.ok(arguments_.length >= 10, platform);
    const spelling = arguments_.filter(([, argument]) => argument.includes(hostile.MARKER));
    assert.ok(spelling.length >= 6, `${platform}: ${spelling.length} arguments spell a command`);
    assert.equal(new Set(arguments_.map(([, argument]) => argument)).size, arguments_.length, platform);
  }
  assert.ok(hostile.hostileArguments('linux').some(([, argument]) => argument.includes('\n')), 'a newline is among them');
});
