'use strict';

// Shared constants and helpers of the P14 published-artifact qualification
// tools (P14 PR 2; ADR 0024 "What P14 delivers" item 2; evidence items RQ-01
// to RQ-04 and RQ-19 of docs/planning/p14-qualification.md).
//
// Everything here is plain CommonJS over `node:` built-ins, so the scripts run
// on the Node.js 22 the hosted runners carry. Commands run with an explicit
// executable and argument list, never through a shell. On Windows the package
// managers' `.cmd` and `.ps1` shims run through invoke.ps1, which passes every
// argument on without interpreting it (the same step the P13 npm qualification
// driver, npm/qualification/qualify.cjs, takes).
//
// Nothing here publishes anything or holds a credential. The only secret a
// script may see is the job's default read-only GitHub token, which it hands to
// `gh` alone (see `githubEnvironment`).

const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

/** The public registry every package-manager step uses. */
const REGISTRY = 'https://registry.npmjs.org/';
/** The repository the packages and the release belong to. */
const REPOSITORY = 'smormah/vsift';
/** The only workflow that may have signed a VSift release file (install.md section 3). */
const SIGNER_WORKFLOW = 'smormah/vsift/.github/workflows/release.yml';
/** Neutral identifier for a request this tooling makes itself: no person, no machine. */
const USER_AGENT = 'vsift-p14-qualification';

/** The four published packages, the launcher first. */
const PACKAGES = Object.freeze(['vsift-cli', '@vsift/win32-x64', '@vsift/darwin-arm64', '@vsift/linux-x64']);

/**
 * The three R0 targets, keyed by `<process.platform> <process.arch>`: the npm
 * platform package, the executable inside it, and the release archive's target.
 */
const TARGETS = Object.freeze({
  'win32 x64': Object.freeze({
    packageName: '@vsift/win32-x64',
    executable: 'vsift.exe',
    archiveTarget: 'x86_64-pc-windows-msvc',
    label: 'Windows 11 x64',
  }),
  'darwin arm64': Object.freeze({
    packageName: '@vsift/darwin-arm64',
    executable: 'vsift',
    archiveTarget: 'aarch64-apple-darwin',
    label: 'macOS 15 arm64',
  }),
  'linux x64': Object.freeze({
    packageName: '@vsift/linux-x64',
    executable: 'vsift',
    archiveTarget: 'x86_64-unknown-linux-gnu',
    label: 'Ubuntu 24.04 x64',
  }),
});

const windows = process.platform === 'win32';
const INVOKE = path.join(__dirname, '..', 'invoke.ps1');

/** The target of the machine running this script. */
function currentTarget() {
  const target = TARGETS[`${process.platform} ${process.arch}`];
  if (target === undefined) {
    throw new QualificationError(`no VSift target for ${process.platform} ${process.arch}`);
  }
  return target;
}

/** A failed check or a refused input: a message for the job log, never a stack. */
class QualificationError extends Error {
  constructor(message) {
    super(message);
    this.name = 'QualificationError';
  }
}

function expect(condition, message) {
  if (!condition) {
    throw new QualificationError(message);
  }
}

/** Parses `--name value` pairs against a spec of required and optional names. */
function parseArguments(argv, { required = [], optional = [], flags = [] }) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!argument.startsWith('--')) {
      throw new QualificationError(`unexpected argument ${argument}`);
    }
    const name = argument.slice(2);
    if (flags.includes(name)) {
      options[name] = true;
      continue;
    }
    const value = argv[index + 1];
    if (!required.includes(name) && !optional.includes(name)) {
      throw new QualificationError(`unknown option --${name}`);
    }
    if (value === undefined || value.startsWith('--')) {
      throw new QualificationError(`--${name} needs a value`);
    }
    options[name] = value;
    index += 1;
  }
  for (const name of required) {
    if (typeof options[name] !== 'string' || options[name] === '') {
      throw new QualificationError(`--${name} is required`);
    }
  }
  return options;
}

// ---------------------------------------------------------------- results

/**
 * Records named checks and reports them as a Markdown table, on standard
 * output and in the job summary.
 */
class Recorder {
  constructor() {
    this.results = [];
  }

  /** Runs one check; returns whether it passed. A thrown failure is recorded, not rethrown. */
  async check(name, action) {
    const started = process.hrtime.bigint();
    process.stdout.write(`- ${name} ... `);
    try {
      const detail = await action();
      this.results.push({ name, ok: true, detail: detail || '' });
      process.stdout.write(`pass (${Math.round(elapsedMs(started))} ms)\n`);
      return true;
    } catch (error) {
      const detail = error instanceof QualificationError ? error.message : `${error && error.stack}`;
      this.results.push({ name, ok: false, detail });
      process.stdout.write(`FAIL (${Math.round(elapsedMs(started))} ms)\n  ${detail.replace(/\n/g, '\n  ')}\n`);
      return false;
    }
  }

