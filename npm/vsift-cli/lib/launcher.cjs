// The `vsift-cli` npm launcher, which provides the `vsift` command (P13 PR 9,
// ADR 0023 section 2 and decision H5; the package name since the 2026-09-30
// amendment of decision A).
// `bin/vsift.cjs` runs `main`; the tests call the rest.
//
// It only selects and runs the native `vsift` executable: it finds the
// installed platform package for this machine (`@vsift/<platform>-<arch>`, an
// exact-version optional dependency), checks that the package's version is
// this launcher's and that the executable's SHA-256 is the one recorded in
// `platform-digests.json` when the release was built, then runs it with an
// explicit executable and argument list (never a shell), with the same
// standard streams, and ends with its exit status or signal.
//
// Plain CommonJS using only `node:` built-ins that Node.js 22+ and Bun 1.2+
// share. No package has an install script; nothing here downloads anything.
// A launcher failure is a readable message on stderr and exit 127 (no usable
// native package) or 126 (the native executable was refused or could not be
// started), never a stack trace: vsift itself never exits with either.

'use strict';

const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const tty = require('node:tty');

/** The R0 targets (ADR 0023 decision D), keyed by `<platform> <arch>`. */
const TARGETS = Object.freeze({
  'darwin arm64': Object.freeze({ packageName: '@vsift/darwin-arm64', executable: 'vsift', label: 'macOS on Apple silicon' }),
  'linux x64': Object.freeze({ packageName: '@vsift/linux-x64', executable: 'vsift', label: 'Linux x64 with glibc' }),
  'win32 x64': Object.freeze({ packageName: '@vsift/win32-x64', executable: 'vsift.exe', label: 'Windows x64' }),
});

const EXIT_NOT_INSTALLED = 127;
const EXIT_REFUSED = 126;
const DIGESTS_FORMAT = 'vsift-platform-digests/1';
// Windows' MAX_PATH: CreateProcess cannot start an executable at a longer path.
const WINDOWS_MAX_PATH = 260;
const INSTALL_GUIDE ='https://github.com/smormah/vsift/blob/main/docs/operations/install.md';

/** An expected launcher failure: a message for people and the exit status. */
class LauncherFailure extends Error {
  constructor(exitCode, message) {
    super(message);
    this.name = 'LauncherFailure';
    this.exitCode = exitCode;
  }
}

function supportedTargets() {
  return Object.values(TARGETS)
    .map((target) => `  ${target.packageName} (${target.label})`)
    .join('\n');
}

/** The target for `platform` and `arch`, or a failure naming the supported ones. */
function selectTarget(platform, arch) {
  const target = TARGETS[`${platform} ${arch}`];
  if (target === undefined) {
    throw new LauncherFailure(
      EXIT_NOT_INSTALLED,
      `VSift has no native build for this platform (${platform} ${arch}). The supported targets are:\n` +
        `${supportedTargets()}\nSee ${INSTALL_GUIDE}`,
    );
  }
  return target;
}

function isRecord(value) {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function readJson(file, what) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (error) {
    throw new LauncherFailure(EXIT_REFUSED, `${what} could not be read (${describe(error)}). Reinstall vsift-cli.`);
  }
}

/** This launcher's version and the recorded digest of `target`'s executable. */
function readLauncherRecords(launcherRoot, target) {
  const manifest = readJson(path.join(launcherRoot, 'package.json'), 'The vsift package manifest');
  const version = isRecord(manifest) ? manifest.version : undefined;
  const digests = readJson(path.join(launcherRoot, 'platform-digests.json'), 'The vsift platform digests');
  const packages = isRecord(digests) && isRecord(digests.packages) ? digests.packages : {};
  const entry = Object.hasOwn(packages, target.packageName) ? packages[target.packageName] : undefined;
  const valid =
    typeof version === 'string' &&
    isRecord(digests) &&
    digests.format === DIGESTS_FORMAT &&
    digests.version === version &&
    isRecord(entry) &&
    entry.file === target.executable &&
    Number.isSafeInteger(entry.size) &&
    entry.size > 0 &&
    typeof entry.sha256 === 'string' &&
    /^[0-9a-f]{64}$/.test(entry.sha256);
  if (!valid) {
    throw new LauncherFailure(EXIT_REFUSED, 'The vsift-cli package is incomplete or damaged (platform-digests.json). Reinstall vsift-cli.');
  }
  return { version, expected: entry };
}

/** The installed platform package's directory, checked to be `version`. */
function findPlatformPackage(target, version, resolve) {
  let manifestPath;
  try {
    manifestPath = resolve(`${target.packageName}/package.json`);
  } catch {
    throw new LauncherFailure(
      EXIT_NOT_INSTALLED,
      `The native package ${target.packageName}@${version} is not installed.\n` +
        'It is an optional dependency of vsift-cli. It is missing when optional dependencies were omitted ' +
        '(for example --omit=optional or --no-optional) or the lockfile was made on another platform.\n' +
        `Reinstall vsift-cli@${version} with optional dependencies included. The supported targets are:\n` +
        `${supportedTargets()}\nSee ${INSTALL_GUIDE}`,
    );
  }
  const manifest = readJson(manifestPath, `The manifest of ${target.packageName}`);
  if (!isRecord(manifest) || manifest.name !== target.packageName || manifest.version !== version) {
    const found = isRecord(manifest) && typeof manifest.version === 'string' ? manifest.version : 'an unknown version';
    throw new LauncherFailure(
      EXIT_REFUSED,
      `${target.packageName} is ${found}, but this vsift launcher is ${version}; both must be the same version.\n` +
        `Reinstall vsift-cli@${version}.`,
    );
  }
  return path.dirname(manifestPath);
}

