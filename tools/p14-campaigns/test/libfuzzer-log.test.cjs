'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const { summarise, plateauLine } = require('../lib/libfuzzer-log.cjs');

const FINISHED = `INFO: Running with entropic power schedule (0xFF, 100).
INFO: seed corpus: files: 3 min: 100b max: 2000b total: 4000b rss: 40Mb
#4\tINITED cov: 120 ft: 150 corp: 3/4000b exec/s: 0 rss: 41Mb
#9\tNEW    cov: 125 ft: 160 corp: 4/4300b lim: 4096 exec/s: 0 rss: 41Mb L: 300/2000 MS: 2 ChangeBit-
#300\tNEW    cov: 131 ft: 171 corp: 9/5000b lim: 4096 exec/s: 4000 rss: 52Mb L: 80/2000 MS: 1 CrossOver-
#1024\tpulse  cov: 131 ft: 171 corp: 9/5000b lim: 4096 exec/s: 4000 rss: 52Mb
#4096\tREDUCE cov: 131 ft: 171 corp: 9/4900b lim: 4096 exec/s: 8000 rss: 60Mb L: 70/2000 MS: 1 EraseBytes-
#1000000\tDONE   cov: 131 ft: 171 corp: 9/4900b lim: 4096 exec/s: 20000 rss: 120Mb
Done 1000000 runs in 50 second(s)
stat::number_of_executed_units: 1000000
stat::average_exec_per_sec:     20000
stat::new_units_added:          22
stat::slowest_unit_time_sec:    0
stat::peak_rss_mb:              122
`;

test('a finished run reports the last new coverage and where it was found', () => {
  const summary = summarise(FINISHED);
  assert.equal(summary.finished, true);
  assert.equal(summary.executions, 1000000);
  assert.equal(summary.seconds, 50);
  assert.equal(summary.coverage, 131);
  assert.equal(summary.lastNewAt, 300);
  assert.equal(summary.lastNewCoverage, 131);
  assert.equal(summary.newEvents, 2);
  assert.equal(summary.peakRssMb, 122);
  assert.equal(summary.failure, null);
  const line = plateauLine('transcript_srt', summary, 50);
  assert.match(line, /^transcript_srt: 1000000 runs in 50 s, 20000 runs\/s, peak 122 MB;/);
  assert.match(line, /last new coverage \(131\) was found at run 300, 0% of the way/);
  assert.doesNotMatch(line, /asked/);
});

test('a run that stopped early says how much it ran', () => {
  const line = plateauLine('t', summarise(FINISHED), 3600);
  assert.match(line, /ran 50 s of the 3600 s asked/);
});

test('a log with no new coverage says so', () => {
  const log = '#4\tINITED cov: 10 ft: 12 corp: 1/10b exec/s: 0 rss: 41Mb\nDone 10 runs in 1 second(s)\n';
  const line = plateauLine('t', summarise(log), 1);
  assert.match(line, /no input beyond the seeds reached new coverage/);
});

test('a crash is a failure, not a plateau', () => {
  const log = `#4\tINITED cov: 10 ft: 12 corp: 1/10b exec/s: 0 rss: 41Mb
==1234== ERROR: libFuzzer: deadly signal
`;
  const summary = summarise(log);
  assert.match(summary.failure, /deadly signal/);
  assert.match(plateauLine('t', summary, 10), /^t: FAILED \(.*deadly signal.*\) after 4 runs$/);
});

test('a timeout and an out-of-memory are failures too', () => {
  assert.match(summarise('ALARM: working on the last Unit for 11 seconds\n==5== ERROR: libFuzzer: timeout after 11 seconds\n').failure, /timeout/);
  assert.match(summarise('==5== ERROR: libFuzzer: out-of-memory (used: 2100Mb; limit: 2048Mb)\n').failure, /out-of-memory/);
});

test('a log that ended without its final line is not a result', () => {
  const log = '#4\tINITED cov: 10 ft: 12 corp: 1/10b exec/s: 0 rss: 41Mb\n#9\tNEW    cov: 11 ft: 13 corp: 2/20b lim: 4096 exec/s: 0 rss: 41Mb L: 5/5 MS: 1 X-\n';
  const summary = summarise(log);
  assert.equal(summary.finished, false);
  assert.match(plateauLine('t', summary, 10), /^t: NO RESULT/);
});

test('CRLF line ends and an empty log are read', () => {
  assert.equal(summarise(FINISHED.replace(/\n/g, '\r\n')).coverage, 131);
  assert.equal(summarise('').finished, false);
});
