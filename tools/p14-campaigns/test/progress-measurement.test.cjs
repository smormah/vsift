'use strict';

// The progress measurement's tooling (brief W8), driven by scripted `job status`
// answers on a fake clock: no binary, no media, no recogniser, no network.

const assert = require('node:assert/strict');
const childProcess = require('node:child_process');
const path = require('node:path');
const test = require('node:test');
const measure = require('../lib/progress-measurement.cjs');

const SECOND = 1_000_000;
const ABSOLUTE = path.resolve(path.sep, 'tools', 'x');
const required = (extra = []) => ['--vsift', ABSOLUTE, '--media', ABSOLUTE, '--work', ABSOLUTE, '--report', ABSOLUTE, '--ffmpeg', ABSOLUTE, '--ffprobe', ABSOLUTE, '--whisper', ABSOLUTE, '--model', ABSOLUTE, ...extra];

// ---------------------------------------------------------------------------
// Public items only
// ---------------------------------------------------------------------------

test('the measurement refuses a private manifest, as --keep-segments does', () => {
  assert.throws(() => measure.parseArguments(required(['--item', 'RM03', '--private-manifest', ABSOLUTE])), /--private-manifest is refused/);
});

test('the command line refuses a private item id, an unknown option, a relative path and no item', () => {
  assert.throws(() => measure.parseArguments(required(['--item', 'P01'])), /not a public item id/);
  assert.throws(() => measure.parseArguments(required(['--item', 'RM03', '--surprise', 'x'])), /unknown option/);
  assert.throws(() => measure.parseArguments(['--vsift', 'vsift', ...required().slice(2), '--item', 'RM03']), /--vsift must be an absolute path/);
  assert.throws(() => measure.parseArguments(required()), /at least one item/);
  assert.throws(() => measure.parseArguments(required(['--item', 'RM03', '--prefix-minutes', '5,0'])), /whole minutes/);
});

test('the command line reads the items and the prefixes', () => {
  const options = measure.parseArguments(required(['--item', 'RM03', '--item', 'RM09', '--prefix-of', 'RM03', '--prefix-minutes', '5, 10,15']));
  assert.deepEqual(options.items, ['RM03', 'RM09']);
  assert.equal(options.prefixOf, 'RM03');
  assert.deepEqual(options.prefixMinutes, [5, 10, 15]);
});

test('an item marked private is refused', () => {
  assert.throws(() => measure.refusePrivate({ id: 'P01', private: true }), /public items only/);
  assert.doesNotThrow(() => measure.refusePrivate({ id: 'RM03' }));
});

test('the driver exits 2 on --private-manifest before it touches anything', () => {
  const result = childProcess.spawnSync(process.execPath, [path.join(__dirname, '..', 'progress-measurement.cjs'), ...required(['--item', 'RM03', '--private-manifest', ABSOLUTE])], { encoding: 'utf8' });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /--private-manifest is refused/);
});

// ---------------------------------------------------------------------------
// The plan and the prefixes
// ---------------------------------------------------------------------------

const ITEMS = [{ id: 'RM03', media: { duration_s: 1156.125 } }, { id: 'RM09', media: { duration_s: 1682.133 } }];

test('the plan cuts the prefixes shorter than the item, then runs each whole item, alternating the order of the two modes', () => {
  const plan = measure.planMedia(ITEMS, 'RM03', [15, 5, 10]);
  assert.deepEqual(plan.map((entry) => entry.label), ['RM03-first-5min', 'RM03-first-10min', 'RM03-first-15min', 'RM03', 'RM09']);
  assert.deepEqual(plan.map((entry) => entry.modes[0]), ['quick', 'watched', 'quick', 'watched', 'quick']);
  assert.ok(plan.every((entry) => entry.modes.length === 2 && new Set(entry.modes).size === 2));
  assert.deepEqual(plan.map((entry) => entry.kind), ['prefix', 'prefix', 'prefix', 'whole', 'whole']);
});

test('a prefix as long as the item is not cut', () => {
  const plan = measure.planMedia([{ id: 'RM03', media: { duration_s: 600 } }], 'RM03', [5, 10]);
  assert.deepEqual(plan.map((entry) => entry.label), ['RM03-first-5min', 'RM03']);
  assert.throws(() => measure.planMedia(ITEMS, 'RM04', [5]), /not among the items/);
});

