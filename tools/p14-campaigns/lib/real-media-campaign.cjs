'use strict';

// The steps of the real-media campaign (#363, docs/planning/real-media-set.md
// section 7) over one item, and the reports. The command line is
// real-media-campaign.cjs; this module takes its `vsift` runner and its file
// operations as arguments, so the tests drive it with hand-made answers.
//
// A private item (the maintainer's own recordings, local use only, section 13)
// is investigated the same way, but nothing of its content may reach a report or
// a log line: every step label and finding is a fixed phrase with numbers and
// codes only, no listening clip is kept, and `privateReport` keeps measurements
// only, with no title, name, file name or text.

const fs = require('node:fs');
const path = require('node:path');
const checks = require('./real-media-checks.cjs');

const SECOND = 1_000_000;
const CANDIDATE_RANGE_US = 300 * SECOND;
/** A recording longer than this is not investigated: the campaign is for short real recordings. */
const MAX_DURATION_US = 4 * 60 * 60 * SECOND;

/**
 * The `setup check` part of a run: the configured tools must be available and
 * local ASR verified (#391's shape is a finding).
 * @param {(step: string, args: string[]) => object} run
 * @param {{ ffmpeg: string, ffprobe: string, whisper: string, model: string }} tools
 */
function setup(run, tools) {
  const answers = [
    run('setup configure ffmpeg', ['setup', 'configure', 'ffmpeg', '--executable', tools.ffmpeg]),
    run('setup configure ffprobe', ['setup', 'configure', 'ffprobe', '--executable', tools.ffprobe]),
    run('setup configure whisper', ['setup', 'configure', 'whisper', '--executable', tools.whisper]),
    run('setup configure-model', ['setup', 'configure-model', '--file', tools.model]),
  ];
  const check = run('setup check', ['setup', 'check']);
  answers.push(check);
  const findings = answers.map((answer) => checks.failureFinding(answer)).filter(Boolean);
  const report = check.json || {};
  for (const dependency of report.dependencies || []) {
    if (dependency.status !== 'available') findings.push(`setup check calls ${dependency.dependency} ${dependency.status} (#391's shape)`);
  }
  if (report.status !== 'ready') findings.push(`setup check status is ${report.status}`);
  const verification = report.local_asr && report.local_asr.verification;
  if (!verification || verification.status !== 'verified') findings.push(`local ASR verification is ${verification ? `${verification.status} (${verification.reason})` : 'missing'}`);
  return {
    passed: findings.length === 0,
    findings,
    seconds: check.seconds,
    status: report.status || null,
    dependencies: (report.dependencies || []).map((dependency) => ({ dependency: dependency.dependency, status: dependency.status, detail: dependency.detail })),
    local_asr: report.local_asr || null,
  };
}

/** Every segment of one revision over the whole video, page by page, and the coverage it states. */
function allSegments(at, session, revision, durationUs) {
  const segments = [];
  let cursor = null;
  let first = null;
  let pages = 0;
  let seconds = 0;
  do {
    const args = ['transcript', 'get', session, '--from', '0', '--to', String(durationUs), '--limit', '100', '--revision', revision];
    if (cursor) args.push('--cursor', cursor);
    const answer = at(`transcript get (page ${pages + 1})`, args);
    pages += 1;
    seconds += answer.seconds;
    if (!first) first = answer;
    if (answer.exit !== 0 || !answer.json || !answer.json.data) return { segments, first, failed: answer, pages, seconds };
    segments.push(...answer.json.data.items);
    cursor = answer.json.data.next_cursor;
  } while (cursor && pages < 2000);
  return { segments, first, failed: null, pages, seconds };
}

/** The gap reasons a later call can close (cli-v1.md, P08 candidates "Coverage"); the others are final. */
const RETRY_REASONS = new Set(['not_analyzed', 'deadline_exceeded']);

/**
 * Candidates over the whole video in five-minute ranges, each range read
 * again while it reports windows not analysed or timed out. The duration is
 * learned from the index of the first answer (a private item has none in its
 * manifest). Final gaps (a window whose changes exceeded its candidate budget,
 * an undecodable stretch) are returned with the candidates they dropped.
 */