/** Requires the executable to have the recorded size and SHA-256 (decision H5). */
function checkExecutable(executablePath, expected, target, version) {
  let bytes;
  try {
    bytes = fs.statSync(executablePath).size === expected.size ? fs.readFileSync(executablePath) : undefined;
  } catch {
    bytes = undefined;
  }
  const actual = bytes === undefined ? undefined : crypto.createHash('sha256').update(bytes).digest('hex');
  if (actual !== expected.sha256) {
    throw new LauncherFailure(
      EXIT_REFUSED,
      `The native executable in ${target.packageName} is missing or does not match the digest recorded ` +
        `for vsift ${version}; it may be damaged or replaced.\nReinstall vsift-cli@${version}.`,
    );
  }
}

/**
 * Finds and checks the executable to run on `platform`/`arch`. `resolve` finds
 * a package file the way `require.resolve` does from this launcher.
 */
function prepare(launcherRoot, platform, arch, resolve) {
  const target = selectTarget(platform, arch);
  const { version, expected } = readLauncherRecords(launcherRoot, target);
  const packageRoot = findPlatformPackage(target, version, resolve);
  const executablePath = path.join(packageRoot, target.executable);
  checkExecutable(executablePath, expected, target, version);
  return { executablePath, target, version };
}

/**
 * Signals the launcher relays to the running executable. On Windows a console
 * Ctrl-C or Ctrl-Break already reaches every process of the console, so the
 * launcher only stays alive until the executable has finished. Elsewhere it
 * relays each signal once, except `SIGINT` while a standard stream is a
 * terminal: a terminal sends Ctrl-C to the whole foreground process group, so
 * vsift already has it, and a second one would escalate its cancellation.
 */
function relaySignals(child) {
  const windows = process.platform === 'win32';
  const interactive = [0, 1, 2].some((fd) => tty.isatty(fd));
  const listeners = [];
  for (const signal of windows ? ['SIGINT', 'SIGBREAK', 'SIGHUP'] : ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    const listener = () => {
      if (windows || (signal === 'SIGINT' && interactive)) {
        return;
      }
      try {
        child.kill(signal);
      } catch {
        // The executable has already ended; its exit is handled below.
      }
    };
    try {
      process.on(signal, listener);
      listeners.push([signal, listener]);
    } catch {
      // A runtime without this signal cannot receive it either.
    }
  }
  return () => listeners.forEach(([signal, listener]) => process.removeListener(signal, listener));
}

/** Ends the launcher as the executable ended: the same status or signal. */
function endAs(code, signal) {
  if (signal && process.platform !== 'win32') {
    process.kill(process.pid, signal);
    const number = os.constants.signals[signal];
    setTimeout(() => process.exit(128 + (number || 0)), 200);
    return;
  }
  process.exit(code === null ? EXIT_REFUSED : code);
}

function describe(error) {
  return error && typeof error.code === 'string' ? error.code : 'unexpected error';
}

function run(executablePath, target, args) {
  const child = childProcess.spawn(executablePath, args, { stdio: 'inherit', shell: false, windowsHide: false });
  const stopRelay = relaySignals(child);
  child.once('error', (error) => {
    stopRelay();
    // The executable was just read, so "not found" means the system could not
    // start it: a missing loader or library on Linux, or too long a path on Windows.
    const notStarted = describe(error) === 'ENOENT';
    let hint = '';
    if (notStarted && target.packageName === '@vsift/linux-x64') {
      hint = ' The Linux build needs glibc 2.35 or later and OpenSSL 3 (libssl.so.3).';
    } else if (notStarted && process.platform === 'win32' && executablePath.length >= WINDOWS_MAX_PATH) {
      hint =
        ` Its path is ${executablePath.length} characters long, and Windows starts programs only from paths ` +
        `shorter than ${WINDOWS_MAX_PATH}; install vsift-cli in a folder with a shorter path.`;
    }
    process.stderr.write(`vsift (npm launcher): could not start ${executablePath} (${describe(error)}).${hint}\n`);
    process.exit(EXIT_REFUSED);
  });
  child.once('exit', (code, signal) => {
    stopRelay();
    endAs(code, signal);
  });
}

function main() {
  try {
    const launcherRoot = path.join(__dirname, '..');
    const { executablePath, target } = prepare(launcherRoot, process.platform, process.arch, require.resolve);
    run(executablePath, target, process.argv.slice(2));
  } catch (error) {
    const failure =
      error instanceof LauncherFailure ? error : new LauncherFailure(EXIT_REFUSED, `Unexpected failure (${describe(error)}).`);
    process.stderr.write(`vsift (npm launcher): ${failure.message}\n`);
    process.exit(failure.exitCode);
  }
}

module.exports = { TARGETS, EXIT_NOT_INSTALLED, EXIT_REFUSED, DIGESTS_FORMAT, LauncherFailure, selectTarget, prepare, main };
