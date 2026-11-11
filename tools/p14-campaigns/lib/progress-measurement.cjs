'use strict';

// The measurement of ADR 0033 section 6 and section 9 on real recordings
// (0.3.0 readiness item 5, brief W8): where the quick/watched threshold
// should sit, how far the progress estimate (D8) is wrong, and whether the
// quick run's sealed revision and the watched run's agree. The command line is
// progress-measurement.cjs; this module is pure or takes its `vsift` runner,
// its clock, its sleep and its process spawner as arguments, so the tests
// drive it with scripted answers and a fake clock and need no binary, no
// media and no recogniser.
//
// Public items only: the plan names items of the approved public manifest,
// `parseArguments` refuses `--private-manifest`, and `refusePrivate` refuses
// any item marked private. Nothing here reads a person's identity.

const SECOND = 1_000_000;
/** The chunk step of the recogniser's plan: a 30 s window, 5 s of it overlapping the next (`local_asr.window_us` and `overlap_us`). */
const CHUNK_STEP_US = 25 * SECOND;
const SEAM_OVERLAP_US = 5 * SECOND;

/** Prefix lengths of the one long item that bracket the ten-minute threshold, in minutes. */
const PREFIX_MINUTES = Object.freeze([5, 10, 15]);
/** How done a watched run is when its estimate is read against what remained (brief W8, part 3), in percent. */
const ESTIMATE_POINTS = Object.freeze([10, 25, 50, 75]);

/** `job status` is sampled this often (brief W8, part 3). */
const SAMPLE_INTERVAL_MS = 10_000;
/** Until the first part is readable the run is sampled faster, so its time is found to a couple of seconds. */
const FIRST_PART_INTERVAL_MS = 2_000;
/** Near the end the run is sampled every second, so the moment of the seal is found to about a second. */
const END_INTERVAL_MS = 1_000;
/**
 * From this fraction of the transcript done the run is sampled at
 * `END_INTERVAL_MS`: a ten-second step from just below it cannot overshoot the
 * seal of a run whose last fifth is longer than ten seconds, and the shortest
 * prefix's last fifth is about a quarter of a minute.
 */
const END_FRACTION = 0.8;
/** A watched run that is not sealed after this long is cancelled and reported as timed out. */
const WATCHED_LIMIT_S = 3 * 3600;

/** Differences between two revisions that are listed in full; the rest are counted. */
const MAX_LISTED_DIFFERENCES = 10;
/** A listed difference shows at most this many characters of each side's text (items are public). */
const MAX_DIFFERENCE_TEXT = 120;
/** The word-level distance is not computed beyond this many words a side (it is quadratic). */
const MAX_WORDS_FOR_DISTANCE = 12_000;

class ProgressMeasurementError extends Error {
  constructor(message) {
    super(message);
    this.name = 'ProgressMeasurementError';
  }
}

// ---------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------

/**
 * The options of `progress-measurement.cjs`. Refuses `--private-manifest`
 * (a private item never runs here: the measurement's artifact is read by
 * people who must not meet a maintainer's own recording) and any unknown
 * option; every path is absolute.
 * @param {string[]} argv
 */
function parseArguments(argv) {
  const options = {
    items: [],
    prefixOf: null,
    prefixMinutes: [...PREFIX_MINUTES],
    manifest: null,
  };
  const names = { '--vsift': 'vsift', '--media': 'media', '--work': 'work', '--report': 'report', '--ffmpeg': 'ffmpeg', '--ffprobe': 'ffprobe',
    '--whisper': 'whisper', '--model': 'model', '--manifest': 'manifest', '--prefix-of': 'prefixOf' };
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    if (flag === '--private-manifest') throw new ProgressMeasurementError('--private-manifest is refused: this measurement runs on public items only');
    const value = argv[index + 1];
    if (value === undefined) throw new ProgressMeasurementError(`${flag} needs a value`);
    if (flag === '--item') options.items.push(value);
    else if (flag === '--prefix-minutes') options.prefixMinutes = parseMinutes(value);
    else if (names[flag]) options[names[flag]] = value;
    else throw new ProgressMeasurementError(`unknown option ${flag}`);
  }
  for (const key of ['vsift', 'media', 'work', 'report', 'ffmpeg', 'ffprobe', 'whisper', 'model']) {
    if (!options[key] || !/^([A-Za-z]:)?[\\/]/.test(options[key])) throw new ProgressMeasurementError(`--${key} must be an absolute path`);
  }
  for (const id of [...options.items, ...(options.prefixOf ? [options.prefixOf] : [])]) {
    if (!/^RM\d{2}$/.test(id)) throw new ProgressMeasurementError(`${id} is not a public item id (RM followed by two digits)`);
  }
  if (options.items.length === 0) throw new ProgressMeasurementError('name at least one item with --item');
  return options;
}

