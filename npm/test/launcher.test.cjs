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

const launcherSource = path.join(__dirname, '..', 'vsift-cli');
const launcher = require(path.join(launcherSource, 'lib', 'launcher.cjs'));
const fakeScript = path.join(__dirname, 'fixtures', 'fake-vsift.cjs');
const sendConsoleControl = path.join(__dirname, '..', '..', 'tools', 'send-console-ctrl.ps1');
const repositoryRoot = path.join(__dirname, '..', '..');
const { MARKER, hostileArguments, hostileFileNames } = require(path.join(repositoryRoot, 'tools', 'p14-published', 'lib', 'hostile.cjs'));
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
 * Removes a folder tree, retrying when Windows still holds a file in it. An
 * executable that just ran or was just copied can stay locked for a moment (the
 * exited process, or the scanner of a hosted image), and `unlink` then fails
 * with EBUSY (#285). `rmSync` retries only when told to, with a linear back-off
 * of `retryDelay` per attempt (about five seconds in all here); a file that is
 * still held after that still fails the test.
 */
function removeTree(target, options = {}) {
  fs.rmSync(target, { recursive: true, maxRetries: 10, retryDelay: 100, ...options });
}

/**
 * Lays out node_modules the way a package manager does, under a directory
 * whose name has spaces and non-ASCII letters: the launcher from source and
 * `target`'s platform package holding `executable` (bytes, or a file to copy).
 */
function makeLayout(t, { target, executable, platformVersion = VERSION, editDigests = (digests) => digests }) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift launcher ü 日本 '));
  t.after(() => removeTree(root, { force: true }));
  const launcherRoot = path.join(root, 'node_modules', 'vsift-cli');
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

test('an unsupported platform names every machine the release is built for, and claims nothing about them', () => {
  for (const [platform, arch] of [['linux', 'arm64'], ['darwin', 'x64'], ['win32', 'arm64'], ['freebsd', 'x64']]) {
    assertFailure(
      () => launcher.selectTarget(platform, arch),
      launcher.EXIT_NOT_INSTALLED,
      `${platform} ${arch}`,
      'This release is built for:',
      '@vsift/win32-x64',
      '@vsift/darwin-arm64',
      '@vsift/linux-x64',
    );
    // The message ships in the package, so it cannot follow the claims ladder (ADR 0024 decision G):
    // it names the targets and uses none of the words the public-claims registry controls.
    assert.throws(
      () => launcher.selectTarget(platform, arch),
      (error) => !/\b(supported|stable|qualified|certified|guaranteed)\b/i.test(error.message),
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
    removeTree(layout.packageRoot);
    assertFailure(
      () => prepareFor(layout, target),
      launcher.EXIT_NOT_INSTALLED,
      `${target.packageName}@${VERSION} is not installed`,
      '--omit=optional',
      `Reinstall vsift-cli@${VERSION}`,
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
    assertFailure(() => prepareFor(layout, target), launcher.EXIT_REFUSED, 'Reinstall vsift-cli');
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

// The route the Windows `vsift.cmd` warning recommends (L-109, #257): Node on
// the launcher with an argument list and no shell. The `.cmd` file that npm
// and pnpm write runs exactly this after cmd.exe has re-read the line, so
// the hostile text of the published-artifact qualification must reach the
// executable unchanged and no command it spells may run.
test('hostile file names and arguments reach the executable unchanged and run nothing (the route install.md recommends)', endToEnd, (t) => {
  const layout = makeLayout(t, { target: hostTarget, executable: process.execPath });
  const cases = [...hostileFileNames(process.platform), ...hostileArguments(process.platform)];
  assert.ok(cases.length >= 25, `only ${cases.length} hostile cases for ${process.platform}`);
  for (const [label, value] of cases) {
    const result = runLauncher(layout, ['args', '--', value], { cwd: layout.root });
    assert.equal(result.status, 0, `${label}: ${result.stderr}`);
    assert.deepEqual(JSON.parse(result.stdout), ['--', value], label);
  }
  assert.equal(fs.existsSync(path.join(layout.root, MARKER)), false, `${MARKER} was created: a command an argument spelled ran`);
});

// The shims npm itself writes on Windows (#257, L-109), installed from this
// source into a temporary prefix: npm's own CLI, started with this Node.js and
// an argument list, no registry (`--offline`, optional dependencies omitted),
// no install scripts. The platform package is laid out by hand, with the
// stand-in executable, as in the tests above. The cases go through each shim
// from PowerShell, the way the published-artifact check sends them.

const npmCli = path.join(path.dirname(process.execPath), 'node_modules', 'npm', 'bin', 'npm-cli.js');
const throughPowerShell = path.join(__dirname, 'fixtures', 'through-powershell.ps1');

function powerShellFound() {
  if (!windows) {
    return false;
  }
  const probe = childProcess.spawnSync('pwsh', ['-NoProfile', '-NonInteractive', '-Command', 'exit 0'], { timeout: 60_000, windowsHide: true });
  return probe.status === 0;
}

const npmShims = {
  skip:
    windowsOnly.skip ||
    (fs.existsSync(npmCli) ? false : `npm is not beside this Node.js (${npmCli})`) ||
    (powerShellFound() ? false : 'PowerShell 7 (pwsh) is not on PATH'),
};

/** An environment for npm that reads no user configuration and writes only under `root`. */
function npmEnvironment(root) {
  const environment = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^npm_/i.test(key)));
  const userconfig = path.join(root, 'empty.npmrc');
  fs.writeFileSync(userconfig, '');
  return {
    ...environment,
    npm_config_userconfig: userconfig,
    npm_config_cache: path.join(root, 'npm-cache'),
    npm_config_update_notifier: 'false',
  };
}

let npmInstall;
// Registered at the top level, so it runs once every test of the file has finished.
test.after(() => {
  if (npmInstall !== undefined) {
    removeTree(npmInstall.root, { force: true });
  }
});

/**
 * Installs the launcher globally into a temporary prefix with npm, and lays out
 * the stand-in platform package; once for the tests below, which only read it.
 */
function installWithNpm() {
  if (npmInstall !== undefined) {
    return npmInstall;
  }
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift npm shims '));
  const packageDir = path.join(root, 'package');
  for (const file of ['package.json', 'README.md', path.join('bin', 'vsift.cjs'), path.join('lib', 'launcher.cjs')]) {
    fs.mkdirSync(path.dirname(path.join(packageDir, file)), { recursive: true });
    fs.copyFileSync(path.join(launcherSource, file), path.join(packageDir, file));
  }
  const bytes = fs.readFileSync(process.execPath);
  fs.writeFileSync(
    path.join(packageDir, 'platform-digests.json'),
    JSON.stringify({
      format: launcher.DIGESTS_FORMAT,
      version: VERSION,
      packages: { [hostTarget.packageName]: { file: hostTarget.executable, size: bytes.length, sha256: sha256(bytes) } },
    }),
  );
  const prefix = path.join(root, 'prefix');
  const install = childProcess.spawnSync(
    process.execPath,
    [npmCli, 'install', '--global', '--prefix', prefix, '--install-links', '--omit=optional', '--ignore-scripts', '--offline', '--no-audit', '--no-fund', '--loglevel=error', packageDir],
    { cwd: root, env: npmEnvironment(root), encoding: 'utf8', timeout: 300_000, windowsHide: true },
  );
  assert.equal(install.status, 0, `npm install: ${install.stderr}`);
  const platformDir = path.join(prefix, 'node_modules', ...hostTarget.packageName.split('/'));
  fs.mkdirSync(platformDir, { recursive: true });
  fs.writeFileSync(path.join(platformDir, 'package.json'), JSON.stringify({ name: hostTarget.packageName, version: VERSION }));
  fs.copyFileSync(process.execPath, path.join(platformDir, hostTarget.executable));
  const work = path.join(root, 'work');
  fs.mkdirSync(work);
  npmInstall = { root, prefix, work };
  return npmInstall;
}

/** `file` with its long name: os.tmpdir() can be an 8.3 short name that PowerShell and where.exe expand. */
function longPath(file) {
  return fs.realpathSync.native(file).toLowerCase();
}

/** Runs `command <prefix...> <case>` from PowerShell once per case; what each printed, its status and whether the marker appeared. */
function runThroughPowerShell(command, prefix, cases, cwd) {
  const carry = (values) => values.map((value) => Buffer.from(value, 'utf8').toString('base64')).join(',');
  const result = childProcess.spawnSync('pwsh', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', throughPowerShell], {
    cwd,
    encoding: 'utf8',
    timeout: 600_000,
    windowsHide: true,
    env: {
      ...process.env,
      VSIFT_TEST_COMMAND: command,
      VSIFT_TEST_PREFIX: carry(prefix),
      VSIFT_TEST_CASES: carry(cases),
      VSIFT_TEST_MARKER: path.join(cwd, MARKER),
    },
  });
  assert.equal(result.status, 0, `pwsh: ${result.stderr}`);
  const runs = JSON.parse(result.stdout);
  assert.equal(runs.length, cases.length);
  return runs;
}

/** The value the stand-in executable received after `--`, or undefined when it printed no argument list. */
function receivedValue(run) {
  try {
    const received = JSON.parse(run.stdout.trim().split('\n').pop());
    return received.length === 2 && received[0] === '--' ? received[1] : { split: received.slice(1) };
  } catch {
    return undefined;
  }
}

/** The hostile cases that did not arrive unchanged, or that ran a command, through `command`. */
function hostileDifferences(command, install) {
  const cases = [...hostileFileNames(process.platform), ...hostileArguments(process.platform)];
  const runs = runThroughPowerShell(command, [fakeScript, 'args', '--'], cases.map(([, value]) => value), install.work);
  return cases.flatMap(([label, value], index) => {
    const run = runs[index];
    const received = receivedValue(run);
    return run.marker || run.status !== 0 || received !== value ? [{ label, value, received, status: run.status, ranACommand: run.marker }] : [];
  });
}

test('Windows: npm writes vsift.cmd, vsift.ps1 and a POSIX script, PowerShell runs the .ps1, and a PATHEXT lookup finds the .cmd', npmShims, () => {
  const install = installWithNpm();
  // npm's cmd-shim writes all three for every `bin` entry (for a native `.exe` target as well, as
  // the reproduction for #257 showed), so no `bin` this package could declare leaves the `.cmd` out.
  assert.deepEqual(fs.readdirSync(install.prefix).filter((file) => file.startsWith('vsift')).sort(), ['vsift', 'vsift.cmd', 'vsift.ps1']);
  const pathWithPrefix = `${install.prefix};${process.env.PATH}`;
  const powerShell = childProcess.spawnSync('pwsh', ['-NoProfile', '-NonInteractive', '-Command', '(Get-Command vsift).Source'], {
    encoding: 'utf8',
    env: { ...process.env, PATH: pathWithPrefix },
    timeout: 60_000,
    windowsHide: true,
  });
  assert.equal(longPath(powerShell.stdout.trim()), longPath(path.join(install.prefix, 'vsift.ps1')), powerShell.stderr);
  // A lookup that follows PATHEXT (where.exe here; Python's shutil.which and Go's exec.LookPath do the same)
  // has no `.ps1` in it, so it finds the `.cmd`, and a program that then passes an argument list still goes through cmd.exe.
  const where = childProcess.spawnSync('where.exe', ['vsift'], { encoding: 'utf8', env: { ...process.env, PATH: pathWithPrefix }, timeout: 60_000 });
  const found = where.stdout.split(/\r?\n/).filter((line) => /\.[a-z]+$/i.test(line));
  assert.equal(longPath(found[0]), longPath(path.join(install.prefix, 'vsift.cmd')), where.stdout);
});

test("Windows: through npm's vsift.ps1 every hostile file name and argument arrives unchanged and runs nothing", npmShims, () => {
  const install = installWithNpm();
  const differences = hostileDifferences(path.join(install.prefix, 'vsift.ps1'), install);
  assert.deepEqual(differences, []);
});

// L-109 is an accepted residual, so this pins it as the register, install.md,
// SECURITY.md and the README describe it. If it fails, npm or Windows no longer
// does what they say: rewrite them (and the entry) rather than this test.
test("Windows: through npm's vsift.cmd, cmd.exe still re-reads the line as L-109 records, and a .cmd that ignores its arguments does the same", npmShims, () => {
  const install = installWithNpm();
  const differences = hostileDifferences(path.join(install.prefix, 'vsift.cmd'), install);
  const byLabel = new Map(differences.map((difference) => [difference.label, difference]));
  // An unquoted redirection with no whitespace runs: the marker is created.
  assert.equal(byLabel.get('semicolon and comma')?.ranACommand, true, JSON.stringify(differences));
  // %NAME% is expanded before the launcher runs.
  assert.equal(byLabel.get('percent variable')?.received, process.env.ComSpec, JSON.stringify(differences));
  // The same cases through the .ps1 shim are exact (the test above), so the difference is the .cmd file's.
  assert.ok(differences.length >= 3, JSON.stringify(differences));

  // Why no shim content, `bin` shape or launcher check can remove it: cmd.exe acts on
  // the caller's command line while it starts the batch file, before the file's
  // first line. A .cmd that never reads its arguments creates the marker as well.
  const noop = path.join(install.root, 'noop.cmd');
  fs.writeFileSync(noop, '@exit /b 0\r\n');
  const [run] = runThroughPowerShell(noop, ['ingest', '--'], [`a;echo,x>${MARKER}`], install.work);
  assert.equal(run.marker, true, JSON.stringify(run));
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
  removeTree(missing.packageRoot);
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
  const script = path.join(deep, 'node_modules', 'vsift-cli', 'bin', 'vsift.cjs');
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

// The documents that tell a Windows user what the `vsift.cmd` file of npm and
// pnpm does (L-109, #257). No package can leave that file out (the npm-shim
// tests above show why), so the warning and the routes around it are the
// mitigation and must not be dropped silently. They name both ways a program
// reaches the `.cmd`: through a shell, and with an argument list after a
// `PATHEXT` lookup such as Python's `shutil.which` (found on 2026-10-16).
const windowsShimDocuments = {
  'docs/operations/install.md': [
    'vsift.cmd',
    'cmd.exe',
    'PowerShell',
    'Git Bash',
    'vsift.exe',
    'execFile',
    'vsift.cjs',
    'child_process.exec',
    'shell=True',
    'PATHEXT',
    'shutil.which',
    'Yarn 1',
    'npx.cmd',
    'L-109',
  ],
  'SECURITY.md': ['vsift.cmd', 'cmd.exe', 'PowerShell', 'Git Bash', 'vsift.exe', 'PATHEXT', 'shutil.which', 'known-limits.md#l-109'],
  'npm/vsift-cli/README.md': ['vsift.cmd', 'cmd.exe', 'PowerShell', 'Git Bash', 'vsift.exe', 'bin/vsift.cjs', 'PATHEXT', 'shutil.which', 'Yarn 1'],
  'docs/planning/known-limits.md': ['PATHEXT', 'shutil.which', 'Yarn 1'],
};

test('the install guide, the security policy, the package README and the register warn about vsift.cmd and name the safe routes', () => {
  for (const [relative, fragments] of Object.entries(windowsShimDocuments)) {
    const text = fs.readFileSync(path.join(repositoryRoot, ...relative.split('/')), 'utf8');
    for (const fragment of fragments) {
      assert.ok(text.includes(fragment), `${relative} does not mention ${fragment}`);
    }
  }
});

test('the launcher path the install guide gives for Node is the file the package really has', () => {
  const guide = fs.readFileSync(path.join(repositoryRoot, 'docs', 'operations', 'install.md'), 'utf8');
  assert.ok(guide.includes('vsift-cli\\bin\\vsift.cjs'), 'install.md names no launcher path');
  assert.ok(fs.existsSync(path.join(launcherSource, 'bin', 'vsift.cjs')));
  for (const target of Object.values(launcher.TARGETS)) {
    if (target.packageName === '@vsift/win32-x64') {
      assert.ok(guide.includes(target.packageName.replace('/', '\\')), 'install.md does not name the Windows platform package');
      assert.equal(target.executable, 'vsift.exe');
    }
  }
});

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
