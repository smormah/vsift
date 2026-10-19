'use strict';

// The property checks of the real-media campaign (#363), on hand-made
// answers: two chunk plans shifted against each other, a seam that loses a
// sentence, a seam that repeats one, and coverage statements that do and do
// not add up. No VSift binary and no media are needed.

const assert = require('node:assert/strict');
const test = require('node:test');
const checks = require('../lib/real-media-checks.cjs');

const SECOND = 1_000_000;
const SENTENCES = [
  'the quick brown fox jumps over the lazy dog',
  'a stitch in time saves nine every single morning',
  'every cloud has a silver lining above the hills',
  'the early bird catches the worm before breakfast today',
  'actions speak louder than words in every meeting',
  'better late than never said the patient teacher',
];

/** Five-second segments of distinct sentences, one per five seconds, over `seconds`. */
function speech(seconds) {
  const out = [];
  for (let start = 0, n = 0; start + 5 <= seconds; start += 5, n += 1) {
    out.push({ start_us: start * SECOND, end_us: (start + 5) * SECOND, text: `${SENTENCES[n % SENTENCES.length]} number ${n}` });
  }
  return out;
}

/** The segments as one run with chunks of 30 s every 25 s from `origin` heard them. */
function run(segments, origin, prefix) {
  return segments.map((segment, index) => {
    const chunk = Math.max(0, Math.floor((segment.start_us - origin) / (25 * SECOND)));
    return {
      ...segment,
      segment_id: `${prefix}${index}`,
      carried_from: segment.start_us < origin ? { revision_id: 'trv_a', segment_id: `a${index}` } : null,
      alignment: { origin: 'local_asr', chunk_index: chunk, chunk_audio_start_us: origin + chunk * 25 * SECOND, chunk_audio_end_us: origin + chunk * 25 * SECOND + 30 * SECOND },
    };
  });
}

test('the chunk plan and its seams are read from the segments themselves', () => {
  const plan = checks.chunkPlan(run(speech(120), 0, 'a'));
  assert.deepEqual(plan.map((chunk) => chunk.from_us), [0, 25, 50, 75, 100].map((s) => s * SECOND));
  assert.deepEqual(checks.seams(plan).map((seam) => [seam.from_us / SECOND, seam.to_us / SECOND]), [[25, 30], [50, 55], [75, 80], [100, 105]]);
  const shifted = checks.chunkPlan(run(speech(120), 15 * SECOND, 'b'));
  assert.equal(shifted[0].from_us, 15 * SECOND, 'carried segments belong to the other plan');
});

test('two runs that agree have full recall at seams and away from them, and no finding', () => {
  const all = speech(300);
  const result = checks.seamComparison(run(all, 0, 'a'), run(all, 15 * SECOND, 'b'), { from_us: 15 * SECOND, to_us: 300 * SECOND });
  assert.ok(result.seams >= 9);
  assert.equal(result.seam_recall, 1);
  assert.equal(result.control_recall, 1);
  assert.ok(result.control_stretches >= 5, `${result.control_stretches} control stretches`);
  assert.deepEqual(result.findings, []);
  assert.equal(result.passed, true);
});

test('a sentence the merge dropped at a seam is a finding, with its time', () => {
  const all = speech(300);
  const lossy = all.filter((segment) => segment.start_us !== 50 * SECOND);
  const result = checks.seamComparison(run(lossy, 0, 'a'), run(all, 15 * SECOND, 'b'), { from_us: 15 * SECOND, to_us: 300 * SECOND });
  assert.equal(result.passed, false);
  assert.ok(result.lost_runs.length >= 1);
  const [worst] = result.lost_runs;
  assert.ok(worst.words >= checks.LOST_RUN_FINDING_WORDS);
  assert.ok(worst.from_us >= 50 * SECOND && worst.to_us <= 55 * SECOND, `${worst.from_us}-${worst.to_us}`);
  assert.ok(result.findings.some((finding) => /missing at the seam 50000000-55000000/.test(finding)), result.findings.join('; '));
});

test('the same loss away from any seam lowers the control, not the seam recall', () => {
  const all = speech(300);
  const lossy = all.filter((segment) => segment.start_us !== 40 * SECOND);
  const result = checks.seamComparison(run(lossy, 0, 'a'), run(all, 15 * SECOND, 'b'), { from_us: 15 * SECOND, to_us: 300 * SECOND });
  assert.equal(result.seam_recall, 1);
  assert.ok(result.control_recall < 1);
  assert.equal(result.lost_runs.length, 0);
});

