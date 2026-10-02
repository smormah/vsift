'use strict';

// SEC-02 against the installed package (threat model: "PATH/current-directory
// executable hijack"; verification rows P-02 and D-01; ADR 0024, RQ-01: "a fake
// `ffmpeg` planted on `PATH` and in the working directory that the installed
// VSift must not use as a trusted tool").
//
// What VSift promises, read from the code and the contract (cli-v1.md, "Lookup
// of every tool"), and what each case therefore asserts:
//
// A. A planted tool in the working directory, and relative or empty `PATH`
//    entries, are never searched: `setup check --json` never selects the
//    planted file and the file is never run.
// B. A planted tool in an *absolute* `PATH` directory is looked up, because a
//    user's own tools on `PATH` are found automatically (install.md section
//    5.2): `setup check` runs it, as a probe, and reports it. That is the
//    documented trust in the user's `PATH`, so it is recorded, not gated. What
//    is gated is that such a tool is never trusted for media work: the media
//    tool check (a built-in test video with known answers) refuses it, and
//    `ingest` ends with the typed `MISSING_CAPABILITY` failure.
// C. A tool the user registered (`setup configure`) wins over `PATH`: the
//    registered file is selected, and the planted one on `PATH` is never run.
//
// A fake is a script (Linux, macOS) or a small program compiled with the .NET
// Framework's compiler that every runner image carries (Windows). Each one
// records its own run in `ran.txt` beside it and prints a banner naming
// itself, so the check can say which file answered.

const childProcess = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const { QualificationError, expect, parseJson, windows, describeStatus } = require('./common.cjs');
const { pathKey, joinPath, splitPath } = require('./scrub.cjs');

const MARKER_NAME = 'ran.txt';

const FAKE_SOURCE = [
  'using System;',
  'using System.IO;',
  'class Fake {',
  '  static int Main(string[] args) {',
  `    File.AppendAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "${MARKER_NAME}"), String.Join(" ", args) + "\\n");`,
  '    Console.WriteLine("ffmpeg version BANNER Copyright (c) fake");',
  '    return 0;',
  '  }',
  '}',
  '',
].join('\n');

/** The .NET Framework compiler of a Windows runner. */
function windowsCompiler() {
  const root = process.env.windir || process.env.SystemRoot || 'C:\\Windows';
  const compiler = path.join(root, 'Microsoft.NET', 'Framework64', 'v4.0.30319', 'csc.exe');
  expect(fs.existsSync(compiler), `the .NET Framework compiler ${compiler} is not on this runner`);
  return compiler;
}

/**
 * Plants an `ffmpeg` and an `ffprobe` in `directory`; each prints a banner
 * naming `banner` and records its run in `ran.txt` beside it.
 */
function plantFakeTools(directory, banner) {
  fs.mkdirSync(directory, { recursive: true });
  if (windows) {
    const source = path.join(directory, 'fake.cs');
    fs.writeFileSync(source, FAKE_SOURCE.replace('BANNER', banner));
    const result = childProcess.spawnSync(windowsCompiler(), ['/nologo', `/out:${path.join(directory, 'ffmpeg.exe')}`, source], {
      encoding: 'utf8',
      timeout: 120_000,
    });
    expect(result.status === 0, `csc exited ${result.status}: ${result.stdout}${result.stderr}`);
    fs.copyFileSync(path.join(directory, 'ffmpeg.exe'), path.join(directory, 'ffprobe.exe'));
    fs.rmSync(source);
    return;
  }
  const marker = path.join(directory, MARKER_NAME);
  const script = `#!/bin/sh\nprintf '%s\\n' "$*" >> '${marker.replace(/'/g, "'\\''")}'\necho 'ffmpeg version ${banner} Copyright (c) fake'\n`;
  for (const name of ['ffmpeg', 'ffprobe']) {
    const file = path.join(directory, name);
    fs.writeFileSync(file, script);
    fs.chmodSync(file, 0o755);
  }
}

/** Whether any fake in `directory` ran. */
function hasRun(directory) {
  return fs.existsSync(path.join(directory, MARKER_NAME));
}

/** What `setup check --json` reports for each tool: `dependency -> { status, lookup, detail }`. */
function readDependencies(result) {
  const document = parseJson(result);
  expect(document.command === 'setup.check', `command ${document.command}`);
  const found = new Map();
  for (const dependency of document.dependencies || []) {
    found.set(dependency.dependency, { status: dependency.status, lookup: dependency.lookup, detail: dependency.detail || '' });
  }
  return { document, found };
}

/** The executable name of a fake: `ffmpeg.exe` on Windows. */
function executableName(name) {
  return windows ? `${name}.exe` : name;
}

/**
 * Runs the three SEC-02 cases through the installed command.
 *
 * @param {object} context
 * @param {import('./common.cjs').Recorder} context.recorder
 * @param {(args: string[], options: {cwd: string, env: object}) => object} context.invoke runs the installed `vsift`
 * @param {object} context.env the scrubbed environment of the job
 * @param {string} context.base the folder every working directory lives in (Yarn finds its project by walking up from it)
 * @param {string} context.video a small video to ingest
 * @param {string} context.transcript its transcript
 */
