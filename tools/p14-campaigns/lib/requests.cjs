'use strict';

// Job requests and batch lines for the load campaign (P14 RQ-09) and the
// runbook walk (RQ-12): what to ask of a worker host, and what each line is
// expected to come to. Every request is a v1 job request (docs/contracts/
// cli-v1.md, "Job request"); the malformed ones are malformed on purpose.

const crypto = require('node:crypto');

/** SHA-256 of `text` as lower-case hexadecimal. */
function sha256(text) {
  return crypto.createHash('sha256').update(text).digest('hex');
}

/** An operation id derived from a campaign seed and a number, so a rerun asks for the same ids. */
function operationId(seed, index) {
  return `op_${sha256(`${seed}:${index}`).slice(0, 32)}`;
}

/** A small deterministic generator (mulberry32) seeded from text. */
class Random {
  constructor(seed) {
    this.state = Number.parseInt(sha256(String(seed)).slice(0, 8), 16) >>> 0;
  }

  /** @returns {number} a float in [0, 1) */
  next() {
    this.state = (this.state + 0x6d2b79f5) >>> 0;
    let t = this.state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }

  /** @returns {number} an integer in [0, bound) */
  int(bound) {
    return Math.floor(this.next() * bound);
  }

  pick(items) {
    return items[this.int(items.length)];
  }
}

/** The corpus videos a request may ingest, and which have speech and a sidecar. */
const VIDEOS = [
  'F01.mp4', 'F02.mp4', 'F03.mp4', 'F04.mp4', 'F05.mp4', 'F06.mp4', 'F07.mp4', 'F08.mp4', 'F09.mkv', 'F10.mp4', 'F12.mp4',
];
// F02, F04 and F05 are left out: on the published 0.1.0 their first five seconds do not recognise (the load run's
// diagnosis lists each clip), and a load campaign needs requests that complete.
const SPEECH_VIDEOS = ['F01-speech.mp4', 'F03-speech.mp4', 'F06-speech.mp4', 'F07-speech.mp4'];
/**
 * How many recent recorded requests a duplicate or a conflict may name. A workspace keeps at most 4,096 request
 * records and prunes the oldest of removed sessions first (worker-host.md, section 5), so the dedupe window is a
 * few thousand requests, not the whole soak; a delivery older than that runs again, which is the documented limit.
 */
const DEDUPE_WINDOW = 1500;
const SIDECAR = { video: 'F10.mp4', transcript: 'canary.srt' };

/**
 * One job request as the object the contract describes.
 * @param {object} spec
 * @param {string} spec.id the operation id
 * @param {string|null} [spec.source] a path below the input root, for an ingest target
 * @param {{path: string, offset_us: number}|null} [spec.transcript]
 * @param {string|null} [spec.session] a session id, for a session target
 * @param {object[]} spec.steps
 * @param {'durable'|'ephemeral'} [spec.durability]
 * @param {number|null} [spec.deadlineMs]
 */
function request(spec) {
  const target = spec.session
    ? { session_id: spec.session }
    : { ingest: { source: spec.source, transcript: spec.transcript || null } };
  const body = {
    schema_version: '1',
    operation_id: spec.id,
    durability: spec.durability || 'ephemeral',
  };
  if (spec.deadlineMs) body.deadline_ms = spec.deadlineMs;
  body.target = target;
  body.steps = spec.steps;
  return body;
}

const STEPS = {
  candidates: (range = null) => ({ candidates: { range } }),
  retranscribe: (range = null) => ({ retranscribe: { range } }),
  retain: (name, includeSource = false) => ({ retain: { bundle_name: name, include_source: includeSource } }),
  close: () => ({ close: {} }),
};

/** A bundle name derived from an operation id (`[a-z0-9][a-z0-9_-]{0,63}`). */
function bundleName(id) {
  return `b-${id.slice(3, 19)}`;
}

/**
 * What a line is expected to come to, in the words of the worker runbook's
 * acknowledgement table (section 4):
 *   - `recorded`:   a result event with status complete or partial;
 *   - `rejected`:   the line is refused alone, with no result (dead-letter);
 *   - `failed`:     a recorded failure with one of `codes`;
 *   - `replayed`:   a result with replayed true, identical to the first.
 * @typedef {{ kind: string, id: string, text: string, expect: string, codes?: string[], of?: string }} Planned
 */

/**
 * The 1,000-request mix of the soak, or any prefix of it: counts follow the
 * plan's list (imports, candidates, frames of the follow-up calls, small
 * recognitions, malformed lines, duplicates and conflicts); cancels, kills and
 * resumes belong to the driver that runs the lines, not to the mix.
 * @param {string} seed
 * @param {number} total
 * @param {'durable'|'ephemeral'} [durability] what each request says it needs
 * @returns {Planned[]}
 */
