'use strict';

// RQ-01: a clean install of the published packages from the real registry
// (P14 PR 2; ADR 0024; docs/planning/p14-qualification.md section 2).
//
//   node tools/p14-published/clean-install.cjs --manager <npm|pnpm|yarn|bun> \
//     --version <published version> --commit <the tag's 40 hex digits> \
//     --work <scratch folder> --repo <checkout of the tools and fixtures> \
//     [--tag-skill <the tag's skills/vsift>]
//
// One job runs one package manager on one operating system. Everything it does
// to install and run VSift happens in a scrubbed environment: no Rust
// toolchain, Git or Python on `PATH` (asserted before anything is installed),
// a fresh per-user state, no token and no package-manager setting of the
// runner's. The registry is `https://registry.npmjs.org/` and nothing else.
//
// What a pass shows is that the published names, scope, provenance and the
// documented commands work from the real registry with no hidden dependency on
// a toolchain, a checkout or a developer's PATH. It does not show a user's
// proxy or policy, a never-used operating system or Smart App Control, and
// hosted runners are not clean machines (the plan says so).
//
// Nothing here publishes, tags or logs in. `gh` gets the job's default
// read-only token and nothing else of a secret kind.

const fs = require('node:fs');
const path = require('node:path');

const {
  PACKAGES,
  QualificationError,
  Recorder,
  REGISTRY,
  assertNoStackTrace,
  cleanEnvironment,
  compareTrees,
  currentTarget,
  describeStatus,
  expect,
  findPackage,
  githubEnvironment,
  parseArguments,
  parseJson,
  run,
  succeed,
  windows,
  writeFile,
} = require('./lib/common.cjs');
const { DRAFT, MARKER, hostileArguments, hostileFileNames } = require('./lib/hostile.cjs');
const { AWKWARD, MANAGERS, managerEnvironment, managerProfile } = require('./lib/managers.cjs');
const { npmReadEnvironment, npmView, tarballName, verifyAttestation, parseVersion } = require('./lib/release.cjs');
const { checkSec02 } = require('./lib/sec02.cjs');
const { describeResolution, namesOf, resolveOnPath, scrubbedEnvironment, withPathPrefix } = require('./lib/scrub.cjs');

/** Hours a version must be old before Yarn's default one-day gate no longer applies. */
const YARN_GATE_HOURS = 24;

function userFolders(root) {
  const folders = {
    HOME: root,
    USERPROFILE: root,
    APPDATA: path.join(root, 'roaming'),
    LOCALAPPDATA: path.join(root, 'local'),
    XDG_CONFIG_HOME: path.join(root, 'config'),
    XDG_CACHE_HOME: path.join(root, 'cache'),
    XDG_DATA_HOME: path.join(root, 'data'),
  };
  for (const folder of Object.values(folders)) {
    fs.mkdirSync(folder, { recursive: true });
  }
  return folders;
}

