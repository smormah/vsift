#!/usr/bin/env node
'use strict';

// Usage (on a hosted ubuntu-24.04 runner):
//   node prepare-host.cjs --work <dir> --out <dir> --repo <checkout> [--version <published version>]
//
// Makes the runner a worker host for the P14 campaigns: the published `vsift`
// from the real registry, the worker image, the folders of worker-host.md
// section 1, the three reviewed tools installed by the published binary, and
// the corpus videos as the input root. Writes `host.json` into --out. Never run
// it on a machine whose state matters (it creates /var/lib/vsift and /srv/vsift
// and builds an image).

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const host = require('./lib/host.cjs');

function parse(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) options[argv[index].replace(/^--/, '')] = argv[index + 1];
  for (const required of ['work', 'out', 'repo']) {
    if (!options[required]) throw new Error(`--${required} is required`);
  }
  return options;
}

/** A transcript sidecar for F10 whose first cue carries a canary the campaigns must never see in an output. */
function canarySidecar(canary) {
  return [
    '1', '00:00:01,000 --> 00:00:03,000', `${canary} is displayed now.`, '',
    '2', '00:00:04,000 --> 00:00:06,000', 'The second cue is plain.', '',
    '3', '00:00:07,000 --> 00:00:10,000', 'The third cue closes the clip.', '',
  ].join('\n');
}

function main() {
  const options = parse(process.argv.slice(2));
  fs.mkdirSync(options.out, { recursive: true });
  const log = (message) => process.stdout.write(`${message}\n`);

  log('installing the published package from the real registry');
  const published = host.installPublished({ version: options.version || '', prefix: path.join(options.work, 'published') });
  log(`  ${published.versionLine}`);

  log('creating the worker folders (runbook section 1)');
  const layout = host.createLayout();
  log(`  state folder: ${layout.state.mounted ? 'ext4 volume with write barriers, ' : 'no volume made, '}${layout.state.detail}`);
  log(`  bundle root: ${layout.bundles.mounted ? 'ext4 volume with write barriers, ' : 'no volume made, '}${layout.bundles.detail}`);

  log('building the worker image around the published executable');
  const imageId = host.buildImage({ binary: published.binary, workDir: options.work });
  log(`  image ${host.IMAGE} is ${imageId}`);

  log('staging the corpus as the input root');
  host.stageInputs(options.repo);
  const canary = `CANARY-TRANSCRIPT-${crypto.randomBytes(8).toString('hex')}`;
  const sidecar = path.join(options.work, 'canary.srt');
  fs.writeFileSync(sidecar, canarySidecar(canary));
  host.sudo(['cp', sidecar, `${host.INPUTS}/incoming/canary.srt`]);
  host.sudo(['chmod', '0644', `${host.INPUTS}/incoming/canary.srt`]);

  log('installing the reviewed tools with the published binary (setup plan, setup install, setup check)');
  const tools = host.installManagedTools();
  const readiness = tools.check.status;
  log(`  setup check readiness: ${readiness}`);

  const record = {
    version: published.version,
    versionLine: published.versionLine,
    image: { tag: host.IMAGE, id: imageId },
    layout,
    host: host.describeHost(),
    tools: {
      planDigest: tools.plan.data && tools.plan.data.plan_digest,
      installStatus: tools.install.status,
      readiness,
      check: tools.check,
    },
    canary,
  };
  fs.writeFileSync(path.join(options.out, 'host.json'), `${JSON.stringify(record, null, 2)}\n`);
  if (readiness !== 'ready') {
    log('the tools are not ready; see host.json');
    process.exitCode = 1;
  }
}

try {
  main();
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 2;
}
