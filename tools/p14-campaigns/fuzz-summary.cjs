#!/usr/bin/env node
'use strict';

// Usage: node fuzz-summary.cjs --target <name> --log <file> --seconds <asked>
//
// Prints one coverage-plateau line for a libFuzzer log (P14 RQ-07). Exit
// status 0 when the run finished and found no failure, 1 otherwise, so a
// workflow step can fail on a log that ended without a final line.

const fs = require('node:fs');
const { summarise, plateauLine } = require('./lib/libfuzzer-log.cjs');

function main(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    options[argv[index]] = argv[index + 1];
  }
  const target = options['--target'];
  const log = options['--log'];
  const seconds = Number(options['--seconds']);
  if (!target || !log || !Number.isInteger(seconds) || seconds <= 0) {
    process.stderr.write('usage: fuzz-summary.cjs --target <name> --log <file> --seconds <n>\n');
    return 2;
  }
  const summary = summarise(fs.readFileSync(log, 'utf8'));
  process.stdout.write(`${plateauLine(target, summary, seconds)}\n`);
  return summary.finished && summary.failure === null ? 0 : 1;
}

process.exitCode = main(process.argv.slice(2));