async function main() {
  const options = parseArguments(process.argv.slice(2), {
    required: ['manager', 'version', 'commit', 'work', 'repo'],
    optional: ['tag-skill'],
  });
  const { manager, version, commit } = options;
  expect(MANAGERS.includes(manager), `--manager must be one of ${MANAGERS.join(', ')}`);
  parseVersion(version);
  expect(/^[0-9a-f]{40}$/.test(commit), '--commit must be 40 hex digits');
  const target = currentTarget();
  // Bun 1.2.23 fails on Windows with "InvalidWtf8" in a non-ASCII install or
  // cache folder (L-092): its folders keep only the spaces. The arguments
  // vsift receives keep the non-ASCII name on every job.
  const label = windows && manager === 'bun' ? 'vsift qualification' : AWKWARD;
  const work = path.join(path.resolve(options.work), label, manager);
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const ascii = path.join(path.resolve(options.work), 'ascii', manager);
  fs.rmSync(ascii, { recursive: true, force: true });
  fs.mkdirSync(ascii, { recursive: true });
  const repo = path.resolve(options.repo);
  const video = path.join(repo, 'fixtures', 'corpus', 'generated', 'F10.mp4');
  const transcript = path.join(repo, 'fixtures', 'corpus', 'transcripts', 'F10.srt');

  const recorder = new Recorder();
  const title = `RQ-01 clean install of vsift-cli ${version} from the real registry: ${manager} on ${target.label}`;

  // ------------------------------------------------------------ environment
  const hidden = namesOf(['rust', 'git', 'python']);
  const before = describeResolution(hidden, process.env[Object.keys(process.env).find((key) => key.toLowerCase() === 'path') || 'PATH'] || '');
  recorder.observe('what the runner carries before the scrub', before.join('; '));

  let scrubbed;
  const scrubbedOk = await recorder.check('scrubbed environment: cargo, rustc, rustup, git and python do not resolve', () => {
    const base = {
      ...cleanEnvironment(process.env),
      ...userFolders(path.join(ascii, 'user')),
      TMPDIR: path.join(ascii, 'tmp'),
      TEMP: path.join(ascii, 'tmp'),
      TMP: path.join(ascii, 'tmp'),
    };
    fs.mkdirSync(base.TMPDIR, { recursive: true });
    scrubbed = scrubbedEnvironment(base, hidden, { farm: path.join(ascii, 'path-farm') });
    const after = describeResolution(hidden, scrubbed.env[scrubbed.key]);
    expect(after.every((line) => line.endsWith('not found')), after.join('; '));
    const notes = [];
    if (scrubbed.dropped.length > 0) {
      notes.push(`dropped ${scrubbed.dropped.length} PATH director${scrubbed.dropped.length === 1 ? 'y' : 'ies'}`);
    }
    if (scrubbed.filtered.length > 0) {
      notes.push(`filtered ${scrubbed.filtered.length}`);
    }
    return `${after.join('; ')} (${notes.join(', ') || 'nothing to remove'})`;
  });
  if (!scrubbedOk) {
    recorder.finish(title);
    return;
  }
  const baseEnv = managerEnvironment(scrubbed.env, work, label);
  const profile = managerProfile(manager, version, path.join(work, `${manager} install ${label}`), baseEnv, work);
  const env = profile.env;

  await recorder.check('tool versions and the registry', () => {
    const lines = [];
    for (const tool of manager === 'npm' ? ['npm'] : ['npm', manager]) {
      lines.push(`${tool} ${succeed(tool, ['--version'], env, { cwd: work }).stdout.trim()}`);
    }
    const registry = succeed('npm', ['config', 'get', 'registry'], env, { cwd: work }).stdout.trim();
    expect(registry === REGISTRY, `npm's registry is ${registry}, not ${REGISTRY}`);
    return `${lines.join(', ')}, Node.js ${process.versions.node}, ${process.platform} ${process.arch}; registry ${registry}`;
  });

  let ageHours;
  await recorder.check(`${PACKAGES.join(', ')} are on the registry at ${version} with no lifecycle script`, () => {
    const notes = [];
    for (const name of PACKAGES) {
      const published = npmView(`${name}@${version}`, 'version', env, work);
      expect(published === version, `${name}@${version} is not on the registry (it reports ${JSON.stringify(published)})`);
      const scripts = npmView(`${name}@${version}`, 'scripts', env, work);
      expect(scripts === null || Object.keys(scripts).length === 0, `${name}@${version} declares scripts: ${JSON.stringify(scripts)}`);
    }
    const times = npmView('vsift-cli', 'time', env, work);
    ageHours = (Date.now() - Date.parse(times[version])) / 3_600_000;
    notes.push(`published ${times[version]} (${ageHours.toFixed(1)} h ago)`);
    return notes.join('; ');
  });

  // ------------------------------------------------------------ the install
  const projectDirectory = profile.project ? path.join(work, `${manager} project ${label}`) : undefined;
  if (profile.project) {
    writeFile(path.join(projectDirectory, 'package.json'), `${JSON.stringify({ name: 'vsift-qualification', version: '0.0.0', private: true }, null, 2)}\n`);
    profile.root = projectDirectory;
  } else {
    fs.mkdirSync(profile.root, { recursive: true });
  }
  const installDirectory = projectDirectory || work;
  let yarnGate = 'not applicable';
  const installed = await recorder.check(`${profile.project ? 'project' : 'global'} install of vsift-cli@${version}, scripts disabled`, () => {
    const [command, args] = profile.install;
    const first = run(command, args, env, { cwd: installDirectory });
    if (first.status === 0) {
      if (manager === 'yarn') {
        yarnGate = `not met: the install was not held back (${ageHours.toFixed(1)} h old)`;
      }
      return `${command} ${args.join(' ')}`;
    }
    const output = `${first.stdout}${first.stderr}`;
    if (manager === 'yarn' && /quarantin/i.test(output)) {
      // Yarn 4 holds back any version published less than a day ago; the
      // guide (install.md section 2) names the exemption. The gate itself is
      // the finding the plan expects to see on the real registry.
      expect(ageHours < YARN_GATE_HOURS + 2, `Yarn quarantined a version published ${ageHours.toFixed(1)} h ago`);
      writeFile(path.join(projectDirectory, '.yarnrc.yml'), 'npmPreapprovedPackages:\n  - vsift-cli\n  - "@vsift/*"\n');
      succeed(command, args, env, { cwd: installDirectory });
      yarnGate = `met: Yarn held the ${ageHours.toFixed(1)} h old version back ("quarantined"); the npmPreapprovedPackages exemption of install.md section 2 let it through`;
      return `${command} ${args.join(' ')} after the documented exemption (${yarnGate})`;
    }
    throw new QualificationError(
      `${command} ${args.join(' ')} exited ${first.status}: ${(first.stderr || '').trim().slice(-1500)} ${(first.stdout || '').trim().slice(-500)}`,
    );
  });
  if (manager === 'yarn') {
    recorder.observe("Yarn's one-day gate (npmMinimalAgeGate)", yarnGate);
  }

  const platformPackages = () => findPackage(profile.root, target.packageName);
  const launcherPackages = () => findPackage(profile.root, 'vsift-cli');
  const shims = profile.runInstalled
    ? [{ label: profile.runInstalled.join(' '), kind: 'yarn', file: null }]
    : profile.shims;
  /** Runs the installed command through `shim`. */
  const invoke = (shim, args, extra = {}) => {
    // Yarn finds its project by walking up from the working directory, so
    // everything it runs lives inside the project folder.
    const cwd = extra.cwd || projectDirectory || work;
    const environment = extra.env || env;
    if (shim.kind === 'yarn') {
      return run('yarn', ['vsift', ...args], environment, { cwd, input: extra.input });
    }
    return run(shim.file, args, environment, { cwd, input: extra.input });
  };
  const gated = shims.filter((shim) => shim.kind !== 'cmd');
  let platformExecutable;
  const direct = (args, extra = {}) => {
    if (platformExecutable === undefined) {
      const [directory] = platformPackages();
      expect(directory !== undefined, `${target.packageName} was not found in the install`);
      platformExecutable = path.join(directory, target.executable);
    }
    return run(platformExecutable, args, extra.env || env, { cwd: extra.cwd || work, timeout: 120_000 });
  };

  if (installed) {
    for (const shim of shims) {
      const observed = shim.kind === 'cmd';
      const name = (text) => `${shim.label}${observed ? ' (cmd.exe shim, observed)' : ''}: ${text}`;
      await recorder.check(name('--version names the version and the tag commit'), () => {
        const result = invoke(shim, ['--version']);
        const expected = `vsift ${version} (${commit.slice(0, 12)})`;
        expect(result.status === 0 && result.stdout.trim() === expected, `${describeStatus(result)} printed ${JSON.stringify(result.stdout.trim())}; expected ${JSON.stringify(expected)}: ${result.stderr}`);
        return result.stdout.trim();
      });
      await recorder.check(name('setup check --json is vsift\'s own answer'), () => {
        const result = invoke(shim, ['setup', 'check', '--json']);
        const documented = direct(['setup', 'check', '--json']);
        const document = parseJson(result);
        expect(document.command === 'setup.check', `command ${document.command}`);
        expect(
          result.status === documented.status && document.status === parseJson(documented).status,
          `${describeStatus(result)} status ${document.status}; vsift itself ${describeStatus(documented)}`,
        );
        return `${describeStatus(result)}, status ${document.status}, as vsift itself`;
      });
    }

    // The package contents: the skill is the tag's, byte for byte.
    await recorder.check("the installed package's skill is the tag's skills/vsift, byte for byte", () => {
      if (manager === 'yarn') {
        return "not compared: Yarn Plug'n'Play keeps the package in a zip archive (the archive job and the other managers compare it)";
      }
      expect(options['tag-skill'], 'the tag checkout (--tag-skill) was not given');
      const [directory] = launcherPackages();
      expect(directory !== undefined, 'vsift-cli was not found in the install');
      const comparison = compareTrees(path.join(directory, 'skills', 'vsift'), path.resolve(options['tag-skill']));
      expect(comparison.equal, comparison.problems.slice(0, 5).join('; '));
      return `${comparison.files} files identical`;
    });

    await checkHostile({ recorder, shims, invoke, direct, base: projectDirectory || work, label: AWKWARD });
    await checkSec02({
      recorder,
      invoke: (args, extra) => invoke(gated[0], args, extra),
      env,
      base: projectDirectory || work,
      video,
      transcript,
    });
  }

  // ------------------------------------------------------------ one-shot
  await recorder.check(`one-shot: ${profile.oneShot[0]} ${profile.oneShot[1].join(' ')} --version`, () => {
    const [command, args] = profile.oneShot;
    const directory = projectDirectory || path.join(work, 'one-shot');
    fs.mkdirSync(directory, { recursive: true });
    const expected = `vsift ${version} (${commit.slice(0, 12)})`;
    const result = run(command, [...args, '--version'], env, { cwd: directory, timeout: 300_000 });
    expect(result.status === 0 && result.stdout.trim() === expected, `${describeStatus(result)}: ${result.stdout} ${result.stderr}`);
    const setup = run(command, [...args, 'setup', 'check', '--json'], env, { cwd: directory, timeout: 300_000 });
    expect(parseJson(setup).command === 'setup.check', setup.stdout.slice(0, 300));
    return `${result.stdout.trim()}; setup check ${describeStatus(setup)}`;
  });
  if (profile.oneShotOnBun) {
    await recorder.check(`one-shot on the Bun runtime: ${profile.oneShotOnBun[0]} ${profile.oneShotOnBun[1].join(' ')} --version`, () => {
      const [command, args] = profile.oneShotOnBun;
      const result = run(command, [...args, '--version'], env, { cwd: work, timeout: 300_000 });
      expect(result.status === 0 && result.stdout.trim() === `vsift ${version} (${commit.slice(0, 12)})`, `${describeStatus(result)}: ${result.stdout} ${result.stderr}`);
      return result.stdout.trim();
    });
  }

  // ------------------------------------------------------------ optional dependencies omitted
  await recorder.check('optional dependencies omitted: a readable failure (exit 127) naming the package and the targets', () => {
    const directory = path.join(work, `omitted ${label}`);
    writeFile(
      path.join(directory, 'package.json'),
      `${JSON.stringify({ name: 'vsift-qualification', version: '0.0.0', private: true, ...(profile.omitOptionalManifest || {}) }, null, 2)}\n`,
    );
    for (const [file, text] of Object.entries(profile.omitOptionalFiles || {})) {
      writeFile(path.join(directory, file), text);
    }
    const [command, args] = profile.omitOptional;
    succeed(command, args, env, { cwd: directory });
    expect(findPackage(directory, target.packageName).length === 0, `${target.packageName} was installed anyway`);
    const [runCommand, runPrefix] = profile.omitOptionalRun || [
      path.join(directory, 'node_modules', '.bin', windows ? (manager === 'bun' ? 'vsift.exe' : 'vsift.cmd') : 'vsift'),
      [],
    ];
    const result = run(runCommand, [...runPrefix, '--version'], env, { cwd: directory });
    expect(result.status === 127, `${describeStatus(result)}: ${result.stderr}`);
    expect(result.stderr.includes(`${target.packageName}@${version}`) && result.stderr.includes('not installed'), result.stderr);
    for (const name of PACKAGES.slice(1)) {
      expect(result.stderr.includes(name), `the message does not name the target ${name}: ${result.stderr}`);
    }
    assertNoStackTrace(result.stderr);
    return `exit 127: ${result.stderr.split('\n')[0]}`;
  });

  // ------------------------------------------------------------ provenance
  await checkProvenance({ recorder, version, target, env, work, label });

  recorder.finish(title);
}

