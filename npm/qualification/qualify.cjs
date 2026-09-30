'use strict';

// The npm qualification driver (P13 PR 9, ADR 0023 section 2; the launcher
// boundary of implementation-work-packets.md).
//
//   node npm/qualification/qualify.cjs --manager <npm|pnpm|yarn|bun> \
//     --packages <dir with the four packed tarballs> --verdaccio <verdaccio's bin/verdaccio> \
//     --version-line "<what vsift --version must print>" --work <scratch dir>
//
// It stands up a Verdaccio registry on 127.0.0.1 with no uplink, publishes the
// packed packages to it with npm under the dist-tag `next`, and qualifies one
// package manager against it: a global install (a project install for Yarn,
// which has no global install) with install scripts disabled, under a path
// with spaces and non-ASCII letters; `vsift --version`, `vsift setup check
// --json` and arguments with spaces and Unicode through the installed command;
// standard input; the exit status; signals (Ctrl-C and SIGTERM); the one-shot
// runner; optional dependencies omitted; a damaged, replaced or non-executable
// binary and a mismatched version; running offline; and a clean uninstall.
//
// Nothing is published anywhere but the loopback registry: the driver refuses
// any other registry, the only credential it holds is the throwaway
// registry's own token, written to a scratch npmrc scoped to 127.0.0.1, and
// the registry has no uplink, so no request it receives reaches another
// registry. Commands run with explicit executable and argument lists; on
// Windows the package managers' `.cmd`/`.ps1` shims run through
// invoke.ps1, which passes each argument on without interpreting it.
//
// Every check is recorded; the driver prints a table (and appends it to
// $GITHUB_STEP_SUMMARY when set) and exits 1 if any check failed.

const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const MANAGERS = ['npm', 'pnpm', 'yarn', 'bun'];
const REGISTRY = 'http://127.0.0.1:4873/';
const REGISTRY_HOST = '127.0.0.1:4873';
const windows = process.platform === 'win32';
const INVOKE = path.join(__dirname, 'invoke.ps1');
const SEND_CONSOLE_CONTROL = path.join(__dirname, '..', '..', 'tools', 'send-console-ctrl.ps1');
const TARGETS = {
  'darwin arm64': { packageName: '@vsift/darwin-arm64', executable: 'vsift' },
  'linux x64': { packageName: '@vsift/linux-x64', executable: 'vsift' },
  'win32 x64': { packageName: '@vsift/win32-x64', executable: 'vsift.exe' },
};
const STATUS_CONTROL_C_EXIT = 0xc000013a;
const DIGEST_BUDGET_MS = 50;
// A folder name with spaces, accents and CJK letters: every install, project
// and argument path below it must survive every package manager and shim.
const AWKWARD = 'vsift qualification ü 日本';
// The folders the package manager itself uses carry the same name, except for
// Bun on Windows: Bun 1.2.23 fails there with "InvalidWtf8" on a non-ASCII
// install or cache folder (L-092), so its folders keep only the spaces. The
// arguments vsift receives keep the non-ASCII name on every job.
let folderLabel = AWKWARD;

class QualificationError extends Error {
  constructor(message) {
    super(message);
    this.name = 'QualificationError';
  }
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag.startsWith('--') || value === undefined) {
      throw new QualificationError(`unexpected argument ${flag}`);
    }
    options[flag.slice(2)] = value;
  }
  for (const required of ['manager', 'packages', 'verdaccio', 'version-line', 'work']) {
    if (typeof options[required] !== 'string' || options[required] === '') {
      throw new QualificationError(`--${required} is required`);
    }
  }
  if (!MANAGERS.includes(options.manager)) {
    throw new QualificationError(`--manager must be one of ${MANAGERS.join(', ')}`);
  }
  return options;
}

// ---------------------------------------------------------------- results

const results = [];

async function check(name, action) {
  const started = process.hrtime.bigint();
  process.stdout.write(`- ${name} ... `);
  try {
    const detail = await action();
    results.push({ name, ok: true, detail: detail || '', ms: elapsed(started) });
    process.stdout.write(`pass (${Math.round(elapsed(started))} ms)\n`);
    return true;
  } catch (error) {
    const detail = error instanceof QualificationError ? error.message : `${error && error.stack}`;
    results.push({ name, ok: false, detail, ms: elapsed(started) });
    process.stdout.write(`FAIL (${Math.round(elapsed(started))} ms)\n  ${detail.replace(/\n/g, '\n  ')}\n`);
    return false;
  }
}

function elapsed(started) {
  return Number(process.hrtime.bigint() - started) / 1e6;
}

function expect(condition, message) {
  if (!condition) {
    throw new QualificationError(message);
  }
}

