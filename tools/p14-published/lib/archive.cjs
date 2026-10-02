'use strict';

// What a release archive must hold and how to tell an executable's format from
// its first bytes (release.md section 2). The packager refuses a wrong-format
// executable before an archive is written; this reads the published bytes back,
// for the two targets a runner cannot execute.

const fs = require('node:fs');

/** The executable formats of the three R0 targets. */
const FORMAT_OF_TARGET = Object.freeze({
  'x86_64-pc-windows-msvc': 'pe-x64',
  'aarch64-apple-darwin': 'macho-arm64',
  'x86_64-unknown-linux-gnu': 'elf-x64',
});

/** The files every archive holds beside the executable (the skill is a directory). */
const REQUIRED_FILES = Object.freeze(['LICENSE', 'LICENSE-APACHE', 'LICENSE-MIT', 'THIRD-PARTY-NOTICES', 'vsift.cdx.json']);

/** The executable's name inside an archive. */
function executableOf(archiveTarget) {
  return archiveTarget === 'x86_64-pc-windows-msvc' ? 'vsift.exe' : 'vsift';
}

/** The directory inside `vsift-<version>-<target>.tar.gz`. */
function archiveRoot(version, archiveTarget) {
  return `vsift-${version}-${archiveTarget}`;
}

/** Reads an executable's format from its header. */
function executableFormat(bytes) {
  if (bytes.length >= 0x40 && bytes[0] === 0x4d && bytes[1] === 0x5a) {
    const header = bytes.readUInt32LE(0x3c);
    if (header + 6 <= bytes.length && bytes.readUInt32LE(header) === 0x00004550) {
      return bytes.readUInt16LE(header + 4) === 0x8664 ? 'pe-x64' : 'pe-other';
    }
    return 'pe-other';
  }
  if (bytes.length >= 20 && bytes[0] === 0x7f && bytes[1] === 0x45 && bytes[2] === 0x4c && bytes[3] === 0x46) {
    return bytes[4] === 2 && bytes.readUInt16LE(18) === 0x3e ? 'elf-x64' : 'elf-other';
  }
  if (bytes.length >= 8 && bytes.readUInt32LE(0) === 0xfeedfacf) {
    return bytes.readUInt32LE(4) === 0x0100000c ? 'macho-arm64' : 'macho-other';
  }
  return 'unknown';
}

/** The format of the executable file at `file`. */
function formatOfFile(file) {
  const descriptor = fs.openSync(file, 'r');
  try {
    const buffer = Buffer.alloc(4096);
    const read = fs.readSync(descriptor, buffer, 0, buffer.length, 0);
    return executableFormat(buffer.subarray(0, read));
  } finally {
    fs.closeSync(descriptor);
  }
}

module.exports = { FORMAT_OF_TARGET, REQUIRED_FILES, archiveRoot, executableFormat, executableOf, formatOfFile };