test('words repeated across the two chunks of a seam are counted, and a repeat inside one segment is not', () => {
  const segments = run(speech(60), 0, 'a');
  const repeat = { ...segments[4], segment_id: 'copy', start_us: 25 * SECOND, end_us: 27 * SECOND, alignment: { ...segments[6].alignment } };
  const result = checks.seamRepeats([...segments, repeat], 60 * SECOND);
  assert.ok(result.seams >= 1);
  assert.ok(result.repeats.length >= 1);
  assert.ok(result.per_hour > 0);
  assert.equal(checks.seamRepeats(segments, 60 * SECOND).repeats.length, 0);
});

test('timing differences pair fresh segments of the same text and count those beyond the tolerance', () => {
  const a = run(speech(60), 0, 'a');
  const b = run(speech(60), 15 * SECOND, 'b').map((segment, index) => (segment.carried_from ? segment
    : { ...segment, start_us: segment.start_us + (index % 2 === 0 ? 100_000 : 700_000), end_us: segment.end_us + 200_000 }));
  const result = checks.timingDifferences(a, b);
  assert.equal(result.matched, b.filter((segment) => !segment.carried_from).length);
  assert.equal(result.start.max_us, 700_000);
  assert.ok(result.start.over_tolerance >= 1);
  assert.equal(result.end.max_us, 200_000);
});

test('seam joins count the gaps between segments of two chunks at a seam against the gaps of one chunk (#381)', () => {
  const segments = run(speech(120), 0, 'a');
  // A seam join (chunk 0's last to chunk 1's first, at 25 s) with a gap of 0.3 s,
  // and a gap of 0.2 s between two segments of chunk 1 away from any seam (at 40 s).
  const gapped = segments.map((segment) => {
    if (segment.start_us === 25 * SECOND) return { ...segment, start_us: 25_300_000 };
    if (segment.start_us === 40 * SECOND) return { ...segment, start_us: 40_200_000 };
    return segment;
  });
  const result = checks.seamJoins(gapped);
  assert.equal(result.seam_joins.bins.up_to_500_ms, 1);
  assert.equal(result.control_joins.bins.up_to_250_ms, 1);
  assert.ok(result.seam_joins.count >= 4, `${result.seam_joins.count}`);
  assert.ok(result.seam_joins.within_tolerance_share > 0 && result.seam_joins.within_tolerance_share < 1);
  // Touching segments are no gap, overlaps are their own bin, and carried text is left out.
  const plain = checks.seamJoins(segments);
  assert.equal(plain.seam_joins.within_tolerance_share, 0);
  assert.equal(plain.seam_joins.bins.touch, plain.seam_joins.count);
  const overlapping = checks.seamJoins(segments.map((segment) => (segment.start_us === 50 * SECOND ? { ...segment, start_us: 49 * SECOND } : segment)));
  assert.equal(overlapping.seam_joins.bins.overlap, 1);
});

test('a coverage statement that adds up passes, and each way of not adding up is named', () => {
  const searched = { from_us: 0, to_us: 100 };
  const good = { searched_range: searched, transcribed_ranges: [{ from_us: 0, to_us: 60 }], untranscribed_ranges: [{ from_us: 60, to_us: 100 }], no_speech_ranges: [{ from_us: 10, to_us: 20 }], ranges_truncated: false };
  assert.deepEqual(checks.coverageProblems(good, 'partial'), []);
  assert.deepEqual(checks.coverageProblems({ ...good, untranscribed_ranges: [], transcribed_ranges: [{ from_us: 0, to_us: 100 }] }, 'complete'), []);
  const named = (coverage, status, pattern) => assert.ok(checks.coverageProblems(coverage, status).some((p) => pattern.test(p)), String(pattern));
  named(good, 'complete', /status complete but a gap/);
  named({ ...good, untranscribed_ranges: [{ from_us: 50, to_us: 100 }] }, 'partial', /both transcribed and untranscribed/);
  named({ ...good, no_speech_ranges: [{ from_us: 70, to_us: 80 }] }, 'partial', /no-speech range lies outside/);
  named({ ...good, untranscribed_ranges: [{ from_us: 70, to_us: 100 }] }, 'partial', /cover 90 of the 100/);
  assert.deepEqual(checks.coverageProblems({ ...good, untranscribed_ranges: [{ from_us: 70, to_us: 100 }], ranges_truncated: true }, 'partial'), []);
  named(null, 'complete', /no transcript_coverage/);
});

