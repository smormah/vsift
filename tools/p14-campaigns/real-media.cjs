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
//   node real-media.cjs pin <absolute destination> --item RMnn [--from-file <absolute file>] [--manifest <file>]
//   node real-media.cjs cross-check <absolute destination> [--item RMnn]... [--manifest <file>]
//   node real-media.cjs release-files <absolute media dir> <absolute output dir> [--manifest <file>]
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
//   write into the manifest in a reviewed change. With --from-file it takes a copy
//   the maintainer downloaded by hand instead (for a source that refuses the tool):
//   the exact size and the source's published checksum are checked, then the file
//   is copied into place with its ATTRIBUTION.txt; the copy given is left as it is.
//   A fetch contacts an item's mirror_url (the project's test-media release) when
//   it has one, and never a source that refuses the tool (upload.wikimedia.org).
// cross-check: the hosted check of the pins: fetches every approved item (or those
//   named), each held to its pin, pins any not yet pinned, prints each SHA-256 and
//   fails if any item was refused. An item the test-media release does not hold
//   yet is reported "not yet mirrored", which is not a failure.
// release-files: prepares the test-media release for review: checks that every
//   approved item is pinned and present with its pinned digest, writes
//   ATTRIBUTION.txt and NOTES.md to the output folder and prints the
//   `gh release create` command. It never creates the release: a person does.
//   With --partial it takes the approved items already pinned and present, says
//   which are still to come, and prints the `gh release upload` command for them.
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
  const options = { command, manifest: DEFAULT_MANIFEST, items: [], positional: [], fromFile: null, partial: false };
  for (let index = 0; index < rest.length; index += 1) {
    const argument = rest[index];
    if (argument === '--partial') {
      options.partial = true;
    } else if (argument === '--manifest' || argument === '--item' || argument === '--from-file') {
      const value = rest[index + 1];
      if (value === undefined) throw new media.RealMediaError(media.ErrorCode.MANIFEST_INVALID, `${argument} needs a value`);
      if (argument === '--manifest') options.manifest = path.resolve(value);
      else if (argument === '--from-file') options.fromFile = value;
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
      if (options.fromFile) {
        const pinned = await media.pinFromFile(item, destination, options.fromFile);
        process.stdout.write(`${item.id} ${pinned.outcome} from a local copy (size and the source's ${item.source_checksum.algorithm} checked): sha256 ${pinned.sha256}\n`);
        return 0;
      }
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
  if (options.command === 'release-files') {
    // The test-media release, prepared for review: every approved item must be
    // pinned and present in <media dir> with its pinned digest (the assets are
    // verbatim copies); the attribution and the notes are written to <out dir>,
    // and the gh command is printed, never run.
    if (options.positional.length !== 2) throw new media.RealMediaError(media.ErrorCode.DESTINATION_REFUSED, 'name the media folder and an output folder, both absolute');
    const manifest = media.readManifest(options.manifest);
    const source = media.checkDestination(options.positional[0], REPOSITORY);
    const out = media.checkDestination(options.positional[1], REPOSITORY);
    const plan = media.releasePlan(manifest, source, { partial: options.partial });
    const fs = require('node:fs');
    fs.writeFileSync(path.join(out, 'ATTRIBUTION.txt'), plan.attribution);
    fs.writeFileSync(path.join(out, 'NOTES.md'), plan.notes);
    process.stdout.write(`assets now: ${plan.present.join(', ')}${plan.toCome.length > 0 ? `; still to come: ${plan.toCome.join(', ')}` : ''}\n`);
    process.stdout.write(`${plan.command(path.join(out, 'NOTES.md'), path.join(out, 'ATTRIBUTION.txt'))}\n`);
    const upload = plan.upload(path.join(out, 'ATTRIBUTION.txt'));
    if (upload) {
      process.stdout.write('later, once the items still to come are pinned, run release-files again without --partial (it rewrites ATTRIBUTION.txt and NOTES.md for every item), then:\n');
      process.stdout.write(`${upload}\n`);
      process.stdout.write(`gh release edit ${plan.tag} --repo smormah/vsift --notes-file "${path.join(out, 'NOTES.md')}"\n`);
    }
    return 0;
  }
  if (options.command === 'cross-check') {
    // The hosted cross-check of the pins: every approved item fetched here,
    // independently of the machine that pinned it. A pinned item must match its
    // pin (fetch refuses anything else); an unpinned one is pinned here and its
    // SHA-256 printed for the reviewer to compare. Every item is tried and the
    // run fails if any one failed.
    if (options.positional.length !== 1) throw new media.RealMediaError(media.ErrorCode.DESTINATION_REFUSED, 'name exactly one absolute destination folder');
    const manifest = media.readManifest(options.manifest);
    const destination = media.checkDestination(options.positional[0], REPOSITORY);
    let failed = 0;
    let taken = 0;
    for (const item of media.selectApproved(manifest, options.items)) {
      try {
        if (item.sha256 === null) {
          process.stdout.write(`${item.id} pinned here: sha256 ${await media.pinItem(item, destination, media.httpsOpen)} (checked against the source's ${item.source_checksum ? item.source_checksum.algorithm : 'nothing: trust on first use'})\n`);
        } else {
          const outcome = await media.fetchItem(item, destination, media.httpsOpen);
          process.stdout.write(`${item.id} ${outcome}: sha256 ${item.sha256} matches the pin\n`);
        }
        taken += 1;
      } catch (error) {
        if (media.crossCheckOutcome(item, error) === 'not_yet_mirrored') {
          process.stdout.write(`${item.id} not yet mirrored: the test-media release does not hold it yet (not a failure)\n`);
          continue;
        }
        failed += 1;
        process.stdout.write(`${item.id} refused: ${error instanceof media.RealMediaError ? `${error.code}: ${error.message}` : 'unexpected failure'}\n`);
      }
    }
    process.stdout.write(`${taken} item(s) taken, ${failed} refused\n`);
    return failed === 0 ? 0 : 1;
  }
  process.stderr.write('usage: real-media.cjs check|attribution [<manifest>] | fetch|cross-check <dir> [--item RMnn]... | release-files <media dir> <out dir> [--partial] | pin <dir> --item RMnn [--from-file <file>]\n');
  return 2;
}

main(process.argv.slice(2)).then((code) => {
  process.exitCode = code;
}, (error) => {
  process.stderr.write(`${error instanceof media.RealMediaError ? `${error.code}: ${error.message}` : `unexpected failure: ${error && error.message}`}\n`);
  process.exitCode = 1;
});