function spawnScript(answers) {
  const calls = [];
  const spawn = (program, args) => {
    calls.push([program, ...args]);
    return answers.shift();
  };
  return { spawn, calls };
}

test('a prefix is cut by stream copy when the cut has the length asked for', () => {
  const { spawn, calls } = spawnScript([{ status: 0, stdout: '' }, { status: 0, stdout: '300.021000\n' }]);
  const cut = measure.cutPrefix({ ffmpeg: 'ffmpeg', ffprobe: 'ffprobe', source: 'in.webm', destination: 'out.webm', seconds: 300, spawn });
  assert.deepEqual(cut, { method: 'stream_copy', duration_s: 300.021 });
  assert.deepEqual(calls[0], ['ffmpeg', '-nostdin', '-v', 'error', '-y', '-i', 'in.webm', '-t', '300', '-map', '0:v:0', '-map', '0:a:0', '-c', 'copy', 'out.webm']);
});

test('a cut that is too short is redone once, the video re-encoded and the audio copied, and the file removed first', () => {
  const removed = [];
  const { spawn, calls } = spawnScript([{ status: 0, stdout: '' }, { status: 0, stdout: '212.0\n' }, { status: 0, stdout: '' }, { status: 0, stdout: '599.98\n' }]);
  const cut = measure.cutPrefix({ ffmpeg: 'ffmpeg', ffprobe: 'ffprobe', source: 'in.webm', destination: 'out.webm', seconds: 600, spawn, remove: (file) => removed.push(file) });
  assert.equal(cut.method, 're_encoded');
  assert.deepEqual(removed, ['out.webm']);
  assert.deepEqual(calls[2].slice(-7), ['-c:v', 'mpeg4', '-q:v', '5', '-c:a', 'copy', 'out.webm']);
});

test('a prefix that neither route cuts is an error, not a silent shorter measurement', () => {
  const { spawn } = spawnScript([{ status: 1, stdout: '' }, { status: 1, stdout: '' }]);
  assert.throws(() => measure.cutPrefix({ ffmpeg: 'ffmpeg', ffprobe: 'ffprobe', source: 'a', destination: 'b', seconds: 300, spawn }), /could not be cut/);
});

// ---------------------------------------------------------------------------
// Reading status and following a run
// ---------------------------------------------------------------------------

/**
 * A scripted watched run on a fake clock. The visual stream takes `visualS`,
 * then the transcript runs at `speed(t)` media seconds a second from where it
 * stands; `claimed` is the speed the estimate divides by (the cumulative
 * measured speed). Parts are published every 25 media seconds. The estimate
 * appears once 20 s of transcript have run.
 */
function scriptedRun({ totalS, visualS, speedAt, claimed }) {
  const clock = { now: 0 };
  const calls = [];
  let done = 0;
  let last = visualS;
  const advance = (t) => {
    // Integrate the speed from `last` to t in 0.1 s steps.
    while (last < t - 1e-9) {
      const step = Math.min(0.1, t - last);
      if (last >= visualS && done < totalS) done = Math.min(totalS, done + speedAt(last) * step);
      last += step;
    }
  };
  const statusJson = (t) => {
    advance(t);
    const running = t >= visualS;
    const sealed = done >= totalS;
    const published = Math.floor(done / 25) * 25;
    const estimate = running && !sealed && t - visualS >= 20 ? { remaining_ms: Math.round(((totalS - done) / claimed) * 1000), basis: 'cumulative_measured_speed', fresh_chunks: 4 } : null;
    return {
      status: 'complete',
      data: {
        state: sealed ? 'succeeded' : 'running',
        failure: null,
        result: sealed ? { revision_id: 'trv_x', generation: 3 } : null,
        streams: [
          { stream: 'visual', state: running ? 'succeeded' : 'running' },
          { stream: 'transcript', state: running ? (sealed ? 'succeeded' : 'running') : 'not_started', stage: 'recognising_speech', chunks_done: Math.floor(done / 25), chunks_total: Math.ceil(totalS / 25), published_until_us: running ? published * SECOND : null,
            media: running ? { done_us: Math.round(done * SECOND), total_us: totalS * SECOND, elapsed_ms: Math.round((t - visualS) * 1000), speed: claimed, percent: Math.floor((done / totalS) * 100), estimate } : null },
        ],
      },
    };
  };
  const at = (step, args) => {
    calls.push([step, ...args]);
    clock.now += 0.05;
    if (step === 'job status') return { exit: 0, seconds: 0.05, json: statusJson(clock.now) };
    if (step === 'transcript get (first part)') {
      advance(clock.now);
      return { exit: 0, seconds: 0.05, json: { data: { items: done >= 25 ? [{ text: 'hello' }] : [] } } };
    }
    return { exit: 0, seconds: 0.05, json: { data: {} } };
  };
  return { at, calls, clock, sleep: (ms) => { clock.now += ms / 1000; }, clockFn: () => clock.now };
}

