#!/usr/bin/env node
'use strict';

// Usage: node install-published.cjs --prefix <dir> [--version <v>] [--github-output <file>]
//
// Installs the published `vsift-cli` from the real registry with scripts
// disabled and finds the native executable of the platform package (Linux
// x64), so a campaign runs the published bytes with no Node.js between. With
// no version it takes the one on the `next` tag. Prints the result as JSON
// and, given a GitHub output file, writes `binary`, `version` and `line`.

const fs = require('node:fs');
const host = require('./lib/host.cjs');

function main(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) options[argv[index].replace(/^--/, '')] = argv[index + 1];
  if (!options.prefix) {
    process.stderr.write('usage: install-published.cjs --prefix <dir> [--version <v>] [--github-output <file>]\n');
    return 2;
  }
  const published = host.installPublished({ version: options.version || '', prefix: options.prefix });
  process.stdout.write(`${JSON.stringify(published)}\n`);
  if (options['github-output']) {
    fs.appendFileSync(options['github-output'], `binary=${published.binary}\nversion=${published.version}\nline=${published.versionLine}\n`);
  }
  return 0;
}

try {
  process.exitCode = main(process.argv.slice(2));
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 2;
}
