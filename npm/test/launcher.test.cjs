'use strict';

// Tests of the npm launcher (P13 PR 9): run with `node --test npm/test/`.
//
// The selection and checks run against fake package layouts for every
// target. The end-to-end tests lay out node_modules as a package manager
// would, with a copy of the Node.js executable standing in for the native
// `vsift` executable (it runs `fixtures/fake-vsift.cjs`), and run the real
// launcher: argument, stream, exit status and signal forwarding.

const test = require('node:test');
const assert = require('node:assert/strict');
const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const launcherSource = path.join(__dirname, '..', 'vsift');
const launcher = require(path.join(launcherSource, 'lib', 'launcher.cjs'));
const fakeScript = path.join(__dirname, 'fixtures', 'fake-vsift.cjs');
const sendConsoleControl = path.join(__dirname, '..', '..', 'tools', 'send-console-ctrl.ps1');
const manifest = JSON.parse(fs.readFileSync(path.join(launcherSource, 'package.json'), 'utf8'));
const VERSION = manifest.version;
const hostTarget = launcher.TARGETS[`${process.platform} ${process.arch}`];
const windows = process.platform === 'win32';
// A Windows console process ended by Ctrl-C or Ctrl-Break exits with this.
const STATUS_CONTROL_C_EXIT = 0xc000013a;

function sha256(bytes) {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

/**
 * Lays out node_modules the way a package manager does, under a directory
 * whose name has spaces and non-ASCII letters: the launcher from source and
 * `target`'s platform package holding `executable` (bytes, or a file to copy).
 */
function makeLayout(t, { target, executable, platformVersion = VERSION, editDigests = (digests) => digests }) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift launcher ü 日本 '));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const launcherRoot = path.join(root, 'node_modules', 'vsift');
  fs.mkdirSync(path.join(launcherRoot, 'bin'), { recursive: true });
  fs.mkdirSync(path.join(launcherRoot, 'lib'), { recursive: true });
  for (const file of ['package.json', path.join('bin', 'vsift.cjs'), path.join('lib', 'launcher.cjs')]) {
    fs.copyFileSync(path.join(launcherSource, file), path.join(launcherRoot, file));
  }
  const packageRoot = path.join(root, 'node_modules', ...target.packageName.split('/'));
  fs.mkdirSync(packageRoot, { recursive: true });
  fs.writeFileSync(
    path.join(packageRoot, 'package.json'),
    JSON.stringify({ name: target.packageName, version: platformVersion }),
  );
  const executablePath = path.join(packageRoot, target.executable);
  if (typeof executable === 'string') {
    fs.copyFileSync(executable, executablePath);
  } else {
    fs.writeFileSync(executablePath, executable);
  }
  fs.chmodSync(executablePath, 0o755);
  const bytes = fs.readFileSync(executablePath);
  const digests = {
    format: launcher.DIGESTS_FORMAT,
    version: VERSION,
    packages: { [target.packageName]: { file: target.executable, size: bytes.length, sha256: sha256(bytes) } },
  };
  fs.writeFileSync(path.join(launcherRoot, 'platform-digests.json'), JSON.stringify(editDigests(digests)));
  return {
    root,
    launcherRoot,
    packageRoot,
    executablePath,
    script: path.join(launcherRoot, 'bin', 'vsift.cjs'),
    resolve: (request) => require.resolve(request, { paths: [launcherRoot] }),
  };
}

function prepareFor(layout, target) {
  const [platform, arch] = Object.keys(launcher.TARGETS).find((key) => launcher.TARGETS[key] === target).split(' ');
  return launcher.prepare(layout.launcherRoot, platform, arch, layout.resolve);
}

function assertFailure(action, exitCode, ...fragments) {
  assert.throws(action, (error) => {
    assert.ok(error instanceof launcher.LauncherFailure, `not a LauncherFailure: ${error}`);
    assert.equal(error.exitCode, exitCode);
    for (const fragment of fragments) {
      assert.ok(error.message.includes(fragment), `message lacks ${fragment}: ${error.message}`);
    }
    return true;
  });
}

