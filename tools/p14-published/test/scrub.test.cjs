'use strict';

// Tests of the scrubbed environment (lib/scrub.cjs): run with
// `node --test tools/p14-published/test/`.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const scrub = require('../lib/scrub.cjs');

const windows = process.platform === 'win32';
const exe = (name) => (windows ? `${name}.exe` : name);

function directory(t, files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 scrub ü '));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const [name, text] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), text);
    fs.chmodSync(path.join(root, name), 0o755);
  }
  return root;
}

test('a name is hidden as itself, with an executable extension, or as a versioned Python or Rust name', () => {
  const hidden = scrub.namesOf(['rust', 'git', 'python']);
  for (const name of ['git', 'cargo', 'rustc', 'rustup', 'python', 'python3', 'python3.12', 'pip3.12', 'cargo-clippy', 'rustfmt', 'rust-analyzer']) {
    assert.equal(scrub.isHidden(name, hidden, 'linux'), true, name);
  }
  for (const name of ['Git.EXE', 'git.cmd', 'PYTHON.EXE', 'Cargo.exe']) {
    assert.equal(scrub.isHidden(name, hidden, 'win32'), true, name);
  }
  for (const name of ['gitk', 'node', 'npm', 'gh', 'ffmpeg', 'rusty', 'pythonista', 'cargo']) {
    assert.equal(scrub.isHidden(name, ['git'], 'linux'), false, name);
  }
  assert.equal(scrub.isHidden('GIT', ['git'], 'linux'), false, 'POSIX names are case sensitive');
  assert.throws(() => scrub.namesOf(['perl']), /unknown toolchain group/);
});

test('resolution searches absolute entries only and finds the first match', (t) => {
  const first = directory(t, { [exe('tool')]: 'one' });
  const second = directory(t, { [exe('tool')]: 'two', [exe('other')]: '' });
  const value = scrub.joinPath(['', '.', 'relative', first, second]);
  assert.equal(scrub.resolveOnPath('tool', value), path.join(first, exe('tool')));
  assert.equal(scrub.resolveOnPath('other', value), path.join(second, exe('other')));
  assert.equal(scrub.resolveOnPath('absent', value), null);
  assert.equal(scrub.resolveOnPath('tool', scrub.joinPath(['', '.', 'relative'])), null, 'the current directory is never searched');
});

test('a directory without a hidden program is kept as it is', (t) => {
  const kept = directory(t, { [exe('node')]: '', [exe('npm')]: '' });
  const farm = path.join(os.tmpdir(), `p14-farm-${process.pid}-a`);
  t.after(() => fs.rmSync(farm, { recursive: true, force: true }));
  const result = scrub.scrubPath(kept, scrub.namesOf(['git']), { farm });
  assert.equal(result.path, kept);
  assert.deepEqual(result.kept, [kept]);
  assert.deepEqual(result.dropped, []);
  assert.deepEqual(result.filtered, []);
});

test('a hidden program no longer resolves, and relative and empty entries are dropped', (t) => {
  const own = directory(t, { [exe('git')]: '', [exe('cargo')]: '' });
  const shared = directory(t, { [exe('git')]: '', [exe('tar')]: '', [windows ? 'python3.exe' : 'python3.12']: '' });
  const plain = directory(t, { [exe('node')]: '' });
  const farm = path.join(os.tmpdir(), `p14-farm-${process.pid}-b`);
  t.after(() => fs.rmSync(farm, { recursive: true, force: true }));
  const hidden = scrub.namesOf(['rust', 'git', 'python']);
  const original = scrub.joinPath(['.', '', 'relative', own, shared, plain]);
  const scrubbed = scrub.scrubbedEnvironment({ [windows ? 'Path' : 'PATH']: original, KEEP: 'yes' }, hidden, { farm });
  const value = scrubbed.env[scrubbed.key];
  for (const name of hidden) {
    assert.equal(scrub.resolveOnPath(name, value), null, name);
  }
  assert.equal(scrub.resolveOnPath('node', value), path.join(plain, exe('node')), 'an unrelated directory is untouched');
  assert.equal(scrubbed.env.KEEP, 'yes');
  assert.ok(!scrub.splitPath(value).includes('.') && !scrub.splitPath(value).includes('') && !scrub.splitPath(value).includes('relative'));
  if (windows) {
    assert.deepEqual(scrubbed.dropped.filter(path.isAbsolute).sort(), [own, shared].sort(), 'a directory holding a hidden program is dropped');
  } else {
    // A shared directory keeps its other programs, through links.
    const link = scrub.resolveOnPath('tar', value);
    assert.ok(link !== null && link.startsWith(farm), link);
    assert.equal(fs.readFileSync(link, 'utf8'), '');
    assert.equal(scrubbed.filtered.length, 2);
  }
});

test('the search path key keeps the case the environment used', () => {
  assert.equal(scrub.pathKey({ Path: 'a' }), 'Path');
  assert.equal(scrub.pathKey({ PATH: 'a' }), 'PATH');
  assert.equal(scrub.pathKey({}), 'PATH');
  const env = scrub.withPathPrefix({ Path: scrub.joinPath([os.tmpdir()]) }, [path.join(os.tmpdir(), 'first')]);
  assert.equal(Object.keys(env).length, 1);
  assert.equal(scrub.splitPath(env.Path)[0], path.join(os.tmpdir(), 'first'));
});

test('the Node.js runtime names are the ones a no-Node step must not find', () => {
  for (const name of ['node', 'npm', 'npx', 'bun', 'yarn', 'pnpm']) {
    assert.ok(scrub.NODE_RUNTIME.includes(name), name);
  }
});