function report(manager) {
  const lines = [
    `### npm qualification: ${manager} on ${process.platform} ${process.arch} (Node.js ${process.versions.node})`,
    '',
    '| Check | Result | Detail |',
    '| --- | --- | --- |',
  ];
  for (const result of results) {
    const detail = result.detail.replace(/\r?\n/g, ' ').replace(/\|/g, '\\|').slice(0, 300);
    lines.push(`| ${result.name} | ${result.ok ? 'pass' : '**FAIL**'} | ${detail} |`);
  }
  const text = `${lines.join('\n')}\n`;
  process.stdout.write(`\n${text}`);
  if (process.env.GITHUB_STEP_SUMMARY) {
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  }
}

// ---------------------------------------------------------------- processes

/**
 * Runs a command a user would type: on Windows through invoke.ps1 (so shims
 * resolve as in PowerShell), elsewhere directly. `input` feeds stdin.
 */
function run(command, args, env, options = {}) {
  const spawnOptions = {
    env,
    cwd: options.cwd,
    input: options.input,
    encoding: 'utf8',
    timeout: options.timeout || 600_000,
    maxBuffer: 64 * 1024 * 1024,
  };
  const direct = !windows || command.toLowerCase().endsWith('.exe');
  const result = direct
    ? childProcess.spawnSync(command, args, spawnOptions)
    : childProcess.spawnSync('pwsh', ['-NoProfile', '-NonInteractive', '-File', INVOKE, command, ...args], spawnOptions);
  if (result.error && result.error.code === 'ETIMEDOUT') {
    throw new QualificationError(
      `${command} ${args.join(' ')} did not finish within ${spawnOptions.timeout / 1000} s: ${(result.stderr || '').slice(-1000)}`,
    );
  }
  if (result.error) {
    throw new QualificationError(`${command} could not be started: ${result.error.code || result.error.message}`);
  }
  return result;
}

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

function assertNoStackTrace(stderr) {
  expect(!/^\s+at .+:\d+:\d+\)?$/m.test(stderr) && !stderr.includes('node:internal'), `stack trace in: ${stderr}`);
}

// ---------------------------------------------------------------- registry

function writeFile(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
}

