#!/usr/bin/env node
'use strict';

// P14 RQ-12: the worker runbook (docs/operations/worker-host.md) walked step
// by step with the PUBLISHED `vsift` on a hosted Ubuntu 24.04 runner: the
// layout of section 1, the one-time setup of section 2, the supervisor
// invocation and the systemd unit of section 3 (read from the runbook itself,
// not retyped), the acknowledgement table of section 4, recovery and cleanup
// of sections 6 and 7 and the container of section 10. Each step states what
// the runbook promises and what happened; a step that does not match is a
// finding (a documentation error, or a product defect), never an edit of the
// expectation.
//
//   node runbook-walk.cjs --out <dir> --repo <checkout> --binary <published vsift>
//
// Never run it on a machine whose state matters: it creates an account, folders
// under /var/lib and /srv, a systemd unit, and starts and stops services.

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const childProcess = require('node:child_process');
const host = require('./lib/host.cjs');
const container = require('./lib/container.cjs');
const events = require('./lib/events.cjs');

const STATE = '/var/lib/vsift';
const HOME = `${STATE}/home`;
const WORKSPACE = `${STATE}/workspace`;
const BINARY = '/opt/vsift/bin/vsift';
const RUNBOOK = 'docs/operations/worker-host.md';

function parse(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) options[argv[index].replace(/^--/, '')] = argv[index + 1];
  for (const required of ['out', 'repo', 'binary']) if (!options[required]) throw new Error(`--${required} is required`);
  return options;
}

const log = (message) => process.stdout.write(`${new Date().toISOString().slice(11, 19)} ${message}\n`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** The step table: what the walk did and what came of it. */
const record = [];

async function step(section, title, expected, action) {
  log(`${section} ${title}`);
  let outcome;
  try {
    outcome = await action();
  } catch (error) {
    outcome = { ok: false, observed: `the step failed to run: ${error.message}` };
  }
  const entry = { section, title, expected, observed: outcome.observed, ok: Boolean(outcome.ok), note: outcome.note || '' };
  record.push(entry);
  if (!entry.ok) log(`  DIVERGED: ${entry.observed}`);
  return entry;
}

/** vsift as the worker account, with the worker's own HOME, on the bare host. */
function asWorker(args, options = {}) {
  return host.run('sudo', ['-n', '-u', 'vsift', 'env', `HOME=${HOME}`, `XDG_CONFIG_HOME=${HOME}/.config`, `XDG_CACHE_HOME=${HOME}/.cache`, BINARY, ...args], options);
}

function json(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/** The first fenced block of `language` under the heading that starts with `heading`. */
function fenced(markdown, heading, language) {
  const start = markdown.indexOf(heading);
  if (start < 0) return null;
  const open = markdown.indexOf(`\`\`\`${language}`, start);
  if (open < 0) return null;
  const from = markdown.indexOf('\n', open) + 1;
  return markdown.slice(from, markdown.indexOf('\n```', from));
}

function writeAsRoot(file, text, owner) {
  host.sudo(['sh', '-c', `cat > ${file}${owner ? ` && chown ${owner} ${file}` : ''}`], { input: text });
}

async function waitFor(check, timeoutMs, everyMs = 500) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    const found = check();
    if (found) return found;
    await sleep(everyMs);
  }
  return null;
}

function request(operation, source, steps, over = {}) {
  return JSON.stringify({ schema_version: '1', operation_id: operation, durability: over.durability || 'durable', deadline_ms: 3600000, target: { ingest: { source, transcript: null } }, steps });
}