/**
 * Hostile file names and arguments through every shim a user's shell may run.
 * A shim that is not run directly by a typical shell (npm's `.cmd` on Windows)
 * is observed and its differences recorded; the others are gated.
 */
async function checkHostile({ recorder, shims, invoke, direct, base, label }) {
  const directory = path.join(base, `hostile ${label}`);
  const files = path.join(directory, 'files');
  const cwd = path.join(directory, 'cwd');
  const sessions = path.join(directory, 'sessions');
  fs.mkdirSync(files, { recursive: true });
  fs.mkdirSync(cwd, { recursive: true });
  fs.mkdirSync(sessions, { recursive: true });
  const cases = [];
  const refused = [];
  for (const [what, name] of hostileFileNames(process.platform)) {
    try {
      fs.writeFileSync(path.join(files, name), DRAFT);
      cases.push({ what, name });
    } catch (error) {
      refused.push(`${what} (${error.code})`);
    }
  }
  recorder.observe('hostile file names created', `${cases.length} names${refused.length > 0 ? `; this file system refused ${refused.join(', ')}` : ''}`);
  const markers = () => [path.join(cwd, MARKER), path.join(files, MARKER), path.join(directory, MARKER), path.join(base, MARKER)].filter((file) => fs.existsSync(file));

  // What VSift itself answers for every case, once: the reference. `--file`
  // takes an absolute path only (`handoff check` refuses a relative one), so
  // each name is given in the two forms the parser accepts: a separate value
  // and one attached with `=`. A leading dash is covered below, as a value after
  // `--`.
  const nameInvocations = [];
  for (const { what, name } of cases) {
    const file = path.join(files, name);
    nameInvocations.push({ what: `${what}, --file <absolute path>`, args: ['--file', file], cwd });
    nameInvocations.push({ what: `${what}, --file=<absolute path>`, args: [`--file=${file}`], cwd: files });
  }
  const argumentInvocations = hostileArguments(process.platform).map(([what, argument]) => ({
    what: `argument: ${what}`,
    args: ['--session-root', sessions, '--json', 'ingest', '--', argument],
    cwd,
    raw: true,
  }));
  const references = new Map();
  for (const invocation of [...nameInvocations, ...argumentInvocations]) {
    const args = invocation.raw ? invocation.args : ['handoff', 'check', '--json', ...invocation.args];
    invocation.full = args;
    references.set(invocation, direct(args, { cwd: invocation.cwd }));
  }
  await recorder.check('hostile file names: vsift reads every file when run directly (the reference)', () => {
    const bad = nameInvocations.filter((invocation) => {
      const result = references.get(invocation);
      let status;
      try {
        status = JSON.parse(result.stdout).status;
      } catch {
        status = 'no JSON';
      }
      return !(result.status === 0 && status === 'complete');
    });
    expect(bad.length === 0, `vsift itself could not read: ${bad.map((invocation) => invocation.what).join('; ')}`);
    return `${nameInvocations.length} invocations over ${cases.length} names`;
  });
  await recorder.check('hostile arguments: vsift answers the same typed failure to each when run directly (the reference)', () => {
    const odd = argumentInvocations.filter((invocation) => {
      const result = references.get(invocation);
      try {
        const document = JSON.parse(result.stdout);
        return !(document.command === 'ingest' && document.status === 'failed' && result.status !== 0);
      } catch {
        return true;
      }
    });
    expect(odd.length === 0, `no typed failure for: ${odd.map((invocation) => invocation.what).join('; ')}`);
    return `${argumentInvocations.length} arguments`;
  });

  for (const shim of shims) {
    const observed = shim.kind === 'cmd';
    const different = [];
    let injected = false;
    for (const invocation of [...nameInvocations, ...argumentInvocations]) {
      for (const marker of markers()) {
        fs.rmSync(marker, { force: true });
      }
      const reference = references.get(invocation);
      const result = invoke(shim, invocation.full, { cwd: invocation.cwd });
      if (result.status !== reference.status || result.stdout !== reference.stdout) {
        different.push(invocation.what);
      }
      if (markers().length > 0) {
        injected = true;
        different.push(`${invocation.what} (CREATED ${MARKER})`);
      }
    }
    const total = nameInvocations.length + argumentInvocations.length;
    const summary = `${total - different.length} of ${total} answered exactly as vsift itself does when started directly (a file name had to open the file; an argument had to run no command)`;
    if (observed) {
      recorder.observe(
        `hostile names and arguments through ${shim.label} (cmd.exe re-parses the command line; npm's shim, known limit L-109)`,
        different.length === 0 ? summary : `${summary}; different: ${different.slice(0, 14).join('; ')}${different.length > 14 ? '; ...' : ''}${injected ? '; a command an argument spelled RAN' : ''}`,
      );
      continue;
    }
    await recorder.check(`hostile file names and arguments through ${shim.label}`, () => {
      expect(different.length === 0 && !injected, `${summary}; different: ${different.slice(0, 12).join('; ')}`);
      return summary;
    });
  }
}