function candidatesOver(at, session, knownDurationUs) {
  const items = [];
  const answers = [];
  const finals = [];
  let gaps = [];
  let index = null;
  let durationUs = knownDurationUs || CANDIDATE_RANGE_US;
  for (let from = 0; from < durationUs; from += CANDIDATE_RANGE_US) {
    for (let round = 0; round < 10; round += 1) {
      const before = items.length;
      const to = Math.min(durationUs, from + CANDIDATE_RANGE_US);
      let cursor = null;
      let data = null;
      do {
        const args = ['candidates', session, '--from', String(from), '--to', String(to), '--limit', '100', '--omit', 'periodic'];
        if (cursor) args.push('--cursor', cursor);
        const answer = at(`candidates ${from / SECOND}-${to / SECOND} s`, args);
        answers.push(answer);
        if (answer.exit !== 0 || !answer.json || !answer.json.data) return { items, answers, gaps: [{ from_us: from, to_us: to }], finals, index, durationUs };
        data = answer.json.data;
        if (data.index) {
          index = data.index;
          if (Number.isSafeInteger(index.duration_us) && index.duration_us > 0) durationUs = Math.min(index.duration_us, MAX_DURATION_US);
        }
        items.push(...data.items);
        cursor = data.next_cursor;
      } while (cursor);
      const all = (data.coverage && data.coverage.gaps) || [];
      gaps = all.filter((gap) => RETRY_REASONS.has(gap.reason));
      if (gaps.length === 0) {
        finals.push(...all);
        break;
      }
      items.length = before;
    }
    if (gaps.length > 0) break;
  }
  return { items, answers, gaps, finals, index, durationUs };
}

function phraseFrom(segment) {
  const list = checks.words(segment.text).filter((word) => /^\p{L}+$/u.test(word));
  return list.length >= 3 ? list.slice(0, 3).join(' ') : null;
}

function summariseRevision(answer) {
  const revision = answer && answer.json && answer.json.data && answer.json.data.revision;
  if (!revision) return null;
  const asr = revision.local_asr || {};
  const unplaced = asr.unplaced_ranges || [];
  return {
    revision_id: revision.revision_id,
    segment_count: revision.segment_count,
    carried_segment_count: revision.carried_segment_count,
    language: revision.language,
    chunk_count: asr.chunk_count,
    transcribed_chunks: asr.transcribed_chunks,
    silent_chunks: asr.silent_chunks,
    no_audio_chunks: asr.no_audio_chunks,
    unplaced_ranges: unplaced.length,
    unplaced_us: checks.mergeRanges(unplaced).reduce((total, range) => total + range.to_us - range.from_us, 0),
    warnings: (revision.warnings || []).map((warning) => ({ code: warning.code, count: warning.count === undefined ? null : warning.count })),
    status: answer.json.status,
  };
}

/**
 * Where the shifted run starts: the start of a segment of the whole run whose
 * offset within the 25 s chunk step is at least 7.5 s from a step boundary, the
 * earliest such start within the first ten minutes (so most of the recording
 * is compared); failing that, the start nearest the middle of a step. Null
 * when the video is too short or has no segment in reach.
 * @param {Array<{ start_us: number }>} segments
 * @param {number} durationUs
 */
function shiftPoint(segments, durationUs) {
  const step = 25 * SECOND;
  const options = segments.map((segment) => segment.start_us)
    .filter((start) => start >= 5 * SECOND && start <= 600 * SECOND && start < durationUs - 30 * SECOND)
    .sort((a, b) => a - b);
  if (options.length === 0) return null;
  const distance = (start) => Math.abs((start % step) - step / 2);
  const good = options.find((start) => distance(start) <= 5 * SECOND);
  if (good !== undefined) return good;
  return options.reduce((best, start) => (distance(start) < distance(best) ? start : best));
}

const rangeLength = (ranges) => checks.mergeRanges(ranges || []).reduce((total, range) => total + range.to_us - range.from_us, 0);

