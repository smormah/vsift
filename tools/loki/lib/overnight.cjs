'use strict';

// The overnight lane on LOKI (ADR 0025, docs/operations/loki-runner.md): which
// suites a run holds, whether a scheduled run is tonight's, and the arguments the
// repetition runner (tools/p14-campaigns/stress.cjs) gets for each suite.
// Everything here is pure, so it is tested without a runner.

const path = require('node:path');
const { validateRepro, ReproOptionError } = require('../../p14-campaigns/lib/repro.cjs');

/** The suites a run may name, in the order a run takes them. */
const SUITES = Object.freeze([
  'locks', 'admission', 'supervisor', 'supervisor-loaded', 'roots', 'roots-loaded', 'engine', 'delivery',
  'batch-277', 'repro', 'real-media',
]);

/**
 * The slot of the real-media campaign (#363): a run may name it, and it is
 * reported as not run until the campaign exists. It is never a pass.
 */
const REAL_MEDIA = 'real-media';
const REPRO = 'repro';

/**
 * The nightly set and each suite's minutes: the Windows race and stress suites
 * of P14's RQ-08 plus #277's opt-in batch test, 260 minutes in all, a starting
 * point to tune once LOKI's first nights show what a repetition costs there.
 */
const NIGHTLY = Object.freeze([
  { name: 'locks', minutes: 30 },
  { name: 'admission', minutes: 30 },
  { name: 'supervisor-loaded', minutes: 45 },
  { name: 'roots-loaded', minutes: 45 },
  { name: 'engine', minutes: 40 },
  { name: 'delivery', minutes: 45 },
  { name: 'batch-277', minutes: 25 },
]);

/** The fewest repetitions a budget suite must complete to pass. */
const MINIMUM_REPETITIONS = 5;

/**
 * The most minutes a run's suites may take together: the LOKI job's timeout
 * (330 minutes) less half an hour for the builds and a repetition that hangs
 * past its suite's budget (the runner stops one after 25 minutes).
 */
const RUN_MINUTES = 300;
const SUITE_MINUTES_MAX = RUN_MINUTES;
const REPRO_RUNS_MAX = 100000;

/**
 * The two schedules of the workflow: 02:07 in the United Kingdom, which is 01:07
 * UTC in British Summer Time and 02:07 UTC in Greenwich Mean Time. GitHub's cron is
 * UTC, so the workflow has both and each runs only in its own season. Seven minutes
 * past the hour, because the top of the hour is when GitHub delays schedules most.
 */
const SCHEDULES = Object.freeze({
  summer: { cron: '7 1 * * *', offsetMinutes: 60 },
  winter: { cron: '7 2 * * *', offsetMinutes: 0 },
});

class PlanError extends Error {
  constructor(message) {
    super(message);
    this.name = 'PlanError';
  }
}

/**
 * London's offset from UTC at `date`, in minutes (60 in summer time, 0 otherwise).
 * @param {Date} date
 */
function londonOffsetMinutes(date) {
  const name = new Intl.DateTimeFormat('en-GB', { timeZone: 'Europe/London', timeZoneName: 'shortOffset' })
    .formatToParts(date)
    .find((part) => part.type === 'timeZoneName');
  const match = /^GMT(?:([+-])(\d{1,2})(?::(\d{2}))?)?$/.exec(name ? name.value : '');
  if (!match) throw new PlanError(`cannot read London's offset from ${name ? name.value : 'nothing'}`);
  if (!match[1]) return 0;
  const minutes = Number(match[2]) * 60 + Number(match[3] || 0);
  return match[1] === '-' ? -minutes : minutes;
}

/**
 * Whether a scheduled run is tonight's: the summer schedule runs while London is
 * on summer time and the winter one otherwise, so exactly one runs each night. On
 * the two nights the clocks change, a run delayed across the change could run
 * twice or not at all; that is accepted.
 * @param {string} cron the schedule that started the run (`github.event.schedule`)
 * @param {Date} now
 */
function decideSchedule(cron, now) {
  const season = Object.entries(SCHEDULES).find(([, schedule]) => schedule.cron === cron);
  if (!season) return { run: false, reason: `the schedule \`${cron}\` is not one of the workflow's` };
  const [name, schedule] = season;
  const offset = londonOffsetMinutes(now);
  if (offset === schedule.offsetMinutes) return { run: true, reason: `the ${name} schedule, and London is at UTC+${offset / 60}` };
  return { run: false, reason: `the ${name} schedule, but London is at UTC+${offset / 60}: the other schedule runs tonight` };
}

function integer(text, name, low, high, fallback) {
  const value = text === undefined || text === '' ? fallback : Number(text);
  if (!Number.isInteger(value) || value < low || value > high) {
    throw new PlanError(`${name} must be an integer from ${low} to ${high}`);
  }
  return value;
}

/**
 * The run's plan, from the workflow's event and inputs (passed through `env`).
 * A scheduled run is the nightly set; a dispatch names its suites, or takes the
 * nightly set when it names none.
 * @param {Record<string, string|undefined>} env
 * @param {Date} now
 * @returns {{ run: boolean, reason: string, suites: object[] }}
 */
