'use strict';

// RQ-03: the offline `--artifact-dir` install with the real reviewed artifacts
// under no network (P14 PR 2; ADR 0024; docs/planning/p14-qualification.md
// section 2; docs/contracts/cli-v1.md "P13 `setup install`"; D-07).
//
//   node tools/p14-published/offline-install.cjs --version <published version> --commit <the tag's 40 hex digits> \
//     --work <scratch folder> --image <pinned Ubuntu 24.04 image, name@sha256:...>
//
// On a hosted Ubuntu 24.04 runner:
//
// 1. the published Linux archive is downloaded, checked and extracted (the
//    published binary is what runs, not a Cargo build);
// 2. a download step fetches the catalogue's three real reviewed artifacts into
//    a folder, each by the address, size and SHA-256 that the published
//    binary's own `setup plan` names, verified as the bytes arrive;
// 3. a container started with `--network none` runs the published
//    `setup plan`, `setup install --plan <file> --accept-plan <digest>
//    --artifact-dir <absolute folder>` and `setup check --json`, and `setup
//    list`; the same install, online, is shown to fail inside the container, so
//    "no network" is a fact and not a hope;
// 4. a tampered artifact (one byte changed), a missing artifact and a relative
//    folder are refused with their typed failures.
//
// A pass shows the offline path with real bytes (until now only stand-ins have
// run). It does not show that the publishers keep their files (L-099), or any
// system but a minimal Ubuntu 24.04 image. Requests carry only a neutral
// identifier; the published binary makes no request at all inside the container.

const fs = require('node:fs');
const path = require('node:path');

const {
  QualificationError,
  Recorder,
  cleanEnvironment,
  currentTarget,
  describeStatus,
  expect,
  githubEnvironment,
  parseArguments,
  parseJson,
  run,
  succeed,
} = require('./lib/common.cjs');
const { archiveRoot, executableOf } = require('./lib/archive.cjs');
const { artifactName, downloadVerified } = require('./lib/download.cjs');
const { archiveName, checkArchiveChecksum, downloadAssets, parseVersion, verifyAttestation } = require('./lib/release.cjs');
const { describeResolution, namesOf, scrubbedEnvironment } = require('./lib/scrub.cjs');

const TARGET = 'x86_64-unknown-linux-gnu';
const EXPECTED_COMPONENTS = ['ffmpeg_ffprobe', 'whisper_cli', 'whisper_model'];