async function checkSec02({ recorder, invoke, env, base, video, transcript }) {
  const root = path.join(base, 'sec02');
  const userEnvironment = (name) => {
    // A fresh per-user state for each case: configuration and sessions of one
    // case never reach another.
    const state = path.join(root, `user ${name}`);
    const folders = {
      HOME: state,
      USERPROFILE: state,
      APPDATA: path.join(state, 'roaming'),
      LOCALAPPDATA: path.join(state, 'local'),
      XDG_CONFIG_HOME: path.join(state, 'config'),
      XDG_CACHE_HOME: path.join(state, 'cache'),
      XDG_DATA_HOME: path.join(state, 'data'),
    };
    for (const folder of Object.values(folders)) {
      fs.mkdirSync(folder, { recursive: true });
    }
    return { ...env, ...folders };
  };
  const withPath = (base, entries) => {
    const key = pathKey(base);
    const result = { ...base };
    result[key] = joinPath([...entries, ...splitPath(base[key] || '')]);
    return result;
  };

  await recorder.check('SEC-02 A: a tool planted in the working directory, and relative or empty PATH entries, are never selected or run', () => {
    const cwd = path.join(root, 'cwd');
    plantFakeTools(cwd, 'PLANTED-IN-CWD');
    // The directory itself on PATH (absolute, relative and empty), which VSift
    // refuses: ambient lookup skips the current directory and relative entries.
    const env_ = withPath(userEnvironment('a'), [cwd, '.', '', 'bin', 'planted-relative']);
    const result = invoke(['setup', 'check', '--json'], { cwd, env: env_ });
    const { found } = readDependencies(result);
    for (const [name, entry] of found) {
      expect(!entry.detail.includes('PLANTED'), `${name} selected the planted file: ${JSON.stringify(entry)}`);
    }
    expect(!hasRun(cwd), `a planted tool ran (${path.join(cwd, MARKER_NAME)} exists)`);
    return `${[...found].map(([name, entry]) => `${name} ${entry.status}`).join(', ')}; the planted files never ran`;
  });

  const planted = path.join(root, 'planted-bin');
  plantFakeTools(planted, 'PLANTED-ON-PATH');
  const pathEnvironment = withPath(userEnvironment('b'), [planted]);
  const workingDirectory = path.join(root, 'run-b');
  fs.mkdirSync(workingDirectory, { recursive: true });

  try {
    const result = invoke(['setup', 'check', '--json'], { cwd: workingDirectory, env: pathEnvironment });
    const { found } = readDependencies(result);
    const ffmpeg = found.get('ffmpeg');
    recorder.observe(
      'SEC-02 B (documented trust in PATH): a tool in an absolute PATH directory is looked up and probed',
      `ffmpeg ${ffmpeg && ffmpeg.status} via ${ffmpeg && ffmpeg.lookup}, the planted file ${hasRun(planted) ? 'was run once as a probe' : 'was not run'} (install.md section 5.2: tools on PATH are found automatically)`,
    );
  } catch (error) {
    recorder.observe('SEC-02 B (documented trust in PATH)', `could not be read: ${error.message}`);
  }
  await recorder.check('SEC-02 B: a planted pair on an absolute PATH is never trusted for media work (ingest refuses it with a typed failure)', () => {
    const sessions = path.join(root, 'sessions-b');
    fs.mkdirSync(sessions, { recursive: true });
    const result = invoke(['--session-root', sessions, '--json', 'ingest', video, '--transcript', transcript], {
      cwd: workingDirectory,
      env: pathEnvironment,
    });
    const document = parseJson(result);
    expect(
      result.status === 2 && document.status === 'failed' && document.error && document.error.code === 'MISSING_CAPABILITY',
      `${describeStatus(result)}, ${JSON.stringify(document.error || document.status)}`,
    );
    const summary = ((document.error.remediation || [])[0] || {}).summary || '';
    expect(/media-tool check/i.test(summary), `the remediation does not name the media-tool check: ${summary.slice(0, 200)}`);
    return 'exit 2, MISSING_CAPABILITY: the fake pair failed the media-tool check';
  });

  await recorder.check('SEC-02 C: a registered tool wins over a planted one on PATH, which never runs', () => {
    const trusted = path.join(root, 'trusted-bin');
    plantFakeTools(trusted, 'TRUSTED-REGISTERED');
    const decoy = path.join(root, 'planted-bin-c');
    plantFakeTools(decoy, 'PLANTED-DECOY');
    const state = userEnvironment('c');
    const env_ = withPath(state, [decoy]);
    for (const name of ['ffmpeg', 'ffprobe']) {
      const result = invoke(['setup', 'configure', name, '--executable', path.join(trusted, executableName(name)), '--json'], {
        cwd: workingDirectory,
        env: env_,
      });
      expect(result.status === 0, `setup configure ${name}: ${describeStatus(result)} ${result.stdout.slice(0, 300)}`);
    }
    const result = invoke(['setup', 'check', '--json'], { cwd: workingDirectory, env: env_ });
    const { found } = readDependencies(result);
    const ffmpeg = found.get('ffmpeg');
    expect(ffmpeg && ffmpeg.lookup === 'configured_user_path', `ffmpeg lookup is ${ffmpeg && ffmpeg.lookup}`);
    expect(ffmpeg.detail.includes('TRUSTED-REGISTERED') && !ffmpeg.detail.includes('PLANTED'), `ffmpeg answered ${JSON.stringify(ffmpeg.detail)}`);
    expect(!hasRun(decoy), 'the planted tool on PATH ran although a tool was registered');
    return 'ffmpeg selected via configured_user_path; the planted tool on PATH never ran';
  });
}

module.exports = { MARKER_NAME, checkSec02, hasRun, plantFakeTools, readDependencies };