function soakMix(seed, total, durability = 'ephemeral') {
  const random = new Random(seed);
  const make = (spec) => request({ durability, ...spec });
  const planned = [];
  const recorded = []; // lines that complete, for duplicates and conflicts
  for (let index = 0; index < total; index += 1) {
    const id = operationId(seed, index);
    const roll = random.int(100);
    let line;
    if (roll < 42) {
      const video = random.pick(VIDEOS);
      line = { kind: 'ingest_candidates', expect: 'recorded', id, body: make({
        id, source: `incoming/${video}`, steps: [STEPS.candidates(), STEPS.close()],
      }) };
    } else if (roll < 48) {
      // Leaves its session open, so the driver can ask evidence calls of it and then close it.
      const video = random.pick(VIDEOS);
      line = { kind: 'ingest_open', expect: 'recorded', id, body: make({
        id, source: `incoming/${video}`, steps: [STEPS.candidates()],
      }) };
    } else if (roll < 63) {
      const video = random.pick(VIDEOS);
      line = { kind: 'ingest_candidates_retain', expect: 'recorded', id, body: make({
        id, source: `incoming/${video}`, steps: [STEPS.candidates(), STEPS.retain(bundleName(id)), STEPS.close()],
      }) };
    } else if (roll < 69) {
      const video = random.pick(SPEECH_VIDEOS);
      line = { kind: 'recognise', expect: 'recorded', id, body: make({
        id, source: `incoming/${video}`, steps: [STEPS.retranscribe({ from_us: 0, to_us: 5_000_000 }), STEPS.close()],
      }) };
    } else if (roll < 75) {
      line = { kind: 'supplied_transcript', expect: 'recorded', id, body: make({
        id,
        source: `incoming/${SIDECAR.video}`,
        transcript: { path: `incoming/${SIDECAR.transcript}`, offset_us: 500000 },
        steps: [STEPS.candidates(), STEPS.close()],
      }) };
    } else if (roll < 85) {
      line = { id: null, ...malformed(random, id) };
    } else if (roll < 93 && recorded.length > 0) {
      const earlier = random.pick(recorded.slice(-DEDUPE_WINDOW));
      line = { kind: 'duplicate', expect: 'replayed', id: earlier.id, of: earlier.id, body: earlier.body };
    } else if (roll < 97 && recorded.length > 0) {
      const earlier = random.pick(recorded.slice(-DEDUPE_WINDOW));
      const other = random.pick(VIDEOS);
      line = { kind: 'conflict', expect: 'failed', codes: ['IDEMPOTENCY_CONFLICT'], id: earlier.id, of: earlier.id, body: make({
        id: earlier.id, source: `incoming/${other}`, steps: [STEPS.close()],
      }) };
    } else {
      line = { kind: 'missing_source', expect: 'failed', codes: ['INVALID_SOURCE', 'INVALID_ARGUMENT'], id, body: make({
        id, source: 'incoming/not-there.mp4', steps: [STEPS.candidates(), STEPS.close()],
      }) };
    }
    const text = typeof line.body === 'string' ? line.body : JSON.stringify(line.body);
    planned.push({ index, kind: line.kind, expect: line.expect, id: line.id, codes: line.codes, of: line.of, text });
    if (line.expect === 'recorded') recorded.push({ id, body: line.body });
  }
  return planned;
}

/** A line that is not a valid request; the host must refuse it alone. */
function malformed(random, id) {
  const variants = [
    () => '{"schema_version":"1","operation_id":',
    () => 'not json at all',
    () => JSON.stringify({ ...request({ id, source: 'incoming/F01.mp4', steps: [STEPS.close()] }), schema_version: '2' }),
    () => JSON.stringify({ ...request({ id, source: 'incoming/F01.mp4', steps: [STEPS.close()] }), unreviewed: true }),
    () => JSON.stringify(request({ id, source: 'incoming/../../etc/passwd', steps: [STEPS.close()] })),
    () => JSON.stringify(request({ id, source: '/etc/passwd', steps: [STEPS.close()] })),
    () => JSON.stringify(request({ id, source: 'incoming/F01.mp4', steps: [STEPS.retain('Bad Name!'), STEPS.close()] })),
    () => JSON.stringify(request({ id, source: 'incoming/F01.mp4', steps: [STEPS.close(), STEPS.candidates()] })),
    () => JSON.stringify(request({ id: 'op_short', source: 'incoming/F01.mp4', steps: [STEPS.close()] })),
    () => JSON.stringify({ ...request({ id, source: 'incoming/F01.mp4', steps: [STEPS.close()] }), deadline_ms: 0 }),
    () => `{"schema_version":"1","padding":"${'x'.repeat(70_000)}"}`,
    () => `{"a":`.repeat(40) + '1' + '}'.repeat(40),
    () => '\u0000\u0001\u0002 control characters',
  ];
  return { kind: 'malformed', expect: 'rejected', body: random.pick(variants)() };
}

/**
 * The ladder's request set: the same `count` requests for every rung, so a
 * rung differs only in concurrency. Each ingests a corpus video and analyses
 * it; one in six also recognises a short range of speech.
 * @param {string} seed
 * @param {number} count
 * @returns {Planned[]}
 */
function ladderRequests(seed, count, durability = 'ephemeral') {
  const planned = [];
  for (let index = 0; index < count; index += 1) {
    const id = operationId(seed, index);
    const speech = index % 6 === 5;
    const video = speech ? SPEECH_VIDEOS[index % SPEECH_VIDEOS.length] : VIDEOS[index % VIDEOS.length];
    const steps = speech
      ? [STEPS.retranscribe({ from_us: 0, to_us: 5_000_000 }), STEPS.close()]
      : [STEPS.candidates(), STEPS.close()];
    const body = request({ id, source: `incoming/${video}`, steps, durability });
    planned.push({ index, id, kind: speech ? 'recognise' : 'ingest_candidates', expect: 'recorded', body, text: JSON.stringify(body) });
  }
  return planned;
}

/**
 * The text of a batch file: the lines, one per line, in order.
 * @param {Planned[]} lines
 */
function batchFile(lines) {
  return `${lines.map((line) => line.text).join('\n')}\n`;
}

module.exports = {
  sha256, operationId, Random, request, STEPS, bundleName, soakMix, ladderRequests, batchFile,
  VIDEOS, SPEECH_VIDEOS, SIDECAR,
};
