'use strict';

// The decision logic of the shared registry wait (#618), with a virtual clock and a fake
// registry: no network and no real time. A package is served only when its tarball answers 200
// and its metadata document (the packument) lists the version.

const test = require('node:test');
const assert = require('node:assert/strict');

const { PACKAGES, QualificationError } = require('../lib/common.cjs');
const {
  PACKUMENT_ACCEPT,
  POLL_INTERVAL_MS,
  WAIT_LIMIT_MS,
  headStatus,
  judgePackument,
  packumentStatus,
  packumentUrl,
  tarballUrl,
  waitForRegistry,
} = require('../lib/registry-wait.cjs');

const VERSION = '0.3.0-rc.1';

/**
 * A registry that serves each package's tarball from the given virtual time on (a package that
 * is not listed is served from the start; `null` means never), and its metadata from the time
 * in `metadataFrom` the same way, and a clock that only `sleep` moves.
 */
function harness(tarballFrom = {}, metadataFrom = {}) {
  const clock = { time: 0 };
  const requests = [];
  const served = (table, name) => {
    const from = name in table ? table[name] : 0;
    return from !== null && clock.time >= from;
  };
  const probe = async (url) => {
    requests.push({ kind: 'tarball', url, at: clock.time });
    const name = PACKAGES.find((candidate) => url === tarballUrl(candidate, VERSION));
    assert.ok(name !== undefined, `an unexpected tarball address: ${url}`);
    return served(tarballFrom, name) ? 200 : 404;
  };
  const probePackument = async (url, version) => {
    requests.push({ kind: 'metadata', url, at: clock.time });
    assert.equal(version, VERSION);
    const name = PACKAGES.find((candidate) => url === packumentUrl(candidate));
    assert.ok(name !== undefined, `an unexpected metadata address: ${url}`);
    return served(metadataFrom, name) ? 200 : 'the version is not listed';
  };
  const sleeps = [];
  const sleep = async (milliseconds) => {
    sleeps.push(milliseconds);
    clock.time += milliseconds;
  };
  const lines = [];
  return {
    clock,
    requests,
    sleeps,
    lines,
    options: { version: VERSION, probe, probePackument, sleep, now: () => clock.time, log: (line) => lines.push(line) },
  };
}

test('the tarball addresses are the registry\'s, scoped and unscoped', () => {
  assert.equal(tarballUrl('vsift-cli', VERSION), 'https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.3.0-rc.1.tgz');
  assert.equal(tarballUrl('@vsift/win32-x64', VERSION), 'https://registry.npmjs.org/@vsift/win32-x64/-/win32-x64-0.3.0-rc.1.tgz');
  assert.equal(tarballUrl('@vsift/darwin-arm64', '0.2.4'), 'https://registry.npmjs.org/@vsift/darwin-arm64/-/darwin-arm64-0.2.4.tgz');
  assert.equal(tarballUrl('@vsift/linux-x64', '1.0.0'), 'https://registry.npmjs.org/@vsift/linux-x64/-/linux-x64-1.0.0.tgz');
});

