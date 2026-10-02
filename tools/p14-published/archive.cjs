'use strict';

// RQ-02: the extracted native archive on each target (P14 PR 2; ADR 0024;
// docs/planning/p14-qualification.md section 2; docs/operations/install.md
// section 3 walked step by step).
//
//   node tools/p14-published/archive.cjs --version <published version> --commit <the tag's 40 hex digits> \
//     --work <scratch folder> --tag-skill <the tag's skills/vsift>
//
// It downloads the three archives and SHA256SUMS from the GitHub release,
// checks the checksums and the Sigstore attestation of each archive, extracts
// them, reads the executable formats of all three, and runs the one for this
// machine: `vsift --version` (the version and the first 12 digits of the tag's
// commit) and `vsift setup check --json`, with **no Node.js on `PATH`** (and
// no Rust toolchain, Git or Python), and requires the shipped skill to be
// byte-identical to the tag's.
//
// What a pass shows: install.md section 3 works end to end from a download, with
// no JavaScript runtime. It does not show the prompts a browser download
// triggers (the download here has no quarantine mark: SmartScreen and
// Gatekeeper are the maintainer's try-outs, RQ-17).

const fs = require('node:fs');
const path = require('node:path');

const {
  QualificationError,
  Recorder,
  cleanEnvironment,
  compareTrees,
  currentTarget,
  describeStatus,
  expect,
  githubEnvironment,
  parseArguments,
  parseJson,
  run,
  succeed,
  windows,
} = require('./lib/common.cjs');
const { FORMAT_OF_TARGET, REQUIRED_FILES, archiveRoot, executableOf, formatOfFile } = require('./lib/archive.cjs');
const { ARCHIVE_TARGETS, archiveName, checkChecksums, downloadAssets, parseVersion, verifyAttestation } = require('./lib/release.cjs');
const { describeResolution, namesOf, scrubbedEnvironment } = require('./lib/scrub.cjs');

