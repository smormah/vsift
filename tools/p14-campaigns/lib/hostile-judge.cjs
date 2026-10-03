'use strict';

// How one operation on a hostile input is judged (P14 RQ-10): a typed failure
// inside its bound, a clean success, or a finding. The rule is the plan's:
// each hostile input ends in `INVALID_SOURCE`, `RESOURCE_LIMIT` or
// `DEADLINE_EXCEEDED` (or, where the case allows, in a result), inside its
// time, memory and process bounds, with no network, no file outside the root
// and nothing a terminal would act on in its output.

const BOUNDS = Object.freeze({
  wallMs: 120_000,
  memoryBytes: 1024 * 1024 * 1024,
  pids: 128,
});

/** The failures that follow-up calls on an accepted source may add (a time outside the video, no audio track). */
const FOLLOW_UP_CODES = ['INVALID_ARGUMENT'];

/**
 * @param {string} stderr
 * @returns {{ exit: number|null, memoryPeak: number|null, pidsPeak: number|null, oomKills: number|null }|null}
 */
function readMetrics(stderr) {
  const lines = String(stderr).split('\n').filter((line) => line.startsWith('P14-METRICS '));
  if (lines.length === 0) return null;
  const line = lines[lines.length - 1];
  const number = (name) => {
    const found = new RegExp(`${name}=(\\d+)`).exec(line);
    return found ? Number(found[1]) : null;
  };
  const events = /memory_events=(.*)$/.exec(line);
  const oom = events ? /oom_kill (\d+)/.exec(events[1]) : null;
  return { exit: number('exit'), memoryPeak: number('memory_peak'), pidsPeak: number('pids_peak'), oomKills: oom ? Number(oom[1]) : null };
}

/** Raw bytes a terminal would act on: C0 controls but tab and line feed, DEL, and C1 controls. */
function rawControls(text) {
  const found = new Set();
  const buffer = Buffer.from(String(text), 'utf8');
  for (let index = 0; index < buffer.length; index += 1) {
    const byte = buffer[index];
    if ((byte < 0x20 && byte !== 0x09 && byte !== 0x0a && byte !== 0x0d) || byte === 0x7f) found.add(`0x${byte.toString(16).padStart(2, '0')}`);
    if (byte === 0xc2 && buffer[index + 1] >= 0x80 && buffer[index + 1] <= 0x9f) found.add(`U+00${buffer[index + 1].toString(16)}`);
  }
  return [...found];
}

/** The failure code of a result document, or null. */
function codeOf(json) {
  if (!json) return null;
  if (json.error && json.error.code) return json.error.code;
  if (json.data && json.data.failure && json.data.failure.code) return json.data.failure.code;
  return null;
}

/**
 * Judges one operation.
 * @param {object} o
 * @param {{ id: string, expect: 'failure'|'any', codes?: string[] }} o.spec the case
 * @param {string} o.op the operation
 * @param {boolean} o.first whether it is the first operation (the ingest) of the case
 * @param {{ code: number|null, stdout: string, stderr: string, durationMs: number, timedOut: boolean, oomKilled: boolean, network: string|null }} o.run
 * @param {string[]} o.canaries strings that must not appear in any output
 * @returns {{ ok: boolean, outcome: string, findings: string[], json: object|null }}
 */
