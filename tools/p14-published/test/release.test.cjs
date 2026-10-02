'use strict';

// Tests of the release helpers (lib/release.cjs) and the shared helpers
// (lib/common.cjs): version precedence, the ten release files, SHA256SUMS and
// the environment given to `gh`.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const common = require('../lib/common.cjs');
const release = require('../lib/release.cjs');

test('version precedence follows SemVer', () => {
  const ascending = ['0.0.0', '0.1.0-alpha', '0.1.0-alpha.1', '0.1.0-alpha.beta', '0.1.0-beta', '0.1.0-beta.2', '0.1.0-beta.11', '0.1.0-rc.1', '0.1.0', '0.1.1-p14local.1', '0.1.1', '0.2.0-rc.1', '0.2.0', '1.0.0'];
  for (let low = 0; low < ascending.length; low += 1) {
    for (let high = 0; high < ascending.length; high += 1) {
      const expected = low === high ? 0 : low < high ? -1 : 1;
      assert.equal(release.compareVersions(ascending[low], ascending[high]), expected, `${ascending[low]} against ${ascending[high]}`);
    }
  }
  assert.equal(release.highestVersion(['0.0.0', '0.2.0-rc.1', '0.1.0', '0.2.0-rc.2']), '0.2.0-rc.2');
  assert.throws(() => release.highestVersion([]), /no version/);
});

test('only plain versions are accepted', () => {
  for (const bad of ['v0.1.0', '0.1', '0.1.0.0', '01.1.0', '0.1.0+build', 'next', '', '0.1.0-', '0.1.0-01', '../0.1.0', '0.1.0 ']) {
    assert.throws(() => release.parseVersion(bad), /not a version/, JSON.stringify(bad));
  }
  assert.equal(release.isStable('0.2.0'), true);
  assert.equal(release.isStable('0.2.0-rc.1'), false);
  assert.equal(release.tagOf('0.1.0'), 'v0.1.0');
  assert.throws(() => release.tagOf('latest'));
});

test('a release has ten files and four tarballs', () => {
  const assets = release.expectedAssets('0.1.0');
  assert.equal(assets.length, 10);
  assert.deepEqual(assets, [...assets].sort());
  assert.ok(assets.includes('SHA256SUMS'));
  assert.ok(assets.includes('vsift-0.1.0-x86_64-pc-windows-msvc.tar.gz'));
  assert.ok(assets.includes('vsift-0.1.0-aarch64-apple-darwin.cdx.json'));
  assert.ok(assets.includes('vsift-0.1.0-x86_64-unknown-linux-gnu.THIRD-PARTY-NOTICES.txt'));
  assert.deepEqual(
    common.PACKAGES.map((name) => release.tarballName(name, '0.1.0')),
    ['vsift-cli-0.1.0.tgz', 'vsift-win32-x64-0.1.0.tgz', 'vsift-darwin-arm64-0.1.0.tgz', 'vsift-linux-x64-0.1.0.tgz'],
  );
});

test('SHA256SUMS is read strictly and every archive is held to its digest', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 sums '));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const lines = [];
  for (const target of release.ARCHIVE_TARGETS) {
    const name = release.archiveName('0.1.0', target);
    fs.writeFileSync(path.join(directory, name), `archive ${target}`);
    lines.push(`${common.sha256Hex(`archive ${target}`)}  ${name}`);
  }
  fs.writeFileSync(path.join(directory, 'SHA256SUMS'), `${lines.join('\n')}\n`);
  assert.equal(release.checkChecksums(directory, '0.1.0'), 3);

  fs.appendFileSync(path.join(directory, release.archiveName('0.1.0', release.ARCHIVE_TARGETS[0])), 'x');
  assert.throws(() => release.checkChecksums(directory, '0.1.0'), /is not the listed/);

  fs.writeFileSync(path.join(directory, 'SHA256SUMS'), `${lines.slice(0, 2).join('\n')}\n`);
  assert.throws(() => release.checkChecksums(directory, '0.1.0'), /expected exactly/);
  assert.throws(() => release.parseChecksums('not a line\n'), /is not "<sha256>  <file>"/);
  assert.throws(() => release.parseChecksums(`${lines[0]}\n${lines[0]}\n`), /twice/);
  assert.equal(release.parseChecksums(`${'a'.repeat(64)} *star.bin\n`).get('star.bin'), 'a'.repeat(64));
});