function parseMinutes(text) {
  const minutes = String(text).split(',').map((part) => Number(part.trim()));
  if (minutes.length === 0 || minutes.some((value) => !Number.isInteger(value) || value < 1 || value > 60)) {
    throw new ProgressMeasurementError('--prefix-minutes is a comma-separated list of whole minutes from 1 to 60');
  }
  return minutes;
}

/**
 * Throws for an item that is private. The measurement's report and artifact
 * carry public item text and timings only.
 * @param {object} item
 */
function refusePrivate(item) {
  if (item.private === true) throw new ProgressMeasurementError(`${item.id} is a private item: this measurement runs on public items only`);
}

// ---------------------------------------------------------------------------
// The plan: which media, which modes, in which order
// ---------------------------------------------------------------------------

/**
 * The media of the measurement, in the order they run: the prefixes of
 * `prefixOf` (when the item is longer than each), then every whole item.
 * Each entry runs both modes; the order of the two alternates from one entry
 * to the next, so a cold cache or a runner that slows over the hour does not
 * favour one mode.
 * @param {Array<{ id: string, media: { duration_s: number } }>} items the approved public items, in the order asked for
 * @param {string | null} prefixOf the item to cut prefixes of
 * @param {number[]} minutes prefix lengths in minutes
 * @returns {Array<{ label: string, item_id: string, kind: 'prefix' | 'whole', minutes: number | null, modes: Array<'quick' | 'watched'> }>}
 */
function planMedia(items, prefixOf, minutes) {
  const entries = [];
  if (prefixOf) {
    const source = items.find((item) => item.id === prefixOf);
    if (!source) throw new ProgressMeasurementError(`${prefixOf} is not among the items to run`);
    for (const length of [...minutes].sort((a, b) => a - b)) {
      if (length * 60 < source.media.duration_s - 30) entries.push({ label: `${prefixOf}-first-${length}min`, item_id: prefixOf, kind: 'prefix', minutes: length });
    }
  }
  for (const item of items) entries.push({ label: item.id, item_id: item.id, kind: 'whole', minutes: null });
  return entries.map((entry, index) => ({ ...entry, modes: index % 2 === 0 ? ['quick', 'watched'] : ['watched', 'quick'] }));
}

/**
 * The FFmpeg arguments that keep the first `seconds` of a file: by stream
 * copy, which changes no byte of the audio the recogniser hears.
 * @param {string} source
 * @param {string} destination
 * @param {number} seconds
 * @param {boolean} reencode when stream copy did not give the length, the video is re-encoded once with FFmpeg's own `mpeg4` encoder (always built in) and the audio is still copied
 */
function cutArguments(source, destination, seconds, reencode) {
  return ['-nostdin', '-v', 'error', '-y', '-i', source, '-t', String(seconds), '-map', '0:v:0', '-map', '0:a:0',
    ...(reencode ? ['-c:v', 'mpeg4', '-q:v', '5', '-c:a', 'copy'] : ['-c', 'copy']), destination];
}

/**
 * Cuts the first `seconds` of `source` into `destination` on this machine
 * (the hosted runner): by stream copy when the container allows it, which is
 * checked by reading the cut's duration back; otherwise re-encoded once.
 * @param {{ ffmpeg: string, ffprobe: string, source: string, destination: string, seconds: number,
 *   spawn: (program: string, args: string[]) => { status: number | null, stdout: string }, remove?: (file: string) => void }} cut
 * @returns {{ method: 'stream_copy' | 're_encoded', duration_s: number }}
 */
