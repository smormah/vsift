'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const archive = require('../lib/archive.cjs');

function pe(machine) {
  const bytes = Buffer.alloc(0x100);
  bytes.write('MZ');
  bytes.writeUInt32LE(0x80, 0x3c);
  bytes.writeUInt32LE(0x00004550, 0x80);
  bytes.writeUInt16LE(machine, 0x84);
  return bytes;
}

function elf(machine, classByte = 2) {
  const bytes = Buffer.alloc(64);
  bytes.set([0x7f, 0x45, 0x4c, 0x46, classByte]);
  bytes.writeUInt16LE(machine, 18);
  return bytes;
}

function macho(cpu) {
  const bytes = Buffer.alloc(32);
  bytes.writeUInt32LE(0xfeedfacf, 0);
  bytes.writeUInt32LE(cpu, 4);
  return bytes;
}

test('the executable format is read from the header, for the three R0 targets and their lookalikes', () => {
  assert.equal(archive.executableFormat(pe(0x8664)), 'pe-x64');
  assert.equal(archive.executableFormat(pe(0xaa64)), 'pe-other');
  assert.equal(archive.executableFormat(elf(0x3e)), 'elf-x64');
  assert.equal(archive.executableFormat(elf(0xb7)), 'elf-other');
  assert.equal(archive.executableFormat(elf(0x3e, 1)), 'elf-other');
  assert.equal(archive.executableFormat(macho(0x0100000c)), 'macho-arm64');
  assert.equal(archive.executableFormat(macho(0x01000007)), 'macho-other');
  for (const bytes of [Buffer.alloc(0), Buffer.from('#!/bin/sh\n'), Buffer.from('MZ'), Buffer.alloc(100)]) {
    assert.equal(archive.executableFormat(bytes), 'unknown');
  }
});

test('each target has its format, its executable name and its archive folder', () => {
  assert.deepEqual(Object.values(archive.FORMAT_OF_TARGET).sort(), ['elf-x64', 'macho-arm64', 'pe-x64']);
  assert.equal(archive.executableOf('x86_64-pc-windows-msvc'), 'vsift.exe');
  assert.equal(archive.executableOf('aarch64-apple-darwin'), 'vsift');
  assert.equal(archive.archiveRoot('0.1.0', 'x86_64-unknown-linux-gnu'), 'vsift-0.1.0-x86_64-unknown-linux-gnu');
});

test('the format of a file is read from its first bytes', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 archive '));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const file = path.join(directory, 'vsift');
  fs.writeFileSync(file, elf(0x3e));
  assert.equal(archive.formatOfFile(file), 'elf-x64');
});
