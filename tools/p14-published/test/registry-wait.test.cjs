'use strict';

// The decision logic of the shared registry wait (#618), with a virtual clock and a fake
// registry: no network and no real time.

const test = require('node:test');
const assert = require('node:assert/strict');

const { PACKAGES, QualificationError } = require('../lib/common.cjs');
const { POLL_INTERVAL_MS, WAIT_LIMIT_MS, headStatus, tarballUrl, waitForRegistry } = require('../lib/registry-wait.cjs');

const VERSION = '0.3.0-rc.1';

/**
 * A registry that serves each package from the given virtual time on (a package that is not
 * listed is served from the start; `null` means never) and a clock that only `sleep` moves.
 */
function harness(servedFrom = {}) {
  const clock = { time: 0 };
  const requests = [];
  const probe = async (url) => {
    requests.push({ url, at: clock.time });
    const name = PACKAGES.find((candidate) => url === tarballUrl(candidate, VERSION));
    assert.ok(name !== undefined, `an unexpected address: ${url}`);
    const from = name in servedFrom ? servedFrom[name] : 0;
    return from !== null && clock.time >= from ? 200 : 404;
  };
  const sleeps = [];
  const sleep = async (milliseconds) => {
    sleeps.push(milliseconds);
    clock.time += milliseconds;
  };
  const lines = [];
  return { clock, requests, sleeps, lines, options: { version: VERSION, probe, sleep, now: () => clock.time, log: (line) => lines.push(line) } };
}

test('the tarball addresses are the registry\'s, scoped and unscoped', () => {
  assert.equal(tarballUrl('vsift-cli', VERSION), 'https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.3.0-rc.1.tgz');
  assert.equal(tarballUrl('@vsift/win32-x64', VERSION), 'https://registry.npmjs.org/@vsift/win32-x64/-/win32-x64-0.3.0-rc.1.tgz');
  assert.equal(tarballUrl('@vsift/darwin-arm64', '0.2.4'), 'https://registry.npmjs.org/@vsift/darwin-arm64/-/darwin-arm64-0.2.4.tgz');
  assert.equal(tarballUrl('@vsift/linux-x64', '1.0.0'), 'https://registry.npmjs.org/@vsift/linux-x64/-/linux-x64-1.0.0.tgz');
});

test('a version that is not a version, and a package name that could change the address, are refused', () => {
  assert.throws(() => tarballUrl('vsift-cli', '0.3.0/../x'), QualificationError);
  assert.throws(() => tarballUrl('vsift-cli', ''), QualificationError);
  assert.throws(() => tarballUrl('vsift-cli/../other', VERSION), QualificationError);
  assert.throws(() => tarballUrl('vsift-cli?x=1', VERSION), QualificationError);
});

test('the wait covers the launcher and the three native packages, every 30 seconds for 15 minutes', () => {
  assert.deepEqual([...PACKAGES].sort(), ['@vsift/darwin-arm64', '@vsift/linux-x64', '@vsift/win32-x64', 'vsift-cli']);
  assert.equal(POLL_INTERVAL_MS, 30_000);
  assert.equal(WAIT_LIMIT_MS, 900_000);
});

test('all four present: one round of requests, no sleep', async () => {
  const { options, requests, sleeps } = harness();
  const result = await waitForRegistry(options);
  assert.deepEqual(result, { attempts: 1, waitedMs: 0 });
  assert.equal(requests.length, 4);
  assert.deepEqual(sleeps, []);
});

test('one missing and then present: it waits, asks only for what was missing, and then returns', async () => {
  const { options, requests, sleeps, lines } = harness({ '@vsift/win32-x64': 100_000 });
  const result = await waitForRegistry(options);
  // Served from 100 s on: seen at the attempt at 120 s (attempts at 0, 30, 60, 90, 120).
  assert.deepEqual(result, { attempts: 5, waitedMs: 120_000 });
  assert.deepEqual(sleeps, [30_000, 30_000, 30_000, 30_000]);
  assert.equal(requests.length, 4 + 4, 'four in the first round, then only the Windows package');
  assert.ok(requests.slice(4).every((request) => request.url === tarballUrl('@vsift/win32-x64', VERSION)));
  assert.ok(lines.some((line) => line.includes('@vsift/win32-x64@0.3.0-rc.1 (404)')), 'the log names what is missing and the answer');
  assert.ok(lines.at(-1).startsWith('all 4 tarballs of 0.3.0-rc.1 answer 200'));
});

