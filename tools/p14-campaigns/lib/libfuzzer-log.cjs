'use strict';

// Reads the log of one libFuzzer run and states where its coverage stopped
// growing (P14 RQ-07: "a coverage-plateau line per target"). libFuzzer prints
// no clock, so the plateau is placed by the number of executions: the run at
// which the last input that reached new coverage was found, against the runs
// the whole campaign made.

const EVENT = /^#(\d+)\s+(INITED|NEW|REDUCE|pulse|DONE|RELOAD)\s+cov:\s*(\d+)\s+ft:\s*(\d+)\s+corp:\s*(\d+)\/(\S+)/;
const DONE_LINE = /^Done\s+(\d+)\s+runs\s+in\s+(\d+)\s+second\(s\)/;
const STAT = /^stat::(\w+):\s*(\d+)/;
const CRASH = /(ERROR: libFuzzer: (?:deadly signal|timeout|out-of-memory)[^\n]*|ERROR: AddressSanitizer[^\n]*|==\d+==ERROR[^\n]*)/;

/**
 * @param {string} text the whole log of one `cargo fuzz run`
 * @returns {{
 *   executions: number, seconds: number|null, coverage: number|null,
 *   features: number|null, corpus: number|null, lastNewAt: number|null,
 *   lastNewCoverage: number|null, newEvents: number, peakRssMb: number|null,
 *   execsPerSecond: number|null, failure: string|null, finished: boolean
 * }}
 */
function summarise(text) {
  const out = {
    executions: 0,
    seconds: null,
    coverage: null,
    features: null,
    corpus: null,
    lastNewAt: null,
    lastNewCoverage: null,
    newEvents: 0,
    peakRssMb: null,
    execsPerSecond: null,
    failure: null,
    finished: false,
  };
  for (const raw of String(text).split(/\r?\n/)) {
    const line = raw.trim();
    const event = EVENT.exec(line);
    if (event) {
      const runs = Number(event[1]);
      out.executions = Math.max(out.executions, runs);
      out.coverage = Number(event[3]);
      out.features = Number(event[4]);
      out.corpus = Number(event[5]);
      if (event[2] === 'NEW') {
        out.newEvents += 1;
        out.lastNewAt = runs;
        out.lastNewCoverage = Number(event[3]);
      }
      continue;
    }
    const done = DONE_LINE.exec(line);
    if (done) {
      out.executions = Math.max(out.executions, Number(done[1]));
      out.seconds = Number(done[2]);
      out.finished = true;
      continue;
    }
    const stat = STAT.exec(line);
    if (stat) {
      if (stat[1] === 'peak_rss_mb') out.peakRssMb = Number(stat[2]);
      if (stat[1] === 'average_exec_per_sec') out.execsPerSecond = Number(stat[2]);
      if (stat[1] === 'number_of_executed_units') {
        out.executions = Math.max(out.executions, Number(stat[2]));
      }
      continue;
    }
    if (out.failure === null) {
      const crash = CRASH.exec(line);
      if (crash) out.failure = crash[1].slice(0, 200);
    }
  }
  return out;
}

/**
 * One plain line per target: what ran and where coverage stopped growing.
 * @param {string} target the target's name
 * @param {ReturnType<typeof summarise>} s
 * @param {number} requestedSeconds the campaign's `-max_total_time`
 */
function plateauLine(target, s, requestedSeconds) {
  if (s.failure !== null) {
    return `${target}: FAILED (${s.failure}) after ${s.executions} runs`;
  }
  if (!s.finished || s.coverage === null) {
    return `${target}: NO RESULT (the log has no final line; ${s.executions} runs seen)`;
  }
  const where = s.lastNewAt === null
    ? 'no input beyond the seeds reached new coverage'
    : `the last new coverage (${s.lastNewCoverage}) was found at run ${s.lastNewAt}, ${percent(s.lastNewAt, s.executions)}% of the way`;
  const shortfall = s.seconds !== null && s.seconds + 5 < requestedSeconds
    ? `; ran ${s.seconds} s of the ${requestedSeconds} s asked`
    : '';
  const rate = s.execsPerSecond === null ? '' : `, ${s.execsPerSecond} runs/s`;
  const rss = s.peakRssMb === null ? '' : `, peak ${s.peakRssMb} MB`;
  return `${target}: ${s.executions} runs in ${s.seconds} s${rate}${rss}; coverage ${s.coverage}, features ${s.features}, corpus ${s.corpus}; ${where}${shortfall}`;
}

function percent(part, whole) {
  if (whole <= 0) return 0;
  return Math.round((part / whole) * 1000) / 10;
}

module.exports = { summarise, plateauLine };
