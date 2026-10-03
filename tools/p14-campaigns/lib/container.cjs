'use strict';

// The Docker calls of the worker-host campaigns (P14 RQ-09, RQ-10, RQ-12).
// Every call is an explicit executable and argument list, never a shell; the
// hardened container is the one of `docs/operations/worker-host.md` section 10
// (read-only root, no network, CPU, memory, swap and process limits, every
// capability dropped, no new privileges, an unprivileged user), scaled to the
// runner. Nothing here is for a developer's machine.

const childProcess = require('node:child_process');

/** The unprivileged account of the image (worker.Dockerfile). */
const WORKER_USER = '10001:10001';
const HOME = '/var/lib/vsift/home';

const OUTPUT_CAP = 64 * 1024 * 1024;

/**
 * The arguments of `docker run` for a hardened worker container.
 * @param {object} o
 * @param {string} o.image an image tag or id
 * @param {string} o.name the container's name
 * @param {string[]} o.command the program and its arguments
 * @param {string} [o.network] `none` for the worker, `bridge` only for the tool install
 * @param {number} o.cpus
 * @param {string} o.memory for example `12g`; swap is set to the same
 * @param {number} [o.pids]
 * @param {string[]} [o.volumes] `host:container:mode`
 * @param {Record<string, string>} [o.env]
 * @param {number} [o.stopTimeout] seconds `docker stop` waits before SIGKILL
 * @param {boolean} [o.remove] pass `--rm`
 * @param {string} [o.tmpfs] the /tmp mount, default 64 MiB noexec
 * @param {string[]} [o.extra] more `docker run` options, placed before the image
 */
function runArguments(o) {
  const args = ['run'];
  if (o.remove) args.push('--rm');
  args.push('--init', '--name', o.name, '--network', o.network || 'none', '--read-only');
  args.push('--tmpfs', o.tmpfs || '/tmp:rw,noexec,nosuid,size=64m');
  args.push('--cpus', String(o.cpus), '--memory', o.memory, '--memory-swap', o.memory, '--pids-limit', String(o.pids || 256));
  args.push('--cap-drop', 'ALL', '--security-opt', 'no-new-privileges', '--user', WORKER_USER);
  const env = { HOME, XDG_CONFIG_HOME: `${HOME}/.config`, XDG_CACHE_HOME: `${HOME}/.cache`, ...(o.env || {}) };
  for (const [name, value] of Object.entries(env)) args.push('--env', `${name}=${value}`);
  for (const volume of o.volumes || []) args.push('--volume', volume);
  args.push('--stop-timeout', String(o.stopTimeout || 45));
  args.push(...(o.extra || []));
  args.push(o.image, ...o.command);
  return args;
}

/**
 * Runs `docker` with `args` to the end (or the timeout, when the container is
 * killed by name) and keeps its output.
 * @param {string[]} args
 * @param {{ timeoutMs?: number, killName?: string, input?: string, onStart?: (child: import('node:child_process').ChildProcess) => void, onStdout?: (text: string) => void }} [options]
 * @returns {Promise<{ code: number|null, stdout: string, stderr: string, durationMs: number, timedOut: boolean }>}
 */
function docker(args, options = {}) {
  return new Promise((resolve) => {
    const begun = Date.now();
    const child = childProcess.spawn('docker', args, { stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    let timedOut = false;
    const keep = (current, chunk) => {
      const next = current + chunk.toString('utf8');
      return next.length > OUTPUT_CAP ? next.slice(next.length - OUTPUT_CAP) : next;
    };
    child.stdout.on('data', (chunk) => {
      stdout = keep(stdout, chunk);
      if (options.onStdout) options.onStdout(chunk.toString('utf8'));
    });
    child.stderr.on('data', (chunk) => { stderr = keep(stderr, chunk); });
    child.stdin.on('error', () => {});
    child.stdin.end(options.input || '');
    let timer = null;
    if (options.timeoutMs) {
      timer = setTimeout(() => {
        timedOut = true;
        if (options.killName) {
          childProcess.spawnSync('docker', ['kill', '--signal', 'KILL', options.killName], { stdio: 'ignore' });
        }
        child.kill('SIGKILL');
      }, options.timeoutMs);
    }
    if (options.onStart) options.onStart(child);
    child.on('error', (error) => {
      if (timer) clearTimeout(timer);
      resolve({ code: null, stdout, stderr: `${stderr}\nspawn error: ${error.message}`, durationMs: Date.now() - begun, timedOut });
    });
    child.on('close', (code) => {
      if (timer) clearTimeout(timer);
      resolve({ code, stdout, stderr, durationMs: Date.now() - begun, timedOut });
    });
  });
}

/** Synchronous docker, for the short calls (inspect, kill, rm, build). */
function dockerSync(args, options = {}) {
  const result = childProcess.spawnSync('docker', args, { encoding: 'utf8', maxBuffer: OUTPUT_CAP, ...options });
  return { code: result.status, stdout: result.stdout || '', stderr: result.stderr || '' };
}

/** Sends `signal` (`TERM`, `KILL`) to the container's main process. */
function signal(name, which) {
  return dockerSync(['kill', '--signal', which, name]);
}

/** What the engine recorded when a container ended. */
function state(name) {
  const result = dockerSync(['inspect', '--format', '{{json .State}}|{{.Id}}|{{.HostConfig.NetworkMode}}', name]);
  if (result.code !== 0) return null;
  const [json, id, network] = result.stdout.trim().split('|');
  try {
    const parsed = JSON.parse(json);
    return { exitCode: parsed.ExitCode, oomKilled: parsed.OOMKilled, id, network, status: parsed.Status };
  } catch {
    return null;
  }
}

function remove(name) {
  return dockerSync(['rm', '--force', '--volumes', name]);
}

module.exports = { WORKER_USER, HOME, runArguments, docker, dockerSync, signal, state, remove };
