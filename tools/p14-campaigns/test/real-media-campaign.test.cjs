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
const { png } = require('./png-fixture.cjs');

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
        local_asr: { chunk_count: 5, transcribed_chunks: 5, silent_chunks: 0, no_audio_chunks: 0, threads: 4, unplaced_ranges: [] }, warnings: [{ code: 'seam_duplicates_removed', count: 1 }] } });
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

test("the runner is named by its CPU model, logical CPUs and the CPU flags that decide the recogniser's kernels (#486)", () => {
  const cpus = [{ model: 'Example  CPU   9000 @ 2.0GHz' }, { model: 'Example  CPU   9000 @ 2.0GHz' }];
  const cpuinfo = ['processor\t: 0', 'flags\t\t: fpu sse4_2 avx avx2 fma f16c avx512f unknownflag', 'processor\t: 1', ''].join('\n');
  assert.deepEqual(campaign.runnerIdentity({ cpus, cpuinfo }), {
    cpu_model: 'Example CPU 9000 @ 2.0GHz',
    logical_cpus: 2,
    cpu_flags: ['sse4_2', 'avx', 'avx2', 'fma', 'f16c', 'avx512f'],
  });
  assert.deepEqual(campaign.runnerIdentity({ cpus: [], cpuinfo: null }), { cpu_model: null, logical_cpus: 0, cpu_flags: null });
  const summary = campaign.markdown({ binary: { version: 'vsift 0.0.0', sha256: '0'.repeat(64) }, started: 's', finished: 'f', platform: 'p',
    runner: campaign.runnerIdentity({ cpus, cpuinfo }), setup: { passed: true, findings: [] }, items: [] });
  assert.match(summary, /- Runner: Example CPU 9000 @ 2\.0GHz, 2 logical CPUs; flags sse4_2 avx avx2 fma f16c avx512f/);
});