function cutPrefix(cut) {
  const attempt = (reencode) => {
    const made = cut.spawn(cut.ffmpeg, cutArguments(cut.source, cut.destination, cut.seconds, reencode));
    if (made.status !== 0) return null;
    const probed = cut.spawn(cut.ffprobe, ['-v', 'error', '-show_entries', 'format=duration', '-of', 'default=noprint_wrappers=1:nokey=1', cut.destination]);
    const duration = probed.status === 0 ? Number(String(probed.stdout).trim()) : Number.NaN;
    // The cut ends at a packet: within two seconds of what was asked for is a prefix of that length.
    return Number.isFinite(duration) && Math.abs(duration - cut.seconds) <= 2 ? duration : null;
  };
  const copied = attempt(false);
  if (copied !== null) return { method: 'stream_copy', duration_s: copied };
  if (cut.remove) cut.remove(cut.destination);
  const encoded = attempt(true);
  if (encoded === null) throw new ProgressMeasurementError(`the first ${cut.seconds} s could not be cut by stream copy or by one re-encode`);
  return { method: 're_encoded', duration_s: encoded };
}

// ---------------------------------------------------------------------------
// Reading `job status`
// ---------------------------------------------------------------------------

/**
 * What one `job status` answer says, in plain numbers: the job's state, and
 * the `transcript` stream's progress.
 * @param {object | null} json the answer's JSON
 * @param {number} t_s seconds since the run started, when the answer came back
 */
function readStatus(json, t_s) {
  const data = json && json.data;
  if (!data || typeof data.state !== 'string') return null;
  const stream = (data.streams || []).find((entry) => entry.stream === 'transcript') || null;
  const visual = (data.streams || []).find((entry) => entry.stream === 'visual') || null;
  const media = (stream && stream.media) || null;
  const estimate = media && media.estimate ? media.estimate : null;
  return {
    t_s,
    job_state: data.state,
    visual_state: visual ? visual.state : null,
    stream_state: stream ? stream.state : null,
    stage: stream ? stream.stage : null,
    chunks_done: stream && Number.isInteger(stream.chunks_done) ? stream.chunks_done : null,
    chunks_total: stream && Number.isInteger(stream.chunks_total) ? stream.chunks_total : null,
    published_until_us: stream && Number.isInteger(stream.published_until_us) ? stream.published_until_us : null,
    done_us: media && Number.isInteger(media.done_us) ? media.done_us : null,
    total_us: media && Number.isInteger(media.total_us) ? media.total_us : null,
    percent: media && Number.isInteger(media.percent) ? media.percent : null,
    elapsed_ms: media && Number.isInteger(media.elapsed_ms) ? media.elapsed_ms : null,
    speed: media && typeof media.speed === 'number' ? media.speed : null,
    estimate_remaining_ms: estimate && Number.isInteger(estimate.remaining_ms) ? estimate.remaining_ms : null,
    fresh_chunks: estimate && Number.isInteger(estimate.fresh_chunks) ? estimate.fresh_chunks : null,
    result_revision_id: data.result && typeof data.result.revision_id === 'string' ? data.result.revision_id : null,
    failure_code: data.failure && typeof data.failure.code === 'string' ? data.failure.code : null,
  };
}

const ENDED_STATES = new Set(['succeeded', 'failed', 'cancelled', 'interrupted']);

/** How long to wait before the next look, from the last status and whether a part is readable yet. */
function nextDelayMs(status, partSeen) {
  if (!partSeen) return FIRST_PART_INTERVAL_MS;
  if (status && status.done_us !== null && status.total_us && status.done_us / status.total_us >= END_FRACTION) return END_INTERVAL_MS;
  return SAMPLE_INTERVAL_MS;
}

/**
 * Follows a started watched job to its end: `job status` about every ten
 * seconds, faster until the first part is readable and again near the end.
 * Everything but the sampling is arithmetic over what it kept.
 * @param {{ at: (step: string, args: string[]) => { exit: number | null, seconds: number, json: object | null },
 *   jobId: string, session: string, durationUs: number, clock: () => number, sleep: (ms: number) => void,
 *   startedAt: number, limitS?: number }} follow
 */