/** `npm audit signatures` and `gh attestation verify` on this install's counterparts, in the same job. */
async function checkProvenance({ recorder, version, target, env, work, label }) {
  const project = path.join(work, `audit ${label}`);
  writeFile(path.join(project, 'package.json'), `${JSON.stringify({ name: 'vsift-qualification', version: '0.0.0', private: true }, null, 2)}\n`);
  const readEnv = { ...npmReadEnvironment(env, work), npm_config_cache: path.join(work, 'audit-cache') };
  // A project install, not the global prefix of the npm profile.
  delete readEnv.npm_config_prefix;
  await recorder.check('npm audit signatures: the installed packages carry verified registry signatures and attestations', () => {
    succeed('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund', `vsift-cli@${version}`], readEnv, { cwd: project });
    const installed = fs.readdirSync(path.join(project, 'node_modules', '@vsift')).length + 1;
    const audit = succeed('npm', ['audit', 'signatures'], { ...readEnv, npm_config_audit: 'true' }, { cwd: project });
    const text = `${audit.stdout}${audit.stderr}`;
    const signatures = /(\d+) packages? ha(?:s|ve) verified registry signatures/.exec(text);
    const attestations = /(\d+) packages? ha(?:s|ve) verified attestations/.exec(text);
    expect(signatures && Number(signatures[1]) === installed && attestations && Number(attestations[1]) === installed, `expected ${installed} and ${installed}: ${text.trim()}`);
    return `${signatures[1]} packages with verified registry signatures, ${attestations[1]} with verified attestations`;
  });
  await recorder.check("gh attestation verify: this install's launcher and platform tarballs came from the Release workflow", () => {
    const directory = path.join(work, 'tarballs');
    fs.mkdirSync(directory, { recursive: true });
    const verified = [];
    for (const name of ['vsift-cli', target.packageName]) {
      succeed('npm', ['pack', `${name}@${version}`, '--pack-destination', directory, '--ignore-scripts'], readEnv, { cwd: project });
      const file = path.join(directory, tarballName(name, version));
      expect(fs.existsSync(file), `npm pack did not write ${file}`);
      verifyAttestation(file, version, githubEnvironment(env));
      verified.push(path.basename(file));
    }
    return `verified ${verified.join(', ')}`;
  });
}

main().catch((error) => {
  process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
  process.exitCode = 1;
});