test('readStatus reads the transcript stream, the estimate and the result, and nothing from a non-answer', () => {
  const status = measure.readStatus({ data: { state: 'running', failure: null, result: null, streams: [{ stream: 'transcript', state: 'running', stage: 'recognising_speech', chunks_done: 3, chunks_total: 9, published_until_us: 75 * SECOND,
    media: { done_us: 90 * SECOND, total_us: 225 * SECOND, elapsed_ms: 40_000, speed: 2.25, percent: 40, estimate: { remaining_ms: 60_000, basis: 'cumulative_measured_speed', fresh_chunks: 3 } } }] } }, 41.5);
  assert.equal(status.t_s, 41.5);
  assert.equal(status.percent, 40);
  assert.equal(status.estimate_remaining_ms, 60_000);
  assert.equal(status.published_until_us, 75 * SECOND);
  assert.equal(measure.readStatus(null, 1), null);
  assert.equal(measure.readStatus({ data: {} }, 1), null);
  const early = measure.readStatus({ data: { state: 'running', streams: [] } }, 2);
  assert.equal(early.done_us, null);
  assert.equal(early.estimate_remaining_ms, null);
});

test('a watched run is followed to its seal: first part, parts, the moment of the seal, and an estimate that was right', () => {
  const script = scriptedRun({ totalS: 100, visualS: 5, speedAt: () => 2, claimed: 2 });
  const followed = measure.followWatched({ at: script.at, jobId: 'job_1', session: 'ses_1', durationUs: 100 * SECOND, clock: script.clockFn, sleep: script.sleep, startedAt: 0 });
  assert.equal(followed.end.state, 'succeeded');
  assert.equal(followed.end.revision_id, 'trv_x');
  // 100 media seconds at 2x take 50 s after the 5 s of the visual stream.
  assert.ok(Math.abs(followed.end.t_s - 55) < 1.5, `sealed at ${followed.end.t_s}`);
  // The first 25-second part is published 12.5 s into the transcript and read within the 2 s of the faster sampling.
  assert.ok(followed.first_part_s > 17.4 && followed.first_part_s < 20.5, `first part at ${followed.first_part_s}`);
  assert.deepEqual(followed.parts.map((part) => part.published_until_us / SECOND), [25, 50, 75, 100]);
  // It slowed to a second a poll near the end: the seal is seen within a second of it.
  const errors = measure.estimateErrors(followed.samples, followed.end.t_s);
  assert.deepEqual(errors.map((entry) => entry.point), [10, 25, 50, 75]);
  assert.equal(errors[0].estimate_s, null, 'at 10% (5 s in) the estimate has not appeared yet');
  for (const entry of errors.slice(1)) {
    assert.ok(Math.abs(entry.error_s) < 1.5, `error at ${entry.point}% is ${entry.error_s}`);
  }
});

test('an estimate that is too hopeful is a negative error, against the time that really remained', () => {
  // 2 media s/s for the first 50 s of media, then 1; the estimate always divides by 2.
  const script = scriptedRun({ totalS: 100, visualS: 5, speedAt: (t) => (t < 30 ? 2 : 1), claimed: 2 });
  const followed = measure.followWatched({ at: script.at, jobId: 'job_1', session: 'ses_1', durationUs: 100 * SECOND, clock: script.clockFn, sleep: script.sleep, startedAt: 0 });
  assert.equal(followed.end.state, 'succeeded');
  assert.ok(Math.abs(followed.end.t_s - 80) < 1.5, `sealed at ${followed.end.t_s}`);
  const errors = measure.estimateErrors(followed.samples, followed.end.t_s);
  const half = errors.find((entry) => entry.point === 50);
  // At 50% (t = 30 s): about 25 s said, about 50 s left.
  assert.ok(half.error_s < -20 && half.error_s > -30, `error ${half.error_s}`);
  assert.ok(half.error_percent < -40 && half.error_percent > -60, `error ${half.error_percent}%`);
  const seventyFive = errors.find((entry) => entry.point === 75);
  assert.ok(seventyFive.error_s < 0);
  assert.equal(measure.firstEstimate(followed.samples, measure.streamStart(followed.samples)).fresh_chunks, 4);
});