function followWatched(follow) {
  const { at, jobId, session, durationUs, clock, sleep, startedAt } = follow;
  const limitS = follow.limitS === undefined ? WATCHED_LIMIT_S : follow.limitS;
  const samples = [];
  const parts = [];
  let firstPartS = null;
  let end = null;
  let unreadable = 0;
  let last = null;
  for (;;) {
    const answer = at('job status', ['job', 'status', jobId]);
    const now = clock() - startedAt;
    const status = readStatus(answer.json, now);
    if (status) {
      unreadable = 0;
      last = status;
      samples.push(status);
      const published = status.published_until_us;
      if (published !== null && published > 0 && (parts.length === 0 || parts[parts.length - 1].published_until_us < published)) {
        parts.push({ published_until_us: published, first_seen_s: now });
      }
      if (ENDED_STATES.has(status.job_state)) {
        end = { state: status.job_state, t_s: now, revision_id: status.result_revision_id, failure_code: status.failure_code };
        break;
      }
      if (firstPartS === null && published !== null && published > 0) {
        const read = at('transcript get (first part)', ['transcript', 'get', session, '--from', '0', '--to', String(durationUs), '--limit', '1']);
        const items = read.json && read.json.data && Array.isArray(read.json.data.items) ? read.json.data.items : [];
        if (read.exit === 0 && items.length > 0) firstPartS = clock() - startedAt;
      }
    } else {
      unreadable += 1;
      if (unreadable >= 3) {
        end = { state: 'unreadable', t_s: now, revision_id: null, failure_code: null };
        break;
      }
    }
    if (now > limitS) {
      at('job cancel', ['job', 'cancel', jobId]);
      end = { state: 'timed_out', t_s: now, revision_id: null, failure_code: null };
      break;
    }
    sleep(nextDelayMs(last, firstPartS !== null));
  }
  return { samples, parts, first_part_s: firstPartS, end, last };
}

// ---------------------------------------------------------------------------
// The estimate's error
// ---------------------------------------------------------------------------

/**
 * The estimate's error when the run was `points` percent done, against the
 * time that really remained to the seal (`sealedS`). The sample read is the
 * first at or after each point (a sample every ten seconds, so the point is
 * reached within that); a sample without an estimate there is reported as
 * having none. Error is the estimate minus the actual remainder: negative is
 * optimistic (the run took longer than said).
 * @param {Array<object>} samples from `readStatus`
 * @param {number} sealedS seconds from the start to the observed seal
 * @param {number[]} [points]
 */
function estimateErrors(samples, sealedS, points = ESTIMATE_POINTS) {
  return points.map((point) => {
    const sample = samples.find((entry) => entry.done_us !== null && entry.total_us && (entry.done_us * 100) / entry.total_us >= point);
    if (!sample) return { point, reached: false };
    const actual = sealedS - sample.t_s;
    const done = (sample.done_us * 100) / sample.total_us;
    if (sample.estimate_remaining_ms === null) return { point, reached: true, at_s: sample.t_s, done_percent: done, estimate_s: null, actual_remaining_s: actual, error_s: null, error_percent: null };
    const estimate = sample.estimate_remaining_ms / 1000;
    return {
      point,
      reached: true,
      at_s: sample.t_s,
      done_percent: done,
      estimate_s: estimate,
      actual_remaining_s: actual,
      error_s: estimate - actual,
      error_percent: actual > 0 ? ((estimate - actual) / actual) * 100 : null,
    };
  });
}

/**
 * When the estimate first appeared: seconds from the start of the transcript
 * stream, and how far the run was. `null` when it never did.
 * @param {Array<object>} samples
 * @param {number | null} streamStartS when the transcript stream was first seen running
 */
function firstEstimate(samples, streamStartS) {
  const sample = samples.find((entry) => entry.estimate_remaining_ms !== null);
  if (!sample) return null;
  return {
    t_s: sample.t_s,
    after_stream_start_s: streamStartS === null ? null : sample.t_s - streamStartS,
    done_percent: sample.done_us !== null && sample.total_us ? (sample.done_us * 100) / sample.total_us : null,
    fresh_chunks: sample.fresh_chunks,
  };
}

/** The moment the transcript stream was first seen running or ahead (the visual stream's end, to the sampling's resolution). */
function streamStart(samples) {
  const sample = samples.find((entry) => entry.stream_state === 'running' || entry.stream_state === 'succeeded');
  return sample ? sample.t_s : null;
}

// ---------------------------------------------------------------------------
// The two sealed revisions
// ---------------------------------------------------------------------------

function normaliseWords(text) {
  return String(text).toLowerCase().match(/[\p{L}\p{N}]+/gu) || [];
}

