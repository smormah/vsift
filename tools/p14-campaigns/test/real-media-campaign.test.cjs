'use strict';

// The campaign's steps over one item, driven by a stand-in `vsift` that answers
// with hand-made JSON (no binary, no media). Above all: a private item's words,
// file name and names never reach its report, its summary or a log line, and no
// clip of its audio is kept.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const campaign = require('../lib/real-media-campaign.cjs');

const SECOND = 1_000_000;
const SECRETS = ['zebrafinch', 'Quillmorton', 'Alice', 'Hartwell', 'Secret Meeting', 'paid-tutorial', 'invoice'];
const FILE = path.join(os.tmpdir(), 'private recordings', 'Secret Meeting With Alice Hartwell.mp4');

/** Two chunk plans' segments of the same speech, every sentence carrying private words. */
function segments(origin, revision) {
  const out = [];
  for (let start = 0, n = 0; start + 5 <= 120; start += 5, n += 1) {
    const chunk = Math.max(0, Math.floor((start * SECOND - origin) / (25 * SECOND)));
    out.push({
      segment_id: `${revision}-${n}`,
      start_us: start * SECOND,
      end_us: (start + 5) * SECOND,
      text: `Alice Hartwell says zebrafinch Quillmorton invoice number ${n} again today`,
      carried_from: start * SECOND < origin ? { revision_id: 'trv_a', segment_id: `a-${n}` } : null,
      alignment: { origin: 'local_asr', chunk_index: chunk, chunk_audio_start_us: origin + chunk * 25 * SECOND, chunk_audio_end_us: origin + chunk * 25 * SECOND + 30 * SECOND },
    });
  }
  return out;
}

function standIn() {
  const steps = [];
  const byRevision = { trv_a: segments(0, 'a'), trv_b: segments(15 * SECOND, 'b') };
  const coverage = { basis: 'local_asr', searched_range: { from_us: 0, to_us: 120 * SECOND }, transcribed_ranges: [{ from_us: 0, to_us: 120 * SECOND }],
    untranscribed_ranges: [], no_speech_ranges: [], ranges_truncated: false };
  const candidates = [{ candidate_id: 'vcd_1', representative_us: 0, reasons: ['first_frame'] }, { candidate_id: 'vcd_2', representative_us: 50 * SECOND, reasons: ['motion_start'] }];
  const ok = (data, status = 'complete') => ({ exit: 0, seconds: 0.1, json: { status, data, error: null } });
  const run = (step, args) => {
    steps.push(step);
    const [first, second] = args;
    let answer;
    if (first === 'ingest') answer = ok({ session_id: 'ses_0123' });
    else if (first === 'candidates') answer = ok({ index: { duration_us: 120 * SECOND }, items: candidates, next_cursor: null, coverage: { gaps: [] } });
    else if (first === 'transcript' && second === 'retranscribe') {
      const shifted = args.includes('--from');
      answer = ok({ revision: { revision_id: shifted ? 'trv_b' : 'trv_a', segment_count: 24, carried_segment_count: shifted ? 3 : 0, language: 'en',
        local_asr: { chunk_count: 5, transcribed_chunks: 5, silent_chunks: 0, no_audio_chunks: 0, unplaced_ranges: [] }, warnings: [{ code: 'seam_duplicates_removed', count: 1 }] } });
    } else if (first === 'transcript' && second === 'get') {
      answer = ok({ items: byRevision[args[args.indexOf('--revision') + 1]], next_cursor: null, transcript_coverage: coverage });
    } else if (first === 'search') {
      const query = args[args.indexOf('--query') + 1];
      answer = ok({ items: byRevision.trv_a.filter((segment) => segment.text.toLowerCase().includes(query)), transcript_coverage: coverage });
    } else if (first === 'audio') answer = ok({ files: [{ path: path.join(os.tmpdir(), 'private recordings', 'Secret Meeting clip.wav') }] });
    else if (first === 'handoff') answer = ok({ valid: true });
    else if (first === 'frame') {
      const id = args[args.indexOf('--candidate') + 1];
      const candidate = candidates.find((entry) => entry.candidate_id === id);
      answer = ok({ selections: [{ actual_us: candidate.representative_us }], items: [{ evidence_id: `evd_${id}`, frame: { width: 1280, height: 720 } }] });
    } else if (first === 'crop') answer = ok({ items: [{ image: { width: 480, height: 120 } }] });
    else if (first === 'session') answer = ok({ closed: true });
    else answer = { exit: 2, seconds: 0, json: { status: 'failed', data: null, error: { code: 'INVALID_ARGUMENT' } } };
    return { step, ...answer };
  };
  return { run, steps };
}

const PRIVATE_ITEM = { id: 'P01', status: 'approved', private: true, file_name: path.basename(FILE), bytes: 10, sha256: null,
  covers: ['conversational-pauses'], approval: { date: '2026-10-19', by: 'maintainer' } };

function scratch(t) {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift-real-media-campaign-'));
  t.after(() => fs.rmSync(folder, { recursive: true, force: true }));
  return folder;
}

function leaks(text) {
  return SECRETS.filter((secret) => text.toLowerCase().includes(secret.toLowerCase()));
}