test('a point the run never reached, and a sample without an estimate, are reported as such', () => {
  const samples = [
    { t_s: 10, done_us: 10 * SECOND, total_us: 100 * SECOND, estimate_remaining_ms: null },
    { t_s: 20, done_us: 30 * SECOND, total_us: 100 * SECOND, estimate_remaining_ms: 70_000 },
  ];
  const errors = measure.estimateErrors(samples, 40);
  assert.equal(errors[0].estimate_s, null);
  assert.equal(errors[0].actual_remaining_s, 30);
  assert.equal(errors[1].error_s, 70 - 20);
  assert.equal(errors[1].error_percent, 250);
  assert.equal(errors[2].reached, false);
  assert.equal(measure.firstEstimate([{ t_s: 1, done_us: 0, total_us: 1, estimate_remaining_ms: null }], null), null);
});

test('a run that never ends is cancelled at the limit and reported as timed out; one that cannot be read three times as unreadable', () => {
  let now = 0;
  const calls = [];
  const at = (step) => {
    calls.push(step);
    now += 1;
    return { exit: 0, seconds: 1, json: { data: { state: 'running', streams: [] } } };
  };
  const timedOut = measure.followWatched({ at, jobId: 'job_1', session: 'ses_1', durationUs: SECOND, clock: () => now, sleep: (ms) => { now += ms / 1000; }, startedAt: 0, limitS: 30 });
  assert.equal(timedOut.end.state, 'timed_out');
  assert.equal(calls.at(-1), 'job cancel');
  let reads = 0;
  const unreadable = measure.followWatched({ at: () => { reads += 1; return { exit: 1, seconds: 0, json: null }; }, jobId: 'job_1', session: 'ses_1', durationUs: SECOND, clock: () => 0, sleep: () => {}, startedAt: 0 });
  assert.equal(unreadable.end.state, 'unreadable');
  assert.equal(reads, 3);
});

test('a job that fails ends the follow with its failure code', () => {
  const at = () => ({ exit: 0, seconds: 0, json: { data: { state: 'failed', failure: { code: 'DEPENDENCY_FAILED', retryable: false }, streams: [] } } });
  const followed = measure.followWatched({ at, jobId: 'job_1', session: 'ses_1', durationUs: SECOND, clock: () => 3, sleep: () => {}, startedAt: 0 });
  assert.deepEqual(followed.end, { state: 'failed', t_s: 3, revision_id: null, failure_code: 'DEPENDENCY_FAILED' });
});

test('the next look is two seconds until a part is readable, ten seconds after, one second in the last fifth', () => {
  assert.equal(measure.nextDelayMs({ done_us: 0, total_us: 100 }, false), measure.FIRST_PART_INTERVAL_MS);
  assert.equal(measure.nextDelayMs({ done_us: 50, total_us: 100 }, true), measure.SAMPLE_INTERVAL_MS);
  assert.equal(measure.nextDelayMs({ done_us: 81, total_us: 100 }, true), measure.END_INTERVAL_MS);
  assert.equal(measure.nextDelayMs(null, true), measure.SAMPLE_INTERVAL_MS);
});

// ---------------------------------------------------------------------------
// Two sealed revisions
// ---------------------------------------------------------------------------

const segment = (start, text, extra = {}) => ({ start_us: start * SECOND, end_us: (start + 4) * SECOND, text, ...extra });