async function main() {
  const options = parseArguments(process.argv.slice(2), { required: ['version', 'commit', 'work', 'image'] });
  const { version, commit } = options;
  parseVersion(version);
  expect(/^[0-9a-f]{40}$/.test(commit), '--commit must be 40 hex digits');
  expect(/^[a-z0-9./-]+@sha256:[0-9a-f]{64}$/.test(options.image), '--image must be an image pinned by digest');
  expect(process.platform === 'linux' && process.arch === 'x64', 'this check runs on Linux x64 only');
  const work = path.resolve(options.work);
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const recorder = new Recorder();
  const title = `RQ-03 offline artifact-directory install of the real reviewed artifacts with vsift ${version} (container, no network)`;

  // ------------------------------------------------------------ host environment
  const hidden = namesOf(['rust', 'git', 'python']);
  let env;
  const scrubbed = await recorder.check('host environment: cargo, rustc, rustup, git and python do not resolve', () => {
    const state = path.join(work, 'host-user');
    const folders = { HOME: state, XDG_CONFIG_HOME: path.join(state, 'config'), XDG_CACHE_HOME: path.join(state, 'cache'), XDG_DATA_HOME: path.join(state, 'data') };
    for (const folder of Object.values(folders)) {
      fs.mkdirSync(folder, { recursive: true });
    }
    const result = scrubbedEnvironment({ ...cleanEnvironment(process.env), ...folders }, hidden, { farm: path.join(work, 'path-farm') });
    env = result.env;
    const lines = describeResolution(hidden, env[result.key]);
    expect(lines.every((line) => line.endsWith('not found')), lines.join('; '));
    return 'none of them resolves';
  });
  if (!scrubbed) {
    recorder.finish(title);
    return;
  }
  const target = currentTarget();
  expect(target.archiveTarget === TARGET, 'unexpected target');

  // ------------------------------------------------------------ the published binary
  const downloads = path.join(work, 'download');
  const extracted = path.join(work, 'extracted');
  let binaryDirectory;
  await recorder.check('the published Linux archive: downloaded, checksummed, attested, extracted, and it names the tag commit', () => {
    const name = archiveName(version, TARGET);
    downloadAssets(version, downloads, [name, 'SHA256SUMS'], githubEnvironment(env));
    checkArchiveChecksum(downloads, name);
    verifyAttestation(path.join(downloads, name), version, githubEnvironment(env));
    fs.mkdirSync(extracted, { recursive: true });
    succeed('tar', ['-xzf', path.join(downloads, name), '-C', extracted], env, { cwd: work });
    binaryDirectory = path.join(extracted, archiveRoot(version, TARGET));
    const result = run(path.join(binaryDirectory, executableOf(TARGET)), ['--version'], env, { cwd: work });
    const expected = `vsift ${version} (${commit.slice(0, 12)})`;
    expect(result.status === 0 && result.stdout.trim() === expected, `${describeStatus(result)} printed ${JSON.stringify(result.stdout.trim())}; expected ${JSON.stringify(expected)}`);
    return result.stdout.trim();
  });
  if (binaryDirectory === undefined) {
    recorder.finish(title);
    return;
  }
  const hostVsift = (args) => run(path.join(binaryDirectory, 'vsift'), args, env, { cwd: work });

  // ------------------------------------------------------------ the plan and the real artifacts
  let actions = [];
  await recorder.check("the published binary's own plan names three reviewed artifacts", () => {
    const result = hostVsift(['setup', 'plan', '--profile', 'desktop', '--json']);
    const plan = parseJson(result);
    expect(result.status === 0 && plan.command === 'setup.plan', `${describeStatus(result)}: ${result.stdout.slice(0, 300)}`);
    expect(plan.data.target === 'ubuntu_24_04_x86_64', `the runner's target is ${plan.data.target}`);
    expect(plan.data.managed_install === 'catalogue_accepted', `managed_install is ${plan.data.managed_install}`);
    actions = plan.data.actions;
    expect(JSON.stringify(actions.map((action) => action.component)) === JSON.stringify(EXPECTED_COMPONENTS), `components ${actions.map((action) => action.component).join(', ')}`);
    return actions.map((action) => `${action.component} ${action.version} (${action.bytes} bytes)`).join('; ');
  });
  const artifacts = path.join(work, 'artifacts');
  await recorder.check('download the three real artifacts by the plan, each verified by size and SHA-256', async () => {
    expect(actions.length === 3, 'no plan');
    const notes = [];
    for (const action of actions) {
      const name = artifactName(action.source_url);
      const { finalUrl } = await downloadVerified({
        url: action.source_url,
        bytes: action.bytes,
        sha256: action.sha256,
        destination: path.join(artifacts, name),
      });
      notes.push(`${name} ${action.bytes} bytes via ${new URL(finalUrl).hostname}`);
    }
    expect(fs.readdirSync(artifacts).length === 3, 'the folder holds more than the three artifacts');
    return notes.join('; ');
  });

  // ------------------------------------------------------------ the container
  let image = options.image;
  const mounts = (state, artifactFolder) => [
    '--volume', `${binaryDirectory}:/opt/vsift:ro`,
    '--volume', `${state}:/state`,
    ...(artifactFolder ? ['--volume', `${artifactFolder}:/artifacts:ro`] : []),
  ];
  const inContainer = (state, args, { artifactFolder } = {}) =>
    run(
      'docker',
      [
        'run', '--rm', '--network', 'none', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--pids-limit', '512',
        '--user', `${process.getuid()}:${process.getgid()}`, '--workdir', '/state',
        '--env', 'HOME=/state/home', '--env', 'PATH=/usr/local/bin:/usr/bin:/bin',
        ...mounts(state, artifactFolder), image, '/opt/vsift/vsift', ...args,
      ],
      env,
      { cwd: work, timeout: 900_000 },
    );
  const freshState = (name) => {
    const state = path.join(work, name);
    fs.mkdirSync(path.join(state, 'home'), { recursive: true });
    return state;
  };

  await recorder.check('the pinned Ubuntu image runs the published binary (with libssl.so.3, as install.md section 1 says it needs)', () => {
    const release = succeed('docker', ['run', '--rm', '--network', 'none', image, 'cat', '/etc/os-release'], env, { cwd: work, timeout: 600_000 }).stdout;
    expect(/VERSION_ID="24\.04"/.test(release), `the image is not Ubuntu 24.04: ${release.slice(0, 200)}`);
    const first = inContainer(freshState('probe'), ['--version']);
    if (first.status === 0) {
      return `${first.stdout.trim()} on Ubuntu 24.04 as pinned`;
    }
    expect(/libssl/.test(first.stderr), `${describeStatus(first)}: ${first.stderr.slice(0, 300)}`);
    // The minimal image lacks the one library the guide names. A user's
    // Ubuntu 24.04 has it; the image is given exactly that package and nothing
    // else, so any other missing library still shows as a failure below.
    const derived = 'vsift-offline-qualification:ubuntu-24.04-libssl3';
    succeed('docker', ['build', '--tag', derived, '-'], env, {
      cwd: work,
      timeout: 600_000,
      input: `FROM ${options.image}\nRUN apt-get update && apt-get install --yes --no-install-recommends libssl3t64 && rm -rf /var/lib/apt/lists/*\n`,
    });
    image = derived;
    const second = inContainer(freshState('probe-derived'), ['--version']);
    expect(second.status === 0, `${describeStatus(second)}: ${second.stderr.slice(0, 300)}`);
    return `${second.stdout.trim()}; the pinned image lacks libssl.so.3 (${first.stderr.trim().split('\n')[0]}), so it was given libssl3t64 and nothing else`;
  });

  // The reviewed whisper.cpp build is dynamically linked. A minimal image may
  // lack a library it needs (the install then fails its compatibility smoke:
  // MISSING_CAPABILITY, the banner check). Ask the dynamic loader what is
  // missing, give the image exactly those packages and say so: install.md
  // section 5.1 names only glibc and OpenSSL 3 for the CLI, so a missing
  // library is a finding about the guide, not something to hide.
  const LIBRARY_PACKAGES = { 'libgomp.so.1': 'libgomp1', 'libstdc++.so.6': 'libstdc++6', 'libgcc_s.so.1': 'libgcc-s1' };
  await recorder.check("the reviewed whisper.cpp build's shared libraries are in the image, or the image is given exactly the missing ones", () => {
    const whisperArchive = artifactName(actions.find((action) => action.component === 'whisper_cli').source_url);
    const diagnostics = path.join(work, 'diagnostics');
    fs.mkdirSync(diagnostics, { recursive: true });
    succeed('tar', ['-xzf', path.join(artifacts, whisperArchive), '-C', diagnostics], env, { cwd: work });
    const find = (directory) => {
      for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const full = path.join(directory, entry.name);
        if (entry.isDirectory()) {
          const inner = find(full);
          if (inner) {
            return inner;
          }
        } else if (entry.name === 'whisper-cli') {
          return full;
        }
      }
      return undefined;
    };
    const cli = find(diagnostics);
    expect(cli !== undefined, 'the whisper.cpp archive holds no whisper-cli');
    const listing = run(
      'docker',
      ['run', '--rm', '--network', 'none', '--volume', `${diagnostics}:/diagnostics:ro`, image, 'ldd', `/diagnostics/${path.relative(diagnostics, cli).split(path.sep).join('/')}`],
      env,
      { cwd: work, timeout: 300_000 },
    );
    const missing = [...`${listing.stdout}`.matchAll(/^\s*(\S+) => not found$/gm)].map((match) => match[1]);
    const inImage = `/diagnostics/${path.relative(diagnostics, cli).split(path.sep).join('/')}`;
    const help = run('docker', ['run', '--rm', '--network', 'none', '--volume', `${diagnostics}:/diagnostics:ro`, image, inImage, '--help'], env, {
      cwd: work,
      timeout: 120_000,
    });
    const firstLine = `${help.stderr}${help.stdout}`.trim().split('\n')[0] || '';
    recorder.observe('whisper-cli --help inside the image', `${describeStatus(help)}: ${firstLine.slice(0, 200)}`);
    if (missing.length === 0) {
      return `every library of whisper-cli resolves in ${image === options.image ? 'the pinned image' : 'the image'}`;
    }
    const packages = missing.map((library) => LIBRARY_PACKAGES[library]);
    expect(packages.every(Boolean), `whisper-cli needs ${missing.join(', ')}, which this tool cannot map to an Ubuntu package`);
    const base = image === options.image ? options.image : image;
    const derived = 'vsift-offline-qualification:ubuntu-24.04-with-libraries';
    succeed('docker', ['build', '--tag', derived, '-'], env, {
      cwd: work,
      timeout: 600_000,
      input: `FROM ${base}\nRUN apt-get update && apt-get install --yes --no-install-recommends ${[...new Set(packages)].join(' ')} && rm -rf /var/lib/apt/lists/*\n`,
    });
    image = derived;
    recorder.observe(
      'FINDING: the minimal Ubuntu 24.04 image cannot run the reviewed whisper.cpp build (install.md section 5.1 does not name this requirement)',
      `whisper-cli needs ${missing.join(', ')} (Ubuntu packages ${[...new Set(packages)].join(', ')}); without it the managed install ends in MISSING_CAPABILITY at the banner check. The image was given exactly those packages`,
    );
    return `missing ${missing.join(', ')}; installed ${[...new Set(packages)].join(', ')}`;
  });

  // The container's own plan: acceptance compares the plan with the machine as it is.
  const state = freshState('state-offline');
  let planDigest;
  await recorder.check("inside the container: the published `setup plan` agrees with the host's on the artifacts", () => {
    const result = inContainer(state, ['setup', 'plan', '--profile', 'desktop', '--json']);
    const plan = parseJson(result);
    expect(result.status === 0 && plan.data.managed_install === 'catalogue_accepted', `${describeStatus(result)}: ${result.stdout.slice(0, 300)}`);
    expect(/^[0-9a-f]{64}$/.test(plan.data.plan_digest), 'no plan digest');
    expect(
      JSON.stringify(plan.data.actions.map((action) => [action.component, action.source_url, action.bytes, action.sha256])) ===
        JSON.stringify(actions.map((action) => [action.component, action.source_url, action.bytes, action.sha256])),
      'the container plan names other artifacts than the host plan',
    );
    fs.writeFileSync(path.join(state, 'plan.json'), result.stdout);
    planDigest = plan.data.plan_digest;
    return `plan digest ${planDigest.slice(0, 12)}..., ${plan.data.actions.length} actions`;
  });
  if (planDigest === undefined) {
    recorder.finish(title);
    return;
  }
  const install = (stateDirectory, extra) =>
    inContainer(stateDirectory, ['setup', 'install', '--plan', '/state/plan.json', '--accept-plan', planDigest, '--json', ...extra], { artifactFolder: undefined });

  await recorder.check('control: the same install without --artifact-dir fails inside the container as `offline`, so the container has no network', () => {
    const control = freshState('state-control');
    fs.copyFileSync(path.join(state, 'plan.json'), path.join(control, 'plan.json'));
    const result = install(control, []);
    const document = parseJson(result);
    expect(result.status === 7 && document.error && document.error.code === 'DOWNLOAD_FAILED', `${describeStatus(result)} ${JSON.stringify(document.error)}`);
    const first = (document.data && document.data.components && document.data.components[0]) || {};
    expect(first.reason === 'offline', `the first component failed with ${JSON.stringify(first.reason)}, not offline`);
    return `exit 7, DOWNLOAD_FAILED, reason ${first.reason}`;
  });

  const withArtifacts = (stateDirectory, folder, args) =>
    inContainer(stateDirectory, ['setup', 'install', '--plan', '/state/plan.json', '--accept-plan', planDigest, '--json', ...args], { artifactFolder: folder });

  await recorder.check('a relative --artifact-dir is refused with INVALID_ARGUMENT before anything changes', () => {
    const scratch = freshState('state-relative');
    fs.copyFileSync(path.join(state, 'plan.json'), path.join(scratch, 'plan.json'));
    const result = withArtifacts(scratch, artifacts, ['--artifact-dir', 'artifacts']);
    const document = parseJson(result);
    expect(result.status === 2 && document.error && document.error.code === 'INVALID_ARGUMENT', `${describeStatus(result)} ${JSON.stringify(document.error)}`);
    return 'exit 2, INVALID_ARGUMENT';
  });

  await recorder.check('--artifact-dir /artifacts: every component is activated from the real files', () => {
    const result = withArtifacts(state, artifacts, ['--artifact-dir', '/artifacts']);
    const document = parseJson(result);
    expect(result.status === 0 && document.status === 'complete', `${describeStatus(result)} ${JSON.stringify(document.error || document.status)} components ${JSON.stringify(document.data && document.data.components)} ${result.stderr.slice(0, 300)}`);
    expect(document.data.source === 'artifact_directory', `source ${document.data.source}`);
    const components = document.data.components.map((component) => [component.component, component.status]);
    expect(JSON.stringify(components) === JSON.stringify(EXPECTED_COMPONENTS.map((name) => [name, 'activated'])), JSON.stringify(components));
    return components.map(([name, status]) => `${name} ${status}`).join(', ');
  });
  await recorder.check('setup check --json: ready, every tool from the managed store', () => {
    const result = inContainer(state, ['setup', 'check', '--json']);
    const document = parseJson(result);
    expect(result.status === 0 && document.status === 'ready', `${describeStatus(result)} status ${document.status}: ${JSON.stringify(document.dependencies).slice(0, 400)}`);
    const lookups = document.dependencies.map((dependency) => `${dependency.dependency} ${dependency.status} via ${dependency.lookup}`);
    for (const dependency of document.dependencies) {
      expect(dependency.status === 'available' && dependency.lookup === 'managed_version', lookups.join(', '));
    }
    return `${lookups.join(', ')}; local ASR ${JSON.stringify(document.local_asr && document.local_asr.verification && document.local_asr.verification.status)}`;
  });
  await recorder.check('setup list --json: every component verifies, and a rerun finds all three already current', () => {
    const listed = parseJson(inContainer(state, ['setup', 'list', '--json']));
    const selections = listed.data.components.map((component) => `${component.component} ${component.selection}`);
    expect(listed.data.components.every((component) => component.selection === 'verified'), selections.join(', '));
    const again = parseJson(withArtifacts(state, artifacts, ['--artifact-dir', '/artifacts']));
    const statuses = again.data.components.map((component) => component.status);
    expect(statuses.every((status) => status === 'already_current'), statuses.join(', '));
    return `${selections.join(', ')}; rerun: ${statuses.join(', ')}`;
  });

  // ------------------------------------------------------------ the refusals
  const linked = (name, change) => {
    const folder = path.join(work, name);
    fs.mkdirSync(folder, { recursive: true });
    for (const file of fs.readdirSync(artifacts)) {
      change(folder, file);
    }
    return folder;
  };
  const whisperName = artifactName(actions.find((action) => action.component === 'whisper_cli').source_url);
  const modelName = artifactName(actions.find((action) => action.component === 'whisper_model').source_url);

  await recorder.check('tamper: one changed byte in an artifact is refused as INTEGRITY_FAILURE (digest_mismatch) and never used', () => {
    const folder = linked('artifacts-tampered', (directory, file) => {
      if (file === whisperName) {
        const bytes = fs.readFileSync(path.join(artifacts, file));
        bytes[bytes.length - 1] ^= 0xff;
        fs.writeFileSync(path.join(directory, file), bytes);
      } else {
        fs.linkSync(path.join(artifacts, file), path.join(directory, file));
      }
    });
    const scratch = freshState('state-tamper');
    fs.copyFileSync(path.join(state, 'plan.json'), path.join(scratch, 'plan.json'));
    const result = withArtifacts(scratch, folder, ['--artifact-dir', '/artifacts']);
    const document = parseJson(result);
    expect(result.status === 7 && document.error && document.error.code === 'INTEGRITY_FAILURE', `${describeStatus(result)} ${JSON.stringify(document.error)}`);
    const byName = Object.fromEntries(document.data.components.map((component) => [component.component, component]));
    expect(byName.whisper_cli.status === 'failed' && byName.whisper_cli.reason === 'digest_mismatch', JSON.stringify(byName.whisper_cli));
    expect(byName.whisper_model.status === 'failed' && byName.whisper_model.reason === 'blocked', `the model: ${JSON.stringify(byName.whisper_model)}`);
    const check = parseJson(inContainer(scratch, ['setup', 'check', '--json']));
    const whisper = check.dependencies.find((dependency) => dependency.dependency === 'whisper');
    expect(check.status !== 'ready' && whisper.status !== 'available', `after the tamper whisper is ${whisper.status} and the report ${check.status}`);
    return `exit 7, INTEGRITY_FAILURE: ffmpeg ${byName.ffmpeg_ffprobe.status}, whisper_cli ${byName.whisper_cli.reason}, whisper_model ${byName.whisper_model.reason}; setup check ${check.status}, whisper ${whisper.status}`;
  });
  await recorder.check('missing: an artifact that is not in the folder is refused as INVALID_ARGUMENT (artifact_missing)', () => {
    const folder = linked('artifacts-missing', (directory, file) => {
      if (file !== modelName) {
        fs.linkSync(path.join(artifacts, file), path.join(directory, file));
      }
    });
    const scratch = freshState('state-missing');
    fs.copyFileSync(path.join(state, 'plan.json'), path.join(scratch, 'plan.json'));
    const result = withArtifacts(scratch, folder, ['--artifact-dir', '/artifacts']);
    const document = parseJson(result);
    expect(result.status === 2 && document.error && document.error.code === 'INVALID_ARGUMENT', `${describeStatus(result)} ${JSON.stringify(document.error)}`);
    const model = document.data.components.find((component) => component.component === 'whisper_model');
    expect(model.status === 'failed' && model.reason === 'artifact_missing', JSON.stringify(model));
    return `exit 2, INVALID_ARGUMENT, whisper_model ${model.reason}`;
  });

  recorder.finish(title);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
  process.exitCode = 1;
});
