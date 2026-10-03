'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const container = require('../lib/container.cjs');
const verdict = require('../lib/load-verdict.cjs');

const SPEC = {
  image: 'vsift-worker:p14',
  name: 'vsift-test',
  command: ['vsift', '--version'],
  cpus: 4,
  memory: '12g',
  volumes: ['/var/lib/vsift:/var/lib/vsift:rw', '/srv/vsift/inputs:/srv/vsift/inputs:ro'],
};

test('the container is the runbook\'s hardened one: no network, read-only root, limits, no capabilities, no new privileges', () => {
  const args = container.runArguments(SPEC);
  const has = (...words) => words.every((word, index) => args[args.indexOf(words[0]) + index] === word);
  assert.equal(args[0], 'run');
  assert.ok(args.includes('--init'));
  assert.ok(has('--network', 'none'));
  assert.ok(args.includes('--read-only'));
  assert.ok(has('--cpus', '4'));
  assert.ok(has('--memory', '12g'));
  assert.ok(has('--memory-swap', '12g'));
  assert.ok(has('--pids-limit', '256'));
  assert.ok(has('--cap-drop', 'ALL'));
  assert.ok(has('--security-opt', 'no-new-privileges'));
  assert.ok(has('--user', '10001:10001'));
  assert.ok(has('--stop-timeout', '45'));
  assert.ok(has('--tmpfs', '/tmp:rw,noexec,nosuid,size=64m'));
  assert.ok(args.includes('HOME=/var/lib/vsift/home'));
  assert.ok(args.includes('XDG_CONFIG_HOME=/var/lib/vsift/home/.config'));
  assert.deepEqual(args.slice(-3), ['vsift-worker:p14', 'vsift', '--version']);
  assert.ok(!args.includes('--rm'));
  assert.ok(container.runArguments({ ...SPEC, remove: true }).includes('--rm'));
});

test('only the install gets a network, and only when it says so; nothing is privileged or mounts the engine socket', () => {
  assert.ok(container.runArguments({ ...SPEC, network: 'bridge' }).includes('bridge'));
  for (const args of [container.runArguments(SPEC), container.runArguments({ ...SPEC, network: 'bridge' })]) {
    assert.ok(!args.includes('--privileged'));
    assert.ok(!args.some((word) => /docker\.sock/.test(word)));
    assert.ok(!args.includes('--network=host') && !args.some((word, index) => word === '--network' && args[index + 1] === 'host'));
  }
});

test('extra environment and options come before the image', () => {
  const args = container.runArguments({ ...SPEC, env: { P14_SENTINEL: 'canary' }, extra: ['--label', 'x=y'] });
  assert.ok(args.includes('P14_SENTINEL=canary'));
  assert.ok(args.indexOf('--label') < args.indexOf('vsift-worker:p14'));
});

test('a resource curve is checked against the plan\'s thresholds', () => {
  const growth = (monotonic) => ({ monotonic, reason: monotonic ? 'rising' : 'flat', maxima: [1, 2, 3, 4] });
  const curve = (rss, monotonic = false) => ({ samples: 100, seconds: 100, coordinatorRssPeakMiB: rss, coordinatorRssGrowth: growth(monotonic), coordinatorFdsPeak: 20, coordinatorFdsGrowth: growth(false), containerMemoryPeakMiB: 900 });
  assert.ok(verdict.resourceChecks('r', curve(255)).every((entry) => entry.ok));
  assert.equal(verdict.resourceChecks('r', curve(257))[0].ok, false);
  assert.equal(verdict.resourceChecks('r', curve(100, true))[1].ok, false);
  assert.equal(verdict.resourceChecks('r', null)[0].ok, false);
  assert.equal(verdict.resourceChecks('r', { samples: 0 })[0].ok, false);
});

test('a stream is checked for readiness, isolation, concurrency and leaks', () => {
  const run = (over = {}) => ({
    problems: [], leaks: [], expectedConcurrency: 4, expectedCapacity: 8,
    batch: { started: { isolation: 'strict_linux', concurrency: 4, admission_capacity: 8 }, peakRunning: 4 },
    ...over,
  });
  assert.ok(verdict.streamChecks('b', run()).every((entry) => entry.ok));
  assert.equal(verdict.streamChecks('b', run({ leaks: ['/srv/vsift'] }))[4].ok, false);
  assert.equal(verdict.streamChecks('b', run({ batch: { started: { isolation: 'process_only', concurrency: 4, admission_capacity: 8 }, peakRunning: 4 } }))[1].ok, false);
  assert.equal(verdict.streamChecks('b', run({ batch: { started: { isolation: 'strict_linux', concurrency: 4, admission_capacity: 8 }, peakRunning: 5 } }))[3].ok, false);
  assert.equal(verdict.streamChecks('b', run({ problems: ['sequence 1 does not follow 1'] }))[0].ok, false);
});

test('the page check needs enough calls and a p95 within 250 ms', () => {
  assert.equal(verdict.pageCheck(new Array(200).fill(100), 250).ok, true);
  assert.equal(verdict.pageCheck(new Array(200).fill(100), 251).ok, false);
  assert.equal(verdict.pageCheck(new Array(10).fill(100), 100).ok, false);
});

test('the markdown names each phase and flags the checks that failed', () => {
  const phases = [
    { name: 'One', checks: [verdict.check('a', true, 'fine')] },
    { name: 'Two', measurements: ['2 requests'], checks: [verdict.check('b', false, 'bad | pipe')] },
  ];
  assert.equal(verdict.allHold(phases), false);
  const text = verdict.renderMarkdown('Title', phases);
  assert.match(text, /### One: held/);
  assert.match(text, /### Two: DID NOT HOLD/);
  assert.match(text, /\*\*FAIL\*\*/);
  assert.match(text, /bad \\\| pipe/);
  assert.match(text, /2 requests/);
});