test('a package that answered once is not asked again, even if a later answer would differ', async () => {
  const clock = { time: 0 };
  const seen = [];
  const probe = async (url) => {
    seen.push(url);
    return url === tarballUrl('vsift-cli', VERSION) && seen.length > 4 ? 503 : url === tarballUrl('@vsift/linux-x64', VERSION) && clock.time < 30_000 ? 404 : 200;
  };
  const sleep = async (milliseconds) => {
    clock.time += milliseconds;
  };
  const result = await waitForRegistry({ version: VERSION, probe, sleep, now: () => clock.time });
  assert.equal(result.attempts, 2);
  assert.equal(seen.filter((url) => url === tarballUrl('vsift-cli', VERSION)).length, 1);
});

test('still missing at the limit: it fails naming each missing package and its last answer, after exactly 15 minutes', async () => {
  const { options, requests, sleeps } = harness({ '@vsift/win32-x64': null, '@vsift/darwin-arm64': null });
  await assert.rejects(waitForRegistry(options), (error) => {
    assert.ok(error instanceof QualificationError);
    assert.match(error.message, /@vsift\/win32-x64@0\.3\.0-rc\.1 \(404\)/);
    assert.match(error.message, /@vsift\/darwin-arm64@0\.3\.0-rc\.1 \(404\)/);
    assert.doesNotMatch(error.message, /vsift-cli@|linux-x64@/, 'a package that is served is not named');
    assert.match(error.message, /after 900 s \(31 attempts, limit 900 s\)/);
    return true;
  });
  assert.equal(sleeps.length, 30);
  assert.ok(sleeps.every((milliseconds) => milliseconds === 30_000));
  assert.equal(requests.at(-1).at, 900_000, 'the last request is at the limit, none after it');
});

test('a package served exactly at the limit still passes; one served a moment later fails', async () => {
  const onTime = harness({ 'vsift-cli': 900_000 });
  assert.equal((await waitForRegistry(onTime.options)).waitedMs, 900_000);
  const late = harness({ 'vsift-cli': 900_001 });
  await assert.rejects(waitForRegistry(late.options), /vsift-cli@0\.3\.0-rc\.1/);
});

test('the interval and the limit can be shortened, and a zero limit asks exactly once', async () => {
  const quick = harness({ 'vsift-cli': 3_000 });
  const result = await waitForRegistry({ ...quick.options, intervalMs: 1_000, limitMs: 5_000 });
  assert.deepEqual(result, { attempts: 4, waitedMs: 3_000 });
  const once = harness({ 'vsift-cli': null });
  await assert.rejects(waitForRegistry({ ...once.options, limitMs: 0 }), /\(1 attempt, limit 0 s\)/);
  assert.deepEqual(once.sleeps, []);
});

test('an answer that is not a number (no connection, a timeout) counts as not yet and is reported', async () => {
  const clock = { time: 0 };
  const probe = async () => (clock.time < 60_000 ? 'no answer: ECONNRESET' : 200);
  const sleep = async (milliseconds) => {
    clock.time += milliseconds;
  };
  const result = await waitForRegistry({ version: VERSION, probe, sleep, now: () => clock.time });
  assert.equal(result.waitedMs, 60_000);
  const never = async () => 'no answer within 20 s';
  await assert.rejects(waitForRegistry({ version: VERSION, probe: never, sleep, now: () => clock.time, limitMs: 0 }), /no answer within 20 s/);
});

