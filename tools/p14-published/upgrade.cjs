'use strict';

// RQ-04: upgrade from a published version, with a session and a configuration
// kept, and the uninstall walked (P14 PR 2; ADR 0024;
// docs/planning/p14-qualification.md section 2; docs/operations/install.md
// sections 7 and 8).
//
//   node tools/p14-published/upgrade.cjs --mode <registry|local> \
//     --from-version <published> --from-commit <its tag's 40 hex digits> \
//     --to-version <version to install> --to-version-line "vsift 0.1.0 (<12 hex>)" \
//     --work <scratch folder> --repo <checkout> \
//     [--ffmpeg-bin <folder with ffmpeg and ffprobe>] [--packages <four tarballs> --verdaccio <bin/verdaccio>]
//
// Two modes, and what each proves is not the same thing:
//
// - `registry`: both versions come from the real registry. While only one
//   version is published, --to-version is --from-version, and the "upgrade" is
//   the guide's command run again over an install that holds a session and a
//   configuration: it proves the procedure and that nothing it does disturbs
//   what the user kept, not that a newer version reads an older one's data.
// - `local`: the from-version is installed from the real registry and the
//   upgrade installs a higher version the workflow built from the pull request
//   and published to a loopback-only registry. The binary is the pull request's
//   own build (its `--version` names the commit); the package version is a
//   throwaway. It proves a newer build reads a published build's sessions,
//   bundles and configuration, and the package manager's upgrade, not the
//   release packaging.
//
// In both: install the from-version, give VSift its media tools (Ubuntu: the
// managed install of that version; Windows and macOS: tools the job staged,
// registered with `setup configure`), register a model, ingest two sessions,
// export one as a bundle, record what the read commands return, upgrade,
// require the same answers (additive members allowed: v1 is additive), the
// configuration unchanged, the bundle valid and a new session possible, and
// walk install.md section 8: managed tools, the package, then VSift's own
// folders, requiring that nothing is left but what the guide names.

const fs = require('node:fs');
const path = require('node:path');

const {
  QualificationError,
  Recorder,
  cleanEnvironment,
  describeStatus,
  expect,
  findPackage,
  currentTarget,
  parseArguments,
  parseJson,
  run,
  sha256File,
  succeed,
  windows,
  writeFile,
} = require('./lib/common.cjs');
const { AWKWARD, managerEnvironment, managerProfile } = require('./lib/managers.cjs');
const { LOCAL_REGISTRY, publishLocal, requireLoopback, startRegistry, stopRegistry } = require('./lib/local-registry.cjs');
const { npmView, parseVersion } = require('./lib/release.cjs');
const { describeResolution, namesOf, scrubbedEnvironment } = require('./lib/scrub.cjs');
const { findVsiftEntries, subsetProblems, vsiftFolders } = require('./lib/upgrade.cjs');

const MODES = ['registry', 'local'];

