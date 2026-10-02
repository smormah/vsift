'use strict';

// Assembles the four npm packages for the *local* upgrade mode (RQ-04, the
// `P14 local upgrade` workflow): the pull request's own executables under a
// throwaway higher version, so a package manager can upgrade a real registry's
// published install to something that exists nowhere else.
//
//   node tools/p14-published/assemble-local-packages.cjs --version <higher version> \
//     --skeleton-version <published version> --repo <checkout> --work <scratch> --out <empty folder> \
//     --binary linux-x64=<vsift> --binary darwin-arm64=<vsift> --binary win32-x64=<vsift.exe>
//
// This is **not** the release assembly (`vsift-release npm`, which needs the
// archives, their notices and SBOMs). The launcher is the checkout's own
// (`npm/vsift-cli/`, with its skill and licences); the three platform packages
// are the published `--skeleton-version` packages (read from the real registry,
// never changed there) with their version and executable replaced; the digests
// file is computed here from the executables. Everything else follows the
// layout `launcher.cjs` checks, so what it proves is the upgrade path and the
// launcher's checks on a newer package, not the release packaging.

const fs = require('node:fs');
const path = require('node:path');

const {
  PACKAGES,
  QualificationError,
  TARGETS,
  cleanEnvironment,
  expect,
  parseArguments,
  sha256File,
  succeed,
  writeFile,
} = require('./lib/common.cjs');
const { npmReadEnvironment, parseVersion, tarballName } = require('./lib/release.cjs');

const DIGESTS_FORMAT = 'vsift-platform-digests/1';

/** The platform packages by `<platform>-<arch>` key. */
const PLATFORM_PACKAGES = Object.freeze(
  Object.fromEntries(Object.values(TARGETS).map((target) => [target.packageName.replace('@vsift/', ''), target])),
);

/** The launcher manifest at `version`, every optional dependency pinned to it. */
function launcherManifest(manifest, version) {
  const optionalDependencies = {};
  for (const name of Object.keys(manifest.optionalDependencies || {})) {
    optionalDependencies[name] = version;
  }
  expect(
    JSON.stringify(Object.keys(optionalDependencies).sort()) === JSON.stringify(PACKAGES.slice(1).sort()),
    `the launcher lists ${Object.keys(optionalDependencies).join(', ')} as optional dependencies`,
  );
  expect(!manifest.scripts, 'the launcher manifest has scripts');
  return { ...manifest, version, optionalDependencies };
}

/** The `platform-digests.json` the launcher reads (lib/launcher.cjs). */
function platformDigests(version, binaries) {
  const packages = {};
  for (const [key, file] of Object.entries(binaries)) {
    const target = PLATFORM_PACKAGES[key];
    expect(target !== undefined, `unknown platform ${key}`);
    packages[target.packageName] = { file: target.executable, size: fs.statSync(file).size, sha256: sha256File(file) };
  }
  return { format: DIGESTS_FORMAT, version, packages };
}

function copyTree(from, to) {
  fs.mkdirSync(to, { recursive: true });
  for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
    const source = path.join(from, entry.name);
    if (entry.isDirectory()) {
      copyTree(source, path.join(to, entry.name));
    } else if (entry.isFile()) {
      fs.copyFileSync(source, path.join(to, entry.name));
    } else {
      throw new QualificationError(`${source} is not a regular file or a directory`);
    }
  }
}

