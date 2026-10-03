#!/usr/bin/env node
'use strict';

// Usage (as root, on a hosted runner):
//   node sampler.cjs --out samples.jsonl [--interval-ms 1000]
//
// Writes one JSON line per tick: the cgroup and process figures of every
// Docker container then running (lib/cgroup-sampler.cjs). Runs until it gets
// SIGTERM. A load run starts it beside the batches and reads the file after.

const fs = require('node:fs');
const { sample } = require('./lib/cgroup-sampler.cjs');

function main(argv) {
  const options = { 'interval-ms': '1000' };
  for (let index = 0; index < argv.length; index += 2) {
    options[argv[index].replace(/^--/, '')] = argv[index + 1];
  }
  if (!options.out) {
    process.stderr.write('usage: sampler.cjs --out <file> [--interval-ms <n>]\n');
    return 2;
  }
  const interval = Number(options['interval-ms']);
  const fd = fs.openSync(options.out, 'a');
  const tick = () => {
    const record = sample({ cgroupRoot: '/sys/fs/cgroup', procRoot: '/proc' });
    if (record.scopes.length > 0) fs.writeSync(fd, `${JSON.stringify(record)}\n`);
  };
  const timer = setInterval(tick, interval);
  const stop = () => {
    clearInterval(timer);
    fs.closeSync(fd);
    process.exit(0);
  };
  process.on('SIGTERM', stop);
  process.on('SIGINT', stop);
  tick();
  return null;
}

const code = main(process.argv.slice(2));
if (code !== null) process.exitCode = code;
