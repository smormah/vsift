'use strict';

// Reads the JSON Lines stream of `vsift job batch --events jsonl` (docs/
// contracts/cli-v1.md, "P11 job batch") and judges it against the worker
// runbook (docs/operations/worker-host.md, sections 3 and 4): what each line
// of the batch came to, whether the supervisor may acknowledge it, and
// whether the stream kept its own rules (bounded events, an order, no path or
// secret in any of them).

const MAX_EVENT_BYTES = 64 * 1024;

/** Failures the worker records for good: the supervisor acknowledges these (to a dead-letter queue). */
const RECORDED_FAILURES = new Set([
  'INVALID_ARGUMENT', 'INVALID_SOURCE', 'RESOURCE_LIMIT', 'INTEGRITY_FAILURE', 'UNSUPPORTED_SCHEMA', 'INTERNAL', 'MISSING_CAPABILITY',
]);

/** Outcomes that are not recorded: the supervisor redelivers the same bytes. */
const REDELIVERED = new Set(['BUSY', 'CANCELLED', 'DEADLINE_EXCEEDED', 'STORAGE_IO']);

/**
 * @param {string} text the standard output of a batch
 * @returns {{ events: object[], problems: string[] }}
 */
function parseEvents(text) {
  const events = [];
  const problems = [];
  const lines = String(text).split('\n');
  lines.forEach((line, index) => {
    if (line.trim() === '') return;
    if (Buffer.byteLength(line) > MAX_EVENT_BYTES) {
      problems.push(`event ${index + 1} is ${Buffer.byteLength(line)} bytes, above the ${MAX_EVENT_BYTES} byte bound`);
    }
    try {
      events.push(JSON.parse(line));
    } catch {
      problems.push(`line ${index + 1} of the stream is not JSON`);
    }
  });
  return { events, problems };
}

/**
 * What a batch's events say. `outcomes` is keyed by the batch line number.
 * @param {object[]} events
 */
function readBatch(events) {
  const outcomes = new Map();
  const results = new Map();
  let started = null;
  let stopped = null;
  let terminal = null;
  const admitted = new Set();
  const finished = new Set();
  let running = 0;
  let peakRunning = 0;
  const problems = [];
  let previousSequence = -1;
  events.forEach((event, position) => {
    if (typeof event.sequence === 'number') {
      if (event.sequence <= previousSequence) problems.push(`sequence ${event.sequence} does not follow ${previousSequence}`);
      previousSequence = event.sequence;
    }
    if (position === 0 && !(event.event === 'lifecycle' && event.kind === 'started')) {
      problems.push('the first event is not lifecycle started (the host is not ready)');
    }
    if (event.event === 'lifecycle') {
      if (event.kind === 'started') started = event.readiness;
      if (event.kind === 'stopped') stopped = event.reason;
      if (event.kind === 'request_admitted') {
        admitted.add(event.line);
        running += 1;
        peakRunning = Math.max(peakRunning, running);
      }
      if (event.kind === 'request_finished') {
        outcomes.set(event.line, {
          status: event.status,
          code: event.code || null,
          rejection: event.rejection || null,
          operationId: event.request_operation_id || null,
        });
        if (admitted.has(event.line) && !finished.has(event.line)) {
          running -= 1;
          finished.add(event.line);
        }
      }
    }
    if (event.event === 'result' && typeof event.line === 'number') {
      results.set(event.line, event.result);
    }
    if (event.event === 'terminal') terminal = event.result;
  });
  const last = events[events.length - 1];
  const ended = Boolean(last) && last.event === 'terminal';
  return { started, stopped, terminal, outcomes, results, peakRunning, ended, problems };
}

/**
 * The supervisor's reading of one line (runbook section 4).
 * @param {{ status: string, code: string|null, rejection: string|null }|undefined} outcome
 * @returns {'recorded'|'dead_letter'|'conflict'|'redeliver'}
 */
function disposition(outcome) {
  if (!outcome) return 'redeliver';
  if (outcome.status === 'complete' || outcome.status === 'partial') return 'recorded';
  if (outcome.status === 'rejected') return 'dead_letter';
  if (outcome.status === 'failed') {
    if (outcome.code === 'IDEMPOTENCY_CONFLICT') return 'conflict';
    if (RECORDED_FAILURES.has(outcome.code)) return 'dead_letter';
    return 'redeliver';
  }
  return 'redeliver';
}

/**
 * Whether a planned line (requests.cjs) came to what it was expected to.
 * @param {{ expect: string, codes?: string[] }} planned
 * @param {{ status: string, code: string|null }|undefined} outcome
 * @param {{ replayed?: boolean }|undefined} result
 * @returns {{ ok: boolean, why: string }}
 */
function meets(planned, outcome, result) {
  const reading = disposition(outcome);
  switch (planned.expect) {
    case 'recorded':
      return reading === 'recorded' ? { ok: true, why: '' } : { ok: false, why: `expected a recorded result, got ${describe(outcome)}` };
    case 'replayed':
      return reading === 'recorded' && result && result.replayed === true
        ? { ok: true, why: '' }
        : { ok: false, why: `expected a replayed result, got ${describe(outcome)}${result ? ` (replayed ${result.replayed})` : ''}` };
    case 'rejected':
      return outcome && outcome.status === 'rejected'
        ? { ok: true, why: '' }
        : { ok: false, why: `expected the line to be refused alone, got ${describe(outcome)}` };
    case 'failed':
      return outcome && outcome.status === 'failed' && (planned.codes || []).includes(outcome.code)
        ? { ok: true, why: '' }
        : { ok: false, why: `expected a failure ${JSON.stringify(planned.codes)}, got ${describe(outcome)}` };
    default:
      return { ok: false, why: `unknown expectation ${planned.expect}` };
  }
}

/**
 * A duplicate or a conflict whose original request's record may have been pruned (its session was removed and the
 * record table was full) is run again as a new request: a fresh `complete` result instead of a replay, and a
 * recorded result instead of `IDEMPOTENCY_CONFLICT`. Whether the original's session was removed is the caller's
 * to check; this only says that the answer is that of a new request.
 * @param {{ expect: string, codes?: string[] }} planned
 * @param {{ status: string, code?: string }|undefined} outcome
 * @param {{ replayed?: boolean }|undefined} result
 * @returns {boolean}
 */
function outsideWindow(planned, outcome, result) {
  if (disposition(outcome) !== 'recorded') return false;
  if (planned.expect === 'replayed') return Boolean(result) && result.replayed === false;
  if (planned.expect === 'failed') return (planned.codes || []).includes('IDEMPOTENCY_CONFLICT');
  return false;
}

function describe(outcome) {
  if (!outcome) return 'no outcome';
  return `${outcome.status}${outcome.code ? ` ${outcome.code}` : ''}${outcome.rejection ? ` (${outcome.rejection})` : ''}`;
}

/**
 * Where a secret or a path appears in output that must hold neither (runbook
 * section 3: events carry "no paths, no transcript text"; O-01: sentinel values
 * stay out of every diagnostic).
 * @param {string} text
 * @param {string[]} forbidden literal strings
 * @returns {string[]} the forbidden strings found
 */
function leaks(text, forbidden) {
  return forbidden.filter((needle) => needle && String(text).includes(needle));
}

module.exports = {
  MAX_EVENT_BYTES, RECORDED_FAILURES, REDELIVERED, parseEvents, readBatch, disposition, meets, outsideWindow, leaks,
};