/** Word-level edit distance (insertions, deletions, substitutions), or `null` beyond `MAX_WORDS_FOR_DISTANCE` words. */
function wordDistance(left, right) {
  if (left.length > MAX_WORDS_FOR_DISTANCE || right.length > MAX_WORDS_FOR_DISTANCE) return null;
  let previous = Array.from({ length: right.length + 1 }, (_, index) => index);
  for (let row = 1; row <= left.length; row += 1) {
    const current = [row];
    for (let column = 1; column <= right.length; column += 1) {
      const substitution = previous[column - 1] + (left[row - 1] === right[column - 1] ? 0 : 1);
      current.push(Math.min(substitution, previous[column] + 1, current[column - 1] + 1));
    }
    previous = current;
  }
  return previous[right.length];
}

/** Where a segment starts against the chunk plan: how far from the nearest seam (a multiple of the chunk step), in seconds. */
function seamDistanceS(startUs) {
  const offset = startUs % CHUNK_STEP_US;
  return Math.min(offset, CHUNK_STEP_US - offset) / SECOND;
}

function keyOf(segment) {
  return `${segment.start_us}|${segment.end_us}|${segment.text}`;
}

/**
 * The quick run's sealed revision against the watched run's (ADR 0033
 * section 9): segment counts, whether the segments are the same, and, when
 * they are not, the segments only one side holds with how near a chunk seam
 * each starts, and the word-level distance between the two texts. A cause is
 * named only as a hint (the threads each recognised with, near a seam or not);
 * the reader settles it.
 * @param {{ segments: object[], threads: number | null, revision_id: string | null }} quick
 * @param {{ segments: object[], threads: number | null, revision_id: string | null }} watched
 */
function compareRevisions(quick, watched) {
  const own = (side) => side.segments.filter((segment) => !segment.carried_from);
  const left = own(quick);
  const right = own(watched);
  const rightKeys = new Map();
  for (const segment of right) rightKeys.set(keyOf(segment), (rightKeys.get(keyOf(segment)) || 0) + 1);
  const onlyQuick = [];
  for (const segment of left) {
    const key = keyOf(segment);
    const count = rightKeys.get(key) || 0;
    if (count > 0) rightKeys.set(key, count - 1);
    else onlyQuick.push(segment);
  }
  const leftKeys = new Map();
  for (const segment of left) leftKeys.set(keyOf(segment), (leftKeys.get(keyOf(segment)) || 0) + 1);
  const onlyWatched = [];
  for (const segment of right) {
    const key = keyOf(segment);
    const count = leftKeys.get(key) || 0;
    if (count > 0) leftKeys.set(key, count - 1);
    else onlyWatched.push(segment);
  }
  const identicalSegments = onlyQuick.length === 0 && onlyWatched.length === 0;
  const quickWords = normaliseWords(left.map((segment) => segment.text).join(' '));
  const watchedWords = normaliseWords(right.map((segment) => segment.text).join(' '));
  const identicalText = quickWords.length === watchedWords.length && quickWords.every((word, index) => word === watchedWords[index]);
  const describe = (segment) => ({
    start_us: segment.start_us,
    end_us: segment.end_us,
    seam_distance_s: seamDistanceS(segment.start_us),
    text: String(segment.text).slice(0, MAX_DIFFERENCE_TEXT),
  });
  const nearSeam = (segment) => seamDistanceS(segment.start_us) <= SEAM_OVERLAP_US / SECOND;
  const differing = [...onlyQuick, ...onlyWatched];
  const threadsDiffer = quick.threads !== null && watched.threads !== null && quick.threads !== watched.threads;
  const hints = [];
  if (!identicalSegments) {
    if (threadsDiffer) hints.push(`the runs recognised with different threads (${quick.threads} and ${watched.threads}), which is part of a revision's provenance (ADR 0033 section 5)`);
    const near = differing.filter(nearSeam).length;
    hints.push(`${near} of ${differing.length} differing segments start within ${SEAM_OVERLAP_US / SECOND} s of a chunk seam`);
  }
  return {
    quick_segments: left.length,
    watched_segments: right.length,
    quick_threads: quick.threads,
    watched_threads: watched.threads,
    identical_segments: identicalSegments,
    identical_text: identicalText,
    only_in_quick: onlyQuick.length,
    only_in_watched: onlyWatched.length,
    word_distance: identicalText ? 0 : wordDistance(quickWords, watchedWords),
    quick_words: quickWords.length,
    watched_words: watchedWords.length,
    listed: {
      only_in_quick: onlyQuick.slice(0, MAX_LISTED_DIFFERENCES).map(describe),
      only_in_watched: onlyWatched.slice(0, MAX_LISTED_DIFFERENCES).map(describe),
    },
    cause_hints: hints,
  };
}