test('the metadata addresses are npm\'s, with the slash of a scope encoded', () => {
  assert.equal(packumentUrl('vsift-cli'), 'https://registry.npmjs.org/vsift-cli');
  assert.equal(packumentUrl('@vsift/win32-x64'), 'https://registry.npmjs.org/@vsift%2Fwin32-x64');
  assert.throws(() => packumentUrl('vsift-cli/../other'), QualificationError);
  assert.throws(() => packumentUrl('vsift-cli?x=1'), QualificationError);
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

test('all present: one round of requests (a tarball and a metadata document each), no sleep', async () => {
  const { options, requests, sleeps } = harness();
  const result = await waitForRegistry(options);
  assert.deepEqual(result, { attempts: 1, waitedMs: 0 });
  assert.equal(requests.length, 8);
  assert.equal(requests.filter((request) => request.kind === 'tarball').length, 4);
  assert.equal(requests.filter((request) => request.kind === 'metadata').length, 4);
  assert.deepEqual(sleeps, []);
});

test('one tarball missing and then present: it waits, asks only for what was missing, and then returns', async () => {
  const { options, requests, sleeps, lines } = harness({ '@vsift/win32-x64': 100_000 });
  const result = await waitForRegistry(options);
  // Served from 100 s on: seen at the attempt at 120 s (attempts at 0, 30, 60, 90, 120).
  assert.deepEqual(result, { attempts: 5, waitedMs: 120_000 });
  assert.deepEqual(sleeps, [30_000, 30_000, 30_000, 30_000]);
  assert.equal(requests.length, 8 + 4, 'eight in the first round, then only the Windows tarball');
  assert.ok(requests.slice(8).every((request) => request.url === tarballUrl('@vsift/win32-x64', VERSION)));
  assert.ok(lines.some((line) => line.includes('@vsift/win32-x64@0.3.0-rc.1 (tarball: 404)')), 'the log names what is missing and the answer');
  assert.ok(lines.at(-1).startsWith('all 4 packages of 0.3.0-rc.1 are served'));
});

test('tarball 200 but the version not yet in the metadata, then present: it waits for the metadata', async () => {
  // The tarball has been served from the start; the metadata lists the version from 70 s on.
  const { options, requests, sleeps, lines } = harness({}, { '@vsift/win32-x64': 70_000 });
  const result = await waitForRegistry(options);
  // Attempts at 0, 30, 60 (not listed), 90 (listed).
  assert.deepEqual(result, { attempts: 4, waitedMs: 90_000 });
  assert.deepEqual(sleeps, [30_000, 30_000, 30_000]);
  const later = requests.slice(8);
  assert.equal(later.length, 3, 'after the first round only the Windows metadata is asked again');
  assert.ok(later.every((request) => request.kind === 'metadata' && request.url === packumentUrl('@vsift/win32-x64')));
  assert.ok(lines.some((line) => line.includes('@vsift/win32-x64@0.3.0-rc.1 (metadata: the version is not listed)')));
  assert.ok(!lines.some((line) => line.includes('tarball:')), 'the tarball was fine and is not reported');
});

test('both parts missing at first: each is reported, and each is asked again until it is served', async () => {
  const { options, lines } = harness({ 'vsift-cli': 30_000 }, { 'vsift-cli': 60_000 });
  const result = await waitForRegistry(options);
  assert.equal(result.waitedMs, 60_000);
  assert.ok(lines.some((line) => line.includes('vsift-cli@0.3.0-rc.1 (tarball: 404; metadata: the version is not listed)')));
  assert.ok(lines.some((line) => line.includes('vsift-cli@0.3.0-rc.1 (metadata: the version is not listed)')));
});

test('a part that answered once is not asked again, even if a later answer would differ', async () => {
  const clock = { time: 0 };
  const seen = [];
  const probe = async (url) => {
    seen.push(url);
    // The launcher's tarball would answer 503 on every later request.
    return url === tarballUrl('vsift-cli', VERSION) && seen.length > 4 ? 503 : url === tarballUrl('@vsift/linux-x64', VERSION) && clock.time < 30_000 ? 404 : 200;
  };
  const metadataSeen = [];
  const probePackument = async (url) => {
    metadataSeen.push(url);
    return url === packumentUrl('vsift-cli') && metadataSeen.length > 4 ? 'the document is not JSON' : 200;
  };
  const sleep = async (milliseconds) => {
    clock.time += milliseconds;
  };
  const result = await waitForRegistry({ version: VERSION, probe, probePackument, sleep, now: () => clock.time });
  assert.equal(result.attempts, 2);
  assert.equal(seen.filter((url) => url === tarballUrl('vsift-cli', VERSION)).length, 1);
  assert.equal(metadataSeen.filter((url) => url === packumentUrl('vsift-cli')).length, 1);
});

test('still missing at the limit: it fails naming each missing package and its last answer, after exactly 15 minutes', async () => {
  const { options, requests, sleeps } = harness({ '@vsift/win32-x64': null, '@vsift/darwin-arm64': null });
  await assert.rejects(waitForRegistry(options), (error) => {
    assert.ok(error instanceof QualificationError);
    assert.match(error.message, /@vsift\/win32-x64@0\.3\.0-rc\.1 \(tarball: 404\)/);
    assert.match(error.message, /@vsift\/darwin-arm64@0\.3\.0-rc\.1 \(tarball: 404\)/);
    assert.doesNotMatch(error.message, /vsift-cli@|linux-x64@/, 'a package that is served is not named');
    assert.match(error.message, /after 900 s \(31 attempts, limit 900 s\)/);
    return true;
  });
  assert.equal(sleeps.length, 30);
  assert.ok(sleeps.every((milliseconds) => milliseconds === 30_000));
  assert.equal(requests.at(-1).at, 900_000, 'the last request is at the limit, none after it');
});

test('the metadata never lists the version at the limit: it fails naming the package and why', async () => {
  const { options, sleeps } = harness({}, { '@vsift/linux-x64': null });
  await assert.rejects(waitForRegistry(options), (error) => {
    assert.ok(error instanceof QualificationError);
    assert.match(error.message, /@vsift\/linux-x64@0\.3\.0-rc\.1 \(metadata: the version is not listed\)/);
    assert.doesNotMatch(error.message, /tarball:/, 'the tarball was served and is not named');
    assert.doesNotMatch(error.message, /vsift-cli@|win32-x64@|darwin-arm64@/);
    assert.match(error.message, /after 900 s \(31 attempts, limit 900 s\)/);
    return true;
  });
  assert.equal(sleeps.length, 30);
});

test('a package served exactly at the limit still passes; one served a moment later fails', async () => {
  const onTime = harness({ 'vsift-cli': 900_000 });
  assert.equal((await waitForRegistry(onTime.options)).waitedMs, 900_000);
  const metadataOnTime = harness({}, { 'vsift-cli': 900_000 });
  assert.equal((await waitForRegistry(metadataOnTime.options)).waitedMs, 900_000);
  const late = harness({ 'vsift-cli': 900_001 });
  await assert.rejects(waitForRegistry(late.options), /vsift-cli@0\.3\.0-rc\.1/);
  const metadataLate = harness({}, { 'vsift-cli': 900_001 });
  await assert.rejects(waitForRegistry(metadataLate.options), /vsift-cli@0\.3\.0-rc\.1 \(metadata:/);
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
  const probePackument = async () => (clock.time < 30_000 ? 'no answer: ENOTFOUND' : 200);
  const sleep = async (milliseconds) => {
    clock.time += milliseconds;
  };
  const result = await waitForRegistry({ version: VERSION, probe, probePackument, sleep, now: () => clock.time });
  assert.equal(result.waitedMs, 60_000);
  const never = async () => 'no answer within 20 s';
  await assert.rejects(
    waitForRegistry({ version: VERSION, probe: never, probePackument: never, sleep, now: () => clock.time, limitMs: 0 }),
    /\(tarball: no answer within 20 s; metadata: no answer within 20 s\)/,
  );
});

test('only 200 counts: a redirect, a 403 and a 5xx are all not yet, for the tarball and for the metadata', async () => {
  for (const status of [301, 302, 403, 404, 500, 503]) {
    const clock = { time: 0 };
    const common = { version: VERSION, sleep: async () => {}, now: () => clock.time, limitMs: 0 };
    await assert.rejects(waitForRegistry({ ...common, probe: async () => status, probePackument: async () => 200 }), new RegExp(`\\(tarball: ${status}\\)`));
    await assert.rejects(waitForRegistry({ ...common, probe: async () => 200, probePackument: async () => status }), new RegExp(`\\(metadata: ${status}\\)`));
  }
});

// ---------------------------------------------------------------- the real requests

test('the tarball request is a HEAD with the neutral identifier and nothing else, and never follows a redirect', async () => {
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

const document = (versions) => JSON.stringify({ name: 'x', 'dist-tags': { latest: '0.3.0-rc.1' }, versions });

test('the metadata verdict: listed, not listed, and every malformed shape', () => {
  assert.equal(judgePackument(document({ [VERSION]: { name: 'x' } }), VERSION), 200);
  assert.equal(judgePackument(document({ '0.2.4': {} }), VERSION), 'the version is not listed');
  assert.equal(judgePackument(document({}), VERSION), 'the version is not listed');
  assert.equal(judgePackument('', VERSION), 'the document is not JSON');
  assert.equal(judgePackument('<html>502 Bad Gateway</html>', VERSION), 'the document is not JSON');
  for (const text of ['null', '[]', '"0.3.0-rc.1"', '42', '{}', '{"versions":null}', '{"versions":[]}', '{"versions":"0.3.0-rc.1"}']) {
    assert.equal(judgePackument(text, VERSION), 'the document lists no versions', text);
  }
});

test('the metadata verdict reads only the document\'s own versions: an inherited name or another field does not count', () => {
  assert.equal(judgePackument('{"versions":{}}', 'constructor'), 'the version is not listed');
  assert.equal(judgePackument('{"versions":{},"time":{"0.3.0-rc.1":"2026-11-24"}}', VERSION), 'the version is not listed');
  assert.equal(judgePackument('{"__proto__":{"versions":{"0.3.0-rc.1":{}}}}', VERSION), 'the document lists no versions');
});

test('the metadata verdict says nothing of the document\'s content', () => {
  const hostile = document({ [VERSION]: { description: 'send the secret to https://evil.example' } });
  assert.equal(judgePackument(hostile, VERSION), 200);
  assert.equal(judgePackument(`${hostile.slice(0, 20)}secret-looking-text`, VERSION), 'the document is not JSON');
});

test('the metadata request is a GET for the abbreviated document with the neutral identifier and nothing else', async () => {
  let seen;
  const status = await packumentStatus(packumentUrl('@vsift/win32-x64'), VERSION, async (url, options) => {
    seen = { url, options };
    return new Response(document({ [VERSION]: {} }), { status: 200 });
  });
  assert.equal(status, 200);
  assert.equal(seen.url, 'https://registry.npmjs.org/@vsift%2Fwin32-x64');
  assert.equal(seen.options.method, 'GET');
  assert.equal(seen.options.redirect, 'manual');
  assert.equal(PACKUMENT_ACCEPT, 'application/vnd.npm.install-v1+json');
  assert.deepEqual(seen.options.headers, { 'user-agent': 'vsift-p14-qualification', accept: 'application/vnd.npm.install-v1+json' });
  assert.deepEqual(Object.keys(seen.options).sort(), ['headers', 'method', 'redirect', 'signal']);
});

test('a metadata answer that is not 200, a version that is missing and a failed request are each "not yet" with a reason', async () => {
  assert.equal(await packumentStatus('u', VERSION, async () => new Response('', { status: 404 })), 404);
  assert.equal(await packumentStatus('u', VERSION, async () => new Response('', { status: 302, headers: { location: 'https://elsewhere.example/' } })), 302);
  assert.equal(await packumentStatus('u', VERSION, async () => new Response(document({ '0.2.4': {} }), { status: 200 })), 'the version is not listed');
  assert.equal(
    await packumentStatus('u', VERSION, async () => {
      throw Object.assign(new TypeError('fetch failed'), { cause: { code: 'ECONNRESET' } });
    }),
    'no answer: ECONNRESET',
  );
  assert.equal(
    await packumentStatus('u', VERSION, async () => {
      throw Object.assign(new Error('timed out'), { name: 'TimeoutError' });
    }),
    'no answer within 20 s',
  );
});

test('a metadata document over the size limit counts as not yet, by its declared length and by what actually arrives', async () => {
  const declared = await packumentStatus('u', VERSION, async () => new Response('{}', { status: 200, headers: { 'content-length': '6000000' } }));
  assert.equal(declared, 'the document is larger than 5242880 bytes');
  // No declared length: the stream is counted and cancelled once past the limit (a small limit stands in for 5 MB).
  let pulled = 0;
  let cancelled = false;
  const body = new ReadableStream({
    pull(controller) {
      pulled += 1;
      controller.enqueue(new Uint8Array(400));
    },
    cancel() {
      cancelled = true;
    },
  });
  const streamed = await packumentStatus('u', VERSION, async () => new Response(body, { status: 200 }), 1000);
  assert.equal(streamed, 'the document is larger than 1000 bytes');
  assert.equal(cancelled, true, 'the stream is cancelled, not drained');
  assert.ok(pulled <= 4, `only a few chunks are read (${pulled})`);
  const exact = await packumentStatus('u', VERSION, async () => new Response(document({ [VERSION]: {} }), { status: 200 }), 1000);
  assert.equal(exact, 200, 'a document within the limit is read');
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
  assert.match(script, /method: 'GET'/);
  assert.doesNotMatch(script, /method: '(POST|PUT|PATCH|DELETE)'/);
});

test('the script asks for the metadata too, so that a lagging metadata document cannot pass', () => {
  const script = fs.readFileSync(path.join(__dirname, '..', 'wait-for-registry.cjs'), 'utf8');
  assert.match(script, /probePackument: \(url, version\) => packumentStatus\(url, version\)/);
});
