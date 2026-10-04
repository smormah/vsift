#!/usr/bin/env node
// Generates the user guide's reference pages (P14 PR 9b, docs/planning/user-guide-spec.md,
// rule 4) and checks that the committed pages are current:
//
//   node tools/guide/generate-reference.cjs --binary <vsift> --write
//   node tools/guide/generate-reference.cjs --binary <vsift> --check
//
// The pages are `docs/guide/reference/commands.md` (from `vsift --help` and the help of
// every command) and `docs/guide/reference/json.md` (from `schemas/v1/`). `--check` also
// holds the hand-written pages to what they promise (see lib/consistency.cjs): the version
// the guide says it was checked against, and the failure codes the troubleshooting page covers.
// Node.js 22, no dependency; the binary may be any build (help text does not depend on tools).

'use strict';

const fs = require('node:fs');
const path = require('node:path');
const help = require('./lib/help.cjs');
const jsonReference = require('./lib/json-reference.cjs');
const consistency = require('./lib/consistency.cjs');

const REPOSITORY = path.resolve(__dirname, '..', '..');
const GUIDE = path.join('docs', 'guide');
const COMMANDS_PAGE = path.join(GUIDE, 'reference', 'commands.md');
const JSON_PAGE = path.join(GUIDE, 'reference', 'json.md');

function parseArguments(argv) {
  const options = { binary: undefined, repository: REPOSITORY, mode: undefined };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--binary') options.binary = argv[(index += 1)];
    else if (argument === '--repository') options.repository = path.resolve(argv[(index += 1)]);
    else if (argument === '--write' || argument === '--check') options.mode = argument.slice(2);
    else throw new Error(`unknown argument ${argument}`);
  }
  if (!options.binary) throw new Error('--binary <path to vsift> is required');
  if (!options.mode) throw new Error('say --write or --check');
  options.binary = path.resolve(options.binary);
  return options;
}

/** The generated pages: `{ file (relative), text }`. */
function generate(options) {
  const env = { ...process.env, NO_COLOR: '1' };
  // The release (0.2.0), not the candidate (0.2.0-rc.1): see releaseOf.
  const version = help.releaseOf(help.readVersion(options.binary, env));
  const tree = help.readCommandTree(options.binary, env);
  const schemas = jsonReference.loadSchemas(path.join(options.repository, 'schemas', 'v1'));
  return {
    version,
    pages: [
      { file: COMMANDS_PAGE, text: help.renderCommands(tree, version) + '\n' },
      { file: JSON_PAGE, text: jsonReference.renderJson(schemas, version) + '\n' },
    ],
  };
}

function readIfThere(file) {
  try {
    return fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n');
  } catch {
    return undefined;
  }
}

/** The first line that differs, for a message that says where a page went stale. */
function firstDifference(expected, actual) {
  const left = expected.split('\n');
  const right = (actual || '').split('\n');
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    if (left[index] !== right[index]) {
      return `line ${index + 1}: generated ${JSON.stringify(left[index])}, committed ${JSON.stringify(right[index])}`;
    }
  }
  return 'no difference';
}

function main(argv) {
  const options = parseArguments(argv);
  const { version, pages } = generate(options);
  const problems = [];
  if (options.mode === 'write') {
    for (const page of pages) {
      const target = path.join(options.repository, page.file);
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, page.text);
      process.stdout.write(`wrote ${page.file}\n`);
    }
  } else {
    for (const page of pages) {
      const committed = readIfThere(path.join(options.repository, page.file));
      if (committed !== page.text) {
        problems.push(
          `${page.file} is out of date (${committed === undefined ? 'missing' : firstDifference(page.text, committed)}); regenerate it: node tools/guide/generate-reference.cjs --binary <vsift> --write`,
        );
      }
    }
  }
  problems.push(...consistency.check(options.repository, version));
  if (problems.length > 0) {
    process.stderr.write(`the guide is not consistent with the code:\n${problems.map((problem) => `- ${problem}`).join('\n')}\n`);
    return 1;
  }
  process.stdout.write(`the guide's reference pages and promises agree with vsift ${version}\n`);
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

module.exports = { generate, main, parseArguments };