test('two revisions with the same segments agree, and a carried segment is not counted', () => {
  const base = [segment(0, 'Hello there everyone'), segment(5, 'welcome to the talk')];
  const comparison = measure.compareRevisions(
    { segments: [...base, segment(9, 'carried over', { carried_from: { revision_id: 'trv_a' } })], threads: 4, revision_id: 'a' },
    { segments: base, threads: 3, revision_id: 'b' },
  );
  assert.equal(comparison.identical_segments, true);
  assert.equal(comparison.identical_text, true);
  assert.equal(comparison.quick_segments, 2);
  assert.equal(comparison.word_distance, 0);
  assert.deepEqual(comparison.cause_hints, []);
  assert.deepEqual([comparison.quick_threads, comparison.watched_threads], [4, 3]);
});

test('a difference is counted, placed against the chunk seams, and carries a hint, with the thread count that differs', () => {
  const quick = [segment(0, 'Hello there everyone'), segment(26, 'and welcome to the talk'), segment(60, 'thanks for coming')];
  const watched = [segment(0, 'Hello there everyone'), segment(26, 'and welcome to this talk'), segment(60, 'thanks for coming')];
  const comparison = measure.compareRevisions({ segments: quick, threads: 4, revision_id: 'a' }, { segments: watched, threads: 3, revision_id: 'b' });
  assert.equal(comparison.identical_segments, false);
  assert.equal(comparison.identical_text, false);
  assert.equal(comparison.only_in_quick, 1);
  assert.equal(comparison.only_in_watched, 1);
  assert.equal(comparison.word_distance, 1);
  assert.equal(comparison.listed.only_in_quick[0].seam_distance_s, 1);
  assert.match(comparison.cause_hints[0], /different threads \(4 and 3\)/);
  assert.match(comparison.cause_hints[1], /2 of 2 differing segments start within 5 s of a chunk seam/);
});

test('a long text is cut for the report, and the word distance is skipped beyond its bound', () => {
  const long = 'x'.repeat(500);
  const comparison = measure.compareRevisions({ segments: [segment(0, long)], threads: null, revision_id: 'a' }, { segments: [], threads: null, revision_id: 'b' });
  assert.equal(comparison.listed.only_in_quick[0].text.length, 120);
  assert.equal(measure.wordDistance(new Array(13_000).fill('a'), ['a']), null);
  assert.equal(measure.wordDistance(['a', 'b', 'c'], ['a', 'c', 'd']), 2);
  assert.equal(measure.seamDistanceS(24 * SECOND), 1);
});

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

