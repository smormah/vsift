'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const {
  NIGHTLY, SCHEDULES, RUN_MINUTES, PlanError, londonOffsetMinutes, decideSchedule, plan, validatePlan, stressArguments,
  renderSummary,
} = require('../lib/overnight.cjs');

const WORKFLOW = fs.readFileSync(path.join(__dirname, '..', '..', '..', '.github', 'workflows', 'loki-overnight.yml'), 'utf8');

const REPRO = {
  EVENT: 'workflow_dispatch',
  SUITES: 'repro',
  MINUTES: '240',
  REPRO_PACKAGE: 'vsift-infrastructure',
  REPRO_TARGET: 'session_root_provisioning',
  REPRO_FILTER: 'concurrent_processes_create_one_root_and_every_one_adopts_it',
  REPRO_MATCH: 'exact',
  REPRO_IGNORED: 'no',
  REPRO_RUNS: '2000',
  REPRO_COPIES: '4',
  REPRO_BURNERS: '1',
};

test('London is an hour ahead of UTC in summer and level in winter', () => {
  assert.equal(londonOffsetMinutes(new Date('2026-07-01T01:07:00Z')), 60);
  assert.equal(londonOffsetMinutes(new Date('2026-12-01T02:07:00Z')), 0);
  // The clocks go back at 01:00 UTC on 2026-10-25 and forward at 01:00 UTC on 2027-03-28.
  assert.equal(londonOffsetMinutes(new Date('2026-10-25T00:59:00Z')), 60);
  assert.equal(londonOffsetMinutes(new Date('2026-10-25T01:07:00Z')), 0);
  assert.equal(londonOffsetMinutes(new Date('2027-03-28T01:07:00Z')), 60);
});

test('exactly one of the two schedules runs each night, at 02:07 in London', () => {
  for (const night of ['2026-01-15', '2026-03-29', '2026-07-01', '2026-10-25', '2026-10-26', '2027-03-28']) {
    const summer = decideSchedule(SCHEDULES.summer.cron, new Date(`${night}T01:07:00Z`));
    const winter = decideSchedule(SCHEDULES.winter.cron, new Date(`${night}T02:07:00Z`));
    assert.equal(Number(summer.run) + Number(winter.run), 1, night);
  }
  assert.equal(decideSchedule('0 3 * * *', new Date()).run, false);
});

test('the workflow lists exactly the two schedules the plan knows', () => {
  const crons = [...WORKFLOW.matchAll(/^\s+- cron: "([^"]+)"/gm)].map((match) => match[1]);
  assert.deepEqual(crons, [SCHEDULES.summer.cron, SCHEDULES.winter.cron]);
});

test('a scheduled night is the nightly set, each suite repeating until its budget', () => {
  const decided = plan({ EVENT: 'schedule', SCHEDULE: SCHEDULES.winter.cron }, new Date('2026-12-01T02:07:00Z'));
  assert.equal(decided.run, true);
  assert.deepEqual(decided.suites.map((suite) => suite.name), NIGHTLY.map((suite) => suite.name));
  assert.ok(decided.suites.every((suite) => suite.until === 'budget' && suite.minimum === 5));
  assert.ok(NIGHTLY.reduce((sum, suite) => sum + suite.minutes, 0) <= RUN_MINUTES);
  const skipped = plan({ EVENT: 'schedule', SCHEDULE: SCHEDULES.summer.cron }, new Date('2026-12-01T01:07:00Z'));
  assert.deepEqual([skipped.run, skipped.suites], [false, []]);
});

test('a dispatch runs the suites it names, in order, and the nightly set when it names none', () => {
  const named = plan({ EVENT: 'workflow_dispatch', SUITES: 'delivery, locks', MINUTES: '60' }, new Date());
  assert.deepEqual(named.suites.map((suite) => [suite.name, suite.minutes]), [['locks', 60], ['delivery', 60]]);
  assert.equal(plan({ EVENT: 'workflow_dispatch', SUITES: '' }, new Date()).suites.length, NIGHTLY.length);
});

test('a repro dispatch runs one test a counted number of times', () => {
  const [suite] = plan(REPRO, new Date()).suites;
  assert.deepEqual(suite, {
    name: 'repro',
    minutes: 240,
    minimum: 2000,
    until: 'runs',
    repro: {
      package: 'vsift-infrastructure',
      target: 'session_root_provisioning',
      filter: 'concurrent_processes_create_one_root_and_every_one_adopts_it',
      match: 'exact',
      ignored: 'no',
      copies: 4,
      burners: 1,
    },
  });
  assert.deepEqual(stressArguments(suite, { repository: '/r', out: '/o', commit: 'c' }).slice(1), [
    '--suite', 'repro', '--runs', '2000', '--until', 'runs', '--minutes', '240', '--timeout-minutes', '25',
    '--max-failures', '10', '--commit', 'c', '--out', path.join('/o', 'repro'),
    '--package', 'vsift-infrastructure', '--target', 'session_root_provisioning',
    '--filter', 'concurrent_processes_create_one_root_and_every_one_adopts_it', '--match', 'exact',
    '--ignored', 'no', '--copies', '4', '--burners', '1',
  ]);
});

test('the real-media slot is accepted and reported as not run, never as a pass of its own', () => {
  const decided = plan({ EVENT: 'workflow_dispatch', SUITES: 'real-media' }, new Date());
  assert.deepEqual(decided.suites, [{ name: 'real-media', slot: true }]);
  const text = renderSummary([{ name: 'real-media', outcome: 'not_run' }], 'abc');
  assert.match(text, /\| real-media \| not run: the real-media campaign is not built yet \(#363\)/);
  assert.match(renderSummary([{ name: 'locks', outcome: 'failed', minutes: 30, summary: '### locks' }], 'abc'), /FAILED[\s\S]*### locks/);
});

test('a plan of the wrong shape is refused where it is made and where it is run', () => {
  for (const env of [
    { EVENT: 'pull_request' },
    { EVENT: 'push' },
    { EVENT: 'workflow_dispatch', SUITES: 'locks,nothing' },
    { EVENT: 'workflow_dispatch', SUITES: 'locks,locks' },
    { EVENT: 'workflow_dispatch', SUITES: 'locks', MINUTES: '0' },
    { EVENT: 'workflow_dispatch', SUITES: 'locks', MINUTES: '301' },
    { EVENT: 'workflow_dispatch', SUITES: 'locks,admission', MINUTES: '200' },
    { ...REPRO, REPRO_FILTER: '' },
    { ...REPRO, REPRO_PACKAGE: 'x y' },
    { ...REPRO, REPRO_RUNS: '0' },
    { ...REPRO, REPRO_COPIES: '9' },
  ]) {
    assert.throws(() => plan(env, new Date()), PlanError, JSON.stringify(env));
  }
  const [good] = plan(REPRO, new Date()).suites;
  for (const bad of [
    null,
    [],
    [{ ...good, name: 'other' }],
    [{ ...good, until: 'budget' }],
    [{ ...good, repro: { ...good.repro, filter: '--ignored' } }],
    [{ name: 'locks', minutes: 30, minimum: 5, until: 'budget', repro: good.repro }],
    [{ name: 'real-media', slot: true, items: ['RM01'] }],
    [{ name: 'locks', minutes: 301, minimum: 5, until: 'budget' }],
  ]) {
    assert.throws(() => validatePlan(bad), Error, JSON.stringify(bad));
  }
});