test('a private item\'s words, file name and names never reach its report, its summary or a log line', (t) => {
  const { run, steps } = standIn();
  const copies = [];
  const written = [];
  const report = campaign.investigate(PRIVATE_ITEM, FILE, run, { folder: scratch(t), copyFile: (from, to) => copies.push([from, to]), exists: () => true, writeFile: (file) => written.push(file) });
  const kept = campaign.privateReport(report);
  const summary = campaign.markdown({ binary: { version: 'vsift 0.0.0', sha256: '0'.repeat(64) }, started: 's', finished: 'f', platform: 'p',
    setup: { passed: true, findings: [] }, items: [kept] });
  assert.deepEqual(leaks(JSON.stringify(kept)), []);
  assert.deepEqual(leaks(summary), []);
  assert.deepEqual(leaks(steps.join('\n')), [], 'a step label is a log line');
  assert.deepEqual(copies, [], 'no clip of a private item\'s audio is kept');
  assert.deepEqual(written, [], 'no transcript of a private item is written');
  assert.equal(kept.id, 'P01');
  assert.equal(kept.private, true);
  assert.equal(kept.measurements.transcription.revision_id, undefined);
  assert.ok(kept.measurements.searches.length > 0, 'the searches ran and are counted');
  assert.equal(typeof kept.measurements.seams_of_whole_run.seam_recall, 'number');
  assert.deepEqual(Object.keys(kept).sort(), ['failure_codes', 'id', 'measurements', 'private', 'verdict']);
});

test('the same run of a public item would carry those words, so the private test can see a leak', (t) => {
  const { run, steps } = standIn();
  const publicItem = { ...PRIVATE_ITEM, id: 'RM01', private: undefined, title: 'Secret Meeting', attribution: 'by Alice Hartwell', tests: ['#363'] };
  const report = campaign.investigate(publicItem, FILE, run, { folder: scratch(t), copyFile: () => {}, exists: () => true });
  assert.ok(leaks(JSON.stringify(report)).length > 0);
  assert.ok(leaks(steps.join('\n')).length > 0, 'a public search label names its words');
});

test('a private report keeps only codes and enumerations among its strings, whatever a check writes', () => {
  const kept = campaign.privateReport({
    id: 'P02',
    checks: { search: { passed: false, findings: ['search "alice hartwell zebrafinch" did not return'] }, coverage: { passed: true, findings: [] } },
    measurements: { note: 'Alice Hartwell', steps: [{ step: 'search 1', code: 'MISSING_CAPABILITY', status: 'failed' }, { step: 'search "zebrafinch quill"', code: null }],
      transcription: { revision_id: 'trv_secret', language: 'en', warnings: [{ code: 'seam_duplicates_removed', count: 2 }] }, lost: [{ words: 9, text: 'zebrafinch' }] },
  });
  assert.deepEqual(leaks(JSON.stringify(kept)), []);
  assert.deepEqual(kept.verdict, { passed: false, failed_checks: [{ check: 'search', findings: 1 }] });
  assert.deepEqual(kept.failure_codes, ['MISSING_CAPABILITY']);
  assert.deepEqual(kept.measurements.steps, [{ step: 'search 1', code: 'MISSING_CAPABILITY', status: 'failed' }, { code: null }]);
  assert.deepEqual(kept.measurements.transcription, { language: 'en', warnings: [{ code: 'seam_duplicates_removed', count: 2 }] });
  assert.deepEqual(kept.measurements.lost, [{ words: 9 }]);
});

test('a public item keeps its listening clip and its findings in words', (t) => {
  const { run } = standIn();
  const copies = [];
  const publicItem = { ...PRIVATE_ITEM, id: 'RM01', private: undefined, title: 'T', attribution: 'A', tests: ['#363'] };
  const report = campaign.investigate(publicItem, FILE, run, { folder: scratch(t), copyFile: (from, to) => copies.push(to), exists: () => true });
  assert.equal(copies.length, 1);
  assert.match(report.measurements.listening_clip, /^RM01\/listen\/RM01-\d+us\.wav$/);
  assert.equal(report.checks.no_failure.passed, true);
});

test('no workflow names a private manifest', () => {
  const workflows = path.join(__dirname, '..', '..', '..', '.github', 'workflows');
  for (const name of fs.readdirSync(workflows)) {
    assert.doesNotMatch(fs.readFileSync(path.join(workflows, name), 'utf8'), /private-manifest|private_manifest/, name);
  }
});