async function main() {
  const options = parseArguments(process.argv.slice(2), { required: ['version', 'commit', 'work', 'tag-skill'] });
  const { version, commit } = options;
  parseVersion(version);
  expect(/^[0-9a-f]{40}$/.test(commit), '--commit must be 40 hex digits');
  const target = currentTarget();
  const work = path.resolve(options.work);
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const recorder = new Recorder();
  const title = `RQ-02 extracted native archive of vsift ${version} on ${target.label}`;
  const downloads = path.join(work, 'download');
  const extracted = path.join(work, 'extracted');
  const ghEnv = githubEnvironment(process.env);

  const downloaded = await recorder.check('download the three archives and SHA256SUMS from the GitHub release', () => {
    downloadAssets(version, downloads, [...ARCHIVE_TARGETS.map((name) => archiveName(version, name)), 'SHA256SUMS'], ghEnv);
    const names = fs.readdirSync(downloads).sort();
    expect(names.length === 4, `downloaded ${names.join(', ')}`);
    return names.join(', ');
  });
  if (!downloaded) {
    recorder.finish(title);
    return;
  }
  await recorder.check('SHA256SUMS lists the three archives and each matches (install.md section 3, step 2)', () => `${checkChecksums(downloads, version)} archives`);
  await recorder.check('gh attestation verify of each archive (install.md section 3, step 3)', () => {
    for (const name of ARCHIVE_TARGETS) {
      verifyAttestation(path.join(downloads, archiveName(version, name)), version, ghEnv);
    }
    return `${ARCHIVE_TARGETS.length} of ${ARCHIVE_TARGETS.length} verified against the Release workflow and refs/tags/v${version}`;
  });

  // The environment every extraction and every run uses: no Node.js and no
  // toolchain on PATH, a fresh per-user state.
  const hidden = namesOf(['node', 'rust', 'git', 'python']);
  let scrubbed;
  await recorder.check('environment: node, npm, npx, bun, pnpm, yarn, cargo, rustc, git and python do not resolve', () => {
    const state = path.join(work, 'user');
    const folders = {
      HOME: state,
      USERPROFILE: state,
      APPDATA: path.join(state, 'roaming'),
      LOCALAPPDATA: path.join(state, 'local'),
      XDG_CONFIG_HOME: path.join(state, 'config'),
      XDG_CACHE_HOME: path.join(state, 'cache'),
      XDG_DATA_HOME: path.join(state, 'data'),
      TMPDIR: path.join(state, 'tmp'),
      TEMP: path.join(state, 'tmp'),
      TMP: path.join(state, 'tmp'),
    };
    for (const folder of Object.values(folders)) {
      fs.mkdirSync(folder, { recursive: true });
    }
    scrubbed = scrubbedEnvironment({ ...cleanEnvironment(process.env), ...folders }, hidden, { farm: path.join(work, 'path-farm') });
    const lines = describeResolution(hidden, scrubbed.env[scrubbed.key]);
    expect(lines.every((line) => line.endsWith('not found')), lines.join('; '));
    return 'none of them resolves';
  });
  if (scrubbed === undefined) {
    recorder.finish(title);
    return;
  }
  const env = scrubbed.env;

  // Windows 11 has `tar` (bsdtar) in System32, which is the one install.md
  // names. The first `tar` on a runner's PATH is Git for Windows' GNU tar, which
  // reads `D:\...` as a host name; a user with Git's tools first on PATH meets
  // the same (install.md section 3 says how to avoid it).
  const tar = windows ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
  await recorder.check('extract the three archives with tar (install.md section 3, step 4)', () => {
    for (const name of ARCHIVE_TARGETS) {
      const destination = path.join(extracted, name);
      fs.mkdirSync(destination, { recursive: true });
      succeed(tar, ['-xzf', path.join(downloads, archiveName(version, name)), '-C', destination], env, { cwd: work });
    }
    return `extracted with ${windows ? 'System32\\tar.exe' : 'tar'}`;
  });

  await recorder.check('every archive holds its executable in the right format, the licences, the notices, the SBOM and the skill', () => {
    const seen = [];
    for (const name of ARCHIVE_TARGETS) {
      const root = path.join(extracted, name, archiveRoot(version, name));
      const executable = path.join(root, executableOf(name));
      expect(fs.existsSync(executable), `${executable} is missing`);
      const format = formatOfFile(executable);
      expect(format === FORMAT_OF_TARGET[name], `${name}: the executable is ${format}, expected ${FORMAT_OF_TARGET[name]}`);
      for (const file of REQUIRED_FILES) {
        expect(fs.statSync(path.join(root, file)).isFile(), `${name}: ${file} is missing`);
      }
      expect(fs.statSync(path.join(root, 'skills', 'vsift', 'SKILL.md')).isFile(), `${name}: the skill is missing`);
      seen.push(`${name} ${format}`);
    }
    return seen.join(', ');
  });

  const root = path.join(extracted, target.archiveTarget, archiveRoot(version, target.archiveTarget));
  const executable = path.join(root, executableOf(target.archiveTarget));
  await recorder.check('--version names the version and the first 12 digits of the tag commit', () => {
    const result = run(executable, ['--version'], env, { cwd: work });
    const expected = `vsift ${version} (${commit.slice(0, 12)})`;
    expect(result.status === 0 && result.stdout.trim() === expected, `${describeStatus(result)} printed ${JSON.stringify(result.stdout.trim())}; expected ${JSON.stringify(expected)}: ${result.stderr}`);
    return result.stdout.trim();
  });
  await recorder.check('setup check --json answers with the setup.check document', () => {
    // A runner may lack the media tools: the report may be `blocked` (exit 2); it is still the contract's document.
    const result = run(executable, ['setup', 'check', '--json'], env, { cwd: work });
    const document = parseJson(result);
    expect(document.command === 'setup.check' && document.schema_version === '1', `${JSON.stringify(document).slice(0, 300)}`);
    expect([0, 2].includes(result.status), describeStatus(result));
    return `${describeStatus(result)}, status ${document.status}`;
  });
  await recorder.check("the shipped skill is the tag's skills/vsift, byte for byte", () => {
    const comparison = compareTrees(path.join(root, 'skills', 'vsift'), path.resolve(options['tag-skill']));
    expect(comparison.equal, comparison.problems.slice(0, 5).join('; '));
    return `${comparison.files} files identical`;
  });

  recorder.finish(title);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
  process.exitCode = 1;
});
