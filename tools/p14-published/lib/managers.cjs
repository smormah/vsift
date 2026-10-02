'use strict';

// The four package managers of the R0 install matrix (ADR 0023 decision H6)
// against the real registry. What differs between them: where a global (or, for
// Yarn, project) install puts the command, how to install, run once, omit
// optional dependencies and uninstall. The profiles follow the P13 npm
// qualification driver (npm/qualification/qualify.cjs), with the loopback
// registry replaced by the public one and the dist-tag by an exact version.

const path = require('node:path');

const { REGISTRY, QualificationError, windows } = require('./common.cjs');
const { pathKey, splitPath, joinPath } = require('./scrub.cjs');

const MANAGERS = Object.freeze(['npm', 'pnpm', 'yarn', 'bun']);

/** A folder name with spaces, accents and CJK letters: every install and argument path below it must survive every manager and shim. */
const AWKWARD = 'vsift qualification ü 日本';

/**
 * The directories `vsift` shims may be in, as a Windows user's shell finds
 * them: PowerShell runs a `.ps1` shim before a `.cmd` one.
 */
function shimFiles(directory, shimNames) {
  return shimNames.map((name) => path.join(directory, name));
}

/**
 * The profile of `manager` for installing `vsift-cli@<version>` into `root`.
 *
 * - `env` is the environment of every command of this manager (the real
 *   registry, caches under the job's work folder, scripts disabled).
 * - `install`, `oneShot`, `omitOptional` and `uninstall` are `[command, args]`.
 * - `shims` are the files a user's shell would run as `vsift`, each with the
 *   kind of shim it is; `runInstalled` is the command for a manager that has no
 *   shim (Yarn runs it as `yarn vsift`).
 */
function managerProfile(manager, version, root, env, work) {
  const spec = `vsift-cli@${version}`;
  const shimNames = windows ? ['vsift.ps1', 'vsift.cmd'] : ['vsift'];
  const kindOf = (file) => (file.endsWith('.cmd') ? 'cmd' : file.endsWith('.ps1') ? 'ps1' : file.endsWith('.exe') ? 'exe' : 'sh');
  const label = (file) => path.basename(file);
  const shimsIn = (directory) => shimFiles(directory, shimNames).map((file) => ({ file, kind: kindOf(file), label: label(file) }));
  switch (manager) {
    case 'npm':
      return {
        manager,
        root,
        env: { ...env, npm_config_prefix: root },
        install: ['npm', ['install', '--global', '--ignore-scripts', spec]],
        shims: shimsIn(windows ? root : path.join(root, 'bin')),
        oneShot: ['npx', ['--yes', spec]],
        omitOptional: ['npm', ['install', '--ignore-scripts', '--omit=optional', spec]],
        uninstall: ['npm', ['uninstall', '--global', 'vsift-cli']],
      };
    case 'pnpm': {
      // pnpm 12 puts global commands in $PNPM_HOME/bin and refuses to install
      // globally unless that folder is on PATH.
      const bin = path.join(root, 'bin');
      const key = pathKey(env);
      return {
        manager,
        root,
        env: { ...env, PNPM_HOME: root, [key]: joinPath([bin, root, ...splitPath(env[key] || '')]) },
        install: ['pnpm', ['add', '--global', '--ignore-scripts', spec]],
        shims: shimsIn(bin),
        oneShot: ['pnpm', ['dlx', spec]],
        omitOptional: ['pnpm', ['install', '--ignore-scripts', '--no-optional']],
        omitOptionalManifest: { dependencies: { 'vsift-cli': version } },
        uninstall: ['pnpm', ['remove', '--global', 'vsift-cli']],
      };
    }
    case 'yarn':
      // Yarn 4 has no global install; a project install is its equivalent.
      return {
        manager,
        root,
        project: true,
        env,
        install: ['yarn', ['add', spec]],
        shims: [],
        runInstalled: ['yarn', ['vsift']],
        oneShot: ['yarn', ['dlx', '--quiet', '--package', spec, 'vsift']],
        omitOptional: ['yarn', ['add', spec]],
        // Yarn cannot omit optional dependencies; installing for another
        // platform leaves this one's package out in the same way.
        // The exemption of install.md section 2 keeps Yarn's one-day gate out
        // of this check, which is about the omitted package and not the gate.
        omitOptionalFiles: {
          '.yarnrc.yml':
            'supportedArchitectures:\n  os:\n    - aix\n  cpu:\n    - ppc64\nnpmPreapprovedPackages:\n  - vsift-cli\n  - "@vsift/*"\n',
        },
        omitOptionalRun: ['yarn', ['vsift']],
        uninstall: ['yarn', ['remove', 'vsift-cli']],
      };
    case 'bun': {
      const bin = path.join(root, 'bin');
      const key = pathKey(env);
      return {
        manager,
        root,
        env: {
          ...env,
          BUN_INSTALL_GLOBAL_DIR: path.join(root, 'global'),
          BUN_INSTALL_BIN: bin,
          [key]: joinPath([bin, ...splitPath(env[key] || '')]),
        },
        install: ['bun', ['add', '--global', '--ignore-scripts', '--registry', REGISTRY, spec]],
        // Bun links an executable (an .exe on Windows), not a script shim.
        shims: [{ file: path.join(bin, windows ? 'vsift.exe' : 'vsift'), kind: windows ? 'exe' : 'sh', label: windows ? 'vsift.exe' : 'vsift' }],
        oneShot: ['bunx', [spec]],
        oneShotOnBun: ['bunx', ['--bun', spec]],
        omitOptional: ['bun', ['install', '--ignore-scripts', '--omit', 'optional', '--registry', REGISTRY]],
        omitOptionalManifest: { dependencies: { 'vsift-cli': version } },
        uninstall: ['bun', ['remove', '--global', 'vsift-cli']],
      };
    }
    default:
      throw new QualificationError(`unknown manager ${manager}`);
  }
}

