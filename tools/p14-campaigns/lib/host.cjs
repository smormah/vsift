'use strict';

// Preparing a hosted Ubuntu 24.04 runner as a worker host for the P14
// campaigns: the published `vsift` from the real registry, the worker
// image, the folders of docs/operations/worker-host.md section 1, the media
// tools installed by the published binary itself, and the corpus videos as
// the operator's input root. Every command is an explicit executable and
// argument list; `sudo` is used for the folders and the files that belong to
// the worker account, which a hosted runner allows without a password.
//
// Not for a developer's machine: it creates folders under /var/lib and /srv,
// builds an image and installs packages inside it.

const childProcess = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const container = require('./container.cjs');

const STATE = '/var/lib/vsift';
const INPUTS = '/srv/vsift/inputs';
const BUNDLES = '/srv/vsift/bundles';
const IMAGE = 'vsift-worker:p14';

/** Runs a program to the end, never through a shell. */
function run(file, args, options = {}) {
  const result = childProcess.spawnSync(file, args, {
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    timeout: options.timeoutMs,
    cwd: options.cwd,
    env: options.env || process.env,
    input: options.input,
  });
  return { code: result.status, stdout: result.stdout || '', stderr: result.stderr || '', error: result.error || null };
}

/** Like `run`, but throws with what the command said when it fails. */
function must(file, args, options = {}) {
  const result = run(file, args, options);
  if (result.code !== 0) {
    throw new Error(`${file} ${args.join(' ')} failed (${result.code}): ${(result.stderr || result.stdout).split('\n').slice(-20).join('\n')}${result.error ? ` ${result.error.message}` : ''}`);
  }
  return result;
}

const sudo = (args, options) => must('sudo', ['-n', ...args], options);

/** The version line a binary prints for `--version`. */
function versionLine(binary) {
  return must(binary, ['--version']).stdout.trim();
}

/**
 * Installs the published launcher package and finds the native executable of
 * the platform package, so no Node.js stands between the campaign and the
 * binary it measures.
 * @param {{ version: string, prefix: string }} options
 */
function installPublished(options) {
  fs.mkdirSync(options.prefix, { recursive: true });
  const env = { ...process.env, npm_config_update_notifier: 'false' };
  let version = options.version;
  if (!version) {
    version = must('npm', ['view', 'vsift-cli', 'dist-tags.next'], { env }).stdout.trim();
  }
  must('npm', ['install', '--prefix', options.prefix, '--ignore-scripts', '--no-audit', '--no-fund', `vsift-cli@${version}`], { env, timeoutMs: 10 * 60 * 1000 });
  const binary = path.join(options.prefix, 'node_modules', '@vsift', 'linux-x64', 'vsift');
  if (!fs.existsSync(binary)) throw new Error(`the platform package holds no executable at ${binary}`);
  const line = versionLine(binary);
  if (!line.startsWith(`vsift ${version}`)) throw new Error(`the installed executable says "${line}", not ${version}`);
  return { version, binary, versionLine: line };
}

/**
 * An ext4 file system in a file, mounted with write barriers on `target`: the
 * local ext4 that docs/operations/worker-host.md section 1 asks for and a
 * hosted runner's own disk (mounted `nobarrier`) does not offer. Returns what
 * the mount table says afterwards, or the reason it could not be made.
 * @param {string} target
 * @param {number} sizeGiB sparse: nothing is written until the volume is used
 * @returns {{ mounted: boolean, detail: string }}
 */
function mountBarrierVolume(target, sizeGiB) {
  const image = `/var/lib/p14-volumes/${path.basename(target)}.img`;
  const steps = [
    ['mkdir', '-p', '/var/lib/p14-volumes', target],
    ['truncate', '--size', `${sizeGiB}G`, image],
    ['mkfs.ext4', '-q', '-F', image],
    ['mount', '-o', 'loop,rw,relatime', image, target],
  ];
  for (const step of steps) {
    const result = run('sudo', ['-n', ...step]);
    if (result.code !== 0) return { mounted: false, detail: `${step[0]} failed (${result.code}): ${(result.stderr || result.stdout).trim().slice(0, 300)}` };
  }
  const table = run('findmnt', ['--target', target, '--noheadings', '--output', 'FSTYPE,OPTIONS']).stdout.trim();
  return { mounted: true, detail: table };
}

/**
 * Creates the layout of the runbook's section 1 and gives the worker account
 * what it owns: the state folder and the bundle root; the input root is
 * readable by it and written by nobody here. The state folder and the bundle
 * root are each an ext4 volume with write barriers, so `durable` workspaces
 * can be exercised; where the mount fails the layout is made on the disk as it
 * is and the campaign falls back to `ephemeral`, saying so.
 * @returns {{ state: { mounted: boolean, detail: string }, bundles: { mounted: boolean, detail: string } }}
 */
function createLayout() {
  const state = mountBarrierVolume(STATE, 24);
  const bundles = mountBarrierVolume(BUNDLES, 8);
  sudo(['mkdir', '-p', `${STATE}/home`, `${STATE}/queue`, `${STATE}/results`, INPUTS, BUNDLES]);
  sudo(['chown', '-R', container.WORKER_USER, STATE, BUNDLES]);
  sudo(['chmod', '0700', STATE, `${STATE}/home`, BUNDLES]);
  sudo(['chmod', '0755', '/srv/vsift', INPUTS]);
  return { state, bundles };
}