test('a failure on valid media is a finding of its own kind', () => {
  assert.equal(checks.failureFinding({ step: 'ingest', exit: 0, json: { error: null } }), null);
  assert.match(checks.failureFinding({ step: 'transcript retranscribe', exit: 2, json: { error: { code: 'MISSING_CAPABILITY' } } }), /MISSING_CAPABILITY on valid media/);
  assert.match(checks.failureFinding({ step: 'candidates', exit: 4, json: { error: { code: 'BUSY' } } }), /failed with BUSY/);
  assert.match(checks.failureFinding({ step: 'frame get', exit: null, json: null }), /no JSON error/);
});

test('an item passes only when every check passes, and its findings are named by check', () => {
  assert.deepEqual(checks.itemVerdict({ a: { passed: true, findings: [] } }), { passed: true, findings: [] });
  const verdict = checks.itemVerdict({ a: { passed: true, findings: [] }, seams: { passed: false, findings: ['lost'] } });
  assert.equal(verdict.passed, false);
  assert.deepEqual(verdict.findings, ['seams: lost']);
});

test('words are compared without case or punctuation, and accents are kept', () => {
  assert.deepEqual(checks.words('Olá, MUNDO! E-409 já'), ['olá', 'mundo', 'e', '409', 'já']);
});

test('a missing run the recogniser explains is classified, and only a plain loss is a finding', () => {
  const all = speech(300);
  // The reference loops on one phrase across the seam at 50-55 s, where the subject heard nothing.
  const loop = all.map((segment) => (segment.start_us === 50 * SECOND
    ? { ...segment, text: 'we are going to go over here we are going to go over here we are going to go over here' } : segment));
  const subject = all.filter((segment) => segment.start_us !== 50 * SECOND);
  const looped = checks.seamComparison(run(subject, 0, 'a'), run(loop, 15 * SECOND, 'b'), { from_us: 15 * SECOND, to_us: 300 * SECOND });
  assert.equal(looped.lost_runs[0].kind, 'repetition');
  assert.equal(looped.passed, true);
  // The subject heard the same stretch as Cyrillic text: a language flip, not a loss.
  const flipped = all.map((segment) => (segment.start_us === 50 * SECOND ? { ...segment, text: 'я уверен что всё будет хорошо и мы увидимся снова' } : segment));
  const result = checks.seamComparison(run(flipped, 0, 'a'), run(all, 15 * SECOND, 'b'), { from_us: 15 * SECOND, to_us: 300 * SECOND });
  assert.equal(result.lost_runs[0].kind, 'script');
  assert.equal(result.passed, true);
  const flips = checks.languageFlips(result);
  assert.equal(flips.count, 1);
  assert.equal(flips.passed, false);
  assert.equal(checks.script([{ word: 'хорошо' }, { word: 'ok' }]), 'cyrillic');
});

test('a labelled change counts when a change window or a motion span brackets it, not merely a nearby candidate (#175)', () => {
  const candidates = [
    { representative_us: 10 * SECOND, reasons: ['visual_change'], change_window: { from_us: 9.5 * SECOND, to_us: 10 * SECOND }, span: { from_us: 10 * SECOND, to_us: 12 * SECOND } },
    { representative_us: 12 * SECOND, reasons: ['motion_start'], change_window: { from_us: 11.5 * SECOND, to_us: 12 * SECOND }, span: { from_us: 12 * SECOND, to_us: 15 * SECOND } },
    { representative_us: 30 * SECOND, reasons: ['first_frame'], change_window: null, span: { from_us: 30 * SECOND, to_us: 40 * SECOND } },
  ];
  const result = checks.labelledChanges([
    9.2 * SECOND, // inside the window widened by half a second
    13.5 * SECOND, // inside the motion, 1.5 s from its candidate
    14 * SECOND, // inside the motion
    30.6 * SECOND, // within a second of a candidate, but no change of it holds the label
    50 * SECOND, // nothing near
  ], candidates, 60 * SECOND);
  assert.equal(result.labelled, 5);
  assert.equal(result.bracketed, 3);
  assert.equal(result.bracketed_share, 0.6);
  // The loose rule counts 9.2 s (by the 10 s candidate) and 30.6 s, a moment no change holds.
  assert.equal(result.within_1s, 2);
  assert.equal(result.within_1s_share, 0.4);
  // Seconds 9 to 13 and 29 to 31 are within a second of a candidate: 6 of 60.
  assert.equal(result.chance_within_1s, 0.1);
  const none = checks.labelledChanges([], candidates, 60 * SECOND);
  assert.equal(none.bracketed_share, null);
  assert.equal(none.chance_within_1s, 0.1);
});