function plan(env, now) {
  if (env.EVENT === 'schedule') {
    const decision = decideSchedule(env.SCHEDULE || '', now);
    if (!decision.run) return { ...decision, suites: [] };
    return { ...decision, suites: nightly() };
  }
  if (env.EVENT !== 'workflow_dispatch') {
    throw new PlanError(`the overnight lane runs only on a schedule or a dispatch, not on \`${env.EVENT}\` (ADR 0025)`);
  }
  const names = (env.SUITES || '').split(',').map((name) => name.trim()).filter(Boolean);
  if (names.length === 0) return { run: true, reason: 'a dispatch that names no suite runs the nightly set', suites: nightly() };
  const minutes = integer(env.MINUTES, 'minutes', 1, SUITE_MINUTES_MAX, 30);
  const suites = [];
  for (const name of names) {
    if (!SUITES.includes(name)) throw new PlanError(`unknown suite \`${name}\`; the suites are ${SUITES.join(', ')}`);
    if (suites.some((suite) => suite.name === name)) throw new PlanError(`the suite \`${name}\` is named twice`);
    if (name === REAL_MEDIA) {
      suites.push({ name, slot: true });
    } else if (name === REPRO) {
      suites.push({
        name,
        minutes,
        minimum: integer(env.REPRO_RUNS, 'repro_runs', 1, REPRO_RUNS_MAX, 1000),
        until: 'runs',
        repro: reproOf(env),
      });
    } else {
      suites.push({ name, minutes, minimum: MINIMUM_REPETITIONS, until: 'budget' });
    }
  }
  suites.sort((a, b) => SUITES.indexOf(a.name) - SUITES.indexOf(b.name));
  return { run: true, reason: `a dispatch of ${names.join(', ')}`, suites: validatePlan(suites) };
}

function nightly() {
  return validatePlan(NIGHTLY.map((suite) => ({ ...suite, minimum: MINIMUM_REPETITIONS, until: 'budget' })));
}

function reproOf(env) {
  const numeric = (text, fallback) => (text === undefined || text === '' ? fallback : Number(text));
  try {
    return validateRepro({
      package: env.REPRO_PACKAGE,
      target: env.REPRO_TARGET,
      filter: env.REPRO_FILTER,
      match: env.REPRO_MATCH || 'exact',
      ignored: env.REPRO_IGNORED || 'no',
      copies: numeric(env.REPRO_COPIES, 4),
      burners: numeric(env.REPRO_BURNERS, 1),
    });
  } catch (error) {
    if (error instanceof ReproOptionError) throw new PlanError(`repro: ${error.message.replace(/--/g, 'repro_')}`);
    throw error;
  }
}

/**
 * Checks a plan's suites again where they are run: the plan crosses from the
 * hosted planning job to LOKI as a job output, so LOKI does not take it on trust.
 * @param {unknown} suites
 */
function validatePlan(suites) {
  if (!Array.isArray(suites) || suites.length === 0) throw new PlanError('the plan names no suite');
  let total = 0;
  const seen = new Set();
  for (const suite of suites) {
    if (!suite || typeof suite !== 'object' || !SUITES.includes(suite.name) || seen.has(suite.name)) {
      throw new PlanError('the plan names an unknown or repeated suite');
    }
    seen.add(suite.name);
    if (suite.name === REAL_MEDIA) {
      if (suite.slot !== true || Object.keys(suite).length !== 2) throw new PlanError('the real-media slot takes nothing yet');
      continue;
    }
    integer(suite.minutes, `${suite.name} minutes`, 1, SUITE_MINUTES_MAX);
    integer(suite.minimum, `${suite.name} minimum`, 1, REPRO_RUNS_MAX);
    if (suite.until !== (suite.name === REPRO ? 'runs' : 'budget')) throw new PlanError(`${suite.name} has the wrong stopping rule`);
    if (suite.name === REPRO) {
      validateRepro(suite.repro || {});
    } else if (suite.repro !== undefined) {
      throw new PlanError(`only the repro suite takes a test`);
    }
    total += suite.minutes;
  }
  if (total > RUN_MINUTES) {
    throw new PlanError(`the suites ask for ${total} minutes; a night holds ${RUN_MINUTES} (the job's timeout less the builds)`);
  }
  return suites;
}

/**
 * The repetition runner's arguments for one suite (without the node executable).
 * @param {object} suite one validated entry of the plan
 * @param {{ repository: string, out: string, commit: string }} where
 */
function stressArguments(suite, where) {
  const args = [
    path.join(where.repository, 'tools', 'p14-campaigns', 'stress.cjs'),
    '--suite', suite.name,
    '--runs', String(suite.minimum),
    '--until', suite.until,
    '--minutes', String(suite.minutes),
    '--timeout-minutes', '25',
    '--max-failures', '10',
    '--commit', where.commit,
    '--out', path.join(where.out, suite.name),
  ];
  if (suite.repro) {
    const repro = suite.repro;
    args.push(
      '--package', repro.package, '--target', repro.target, '--filter', repro.filter, '--match', repro.match,
      '--ignored', repro.ignored, '--copies', String(repro.copies), '--burners', String(repro.burners),
    );
  }
  return args;
}

/**
 * The run's summary: one line per suite, then each suite's own summary.
 * @param {{ name: string, outcome: string, minutes?: number, summary?: string }[]} results
 * @param {string} commit
 */
function renderSummary(results, commit) {
  const lines = [
    `## LOKI overnight on ${commit}: ${results.some((result) => result.outcome === 'failed') ? 'FAILED' : 'passed'}`,
    '',
    '| Suite | Outcome | Minutes |',
    '| --- | --- | --- |',
  ];
  for (const result of results) {
    const outcome = result.outcome === 'not_run'
      ? 'not run: the real-media campaign is not built yet (#363); this is its slot'
      : result.outcome;
    lines.push(`| ${result.name} | ${outcome} | ${result.minutes === undefined ? '' : result.minutes} |`);
  }
  lines.push('');
  for (const result of results) {
    if (result.summary) lines.push(result.summary.trim(), '');
  }
  return `${lines.join('\n')}\n`;
}

module.exports = {
  SUITES, NIGHTLY, SCHEDULES, MINIMUM_REPETITIONS, RUN_MINUTES, REAL_MEDIA, PlanError,
  londonOffsetMinutes, decideSchedule, plan, validatePlan, stressArguments, renderSummary,
};