test("a public item's revisions are kept as times and text digests with their thread count, and a private item's are not (#486)", (t) => {
  const publicItem = { ...PRIVATE_ITEM, id: 'RM01', private: undefined, title: 'T', attribution: 'A', tests: ['#363'] };
  const shown = campaign.investigate(publicItem, FILE, standIn().run, { folder: scratch(t), copyFile: () => {}, exists: () => true, writeFile: () => {} });
  const whole = shown.measurements.transcription;
  assert.equal(whole.threads, 4);
  assert.equal(whole.segment_fingerprints.length, segments(0, 'a').length);
  for (const entry of [...whole.segment_fingerprints, ...shown.measurements.shifted_transcription.segment_fingerprints]) {
    assert.equal(entry.length, 4);
    assert.ok(Number.isInteger(entry[0]) && Number.isInteger(entry[1]));
    assert.match(entry[3], /^[0-9a-f]{16}$/);
  }
  assert.deepEqual(leaks(JSON.stringify(whole.segment_fingerprints)), [], 'a digest carries no word');
  // Carried segments belong to the other run and are left out.
  assert.equal(shown.measurements.shifted_transcription.segment_fingerprints.length, segments(15 * SECOND, 'b').filter((s) => !s.carried_from).length);
  // The same text gives the same digest in both revisions, so two runs compare segment by segment.
  assert.equal(whole.segment_fingerprints[10][3], campaign.segmentFingerprints([segments(0, 'a')[10]])[0][3]);
  const hidden = campaign.investigate(PRIVATE_ITEM, FILE, standIn().run, { folder: scratch(t), copyFile: () => {}, exists: () => true, writeFile: () => {} });
  assert.equal(hidden.measurements.transcription.segment_fingerprints, undefined);
  assert.equal(hidden.measurements.shifted_transcription.segment_fingerprints, undefined);
  const kept = campaign.privateReport(hidden);
  assert.equal(JSON.stringify(kept).includes('segment_fingerprints'), false);
  assert.equal(kept.measurements.transcription.threads, 4, 'the thread count is a measurement and is kept');
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

test('the frames of a screen recording are cut into native crops whose lines of text are measured (#173)', (t) => {
  const { run } = standIn();
  const files = new Map();
  /** A crop of light grey with one 9-row and one 12-row "line of text" (strokes every 4 pixels). */
  const textCrop = (width, height) => {
    const luma = Buffer.alloc(width * height, 230);
    for (const [top, rows] of [[20, 9], [50, 12]]) {
      for (let y = top; y < Math.min(height, top + rows); y += 1) for (let x = 8; x < width - 8; x += 4) luma[y * width + x] = 30;
    }
    return png(width, height, 0, 1, luma);
  };
  let shrink = true;
  const screen = (step, args, extra) => {
    if (args[0] !== 'crop') return run(step, args, extra);
    const [x, y, width, height] = args[args.indexOf('--rect') + 1].split(',').map(Number);
    const file = `crop-${files.size}.png`;
    // The first crop of the grid comes back a pixel short: a crop that is not native is a finding.
    const returned = shrink && step === 'crop (small text)' ? width - 1 : width;
    if (step === 'crop (small text)') shrink = false;
    files.set(file, textCrop(returned, height));
    return { step, exit: 0, seconds: 0.1, json: { status: 'complete', error: null,
      data: { files: [{ path: file }], items: [{ image: { width: returned, height }, crop: { x, y, width, height } }] } } };
  };
  const screenItem = { ...PRIVATE_ITEM, id: 'RM10', private: undefined, title: 'T', attribution: 'A', tests: ['#173'], covers: ['screen-ui-small-text'] };
  const report = campaign.investigate(screenItem, FILE, screen, { folder: scratch(t), copyFile: () => {}, exists: () => false, writeFile: () => {},
    readFile: (file) => files.get(file) });
  const measured = report.measurements.small_text;
  // Two chosen frames of 1280x720, each a grid of 3 x 6 crops.
  assert.equal(measured.frames, 2);
  assert.equal(measured.crops, 36);
  assert.equal(measured.not_native, 1);
  assert.equal(measured.line_heights.lines, 72);
  assert.equal(measured.line_heights.min_px, 9);
  assert.equal(measured.line_heights.max_px, 12);
  assert.deepEqual(report.checks.frames.findings, ['1 small-text crop(s) did not come back at native size']);
  // An item that is not a screen recording is not cut up.
  const other = campaign.investigate({ ...screenItem, covers: ['conversational-pauses'] }, FILE, run, { folder: scratch(t), copyFile: () => {}, exists: () => false, writeFile: () => {} });
  assert.equal(other.measurements.small_text, undefined);
});

/** A stand-in whose small-text crops hold one "line of text" whose height grows with each crop (5, 6, 7 ... px). */
function screenStandIn() {
  const { run } = standIn();
  const files = new Map();
  const screen = (step, args, extra) => {
    if (args[0] !== 'crop') return run(step, args, extra);
    const [, , width, height] = args[args.indexOf('--rect') + 1].split(',').map(Number);
    const rows = 5 + (files.size % 20);
    const luma = Buffer.alloc(width * height, 230);
    for (let y = 30; y < 30 + rows; y += 1) for (let x = 8; x < width - 8; x += 4) luma[y * width + x] = 30;
    const file = `crop-${files.size}.png`;
    files.set(file, png(width, height, 0, 1, luma));
    return { step, exit: 0, seconds: 0.1, json: { status: 'complete', error: null, data: { files: [{ path: file }], items: [{ image: { width, height } }] } } };
  };
  return { screen, readFile: (file) => files.get(file) };
}

const OPEN_SCREEN = { id: 'RM10', status: 'approved', title: 'T', attribution: 'A by the U.S. Census Bureau, public domain', tests: ['#173'], covers: ['screen-ui-small-text'],
  licence: { id: 'public-domain-us-government', url: 'https://example.invalid/licence' } };

test('a public, openly licensed item keeps a handful of crops with its smallest text, with its attribution (#173)', (t) => {
  const { screen, readFile } = screenStandIn();
  const copies = [];
  const written = new Map();
  const folder = scratch(t);
  const report = campaign.investigate(OPEN_SCREEN, FILE, screen, { folder, keepCrops: 12, readFile, exists: () => false,
    copyFile: (from, to) => copies.push([from, path.relative(folder, to)]), writeFile: (file, text) => written.set(path.relative(folder, file), text) });
  const kept = report.measurements.small_text.kept_crops;
  assert.equal(campaign.KEPT_CROPS_PER_ITEM, 4);
  assert.equal(kept.length, 4, 'at most four of one item, whatever the run allows');
  // The crops with the smallest text, ties to the earlier crop. The stand-in's file 0 is the campaign's
  // own top-left crop; the grid's crops are files 1 to 36, with lines of 6 to 24 px, then 5 px again.
  assert.deepEqual(kept.map((entry) => entry.smallest_line_px), [5, 6, 6, 7]);
  assert.deepEqual(copies.map(([, to]) => to), [1, 2, 3, 4].map((n) => path.join('crops', `RM10-crop-${n}.png`)));
  assert.deepEqual(copies.map(([from]) => from), ['crop-20.png', 'crop-1.png', 'crop-21.png', 'crop-2.png']);
  assert.match(written.get(path.join('crops', 'ATTRIBUTION.txt')), /^A by the U\.S\. Census Bureau, public domain\nLicence: public-domain-us-government/);
  assert.equal(report.measurements.small_text.files, undefined, 'the run\'s own file paths are not reported');
  // The run's budget is honoured: none left, none kept.
  const { screen: again, readFile: readAgain } = screenStandIn();
  const spent = campaign.investigate(OPEN_SCREEN, FILE, again, { folder: scratch(t), keepCrops: 0, readFile: readAgain, exists: () => false, copyFile: () => assert.fail('copied'), writeFile: () => {} });
  assert.deepEqual(spent.measurements.small_text.kept_crops, []);
});

test('a private item\'s crops are never kept, nor those of an item without an open licence (#173)', (t) => {
  for (const item of [{ ...PRIVATE_ITEM, covers: ['screen-ui-small-text'] }, { ...OPEN_SCREEN, licence: { id: 'all-rights-reserved', url: 'https://example.invalid' } }, { ...OPEN_SCREEN, licence: undefined }]) {
    const { screen, readFile } = screenStandIn();
    const copies = [];
    const written = [];
    const folder = scratch(t);
    const report = campaign.investigate(item, FILE, screen, { folder, keepCrops: 12, readFile, exists: () => true,
      copyFile: (from, to) => copies.push(to), writeFile: (file) => written.push(file) });
    assert.deepEqual(copies.filter((to) => to.endsWith('.png')), [], `${item.id}: no crop copied`);
    assert.deepEqual(written.filter((file) => file.includes(`${path.sep}crops${path.sep}`)), [], `${item.id}: nothing written beside crops`);
    assert.equal(fs.existsSync(path.join(folder, 'crops')), false, `${item.id}: no crops folder`);
    assert.deepEqual(report.measurements.small_text.kept_crops, []);
    if (item.private) {
      const kept = campaign.privateReport(report);
      assert.ok(kept.measurements.small_text.line_heights.lines > 0, 'the private item is still measured');
      assert.deepEqual(kept.measurements.small_text.kept_crops, []);
    }
  }
});

const OPEN_SPEECH = { id: 'RM09', status: 'approved', title: 'T', attribution: 'A by NASA, public domain', tests: ['#485'], covers: ['conversational-pauses'],
  licence: { id: 'public-domain-us-government', url: 'https://example.invalid/licence' } };

test('the segments a person asked for of a public, openly licensed item are kept as text, with its attribution (#485)', (t) => {
  const { run } = standIn();
  const written = new Map();
  const folder = scratch(t);
  const keepSegments = campaign.parseKeepSegments(['RM09:40-55']);
  const report = campaign.investigate(OPEN_SPEECH, FILE, run, { folder, keepSegments, copyFile: () => {}, exists: () => true,
    writeFile: (file, text) => written.set(file, text) });
  const kept = JSON.parse(written.get(path.join(folder, 'kept-segments', 'segments.json')));
  assert.equal(kept.item, 'RM09');
  assert.equal(kept.licence.id, 'public-domain-us-government');
  assert.deepEqual(kept.ranges, [{ from_us: 40 * SECOND, to_us: 55 * SECOND }]);
  // The whole revision's segments over 40 to 55 s: 40-45, 45-50, 50-55; nothing outside the range.
  assert.deepEqual(kept.whole.map((segment) => [segment.start_us / SECOND, segment.end_us / SECOND]), [[40, 45], [45, 50], [50, 55]]);
  assert.ok(kept.whole.every((segment) => typeof segment.text === 'string' && Number.isInteger(segment.chunk) && segment.chunk_audio_us.length === 2));
  assert.ok(kept.shifted.length >= 3);
  assert.match(written.get(path.join(folder, 'kept-segments', 'ATTRIBUTION.txt')), /A by NASA, public domain\nLicence: public-domain-us-government/);
  assert.deepEqual(report.measurements.kept_segments, { file: 'RM09/kept-segments/segments.json', ranges: kept.ranges, whole: 3, shifted: kept.shifted.length });
  // An item not asked for keeps nothing.
  const other = campaign.investigate({ ...OPEN_SPEECH, id: 'RM06' }, FILE, standIn().run, { folder: scratch(t), keepSegments, copyFile: () => {}, exists: () => true, writeFile: () => {} });
  assert.equal(other.measurements.kept_segments, undefined);
});

test('a private item\'s segments are never kept, nor those of an item without an open licence (#485)', (t) => {
  const asked = new Map([['P01', [{ from_us: 0, to_us: 60 * SECOND }]], ['RM09', [{ from_us: 0, to_us: 60 * SECOND }]]]);
  for (const item of [PRIVATE_ITEM, { ...OPEN_SPEECH, licence: { id: 'all-rights-reserved', url: 'https://example.invalid' } }, { ...OPEN_SPEECH, licence: undefined }]) {
    const written = [];
    const folder = scratch(t);
    const report = campaign.investigate(item, FILE, standIn().run, { folder, keepSegments: asked, copyFile: () => {}, exists: () => true, writeFile: (file) => written.push(file) });
    assert.deepEqual(written.filter((file) => file.includes('kept-segments')), [], `${item.id}: nothing written`);
    assert.equal(fs.existsSync(path.join(folder, 'kept-segments')), false, `${item.id}: no folder`);
    assert.equal(report.measurements.kept_segments, undefined);
    if (item.private) assert.deepEqual(leaks(JSON.stringify(campaign.privateReport(report))), []);
  }
});

test('a range to keep is a public item\'s, in whole seconds, and bounded (#485)', () => {
  assert.deepEqual([...campaign.parseKeepSegments(['RM09:915-940', 'RM09:1000-1010']).entries()],
    [['RM09', [{ from_us: 915 * SECOND, to_us: 940 * SECOND }, { from_us: 1000 * SECOND, to_us: 1010 * SECOND }]]]);
  for (const bad of ['P01:0-10', 'RM9:0-10', 'RM09:10-10', 'RM09:20-10', 'RM09:1.5-3', 'RM09:0-10;rm', 'RM09:0-121']) {
    assert.throws(() => campaign.parseKeepSegments([bad]), /keep-segments/, bad);
  }
  assert.throws(() => campaign.parseKeepSegments(['RM09:0-1', 'RM09:2-3', 'RM09:4-5', 'RM09:6-7', 'RM09:8-9']), /at most 4 ranges/);
  assert.equal(campaign.parseKeepSegments([]).size, 0);
});

test('a private report keeps the kind of each missing run, from the closed set only, and still no text', () => {
  const kept = campaign.privateReport({
    id: 'P01',
    checks: { seams: { passed: false, findings: ['9 words heard mid-chunk are missing: zebrafinch'] } },
    measurements: {
      seams_of_whole_run: {
        lost_runs: [
          { from_us: 1, to_us: 2, words: 9, seam: [0, 5], kind: 'loss' },
          { from_us: 3, to_us: 4, words: 6, seam: [0, 5], kind: 'repetition' },
          { from_us: 5, to_us: 6, words: 4, seam: [0, 5], kind: 'script' },
          { from_us: 7, to_us: 8, words: 3, seam: [0, 5], kind: 'Alice Hartwell' },
        ],
        lost_run_kinds: { loss: 1, repetition: 1, script: 1 },
      },
      note: { kind: 'zebrafinch Quillmorton' },
    },
  });
  const runs = kept.measurements.seams_of_whole_run.lost_runs;
  assert.deepEqual(runs.map((entry) => entry.kind), ['loss', 'repetition', 'script', undefined]);
  assert.equal(runs[3].words, 3, 'a run whose kind is not in the set keeps its numbers');
  assert.deepEqual(kept.measurements.note, {}, 'a kind outside the set is dropped wherever it appears');
  assert.deepEqual(leaks(JSON.stringify(kept)), []);
});

/**
 * A stand-in `vsift <video>` (ADR 0037): it writes the three outputs beside the
 * video it is given and answers with their hashes, as the binary does.
 * `changes` screen changes, frames for at most 180 of them; `spoil` changes one
 * thing so a check can see it.
 */
function watchStandIn({ videoSha256, changes, listed = changes, spoil = null }) {
  const calls = [];
  const hash = (bytes) => require('node:crypto').createHash('sha256').update(bytes).digest('hex');
  const run = (step, args, { root }) => {
    calls.push({ step, args, root });
    const video = args[0];
    const folder = path.dirname(video);
    const stem = path.basename(video, path.extname(video));
    const transcript = path.join(folder, `${stem}.txt`);
    const subtitles = path.join(folder, `${stem}.srt`);
    const frames = path.join(folder, `${stem}-vsift`);
    const list = path.join(frames, 'screen-changes.txt');
    fs.mkdirSync(frames);
    const header = `${path.basename(video)}: transcript by VSift 0.3.0, 2026-10-28 09:00 UTC (local speech recognition).\n\nVideo SHA-256    ${spoil === 'header' ? 'f'.repeat(64) : videoSha256}\n\n[00:00:01] Hello.\n`;
    fs.writeFileSync(transcript, header);
    fs.writeFileSync(subtitles, '1\n00:00:01,000 --> 00:00:02,000\nHello.\n\n');
    const written = [];
    const count = Math.min(listed, campaign.WATCH_MAX_FRAMES) - (spoil === 'frames' ? 1 : 0);
    for (let index = 0; index < count; index += 1) {
      const file = path.join(frames, `00-00-${String(index % 60).padStart(2, '0')}.${String(index).padStart(3, '0')}.png`);
      const bytes = Buffer.from(`frame ${index}`);
      fs.writeFileSync(file, bytes);
      written.push({ time_us: index * SECOND, candidate_id: `vcd_${index}`, evidence_id: `evd_${index}`, path: file, sha256: hash(bytes) });
    }
    if (spoil === 'frame bytes' && written.length > 0) fs.writeFileSync(written[0].path, 'replaced');
    const reasons = ['first frame', 'settled after motion', 'visual change'];
    const lines = Array.from({ length: listed }, (_, index) => {
      const frame = written[index];
      return `00:00:${String(index % 60).padStart(2, '0')}    ${reasons[Math.min(index, 2)].padEnd(20)}  ${frame ? path.basename(frame.path) : '(not written)'}  vcd_${index}  ${frame ? frame.sha256 : '-'}`;
    });
    fs.writeFileSync(list, `Video SHA-256    ${videoSha256}\n\n${lines.join('\n')}\n`);
    if (spoil === 'partial') fs.writeFileSync(path.join(folder, `.${stem}.txt.1234.vsift-partial`), '');
    const output = (file) => ({ path: file, name: path.basename(file), sha256: hash(fs.readFileSync(file)) });
    return {
      step,
      exit: 0,
      seconds: 42,
      json: {
        command: 'watch',
        status: 'complete',
        error: null,
        data: {
          source_id: `src_sha256_${videoSha256}`, session_id: 'ses_1', session_closed: spoil !== 'session', job_id: 'job_1', duration_us: 120 * SECOND,
          transcript_revision_id: 'trv_1', visual_index_revision_id: 'vix_1', outcome: 'complete', not_transcribed: [],
          screen_changes_beyond_bound: false, names_shortened: false,
          outputs: { transcript: output(transcript), subtitles: output(subtitles), folder: { path: frames, name: path.basename(frames), sha256: null }, screen_changes: output(list) },
          changes_found: changes, changes_listed: listed, frames: written,
        },
      },
    };
  };
  return { run, calls };
}

const WATCHED_ITEM = { id: 'RM10', status: 'approved', title: 'A screen recording', attribution: 'public domain', file_name: 'talk.mp4', sha256: 'a'.repeat(64) };

function watchedSource(t) {
  const folder = scratch(t);
  const media = path.join(folder, 'media');
  fs.mkdirSync(media);
  const file = path.join(media, 'talk.mp4');
  fs.writeFileSync(file, 'not really a video');
  const item = path.join(folder, 'RM10');
  fs.mkdirSync(item);
  return { file, item };
}

test('vsift <video> runs the bare form on a copy, and its outputs, header, frames and hashes are checked (ADR 0037)', (t) => {
  const { file, item } = watchedSource(t);
  const { run, calls } = watchStandIn({ videoSha256: WATCHED_ITEM.sha256, changes: 576, listed: 200 });
  const watched = campaign.watchItem(WATCHED_ITEM, file, run, { folder: item });
  assert.deepEqual(watched.check, { passed: true, findings: [] });
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].args, [path.join(item, 'watch', 'talk.mp4')], 'the bare form, on the copy, by its absolute path');
  assert.equal(calls[0].root, path.join(item, 'watch-root'), 'a session root of its own');
  assert.ok(fs.existsSync(file), 'the item itself is never touched');
  assert.equal(watched.measurements.changes_found, 576);
  assert.equal(watched.measurements.changes_listed, 200, 'the settled screens');
  assert.deepEqual(watched.measurements.listed_by_reason, { 'first frame': 1, 'settled after motion': 1, 'visual change': 198 });
  assert.equal(watched.measurements.frames_written, 180, 'the bound of L-234');
  assert.equal(watched.measurements.seconds, 42);
  assert.ok(watched.measurements.frame_bytes > 0);
  const summary = campaign.markdown({ binary: { version: 'vsift 0.0.0', sha256: '0'.repeat(64) }, started: 's', finished: 'f', platform: 'p',
    setup: { passed: true, findings: [] }, items: [{ id: 'RM10', measurements: { watch: watched.measurements }, verdict: { passed: true, findings: [] } }] });
  assert.match(summary, /\| RM10 \| 42 s \(0\.35\) \| complete \| 576 \| 200 \| 180 \(\d+\) \|/);
});