test('the manifest lists every target as an exact-version optional dependency', () => {
  const expected = Object.fromEntries(
    Object.values(launcher.TARGETS)
      .map((target) => [target.packageName, VERSION])
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  assert.deepEqual(manifest.optionalDependencies, expected);
  assert.equal(manifest.scripts, undefined);
  assert.deepEqual(manifest.bin, { vsift: 'bin/vsift.cjs' });
});

test('an unsupported platform names every supported target', () => {
  for (const [platform, arch] of [['linux', 'arm64'], ['darwin', 'x64'], ['win32', 'arm64'], ['freebsd', 'x64']]) {
    assertFailure(
      () => launcher.selectTarget(platform, arch),
      launcher.EXIT_NOT_INSTALLED,
      `${platform} ${arch}`,
      '@vsift/win32-x64',
      '@vsift/darwin-arm64',
      '@vsift/linux-x64',
    );
  }
});

for (const target of Object.values(launcher.TARGETS)) {
  test(`${target.packageName}: the matching package is found and checked`, (t) => {
    const layout = makeLayout(t, { target, executable: Buffer.from('native executable') });
    // require.resolve answers real paths (macOS keeps its temporary folder behind /var).
    assert.equal(fs.realpathSync(prepareFor(layout, target).executablePath), fs.realpathSync(layout.executablePath));
  });

  test(`${target.packageName}: a missing package names it and how to reinstall`, (t) => {
    const layout = makeLayout(t, { target, executable: Buffer.from('native executable') });
    fs.rmSync(layout.packageRoot, { recursive: true });
    assertFailure(
      () => prepareFor(layout, target),
      launcher.EXIT_NOT_INSTALLED,
      `${target.packageName}@${VERSION} is not installed`,
      '--omit=optional',
      `Reinstall vsift@${VERSION}`,
    );
  });

  test(`${target.packageName}: another version is refused`, (t) => {
    const layout = makeLayout(t, { target, executable: Buffer.from('native executable'), platformVersion: '9.9.9' });
    assertFailure(() => prepareFor(layout, target), launcher.EXIT_REFUSED, '9.9.9', VERSION);
  });
}

test('an executable that differs from its recorded digest is refused', (t) => {
  const target = launcher.TARGETS['linux x64'];
  const refusals = [
    ['same size, other bytes', (layout) => fs.writeFileSync(layout.executablePath, 'native executablf')],
    ['another size', (layout) => fs.appendFileSync(layout.executablePath, '\n')],
    ['missing', (layout) => fs.rmSync(layout.executablePath)],
  ];
  for (const [, damage] of refusals) {
    const layout = makeLayout(t, { target, executable: Buffer.from('native executable') });
    damage(layout);
    assertFailure(() => prepareFor(layout, target), launcher.EXIT_REFUSED, 'digest', target.packageName);
  }
});

test('a damaged digest file is refused', (t) => {
  const target = launcher.TARGETS['darwin arm64'];
  const edits = [
    (digests) => ({ ...digests, format: 'something-else/1' }),
    (digests) => ({ ...digests, version: '9.9.9' }),
    (digests) => ({ ...digests, packages: {} }),
    (digests) => ({ ...digests, packages: { [target.packageName]: { file: 'other', size: 1, sha256: '0'.repeat(64) } } }),
    (digests) => ({ ...digests, packages: { [target.packageName]: { file: target.executable, size: 17, sha256: 'Z'.repeat(64) } } }),
    () => [],
    () => null,
  ];
  for (const editDigests of edits) {
    const layout = makeLayout(t, { target, executable: Buffer.from('native executable'), editDigests });
    assertFailure(() => prepareFor(layout, target), launcher.EXIT_REFUSED, 'Reinstall vsift');
  }
  const layout = makeLayout(t, { target, executable: Buffer.from('native executable') });
  fs.writeFileSync(path.join(layout.launcherRoot, 'platform-digests.json'), '{ not json');
  assertFailure(() => prepareFor(layout, target), launcher.EXIT_REFUSED, 'platform digests');
});

// End-to-end: the real launcher runs a stand-in executable on this host.

const endToEnd = { skip: hostTarget === undefined ? `no VSift target for ${process.platform} ${process.arch}` : false };
const posix = { skip: endToEnd.skip || (windows ? 'POSIX signals' : false) };
const windowsOnly = { skip: endToEnd.skip || (windows ? false : 'Windows only') };

function runLauncher(layout, args, options = {}) {
  return childProcess.spawnSync(process.execPath, [layout.script, fakeScript, ...args], {
    encoding: 'utf8',
    timeout: 60_000,
    ...options,
  });
}

function assertNoStackTrace(stderr) {
  assert.doesNotMatch(stderr, /^\s+at .+:\d+:\d+\)?$/m);
  assert.doesNotMatch(stderr, /node:internal/);
}

test('arguments reach the executable exactly, spaces and Unicode included', endToEnd, (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  const args = ['a b', 'ü 日本 🎞', '', '--json', 'x"y', "it's", 'C:\\dir with space\\', '%PATH%', '$HOME', 'a&b|c<d>e^f', '*'];
  const result = runLauncher(layout, ['args', ...args]);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), args);
});

test('the exit status is the executable’s', endToEnd, (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  for (const status of [0, 1, 2, 6, 7, 126, 255]) {
    assert.equal(runLauncher(layout, ['exit', String(status)]).status, status);
  }
});

test('standard input, output and error are the executable’s', endToEnd, (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  const input = 'draft ü 日本\n'.repeat(10_000);
  const result = runLauncher(layout, ['streams'], { input });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, input);
  assert.equal(result.stderr, 'to stderr');
});

