// Runs the checked examples of one guide page against a real `vsift` and compares what they
// print with what the page shows (P14 PR 9b, docs/planning/user-guide-spec.md rule 3).
//
// Every command runs with an argument list and no shell, in a fresh sandbox: its own folder as
// the working directory, and its own user state (configuration, sessions) so a page never sees
// another page's sessions. The files a command names are copied in from the corpus
// (`fixtures/corpus/generated/`) or the guide's own files (`docs/guide/files/`).

'use strict';

const childProcess = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const { extractBlocks } = require('./blocks.cjs');
const { compareJson, compareText } = require('./normalise.cjs');
const { splitCommand, usesShellSyntax } = require('./shellwords.cjs');

// Where a command's input files come from, in order.
const FILE_SOURCES = [
  path.join('fixtures', 'corpus', 'generated'),
  path.join('fixtures', 'corpus', 'transcripts'),
  path.join('docs', 'guide', 'files'),
];
// What a placeholder such as <session> stands for: the first identifier of that kind in the most
// recent output that held one (`<candidate:3>` is the third).
// <frame>, <crop> and <clip> are the evidence id the latest `frame get`, `crop` or `audio` printed.
const PLACEHOLDER_PREFIX = {
  session: 'ses',
  revision: 'trv',
  segment: 'tsg',
  candidate: 'vcd',
  evidence: 'evd',
  job: 'job',
  operation: 'op',
  frame: 'frame',
  crop: 'crop',
  clip: 'clip',
};
// Which command makes which of those.
const LABELLED_COMMANDS = [
  { words: ['frame', 'get'], label: 'frame' },
  { words: ['crop'], label: 'crop' },
  { words: ['audio'], label: 'clip' },
];
const PLACEHOLDER = /<([a-z]+)(?::(\d+))?>/g;
const COMMAND_TIMEOUT_MS = 600_000;

/** The user state of one page: folders under `root`, and the environment that points at them. */
function makeSandbox(root, shared) {
  const dirs = {
    home: path.join(root, 'home'),
    cwd: path.join(root, 'work'),
    local: path.join(root, 'local'),
    cache: path.join(root, 'cache'),
    config: shared ? path.join(shared, 'config') : path.join(root, 'config'),
    data: shared ? path.join(shared, 'data') : path.join(root, 'data'),
  };
  for (const directory of Object.values(dirs)) fs.mkdirSync(directory, { recursive: true });
  const env = {
    ...process.env,
    HOME: dirs.home,
    USERPROFILE: dirs.home,
    LOCALAPPDATA: dirs.local,
    APPDATA: path.join(dirs.local, 'roaming'),
    XDG_CONFIG_HOME: dirs.config,
    XDG_CACHE_HOME: dirs.cache,
    XDG_DATA_HOME: dirs.data,
    NO_COLOR: '1',
  };
  return { dirs, env };
}

/** Runs the binary with `args`; stdout and stderr are returned together as a terminal would show them. */
function runProgram(binary, args, sandbox) {
  const result = childProcess.spawnSync(binary, args, {
    cwd: sandbox.dirs.cwd,
    env: sandbox.env,
    encoding: 'utf8',
    windowsHide: true,
    timeout: COMMAND_TIMEOUT_MS,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) return { status: null, stdout: '', stderr: String(result.error.message), output: String(result.error.message) };
  return { status: result.status, stdout: result.stdout, stderr: result.stderr, output: result.stdout + result.stderr };
}

/** Copies into the working directory each named file the corpus or the guide has. */
function stageFiles(args, repository, sandbox) {
  for (const argument of args) {
    const name = path.basename(argument.replace(/^\.[\\/]/, ''));
    if (name === '' || name.startsWith('-')) continue;
    const target = path.join(sandbox.dirs.cwd, name);
    if (fs.existsSync(target)) continue;
    for (const source of FILE_SOURCES) {
      const candidate = path.join(repository, source, name);
      if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
        fs.copyFileSync(candidate, target);
        break;
      }
    }
  }
}