/** Builds the worker image around the published executable. */
function buildImage(options) {
  const context = path.join(options.workDir, 'image');
  fs.rmSync(context, { recursive: true, force: true });
  fs.mkdirSync(context, { recursive: true });
  fs.copyFileSync(options.binary, path.join(context, 'vsift'));
  fs.copyFileSync(path.join(__dirname, '..', 'worker.Dockerfile'), path.join(context, 'Dockerfile'));
  fs.copyFileSync(path.join(__dirname, '..', 'case-wrapper.sh'), path.join(context, 'case-wrapper.sh'));
  must('docker', ['build', '--tag', IMAGE, context], { timeoutMs: 15 * 60 * 1000 });
  return must('docker', ['inspect', '--format', '{{.Id}}', IMAGE]).stdout.trim();
}

/**
 * Runs a short command in the image with the worker's folders mounted: with
 * network for the install, without it for everything else.
 * @param {string[]} command
 * @param {{ network?: string, name: string, cpus?: number, memory?: string, timeoutMs?: number }} options
 */
function inWorker(command, options) {
  const args = container.runArguments({
    image: IMAGE,
    name: options.name,
    command,
    network: options.network || 'none',
    cpus: options.cpus || 2,
    memory: options.memory || '4g',
    remove: true,
    volumes: [`${STATE}:${STATE}:rw`, `${INPUTS}:${INPUTS}:ro`, `${BUNDLES}:${BUNDLES}:rw`],
  });
  return run('docker', args, { timeoutMs: options.timeoutMs || 15 * 60 * 1000 });
}

/**
 * Installs the three reviewed tools with the published binary, as a user would:
 * read the plan, accept its digest, install, then check.
 * @returns {{ plan: object, install: object, check: object }}
 */
function installManagedTools() {
  const plan = inWorker(['vsift', 'setup', 'plan', '--profile', 'worker', '--json'], { name: 'vsift-plan', network: 'bridge' });
  if (plan.code !== 0 && plan.code !== 1) throw new Error(`setup plan failed (${plan.code}): ${plan.stderr}`);
  const planJson = JSON.parse(plan.stdout);
  const digest = planJson.data && planJson.data.plan_digest;
  if (!digest) throw new Error(`the plan has no digest: ${plan.stdout.slice(0, 400)}`);
  sudo(['sh', '-c', `cat > ${STATE}/queue/plan.json && chown ${container.WORKER_USER} ${STATE}/queue/plan.json`], { input: plan.stdout });
  const install = inWorker(
    ['vsift', 'setup', 'install', '--plan', `${STATE}/queue/plan.json`, '--accept-plan', digest, '--json'],
    { name: 'vsift-install', network: 'bridge', timeoutMs: 30 * 60 * 1000 },
  );
  const installJson = JSON.parse(install.stdout);
  if (install.code !== 0) throw new Error(`setup install failed (${install.code}): ${install.stdout.slice(0, 2000)} ${install.stderr.slice(0, 1000)}`);
  const check = inWorker(['vsift', 'setup', 'check', '--json'], { name: 'vsift-check' });
  return { plan: planJson, install: installJson, check: JSON.parse(check.stdout) };
}

/** Puts the corpus videos and sidecars in the input root, below `incoming/`. */
function stageInputs(repository) {
  const corpus = path.join(repository, 'fixtures', 'corpus');
  sudo(['mkdir', '-p', `${INPUTS}/incoming`]);
  const generated = path.join(corpus, 'generated');
  for (const name of fs.readdirSync(generated)) {
    if (/\.(mp4|mkv|m4a)$/.test(name)) sudo(['cp', path.join(generated, name), `${INPUTS}/incoming/${name}`]);
  }
  for (const name of fs.readdirSync(path.join(corpus, 'transcripts'))) {
    sudo(['cp', path.join(corpus, 'transcripts', name), `${INPUTS}/incoming/${name}`]);
  }
  sudo(['chmod', '-R', 'a+rX', `${INPUTS}`]);
  sudo(['chown', '-R', 'root:root', `${INPUTS}`]);
}

/** Facts about the machine, for the record. */
function describeHost() {
  const read = (file) => {
    try {
      return fs.readFileSync(file, 'utf8');
    } catch {
      return '';
    }
  };
  const os = require('node:os');
  const memTotal = /MemTotal:\s+(\d+) kB/.exec(read('/proc/meminfo'));
  const release = /PRETTY_NAME="?([^"\n]+)/.exec(read('/etc/os-release'));
  return {
    os: release ? release[1] : os.platform(),
    kernel: os.release(),
    cpus: os.cpus().length,
    memoryMiB: memTotal ? Math.round(Number(memTotal[1]) / 1024) : null,
    cpuModel: (os.cpus()[0] || {}).model || null,
    docker: run('docker', ['--version']).stdout.trim(),
    storage: run('findmnt', ['--target', STATE, '--noheadings', '--output', 'FSTYPE,OPTIONS']).stdout.trim(),
    cgroup: run('stat', ['-fc', '%T', '/sys/fs/cgroup']).stdout.trim(),
  };
}

module.exports = {
  STATE, INPUTS, BUNDLES, IMAGE, run, must, sudo, versionLine, installPublished, createLayout, mountBarrierVolume, buildImage, inWorker,
  installManagedTools, stageInputs, describeHost,
};
