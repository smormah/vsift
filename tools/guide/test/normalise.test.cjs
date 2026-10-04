'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const { compareJson, compareText, jsonSubset, linesMatch, maskLine, maskLines } = require('../lib/normalise.cjs');

test('values that differ on every run are masked on both sides, a shortened form the same as a full one', () => {
  const full = 'Opened session ses_20f7f17df1cb03ea66377ed8993e0ae2 (src_sha256_c93562cb3cdf32b266edbd1faf56e3df61fb0b604ae352cd0fc2fd5ccaf14629)';
  const short = 'Opened session ses_20f7f17d… (src_sha256_c93562cb…)';
  assert.equal(maskLine(full), maskLine(short));
  assert.equal(maskLine(full), 'Opened session ses_<id> (src_sha256_<id>)');
  assert.equal(maskLine('Lifecycle: ephemeral, expires 2026-10-05T17:59:29Z'), 'Lifecycle: ephemeral, expires <time>');
  assert.equal(maskLine('Image 1440x900, 25741 bytes, sha256 eb8702ef4d54cff1bca0839a92f58b8099e13c6bb7530bb364638dba0182fb2c'), 'Image 1440x900, <n> bytes, sha256 <hex>');
  assert.equal(maskLine('Supplied file: srt, 161 bytes, sha256 b90a2d19…'), 'Supplied file: srt, <n> bytes, sha256 <hex>');
  assert.equal(maskLine('visual hash 6059303000000000'), 'visual hash <hex>');
  assert.equal(maskLine('Changed between 00:00:06.500000 and 00:00:07.000000: 1 blocks changed, the largest by 6'), 'Changed between 00:00:06.500000 and 00:00:07.000000: <n> blocks changed, the largest by <n>');
  assert.equal(maskLine('  confidence 82.20%, language en'), '  confidence <pct>, language en');
  assert.equal(maskLine('Artifacts: 4 (29190 bytes)'), 'Artifacts: <n> (<n> bytes)');
  assert.equal(maskLine('Generation: 17 (process_crash_consistent)'), 'Generation: <n> (process_crash_consistent)');
  assert.equal(maskLine('More on the next page: repeat the command with --cursor 127'), 'More on the next page: repeat the command with --cursor <n>');
  assert.equal(maskLine('vsift 0.1.0 (011bc4da1af6)'), 'vsift <version> (<commit>)');
  assert.equal(maskLine('vsift 0.2.0-rc.1 (011bc4da1af6)'), maskLine('vsift 0.2.0 (aaaaaaaaaaaa)'));
});

test('a delivered file path is one fixed word, whatever the machine; a number that is not on the list is kept', () => {
  assert.equal(maskLine('    C:\\Users\\alex\\x\\artifact-eb87.png'), '    <file path>');
  assert.equal(maskLine('    /home/alex/.cache/vsift-sessions/artifact-d7e6.wav'), '    <file path>');
  assert.equal(maskLine('Segments on this page: 3'), 'Segments on this page: 3');
  assert.equal(maskLine('Candidates on this page: 3'), 'Candidates on this page: 3');
});

test('speech recognition masks what a recogniser decides, only when asked', () => {
  const quoted = '  | Scroll to Order 1017.';
  assert.equal(maskLine(quoted), quoted);
  assert.equal(maskLine(quoted, true), '  | <speech>');
  assert.equal(maskLine('tsg_a80c548d…  00:00:00.000000 --> 00:00:07.000000', true), 'tsg_<id>  <span>');
  assert.equal(maskLine('Revision 1: trv_582a2361… (local_asr, language en, segments in all: 1)', true), 'Revision 1: trv_<id> (local_asr, language en, segments in all: <n>)');
  assert.equal(maskLine('Local ASR: whisper_cpp, model base, decoding r0-v1, 8 threads', true), 'Local ASR: whisper_cpp, model base, decoding r0-v1, <n> threads');
});

test('a line of three dots stands for any lines, and nothing else is flexible', () => {
  assert.ok(linesMatch(['a', '...', 'd'], ['a', 'b', 'c', 'd']));
  assert.ok(linesMatch(['a', '...', 'd'], ['a', 'd']));
  assert.ok(linesMatch(['…'], []));
  assert.ok(!linesMatch(['a', '...', 'd'], ['a', 'b', 'c']));
  assert.ok(!linesMatch(['a', 'b'], ['a', 'c']));
  assert.ok(!linesMatch(['a'], ['a', 'extra']));
});

test('a platform-only note and edge blank lines do not matter', () => {
  const actual = ['', 'Frame', 'Note: a path starting with \\\\?\\ is in Windows\' extended-length form.', '', ''].join('\n');
  assert.ok(compareText(['Frame'], actual).ok);
});

test('a changed word fails and says where', () => {
  const compared = compareText(['Opened session ses_1234…', 'Transcript: none imported'], 'Opened session ses_0000aaaabbbbccccddddeeeeffff1111\nTranscript: one imported\n');
  assert.equal(compared.ok, false);
  assert.match(compared.why, /first difference at shown line 2/);
});

test('shown JSON must be inside the real result, with elisions allowed', () => {
  const actual = { schema_version: '1', status: 'failed', error: { code: 'INVALID_ARGUMENT', retryable: false, remediation: [{ summary: 'a' }, { summary: 'b' }] }, warnings: [] };
  assert.deepEqual(jsonSubset({ status: 'failed', error: { code: 'INVALID_ARGUMENT', '…': '…' } }, actual), []);
  assert.deepEqual(jsonSubset({ error: { remediation: [{ summary: 'a' }, '…'] } }, actual), []);
  assert.deepEqual(jsonSubset({ error: { remediation: '…' } }, actual), []);
  assert.equal(jsonSubset({ status: 'complete' }, actual).length, 1);
  assert.equal(jsonSubset({ error: { remediation: [{ summary: 'a' }] } }, actual).length, 1);
  assert.equal(jsonSubset({ missing: 1 }, actual).length, 1);
  // Identifiers and times in strings are compared by kind.
  assert.deepEqual(jsonSubset({ id: 'ses_1234abcd…' }, { id: 'ses_0000aaaabbbbccccddddeeeeffff1111' }), []);
});

test('compareJson reads both sides as JSON and reports a bad one', () => {
  assert.ok(compareJson(['{"status": "failed", "…": "…"}'], '{"status":"failed","x":1}').ok);
  assert.equal(compareJson(['{"status": "failed"'], '{}').ok, false);
  assert.equal(compareJson(['{"a": 1}'], 'not json').ok, false);
  assert.equal(maskLines(['', 'x', '', '']).length, 1);
});
