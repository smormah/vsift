#!/usr/bin/env node
// Runs the user guide's checked examples against a real `vsift` and fails when what a page
// shows is not what the command prints (P14 PR 9b, docs/planning/user-guide-spec.md rule 3).
//
//   node tools/guide/check-examples.cjs --binary <vsift> [--install-managed] [--require-speech]
//
// A page marks an example with a comment line above its `console` block (see lib/blocks.cjs);
// only marked blocks run. Each page runs in its own sandbox, with the corpus clips and the
// guide's own files copied in by name. The comparison (lib/normalise.cjs) is exact except for
// identifiers, timestamps, digests and a few measurements that differ on every run.
//
// Tools: FFmpeg and FFprobe must be on PATH (or managed). `--install-managed` runs the
// binary's own `setup plan` and `setup install` into a shared state (Ubuntu 24.04 x64 only, a
// download from the publishers). `--whisper <executable> --model <file>` registers your own
// whisper.cpp and model instead. Examples marked `needs speech` run only when local speech
// recognition is ready; `--require-speech` makes a machine without it a failure (CI).
// Node.js 22, no dependency.

'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { markdownPages } = require('./lib/consistency.cjs');
const { checkBlocks, makeSandbox, runProgram } = require('./lib/runner.cjs');

const REPOSITORY = path.resolve(__dirname, '..', '..');

function parseArguments(argv) {
  const options = {
    binary: undefined,
    repository: REPOSITORY,
    work: undefined,
    pages: undefined,
    installManaged: false,
    whisper: undefined,
    model: undefined,
    requireSpeech: false,
    writeActual: undefined,
    keep: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const value = () => {
      index += 1;
      if (argv[index] === undefined) throw new Error(`${argument} needs a value`);
      return argv[index];
    };
    if (argument === '--binary') options.binary = path.resolve(value());
    else if (argument === '--repository') options.repository = path.resolve(value());
    else if (argument === '--work') options.work = path.resolve(value());
    else if (argument === '--pages') options.pages = value().split(',').filter(Boolean);
    else if (argument === '--install-managed') options.installManaged = true;
    else if (argument === '--whisper') options.whisper = path.resolve(value());
    else if (argument === '--model') options.model = path.resolve(value());
    else if (argument === '--require-speech') options.requireSpeech = true;
    else if (argument === '--write-actual') options.writeActual = path.resolve(value());
    else if (argument === '--keep') options.keep = true;
    else throw new Error(`unknown argument ${argument}`);
  }
  if (!options.binary) throw new Error('--binary <path to vsift> is required');
  if ((options.whisper === undefined) !== (options.model === undefined)) throw new Error('--whisper and --model go together');
  return options;
}

/** The pages with checked examples: the hand-written ones, never the generated reference. */
function pagesToCheck(options) {
  const all = markdownPages(options.repository, 'docs/guide').filter((page) => !page.startsWith('docs/guide/reference/'));
  return options.pages ? all.filter((page) => options.pages.some((wanted) => page.endsWith(wanted))) : all;
}

function log(message) {
  process.stdout.write(`${message}\n`);
}

/** Installs the reviewed tools into the shared state with the binary's own plan, as a user would. */
function installManaged(binary, sandbox) {
  const plan = runProgram(binary, ['setup', 'plan', '--profile', 'desktop', '--json'], sandbox);
  if (plan.status !== 0) throw new Error(`setup plan failed: ${plan.output}`);
  const digest = JSON.parse(plan.stdout).data.plan_digest;
  const planFile = path.join(sandbox.dirs.cwd, 'plan.json');
  fs.writeFileSync(planFile, plan.stdout);
  const install = runProgram(binary, ['setup', 'install', '--plan', planFile, '--accept-plan', digest], sandbox);
  if (install.status !== 0) throw new Error(`setup install failed: ${install.output}`);
  log('managed tools installed');
}

/** Registers the user's own whisper.cpp and model in a sandbox. */
function registerSpeech(binary, sandbox, whisper, model) {
  for (const args of [
    ['setup', 'configure', 'whisper', '--executable', whisper],
    ['setup', 'configure-model', '--file', model],
  ]) {
    const run = runProgram(binary, args, sandbox);
    if (run.status !== 0) throw new Error(`vsift ${args.join(' ')} failed: ${run.output}`);
  }
}

/** Whether `setup check` says local speech recognition works in this sandbox. */
function speechIsReady(binary, sandbox) {
  const run = runProgram(binary, ['setup', 'check', '--json'], sandbox);
  try {
    const check = JSON.parse(run.stdout);
    const verification = check.local_asr && check.local_asr.verification;
    return Boolean(verification && verification.status === 'verified');
  } catch {
    return false;
  }
}

function main(argv) {
  const options = parseArguments(argv);
  const base = fs.mkdtempSync(path.join(options.work || os.tmpdir(), 'vsift-guide-'));
  const shared = path.join(base, 'shared');
  const toolBox = makeSandbox(path.join(base, 'tools'), process.platform === 'win32' ? undefined : shared);
  const failures = [];
  let ran = 0;
  let skipped = 0;
  try {
    if (options.installManaged) installManaged(options.binary, toolBox);
    if (options.whisper) registerSpeech(options.binary, toolBox, options.whisper, options.model);
    const speechReady = speechIsReady(options.binary, toolBox);
    log(`local speech recognition: ${speechReady ? 'ready' : 'not ready'}`);
    if (!speechReady && options.requireSpeech) failures.push('local speech recognition is not ready, and --require-speech was given');
    const pages = pagesToCheck(options);
    if (pages.length === 0) failures.push('no guide page was found to check');
    for (const [number, page] of pages.entries()) {
      const markdown = fs.readFileSync(path.join(options.repository, page), 'utf8');
      // Windows keeps configuration and sessions in one folder, so each page registers its own tools;
      // elsewhere the configuration and the managed tools are shared and each page has its own sessions.
      // The folder is named by number: the media tools fail on paths longer than Windows allows.
      const sandbox = makeSandbox(path.join(base, `p${number + 1}`), process.platform === 'win32' ? undefined : shared);
      if (process.platform === 'win32' && options.whisper) registerSpeech(options.binary, sandbox, options.whisper, options.model);
      const onActual = options.writeActual
        ? (name, command, run) => {
            fs.mkdirSync(options.writeActual, { recursive: true });
            const file = path.join(options.writeActual, `${name.replace(/[\\/]/g, '_')}.actual.txt`);
            fs.appendFileSync(file, `$ ${command.text}\n${run.output}\n`);
          }
        : undefined;
      const result = checkBlocks(page, markdown, { repository: options.repository, binary: options.binary, sandbox, speechReady, onActual });
      ran += result.ran;
      skipped += result.skipped;
      failures.push(...result.failures);
      log(`${page}: ${result.ran} commands run, ${result.skipped} skipped, ${result.failures.length} failed`);
    }
  } finally {
    if (!options.keep) fs.rmSync(base, { recursive: true, force: true });
  }
  if (failures.length > 0) {
    process.stderr.write(`the guide's examples do not match the real output:\n${failures.map((failure) => `- ${failure}`).join('\n')}\n`);
    return 1;
  }
  log(`the guide's examples match the real output (${ran} commands run, ${skipped} skipped)`);
  return 0;
}

if (require.main === module) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 2;
  }
}

module.exports = { main, pagesToCheck, parseArguments };