// ---------------------------------------------------------------------------
// One run, and the report
// ---------------------------------------------------------------------------

const round = (value, digits = 1) => (value === null || value === undefined ? null : Number(value.toFixed(digits)));

/**
 * The figures of one finished run for the report, whichever its mode.
 * @param {{ mode: string, label: string, duration_us: number, wall_s: number, revision: object | null, followed: object | null, final_status: object | null }} run
 */
function summariseRun(run) {
  const durationS = run.duration_us / SECOND;
  const followed = run.followed;
  const sealedS = run.mode === 'watched' ? (followed && followed.end && followed.end.state === 'succeeded' ? followed.end.t_s : null) : run.wall_s;
  const out = {
    label: run.label,
    mode: run.mode,
    order: run.order,
    media_s: round(durationS, 1),
    sealed_s: round(sealedS, 1),
    real_time_factor: sealedS === null ? null : round(sealedS / durationS, 3),
    threads: run.revision ? run.revision.threads : null,
    revision_id: run.revision ? run.revision.revision_id : null,
    segments: run.revision ? run.revision.segment_count : null,
    ended: run.mode === 'watched' ? (followed && followed.end ? followed.end.state : null) : (run.exit === 0 ? 'succeeded' : 'failed'),
    failure_code: run.mode === 'watched' ? (followed && followed.end ? followed.end.failure_code : null) : (run.failure_code || null),
    // The transcript stream's own elapsed wall time, read from the last status (this attempt's, admission waits included).
    transcript_elapsed_s: run.final_status && run.final_status.elapsed_ms !== null ? round(run.final_status.elapsed_ms / 1000, 1) : null,
  };
  if (run.mode === 'watched' && followed) {
    const start = streamStart(followed.samples);
    out.first_part_s = round(followed.first_part_s, 1);
    out.transcript_started_s = round(start, 1);
    out.first_part_after_start_s = followed.first_part_s !== null && start !== null ? round(followed.first_part_s - start, 1) : null;
    out.parts = followed.parts.map((part) => ({ published_until_s: round(part.published_until_us / SECOND, 1), first_seen_s: round(part.first_seen_s, 1) }));
    out.sample_count = followed.samples.length;
    out.sample_resolution_s = { normal: SAMPLE_INTERVAL_MS / 1000, before_first_part: FIRST_PART_INTERVAL_MS / 1000, near_end: END_INTERVAL_MS / 1000 };
    if (sealedS !== null) {
      out.estimate_errors = estimateErrors(followed.samples, sealedS).map((entry) => ({
        ...entry,
        at_s: round(entry.at_s, 1),
        done_percent: round(entry.done_percent, 1),
        estimate_s: round(entry.estimate_s, 1),
        actual_remaining_s: round(entry.actual_remaining_s, 1),
        error_s: round(entry.error_s, 1),
        error_percent: round(entry.error_percent, 1),
      }));
      out.first_estimate = firstEstimate(followed.samples, start);
      if (out.first_estimate) {
        out.first_estimate = { ...out.first_estimate, t_s: round(out.first_estimate.t_s, 1), after_stream_start_s: round(out.first_estimate.after_stream_start_s, 1), done_percent: round(out.first_estimate.done_percent, 1) };
      }
    }
    // Every sample, compact, so the planner can read the curve itself.
    out.samples = followed.samples.map((sample) => [round(sample.t_s, 1), sample.done_us, sample.percent, sample.speed, sample.estimate_remaining_ms, sample.published_until_us]);
  }
  return out;
}

const minutesText = (seconds) => (seconds === null || seconds === undefined ? 'n/a' : `${Math.floor(seconds / 60)}:${String(Math.round(seconds % 60)).padStart(2, '0')}`);
const secondsText = (value) => (value === null || value === undefined ? 'n/a' : `${value}`);
const errorCell = (entry) => {
  if (!entry || !entry.reached) return 'not reached';
  if (entry.error_s === null) return 'no estimate yet';
  const sign = entry.error_s > 0 ? '+' : '';
  return `${sign}${entry.error_s} s (${entry.error_percent === null ? 'n/a' : `${entry.error_percent > 0 ? '+' : ''}${entry.error_percent}%`})`;
};

/**
 * The short Markdown summary of the measurement.
 * @param {{ runner: object, binary: object, started: string, finished: string, prefixes: object[], runs: object[], comparisons: object[] }} record
 */