/**
 * Runs every step of the campaign over one item and returns its report.
 * @param {object} item a manifest item (public or private)
 * @param {string} file the verified media file
 * @param {(step: string, args: string[], extra?: object) => { step: string, exit: number | null, seconds: number, json: object | null }} run
 * @param {{ folder: string, copyFile?: (from: string, to: string) => void, exists?: (file: string) => boolean }} where
 */
function investigate(item, file, run, where) {
  const isPrivate = item.private === true;
  const copyFile = where.copyFile || fs.copyFileSync;
  const writeFile = where.writeFile || fs.writeFileSync;
  const exists = where.exists || fs.existsSync;
  const root = path.join(where.folder, 'root');
  const at = (step, args, extra) => run(step, args, { ...extra, root });
  const answers = [];
  const record = (answer) => {
    answers.push(answer);
    return answer;
  };
  const report = isPrivate
    ? { id: item.id, private: true, covers: item.covers, checks: {}, measurements: {} }
    : { id: item.id, title: item.title, attribution: item.attribution, covers: item.covers, tests: item.tests, checks: {}, measurements: {} };

  const ingest = record(at('ingest', ['ingest', file]));
  const session = ingest.json && ingest.json.data && ingest.json.data.session_id;
  report.measurements.ingest_seconds = ingest.seconds;
  if (!session) {
    report.checks.no_failure = { passed: false, findings: [checks.failureFinding(ingest) || 'ingest gave no session'] };
    return report;
  }

  const known = item.media && typeof item.media.duration_s === 'number' ? Math.ceil(item.media.duration_s * SECOND) : null;
  const visual = candidatesOver(at, session, known);
  answers.push(...visual.answers);
  const durationUs = visual.durationUs;
  report.measurements.duration_us = durationUs;
  const minutes = durationUs / (60 * SECOND);
  report.measurements.candidates = {
    count: visual.items.length,
    per_minute: minutes > 0 ? visual.items.length / minutes : null,
    calls: visual.answers.length,
    seconds: visual.answers.reduce((total, answer) => total + answer.seconds, 0),
    reasons: visual.items.reduce((counts, candidate) => {
      for (const reason of candidate.reasons || []) counts[reason] = (counts[reason] || 0) + 1;
      return counts;
    }, {}),
    // Final gaps are VSift's own honest report (L-152), measured, not findings.
    final_gaps: visual.finals.reduce((counts, gap) => {
      counts[gap.reason] = (counts[gap.reason] || 0) + 1;
      return counts;
    }, {}),
    dropped_candidates: visual.finals.reduce((total, gap) => total + (gap.dropped_candidates || 0), 0),
  };
  const visualFindings = [];
  if (visual.gaps.length > 0) visualFindings.push(`candidates left ${visual.gaps.length} range(s) not analysed or timed out after ten rounds`);
  const labels = item.labels || {};
  if (Array.isArray(labels.changes_us) && labels.changes_us.length > 0) {
    const hit = labels.changes_us.filter((change) => visual.items.some((candidate) => Math.abs(candidate.representative_us - change) <= SECOND));
    report.measurements.labelled_change_recall = hit.length / labels.changes_us.length;
  } else {
    report.measurements.labelled_change_recall = null;
  }
  report.checks.candidates = { passed: visualFindings.length === 0, findings: visualFindings };

  const first = record(at('transcript retranscribe (whole)', ['transcript', 'retranscribe', session]));
  const revisionA = summariseRevision(first);
  report.measurements.transcription = revisionA ? { ...revisionA, seconds: first.seconds, real_time_factor: first.seconds / (durationUs / SECOND) } : null;

  if (revisionA) {
    const a = allSegments(at, session, revisionA.revision_id, durationUs);
    answers.push(a.first, ...(a.failed ? [a.failed] : []));
    // A range transcription is widened to the segments of the revision it
    // replaces, so the shifted run starts at a segment boundary of the whole run:
    // the one nearest the middle of a chunk step, which puts each new seam far
    // from an old one.
    const shiftUs = shiftPoint(a.segments, durationUs);
    report.measurements.shift_us = shiftUs;
    const shifted = shiftUs !== null
      ? record(at('transcript retranscribe (seams shifted)', ['transcript', 'retranscribe', session, '--from', String(shiftUs), '--to', String(durationUs)]))
      : null;
    const revisionB = summariseRevision(shifted);
    report.measurements.shifted_transcription = revisionB ? { ...revisionB, seconds: shifted.seconds } : null;
    report.measurements.transcript_get = { pages: a.pages, seconds: a.seconds, segments: a.segments.length };
    const coverage = a.first && a.first.json && a.first.json.data ? a.first.json.data.transcript_coverage : null;
    const coverageFindings = checks.coverageProblems(coverage, a.first && a.first.json ? a.first.json.status : null);
    report.measurements.coverage = coverage ? {
      basis: coverage.basis,
      transcribed_us: rangeLength(coverage.transcribed_ranges),
      untranscribed_us: rangeLength(coverage.untranscribed_ranges),
      no_speech_us: rangeLength(coverage.no_speech_ranges),
    } : null;
    if (a.segments.length === 0) coverageFindings.push('the transcription of a recording with speech holds no segment');
    report.checks.coverage = { passed: coverageFindings.length === 0, findings: coverageFindings };

    const speechStart = a.segments.length > 0 ? Math.min(...a.segments.map((segment) => segment.start_us)) : null;
    report.measurements.first_segment_start_us = speechStart;
    report.measurements.speech_start_error_us = Number.isInteger(labels.speech_start_us) && speechStart !== null ? speechStart - labels.speech_start_us : null;
    const repeats = checks.seamRepeats(a.segments, durationUs);
    // The repeated words themselves are content: only their times and count are kept.
    report.measurements.seam_repeats = { seams: repeats.seams, count: repeats.repeats.length, per_hour: repeats.per_hour, at_us: repeats.repeats.map((repeat) => repeat.at_us) };

    if (revisionB) {
      const b = allSegments(at, session, revisionB.revision_id, durationUs);
      answers.push(b.first, ...(b.failed ? [b.failed] : []));
      const compared = { from_us: shiftUs, to_us: durationUs };
      const seamsOfA = checks.seamComparison(a.segments, b.segments, compared);
      const seamsOfB = checks.seamComparison(b.segments, a.segments, compared);
      report.measurements.seams_of_whole_run = seamsOfA;
      report.measurements.seams_of_shifted_run = seamsOfB;
      report.measurements.timing_differences = checks.timingDifferences(a.segments, b.segments);
      // Two plans with the same seams compare nothing: the check is then not run, and says so.
      const startsA = new Set(checks.seams(checks.chunkPlan(a.segments)).map((seam) => seam.from_us));
      const seamsB = checks.seams(checks.chunkPlan(b.segments));
      const shared = seamsB.filter((seam) => startsA.has(seam.from_us)).length;
      report.measurements.shift_achieved = seamsB.length > 0 && shared < seamsB.length / 2;
      report.checks.seams = report.measurements.shift_achieved
        ? { passed: seamsOfA.passed && seamsOfB.passed, findings: [...seamsOfA.findings, ...seamsOfB.findings] }
        : { passed: true, findings: [], not_run: 'the shifted run kept the chunk plan of the whole run' };
      const flips = checks.languageFlips(seamsOfA, seamsOfB);
      report.measurements.language_flips = { count: flips.count, at_us: flips.at_us };
      if (report.measurements.shift_achieved) report.checks.language = { passed: flips.passed, findings: flips.findings };
      // A public item's two revisions are kept beside its run for whoever checks a
      // finding (local only, never uploaded); a private item's text is never written.
      if (!isPrivate) {
        const slim = (list) => list.map((segment) => ({ start_us: segment.start_us, end_us: segment.end_us, chunk: segment.alignment ? segment.alignment.chunk_index : null,
          carried: Boolean(segment.carried_from), text: segment.text }));
        writeFile(path.join(where.folder, 'revisions.json'), `${JSON.stringify({ whole: slim(a.segments), shifted: slim(b.segments) }, null, 1)}
`);
        report.measurements.revisions_file = `${item.id}/revisions.json`;
      }

      const worst = [...seamsOfA.lost_runs, ...seamsOfB.lost_runs].sort((x, y) => y.words - x.words)[0];
      const firstSeam = checks.seams(checks.chunkPlan(a.segments))[0];
      const listenAt = worst ? worst.from_us : (firstSeam ? firstSeam.from_us : null);
      if (listenAt !== null) {
        const from = Math.max(0, listenAt - 2 * SECOND);
        const clip = record(at('audio (around a seam)', ['audio', session, '--from', String(from), '--to', String(Math.min(durationUs, listenAt + 4 * SECOND))]));
        const kept = clip.json && clip.json.data && clip.json.data.files && clip.json.data.files[0];
        // A private item's audio never leaves its session: no clip is kept for it.
        if (!isPrivate && kept && exists(kept.path)) {
          const listen = path.join(where.folder, 'listen');
          fs.mkdirSync(listen, { recursive: true });
          copyFile(kept.path, path.join(listen, `${item.id}-${from}us.wav`));
          report.measurements.listening_clip = `${item.id}/listen/${item.id}-${from}us.wav`;
        }
      }
    }

    const searchFindings = [];
    const seamPlan = checks.seams(checks.chunkPlan(a.segments));
    const atSeam = (segment) => seamPlan.some((seam) => segment.start_us < seam.to_us && segment.end_us > seam.from_us);
    const picks = [...a.segments.filter(atSeam).slice(0, 3), ...a.segments.filter((segment) => !atSeam(segment)).slice(0, 3)];
    const searches = [];
    let number = 0;
    for (const segment of picks) {
      const phrase = phraseFrom(segment);
      if (!phrase) continue;
      number += 1;
      // The phrase is the recording's own words: a private item's label and findings never carry it.
      const label = isPrivate ? `search ${number}` : `search "${phrase}"`;
      const answer = record(at(label, ['search', session, '--query', phrase, '--revision', revisionA.revision_id, '--limit', '100']));
      const data = answer.json && answer.json.data;
      const found = Boolean(data && data.items.some((hit) => hit.segment_id === segment.segment_id));
      if (!found) searchFindings.push(`${label} did not return the segment that holds its words`);
      if (data) searchFindings.push(...checks.coverageProblems(data.transcript_coverage, answer.json.status).map((problem) => `${label}: ${problem}`));
      searches.push({ at_seam: atSeam(segment), found, seconds: answer.seconds });
    }
    report.measurements.searches = searches;
    report.checks.search = { passed: searchFindings.length === 0, findings: searchFindings };

    if (a.segments.length > 0) {
      const citation = a.segments[Math.floor(a.segments.length / 2)];
      const draft = `## Problem\n\nThe speaker says something at this moment [c1: e1].\n\n\`\`\`vsift-handoff\n${JSON.stringify({
        handoff_version: '1', status: 'complete', question: 'What is said at this moment?',
        capabilities: { image_access: 'unavailable' },
        claims: [{ id: 'c1', section: 'actual', kind: 'observed', support: 'supported', certainty: 'medium', statement: 'The speaker says something here.', citations: ['e1'] }],
        citations: [{ id: 'e1', type: 'transcript_segment', segment_id: citation.segment_id }],
        gaps: [{ kind: 'image_access', reason: 'image_access_unavailable', note: null }],
        untrusted_instructions: [],
        lifecycle: { action: 'left_open' },
      })}\n\`\`\`\n`;
      const handoff = record(at('handoff check', ['handoff', 'check', '--session', session], { input: draft }));
      const valid = Boolean(handoff.json && handoff.json.data && handoff.json.data.valid);
      report.checks.handoff = { passed: valid, findings: valid ? [] : ['a mechanical draft citing a segment of the run is not valid'] };
    }
  }

  const frameFindings = [];
  const chosen = visual.items.length > 0 ? [...new Set([0, Math.floor(visual.items.length / 2), visual.items.length - 1])].map((index) => visual.items[index]) : [];
  let cropParent = null;
  for (const candidate of chosen) {
    const frame = record(at('frame get', ['frame', 'get', session, '--candidate', candidate.candidate_id]));
    const selection = frame.json && frame.json.data && frame.json.data.selections && frame.json.data.selections[0];
    if (!selection) {
      frameFindings.push(`frame get of a candidate gave no selection${frame.json && frame.json.error ? ` (${frame.json.error.code})` : ''}`);
      continue;
    }
    if (selection.actual_us !== candidate.representative_us) frameFindings.push(`frame of candidate at ${candidate.representative_us} us is at ${selection.actual_us} us`);
    cropParent = cropParent || frame.json.data.items[0];
  }
  if (cropParent) {
    const width = Math.min(480, cropParent.frame.width);
    const height = Math.min(120, cropParent.frame.height);
    const crop = record(at('crop', ['crop', session, cropParent.evidence_id, '--rect', `0,0,${width},${height}`]));
    const image = crop.json && crop.json.data && crop.json.data.items && crop.json.data.items[0] && crop.json.data.items[0].image;
    if (!image) frameFindings.push('crop gave no image');
    else if (image.width !== width || image.height !== height) frameFindings.push(`crop of ${width}x${height} came back ${image.width}x${image.height}, not at native size`);
    report.measurements.crop = image ? { width: image.width, height: image.height } : null;
  }
  report.checks.frames = { passed: frameFindings.length === 0, findings: frameFindings };

  record(at('session close', ['session', 'close', session]));
  // A closed session is only eligible for removal (cli-v1.md; L-009): cleaning
  // it removes VSift's copy of the recording from the work folder, which matters
  // above all for a private item.
  record(at('session clean', ['session', 'clean', '--expired']));
  const failures = answers.filter(Boolean).map((answer) => checks.failureFinding(answer)).filter(Boolean);
  report.checks.no_failure = { passed: failures.length === 0, findings: failures };
  report.measurements.steps = answers.filter(Boolean).map((answer) => ({ step: answer.step, exit: answer.exit, seconds: Number(answer.seconds.toFixed(2)), status: answer.json ? answer.json.status : null, code: answer.json && answer.json.error ? answer.json.error.code : null }));
  return report;
}