test('vsift <video> findings: a wrong header hash, a missing frame, changed frame bytes, a temporary file left, an open session, a failure', (t) => {
  const cases = [
    ['header', /does not name the item's pinned SHA-256/],
    ['frames', /179 frames were written for 576 listed changes, not 180/],
    ['partial', /1 temporary file\(s\) left beside the video/],
    ['frame bytes', /1 frame\(s\) missing or differing from their SHA-256/],
    ['session', /the session was not closed/],
  ];
  for (const [spoil, finding] of cases) {
    const { file, item } = watchedSource(t);
    const { run } = watchStandIn({ videoSha256: WATCHED_ITEM.sha256, changes: 576, spoil });
    const watched = campaign.watchItem(WATCHED_ITEM, file, run, { folder: item });
    assert.equal(watched.check.passed, false, spoil);
    assert.ok(watched.check.findings.some((entry) => finding.test(entry)), `${spoil}: ${watched.check.findings.join('; ')}`);
  }
  const { file, item } = watchedSource(t);
  const failed = campaign.watchItem(WATCHED_ITEM, file, (step) => ({ step, exit: 3, seconds: 1, json: { command: 'watch', status: 'failed', data: null, error: { code: 'MISSING_CAPABILITY' } } }), { folder: item });
  assert.deepEqual(failed.check, { passed: false, findings: ['vsift <video>: MISSING_CAPABILITY on valid media (exit 3)'] });
});