function markdown(record) {
  const lines = [];
  lines.push('## Quick against watched, and the estimate (ADR 0033 sections 6 and 9)');
  lines.push('');
  lines.push(`vsift ${record.binary.version}; runner: ${record.runner.cpu_model || 'unknown CPU'}, ${record.runner.logical_cpus} logical CPUs.`);
  for (const prefix of record.prefixes) lines.push(`Prefix ${prefix.label}: cut by ${prefix.method === 'stream_copy' ? 'stream copy' : 're-encoding the video once with FFmpeg\'s mpeg4 encoder (the audio copied)'}, ${prefix.duration_s} s.`);
  lines.push('');
  lines.push('Each run is a fresh session on its own root; `job start --mode quick|watched` on the same runner and model. Wall time is to the sealed revision and includes the visual stream the job runs first in both modes. A negative error is an optimistic estimate.');
  lines.push('');
  lines.push('| Media | Length | Mode | Order | Sealed after | First part | Real-time factor | Threads | Estimate error at 10% | 25% | 50% | 75% |');
  lines.push('| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |');
  for (const run of record.runs) {
    const errors = run.estimate_errors || [];
    const at = (point) => (run.mode === 'watched' ? errorCell(errors.find((entry) => entry.point === point)) : '-');
    lines.push(`| ${run.label} | ${minutesText(run.media_s)} | ${run.mode}${run.ended === 'succeeded' ? '' : ` (${run.ended})`} | ${run.order} | ${run.sealed_s === null ? 'n/a' : `${minutesText(run.sealed_s)} (${run.sealed_s} s)`} | ${run.mode === 'watched' ? (run.first_part_s === null ? 'n/a' : `${run.first_part_s} s`) : '-'} | ${secondsText(run.real_time_factor)} | ${secondsText(run.threads)} | ${at(10)} | ${at(25)} | ${at(50)} | ${at(75)} |`);
  }
  lines.push('');
  lines.push('| Watched run | Transcript started | First part after that | Estimate first shown | at % done | Parts seen | Samples |');
  lines.push('| --- | --- | --- | --- | --- | --- | --- |');
  for (const run of record.runs.filter((entry) => entry.mode === 'watched')) {
    lines.push(`| ${run.label} | ${secondsText(run.transcript_started_s)} s | ${secondsText(run.first_part_after_start_s)} s | ${run.first_estimate ? `${run.first_estimate.t_s} s (${secondsText(run.first_estimate.after_stream_start_s)} s into the transcript)` : 'never'} | ${run.first_estimate ? secondsText(run.first_estimate.done_percent) : 'n/a'} | ${run.parts ? run.parts.length : 0} | ${run.sample_count} |`);
  }
  lines.push('');
  lines.push('| Media | Quick segments | Watched segments | Same segments | Same text | Only quick | Only watched | Word distance | Threads (quick, watched) |');
  lines.push('| --- | --- | --- | --- | --- | --- | --- | --- | --- |');
  for (const comparison of record.comparisons) {
    lines.push(`| ${comparison.label} | ${comparison.quick_segments} | ${comparison.watched_segments} | ${comparison.identical_segments ? 'yes' : 'no'} | ${comparison.identical_text ? 'yes' : 'no'} | ${comparison.only_in_quick} | ${comparison.only_in_watched} | ${comparison.word_distance === null ? 'n/a' : comparison.word_distance} | ${comparison.quick_threads}, ${comparison.watched_threads} |`);
  }
  for (const comparison of record.comparisons.filter((entry) => !entry.identical_segments)) {
    lines.push('');
    lines.push(`**${comparison.label}:** ${comparison.cause_hints.join('; ')}.`);
  }
  lines.push('');
  return `${lines.join('\n')}\n`;
}

module.exports = {
  SECOND,
  PREFIX_MINUTES,
  ESTIMATE_POINTS,
  SAMPLE_INTERVAL_MS,
  FIRST_PART_INTERVAL_MS,
  END_INTERVAL_MS,
  WATCHED_LIMIT_S,
  ProgressMeasurementError,
  parseArguments,
  refusePrivate,
  planMedia,
  cutArguments,
  cutPrefix,
  readStatus,
  nextDelayMs,
  followWatched,
  estimateErrors,
  firstEstimate,
  streamStart,
  compareRevisions,
  seamDistanceS,
  wordDistance,
  summariseRun,
  markdown,
};