/**
 * The environment of every package-manager command: the real registry, caches
 * and stores inside `work`, no telemetry, no install scripts. `user` holds the
 * per-user folder overrides (HOME and the like) of the job's fresh state.
 */
function managerEnvironment(base, work, label) {
  return {
    ...base,
    npm_config_registry: REGISTRY,
    NPM_CONFIG_REGISTRY: REGISTRY,
    npm_config_update_notifier: 'false',
    npm_config_audit: 'false',
    npm_config_fund: 'false',
    npm_config_cache: path.join(work, `npm cache ${label}`),
    // pnpm's store and metadata cache in this job's folder, so no earlier
    // run's metadata for the same version can stand in for this one's.
    npm_config_store_dir: path.join(work, `pnpm store ${label}`),
    pnpm_config_store_dir: path.join(work, `pnpm store ${label}`),
    npm_config_cache_dir: path.join(work, `pnpm cache ${label}`),
    pnpm_config_cache_dir: path.join(work, `pnpm cache ${label}`),
    YARN_NPM_REGISTRY_SERVER: REGISTRY.slice(0, -1),
    YARN_ENABLE_TELEMETRY: '0',
    YARN_ENABLE_SCRIPTS: 'false',
    YARN_ENABLE_IMMUTABLE_INSTALLS: 'false',
    YARN_ENABLE_GLOBAL_CACHE: 'false',
    YARN_CACHE_FOLDER: path.join(work, `yarn cache ${label}`),
    YARN_GLOBAL_FOLDER: path.join(work, `yarn global ${label}`),
    BUN_CONFIG_REGISTRY: REGISTRY,
    BUN_INSTALL_CACHE_DIR: path.join(work, `bun cache ${label}`),
    DO_NOT_TRACK: '1',
  };
}

module.exports = { AWKWARD, MANAGERS, managerEnvironment, managerProfile };