test('the record and the summary carry every figure the brief asks for and nothing private', () => {
  const script = scriptedRun({ totalS: 100, visualS: 5, speedAt: () => 2, claimed: 2 });
  const followed = measure.followWatched({ at: script.at, jobId: 'job_1', session: 'ses_1', durationUs: 100 * SECOND, clock: script.clockFn, sleep: script.sleep, startedAt: 0 });
  const revision = { revision_id: 'trv_x', segment_count: 12, threads: 3 };
  const watched = measure.summariseRun({ mode: 'watched', label: 'RM03-first-5min', order: 'second', duration_us: 100 * SECOND, wall_s: null, revision, followed, final_status: followed.last, exit: 0 });
  const quick = measure.summariseRun({ mode: 'quick', label: 'RM03-first-5min', order: 'first', duration_us: 100 * SECOND, wall_s: 61.4, revision: { ...revision, threads: 4 }, followed: null, final_status: { elapsed_ms: 40_000 }, exit: 0 });
  assert.equal(quick.sealed_s, 61.4);
  assert.equal(quick.real_time_factor, 0.614);
  assert.equal(quick.transcript_elapsed_s, 40);
  assert.equal(watched.ended, 'succeeded');
  assert.equal(watched.estimate_errors.length, 4);
  assert.ok(watched.first_part_s > 17);
  assert.equal(watched.parts.length, 4);
  assert.ok(watched.samples.length > 5);
  const comparison = { label: 'RM03-first-5min', ...measure.compareRevisions({ segments: [segment(0, 'a b c')], threads: 4, revision_id: 'a' }, { segments: [segment(0, 'a b c')], threads: 3, revision_id: 'b' }) };
  const text = measure.markdown({ binary: { version: 'vsift 0.3.0' }, runner: { cpu_model: 'Test CPU', logical_cpus: 4 }, prefixes: [{ label: 'RM03-first-5min', method: 'stream_copy', duration_s: 300 }], runs: [quick, watched], comparisons: [comparison] });
  assert.match(text, /\| RM03-first-5min \| 1:40 \| quick \| first \| 1:01 \(61.4 s\) \| - \| 0.614 \| 4 \|/);
  assert.match(text, /\| RM03-first-5min \| 1:40 \| watched \| second \|/);
  assert.match(text, /not reached|no estimate yet|\d s \(/);
  assert.match(text, /cut by stream copy/);
  assert.match(text, /\| RM03-first-5min \| 1 \| 1 \| yes \| yes \| 0 \| 0 \| 0 \| 4, 3 \|/);
});

test('a run that failed shows how it ended and has no factor', () => {
  const failed = measure.summariseRun({ mode: 'quick', label: 'RM09', order: 'first', duration_us: 100 * SECOND, wall_s: 4, revision: null, followed: null, final_status: null, exit: 1, failure_code: 'DEPENDENCY_FAILED' });
  assert.equal(failed.ended, 'failed');
  assert.equal(failed.failure_code, 'DEPENDENCY_FAILED');
  const text = measure.markdown({ binary: { version: 'v' }, runner: { cpu_model: null, logical_cpus: 2 }, prefixes: [], runs: [failed], comparisons: [] });
  assert.match(text, /quick \(failed\)/);
});

// ---------------------------------------------------------------------------
// One run (hosted run 43035185042: every ingest failed with STORAGE_IO)
// ---------------------------------------------------------------------------

/**
 * A stand-in for `vsift` that behaves as VSift does about its session root: a
 * root that was made beforehand by anyone else (here: by `makeDirectory`) is
 * refused as `STORAGE_IO`, because it is not private to its owner.
 */
function standInVsift(made) {
  const calls = [];
  const run = (step, args, { root }) => {
    calls.push({ step, args, root });
    if (made.includes(root)) return { step, exit: 7, seconds: 5, json: { status: 'failed', error: { code: 'STORAGE_IO' }, data: null } };
    if (args[0] === 'ingest') return { step, exit: 0, seconds: 1, json: { data: { session_id: 'ses_1' } } };
    if (args[0] === 'job' && args[1] === 'start') {
      return { step, exit: 0, seconds: 30, json: { data: { job: { job_id: 'job_1' }, outcome: { revision: { revision_id: 'trv_q' } } } } };
    }
    if (args[0] === 'job' && args[1] === 'status') {
      return { step, exit: 0, seconds: 0.1, json: { data: { state: 'succeeded', result: { revision_id: 'trv_q' }, streams: [{ stream: 'transcript', state: 'succeeded', media: { done_us: 10 * SECOND, total_us: 10 * SECOND, elapsed_ms: 25_000 } }] } } };
    }
    return { step, exit: 1, seconds: 0, json: null };
  };
  return { run, calls };
}

test('a run never creates its session root, only the folder that holds it, so VSift can make the root private', () => {
  const made = [];
  const { run, calls } = standInVsift(made);
  const folder = path.join(ABSOLUTE, 'RM03', 'quick');
  const result = measure.measureRun({ run, log: () => {}, clock: () => 0, sleep: () => {}, makeDirectory: (directory) => made.push(directory),
    readRevision: () => ({ revision_id: 'trv_q', segment_count: 1, threads: 4, segments: [] }), entry: { label: 'RM03' }, mode: 'quick', order: 'first', folder, file: path.join(ABSOLUTE, 'RM03.webm'), durationUs: 10 * SECOND });
  assert.deepEqual(made, [folder]);
  assert.equal(calls[0].root, path.join(folder, 'root'));
  assert.equal(result.summary.ended, 'succeeded');
  assert.equal(result.summary.sealed_s, 30);
  assert.equal(result.summary.transcript_elapsed_s, 25);
  assert.equal(result.summary.threads, 4);
});

test('a failed ingest is a failed run with its code, never a silent success', () => {
  const folder = path.join(ABSOLUTE, 'RM03', 'watched');
  const root = path.join(folder, 'root');
  const { run } = standInVsift([root]);
  const result = measure.measureRun({ run, log: () => {}, clock: () => 0, sleep: () => {}, makeDirectory: () => {}, readRevision: () => null,
    entry: { label: 'RM03' }, mode: 'watched', order: 'second', folder, file: 'x', durationUs: SECOND });
  assert.equal(result.summary.ended, 'failed');
  assert.equal(result.summary.failure_code, 'STORAGE_IO');
  assert.equal(result.segments, null);
});