/** The string members a private report may keep: VSift's own codes and enumerations, never content. */
const PRIVATE_STRING_KEYS = new Set(['id', 'status', 'code', 'basis', 'language', 'step']);
const PRIVATE_STRING = /^[A-Za-z0-9_ ().:-]{1,80}$/;

function measurementsOnly(value, key) {
  if (value === null || typeof value === 'number' || typeof value === 'boolean') return value;
  if (typeof value === 'string') return PRIVATE_STRING_KEYS.has(key) && PRIVATE_STRING.test(value) ? value : undefined;
  if (Array.isArray(value)) return value.map((entry) => measurementsOnly(entry, key)).filter((entry) => entry !== undefined);
  if (typeof value === 'object') {
    const out = {};
    for (const [name, entry] of Object.entries(value)) {
      // Revision ids identify a session's stored text; they are left out too.
      if (name === 'revision_id') continue;
      const kept = measurementsOnly(entry, name);
      if (kept !== undefined) out[name] = kept;
    }
    return out;
  }
  return undefined;
}

/**
 * A private item's report: its id (P01, P02, ...), the verdict and the
 * measurements, with every string dropped that is not one of VSift's codes or
 * enumerations; a finding keeps only its check's name, so no phrase, file
 * name or name can reach the report even if a check's wording changes.
 * @param {object} report from `investigate`
 */