function judge(o) {
  const findings = [];
  let json = null;
  try {
    json = JSON.parse(o.run.stdout);
  } catch {
    json = null;
  }
  const human = o.op === 'ingest_human';
  const allowed = new Set([...(o.spec.codes || ['INVALID_SOURCE', 'RESOURCE_LIMIT', 'DEADLINE_EXCEEDED']), ...(o.first ? [] : FOLLOW_UP_CODES)]);
  const metrics = readMetrics(o.run.stderr);

  if (o.run.timedOut) findings.push(`did not finish within ${BOUNDS.wallMs / 1000} s (a hang)`);
  else if (o.run.durationMs > BOUNDS.wallMs) findings.push(`took ${Math.round(o.run.durationMs / 1000)} s, above the ${BOUNDS.wallMs / 1000} s bound`);
  if (o.run.oomKilled || (metrics && metrics.oomKills > 0) || (o.run.code === 137 && !o.run.timedOut)) findings.push('the container ran out of memory');
  if (metrics && metrics.memoryPeak !== null && metrics.memoryPeak > BOUNDS.memoryBytes) findings.push(`memory peaked at ${Math.round(metrics.memoryPeak / 1048576)} MiB, above the ${BOUNDS.memoryBytes / 1048576} MiB limit`);
  if (metrics && metrics.pidsPeak !== null && metrics.pidsPeak > BOUNDS.pids) findings.push(`${metrics.pidsPeak} processes at once, above the ${BOUNDS.pids} limit`);
  if (o.run.network !== null && o.run.network !== 'none') findings.push(`the container's network was ${o.run.network}, not none`);

  const controls = rawControls(`${o.run.stdout}${o.run.stderr.replace(/^P14-METRICS .*$/m, '')}`);
  if (controls.length > 0) findings.push(`raw control characters in the output (${controls.join(', ')})`);
  for (const canary of o.canaries) {
    if (canary && (o.run.stdout.includes(canary) || o.run.stderr.includes(canary))) findings.push('a canary from outside the root is in the output');
  }

  let outcome;
  if (human) {
    outcome = o.run.code === 0 ? 'complete' : `exit ${o.run.code}`;
  } else if (!json) {
    outcome = `no JSON (exit ${o.run.code})`;
    if (!o.run.timedOut) findings.push(`the answer is not JSON (exit ${o.run.code}): ${o.run.stdout.slice(0, 120) || o.run.stderr.slice(0, 120)}`);
  } else if (json.status === 'complete' || json.status === 'partial') {
    outcome = json.status;
    if (o.first && o.spec.expect === 'failure') findings.push(`was accepted (${json.status}) but must be refused`);
  } else {
    const code = codeOf(json);
    outcome = `${json.status} ${code}`;
    if (!allowed.has(code)) findings.push(`failed with ${code}, not one of ${[...allowed].join(', ')}`);
    if (o.run.code === 0) findings.push('failed in the answer but exited 0');
  }
  return { ok: findings.length === 0, outcome, findings, json, metrics };
}

module.exports = { BOUNDS, FOLLOW_UP_CODES, readMetrics, rawControls, codeOf, judge };

/**
 * The case-level rule for a media file that must be refused: some operation
 * must have failed typed, and no operation after it may have succeeded.
 * @param {{ outcome: string, ok: boolean }[]} results
 * @returns {string|null} a finding, or null
 */
function refusedSomewhere(results) {
  const refusal = results.findIndex((entry) => /^failed /.test(entry.outcome));
  if (refusal < 0) return 'was never refused: every operation succeeded';
  const later = results.slice(refusal + 1).find((entry) => /^(complete|partial)$/.test(entry.outcome));
  return later ? `an operation succeeded after the refusal (${later.outcome})` : null;
}

module.exports.refusedSomewhere = refusedSomewhere;

/**
 * Findings the campaign already tracks are told apart from new ones, so that
 * a run which finds only what is filed does not fail the pull request that
 * carries the campaign, while a new finding does. A tracked case that now
 * passes is reported so its entry can be removed.
 * @param {{ id: string, op: string, findings: string[] }[]} findings
 * @param {Record<string, number>} tracked case id to issue number
 * @param {string[]} passingIds ids of the cases that had no finding
 * @returns {{ known: { id: string, op: string, findings: string[], issue: number }[], fresh: { id: string, op: string, findings: string[] }[], fixed: string[] }}
 */
function splitKnown(findings, tracked, passingIds) {
  const known = [];
  const fresh = [];
  for (const finding of findings) {
    if (Object.prototype.hasOwnProperty.call(tracked, finding.id)) known.push({ ...finding, issue: tracked[finding.id] });
    else fresh.push(finding);
  }
  const fixed = Object.keys(tracked).filter((id) => passingIds.includes(id));
  return { known, fresh, fixed };
}

module.exports.splitKnown = splitKnown;