async function main() {
  const options = parse(process.argv.slice(2));
  fs.mkdirSync(options.out, { recursive: true });
  const runbook = fs.readFileSync(path.join(options.repo, RUNBOOK), 'utf8');
  let hostInfo = host.describeHost();
  let durability = 'durable';
  let image = null;
  let volumes = null;

  // The published executable is where the runbook's unit expects it.
  host.sudo(['install', '-D', '-m', '0755', options.binary, BINARY]);
  const versionLine = host.versionLine(BINARY);

  await step('1', 'Layout: the worker account and the folders of the table', 'an account `vsift` exists; `home` 0700 and the bundle root are owned by it; the input root is readable by it and written by nobody', () => {
    // The runbook names the owner but gives no command; the walk makes the account as an operator would.
    host.sudo(['useradd', '--system', '--uid', '10001', '--home-dir', HOME, '--no-create-home', '--shell', '/usr/sbin/nologin', '--user-group', 'vsift']);
    // The runbook asks for local ext4 with write barriers under the state folder; a hosted runner's own disk is
    // mounted without them, so the operator here makes an ext4 volume in a file for it and for the bundle root.
    volumes = { state: host.mountBarrierVolume(STATE, 24), bundles: host.mountBarrierVolume('/srv/vsift/bundles', 8) };
    host.sudo(['mkdir', '-p', HOME, `${STATE}/queue`, `${STATE}/results`, '/srv/vsift/inputs/incoming/2026-09-28', '/srv/vsift/bundles']);
    host.sudo(['chown', '-R', 'vsift:vsift', STATE, '/srv/vsift/bundles']);
    host.sudo(['chmod', '0700', HOME, '/srv/vsift/bundles']);
    const listing = host.run('stat', ['-c', '%U %a %n', HOME, '/srv/vsift/bundles', '/srv/vsift/inputs']).stdout.trim();
    return { ok: /vsift 700 .*home/.test(listing) && /vsift 700 \/srv\/vsift\/bundles/.test(listing), observed: `${listing.replace(/\n/g, '; ')}; state volume: ${volumes.state.detail}; bundle volume: ${volumes.bundles.detail}`, note: 'the runbook lists owners and modes but no command that creates the account or the folders, and its container example runs as uid 10001 without saying the host account `vsift` must have that uid for the volumes to be writable; its "local ext4 with write barriers" had to be made here as an ext4 volume in a file, because a hosted runner\'s own disk is mounted without barriers' };
  });

  hostInfo = host.describeHost();
  host.sudo(['cp', path.join(options.repo, 'fixtures/corpus/generated/F01-speech.mp4'), '/srv/vsift/inputs/incoming/2026-09-28/walkthrough.mp4']);
  for (const name of ['F01-speech.mp4', 'F03-speech.mp4', 'F06-speech.mp4', 'F07-speech.mp4', 'F01.mp4', 'F10.mp4']) host.sudo(['cp', path.join(options.repo, 'fixtures/corpus/generated', name), `/srv/vsift/inputs/incoming/${name}`]);
  host.sudo(['chmod', '-R', 'a+rX', '/srv/vsift/inputs']);

  // The tools: installed by the published binary as the worker account, then registered as the runbook's section 2 says.
  const installed = await step('2', 'Tools: the reviewed tools, installed as the worker account (the runbook assumes they exist)', 'setup plan and setup install succeed for the worker account; setup check is ready', () => {
    const plan = asWorker(['setup', 'plan', '--profile', 'worker', '--json']);
    const planJson = json(plan.stdout);
    const digest = planJson && planJson.data && planJson.data.plan_digest;
    if (!digest) return { ok: false, observed: `no plan digest: ${plan.stdout.slice(0, 200)}` };
    writeAsRoot(`${STATE}/queue/plan.json`, plan.stdout, 'vsift');
    const install = asWorker(['setup', 'install', '--plan', `${STATE}/queue/plan.json`, '--accept-plan', digest, '--json'], { timeoutMs: 30 * 60 * 1000 });
    return { ok: install.code === 0, observed: `setup install exit ${install.code}: ${install.stdout.slice(0, 200)}`, note: 'the runbook points at install.md only by implication; its one-time setup starts from tools already on the machine' };
  });
  if (!installed.ok) return finish(options, record, hostInfo, versionLine, image);

  await step('2', '`setup configure` and `setup check` as written, with the managed tools\' paths for `/usr/bin/ffmpeg`, `/opt/whisper.cpp/...`', 'each configure answers; setup check is ready and names each tool as configured', () => {
    const find = (name) => host.run('sudo', ['-n', 'find', `${HOME}/.local/share/vsift`, '-type', 'f', '-name', name, '-perm', '-u+x']).stdout.split('\n').find(Boolean);
    const model = host.run('sudo', ['-n', 'find', `${HOME}/.local/share/vsift`, '-type', 'f', '-name', 'ggml-base*.bin']).stdout.split('\n').find(Boolean);
    const paths = { ffmpeg: find('ffmpeg'), ffprobe: find('ffprobe'), whisper: find('whisper-cli') };
    if (!paths.ffmpeg || !paths.ffprobe || !paths.whisper || !model) return { ok: false, observed: `tools not found under the managed folder: ${JSON.stringify({ ...paths, model })}` };
    const answers = [];
    for (const [name, executable] of Object.entries(paths)) answers.push(asWorker(['setup', 'configure', name, '--executable', executable, '--json']).code);
    answers.push(asWorker(['setup', 'configure-model', '--file', model, '--json']).code);
    const check = asWorker(['setup', 'check', '--json']);
    const checkJson = json(check.stdout);
    return { ok: answers.every((code) => code === 0) && Boolean(checkJson) && checkJson.status === 'ready', observed: `configure exits ${answers.join(',')}; setup check ${checkJson ? checkJson.status : check.stdout.slice(0, 120)}`, note: 'the runbook\'s example paths (`/usr/bin/ffmpeg`, `/opt/whisper.cpp/bin/whisper-cli`, `.../ggml-base.bin`) are the operator\'s own tools; here the same reviewed files were registered by path' };
  });

  let durableCreated = false;
  await step('2', '`session init-workspace --durability durable --admission-slots 8 --retention-hours 168`', 'created on Ubuntu 24.04 with local ext4 and write barriers; otherwise MISSING_CAPABILITY and nothing created', () => {
    const created = asWorker(['--session-root', WORKSPACE, 'session', 'init-workspace', '--durability', 'durable', '--admission-slots', '8', '--retention-hours', '168', '--json']);
    const result = json(created.stdout);
    if (created.code === 0) {
      durableCreated = true;
      return { ok: true, observed: `created: ${JSON.stringify(result.data)}` };
    }
    // The runbook says durable is accepted only with write barriers; a refusal on a filesystem mounted without them is what it promises.
    const code = result && result.error ? result.error.code : null;
    const withoutBarriers = /nobarrier/.test(hostInfo.storage);
    return { ok: code === 'MISSING_CAPABILITY' && withoutBarriers, observed: `exit ${created.code}, ${code}: ${created.stdout.slice(0, 200)}`, note: `the hosted runner's root filesystem is mounted: ${hostInfo.storage}, so the durable example cannot run here (the refusal is the documented one)` };
  });
  if (!durableCreated) {
    durability = 'ephemeral';
    await step('2', 'the runbook\'s alternative: `--durability ephemeral` when durable is refused', 'created; sessions survive a crash or kill of VSift, not an OS crash', () => {
      const created = asWorker(['--session-root', WORKSPACE, 'session', 'init-workspace', '--durability', 'ephemeral', '--admission-slots', '8', '--retention-hours', '168', '--json']);
      return { ok: created.code === 0, observed: `exit ${created.code}: ${created.stdout.slice(0, 200)}` };
    });
  }
  await step('2', 'the same `init-workspace` again', 'answers `already_initialized`', () => {
    const again = asWorker(['--session-root', WORKSPACE, 'session', 'init-workspace', '--durability', durability, '--admission-slots', '8', '--retention-hours', '168', '--json']);
    const result = json(again.stdout);
    return { ok: again.code === 0 && result && result.data && result.data.outcome === 'already_initialized', observed: `exit ${again.code}: ${again.stdout.slice(0, 160)}` };
  });
  await step('2', 'another policy against the existing workspace', 'refused', () => {
    const other = asWorker(['--session-root', WORKSPACE, 'session', 'init-workspace', '--durability', durability, '--admission-slots', '4', '--retention-hours', '168', '--json']);
    return { ok: other.code !== 0, observed: `exit ${other.code}: ${other.stdout.slice(0, 160)}` };
  });

  // Section 3: the example request, verbatim, and the invocation on the bare host.
  const exampleRequest = fenced(runbook, '## 3. Supervisor invocation', 'json');
  const requestObject = exampleRequest ? JSON.parse(exampleRequest) : null;
  if (requestObject) requestObject.durability = durability;
  const exampleText = requestObject ? JSON.stringify(requestObject) : null;
  await step('3', 'the example `request.json` is valid as printed', 'a request the engine accepts (a v1 job request)', () => ({ ok: Boolean(requestObject) && requestObject.schema_version === '1', observed: exampleRequest ? `${exampleRequest.length} bytes, operation ${requestObject.operation_id}` : 'no JSON block under section 3' }));
  if (!exampleText) return finish(options, record, hostInfo, versionLine, image);
  writeAsRoot(`${STATE}/queue/3f9c2a7e.json`, exampleText, 'vsift');

  const invocation = ['--host-isolation', 'strict-linux', '--session-root', WORKSPACE, 'job', 'run', '--request', `${STATE}/queue/3f9c2a7e.json`, '--input-root', '/srv/vsift/inputs', '--bundle-root', '/srv/vsift/bundles', '--admission-wait-ms', '60000', '--drain-timeout-ms', '30000', '--events', 'jsonl'];
  await step('3', 'the supervisor invocation, run as printed in a plain shell as the worker account', 'a failure before the first event is the only event (isolation not attested outside the unit or the container): ISOLATION_UNAVAILABLE', () => {
    const answered = asWorker(invocation);
    const parsed = events.parseEvents(answered.stdout);
    const last = parsed.events[parsed.events.length - 1] || {};
    const code = last.result && last.result.error ? last.result.error.code : null;
    return { ok: code === 'ISOLATION_UNAVAILABLE' && parsed.events.length === 1, observed: `exit ${answered.code}; ${parsed.events.length} event(s); ${code || answered.stdout.slice(0, 160)}`, note: 'the runbook prints this command as the invocation without saying it needs the hardened unit or container to attest isolation' };
  });

  // Section 10: the container, with the runbook's flags scaled to the runner (its example asks for 8 CPUs).
  image = host.buildImage({ binary: options.binary, workDir: path.join(path.dirname(path.resolve(options.out)), 'p14-walk-image') });
  const containerArgs = (name, command) => ['run', '--rm', '--init', '--name', name, '--network', 'none', '--read-only', '--tmpfs', '/tmp:rw,noexec,nosuid,size=64m', '--cpus', '4', '--memory', '12g', '--memory-swap', '12g', '--pids-limit', '256', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--user', '10001:10001', '--env', `HOME=${HOME}`, '--env', `XDG_CONFIG_HOME=${HOME}/.config`, '--volume', '/srv/vsift/inputs:/srv/vsift/inputs:ro', '--volume', `${STATE}:${STATE}:rw`, '--volume', '/srv/vsift/bundles:/srv/vsift/bundles:rw', '--stop-timeout', '45', image, ...command];
  await step('10', 'the container example (`docker run ... vsift ... job batch`) with the same hardening, a `job run` of the example request', 'strict isolation attested; the request completes; the events begin with lifecycle started with readiness', () => {
    const answered = host.run('docker', containerArgs('vsift-walk-run', ['vsift', '--host-isolation', 'strict-linux', '--session-root', WORKSPACE, 'job', 'run', '--request', `${STATE}/queue/3f9c2a7e.json`, '--input-root', '/srv/vsift/inputs', '--bundle-root', '/srv/vsift/bundles', '--admission-wait-ms', '60000', '--drain-timeout-ms', '30000', '--events', 'jsonl']), { timeoutMs: 20 * 60 * 1000 });
    const batch = events.readBatch(events.parseEvents(answered.stdout).events);
    const terminal = batch.terminal || {};
    return { ok: answered.code === 0 && batch.started && batch.started.isolation === 'strict_linux', observed: `exit ${answered.code}; started ${JSON.stringify(batch.started)}; terminal status ${terminal.status}${answered.code === 0 ? '' : `; said: ${(answered.stdout + answered.stderr).replace(/\s+/g, ' ').slice(0, 500)}`}`, note: `the runbook's example asks for 8 CPUs and names an image it does not say how to build; this run used 4 CPUs on a ${hostInfo.cpus}-CPU runner and an image built from tools/p14-campaigns/worker.Dockerfile` };
  });

  // The acknowledgement table (section 4), through the container.
  await step('4', 'redelivery of the finished request', '`replayed: true`, nothing runs again', () => {
    const answered = host.run('docker', containerArgs('vsift-walk-replay', ['vsift', '--host-isolation', 'strict-linux', '--session-root', WORKSPACE, 'job', 'run', '--request', `${STATE}/queue/3f9c2a7e.json`, '--input-root', '/srv/vsift/inputs', '--bundle-root', '/srv/vsift/bundles', '--json']), { timeoutMs: 10 * 60 * 1000 });
    const result = json(answered.stdout);
    return { ok: answered.code === 0 && result && result.data && result.data.replayed === true, observed: `exit ${answered.code}; replayed ${result && result.data ? result.data.replayed : '?'}` };
  });
  await step('4', 'the same operation id with another request', '`IDEMPOTENCY_CONFLICT`, exit 2, nothing changed', () => {
    const other = JSON.stringify({ ...JSON.parse(exampleText), steps: [{ close: {} }] });
    writeAsRoot(`${STATE}/queue/conflict.json`, other, 'vsift');
    const answered = host.run('docker', containerArgs('vsift-walk-conflict', ['vsift', '--host-isolation', 'strict-linux', '--session-root', WORKSPACE, 'job', 'run', '--request', `${STATE}/queue/conflict.json`, '--input-root', '/srv/vsift/inputs', '--bundle-root', '/srv/vsift/bundles', '--json']), { timeoutMs: 10 * 60 * 1000 });
    const result = json(answered.stdout);
    const code = result && ((result.error && result.error.code) || (result.data && result.data.failure && result.data.failure.code));
    return { ok: code === 'IDEMPOTENCY_CONFLICT' && answered.code === 2, observed: `exit ${answered.code}; ${code}` };
  });
  await step('4', 'a duplicate delivered while the first runs', '`BUSY` (exit 4, `retry_after_ms` 2000) for the second; the first completes', async () => {
    const id = `op_${crypto.createHash('sha256').update('busy').digest('hex').slice(0, 32)}`;
    writeAsRoot(`${STATE}/queue/busy.json`, request(id, 'incoming/F06-speech.mp4', [{ retranscribe: { range: null } }, { close: {} }], { durability }), 'vsift');
    const command = (name) => containerArgs(name, ['vsift', '--host-isolation', 'strict-linux', '--session-root', WORKSPACE, 'job', 'run', '--request', `${STATE}/queue/busy.json`, '--input-root', '/srv/vsift/inputs', '--bundle-root', '/srv/vsift/bundles', '--json']);
    const first = childProcess.spawn('docker', command('vsift-walk-busy-1'), { stdio: ['ignore', 'pipe', 'pipe'] });
    let firstOut = '';
    first.stdout.on('data', (chunk) => { firstOut += chunk; });
    // Waiting must not depend on a 'close' that may already have happened.
    const firstDone = new Promise((resolve) => first.on('close', resolve));
    await sleep(2500);
    const second = host.run('docker', command('vsift-walk-busy-2'), { timeoutMs: 5 * 60 * 1000 });
    await firstDone;
    const secondJson = json(second.stdout);
    const code = secondJson && ((secondJson.error && secondJson.error.code) || (secondJson.data && secondJson.data.failure && secondJson.data.failure.code));
    const retry = secondJson && secondJson.error ? secondJson.error.retry_after_ms : null;
    return { ok: code === 'BUSY' && second.code === 4 && retry === 2000, observed: `second delivery: exit ${second.code}, ${code}, retry_after_ms ${retry}; first: ${(json(firstOut) || {}).status}` };
  });

  // The batch file of 1,001 lines is refused whole (L-066).
  await step('3', 'a batch file of 1,001 lines', 'refused whole: RESOURCE_LIMIT, exit 5, `termination_reason` `line_limit`, nothing run', () => {
    writeAsRoot(`${STATE}/queue/long.jsonl`, `${Array.from({ length: 1001 }, () => '').join('\n')}\n`, 'vsift');
    const answered = host.run('docker', containerArgs('vsift-walk-long', ['vsift', '--host-isolation', 'strict-linux', '--session-root', WORKSPACE, 'job', 'batch', '--requests', `${STATE}/queue/long.jsonl`, '--input-root', '/srv/vsift/inputs', '--bundle-root', '/srv/vsift/bundles', '--concurrency', '2', '--events', 'jsonl']), { timeoutMs: 5 * 60 * 1000 });
    const batch = events.readBatch(events.parseEvents(answered.stdout).events);
    const terminal = batch.terminal || {};
    const reason = terminal.data && terminal.data.termination_reason;
    return { ok: answered.code === 5 && reason === 'line_limit', observed: `exit ${answered.code}; termination_reason ${reason}`, note: 'blank lines count' };
  });

  // The systemd unit, read from the runbook.
  const unit = fenced(runbook, '### Example systemd unit', 'ini');
  await step('3', 'the example systemd unit, installed exactly as printed, started for batch 0042', 'it starts; the events file begins with lifecycle started and `"isolation":"strict_linux"`', async () => {
    if (!unit) return { ok: false, observed: 'no ini block under "Example systemd unit"' };
    writeAsRoot('/etc/systemd/system/vsift-batch@.service', unit);
    host.sudo(['systemctl', 'daemon-reload']);
    const lines = [];
    const sources = ['F01-speech.mp4', 'F03-speech.mp4', 'F06-speech.mp4', 'F07-speech.mp4'];
    for (let index = 0; index < 16; index += 1) {
      const id = `op_${crypto.createHash('sha256').update(`unit:${index}`).digest('hex').slice(0, 32)}`;
      lines.push(request(id, `incoming/${sources[index % 4]}`, [{ retranscribe: { range: null } }, { candidates: { range: null } }, { close: {} }], { durability }));
    }
    writeAsRoot(`${STATE}/queue/batch-0042.jsonl`, `${lines.join('\n')}\n`, 'vsift');
    const started = host.run('sudo', ['-n', 'systemctl', 'start', 'vsift-batch@0042']);
    const ready = await waitFor(() => {
      const text = host.run('sudo', ['-n', 'cat', `${STATE}/results/batch-0042.events.jsonl`]).stdout;
      return text.includes('"kind":"started"') ? text : null;
    }, 60_000);
    if (!ready) {
      const journal = host.run('sudo', ['-n', 'journalctl', '-u', 'vsift-batch@0042', '--no-pager', '-n', '20']).stdout;
      return { ok: false, observed: `start exit ${started.code}, no started event in 60 s. Journal: ${journal.slice(-600)}` };
    }
    const batch = events.readBatch(events.parseEvents(ready).events);
    return { ok: batch.started && batch.started.isolation === 'strict_linux', observed: `readiness ${JSON.stringify(batch.started)}` };
  });
  await step('3', 'stop the unit while requests run (`systemctl stop`)', 'SIGTERM to vsift alone; it stops admitting, drains 30 s, ends within TimeoutStopSec; the events file ends with lifecycle stopped and the terminal event; the unit exits non-zero (exit 6, a shutdown)', async () => {
    await sleep(8000);
    const stopStart = Date.now();
    const stopped = host.run('sudo', ['-n', 'systemctl', 'stop', 'vsift-batch@0042'], { timeoutMs: 120_000 });
    const seconds = Math.round((Date.now() - stopStart) / 100) / 10;
    const text = host.run('sudo', ['-n', 'cat', `${STATE}/results/batch-0042.events.jsonl`]).stdout;
    const batch = events.readBatch(events.parseEvents(text).events);
    const show = host.run('systemctl', ['show', 'vsift-batch@0042', '--property=ExecMainStatus,Result,ActiveState']).stdout.replace(/\n/g, ' ');
    fs.writeFileSync(path.join(options.out, 'unit-stop-events.jsonl'), text);
    return { ok: stopped.code === 0 && batch.ended && batch.stopped === 'shutdown' && seconds <= 45, observed: `stop took ${seconds} s; stopped reason ${batch.stopped}; terminal event ${batch.ended}; ${show.trim()}`, note: 'the runbook says the unit then reports a non-zero status' };
  });
  await step('6', 'start the same unit again (the supervisor redelivers every unacknowledged message)', 'finished requests replay, interrupted ones continue; the batch ends complete', async () => {
    host.run('sudo', ['-n', 'systemctl', 'reset-failed', 'vsift-batch@0042']);
    host.run('sudo', ['-n', 'systemctl', 'start', 'vsift-batch@0042']);
    const done = await waitFor(() => {
      const state = host.run('systemctl', ['is-active', 'vsift-batch@0042']).stdout.trim();
      return state === 'active' || state === 'activating' ? null : state;
    }, 25 * 60 * 1000, 3000);
    const text = host.run('sudo', ['-n', 'cat', `${STATE}/results/batch-0042.events.jsonl`]).stdout;
    const batch = events.readBatch(events.parseEvents(text).events);
    fs.writeFileSync(path.join(options.out, 'unit-restart-events.jsonl'), text);
    const recorded = [...batch.outcomes.values()].filter((outcome) => events.disposition(outcome) === 'recorded').length;
    return { ok: done !== null && batch.ended && recorded === 16, observed: `unit ${done}; ${recorded} of 16 lines recorded; stopped reason ${batch.stopped}` };
  });
  await step('6', 'a SIGKILL of the whole unit mid-batch, then redelivery', 'the interrupted requests continue from their first unfinished step; the redelivered batch is complete', async () => {
    const lines = [];
    for (let index = 0; index < 8; index += 1) {
      const id = `op_${crypto.createHash('sha256').update(`kill:${index}`).digest('hex').slice(0, 32)}`;
      lines.push(request(id, `incoming/${['F06-speech.mp4', 'F03-speech.mp4'][index % 2]}`, [{ retranscribe: { range: null } }, { close: {} }], { durability }));
    }
    writeAsRoot(`${STATE}/queue/batch-0043.jsonl`, `${lines.join('\n')}\n`, 'vsift');
    host.sudo(['systemctl', 'start', 'vsift-batch@0043']);
    await sleep(9000);
    host.run('sudo', ['-n', 'systemctl', 'kill', '--signal=SIGKILL', '--kill-whom=all', 'vsift-batch@0043']);
    await sleep(3000);
    host.run('sudo', ['-n', 'systemctl', 'reset-failed', 'vsift-batch@0043']);
    host.run('sudo', ['-n', 'systemctl', 'start', 'vsift-batch@0043']);
    const done = await waitFor(() => {
      const state = host.run('systemctl', ['is-active', 'vsift-batch@0043']).stdout.trim();
      return state === 'active' || state === 'activating' ? null : state;
    }, 20 * 60 * 1000, 3000);
    const text = host.run('sudo', ['-n', 'cat', `${STATE}/results/batch-0043.events.jsonl`]).stdout;
    const batch = events.readBatch(events.parseEvents(text).events);
    const recorded = [...batch.outcomes.values()].filter((outcome) => events.disposition(outcome) === 'recorded').length;
    return { ok: done !== null && recorded === 8, observed: `unit ${done}; ${recorded} of 8 lines recorded after the kill and redelivery` };
  });

  await step('7', '`session clean --expired --json` over the buckets, as the periodic job runs it', 'removes closed, expired and abandoned sessions; never touches the input root or the bundle root', () => {
    const before = host.run('sudo', ['-n', 'find', '/srv/vsift/inputs', '/srv/vsift/bundles', '-type', 'f', '-printf', '%p %s\n']).stdout;
    let pages = 0;
    let removed = 0;
    let cursor = null;
    for (let guard = 0; guard < 300; guard += 1) {
      const answered = asWorker(['--session-root', WORKSPACE, 'session', 'clean', '--expired', ...(cursor === null ? [] : ['--cursor', String(cursor)]), '--json']);
      const result = json(answered.stdout);
      pages += 1;
      if (!result || !result.data) return { ok: false, observed: `a page answered ${answered.code}: ${answered.stdout.slice(0, 160)}` };
      removed += result.data.items.filter((item) => item.outcome === 'removed').length;
      cursor = result.data.next_cursor === undefined ? null : result.data.next_cursor;
      if (cursor === null) break;
    }
    const after = host.run('sudo', ['-n', 'find', '/srv/vsift/inputs', '/srv/vsift/bundles', '-type', 'f', '-printf', '%p %s\n']).stdout;
    return { ok: before === after, observed: `${pages} bucket pages, ${removed} sessions removed; the input and bundle roots ${before === after ? 'are unchanged' : 'changed'}`, note: 'the runbook says to page with `--cursor` until `next_cursor` is null; the cursor is a bucket number 0 to 255, so a full pass is up to 256 calls' };
  });

  return finish(options, record, hostInfo, versionLine, image);
}

function finish(options, steps, hostInfo, versionLine, image) {
  const failed = steps.filter((entry) => !entry.ok);
  const lines = [`## P14 runbook walk (RQ-12): ${failed.length === 0 ? 'every step matched' : `${failed.length} step(s) DIVERGED`}`, '', `Published ${versionLine}; ${hostInfo.os}, kernel ${hostInfo.kernel}, ${hostInfo.cpus} CPUs; storage ${hostInfo.storage}.`, '', '| Section | Step | The runbook promises | What happened | |', '| --- | --- | --- | --- | --- |'];
  for (const entry of steps) {
    lines.push(`| ${entry.section} | ${entry.title.replace(/\|/g, '\\|')} | ${entry.expected.replace(/\|/g, '\\|')} | ${entry.observed.replace(/\|/g, '\\|').replace(/\n/g, ' ').slice(0, 400)} | ${entry.ok ? 'matched' : '**DIVERGED**'} |`);
  }
  lines.push('');
  const notes = steps.filter((entry) => entry.note);
  if (notes.length) {
    lines.push('### Notes for the runbook', '');
    for (const entry of notes) lines.push(`- Section ${entry.section}, "${entry.title.slice(0, 60)}": ${entry.note}`);
    lines.push('');
  }
  const text = lines.join('\n');
  fs.writeFileSync(path.join(options.out, 'runbook-walk.json'), `${JSON.stringify({ version: versionLine, host: hostInfo, image, steps }, null, 2)}\n`);
  fs.writeFileSync(path.join(options.out, 'runbook-summary.md'), text);
  process.stdout.write(`\n${text}\n`);
  process.exitCode = failed.length === 0 ? 0 : 1;
  return failed.length === 0;
}

process.on('exit', (code) => {
  if (code === 0 && !fs.existsSync(path.join(process.argv[process.argv.indexOf('--out') + 1], 'runbook-summary.md'))) {
    process.stderr.write('the walk ended without writing its summary\n');
    process.exitCode = 3;
  }
});

main().catch((error) => {
  process.stderr.write(`${error.stack || error.message}\n`);
  process.exitCode = 2;
});