async function startRegistry(verdaccioEntry, work) {
  const root = path.join(work, 'verdaccio');
  fs.mkdirSync(path.join(root, 'storage'), { recursive: true });
  const config = path.join(root, 'config.yaml');
  // No uplinks: a request for any other package is answered here (not
  // found), never passed to a public registry. The audit middleware, which
  // would forward `npm audit` requests to npmjs.com, is off; so is the web UI.
  writeFile(
    config,
    [
      'storage: ./storage',
      'auth:',
      '  htpasswd:',
      '    file: ./htpasswd',
      '    max_users: 1',
      'uplinks: {}',
      'packages:',
      "  'vsift-cli':",
      '    access: $all',
      '    publish: $authenticated',
      "  '@vsift/*':",
      '    access: $all',
      '    publish: $authenticated',
      "  '**':",
      '    access: $all',
      '    publish: $authenticated',
      'middlewares:',
      '  audit:',
      '    enabled: false',
      'web:',
      '  enable: false',
      'log:',
      '  type: stdout',
      '  format: pretty',
      '  level: warn',
      '',
    ].join('\n'),
  );
  const log = fs.openSync(path.join(root, 'verdaccio.log'), 'a');
  const server = childProcess.spawn(process.execPath, [verdaccioEntry, '--config', config, '--listen', REGISTRY_HOST], {
    cwd: root,
    stdio: ['ignore', log, log],
    windowsHide: true,
  });
  const deadline = Date.now() + 120_000;
  for (;;) {
    try {
      const response = await fetch(`${REGISTRY}-/ping`);
      if (response.ok) {
        break;
      }
    } catch {
      // Not listening yet.
    }
    expect(server.exitCode === null, `Verdaccio exited ${server.exitCode}; see ${root}/verdaccio.log`);
    expect(Date.now() < deadline, 'Verdaccio did not answer /-/ping within 120 s');
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  return { server, root };
}

async function stopRegistry(registry) {
  if (registry.server.exitCode !== null) {
    return;
  }
  const exited = new Promise((resolve) => registry.server.once('exit', resolve));
  registry.server.kill();
  await exited;
}

/** A throwaway user of the loopback registry; returns its token. */
async function registryToken() {
  const name = 'vsift-qualification';
  const response = await fetch(`${REGISTRY}-/user/org.couchdb.user:${name}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name, password: crypto.randomBytes(24).toString('hex'), type: 'user', roles: [] }),
  });
  const body = await response.json();
  expect(response.status === 201 && typeof body.token === 'string', `the registry refused the user: ${response.status}`);
  return body.token;
}

// ---------------------------------------------------------------- layout helpers

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

function isPackageDirectory(directory, name) {
  const parts = name.split('/');
  const tail = path.join('node_modules', ...parts);
  if (!directory.endsWith(tail)) {
    return false;
  }
  try {
    return JSON.parse(fs.readFileSync(path.join(directory, 'package.json'), 'utf8')).name === name;
  } catch {
    return false;
  }
}

function findPackage(root, name) {
  return findDirectories(root, (directory) => isPackageDirectory(directory, name));
}

// ---------------------------------------------------------------- managers

/**
 * What differs between package managers: where the global (or project)
 * install puts the command, how to install, run once, omit optional
 * dependencies and uninstall.
 */
function managerProfile(manager, work, env) {
  const installRoot = path.join(work, `${manager} install ${folderLabel}`);
  const shimNames = windows ? ['vsift.cmd', 'vsift.ps1'] : ['vsift'];
  switch (manager) {
    case 'npm':
      return {
        installRoot,
        env: { ...env, npm_config_prefix: installRoot },
        install: ['npm', ['install', '--global', '--ignore-scripts', 'vsift-cli@next']],
        shims: shimNames.map((name) => (windows ? path.join(installRoot, name) : path.join(installRoot, 'bin', name))),
        oneShot: ['npx', ['--yes', 'vsift-cli@next']],
        omitOptional: ['npm', ['install', '--ignore-scripts', '--omit=optional', 'vsift-cli@next']],
        uninstall: ['npm', ['uninstall', '--global', 'vsift-cli']],
      };
    case 'pnpm': {
      // pnpm 12 puts global commands in $PNPM_HOME/bin and refuses to install
      // globally unless that folder is on PATH.
      const bin = path.join(installRoot, 'bin');
      return {
        installRoot,
        env: { ...env, PNPM_HOME: installRoot, PATH: `${bin}${path.delimiter}${installRoot}${path.delimiter}${env.PATH}` },
        install: ['pnpm', ['add', '--global', '--ignore-scripts', 'vsift-cli@next']],
        shims: shimNames.map((name) => path.join(bin, name)),
        oneShot: ['pnpm', ['dlx', 'vsift-cli@next']],
        omitOptional: ['pnpm', ['install', '--ignore-scripts', '--no-optional']],
        omitOptionalManifest: { dependencies: { 'vsift-cli': 'next' } },
        uninstall: ['pnpm', ['remove', '--global', 'vsift-cli']],
      };
    }
    case 'yarn':
      // Yarn 4 has no global install; a project install is its equivalent.
      return {
        installRoot,
        project: true,
        env,
        install: ['yarn', ['add', 'vsift-cli@next']],
        shims: [],
        runInstalled: ['yarn', ['vsift']],
        oneShot: ['yarn', ['dlx', '--quiet', '--package', 'vsift-cli@next', 'vsift']],
        omitOptional: ['yarn', ['add', 'vsift-cli@next']],
        // Yarn cannot omit optional dependencies; installing for another
        // platform leaves this one's package out in the same way.
        omitOptionalFiles: { '.yarnrc.yml': 'supportedArchitectures:\n  os:\n    - aix\n  cpu:\n    - ppc64\n' },
        omitOptionalRun: ['yarn', ['vsift']],
        uninstall: ['yarn', ['remove', 'vsift-cli']],
      };
    case 'bun': {
      const bin = path.join(installRoot, 'bin');
      return {
        installRoot,
        env: {
          ...env,
          BUN_INSTALL_GLOBAL_DIR: path.join(installRoot, 'global'),
          BUN_INSTALL_BIN: bin,
          PATH: `${bin}${path.delimiter}${env.PATH}`,
        },
        install: ['bun', ['add', '--global', '--ignore-scripts', '--registry', REGISTRY, 'vsift-cli@next']],
        shims: [path.join(bin, windows ? 'vsift.exe' : 'vsift')],
        oneShot: ['bunx', ['vsift-cli@next']],
        oneShotOnBun: ['bunx', ['--bun', 'vsift-cli@next']],
        omitOptional: ['bun', ['install', '--ignore-scripts', '--omit', 'optional', '--registry', REGISTRY]],
        omitOptionalManifest: { dependencies: { 'vsift-cli': 'next' } },
        uninstall: ['bun', ['remove', '--global', 'vsift-cli']],
      };
    }
    default:
      throw new QualificationError(`unknown manager ${manager}`);
  }
}

/** The environment every package manager runs in: the loopback registry only. */
function baseEnvironment(work) {
  const userConfig = path.join(work, 'user.npmrc');
  writeFile(userConfig, `registry=${REGISTRY}\n`);
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    // No token or registry setting from the runner reaches a package manager.
    if (/^(npm_config_|pnpm_config_|YARN_|BUN_CONFIG_|PNPM_)/i.test(key) || /TOKEN/i.test(key)) {
      delete env[key];
    }
  }
  return {
    ...env,
    npm_config_registry: REGISTRY,
    NPM_CONFIG_REGISTRY: REGISTRY,
    npm_config_userconfig: userConfig,
    npm_config_cache: path.join(work, `npm cache ${folderLabel}`),
    npm_config_update_notifier: 'false',
    npm_config_audit: 'false',
    npm_config_fund: 'false',
    // pnpm's store and metadata cache in this run's folder, so no earlier run's
    // metadata for the same version can stand in for this one's.
    npm_config_store_dir: path.join(work, `pnpm store ${folderLabel}`),
    pnpm_config_store_dir: path.join(work, `pnpm store ${folderLabel}`),
    npm_config_cache_dir: path.join(work, `pnpm cache ${folderLabel}`),
    pnpm_config_cache_dir: path.join(work, `pnpm cache ${folderLabel}`),
    YARN_NPM_REGISTRY_SERVER: REGISTRY.slice(0, -1),
    YARN_UNSAFE_HTTP_WHITELIST: '127.0.0.1',
    YARN_ENABLE_TELEMETRY: '0',
    YARN_ENABLE_SCRIPTS: 'false',
    YARN_ENABLE_IMMUTABLE_INSTALLS: 'false',
    // Yarn 4.18 quarantines versions published less than a day ago
    // (`npmMinimalAgeGate`, default 1d); these were published seconds ago.
    // Users meet the gate after a real release (install.md, L-092).
    YARN_NPM_MINIMAL_AGE_GATE: '0',
    YARN_ENABLE_GLOBAL_CACHE: 'false',
    YARN_CACHE_FOLDER: path.join(work, `yarn cache ${folderLabel}`),
    YARN_GLOBAL_FOLDER: path.join(work, `yarn global ${folderLabel}`),
    BUN_CONFIG_REGISTRY: REGISTRY,
    BUN_INSTALL_CACHE_DIR: path.join(work, `bun cache ${folderLabel}`),
    DO_NOT_TRACK: '1',
  };
}

function newProject(work, name, files = {}, manifest = {}) {
  const directory = path.join(work, `${name} ${folderLabel}`);
  fs.mkdirSync(directory, { recursive: true });
  writeFile(
    path.join(directory, 'package.json'),
    `${JSON.stringify({ name: 'vsift-qualification', version: '0.0.0', private: true, ...manifest }, null, 2)}\n`,
  );
  for (const [file, text] of Object.entries(files)) {
    writeFile(path.join(directory, file), text);
  }
  return directory;
}

// ---------------------------------------------------------------- main

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const manager = options.manager;
  const target = TARGETS[`${process.platform} ${process.arch}`];
  expect(target !== undefined, `no VSift target for ${process.platform} ${process.arch}`);
  if (windows && manager === 'bun') {
    folderLabel = 'vsift qualification';
  }
  const work = path.join(path.resolve(options.work), folderLabel, manager);
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const tarballs = fs
    .readdirSync(options.packages)
    .filter((name) => name.endsWith('.tgz'))
    .map((name) => path.join(path.resolve(options.packages), name));
  expect(tarballs.length === 4, `expected four tarballs in ${options.packages}, found ${tarballs.length}`);

  const env = baseEnvironment(work);
  const profile = managerProfile(manager, work, env);
  const registry = await startRegistry(path.resolve(options.verdaccio), work);
  let offline = false;
  try {
    await qualify({ options, manager, target, work, tarballs, env, profile, registry, goOffline: async () => {
      await stopRegistry(registry);
      offline = true;
    } });
  } finally {
    if (!offline) {
      await stopRegistry(registry);
    }
  }
  report(manager);
  process.exitCode = results.every((result) => result.ok) ? 0 : 1;
}

async function qualify({ options, manager, target, work, tarballs, env, profile, goOffline }) {
  const expectedVersion = options['version-line'];
  const fromTheRegistry = env.npm_config_registry;
  expect(fromTheRegistry === REGISTRY, 'the driver only ever uses the loopback registry');

  await check('tool versions', () => {
    const versions = [];
    for (const tool of manager === 'npm' ? ['npm'] : ['npm', manager]) {
      versions.push(`${tool} ${succeed(tool, ['--version'], env, { cwd: work }).stdout.trim()}`);
    }
    return `${versions.join(', ')}, Node.js ${process.versions.node}`;
  });

  await check('publish the four packages to the loopback registry (npm, dist-tag next)', async () => {
    const token = await registryToken();
    const publishConfig = path.join(work, 'publish.npmrc');
    // Scoped to the loopback registry: npm sends this token nowhere else.
    writeFile(publishConfig, `registry=${REGISTRY}\n//${REGISTRY_HOST}/:_authToken=${token}\n`);
    const ordered = [...tarballs].sort((left, right) => Number(path.basename(left).startsWith('vsift-cli-')) - Number(path.basename(right).startsWith('vsift-cli-')));
    for (const tarball of ordered) {
      succeed('npm', ['publish', tarball, '--tag', 'next', '--ignore-scripts', '--registry', REGISTRY, '--userconfig', publishConfig], env);
    }
    fs.rmSync(publishConfig);
    return ordered.map((file) => path.basename(file)).join(', ');
  });

  // The global (or project) install, with install scripts disabled.
  const projectDirectory = profile.project ? newProject(work, `${manager} project`) : undefined;
  if (profile.project) {
    profile.installRoot = projectDirectory;
  } else {
    fs.mkdirSync(profile.installRoot, { recursive: true });
  }
  const installed = await check(`${profile.project ? 'project' : 'global'} install, scripts disabled`, () => {
    const [command, args] = profile.install;
    succeed(command, args, profile.env, { cwd: projectDirectory || work });
    return `${command} ${args.join(' ')}`;
  });

  const invocations = profile.runInstalled
    ? [{ label: profile.runInstalled.join(' '), command: profile.runInstalled[0], prefix: profile.runInstalled[1], cwd: projectDirectory }]
    : profile.shims.map((shim) => ({ label: path.basename(shim), command: shim, prefix: [], cwd: work }));
  const runInstalled = (invocation, args, extra = {}) =>
    run(invocation.command, [...invocation.prefix, ...args], profile.env, { cwd: invocation.cwd, ...extra });

  const draftDirectory = path.join(work, `drafts ${AWKWARD}`);
  const draft = path.join(draftDirectory, 'draft ü 日本.md');
  writeFile(draft, '# Report\n\nNo handoff block yet.\n');

  const platformPackages = () => findPackage(profile.installRoot, target.packageName);
  const launcherPackages = () => findPackage(profile.installRoot, 'vsift-cli');

  if (installed) {
    for (const invocation of invocations) {
      await check(`${invocation.label} --version`, () => {
        const result = runInstalled(invocation, ['--version']);
        expect(result.status === 0, `${describeStatus(result)}: ${result.stderr}`);
        expect(result.stdout.trim() === expectedVersion, `printed ${JSON.stringify(result.stdout.trim())}`);
        return result.stdout.trim();
      });
      await check(`${invocation.label} setup check --json`, () => {
        // The runner may lack the media tools, so the report may be `blocked`
        // (exit 2); the launcher must give exactly what vsift gives.
        const [directory] = platformPackages();
        expect(directory !== undefined, `${target.packageName} not found`);
        const direct = childProcess.spawnSync(path.join(directory, target.executable), ['setup', 'check', '--json'], {
          env: profile.env,
          encoding: 'utf8',
          timeout: 120_000,
        });
        const result = runInstalled(invocation, ['setup', 'check', '--json']);
        const document = parseJson(result);
        expect(document.command === 'setup.check', `command ${document.command}`);
        expect(
          result.status === direct.status && document.status === parseJson(direct).status,
          `${describeStatus(result)} status ${document.status}; vsift itself ${describeStatus(direct)}`,
        );
        return `${describeStatus(result)}, status ${document.status}, as vsift itself`;
      });
      await check(`${invocation.label}: a path argument with spaces and Unicode`, () => {
        const result = runInstalled(invocation, ['handoff', 'check', '--file', draft, '--json']);
        expect(result.status === 0, `${describeStatus(result)}: ${result.stdout.slice(0, 500)} ${result.stderr}`);
        expect(parseJson(result).status === 'complete', result.stdout.slice(0, 500));
        return 'handoff check --file read the draft';
      });
      await check(`${invocation.label}: exit statuses are vsift's`, () => {
        const [directory] = platformPackages();
        expect(directory !== undefined, `${target.packageName} not found`);
        const binary = path.join(directory, target.executable);
        const sessions = path.join(work, `sessions ${AWKWARD}`);
        const statuses = [];
        for (const args of [
          ['frobnicate'],
          ['handoff', 'check', '--file', path.join(draftDirectory, 'missing.md')],
          ['--session-root', sessions, 'ingest', path.join(draftDirectory, 'missing video.mp4')],
        ]) {
          const direct = childProcess.spawnSync(binary, args, { env: profile.env, encoding: 'utf8', timeout: 120_000 });
          const result = runInstalled(invocation, args);
          expect(
            direct.status !== 0 && result.status === direct.status,
            `${args.join(' ')}: ${describeStatus(result)}, vsift itself ${describeStatus(direct)}`,
          );
          statuses.push(result.status);
        }
        return `the same as vsift run directly: ${statuses.join(', ')}`;
      });
    }

    // The launcher itself, run by its runtime: standard input, the digest
    // check's cost and signals. Yarn's Plug'n'Play keeps the launcher in a
    // zip archive that only `yarn` can run, so these use the other managers.
    const launcherScripts = launcherPackages().map((directory) => path.join(directory, 'bin', 'vsift.cjs'));
    const runtime = manager === 'bun' ? 'bun' : process.execPath;
    if (!profile.project) {
      await check('the launcher is installed once', () => {
        expect(launcherScripts.length === 1 && fs.existsSync(launcherScripts[0]), `found ${launcherScripts.join(', ')}`);
        return launcherScripts[0];
      });
      const launcherScript = launcherScripts[0];
      if (launcherScript) {
        await qualifyLauncher({ manager, runtime, launcherScript, target, env: profile.env, work });
      }
    } else {
      await check('standard input reaches vsift (yarn vsift)', () => {
        const result = runInstalled(invocations[0], ['handoff', 'check', '--json'], { input: '# Draft ü 日本\n' });
        expect(result.status === 0 && parseJson(result).status === 'complete', `${describeStatus(result)} ${result.stderr}`);
        return 'handoff check read stdin';
      });
    }

    await qualifyDamage({ profile, invocations, runInstalled, platformPackages, target });
  }

  await check(`one-shot: ${profile.oneShot[0]} ${profile.oneShot[1].join(' ')}`, () => {
    const [command, args] = profile.oneShot;
    const oneShotDirectory = newProject(work, 'one-shot');
    const version = run(command, [...args, '--version'], profile.env, { cwd: oneShotDirectory, timeout: 240_000 });
    expect(version.status === 0 && version.stdout.trim() === expectedVersion, `${describeStatus(version)}: ${version.stdout} ${version.stderr}`);
    const setup = run(command, [...args, 'setup', 'check', '--json'], profile.env, { cwd: oneShotDirectory, timeout: 240_000 });
    expect(parseJson(setup).command === 'setup.check', setup.stdout.slice(0, 300));
    return `${version.stdout.trim()}; setup check ${describeStatus(setup)}`;
  });
  if (profile.oneShotOnBun) {
    await check(`one-shot on the Bun runtime: ${profile.oneShotOnBun[0]} ${profile.oneShotOnBun[1].join(' ')}`, () => {
      const [command, args] = profile.oneShotOnBun;
      const result = run(command, [...args, '--version'], profile.env, { cwd: work, timeout: 240_000 });
      expect(result.status === 0 && result.stdout.trim() === expectedVersion, `${describeStatus(result)}: ${result.stdout} ${result.stderr}`);
      return result.stdout.trim();
    });
  }

  await check('optional dependencies omitted: a readable failure', () => {
    const directory = newProject(work, 'omitted', profile.omitOptionalFiles, profile.omitOptionalManifest);
    const [command, args] = profile.omitOptional;
    succeed(command, args, profile.env, { cwd: directory });
    expect(findPackage(directory, target.packageName).length === 0, `${target.packageName} was installed anyway`);
    const [runCommand, runPrefix] = profile.omitOptionalRun || [
      path.join(directory, 'node_modules', '.bin', windows ? (manager === 'bun' ? 'vsift.exe' : 'vsift.cmd') : 'vsift'),
      [],
    ];
    const result = run(runCommand, [...runPrefix, '--version'], profile.env, { cwd: directory });
    expect(result.status === 127, `${describeStatus(result)}: ${result.stderr}`);
    expect(result.stderr.includes(`${target.packageName}@`) && result.stderr.includes('not installed'), result.stderr);
    assertNoStackTrace(result.stderr);
    return `exit 127: ${result.stderr.split('\n')[0]}`;
  });

  if (installed) {
    await check('offline: the installed command runs with the registry stopped', async () => {
      await goOffline();
      const invocation = invocations[0];
      const result = runInstalled(invocation, ['--version']);
      expect(result.status === 0 && result.stdout.trim() === expectedVersion, `${describeStatus(result)}: ${result.stderr}`);
      return result.stdout.trim();
    });

    await check('clean uninstall', () => {
      const [command, args] = profile.uninstall;
      succeed(command, args, profile.env, { cwd: projectDirectory || work });
      const launchers = launcherPackages();
      expect(launchers.length === 0, `the launcher is left behind: ${launchers.join(', ')}`);
      const platforms = findPackage(profile.installRoot, target.packageName);
      let note = '';
      if (manager === 'bun' && platforms.length > 0) {
        // Bun 1.2's global remove leaves the removed package's optional
        // dependency in its global folder; no command reaches it (L-092).
        note = `; Bun left ${target.packageName} in its global folder (${platforms.length} copy), with no command`;
      } else {
        expect(platforms.length === 0, `left behind: ${platforms.join(', ')}`);
      }
      for (const shim of profile.shims) {
        if (!fs.existsSync(shim)) {
          continue;
        }
        // Bun 1.2.23 on Windows leaves its `vsift.exe` shim behind; it must
        // no longer start vsift (L-092).
        expect(manager === 'bun' && windows, `${shim} is still there`);
        const stale = run(shim, ['--version'], profile.env, { cwd: work, timeout: 120_000 });
        expect(stale.status !== 0 && !stale.stdout.includes('vsift '), `the leftover ${shim} still runs vsift`);
        note += `; Bun left ${path.basename(shim)}, which no longer runs vsift (${describeStatus(stale)})`;
      }
      if (profile.runInstalled) {
        const result = runInstalled(invocations[0], ['--version']);
        expect(result.status !== 0, 'yarn vsift still runs');
        const pnp = path.join(profile.installRoot, '.pnp.cjs');
        expect(!fs.existsSync(pnp) || !fs.readFileSync(pnp, 'utf8').includes('@vsift/'), '.pnp.cjs still names @vsift');
      }
      return `${command} ${args.join(' ')}${note}`;
    });
  }
}