function main() {
  const argv = process.argv.slice(2);
  // `--binary` repeats, which the shared parser does not allow.
  const binaries = {};
  const rest = [];
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === '--binary') {
      const [key, file] = String(argv[index + 1]).split('=');
      expect(PLATFORM_PACKAGES[key] !== undefined && file, `--binary takes <platform>-<arch>=<file>, one of ${Object.keys(PLATFORM_PACKAGES).join(', ')}`);
      binaries[key] = path.resolve(file);
      index += 1;
    } else {
      rest.push(argv[index]);
    }
  }
  const options = parseArguments(rest, { required: ['version', 'skeleton-version', 'repo', 'work', 'out'] });
  const { version } = options;
  parseVersion(version);
  parseVersion(options['skeleton-version']);
  expect(Object.keys(binaries).length === 3, 'give --binary for linux-x64, darwin-arm64 and win32-x64');
  for (const file of Object.values(binaries)) {
    expect(fs.statSync(file).isFile(), `${file} is not a file`);
  }
  const repo = path.resolve(options.repo);
  const work = path.resolve(options.work);
  const out = path.resolve(options.out);
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  fs.mkdirSync(out, { recursive: true });
  expect(fs.readdirSync(out).length === 0, `${out} is not empty`);
  const env = cleanEnvironment(process.env);
  const read = npmReadEnvironment(env, work);

  const packages = path.join(work, 'packages');
  // The launcher.
  const launcher = path.join(packages, 'vsift-cli');
  const source = path.join(repo, 'npm', 'vsift-cli');
  const manifest = launcherManifest(JSON.parse(fs.readFileSync(path.join(source, 'package.json'), 'utf8')), version);
  writeFile(path.join(launcher, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  for (const file of ['README.md', path.join('bin', 'vsift.cjs'), path.join('lib', 'launcher.cjs')]) {
    fs.mkdirSync(path.dirname(path.join(launcher, file)), { recursive: true });
    fs.copyFileSync(path.join(source, file), path.join(launcher, file));
  }
  for (const licence of ['LICENSE', 'LICENSE-APACHE', 'LICENSE-MIT']) {
    fs.copyFileSync(path.join(repo, licence), path.join(launcher, licence));
  }
  copyTree(path.join(repo, 'skills', 'vsift'), path.join(launcher, 'skills', 'vsift'));
  writeFile(path.join(launcher, 'platform-digests.json'), `${JSON.stringify(platformDigests(version, binaries), null, 2)}\n`);

  // The platform packages: the published skeleton with the new version and executable.
  const skeletons = path.join(work, 'skeletons');
  fs.mkdirSync(skeletons, { recursive: true });
  for (const [key, target] of Object.entries(PLATFORM_PACKAGES)) {
    const name = target.packageName;
    succeed('npm', ['pack', `${name}@${options['skeleton-version']}`, '--pack-destination', skeletons, '--ignore-scripts'], read, { cwd: work });
    const tarball = path.join(skeletons, tarballName(name, options['skeleton-version']));
    const extracted = path.join(skeletons, key);
    fs.mkdirSync(extracted, { recursive: true });
    succeed('tar', ['-xzf', tarball, '-C', extracted], env, { cwd: work });
    const directory = path.join(packages, `vsift-${key}`);
    copyTree(path.join(extracted, 'package'), directory);
    const platformManifest = JSON.parse(fs.readFileSync(path.join(directory, 'package.json'), 'utf8'));
    expect(platformManifest.name === name && !platformManifest.scripts, `the skeleton of ${name} is not the platform package`);
    platformManifest.version = version;
    writeFile(path.join(directory, 'package.json'), `${JSON.stringify(platformManifest, null, 2)}\n`);
    fs.rmSync(path.join(directory, target.executable), { force: true });
    fs.copyFileSync(binaries[key], path.join(directory, target.executable));
    fs.chmodSync(path.join(directory, target.executable), 0o755);
  }

  // Pack each directory.
  for (const directory of fs.readdirSync(packages)) {
    succeed('npm', ['pack', '--ignore-scripts', '--pack-destination', out], read, { cwd: path.join(packages, directory) });
  }
  const tarballs = fs.readdirSync(out).sort();
  expect(
    JSON.stringify(tarballs) === JSON.stringify(PACKAGES.map((name) => tarballName(name, version)).sort()),
    `packed ${tarballs.join(', ')}`,
  );
  process.stdout.write(`${tarballs.map((name) => `${name} ${fs.statSync(path.join(out, name)).size} bytes`).join('\n')}\n`);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
    process.exitCode = 1;
  }
}

module.exports = { PLATFORM_PACKAGES, launcherManifest, platformDigests };
