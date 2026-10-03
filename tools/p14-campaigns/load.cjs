#!/usr/bin/env node
'use strict';

// P14 RQ-09: the load ladder, a 100-request batch, a cancel that must leave no
// descendant, the warm candidate page of a prepared 30-minute session and a
// mixed soak, all through the PUBLISHED `vsift` in the hardened worker
// container of docs/operations/worker-host.md section 10, on a hosted
// Ubuntu 24.04 runner prepared by prepare-host.cjs.
//
//   node load.cjs --out <dir> [--phases ladder,batch,cancel,page,soak]
//        [--ladder-requests 24] [--batch-requests 100] [--soak-requests 1000]
//        [--soak-minutes 270] [--seed p14] [--cpus 4] [--memory 12g]
//
// Never run it on a machine whose state matters: it creates workspaces under
// /var/lib/vsift, runs sudo for the sampler and kills containers at random.

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const zlib = require('node:zlib');
const childProcess = require('node:child_process');
const host = require('./lib/host.cjs');
const container = require('./lib/container.cjs');
const requests = require('./lib/requests.cjs');
const events = require('./lib/events.cjs');
const cgroup = require('./lib/cgroup-sampler.cjs');
const verdict = require('./lib/load-verdict.cjs');

function parse(argv) {
  const options = {
    phases: 'diagnose,ladder,batch,cancel,page,soak',
    'ladder-requests': '24',
    'batch-requests': '100',
    'soak-requests': '1000',
    'soak-minutes': '270',
    'batch-size': '60',
    seed: 'p14',
    cpus: String(Math.min(os.cpus().length, 8)),
    memory: '12g',
    slots: '8',
  };
  for (let index = 0; index < argv.length; index += 2) options[argv[index].replace(/^--/, '')] = argv[index + 1];
  if (!options.out) throw new Error('--out is required');
  return options;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
/** What a recorded result says went wrong: the request failure and each step's status and failure. */
function failureDetail(result) {
  if (!result) return 'no result event';
  const steps = (result.steps || []).map((step) => `${step.kind} ${step.status}${step.failure ? ` ${step.failure.code}` : ''}`);
  return `failure ${JSON.stringify(result.failure)}; steps [${steps.join(', ')}]`;
}

const log = (message) => process.stdout.write(`${new Date().toISOString().slice(11, 19)} ${message}\n`);

class Load {
  constructor(options) {
    this.options = options;
    this.out = options.out;
    this.hostRecord = JSON.parse(fs.readFileSync(path.join(options.out, 'host.json'), 'utf8'));
    this.sentinel = `P14-SENTINEL-${requests.sha256(`${options.seed}:sentinel`).slice(0, 16)}`;
    this.counter = 0;
    this.slots = Number(options.slots);
    this.durability = 'durable';
    this.containers = []; // {name, id, phase}
    this.forbidden = [this.sentinel, this.hostRecord.canary, '/srv/vsift', '/var/lib/vsift', 'incoming/'];
    this.runDir = path.join(options.out, 'runs');
    fs.mkdirSync(this.runDir, { recursive: true });
  }

  /** One container of the worker image, hardened as the runbook says, to the end. */
  async worker(spec) {
    this.counter += 1;
    const name = `vsift-load-${spec.tag}-${this.counter}`;
    const args = container.runArguments({
      image: host.IMAGE,
      name,
      command: spec.command,
      network: 'none',
      cpus: Number(this.options.cpus),
      memory: this.options.memory,
      pids: 256,
      volumes: [`${host.STATE}:${host.STATE}:rw`, `${host.INPUTS}:${host.INPUTS}:ro`, `${host.BUNDLES}:${host.BUNDLES}:rw`, ...(spec.volumes || [])],
      tmpfs: spec.tmpfs || undefined,
      env: { P14_SENTINEL: this.sentinel },
    });
    let interrupted = null;
    let timer = null;
    if (spec.interrupt) {
      timer = setTimeout(() => {
        interrupted = { signal: spec.interrupt.signal, afterMs: spec.interrupt.afterMs };
        container.signal(name, spec.interrupt.signal);
      }, spec.interrupt.afterMs);
    }
    const result = await container.docker(args, { timeoutMs: spec.timeoutMs || 60 * 60 * 1000, killName: name, onStdout: spec.onStdout });
    if (timer) clearTimeout(timer);
    const state = container.state(name);
    container.remove(name);
    const id = state && state.id ? state.id.slice(0, 12) : null;
    this.containers.push({ name, id, phase: spec.phase || spec.tag, startedAt: Date.now() - result.durationMs, endedAt: Date.now() });
    return { ...result, name, id, oomKilled: Boolean(state && state.oomKilled), interrupted };
  }

  /** `vsift ... --json` in a container: the exit status and the parsed result. */
  async cli(args, tag, workspace, before = []) {
    const result = await this.worker({ tag, command: ['vsift', ...before, '--session-root', workspace, ...args, '--json'], phase: tag, timeoutMs: 20 * 60 * 1000 });
    let json = null;
    try {
      json = JSON.parse(result.stdout);
    } catch {
      json = null;
    }
    return { code: result.code, json, stdout: result.stdout, stderr: result.stderr };
  }

  async initWorkspace(name, slots) {
    const workspace = `${host.STATE}/ws-${name}`;
    const first = await this.cli(['session', 'init-workspace', '--durability', this.durability, '--admission-slots', String(slots), '--retention-hours', '24'], `init-${name}`, workspace);
    if (first.code === 0) return { workspace, durability: this.durability, note: null };
    if (this.durability === 'durable' && first.json && first.json.error && first.json.error.code === 'MISSING_CAPABILITY') {
      this.durability = 'ephemeral';
      const second = await this.cli(['session', 'init-workspace', '--durability', 'ephemeral', '--admission-slots', String(slots), '--retention-hours', '24'], `init-${name}`, workspace);
      if (second.code === 0) return { workspace, durability: 'ephemeral', note: `durable was refused (${JSON.stringify(first.json.error)}); the campaign ran on ephemeral workspaces` };
    }
    throw new Error(`session init-workspace failed (${first.code}): ${first.stdout.slice(0, 600)} ${first.stderr.slice(0, 400)}`);
  }

  writeQueue(name, text) {
    host.sudo(['sh', '-c', `cat > ${host.STATE}/queue/${name} && chown ${container.WORKER_USER} ${host.STATE}/queue/${name}`], { input: text });
  }

  /** One `job batch` run: the file, the workspace, the concurrency, perhaps an interruption. */
  async batch(spec) {
    this.writeQueue(spec.file, requests.batchFile(spec.lines));
    const command = [
      'vsift', '--host-isolation', 'strict-linux', '--session-root', spec.workspace, 'job', 'batch',
      '--requests', `${host.STATE}/queue/${spec.file}`, '--input-root', host.INPUTS, '--bundle-root', host.BUNDLES,
      '--concurrency', String(spec.concurrency), '--admission-wait-ms', '60000', '--drain-timeout-ms', '30000', '--events', 'jsonl',
    ];
    const result = await this.worker({ tag: spec.tag, phase: spec.phase, command, interrupt: spec.interrupt, onStdout: spec.onStdout, timeoutMs: spec.timeoutMs });
    const parsed = events.parseEvents(result.stdout);
    const batch = events.readBatch(parsed.events);
    const leaks = events.leaks(`${result.stdout}\n${result.stderr}`, this.forbidden);
    fs.writeFileSync(path.join(this.runDir, `${result.name}.events.jsonl`), result.stdout);
    if (result.stderr.trim()) fs.writeFileSync(path.join(this.runDir, `${result.name}.stderr.txt`), result.stderr);
    return { ...result, parsed, batch, leaks, problems: [...parsed.problems, ...batch.problems] };
  }

  /** What each planned line of a finished batch came to, against what it was expected to. */
  judge(lines, run, options = {}) {
    const verdicts = [];
    lines.forEach((planned, index) => {
      const number = index + 1;
      const outcome = run.batch.outcomes.get(number);
      const result = run.batch.results.get(number);
      const reading = events.disposition(outcome);
      const met = events.meets(planned, outcome, result);
      verdicts.push({ number, planned, outcome, result, reading, ...met });
    });
    return { verdicts, options };
  }

  /** The curve of one container run from the sampler's records. */
  curve(runs) {
    const records = this.sampled();
    const groups = cgroup.bySeries(records);
    return runs.map((run) => {
      const series = run.id ? groups.get(run.id) || [] : [];
      return { run: run.name, curve: series.length ? cgroup.summariseRun(series) : null, series };
    });
  }

  sampled() {
    try {
      return fs.readFileSync(path.join(this.out, 'samples.jsonl'), 'utf8').split('\n').filter(Boolean).map((line) => JSON.parse(line));
    } catch {
      return [];
    }
  }
}

function startSampler(out) {
  const samples = path.join(out, 'samples.jsonl');
  fs.writeFileSync(samples, '');
  const child = childProcess.spawn('sudo', ['-n', process.execPath, path.join(__dirname, 'sampler.cjs'), '--out', samples, '--interval-ms', '1000'], { stdio: 'ignore', detached: true });
  child.unref();
  return () => {
    childProcess.spawnSync('sudo', ['-n', 'pkill', '-TERM', '-f', 'sampler.cjs --out'], { stdio: 'ignore' });
  };
}

/**
 * Local speech recognition in the worker container, called directly: once as a
 * desktop command and once under strict isolation, so a refusal in the batches
 * below can be told from the tools being unusable here.
 */
async function diagnoseRecognition(load) {
  const workspace = `${host.STATE}/ws-diagnose`;
  const ingest = await load.cli(['ingest', `${host.INPUTS}/incoming/F01-speech.mp4`], 'diagnose-ingest', workspace);
  const session = ingest.json && ingest.json.data && ingest.json.data.session_id;
  const trials = [{ name: 'ingest', code: ingest.code, said: ingest.stdout.slice(0, 700) }];
  if (session) {
    for (const [label, before] of [['plain', []], ['strict-linux', ['--host-isolation', 'strict-linux']]]) {
      const answered = await load.cli(['transcript', 'retranscribe', session, '--from', '0', '--to', '5000000'], `diagnose-${label}`, workspace, before);
      trials.push({ name: `retranscribe (${label})`, code: answered.code, said: answered.stdout.slice(0, 1500), stderr: answered.stderr.slice(0, 500) });
    }
  }
  // The same recognition as a worker request: `job run` under strict isolation, once with the default /tmp and once with a larger one.
  const workerWorkspace = `${host.STATE}/ws-diagnose-worker`;
  const initialised = await load.cli(['session', 'init-workspace', '--durability', 'ephemeral', '--admission-slots', '8', '--retention-hours', '1'], 'diagnose-init', workerWorkspace);
  trials.push({ name: 'init-workspace (ephemeral)', code: initialised.code, said: initialised.stdout.slice(0, 300) });
  for (const [label, tmpfs] of [['default /tmp', null], ['/tmp of 1 GiB', '/tmp:rw,nosuid,size=1g']]) {
    const id = requests.operationId(`${load.options.seed}-diagnose-${label}`, 0);
    load.writeQueue('diagnose.json', JSON.stringify(requests.request({ id, source: 'incoming/F01-speech.mp4', steps: [requests.STEPS.retranscribe({ from_us: 0, to_us: 5_000_000 }), requests.STEPS.close()] })));
    const answered = await load.worker({
      tag: 'diagnose-job', phase: 'diagnose', tmpfs,
      command: ['vsift', '--host-isolation', 'strict-linux', '--session-root', workerWorkspace, 'job', 'run', '--request', `${host.STATE}/queue/diagnose.json`, '--input-root', host.INPUTS, '--bundle-root', host.BUNDLES, '--json'],
    });
    trials.push({ name: `job run retranscribe (${label})`, code: answered.code, said: answered.stdout.slice(0, 1800), stderr: answered.stderr.slice(0, 600) });
  }
  // Every speech-bearing corpus video, one by one: ingest, then recognise the first five seconds under strict isolation.
  for (const video of ['F01-speech.mp4', 'F02-speech.mp4', 'F03-speech.mp4', 'F04-speech.mp4', 'F05-speech.mp4', 'F06-speech.mp4', 'F07-speech.mp4', 'F08-speech.mp4', 'F09-speech.mkv', 'F12-speech.mp4']) {
    const sessionWorkspace = `${host.STATE}/ws-diagnose-${video.replace(/\W/g, '')}`;
    const ingested = await load.cli(['ingest', `${host.INPUTS}/incoming/${video}`], 'diagnose-each-ingest', sessionWorkspace);
    const sessionId = ingested.json && ingested.json.data && ingested.json.data.session_id;
    if (!sessionId) {
      trials.push({ name: `${video}: ingest`, code: ingested.code, said: ingested.stdout.slice(0, 500), stderr: ingested.stderr.slice(0, 300) });
      continue;
    }
    const answered = await load.cli(['transcript', 'retranscribe', sessionId, '--from', '0', '--to', '5000000'], 'diagnose-each', sessionWorkspace, ['--host-isolation', 'strict-linux']);
    const detail = answered.json && answered.json.error ? JSON.stringify(answered.json.error) : (answered.json && answered.json.data ? `${answered.json.data.recognised_segment_count} segment(s)` : '');
    trials.push({ name: `${video}: retranscribe 0-5 s`, code: answered.code, said: answered.stdout.slice(0, 900), stderr: answered.stderr.slice(0, 900), detail });
    if (answered.code !== 0) {
      // Does the whole clip recognise where its first five seconds did not? (Plain wording on purpose: not a gating trial.)
      const whole = await load.cli(['transcript', 'retranscribe', sessionId], 'diagnose-each-whole', sessionWorkspace, ['--host-isolation', 'strict-linux']);
      const wholeDetail = whole.json && whole.json.error ? JSON.stringify(whole.json.error.remediation && whole.json.error.remediation[0] ? whole.json.error.remediation[0].summary : whole.json.error.code).slice(0, 260) : (whole.json && whole.json.data ? `${whole.json.data.recognised_segment_count} segment(s)` : '');
      trials.push({ name: `${video}: retranscribe the whole clip`, code: whole.code, said: whole.stdout.slice(0, 900), stderr: whole.stderr.slice(0, 900), detail: wholeDetail });
    }
  }
  fs.writeFileSync(path.join(load.out, 'diagnose-recognition.json'), `${JSON.stringify(trials, null, 2)}\n`);
  return {
    name: 'Local speech recognition in the worker container (diagnosis)',
    measurements: trials.map((trial) => `- ${trial.name}: exit ${trial.code}. ${trial.detail || trial.said.replace(/\s+/g, ' ').slice(0, 420)}`),
    // Only the path itself gates the load: a clip that will not recognise is a finding about recognition, filed on its own and listed above, not a measure of load.
    checks: trials.filter((trial) => !/^F\d\d-speech/.test(trial.name)).map((trial) => verdict.check(`${trial.name} succeeds`, trial.code === 0, `exit ${trial.code}`)),
    findings: trials.filter((trial) => /^F\d\d-speech/.test(trial.name) && trial.code !== 0).map((trial) => trial.name),
  };
}

/** The ladder: the same requests at 1, 2, 4 and 8 admitted jobs. */
async function ladder(load) {
  const rungs = [];
  const checks = [];
  const lines = requests.ladderRequests(`${load.options.seed}-ladder`, Number(load.options['ladder-requests']), load.durability);
  for (const concurrency of [1, 2, 4, 8]) {
    const init = await load.initWorkspace(`ladder-${concurrency}`, load.slots);
    const rungLines = requests.ladderRequests(`${load.options.seed}-ladder`, lines.length, init.durability);
    log(`ladder: ${rungLines.length} requests at concurrency ${concurrency}`);
    const run = await load.batch({ tag: `ladder${concurrency}`, phase: 'ladder', file: `ladder-${concurrency}.jsonl`, workspace: init.workspace, lines: rungLines, concurrency });
    const judged = load.judge(rungLines, run);
    const bad = judged.verdicts.filter((entry) => !entry.ok);
    const label = `rung ${concurrency}`;
    const seconds = run.durationMs / 1000;
    rungs.push({
      concurrency, requests: rungLines.length, seconds: Math.round(seconds * 10) / 10, perMinute: Math.round((rungLines.length / seconds) * 600) / 10,
      exit: run.code, oomKilled: run.oomKilled, durability: init.durability, note: init.note, peakRunning: run.batch.peakRunning,
    });
    if (init.note) log(`ladder: ${init.note}`);
    checks.push(verdict.check(`${label}: every request recorded as asked`, bad.length === 0 && run.code === 0 && !run.oomKilled, bad.length ? `${bad.length} line(s): ${bad.slice(0, 3).map((entry) => `${entry.number} ${entry.why}; ${failureDetail(entry.result)}`).join('; ')}` : `${rungLines.length} of ${rungLines.length}, exit ${run.code}`));
    checks.push(...verdict.streamChecks(label, { problems: run.problems, leaks: run.leaks, batch: run.batch, expectedConcurrency: concurrency, expectedCapacity: load.slots }));
  }
  return { name: 'The load ladder (1, 2, 4 and 8 admitted jobs)', rungs, checks, runs: load.containers.filter((entry) => entry.phase === 'ladder') };
}

/** One batch of 100 requests at concurrency 4, then every retained bundle and session validated. */
async function batch100(load) {
  const init = await load.initWorkspace('batch100', load.slots);
  const mix = requests.soakMix(`${load.options.seed}-batch`, 400, init.durability).filter((line) => ['ingest_candidates', 'ingest_candidates_retain', 'supplied_transcript', 'recognise'].includes(line.kind)).slice(0, Number(load.options['batch-requests']));
  log(`batch: ${mix.length} requests at concurrency 4`);
  const run = await load.batch({ tag: 'batch100', phase: 'batch100', file: 'batch100.jsonl', workspace: init.workspace, lines: mix, concurrency: 4 });
  const judged = load.judge(mix, run);
  const bad = judged.verdicts.filter((entry) => !entry.ok);
  const checks = [
    verdict.check('the batch: every request recorded as asked', bad.length === 0 && run.code === 0 && !run.oomKilled, bad.length ? `${bad.length} line(s): ${bad.slice(0, 3).map((entry) => `${entry.number} ${entry.why}; ${failureDetail(entry.result)}`).join('; ')}` : `${mix.length} of ${mix.length}, exit ${run.code}`),
    ...verdict.streamChecks('the batch', { problems: run.problems, leaks: run.leaks, batch: run.batch, expectedConcurrency: 4, expectedCapacity: load.slots }),
  ];
  const validation = await validateEverything(load, init.workspace, 'batch100');
  checks.push(...validation.checks);
  return { name: 'A 100-request batch', requests: mix.length, seconds: Math.round(run.durationMs / 100) / 10, checks, validation: validation.summary, runs: [run] };
}

/**
 * Follows a paged command (`session list`, `session clean --expired`) through its hash buckets:
 * one page at a time by `--cursor`, until `next_cursor` is null.
 */
async function scan(load, workspace, args, tag, firstCursor = null, onlyOnePage = false) {
  const items = [];
  const failures = [];
  let pages = 0;
  let cursor = firstCursor;
  for (let guard = 0; guard < 300; guard += 1) {
    const result = await load.cli([...args, ...(cursor === null ? [] : ['--cursor', String(cursor)])], tag, workspace);
    pages += 1;
    if (!result.json || !result.json.data) {
      failures.push(`${args.join(' ')} page ${pages}: exit ${result.code}`);
      break;
    }
    if (result.code !== 0) failures.push(`${args.join(' ')} page ${pages}: exit ${result.code}`);
    items.push(...(result.json.data.items || []));
    cursor = result.json.data.next_cursor === undefined ? null : result.json.data.next_cursor;
    if (cursor === null || onlyOnePage) break;
  }
  return { items, failures, pages };
}

/** Lists the retained bundles and the sessions of a workspace and validates every one. */
async function validateEverything(load, workspace, tag, { killed = false } = {}) {
  const bundles = host.run('sudo', ['-n', 'find', host.BUNDLES, '-mindepth', '1', '-maxdepth', '1', '-type', 'd', '!', '-name', '.*', '!', '-name', 'lost+found']).stdout.split('\n').filter(Boolean);
  const failures = [];
  let validated = 0;
  const pool = [...bundles];
  const work = async () => {
    for (let next = pool.pop(); next; next = pool.pop()) {
      const result = await load.cli(['bundle', 'validate', next], `bundle-${tag}`, workspace);
      if (result.code === 0) validated += 1;
      else failures.push(`${path.basename(next)}: exit ${result.code} ${result.json && result.json.error ? result.json.error.code : ''}`);
    }
  };
  await Promise.all([work(), work(), work(), work()]);
  const listed = await scan(load, workspace, ['session', 'list'], `list-${tag}`);
  failures.push(...listed.failures);
  // A session still `initializing` has an index entry and no first generation: what a kill between the two leaves.
  const initializing = listed.items.filter((item) => item.state === 'initializing').map((item) => item.session_id);
  const sessions = listed.items.filter((item) => item.state !== 'initializing').map((item) => item.session_id);
  const initializingStatus = initializing.length ? await load.cli(['session', 'status', initializing[0]], `status-initializing-${tag}`, workspace) : null;
  let statusOk = 0;
  const queue = [...sessions];
  const readStatus = async () => {
    for (let next = queue.pop(); next; next = queue.pop()) {
      const result = await load.cli(['session', 'status', next], `status-${tag}`, workspace);
      if (result.code === 0) statusOk += 1;
      else {
        // What was on disk for the session that would not read back: names, kinds, sizes and owners only.
        const onDisk = host.run('sudo', ['-n', 'find', workspace, '-maxdepth', '5', '-path', `*${next}*`, '-printf', '%P %y %s %u:%g %m\\n']).stdout.split('\n').filter(Boolean).slice(0, 24);
        const listedItem = listed.items.find((item) => item.session_id === next);
        failures.push(`session ${next}: exit ${result.code} ${result.json && result.json.error ? result.json.error.code : ''} ${(result.stdout || result.stderr).replace(/\s+/g, ' ').slice(0, 200)} | listed as ${JSON.stringify(listedItem).slice(0, 300)} | on disk: ${onDisk.join('; ')}`);
      }
    }
  };
  await Promise.all([readStatus(), readStatus(), readStatus(), readStatus()]);
  return {
    summary: {
      bundles: bundles.length, bundlesValidated: validated, sessions: sessions.length, sessionsRead: statusOk, initializing: initializing.length,
      initializingStatus: initializingStatus ? { exit: initializingStatus.code, code: initializingStatus.json && initializingStatus.json.error ? initializingStatus.json.error.code : null } : null,
      failures: failures.slice(0, 20),
    },
    checks: [
      verdict.check('every retained bundle validates', failures.filter((entry) => !/^session/.test(entry)).length === 0 && validated === bundles.length, `${validated} of ${bundles.length} bundles`),
      verdict.check(killed ? 'sessions left initializing by the kills are counted, not failed' : 'no session is left initializing (nothing was killed)', killed || initializing.length === 0, `${initializing.length} initializing${initializingStatus ? `; session status of one answered exit ${initializingStatus.code} ${initializingStatus.json && initializingStatus.json.error ? initializingStatus.json.error.code : ''}` : ''}`),
      verdict.check('every committed session reads back', statusOk === sessions.length && !failures.some((entry) => /^session/.test(entry)), `${statusOk} of ${sessions.length} sessions${failures.filter((entry) => /^session/.test(entry)).length ? `: ${failures.filter((entry) => /^session/.test(entry)).slice(0, 3).join('; ')}` : ''}`),
    ],
  };
}

/** Cancels one running recognition with `job cancel` and requires that nothing it started outlives the cancel by 10 s. */
async function cancel(load) {
  const init = await load.initWorkspace('cancel', load.slots);
  await prepareLongVideo(load);
  const id = requests.operationId(`${load.options.seed}-cancel`, 0);
  const line = { text: JSON.stringify(requests.request({ id, source: 'incoming/long30.mp4', durability: init.durability, steps: [requests.STEPS.retranscribe(), requests.STEPS.close()] })), expect: 'recorded' };
  let jobId = null;
  let progressAt = null;
  let finishedAt = null;
  let cancelAt = null;
  let cancelResult = null;
  const onStdout = (text) => {
    if (/"event":"result"/.test(text) && finishedAt === null) finishedAt = Date.now();
    if (jobId) return;
    for (const piece of text.split('\n')) {
      const found = /"job_id":"(job_[0-9a-f]+)"/.exec(piece);
      if (found && /"event":"progress"/.test(piece)) {
        jobId = found[1];
        progressAt = Date.now();
      }
    }
  };
  log('cancel: starting a whole-video recognition of a 30-minute source');
  const running = load.batch({ tag: 'cancel', phase: 'cancel', file: 'cancel.jsonl', workspace: init.workspace, lines: [line], concurrency: 1, onStdout, timeoutMs: 30 * 60 * 1000 });
  for (let waited = 0; waited < 600 && !jobId; waited += 1) await sleep(1000);
  const name = `vsift-load-cancel-${load.counter}`;
  if (jobId) {
    await sleep(3000);
    cancelAt = Date.now();
    const result = childProcess.spawnSync('docker', [
      'exec', '--user', container.WORKER_USER, '--env', `HOME=${container.HOME}`, '--env', `XDG_CONFIG_HOME=${container.HOME}/.config`, name,
      'vsift', '--session-root', init.workspace, 'job', 'cancel', jobId, '--json',
    ], { encoding: 'utf8' });
    cancelResult = { code: result.status, stdout: (result.stdout || '').slice(0, 600), stderr: (result.stderr || '').slice(0, 400) };
  }
  const run = await running;
  const owner = load.containers.find((entry) => entry.name === run.name);
  const series = owner && owner.id ? (cgroup.bySeries(load.sampled()).get(owner.id) || []) : [];
  const lingering = cancelAt ? cgroup.lingeringDescendants(series, cancelAt, verdict.THRESHOLDS.descendantGraceMs) : [];
  const outcome = run.batch.outcomes.get(1);
  const checks = [
    verdict.check('a recognition was running when it was cancelled', Boolean(jobId && cancelAt) && (finishedAt === null || finishedAt > cancelAt), jobId ? `job ${jobId}, progress seen ${progressAt ? Math.round((cancelAt - progressAt) / 1000) : '?'} s before the cancel${finishedAt !== null && finishedAt <= cancelAt ? `; the request had already ended by itself ${Math.round((cancelAt - finishedAt) / 1000)} s before (${run.batch.outcomes.get(1) ? `${run.batch.outcomes.get(1).status} ${run.batch.outcomes.get(1).code || ''}` : 'no outcome'})` : ''}` : 'no progress event with a job id was seen'),
    verdict.check('job cancel answered', Boolean(cancelResult) && cancelResult.code === 0, cancelResult ? `exit ${cancelResult.code} ${cancelResult.stdout.slice(0, 160)} ${cancelResult.stderr.slice(0, 200)}` : 'not run'),
    verdict.check('the request ended cancelled, not finished', Boolean(outcome) && outcome.status !== 'complete', outcome ? `${outcome.status} ${outcome.code || ''}` : 'no outcome (the batch ended without one)'),
    verdict.check(`no descendant of the coordinator ${verdict.THRESHOLDS.descendantGraceMs / 1000} s after the cancel`, cancelAt !== null && series.length > 0 && lingering.length === 0, series.length === 0 ? 'the sampler recorded nothing' : `${lingering.length} later sample(s) with a descendant${lingering.length ? ` (${lingering[0].scope.descendants.map((entry) => entry.comm).join(', ')})` : ''}`),
    verdict.check('the container did not run out of memory', !run.oomKilled, `exit ${run.code}`),
  ];
  return { name: 'A cancel leaves nothing running', checks, jobId, cancelResult };
}

/** A 30-minute source: two speech clips of one resolution (F01, F06) alternated 113 times by the concat demuxer of FFmpeg itself, copied not re-encoded. */
async function prepareLongVideo(load) {
  if (fs.existsSync(`${host.INPUTS}/incoming/long30.mp4`)) return;
  const finder = host.run('sudo', ['-n', 'find', `${host.STATE}/home`, '-type', 'f', '-name', 'ffmpeg', '-perm', '-u+x']);
  const ffmpeg = finder.stdout.split('\n').find(Boolean);
  if (!ffmpeg) throw new Error('the managed FFmpeg was not found under the worker home');
  const entries = [];
  for (let pair = 0; pair < 113; pair += 1) entries.push(`file '${host.INPUTS}/incoming/F01-speech.mp4'`, `file '${host.INPUTS}/incoming/F06-speech.mp4'`);
  fs.writeFileSync('/tmp/long30.txt', `${entries.join('\n')}\n`);
  const args = container.runArguments({
    image: host.IMAGE, name: `vsift-load-longvideo-${load.counter += 1}`, network: 'none', cpus: 2, memory: '4g', remove: true,
    volumes: [`${host.STATE}:${host.STATE}:ro`, `${host.INPUTS}:${host.INPUTS}:ro`, '/tmp:/hosttmp:rw'],
    command: [ffmpeg, '-nostdin', '-v', 'error', '-f', 'concat', '-safe', '0', '-i', '/hosttmp/long30.txt', '-c', 'copy', '-f', 'mp4', '/hosttmp/long30.mp4'],
  });
  const result = host.run('docker', args, { timeoutMs: 10 * 60 * 1000 });
  if (result.code !== 0) throw new Error(`could not build the 30-minute source: ${result.stderr.slice(0, 400)}`);
  host.sudo(['cp', '/tmp/long30.mp4', `${host.INPUTS}/incoming/long30.mp4`]);
  host.sudo(['chmod', '0644', `${host.INPUTS}/incoming/long30.mp4`]);
}

/** The warm candidate page of a prepared 30-minute session. */
async function page(load) {
  const init = await load.initWorkspace('page', load.slots);
  await prepareLongVideo(load);
  const id = requests.operationId(`${load.options.seed}-page`, 0);
  const prepare = JSON.stringify(requests.request({ id, source: 'incoming/long30.mp4', durability: init.durability, steps: [requests.STEPS.candidates()] }));
  load.writeQueue('page.json', prepare);
  const prepared = await load.worker({ tag: 'page-prepare', phase: 'page', timeoutMs: 40 * 60 * 1000, command: [
    'vsift', '--session-root', init.workspace, 'job', 'run', '--request', `${host.STATE}/queue/page.json`, '--input-root', host.INPUTS, '--bundle-root', host.BUNDLES, '--json',
  ] });
  let session = null;
  try {
    session = JSON.parse(prepared.stdout).data.session_id;
  } catch {
    session = null;
  }
  if (!session) {
    return { name: 'The warm candidate page', checks: [verdict.check('a prepared 30-minute session', false, `job run exit ${prepared.code}: ${prepared.stdout.slice(0, 300)}`)] };
  }
  const script = `i=0; while [ $i -lt 200 ]; do s=$(date +%s%N); out=$(vsift --session-root ${init.workspace} candidates ${session} --from 0 --to 1800000000 --limit 20 --json 2>&1); rc=$?; e=$(date +%s%N); echo "$(( (e - s) / 1000000 )) $rc \${#out}"; if [ $i -eq 0 ]; then echo "FIRST: $out" | head -c 700 >&2; fi; i=$((i+1)); done`;
  const timed = await load.worker({ tag: 'page-timing', phase: 'page', command: ['sh', '-c', script], timeoutMs: 20 * 60 * 1000 });
  const calls = timed.stdout.split('\n').map((entry) => /^(\d+) (\d+) (\d+)$/.exec(entry)).filter(Boolean).map((match) => ({ ms: Number(match[1]), rc: Number(match[2]), bytes: Number(match[3]) }));
  const failed = calls.filter((call) => call.rc !== 0 || call.bytes < 200);
  const millis = calls.map((call) => call.ms);
  const p50 = cgroup.percentile(millis, 50);
  const p95 = cgroup.percentile(millis, 95);
  const p99 = cgroup.percentile(millis, 99);
  fs.writeFileSync(path.join(load.out, 'page-latencies-ms.txt'), `${millis.join('\n')}\n`);
  return {
    name: 'The warm candidate page of a prepared 30-minute session',
    measurements: [`${millis.length} calls: p50 ${p50} ms, p95 ${p95} ms, p99 ${p99} ms, max ${Math.max(0, ...millis)} ms. Prepared in ${Math.round(prepared.durationMs / 1000)} s (the first page analyses the 30 minutes).`],
    checks: [verdict.check('every timed call returned a page', calls.length === 200 && failed.length === 0, failed.length ? `${failed.length} of ${calls.length} calls failed or were empty; first call said: ${timed.stderr.slice(0, 500)}` : `${calls.length} calls, exit 0, at least 200 bytes each`), verdict.pageCheck(millis, p95)],
    latency: { calls: millis.length, p50, p95, p99 },
  };
}

/** The soak: lines of the mix delivered in batches, interrupted and redelivered until each is recorded as asked. */
async function soak(load) {
  const total = Number(load.options['soak-requests']);
  const deadline = Date.now() + Number(load.options['soak-minutes']) * 60 * 1000;
  const init = await load.initWorkspace('soak', load.slots);
  const lines = requests.soakMix(`${load.options.seed}-soak`, total, init.durability);
  const random = new requests.Random(`${load.options.seed}-interrupts`);
  const state = lines.map(() => ({ settled: false, violated: null, attempts: 0, result: null }));
  const counts = { rounds: 0, term: 0, kill: 0, redelivered: 0, followups: 0, cleans: 0, outsideWindow: 0 };
  // Sessions the operator's cleaner has removed: a request that names one of them is outside the dedupe window
  // (its record may be pruned once the table is full, #286), so a late duplicate or conflict is judged leniently.
  const removedSessions = new Set();
  const noteRemoved = (cleaned) => {
    for (const item of cleaned.items) if (item.outcome === 'removed' && item.session_id) removedSessions.add(item.session_id);
  };
  const problems = [];
  const violations = [];
  const leaks = [];
  const roundSummaries = [];
  const all = [];
  let busy = 0;
  const batchSize = Number(load.options['batch-size']);
  const concurrency = 4;

  const settled = (index) => state[index].settled || state[index].violated;
  while (Date.now() < deadline) {
    const pending = [];
    const idsInBatch = new Set();
    for (let index = 0; index < lines.length && pending.length < batchSize; index += 1) {
      if (settled(index)) continue;
      const line = lines[index];
      if (line.kind === 'duplicate' || line.kind === 'conflict') {
        const original = lines.findIndex((other) => other.id === line.of && other.kind !== 'duplicate' && other.kind !== 'conflict');
        if (original >= 0 && state[original].violated) {
          state[index].violated = `its original (line ${original}) did not settle, so this ${line.kind} cannot be judged`;
          continue;
        }
        if (original < 0 || !state[original].settled || idsInBatch.has(line.id)) continue;
      }
      if (line.id && idsInBatch.has(line.id)) continue;
      if (line.id) idsInBatch.add(line.id);
      pending.push(index);
    }
    if (pending.length === 0) break;
    counts.rounds += 1;
    const chosen = pending.map((index) => lines[index]);
    const roll = random.next();
    const interrupt = counts.rounds > 1 && roll < 0.5 ? { signal: roll < 0.25 ? 'TERM' : 'KILL', afterMs: 3000 + random.int(20_000) } : null;
    log(`soak round ${counts.rounds}: ${chosen.length} lines${interrupt ? `, ${interrupt.signal} after ${interrupt.afterMs} ms` : ''}; ${state.filter((entry) => entry.settled || entry.violated).length} of ${lines.length} settled`);
    const run = await load.batch({ tag: `soak${counts.rounds}`, phase: 'soak', file: `soak-${counts.rounds}.jsonl`, workspace: init.workspace, lines: chosen, concurrency, interrupt, timeoutMs: 30 * 60 * 1000 });
    all.push(run);
    if (run.interrupted) counts[run.interrupted.signal === 'TERM' ? 'term' : 'kill'] += 1;
    for (const found of run.leaks) leaks.push(`round ${counts.rounds}: ${found}`);
    // A killed stream may end without its closing events; any other break of the rules is a finding.
    for (const problem of run.problems) {
      const expectedOfKill = run.interrupted && run.interrupted.signal === 'KILL' && /not JSON|not lifecycle started/.test(problem);
      if (!expectedOfKill) problems.push(`round ${counts.rounds}: ${problem}`);
    }
    if (run.interrupted && run.interrupted.signal === 'TERM' && !run.batch.ended && run.durationMs > run.interrupted.afterMs + 1000) problems.push(`round ${counts.rounds}: a SIGTERM drain ended with no terminal event (exit ${run.code})`);
    if (run.oomKilled) problems.push(`round ${counts.rounds}: the container ran out of memory`);
    chosen.forEach((planned, offset) => {
      const index = pending[offset];
      const number = offset + 1;
      const outcome = run.batch.outcomes.get(number);
      const result = run.batch.results.get(number);
      state[index].attempts += 1;
      const reading = events.disposition(outcome);
      if (reading === 'redeliver') {
        if (state[index].attempts > 1) counts.redelivered += 1;
        if (outcome && outcome.code === 'BUSY') busy += 1;
        return;
      }
      let met = events.meets(planned, outcome, result);
      if (!met.ok && (planned.kind === 'duplicate' || planned.kind === 'conflict') && events.outsideWindow(planned, outcome, result)) {
        // Judged leniently only when the original's session has been removed by the cleaner (#286).
        const original = lines.findIndex((other) => other.id === planned.of && other.kind !== 'duplicate' && other.kind !== 'conflict');
        const session = original >= 0 && state[original].result ? state[original].result.session_id : null;
        if (session && removedSessions.has(session)) {
          counts.outsideWindow += 1;
          met = { ok: true, why: '' };
        }
      }
      if (met.ok) {
        state[index].settled = true;
        state[index].result = result || null;
      } else {
        state[index].violated = met.why;
        violations.push(`line ${index} (${planned.kind}): ${met.why}; ${failureDetail(result)}`);
      }
    });
    roundSummaries.push({ round: counts.rounds, lines: chosen.length, seconds: Math.round(run.durationMs / 100) / 10, exit: run.code, interrupted: run.interrupted ? run.interrupted.signal : null, ended: run.batch.ended, peakRunning: run.batch.peakRunning });
    if (counts.rounds % 4 === 0) {
      const cleaned = await scan(load, init.workspace, ['session', 'clean', '--expired'], 'clean', random.int(256), true);
      noteRemoved(cleaned);
      counts.cleans += 1;
      for (const failure of cleaned.failures) problems.push(failure);
    }
    // The operator's periodic job: a whole pass over every bucket now and then. A workspace keeps at most 4,096 request
    // records and prunes only those of sessions that no longer exist (worker-host.md, section 5), so a long soak that
    // never cleaned everything meets that documented RESOURCE_LIMIT (the first 12,000-request run did, at line 5,881).
    if (counts.rounds % 30 === 0) {
      const cleaned = await scan(load, init.workspace, ['session', 'clean', '--expired'], 'clean-pass');
      noteRemoved(cleaned);
      counts.cleans += 1;
      for (const failure of cleaned.failures) problems.push(failure);
    }
    // Evidence calls on the sessions that were left open, then close them.
    const open = chosen.map((planned, offset) => ({ planned, index: pending[offset] })).filter((entry) => entry.planned.kind === 'ingest_open' && state[entry.index].settled && state[entry.index].result && !state[entry.index].followed);
    for (const entry of open) {
      state[entry.index].followed = true;
      const session = state[entry.index].result.session_id;
      for (const args of [['frame', 'get', session, '--at', '1500000'], ['frame', 'burst', session, '--from', '0', '--to', '4000000', '--max-frames', '4'], ['session', 'close', session]]) {
        const answered = await load.cli(args, 'followup', init.workspace);
        counts.followups += 1;
        if (answered.code !== 0) problems.push(`${args.slice(0, 2).join(' ')} on an open session answered ${answered.code}${answered.json && answered.json.error ? ` ${answered.json.error.code}` : ''}`);
      }
    }
  }
  const unsettled = state.map((entry, index) => ({ entry, index })).filter(({ entry }) => !entry.settled && !entry.violated);
  const violated = state.filter((entry) => entry.violated).length;
  const byKind = {};
  lines.forEach((line, index) => {
    byKind[line.kind] = byKind[line.kind] || { lines: 0, settled: 0 };
    byKind[line.kind].lines += 1;
    if (state[index].settled) byKind[line.kind].settled += 1;
  });
  // Every committed session and bundle is read back before the final cleaning,
  // then the cleaner runs over every bucket.
  const validation = await validateEverything(load, init.workspace, 'soak', { killed: true });
  const before = host.run('sudo', ['-n', 'du', '-sk', init.workspace]).stdout.split('\t')[0];
  const finalClean = await scan(load, init.workspace, ['session', 'clean', '--expired'], 'clean');
  const removed = finalClean.items.filter((item) => item.outcome === 'removed').length;
  const remaining = await scan(load, init.workspace, ['session', 'list'], 'list-after');
  const disk = host.run('sudo', ['-n', 'du', '-sk', init.workspace]).stdout.split('\t')[0];
  const leftovers = host.run('sudo', ['-n', 'find', host.BUNDLES, '-mindepth', '1', '-maxdepth', '1', '-name', '.*.retaining']).stdout.split('\n').filter(Boolean).length;
  const curves = load.curve(all);
  const worst = Math.max(0, ...curves.map((entry) => (entry.curve && entry.curve.coordinatorRssPeakMiB) || 0));
  const checks = [
    verdict.check('every line of the mix came to what it was asked to', unsettled.length === 0 && violated === 0, `${state.filter((entry) => entry.settled).length} of ${lines.length} settled; ${violated} violated; ${unsettled.length} unsettled${unsettled.length ? ` (${Date.now() >= deadline ? 'the time budget ended the soak' : 'none could be delivered'})` : ''}${violations.length ? `. First violations: ${violations.slice(0, 3).join(' | ')}` : ''}`),
    verdict.check('the stream kept its rules in every round', problems.length === 0, problems.slice(0, 4).join('; ') || 'ordered, bounded; every drain ended with its terminal event'),
    verdict.check('no path and no sentinel in any output of any round', leaks.length === 0, leaks.length ? leaks.slice(0, 3).join('; ') : 'none found'),
    verdict.check('coordinator memory at most 256 MiB in every round', worst > 0 && worst <= verdict.THRESHOLDS.coordinatorRssMiB, `the largest peak of ${curves.length} runs was ${worst} MiB`),
    ...curves.filter((entry) => entry.curve && entry.curve.seconds >= 60).slice(0, 3).flatMap((entry) => verdict.resourceChecks(entry.run, entry.curve).slice(1)),
    ...validation.checks,
    verdict.check('the interruptions were made', counts.term + counts.kill > 0 || total < 200, `${counts.term} SIGTERM drains and ${counts.kill} SIGKILLs over ${counts.rounds} rounds; ${counts.redelivered} redeliveries; ${counts.followups} evidence calls on open sessions`),
  ];
  const measurements = [
    `${lines.length} lines in ${counts.rounds} rounds at concurrency ${concurrency}: ${counts.term} SIGTERM drains, ${counts.kill} SIGKILLs, ${counts.redelivered} redeliveries, ${busy} BUSY answers, ${counts.followups} evidence calls on open sessions, ${counts.cleans} cleaner passes${counts.outsideWindow > 0 ? `; ${counts.outsideWindow} duplicates or conflicts named a request whose session the cleaner had removed, and ran again (outside the dedupe window, #286)` : ''}.`,
    `Before the final clean the workspace held ${before} KiB and ${validation.summary.sessions} sessions; the cleaner (${finalClean.pages} bucket pages) removed ${removed}, leaving ${remaining.items.length} session(s) and ${disk} KiB. The bundle root holds ${validation.summary.bundles} bundles and ${leftovers} staging folder(s) a killed retain left (L-064).`,
  ];
  fs.writeFileSync(path.join(load.out, 'soak-rounds.json'), `${JSON.stringify(roundSummaries, null, 2)}\n`);
  fs.writeFileSync(path.join(load.out, 'soak-events.jsonl.gz'), zlib.gzipSync(all.map((run) => run.stdout).join('')));
  return { name: `The mixed soak (${total} requests)`, measurements, checks, counts, byKind, validation: validation.summary, violations: state.map((entry, index) => ({ entry, index })).filter(({ entry }) => entry.violated).slice(0, 30).map(({ entry, index }) => ({ line: index, kind: lines[index].kind, why: entry.violated })) };
}

async function main() {
  const options = parse(process.argv.slice(2));
  const load = new Load(options);
  const stopSampler = startSampler(options.out);
  await sleep(2000);
  const wanted = new Set(options.phases.split(','));
  const phases = [];
  try {
    if (wanted.has('diagnose')) phases.push(await diagnoseRecognition(load));
    if (wanted.has('ladder')) phases.push(await ladder(load));
    if (wanted.has('batch')) phases.push(await batch100(load));
    if (wanted.has('cancel')) phases.push(await cancel(load));
    if (wanted.has('page')) phases.push(await page(load));
    if (wanted.has('soak')) phases.push(await soak(load));
  } finally {
    await sleep(3000);
    stopSampler();
  }
  // Resource curves of the ladder and the batch, from the sampler's file.
  for (const phase of phases) {
    if (phase.name.startsWith('The load ladder') || phase.name.startsWith('A 100-request')) {
      const runs = load.containers.filter((entry) => entry.phase === (phase.name.startsWith('A 100') ? 'batch100' : 'ladder'));
      const curves = load.curve(runs);
      phase.curves = curves.map((entry) => ({ run: entry.run, ...entry.curve }));
      for (const entry of curves) phase.checks.push(...verdict.resourceChecks(entry.run, entry.curve));
      if (phase.rungs) {
        phase.measurements = ['| Concurrency | Requests | Seconds | Requests per minute | Coordinator peak MiB | Container peak MiB | Peak processes |', '| --- | --- | --- | --- | --- | --- | --- |', ...phase.rungs.map((rung, index) => {
          const curve = (curves[index] || {}).curve || {};
          return `| ${rung.concurrency} | ${rung.requests} | ${rung.seconds} | ${rung.perMinute} | ${curve.coordinatorRssPeakMiB} | ${curve.containerMemoryPeakMiB} | ${curve.pidsPeak} |`;
        }), '', `Throughput is a measurement of this ${options.cpus}-CPU runner, never a promise.`];
      }
    }
  }
  const markdown = verdict.renderMarkdown('P14 load campaign (RQ-09)', phases);
  const result = { host: load.hostRecord.host, version: load.hostRecord.versionLine, image: load.hostRecord.image, options, phases: phases.map(({ runs, ...rest }) => rest), ok: verdict.allHold(phases) };
  fs.writeFileSync(path.join(options.out, 'load-result.json'), `${JSON.stringify(result, null, 2)}\n`);
  fs.writeFileSync(path.join(options.out, 'load-summary.md'), markdown);
  process.stdout.write(`\n${markdown}\n`);
  process.exitCode = result.ok ? 0 : 1;
}

main().catch((error) => {
  process.stderr.write(`${error.stack || error.message}\n`);
  process.exitCode = 2;
});
