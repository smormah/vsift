'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const assemble = require('../assemble-local-packages.cjs');
const { sha256Hex } = require('../lib/common.cjs');

const repository = path.join(__dirname, '..', '..', '..');
const manifest = JSON.parse(fs.readFileSync(path.join(repository, 'npm', 'vsift-cli', 'package.json'), 'utf8'));

test('the launcher manifest takes the higher version and pins every platform package to it', () => {
  const result = assemble.launcherManifest(manifest, '0.1.1-p14local.1');
  assert.equal(result.version, '0.1.1-p14local.1');
  assert.deepEqual(result.optionalDependencies, {
    '@vsift/darwin-arm64': '0.1.1-p14local.1',
    '@vsift/linux-x64': '0.1.1-p14local.1',
    '@vsift/win32-x64': '0.1.1-p14local.1',
  });
  assert.equal(result.name, 'vsift-cli');
  assert.equal(manifest.version, '0.1.0', 'the repository manifest is untouched');
});

test('a launcher manifest with a script or another dependency set is refused', () => {
  assert.throws(() => assemble.launcherManifest({ ...manifest, scripts: { postinstall: 'x' } }, '1.0.0'), /has scripts/);
  assert.throws(() => assemble.launcherManifest({ ...manifest, optionalDependencies: { '@vsift/linux-x64': '0.1.0' } }, '1.0.0'), /optional dependencies/);
});

test('the digests file holds each executable\'s size and SHA-256 in the format the launcher reads', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 digests '));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const binaries = {};
  for (const key of ['linux-x64', 'darwin-arm64', 'win32-x64']) {
    binaries[key] = path.join(directory, key);
    fs.writeFileSync(binaries[key], `binary of ${key}`);
  }
  const digests = assemble.platformDigests('0.1.1-p14local.1', binaries);
  const launcher = require(path.join(repository, 'npm', 'vsift-cli', 'lib', 'launcher.cjs'));
  assert.equal(digests.format, launcher.DIGESTS_FORMAT);
  assert.equal(digests.version, '0.1.1-p14local.1');
  assert.deepEqual(digests.packages['@vsift/win32-x64'], { file: 'vsift.exe', size: 'binary of win32-x64'.length, sha256: sha256Hex('binary of win32-x64') });
  assert.equal(digests.packages['@vsift/linux-x64'].file, 'vsift');
  assert.throws(() => assemble.platformDigests('1.0.0', { 'linux-arm64': binaries['linux-x64'] }), /unknown platform/);
});