/** Standard input, the digest check's cost and signal forwarding, through the launcher script. */
async function qualifyLauncher({ manager, runtime, launcherScript, target, env, work }) {
  const runtimeName = manager === 'bun' ? 'Bun' : 'Node.js';
  await check(`standard input reaches vsift (${runtimeName} runs the launcher)`, () => {
    const result = childProcess.spawnSync(runtime, [launcherScript, 'handoff', 'check', '--json'], {
      env,
      input: '# Draft ü 日本\n'.repeat(1000),
      encoding: 'utf8',
      timeout: 120_000,
    });
    expect(result.status === 0 && parseJson(result).status === 'complete', `${describeStatus(result)} ${result.stderr}`);
    return 'handoff check read stdin';
  });

  await check(`the launcher's checks cost less than ${DIGEST_BUDGET_MS} ms (${runtimeName})`, () => {
    // Times the launcher's own `prepare` (version and digest checks) in the
    // installed package, seven times in one process; decision H5 keeps the
    // digest check only while it stays under the budget.
    const probe = path.join(work, 'probe.cjs');
    writeFile(
      probe,
      [
        "'use strict';",
        "const fs = require('node:fs');",
        "const path = require('node:path');",
        'const script = process.env.VSIFT_QUALIFY_LAUNCHER;',
        'const launcher = require(script);',
        "const root = path.join(path.dirname(script), '..');",
        'const resolve = (request) => require.resolve(request, { paths: [root] });',
        'const times = [];',
        'let executable;',
        'for (let i = 0; i < 7; i += 1) {',
        '  const started = process.hrtime.bigint();',
        '  executable = launcher.prepare(root, process.platform, process.arch, resolve).executablePath;',
        '  times.push(Number(process.hrtime.bigint() - started) / 1e6);',
        '}',
        'times.sort((a, b) => a - b);',
        'process.stdout.write(JSON.stringify({ times, size: fs.statSync(executable).size }));',
        '',
      ].join('\n'),
    );
    const result = childProcess.spawnSync(runtime, [probe], {
      // The library the command runs, so the probe times its checks without starting vsift.
      env: { ...env, VSIFT_QUALIFY_LAUNCHER: path.join(path.dirname(launcherScript), '..', 'lib', 'launcher.cjs') },
      encoding: 'utf8',
      timeout: 120_000,
    });
    expect(result.status === 0, `${describeStatus(result)} ${result.stderr}`);
    const { times, size } = JSON.parse(result.stdout);
    const median = times[Math.floor(times.length / 2)];
    const summary = `median ${median.toFixed(1)} ms (min ${times[0].toFixed(1)}, max ${times[times.length - 1].toFixed(1)}) for ${size} bytes`;
    expect(median < DIGEST_BUDGET_MS, summary);
    return summary;
  });

  // `handoff check` without --file reads stdin until it ends; the pipe stays
  // open, so vsift waits, like any command in progress. A short command keeps
  // the operating system's default for an interruption.
  const start = (options) =>
    childProcess.spawn(runtime, [launcherScript, 'handoff', 'check', '--json'], { env, stdio: ['pipe', 'pipe', 'pipe'], ...options });
  // The launcher's end, and whether vsift outlived it: vsift holds the same
  // stdout and stderr pipes, so they reach their end (the child's `close`)
  // only once no process is left writing to them.
  const outcome = (child) =>
    new Promise((resolve) => {
      const timer = setTimeout(() => child.kill('SIGKILL'), 60_000);
      child.on('exit', (code, signal) => {
        clearTimeout(timer);
        const orphanTimer = setTimeout(() => resolve({ code, signal, orphan: true }), 5_000);
        child.on('close', () => {
          clearTimeout(orphanTimer);
          resolve({ code, signal, orphan: false });
        });
      });
    });

  if (windows) {
    await check(`console Ctrl-Break ends vsift and the launcher with its status (${runtimeName})`, async () => {
      // With every stream a pipe and windowsHide, the runtime gets a console
      // of its own (CREATE_NO_WINDOW), which vsift shares; the event goes to
      // that console only. The driver ignores Ctrl-Break meanwhile anyway.
      const ignore = () => {};
      process.on('SIGBREAK', ignore);
      const child = start({ windowsHide: true });
      const ended = outcome(child);
      await new Promise((resolve) => setTimeout(resolve, 5000));
      const sent = childProcess.spawnSync(
        'powershell.exe',
        ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', SEND_CONSOLE_CONTROL, '-ProcessId', String(child.pid), '-Event', 'CtrlBreak'],
        { encoding: 'utf8', timeout: 60_000 },
      );
      expect(sent.status === 0, `send-console-ctrl exited ${sent.status}`);
      const result = await ended;
      process.removeListener('SIGBREAK', ignore);
      expect(result.code === STATUS_CONTROL_C_EXIT, `exit ${result.code} signal ${result.signal}`);
      expect(!result.orphan, 'vsift still held the output pipes 5 s after the launcher ended');
      return `exit 0x${result.code.toString(16)}, no orphan`;
    });
    return;
  }
  for (const signal of ['SIGTERM', 'SIGINT']) {
    await check(`${signal} to the launcher alone reaches vsift, and the launcher ends with it (${runtimeName})`, async () => {
      const child = start({});
      const ended = outcome(child);
      await new Promise((resolve) => setTimeout(resolve, 5000));
      child.kill(signal);
      const result = await ended;
      // Node.js re-raises the signal; a runtime that cannot is left the
      // shell's form of it, 128 plus the signal number.
      const shellForm = 128 + os.constants.signals[signal];
      expect(
        result.signal === signal || (manager === 'bun' && result.code === shellForm),
        `exit ${result.code} signal ${result.signal}`,
      );
      expect(!result.orphan, 'vsift still held the output pipes 5 s after the launcher ended');
      return `ended by ${result.signal || `exit ${result.code}`}, no orphan`;
    });
  }
}