/** Remembers the identifiers an output held, by kind, for the placeholders of later commands. */
function rememberIdentifiers(output, known) {
  const found = new Map();
  for (const match of output.matchAll(/\b(ses|trv|tsg|vcd|evd|job|op)_[0-9a-z]{16,}/g)) {
    const list = found.get(match[1]) || [];
    if (!list.includes(match[0])) list.push(match[0]);
    found.set(match[1], list);
  }
  for (const [prefix, list] of found) known.set(prefix, list);
}

/** Remembers the evidence id a labelled command printed (the first `evd_` of its output). */
function rememberLabel(args, output, known) {
  for (const { words, label } of LABELLED_COMMANDS) {
    if (words.every((word, index) => args[index] === word)) {
      const found = output.match(/\bevd_[0-9a-z]{16,}/g) || [];
      // A crop's output names its parent frame first and its own id after.
      const id = label === 'crop' ? found[found.length - 1] : found[0];
      if (id) known.set(label, [id]);
    }
  }
}

/** The command line with its placeholders replaced, or an error naming the one that has no value yet. */
function substitute(line, known) {
  let missing;
  const text = line.replace(PLACEHOLDER, (whole, name, index) => {
    const prefix = PLACEHOLDER_PREFIX[name];
    if (prefix === undefined) return whole;
    const value = (known.get(prefix) || [])[Number(index || 1) - 1];
    if (value === undefined) missing = whole;
    return value === undefined ? whole : value;
  });
  if (missing) throw new Error(`the placeholder ${missing} has no value yet: no earlier output of this page held one`);
  return text;
}

/**
 * Checks the blocks of one page. `context`: `{ repository, binary, sandbox, speechReady, onActual }`.
 * Returns `{ ran, skipped, failures }`; each failure says where and why.
 */
function checkBlocks(page, markdown, context) {
  const blocks = extractBlocks(markdown);
  const result = { ran: 0, skipped: 0, failures: [] };
  const known = new Map();
  let broken = false;
  for (const block of blocks) {
    if (block.options.needs.has('speech') && !context.speechReady) {
      result.skipped += block.commands.length;
      continue;
    }
    for (const command of block.commands) {
      const where = `${page}:${command.line}`;
      if (broken) {
        result.skipped += 1;
        continue;
      }
      let args;
      try {
        const line = substitute(command.text, known);
        if (usesShellSyntax(line)) throw new Error('the command uses shell syntax (a pipe, a redirection or a variable); a checked command is one program and its arguments');
        const words = splitCommand(line);
        if (words[0] !== 'vsift') throw new Error('a checked command starts with vsift');
        args = words.slice(1);
      } catch (error) {
        result.failures.push(`${where}: ${error.message}`);
        broken = true;
        continue;
      }
      stageFiles(args, context.repository, context.sandbox);
      // `context.run` lets a test stand in for the program.
      const run = context.run ? context.run(args, context.sandbox) : runProgram(context.binary, args, context.sandbox);
      result.ran += 1;
      if (context.onActual) context.onActual(page, command, run);
      if (run.status !== block.options.exit) {
        result.failures.push(`${where}: \`vsift ${args.join(' ')}\` exited ${run.status}, the page expects ${block.options.exit}\n${indent(run.output)}`);
        broken = true;
        continue;
      }
      rememberIdentifiers(run.output, known);
      rememberLabel(args, run.output, known);
      if (command.expected.length === 0) continue;
      const asJson = args.includes('--json') && command.expected[0].trimStart().startsWith('{');
      const compared = asJson
        ? compareJson(command.expected, run.stdout)
        : compareText(command.expected, run.output, block.options.needs.has('speech'));
      if (!compared.ok) {
        result.failures.push(`${where}: \`vsift ${args.join(' ')}\` printed something other than the page shows: ${compared.why}\n  the real output was:\n${indent(run.output)}`);
        broken = true;
      }
    }
  }
  return result;
}

function indent(text) {
  return text
    .split('\n')
    .map((line) => `    ${line}`)
    .join('\n');
}

module.exports = { checkBlocks, makeSandbox, rememberIdentifiers, runProgram, stageFiles, substitute };