function privateReport(report) {
  const verdict = checks.itemVerdict(report.checks);
  const failed = Object.entries(report.checks).filter(([, check]) => !check.passed).map(([name, check]) => ({ check: name, findings: (check.findings || []).length }));
  return {
    id: report.id,
    private: true,
    verdict: { passed: verdict.passed, failed_checks: failed },
    failure_codes: [...new Set(((report.measurements && report.measurements.steps) || []).map((step) => step.code).filter(Boolean))],
    measurements: measurementsOnly(report.measurements, 'measurements'),
  };
}

/**
 * Whether an item's file may be investigated: `ready` when it is there with its
 * pinned size and SHA-256 (or, for an item not yet pinned in the manifest, the
 * checksum its source publishes, which the hosted cross-check verified it
 * against; a private item with no SHA-256 by size alone); `missing` when it is
 * not there; `differs` otherwise. A missing public item in a run over whatever
 * the test-media release holds is "not yet mirrored", not a failure.
 * @param {object} item
 * @param {{ exists: boolean, size: number | null, digest: (algorithm: string) => string }} file
 * @returns {'ready' | 'missing' | 'differs'}
 */
function mediaState(item, file) {
  if (!file.exists) return 'missing';
  if (file.size !== item.bytes) return 'differs';
  if (item.sha256) return file.digest('sha256') === item.sha256 ? 'ready' : 'differs';
  if (item.source_checksum) return file.digest(item.source_checksum.algorithm) === item.source_checksum.value ? 'ready' : 'differs';
  return item.private ? 'ready' : 'differs';
}