/** A replaced, damaged or non-executable binary and a mismatched version are refused readably. */
async function qualifyDamage({ invocations, runInstalled, platformPackages, target }) {
  const directories = platformPackages();
  const ok = await check(`the platform package ${target.packageName} is installed`, () => {
    expect(directories.length >= 1, 'not found');
    return `${directories.length} ${directories.length === 1 ? 'copy' : 'copies'} in the install`;
  });
  if (!ok) {
    return;
  }
  const invocation = invocations[0];
  // Every copy is changed together (pnpm links its store into the install).
  const files = directories.map((directory) => path.join(directory, target.executable));
  const manifests = directories.map((directory) => path.join(directory, 'package.json'));
  const refused = (expectedStatus, fragment) => {
    const result = runInstalled(invocation, ['--version']);
    expect(result.status === expectedStatus, `${describeStatus(result)}: ${result.stderr}`);
    expect(result.stderr.includes(fragment), result.stderr);
    assertNoStackTrace(result.stderr);
    return `exit ${result.status}: ${result.stderr.trim().split('\n')[0]}`;
  };
  const withChange = (paths, change, action) => {
    const saved = paths.map((file) => ({ file, bytes: fs.readFileSync(file), mode: fs.statSync(file).mode }));
    try {
      for (const file of paths) {
        change(file);
      }
      return action();
    } finally {
      for (const { file, bytes, mode } of saved) {
        fs.chmodSync(file, 0o644);
        fs.writeFileSync(file, bytes);
        fs.chmodSync(file, mode);
      }
    }
  };
  await check('a damaged binary (one byte changed) is refused', () =>
    withChange(files, (file) => {
      const bytes = fs.readFileSync(file);
      bytes[bytes.length - 1] ^= 0xff;
      fs.chmodSync(file, 0o755);
      fs.writeFileSync(file, bytes);
    }, () => refused(126, 'does not match the digest')));
  await check('a replaced binary (another size) is refused', () =>
    withChange(files, (file) => {
      fs.chmodSync(file, 0o755);
      fs.writeFileSync(file, '#!/bin/sh\necho replaced\n');
    }, () => refused(126, 'does not match the digest')));
  await check('a platform package of another version is refused', () =>
    withChange(manifests, (file) => {
      const manifest = JSON.parse(fs.readFileSync(file, 'utf8'));
      manifest.version = '0.0.1';
      fs.writeFileSync(file, JSON.stringify(manifest));
    }, () => refused(126, 'must be the same version')));
  if (!windows) {
    await check('a binary that cannot be executed is refused', () =>
      withChange(files, (file) => fs.chmodSync(file, 0o644), () => refused(126, 'could not start')));
  }
  await check('the restored install runs again', () => {
    const result = runInstalled(invocation, ['--version']);
    expect(result.status === 0, `${describeStatus(result)}: ${result.stderr}`);
    return result.stdout.trim();
  });
}

main().catch((error) => {
  results.push({ name: 'driver', ok: false, detail: error instanceof QualificationError ? error.message : `${error && error.stack}`, ms: 0 });
  try {
    report(process.argv[process.argv.indexOf('--manager') + 1] || 'unknown');
  } finally {
    process.exitCode = 1;
  }
});