async function main() {
  const options = parseArguments(process.argv.slice(2), {
    required: ['mode', 'from-version', 'from-commit', 'to-version', 'to-version-line', 'work', 'repo'],
    optional: ['ffmpeg-bin', 'packages', 'verdaccio'],
  });
  expect(MODES.includes(options.mode), `--mode must be one of ${MODES.join(', ')}`);
  const local = options.mode === 'local';
  const fromVersion = options['from-version'];
  const toVersion = options['to-version'];
  parseVersion(fromVersion);
  parseVersion(toVersion);
  expect(/^[0-9a-f]{40}$/.test(options['from-commit']), '--from-commit must be 40 hex digits');
  if (local) {
    expect(options.packages && options.verdaccio, 'the local mode needs --packages and --verdaccio');
  }
  const target = currentTarget();
  const managed = process.platform === 'linux';
  if (!managed) {
    expect(options['ffmpeg-bin'], '--ffmpeg-bin is required where VSift does not install its own tools');
  }
  const work = path.resolve(options.work, AWKWARD);
  fs.rmSync(path.resolve(options.work), { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const repo = path.resolve(options.repo);
  const video = path.join(repo, 'fixtures', 'corpus', 'generated', 'F10.mp4');
  const srt = path.join(repo, 'fixtures', 'corpus', 'transcripts', 'F10.srt');
  const vtt = path.join(repo, 'fixtures', 'corpus', 'transcripts', 'F10.vtt');
  const fromLine = `vsift ${fromVersion} (${options['from-commit'].slice(0, 12)})`;
  const toLine = options['to-version-line'];

  const recorder = new Recorder();
  const title = `RQ-04 upgrade vsift-cli ${fromVersion} to ${toVersion} on ${target.label} (${local ? 'local registry mode: a higher locally built version' : 'real registry mode'})`;
  recorder.observe(
    'what this run proves',
    local
      ? `the published ${fromVersion} is upgraded by the pull request's own build, served by a loopback-only registry as ${toVersion}; the package version is a throwaway`
      : fromVersion === toVersion
        ? `the real registry holds only ${fromVersion}: the upgrade command is run again over an install that holds a session and a configuration; no newer version is read`
        : `${toVersion} from the real registry replaces ${fromVersion}`,
  );

  // ------------------------------------------------------------ environment
  const hidden = namesOf(['rust', 'git', 'python']);
  const userRoot = path.join(path.resolve(options.work), 'ascii', 'user');
  const userEnvironment = {
    HOME: userRoot,
    USERPROFILE: userRoot,
    APPDATA: path.join(userRoot, 'AppData', 'Roaming'),
    LOCALAPPDATA: path.join(userRoot, 'AppData', 'Local'),
    XDG_CONFIG_HOME: path.join(userRoot, '.config'),
    XDG_CACHE_HOME: path.join(userRoot, '.cache'),
    XDG_DATA_HOME: path.join(userRoot, '.local', 'share'),
    TMPDIR: path.join(userRoot, 'tmp'),
    TEMP: path.join(userRoot, 'tmp'),
    TMP: path.join(userRoot, 'tmp'),
  };
  for (const folder of Object.values(userEnvironment)) {
    fs.mkdirSync(folder, { recursive: true });
  }
  let env;
  const scrubbedOk = await recorder.check('scrubbed environment: cargo, rustc, rustup, git and python do not resolve', () => {
    const result = scrubbedEnvironment({ ...cleanEnvironment(process.env), ...userEnvironment }, hidden, { farm: path.join(path.resolve(options.work), 'ascii', 'path-farm') });
    env = result.env;
    const lines = describeResolution(hidden, env[result.key]);
    expect(lines.every((line) => line.endsWith('not found')), lines.join('; '));
    return 'none of them resolves';
  });
  if (!scrubbedOk) {
    recorder.finish(title);
    return;
  }
  const baseEnv = managerEnvironment(env, work, AWKWARD);
  const profile = managerProfile('npm', fromVersion, path.join(work, `npm install ${AWKWARD}`), baseEnv, work);
  fs.mkdirSync(profile.root, { recursive: true });
  const folders = vsiftFolders(process.platform, userEnvironment);

  let registry;
  try {
    // ---------------------------------------------------------- the real registry's from-version
    await recorder.check(`vsift-cli@${fromVersion} is on the real registry`, () => {
      const published = npmView(`vsift-cli@${fromVersion}`, 'version', baseEnv, work);
      expect(published === fromVersion, `the registry reports ${JSON.stringify(published)}`);
      return `${fromVersion} published`;
    });
    const installed = await recorder.check(`install vsift-cli@${fromVersion} from the real registry (npm, global, scripts disabled)`, () => {
      const [command, args] = profile.install;
      succeed(command, args, profile.env, { cwd: work });
      return `${command} ${args.join(' ')}`;
    });
    if (!installed) {
      recorder.finish(title);
      return;
    }
    const shim = profile.shims[0];
    const vsift = (args, extra = {}) => run(shim.file, args, extra.env || profile.env, { cwd: extra.cwd || work, input: extra.input, timeout: extra.timeout || 600_000 });
    const json = (args, extra) => parseJson(vsift([...args, '--json'], extra));
    // `session list` pages by hash bucket (the cursor runs 0 to 255): follow it to the end.
    const listAll = () => {
      const items = [];
      let cursor;
      for (let page = 0; page < 300; page += 1) {
        const document = json(['session', 'list', ...(cursor === undefined ? [] : ['--cursor', String(cursor)])]);
        expect(document.status === 'complete' || document.status === 'partial', `session list: ${JSON.stringify(document.error || document.status)}`);
        items.push(...document.data.items);
        cursor = document.data.next_cursor;
        if (cursor === null || cursor === undefined) {
          return items.sort((a, b) => (a.session_id < b.session_id ? -1 : 1));
        }
      }
      throw new QualificationError('session list did not end after 300 pages');
    };

    await recorder.check(`--version is ${fromLine}`, () => {
      const result = vsift(['--version']);
      expect(result.status === 0 && result.stdout.trim() === fromLine, `${describeStatus(result)} printed ${JSON.stringify(result.stdout.trim())}`);
      return result.stdout.trim();
    });

    // ---------------------------------------------------------- tools and configuration
    if (managed) {
      await recorder.check('managed tools: the installed version plans and installs the reviewed tools (Ubuntu 24.04)', () => {
        const plan = vsift(['setup', 'plan', '--profile', 'desktop', '--json']);
        const document = parseJson(plan);
        expect(document.data.managed_install === 'catalogue_accepted', `managed_install ${document.data.managed_install}`);
        const planFile = path.join(work, 'plan.json');
        fs.writeFileSync(planFile, plan.stdout);
        const result = vsift(['setup', 'install', '--plan', planFile, '--accept-plan', document.data.plan_digest, '--json'], { timeout: 1_800_000 });
        const installedDocument = parseJson(result);
        expect(result.status === 0 && installedDocument.status === 'complete', `${describeStatus(result)} ${JSON.stringify(installedDocument.error)}`);
        return installedDocument.data.components.map((component) => `${component.component} ${component.status}`).join(', ');
      });
    } else {
      await recorder.check('tools: ffmpeg and ffprobe registered with setup configure', () => {
        for (const name of ['ffmpeg', 'ffprobe']) {
          const executable = path.resolve(options['ffmpeg-bin'], windows ? `${name}.exe` : name);
          expect(fs.existsSync(executable), `${executable} does not exist`);
          const result = vsift(['setup', 'configure', name, '--executable', executable, '--json']);
          expect(result.status === 0, `setup configure ${name}: ${describeStatus(result)} ${result.stdout.slice(0, 300)}`);
        }
        return `registered ${path.resolve(options['ffmpeg-bin'])}`;
      });
    }
    const model = path.join(work, 'registered model.bin');
    writeFile(model, 'a stand-in for a model file: registration does not parse it\n');
    await recorder.check('configuration: a model file registered with setup configure-model', () => {
      const result = vsift(['setup', 'configure-model', '--file', model, '--json']);
      expect(result.status === 0, `${describeStatus(result)} ${result.stdout.slice(0, 300)}`);
      return 'registered';
    });
    const configSnapshot = () => {
      const files = new Map();
      const walk = (directory) => {
        if (!fs.existsSync(directory)) {
          return;
        }
        for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
          const full = path.join(directory, entry.name);
          if (entry.isDirectory()) {
            walk(full);
          } else if (entry.isFile() && !/\.lock$/i.test(entry.name)) {
            files.set(path.relative(folders.config, full), sha256File(full));
          }
        }
      };
      walk(folders.config);
      return files;
    };
    const configBefore = configSnapshot();
    let checkBefore;
    await recorder.check('the configuration exists, and setup check reports the tools it will use', () => {
      expect(configBefore.size > 0, `${folders.config} holds no configuration file`);
      const result = vsift(['setup', 'check', '--json']);
      checkBefore = parseJson(result);
      expect(checkBefore.command === 'setup.check', `command ${checkBefore.command}`);
      const dependencies = checkBefore.dependencies.map((dependency) => `${dependency.dependency} ${dependency.status} via ${dependency.lookup}`);
      expect(checkBefore.dependencies.slice(0, 2).every((dependency) => dependency.status === 'available'), dependencies.join(', '));
      return `${[...configBefore.keys()].join(', ')}; ${dependencies.join(', ')}`;
    });

    // ---------------------------------------------------------- sessions made by the from-version
    const sessions = {};
    for (const [label, transcript] of [['A', srt], ['B', vtt]]) {
      await recorder.check(`ingest session ${label}: F10 with a supplied transcript (${path.basename(transcript)})`, () => {
        const result = vsift(['ingest', video, '--transcript', transcript, '--json']);
        const document = parseJson(result);
        expect(result.status === 0 && document.status === 'complete' && /^ses_/.test(document.data.session_id), `${describeStatus(result)} ${JSON.stringify(document.error || document.status)}`);
        sessions[label] = document.data.session_id;
        return document.data.session_id;
      });
    }
    const bundle = path.join(work, 'bundles', 'session B');
    fs.mkdirSync(path.dirname(bundle), { recursive: true });
    const reads = (id) => ({
      status: ['session', 'status', id],
      transcript: ['transcript', 'get', '--from', '0', '--to', '12000000', id],
      search: ['search', '--query', 'R-17', id],
    });
    const before = {};
    let retained = false;
    if (sessions.A && sessions.B) {
      await recorder.check('read session A: status, the transcript and a search for R-17', () => {
        for (const [name, args] of Object.entries(reads(sessions.A))) {
          const document = json(args);
          expect(document.status === 'complete', `${name}: ${JSON.stringify(document.error || document.status)}`);
          before[name] = document;
        }
        expect(before.transcript.data.items.length === 3, `${before.transcript.data.items.length} transcript segments`);
        expect(before.search.data.items.length === 1 && before.search.data.hits.length === 1, 'the search for R-17 did not find its one segment');
        return `3 segments, 1 hit, status ${before.status.data.state}`;
      });
      retained = await recorder.check('retain session B as a bundle and validate it', () => {
        const exported = vsift(['session', 'retain', sessions.B, '--output', bundle, '--json']);
        const retainedDocument = parseJson(exported);
        expect(exported.status === 0 && retainedDocument.status === 'complete', `${describeStatus(exported)} ${JSON.stringify(retainedDocument.error)}`);
        const validated = json(['bundle', 'validate', bundle]);
        expect(validated.status === 'complete', JSON.stringify(validated.error || validated.status));
        before.bundle = validated;
        return `bundle of ${sessions.B} validates`;
      });
    }

    // ---------------------------------------------------------- the upgrade
    before.list = listAll();
    recorder.observe('sessions listed before the upgrade', before.list.map((item) => `${item.session_id} ${item.state}`).join(', ') || 'none');
    const configBeforeUpgrade = configSnapshot();
    const upgradeEnv = local ? { ...profile.env, npm_config_registry: LOCAL_REGISTRY, NPM_CONFIG_REGISTRY: LOCAL_REGISTRY } : profile.env;
    if (local) {
      requireLoopback(upgradeEnv.npm_config_registry);
      registry = await startRegistry(path.resolve(options.verdaccio), work);
      await recorder.check('publish the four locally built packages to the loopback registry, and nothing anywhere else', async () => {
        const tarballs = fs.readdirSync(path.resolve(options.packages)).filter((name) => name.endsWith('.tgz')).map((name) => path.join(path.resolve(options.packages), name));
        const published = await publishLocal(tarballs, 'next', baseEnv, work);
        const real = npmView(`vsift-cli@${toVersion}`, 'version', baseEnv, work);
        expect(real === null, `the real registry also holds ${toVersion}`);
        return `${published.join(', ')} (and the real registry has no ${toVersion})`;
      });
    }
    const upgraded = await recorder.check(`upgrade: npm install --global vsift-cli@${toVersion}${local ? ' from the loopback registry' : ''}`, () => {
      const args = ['install', '--global', '--ignore-scripts', ...(local ? ['--registry', LOCAL_REGISTRY] : []), `vsift-cli@${toVersion}`];
      succeed('npm', args, upgradeEnv, { cwd: work });
      const result = vsift(['--version']);
      expect(result.status === 0 && result.stdout.trim() === toLine, `${describeStatus(result)} printed ${JSON.stringify(result.stdout.trim())}; expected ${JSON.stringify(toLine)}: ${result.stderr}`);
      const launcher = findPackage(profile.root, 'vsift-cli');
      expect(launcher.length === 1, `${launcher.length} copies of the launcher after the upgrade`);
      const installedVersion = JSON.parse(fs.readFileSync(path.join(launcher[0], 'package.json'), 'utf8')).version;
      expect(installedVersion === toVersion, `the installed launcher is ${installedVersion}`);
      return `now ${result.stdout.trim()} (package ${installedVersion})`;
    });
    if (registry) {
      await stopRegistry(registry);
      registry = undefined;
    }
    if (!upgraded) {
      recorder.finish(title);
      return;
    }

    // ---------------------------------------------------------- what survived
    await recorder.check('the configuration is unchanged byte for byte, and setup check reports the same tools', () => {
      const after = configSnapshot();
      // The configuration proper is what `setup configure` wrote; the folder
      // also holds the media-tool check record, which commands rewrite.
      const changed = [...configBefore].filter(([file, digest]) => after.get(file) !== digest).map(([file]) => file);
      expect(changed.length === 0, `the configuration changed: ${changed.join(', ')}`);
      const others = [...configBeforeUpgrade].filter(([file]) => !configBefore.has(file));
      const record = others.map(([file, digest]) => `${file} ${after.get(file) === digest ? 'unchanged' : after.has(file) ? 'rewritten' : 'removed'}`);
      const result = parseJson(vsift(['setup', 'check', '--json']));
      const lookups = (document) => document.dependencies.map((dependency) => `${dependency.dependency}:${dependency.status}:${dependency.lookup}`);
      expect(JSON.stringify(lookups(result)) === JSON.stringify(lookups(checkBefore)), `before ${lookups(checkBefore)}, after ${lookups(result)}`);
      return `${[...configBefore.keys()].join(', ')} identical${record.length > 0 ? `; also in the folder: ${record.join(', ')}` : ''}; ${lookups(result).join(', ')}`;
    });
    if (managed) {
      await recorder.check('the managed tools are still installed and verify', () => {
        const listed = json(['setup', 'list']);
        const selections = listed.data.components.map((component) => `${component.component} ${component.selection}`);
        expect(listed.data.components.every((component) => component.selection === 'verified'), selections.join(', '));
        return selections.join(', ');
      });
    }
    if (sessions.A && sessions.B) {
      await recorder.check('session A reads the same after the upgrade (additive members allowed)', () => {
        const notes = [];
        for (const [name, args] of Object.entries(reads(sessions.A))) {
          const document = json(args);
          const problems = subsetProblems(before[name], document);
          expect(problems.length === 0, `${name}: ${problems.slice(0, 5).join('; ')}`);
          notes.push(name);
        }
        return `${notes.join(', ')} unchanged`;
      });
      await recorder.check('the session list is as before (session A in it), and the retained bundle still validates', () => {
        const listed = listAll();
        const identifiers = listed.map((item) => item.session_id);
        expect(identifiers.includes(sessions.A), `session A is not listed: ${identifiers.join(', ')}`);
        const listProblems = subsetProblems(before.list, listed.filter((item) => before.list.some((earlier) => earlier.session_id === item.session_id)));
        expect(listProblems.length === 0, `the session list changed: ${listProblems.slice(0, 5).join('; ')}`);
        const validated = json(['bundle', 'validate', bundle]);
        const problems = subsetProblems(before.bundle, validated);
        expect(validated.status === 'complete' && problems.length === 0, `${JSON.stringify(validated.error || problems)}`);
        return `${identifiers.length} session(s) listed as before; the bundle validates`;
      });
    }
    await recorder.check('the upgraded version makes a new session too', () => {
      const result = vsift(['ingest', video, '--transcript', srt, '--json']);
      const document = parseJson(result);
      expect(result.status === 0 && document.status === 'complete', `${describeStatus(result)} ${JSON.stringify(document.error || document.status)}`);
      return document.data.session_id;
    });

    // ---------------------------------------------------------- install.md section 8
    if (managed) {
      await recorder.check('uninstall 1: vsift setup remove for each managed component, then --stale-stages', () => {
        for (const component of ['ffmpeg_ffprobe', 'whisper_cli', 'whisper_model']) {
          const result = vsift(['setup', 'remove', component, '--json']);
          expect(result.status === 0, `setup remove ${component}: ${describeStatus(result)} ${result.stdout.slice(0, 400)}`);
        }
        const stale = vsift(['setup', 'remove', '--stale-stages', '--json']);
        expect(stale.status === 0, `setup remove --stale-stages: ${describeStatus(stale)} ${stale.stdout.slice(0, 400)}`);
        const listed = json(['setup', 'list']);
        expect(listed.data.components.every((component) => component.selection === 'none'), JSON.stringify(listed.data.components.map((component) => component.selection)));
        return 'all three components removed; nothing selected';
      });
    }
    await recorder.check('uninstall 2: npm uninstall --global vsift-cli removes the launcher, the platform package and the command', () => {
      const [command, args] = profile.uninstall;
      succeed(command, args, profile.env, { cwd: work });
      expect(findPackage(profile.root, 'vsift-cli').length === 0, 'the launcher is left behind');
      expect(findPackage(profile.root, target.packageName).length === 0, 'the platform package is left behind');
      const left = findVsiftEntries(profile.root, { ignoreEmpty: true });
      expect(left.length === 0, `left in the install folder: ${left.join(', ')}`);
      for (const file of profile.shims) {
        expect(!fs.existsSync(file.file), `${file.file} is still there`);
      }
      return 'nothing named vsift is left in the install folder';
    });
    let present;
    await recorder.check("uninstall 3: VSift's own folders are exactly the ones section 8 names, and nothing else of VSift's is in the user's state", () => {
      const named = Object.entries(folders).filter(([, folder]) => folder !== null);
      present = named.filter(([, folder]) => fs.existsSync(folder));
      expect(
        present.some(([name]) => name === 'config') && present.some(([name]) => name === 'sessions'),
        `the guide's configuration and sessions folders do not both exist: ${present.map(([name]) => name).join(', ')}`,
      );
      if (managed) {
        // The managed folder lives in a `vsift` folder that holds nothing else.
        expect(fs.existsSync(folders.managed), 'the managed folder is gone before the guide says to delete it');
      }
      const allowed = new Set([folders.config, folders.sessions, ...(managed ? [path.dirname(folders.managed)] : [])]);
      const unexpected = findVsiftEntries(userRoot).filter((entry) => !allowed.has(entry));
      expect(unexpected.length === 0, `VSift left something the guide does not name: ${unexpected.join(', ')}`);
      if (managed) {
        const inside = fs.readdirSync(path.dirname(folders.managed));
        expect(JSON.stringify(inside) === JSON.stringify(['managed-v1']), `the vsift data folder holds ${inside.join(', ')}`);
      }
      return `present: ${present.map(([name, folder]) => `${name} (${path.relative(userRoot, folder)})`).join(', ')}`;
    });
    await recorder.check('uninstall 4: deleting those folders removes every trace; the bundle, the registered tools and the source files stay', () => {
      for (const [name, folder] of present || []) {
        // The managed folder sits in a `vsift` folder of its own, which goes with it.
        fs.rmSync(name === 'managed' ? path.dirname(folder) : folder, { recursive: true, force: true });
      }
      const left = findVsiftEntries(userRoot);
      expect(left.length === 0, `still there: ${left.join(', ')}`);
      expect(fs.existsSync(bundle), 'the retained bundle was removed (VSift never deletes it)');
      expect(fs.existsSync(video) && fs.existsSync(srt), 'a source file is gone');
      if (!managed) {
        expect(fs.existsSync(path.resolve(options['ffmpeg-bin'], windows ? 'ffmpeg.exe' : 'ffmpeg')), 'the registered tool is gone (VSift never deletes it)');
      }
      expect(fs.existsSync(model), 'the registered model file is gone (VSift never deletes it)');
      return 'no trace of VSift in the user state; the exported bundle, the registered tools and the originals remain';
    });
  } finally {
    if (registry) {
      await stopRegistry(registry);
    }
  }
  recorder.finish(title);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
  process.exitCode = 1;
});