test('only 200 counts: a redirect, a 403 and a 5xx are all not yet', async () => {
  for (const status of [301, 302, 403, 404, 500, 503]) {
    const clock = { time: 0 };
    await assert.rejects(
      waitForRegistry({ version: VERSION, probe: async () => status, sleep: async () => {}, now: () => clock.time, limitMs: 0 }),
      new RegExp(`\\(${status}\\)`),
    );
  }
});

test('the real request is a HEAD with the neutral identifier and nothing else, and never follows a redirect', async () => {
  let seen;
  const status = await headStatus('https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.3.0-rc.1.tgz', async (url, options) => {
    seen = { url, options };
    return { status: 200 };
  });
  assert.equal(status, 200);
  assert.equal(seen.options.method, 'HEAD');
  assert.equal(seen.options.redirect, 'manual');
  assert.deepEqual(seen.options.headers, { 'user-agent': 'vsift-p14-qualification' });
  assert.deepEqual(Object.keys(seen.options).sort(), ['headers', 'method', 'redirect', 'signal']);
});

test('a refused connection and a timeout come back as a description, never as a thrown error', async () => {
  const refused = await headStatus('https://registry.npmjs.org/x', async () => {
    throw Object.assign(new TypeError('fetch failed'), { cause: { code: 'ECONNREFUSED' } });
  });
  assert.equal(refused, 'no answer: ECONNREFUSED');
  const timedOut = await headStatus('https://registry.npmjs.org/x', async () => {
    throw Object.assign(new Error('the operation timed out'), { name: 'TimeoutError' });
  });
  assert.equal(timedOut, 'no answer within 20 s');
});

// ---------------------------------------------------------------- where the wait is used

const fs = require('node:fs');
const path = require('node:path');

const workflow = (name) => fs.readFileSync(path.join(__dirname, '..', '..', '..', '.github', 'workflows', name), 'utf8');

test('every post-publish check waits for the registry before the step that packs or installs', () => {
  for (const [file, waiting, consumer] of [
    ['p14-verify-release.yml', 'wait-for-registry.cjs --version "${VERSION}"', 'verify-release.cjs'],
    ['p14-published-artifacts.yml', 'wait-for-registry.cjs --version "${VERSION}"', 'clean-install.cjs'],
    ['p14-journeys.yml', 'wait-for-registry.cjs --version "$VERSION"', 'p14_journeys.py install'],
  ]) {
    const text = workflow(file);
    const wait = text.indexOf(waiting);
    assert.ok(wait !== -1, `${file} has the wait`);
    assert.equal(text.indexOf(waiting, wait + 1), -1, `${file} waits once`);
    assert.ok(wait < text.indexOf(consumer), `${file}: the wait comes before ${consumer}`);
  }
});

test('in the published-artifacts workflow the wait is in the job every installing job needs', () => {
  const text = workflow('p14-published-artifacts.yml');
  const resolveJob = /^  resolve:\n[\s\S]*?(?=^  clean-install:)/m.exec(text);
  assert.ok(resolveJob, 'the resolve job');
  assert.match(resolveJob[0], /wait-for-registry\.cjs/);
  for (const job of ['clean-install', 'upgrade']) {
    const start = text.indexOf(`\n  ${job}:\n`);
    assert.ok(start !== -1, `the ${job} job`);
    const header = text.slice(start, text.indexOf('\n    steps:', start));
    assert.ok(header.includes('\n    needs:\n      - resolve\n'), `${job} needs resolve`);
  }
});

test('the wait changes nothing and holds no credential: no publish, no token, no write', () => {
  const script = fs.readFileSync(path.join(__dirname, '..', 'wait-for-registry.cjs'), 'utf8') + fs.readFileSync(path.join(__dirname, '..', 'lib', 'registry-wait.cjs'), 'utf8');
  assert.doesNotMatch(script, /npm publish|dist-tag|GH_TOKEN|GITHUB_TOKEN|NPM_TOKEN|authorization/i);
  assert.match(script, /method: 'HEAD'/);
});