function markdown(run) {
  const lines = ['# Real-media campaign', '', `- Binary: ${run.binary.version} (SHA-256 \`${run.binary.sha256}\`)`,
    `- Started ${run.started}, finished ${run.finished}; ${run.platform}`, `- Setup: ${run.setup.passed ? 'passed' : `**failed**: ${run.setup.findings.join('; ')}`}`, '',
    '| Item | Verdict | Transcription (RTF) | Seam recall / control | Repeats at seams | Timing diff p95 | Candidates per min | Findings |',
    '| --- | --- | --- | --- | --- | --- | --- | --- |'];
  const fmt = (value) => (typeof value === 'number' ? value.toFixed(3) : 'n/a');
  for (const item of run.items) {
    if (item.not_yet_mirrored) {
      lines.push(`| ${item.id} | not yet mirrored | n/a | n/a | n/a | n/a | n/a | the test-media release does not hold it yet (not a failure) |`);
      continue;
    }
    const m = item.measurements || {};
    const t = m.transcription;
    const s = m.seams_of_whole_run;
    const findings = item.private
      ? (item.verdict.failed_checks.length === 0 ? 'none' : item.verdict.failed_checks.map((entry) => `${entry.check} (${entry.findings})`).join('<br>'))
      : (item.verdict.findings.length === 0 ? 'none' : item.verdict.findings.join('<br>'));
    lines.push(`| ${item.id} | ${item.verdict.passed ? 'pass' : '**fail**'} | ${t ? `${t.seconds.toFixed(0)} s (${t.real_time_factor.toFixed(2)})` : 'n/a'} | ${s ? `${fmt(s.seam_recall)} / ${fmt(s.control_recall)}` : 'n/a'} | ${m.seam_repeats ? m.seam_repeats.count : 'n/a'} | ${m.timing_differences && m.timing_differences.start.p95_us !== null ? `${(m.timing_differences.start.p95_us / 1000).toFixed(0)} ms` : 'n/a'} | ${m.candidates ? fmt(m.candidates.per_minute) : 'n/a'} | ${findings} |`);
  }
  const attributed = run.items.filter((item) => !item.private && !item.not_yet_mirrored);
  if (attributed.length > 0) {
    lines.push('', '## Attribution', '');
    for (const item of attributed) lines.push(`- ${item.id}: ${item.attribution}`);
  }
  if (run.items.some((item) => item.private)) lines.push('', 'Private items are reported by their measurements only (local use, never committed or uploaded).');
  lines.push('', 'Advisory evidence (decided 2026-10-18): a finding becomes an issue after a person has listened or looked; it is not a gate.', '');
  return lines.join('\n');
}

module.exports = { SECOND, setup, investigate, privateReport, markdown, measurementsOnly, shiftPoint, mediaState };