test('the environments given to children hold no credential and no package-manager setting', () => {
  const source = {
    PATH: '/usr/bin',
    GITHUB_TOKEN: 'secret-token',
    GH_TOKEN: 'another',
    NODE_AUTH_TOKEN: 'npm-secret',
    ACTIONS_ID_TOKEN_REQUEST_TOKEN: 'oidc',
    ACTIONS_RUNTIME_TOKEN: 'runtime',
    npm_config_registry: 'http://elsewhere/',
    PNPM_HOME: '/x',
    YARN_NPM_AUTH_TOKEN: 'yarn-secret',
    BUN_CONFIG_REGISTRY: 'http://elsewhere/',
    NODE_OPTIONS: '--require x',
    KEEP: 'yes',
  };
  const clean = common.cleanEnvironment(source);
  assert.deepEqual(Object.keys(clean).sort(), ['KEEP', 'PATH']);
  const forGh = common.githubEnvironment(source, source);
  assert.equal(forGh.GH_TOKEN, 'another', 'gh gets the job token and nothing else of a secret kind');
  assert.equal(forGh.NODE_AUTH_TOKEN, undefined);
  assert.equal(forGh.GITHUB_TOKEN, undefined);
  assert.equal(forGh.KEEP, 'yes');
  const none = common.githubEnvironment({ PATH: 'x' }, {});
  assert.equal(none.GH_TOKEN, undefined);
});

test('arguments are parsed strictly', () => {
  const spec = { required: ['version'], optional: ['work'], flags: ['quiet'] };
  assert.deepEqual(common.parseArguments(['--version', '0.1.0', '--quiet'], spec), { version: '0.1.0', quiet: true });
  assert.throws(() => common.parseArguments([], spec), /--version is required/);
  assert.throws(() => common.parseArguments(['--version'], spec), /needs a value/);
  assert.throws(() => common.parseArguments(['--version', '--work'], spec), /needs a value/);
  assert.throws(() => common.parseArguments(['--version', '1', '--other', 'x'], spec), /unknown option/);
  assert.throws(() => common.parseArguments(['version'], spec), /unexpected argument/);
});

test('two directory trees compare by file names and bytes', (t) => {
  const make = (files) => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 tree '));
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    for (const [name, text] of Object.entries(files)) {
      fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
      fs.writeFileSync(path.join(root, name), text);
    }
    return root;
  };
  const left = make({ 'a.md': 'one', 'sub/b.md': 'two' });
  assert.deepEqual(common.compareTrees(left, make({ 'a.md': 'one', 'sub/b.md': 'two' })), { equal: true, files: 2, problems: [] });
  const different = common.compareTrees(left, make({ 'a.md': 'ONE', 'c.md': 'x' }));
  assert.equal(different.equal, false);
  assert.equal(different.problems.length, 3);
});

test('a recorder reports checks, observations and failures as a table and sets the exit status', async () => {
  const recorder = new common.Recorder();
  const quiet = process.stdout.write;
  process.stdout.write = () => true;
  try {
    assert.equal(await recorder.check('passes', () => 'fine'), true);
    recorder.observe('observed', 'a | b');
    assert.equal(await recorder.check('fails', () => { throw new common.QualificationError('went wrong'); }), false);
  } finally {
    process.stdout.write = quiet;
  }
  const text = recorder.markdown('Title');
  assert.match(text, /\| passes \| pass \| fine \|/);
  assert.match(text, /\| observed \| observed \| a \\\| b \|/);
  assert.match(text, /\| fails \| \*\*FAIL\*\* \| went wrong \|/);
  assert.equal(recorder.ok, false);
});