test('a launcher failure is a readable message without a stack trace', endToEnd, (t) => {
  const missing = makeLayout(t, { target: hostTarget, executable: process.execPath });
  fs.rmSync(missing.packageRoot, { recursive: true });
  const notInstalled = runLauncher(missing, ['args']);
  assert.equal(notInstalled.status, launcher.EXIT_NOT_INSTALLED);
  assert.equal(notInstalled.stdout, '');
  assert.match(notInstalled.stderr, /^vsift \(npm launcher\): The native package @vsift\//);
  assert.ok(notInstalled.stderr.includes(hostTarget.packageName));
  assertNoStackTrace(notInstalled.stderr);

  const replaced = makeLayout(t, {
    target: hostTarget,
    executable: process.execPath,
    editDigests: (digests) => {
      digests.packages[hostTarget.packageName].sha256 = '0'.repeat(64);
      return digests;
    },
  });
  const refused = runLauncher(replaced, ['args']);
  assert.equal(refused.status, launcher.EXIT_REFUSED);
  assert.equal(refused.stdout, '');
  assert.match(refused.stderr, /does not match the digest recorded for vsift/);
  assertNoStackTrace(refused.stderr);
});

test('Windows: an executable path past MAX_PATH is named as the reason it cannot start', windowsOnly, (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  // Move the whole install under folders deep enough to pass 260 characters.
  let deep = layout.root;
  while (path.join(deep, 'node_modules', '@vsift', 'win32-x64', 'vsift.exe').length < 300) {
    deep = path.join(deep, 'a-folder-name-of-forty-characters-ü-日本-x');
  }
  fs.mkdirSync(deep, { recursive: true });
  fs.renameSync(path.join(layout.root, 'node_modules'), path.join(deep, 'node_modules'));
  const script = path.join(deep, 'node_modules', 'vsift', 'bin', 'vsift.cjs');
  const result = childProcess.spawnSync(process.execPath, [script, fakeScript, 'args'], { encoding: 'utf8', timeout: 60_000 });
  assert.equal(result.status, launcher.EXIT_REFUSED, result.stderr);
  assert.match(result.stderr, /Windows starts programs only from paths shorter than 260/);
  assertNoStackTrace(result.stderr);
});

/** Starts the launcher in `mode` and resolves once the executable is ready. */
function startLauncher(layout, mode, options = {}) {
  const child = childProcess.spawn(process.execPath, [layout.script, fakeScript, mode], {
    stdio: ['pipe', 'pipe', 'pipe'],
    ...options,
  });
  let stdout = '';
  let stderr = '';
  child.stderr.on('data', (chunk) => {
    stderr += chunk;
  });
  const ended = new Promise((resolve) => {
    // 'close' comes once every stream has ended, so no output is still in flight.
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }));
  });
  const ready = new Promise((resolve, reject) => {
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
      if (stdout.includes('ready\n')) {
        resolve();
      }
    });
    ended.then((result) => reject(new Error(`the launcher ended before it was ready: ${JSON.stringify(result)}`)));
  });
  const timer = setTimeout(() => child.kill('SIGKILL'), 60_000);
  ended.then(() => clearTimeout(timer));
  return { child, ready, ended };
}


test('a signal sent to the launcher alone reaches the executable', posix, async (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  for (const signal of ['SIGTERM', 'SIGINT']) {
    const run = startLauncher(layout, 'trap');
    await run.ready;
    run.child.kill(signal);
    const result = await run.ended;
    assert.deepEqual([result.code, result.signal], [6, null], result.stderr);
    assert.match(result.stdout, new RegExp(`caught ${signal}\\n`));
  }
});

test('the launcher ends with the signal that ended the executable', posix, async (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  const run = startLauncher(layout, 'hang');
  await run.ready;
  run.child.kill('SIGTERM');
  const result = await run.ended;
  assert.deepEqual([result.code, result.signal], [null, 'SIGTERM'], result.stderr);
});

test('Ctrl-C to the whole process group leaves the launcher waiting for the executable', posix, async (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  // A process group of its own, like a terminal's foreground job.
  const run = startLauncher(layout, 'trap', { detached: true });
  await run.ready;
  process.kill(-run.child.pid, 'SIGINT');
  const result = await run.ended;
  assert.deepEqual([result.code, result.signal], [6, null], result.stderr);
  assert.match(result.stdout, /caught SIGINT/);
});

function sendConsoleEvent(pid, event) {
  return childProcess.spawnSync(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', sendConsoleControl, '-ProcessId', String(pid), '-Event', event],
    { encoding: 'utf8', timeout: 60_000 },
  );
}

test('a console Ctrl-Break leaves the launcher waiting for the executable', windowsOnly, async (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  // With every stream a pipe and windowsHide, the launcher gets a console of
  // its own (CREATE_NO_WINDOW), which the executable shares.
  const trapped = startLauncher(layout, 'trap', { windowsHide: true });
  await trapped.ready;
  assert.equal(sendConsoleEvent(trapped.child.pid, 'CtrlBreak').status, 0);
  const cancelled = await trapped.ended;
  assert.equal(cancelled.code, 6, cancelled.stderr);
  assert.match(cancelled.stdout, /caught SIGBREAK/);

  const ended = startLauncher(layout, 'hang', { windowsHide: true });
  await ended.ready;
  assert.equal(sendConsoleEvent(ended.child.pid, 'CtrlBreak').status, 0);
  const interrupted = await ended.ended;
  assert.equal(interrupted.code, STATUS_CONTROL_C_EXIT, interrupted.stderr);
});
