#!/usr/bin/env node
'use strict';

// Usage: node scan-inventory.cjs --sbom <file.cdx.json> --metadata <cargo-metadata.json> [--package vsift-cli]
//
// P14 RQ-13, the "Inventory" row of the scan reading (docs/planning/p14-
// qualification.md section 6): compares a release SBOM (a CycloneDX list of
// the Rust dependency graph of one target) with what `cargo metadata` resolves
// for the same crate and target, and prints what each lacks. It reads two
// files and prints; it fetches nothing. The SBOM lists the Rust graph only:
// the native tools VSift installs (FFmpeg, whisper.cpp, the model) are not in
// it, which is stated in the reading, not hidden here.

const fs = require('node:fs');

/** The `name version` of every crate the package reaches by a normal or build dependency. */
function resolved(metadata, packageName) {
  const byId = new Map(metadata.packages.map((entry) => [entry.id, entry]));
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const root = metadata.packages.find((entry) => entry.name === packageName);
  if (!root) throw new Error(`no package ${packageName} in the metadata`);
  const seen = new Set();
  const queue = [root.id];
  while (queue.length > 0) {
    const id = queue.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of (nodes.get(id) || { deps: [] }).deps) {
      if (dep.dep_kinds.some((kind) => kind.kind === null || kind.kind === 'build')) queue.push(dep.pkg);
    }
  }
  return new Set([...seen].map((id) => byId.get(id)).filter(Boolean).map((entry) => `${entry.name} ${entry.version}`));
}

/** The `name version` of every library or application component of a CycloneDX document. */
function listed(sbom) {
  return new Set((sbom.components || []).filter((component) => ['library', 'application'].includes(component.type)).map((component) => `${component.name} ${component.version}`));
}

function compare(sbom, metadata, packageName) {
  const inSbom = listed(sbom);
  const inGraph = resolved(metadata, packageName);
  return {
    sbomComponents: inSbom.size,
    resolvedCrates: inGraph.size,
    missingFromSbom: [...inGraph].filter((entry) => !inSbom.has(entry)).sort(),
    notResolved: [...inSbom].filter((entry) => !inGraph.has(entry)).sort(),
  };
}

function main(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) options[argv[index].replace(/^--/, '')] = argv[index + 1];
  if (!options.sbom || !options.metadata) {
    process.stderr.write('usage: scan-inventory.cjs --sbom <file> --metadata <file> [--package vsift-cli]\n');
    return 2;
  }
  const result = compare(JSON.parse(fs.readFileSync(options.sbom, 'utf8')), JSON.parse(fs.readFileSync(options.metadata, 'utf8')), options.package || 'vsift-cli');
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  return 0;
}

if (require.main === module) process.exitCode = main(process.argv.slice(2));

module.exports = { resolved, listed, compare };