test('a window over its candidate budget is measured, not retried and not a finding; a window not analysed is retried', (t) => {
  const { run } = standIn();
  let calls = 0;
  let pending = 1;
  const budget = (step, args, extra) => {
    if (args[0] !== 'candidates') return run(step, args, extra);
    calls += 1;
    const gaps = [{ from_us: 0, to_us: 60 * SECOND, reason: 'candidate_budget_exhausted', dropped_candidates: 7 }];
    if (pending > 0) {
      pending -= 1;
      gaps.push({ from_us: 60 * SECOND, to_us: 120 * SECOND, reason: 'not_analyzed', dropped_candidates: 0 });
    }
    return { step, exit: 0, seconds: 0.1, json: { status: 'partial', error: null,
      data: { index: { duration_us: 120 * SECOND }, items: [{ candidate_id: 'vcd_1', representative_us: 0, reasons: ['first_frame'] }], next_cursor: null, coverage: { gaps } } } };
  };
  const publicItem = { ...PRIVATE_ITEM, id: 'RM01', private: undefined, title: 'T', attribution: 'A', tests: ['#363'] };
  const report = campaign.investigate(publicItem, FILE, budget, { folder: scratch(t), copyFile: () => {}, exists: () => false });
  assert.equal(calls, 2, 'one retry for the window not analysed, none for the budget');
  assert.equal(report.checks.candidates.passed, true);
  assert.deepEqual(report.measurements.candidates.final_gaps, { candidate_budget_exhausted: 1 });
  assert.equal(report.measurements.candidates.dropped_candidates, 7);
});

test('the shifted run starts at a segment boundary well inside a chunk step, or not at all', () => {
  const at = (...seconds) => seconds.map((s) => ({ start_us: s * SECOND }));
  assert.equal(campaign.shiftPoint(at(0, 6, 11.9, 17.92, 30), 200 * SECOND), 11.9 * SECOND);
  assert.equal(campaign.shiftPoint(at(0, 25, 50, 61, 300.5, 312), 2000 * SECOND), 61 * SECOND, 'the earliest start well inside a step');
  assert.equal(campaign.shiftPoint(at(0, 25, 50, 75, 300, 312), 2000 * SECOND), 312 * SECOND, 'a silent lead is passed over');
  assert.equal(campaign.shiftPoint(at(0, 25, 51), 200 * SECOND), 51 * SECOND, 'failing that, the nearest the middle');
  assert.equal(campaign.shiftPoint(at(0, 3, 700), 2000 * SECOND), null, 'no boundary between 5 s and ten minutes');
  assert.equal(campaign.shiftPoint(at(0, 12), 40 * SECOND), null, 'too short to shift');
});

test('a shifted run that kept the same chunk plan is reported, and its seam check is not run', (t) => {
  const { run } = standIn();
  const same = (step, args, extra) => {
    const answer = run(step, args, extra);
    if (args[0] === 'transcript' && args[1] === 'get' && args.includes('trv_b')) {
      return { ...answer, json: { ...answer.json, data: { ...answer.json.data, items: segments(0, 'b') } } };
    }
    return answer;
  };
  const publicItem = { ...PRIVATE_ITEM, id: 'RM01', private: undefined, title: 'T', attribution: 'A', tests: ['#363'] };
  const report = campaign.investigate(publicItem, FILE, same, { folder: scratch(t), copyFile: () => {}, exists: () => false, writeFile: () => {} });
  assert.equal(report.measurements.shift_achieved, false);
  assert.equal(report.checks.seams.not_run, 'the shifted run kept the chunk plan of the whole run');
});

test('a file is investigated only at its pin, or at its source checksum before it is pinned; a missing one is told apart', () => {
  const crypto = require('node:crypto');
  const bytes = Buffer.from('stand-in media');
  const digest = (algorithm) => crypto.createHash(algorithm).update(bytes).digest('hex');
  const there = { exists: true, size: bytes.length, digest };
  const pinned = { bytes: bytes.length, sha256: digest('sha256'), source_checksum: null };
  assert.equal(campaign.mediaState(pinned, there), 'ready');
  assert.equal(campaign.mediaState({ ...pinned, sha256: '0'.repeat(64) }, there), 'differs');
  assert.equal(campaign.mediaState({ ...pinned, bytes: 1 }, there), 'differs');
  assert.equal(campaign.mediaState(pinned, { exists: false, size: null, digest }), 'missing');
  const unpinned = { bytes: bytes.length, sha256: null, source_checksum: { algorithm: 'sha1', value: digest('sha1') } };
  assert.equal(campaign.mediaState(unpinned, there), 'ready');
  assert.equal(campaign.mediaState({ ...unpinned, source_checksum: { algorithm: 'sha1', value: '0'.repeat(40) } }, there), 'differs');
  assert.equal(campaign.mediaState({ ...unpinned, source_checksum: null }, there), 'differs', 'nothing to check a public file against');
  assert.equal(campaign.mediaState({ bytes: bytes.length, sha256: null, private: true }, there), 'ready');
});

test('the summary shows an item not yet mirrored as such, and the hosted run asks for it', () => {
  const summary = campaign.markdown({ binary: { version: 'v', sha256: '0' }, started: 's', finished: 'f', platform: 'p', setup: { passed: true, findings: [] },
    items: [{ id: 'RM01', title: 'T', not_yet_mirrored: true, verdict: { passed: true, findings: [] } }] });
  assert.match(summary, /\| RM01 \| not yet mirrored \|/);
  assert.doesNotMatch(summary, /## Attribution/);
  const workflow = fs.readFileSync(path.join(__dirname, '..', '..', '..', '.github', 'workflows', 'p14-real-media.yml'), 'utf8');
  assert.match(workflow, /real-media-campaign\.cjs[\s\S]*--available-only/);
});
