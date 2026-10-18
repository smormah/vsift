#!/usr/bin/env node
'use strict';

// #363: the small, freely licensed real-media set (docs/planning/real-media-set.md).
// The media never enters the repository; real-media/manifest.json pins each
// item's URL, size, SHA-256, licence and attribution, and this tool fetches an
// approved item on demand into a folder outside the repository.
//
//   node real-media.cjs check [<manifest>]
//   node real-media.cjs attribution [<manifest>]
//   node real-media.cjs fetch <absolute destination> [--manifest <file>] [--item RMnn]...
//   node real-media.cjs pin <absolute destination> --item RMnn [--manifest <file>]
//
// check: validates the manifest and lists its items and the shapes it covers.
// attribution: prints the attribution and licence of every item that is not withdrawn.
// fetch: fetches approved, pinned items (all of them, or those named) over HTTPS,
//   bounded by the pinned size and verified by the pinned SHA-256 and the
//   source's own checksum before the file is placed at <destination>/<id>/<file>,
//   with ATTRIBUTION.txt beside it. A file already there with the pinned digest is
//   kept; one with another digest is refused and left alone. Nothing is deleted
//   but this run's own partial file.
// pin: for an approved item whose SHA-256 is not yet pinned: fetches it bounded
//   by its size, verifies the source's own checksum and prints the SHA-256 to
//   write into the manifest in a reviewed change.
//
// Fetch and pin download media: run them only for items the maintainer approved,
// and only where downloading is allowed (a hosted runner, or the maintainer's own
// go on a workstation). Node.js 22, no dependency.

const path = require('node:path');
const media = require('./lib/real-media.cjs');

const REPOSITORY = path.resolve(__dirname, '..', '..');
const DEFAULT_MANIFEST = path.join(__dirname, 'real-media', 'manifest.json');

function parse(argv) {
  const [command, ...rest] = argv;
  const options = { command, manifest: DEFAULT_MANIFEST, items: [], positional: [] };
  for (let index = 0; index < rest.length; index += 1) {
    const argument = rest[index];
    if (argument === '--manifest' || argument === '--item') {
      const value = rest[index + 1];
      if (value === undefined) throw new media.RealMediaError(media.ErrorCode.MANIFEST_INVALID, `${argument} needs a value`);
      if (argument === '--manifest') options.manifest = path.resolve(value);
      else options.items.push(value);
      index += 1;
    } else {
      options.positional.push(argument);
    }
  }
  return options;
}

async function main(argv) {
  const options = parse(argv);
  if (options.command === 'check' || options.command === 'attribution') {
    const manifest = media.readManifest(options.positional[0] ? path.resolve(options.positional[0]) : options.manifest);
    const live = manifest.items.filter((item) => item.status !== 'withdrawn');
    if (options.command === 'attribution') {
      for (const item of live) process.stdout.write(`${media.attributionText(item)}\n`);
      return 0;
    }
    for (const item of manifest.items) {
      process.stdout.write(`${item.id} ${item.status.padEnd(9)} ${String(item.bytes).padStart(10)} B  ${item.covers.join(', ')}\n`);
    }
    const total = live.reduce((sum, item) => sum + item.bytes, 0);
    const missing = media.uncoveredShapes(manifest);
    process.stdout.write(`${live.length} items, ${total} bytes; uncovered shapes: ${missing.length === 0 ? 'none' : missing.join(', ')}\n`);
    return missing.length === 0 ? 0 : 1;
  }
  if (options.command === 'fetch' || options.command === 'pin') {
    if (options.positional.length !== 1) throw new media.RealMediaError(media.ErrorCode.DESTINATION_REFUSED, 'name exactly one absolute destination folder');
    const manifest = media.readManifest(options.manifest);
    const destination = media.checkDestination(options.positional[0], REPOSITORY);
    if (options.command === 'pin') {
      if (options.items.length !== 1) throw new media.RealMediaError(media.ErrorCode.NOT_APPROVED, 'pin takes exactly one --item');
      const [item] = media.selectApproved(manifest, options.items);
      const sha256 = await media.pinItem(item, destination, media.httpsOpen);
      process.stdout.write(`${item.id} sha256 ${sha256}\n`);
      return 0;
    }
    for (const item of media.selectApproved(manifest, options.items)) {
      const outcome = await media.fetchItem(item, destination, media.httpsOpen);
      process.stdout.write(`${item.id} ${outcome}\n`);
    }
    return 0;
  }
  process.stderr.write('usage: real-media.cjs check|attribution [<manifest>] | fetch <dir> [--item RMnn]... | pin <dir> --item RMnn\n');
  return 2;
}

main(process.argv.slice(2)).then((code) => {
  process.exitCode = code;
}, (error) => {
  process.stderr.write(`${error instanceof media.RealMediaError ? `${error.code}: ${error.message}` : `unexpected failure: ${error && error.message}`}\n`);
  process.exitCode = 1;
});