  /** Records a fact that is shown but never gates the job (an observation). */
  observe(name, detail) {
    this.results.push({ name, ok: true, observation: true, detail });
    process.stdout.write(`- ${name} ... observed: ${detail}\n`);
  }

  get ok() {
    return this.results.every((result) => result.ok);
  }

  /** The Markdown table of every result. */
  markdown(title) {
    const lines = [`### ${title}`, '', '| Check | Result | Detail |', '| --- | --- | --- |'];
    for (const result of this.results) {
      const detail = result.detail.replace(/\r?\n/g, ' ').replace(/\|/g, '\\|').slice(0, 400);
      const verdict = result.ok ? (result.observation ? 'observed' : 'pass') : '**FAIL**';
      lines.push(`| ${result.name.replace(/\|/g, '\\|')} | ${verdict} | ${detail} |`);
    }
    return `${lines.join('\n')}\n`;
  }

  /** Prints the table, appends it to the job summary and sets the exit status. */
  finish(title) {
    const text = this.markdown(title);
    process.stdout.write(`\n${text}`);
    if (process.env.GITHUB_STEP_SUMMARY) {
      fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${text}\n`);
    }
    process.exitCode = this.ok ? 0 : 1;
  }
}

function elapsedMs(started) {
  return Number(process.hrtime.bigint() - started) / 1e6;
}

// ---------------------------------------------------------------- processes

/**
 * Runs a command the way a user types it: on Windows a `.cmd`/`.ps1` shim goes
 * through invoke.ps1 (so PowerShell resolves it as it does for a user),
 * elsewhere and for `.exe` files the executable starts directly.
 */
function run(command, args, env, options = {}) {
  const spawnOptions = {
    env,
    cwd: options.cwd,
    input: options.input,
    encoding: 'utf8',
    timeout: options.timeout || 600_000,
    maxBuffer: 64 * 1024 * 1024,
    windowsHide: true,
  };
  const direct = !windows || command.toLowerCase().endsWith('.exe');
  let result;
  if (direct) {
    result = childProcess.spawnSync(command, args, spawnOptions);
  } else {
    // The command and its arguments travel in the environment, each argument as
    // Base64 of its UTF-8 bytes: see invoke.ps1 for why they are not on pwsh's
    // own command line.
    const carried = {
      ...env,
      P14_COMMAND: command,
      P14_ARGUMENT_COUNT: String(args.length),
      P14_ARGUMENTS: args.map((argument) => Buffer.from(argument, 'utf8').toString('base64')).join(','),
    };
    result = childProcess.spawnSync('pwsh', ['-NoProfile', '-NonInteractive', '-File', INVOKE], { ...spawnOptions, env: carried });
  }
  if (result.error && result.error.code === 'ETIMEDOUT') {
    throw new QualificationError(
      `${path.basename(command)} ${args.join(' ')} did not finish within ${spawnOptions.timeout / 1000} s: ${(result.stderr || '').slice(-1000)}`,
    );
  }
  if (result.error) {
    throw new QualificationError(`${command} could not be started: ${result.error.code || result.error.message}`);
  }
  return result;
}

/** Like `run`, and a nonzero exit status is a failure naming what it printed. */
function succeed(command, args, env, options) {
  const result = run(command, args, env, options);
  expect(
    result.status === 0,
    `${path.basename(command)} ${args.join(' ')} exited ${result.status} (signal ${result.signal}): ` +
      `${(result.stderr || '').trim().slice(-1500)} ${(result.stdout || '').trim().slice(-500)}`,
  );
  return result;
}

function describeStatus(result) {
  return `exit ${result.status}${result.signal ? ` signal ${result.signal}` : ''}`;
}

/** The JSON document a command printed, or a failure naming its status and stderr. */
function parseJson(result) {
  try {
    return JSON.parse(result.stdout);
  } catch {
    throw new QualificationError(`${describeStatus(result)}, no JSON on stdout: ${(result.stderr || '').trim().slice(0, 1500)}`);
  }
}

function assertNoStackTrace(text) {
  expect(!/^\s+at .+:\d+:\d+\)?$/m.test(text) && !text.includes('node:internal'), `stack trace in: ${text}`);
}

// ---------------------------------------------------------------- environments

/** Variables whose values could carry a credential into a child process. */
const SECRET_NAME = /(TOKEN|SECRET|PASSWORD|CREDENTIAL|^ACTIONS_(ID_TOKEN|RUNTIME)|^ACTIONS_CACHE)/i;
/** Package-manager configuration the environment must not carry into a check. */
const MANAGER_CONFIGURATION = /^(npm_config_|npm_package_|npm_lifecycle_|pnpm_config_|PNPM_|YARN_|BUN_|COREPACK_|NODE_OPTIONS$|NODE_PATH$)/i;

/**
 * A copy of the environment for a child that must hold no credential and no
 * package-manager setting of the runner's.
 */
function cleanEnvironment(source = process.env) {
  const env = {};
  for (const [key, value] of Object.entries(source)) {
    if (value === undefined || SECRET_NAME.test(key) || MANAGER_CONFIGURATION.test(key)) {
      continue;
    }
    env[key] = value;
  }
  return env;
}

/**
 * The environment `gh` runs in: `base` (a scrubbed environment, or the
 * process's own) with the job's default read-only token and nothing else of a
 * secret kind. `gh` never writes in this tooling.
 */
function githubEnvironment(base = process.env, tokenSource = process.env) {
  const env = cleanEnvironment(base);
  const token = tokenSource.GH_TOKEN || tokenSource.GITHUB_TOKEN;
  if (token) {
    env.GH_TOKEN = token;
  }
  env.GH_PROMPT_DISABLED = '1';
  env.GH_NO_UPDATE_NOTIFIER = '1';
  return env;
}

// ---------------------------------------------------------------- files

function writeFile(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
}

function sha256Hex(bytes) {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

function sha256File(file) {
  const hash = crypto.createHash('sha256');
  const descriptor = fs.openSync(file, 'r');
  try {
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    for (;;) {
      const read = fs.readSync(descriptor, buffer, 0, buffer.length, null);
      if (read === 0) {
        break;
      }
      hash.update(buffer.subarray(0, read));
    }
  } finally {
    fs.closeSync(descriptor);
  }
  return hash.digest('hex');
}

/** Every directory under `root` (links are not followed) for which `accept` holds. */
function findDirectories(root, accept, limit = 200_000) {
  const found = [];
  const pending = [root];
  let visited = 0;
  while (pending.length > 0) {
    const directory = pending.pop();
    let entries;
    try {
      entries = fs.readdirSync(directory, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      visited += 1;
      if (visited > limit) {
        return found;
      }
      if (entry.isDirectory()) {
        const child = path.join(directory, entry.name);
        if (accept(child)) {
          found.push(child);
        }
        pending.push(child);
      }
    }
  }
  return found;
}

/** The directories of an installed package called `name` under `root`. */
function findPackage(root, name) {
  const tail = path.join('node_modules', ...name.split('/'));
  return findDirectories(root, (directory) => {
    if (!directory.endsWith(tail)) {
      return false;
    }
    try {
      return JSON.parse(fs.readFileSync(path.join(directory, 'package.json'), 'utf8')).name === name;
    } catch {
      return false;
    }
  });
}

/**
 * The relative paths of every file under `root` with its SHA-256, in path
 * order; a link or anything but a regular file is a failure.
 */
function treeDigests(root) {
  const entries = [];
  const walk = (directory, prefix) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
      const full = path.join(directory, entry.name);
      const relative = prefix === '' ? entry.name : `${prefix}/${entry.name}`;
      if (entry.isDirectory()) {
        walk(full, relative);
      } else if (entry.isFile()) {
        entries.push([relative, sha256File(full)]);
      } else {
        throw new QualificationError(`${full} is not a regular file or a directory`);
      }
    }
  };
  walk(root, '');
  return entries;
}

/** Whether two directory trees hold the same files with the same bytes; explains a difference. */
function compareTrees(left, right) {
  const a = new Map(treeDigests(left));
  const b = new Map(treeDigests(right));
  const problems = [];
  for (const [file, digest] of a) {
    if (!b.has(file)) {
      problems.push(`${file} is only in ${path.basename(left)}`);
    } else if (b.get(file) !== digest) {
      problems.push(`${file} differs`);
    }
  }
  for (const file of b.keys()) {
    if (!a.has(file)) {
      problems.push(`${file} is only in ${path.basename(right)}`);
    }
  }
  return { equal: problems.length === 0, files: a.size, problems };
}

module.exports = {
  REGISTRY,
  REPOSITORY,
  SIGNER_WORKFLOW,
  USER_AGENT,
  PACKAGES,
  TARGETS,
  QualificationError,
  Recorder,
  assertNoStackTrace,
  cleanEnvironment,
  compareTrees,
  currentTarget,
  describeStatus,
  expect,
  findDirectories,
  findPackage,
  githubEnvironment,
  parseArguments,
  parseJson,
  run,
  sha256File,
  sha256Hex,
  succeed,
  treeDigests,
  windows,
  writeFile,
};
