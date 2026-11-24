'use strict';

// Waiting until the public npm registry serves a just-published version (#618).
//
// The post-publish checks (`P14 verify release`, `P14 published artifacts`, `P14 journeys`)
// install or pack the published packages. Dispatched a couple of minutes after the publish
// job ended, three of them failed on Windows on 2026-11-24 because registry.npmjs.org was not
// yet serving `@vsift/win32-x64@0.3.0-rc.1`: `npm pack` got E404 and an install silently
// skipped the optional native package. A later HEAD on all four tarballs answered 200, so the
// publish was good; the checks had only run too early.
//
// This module holds the decision logic of the one shared wait that every such job runs
// before it installs or packs: poll the four packages' tarball URLs until all of them
// answer 200, every 30 seconds, for at most 15 minutes, then fail naming what is missing.
// The clock, the sleep and the request are passed in, so the tests run with no network and
// no real time. Requests carry only the neutral identifier of lib/common.cjs
// (`USER_AGENT`): no person, machine or credential.

const { PACKAGES, QualificationError, USER_AGENT, expect } = require('./common.cjs');
const { parseVersion } = require('./release.cjs');

/** The registry the published packages are read from, with no trailing slash. */
const REGISTRY_ORIGIN = 'https://registry.npmjs.org';
/** How often the registry is asked again. */
const POLL_INTERVAL_MS = 30_000;
/** How long the wait lasts at most (the issue's 15 minutes). */
const WAIT_LIMIT_MS = 15 * 60_000;
/** The most one request may take before it counts as "not yet". */
const REQUEST_TIMEOUT_MS = 20_000;

/**
 * The address of a package's tarball on the registry: `@vsift/win32-x64` at `0.3.0-rc.1`
 * is `https://registry.npmjs.org/@vsift/win32-x64/-/win32-x64-0.3.0-rc.1.tgz`, and the
 * unscoped launcher is `.../vsift-cli/-/vsift-cli-0.3.0-rc.1.tgz`.
 */
function tarballUrl(packageName, version) {
  parseVersion(version);
  expect(/^(@[a-z0-9-]+\/)?[a-z0-9][a-z0-9-]*$/.test(packageName), `${JSON.stringify(packageName)} is not a package name of this project`);
  const base = packageName.slice(packageName.lastIndexOf('/') + 1);
  return `${REGISTRY_ORIGIN}/${packageName}/-/${base}-${version}.tgz`;
}

function plural(count, word) {
  return `${count} ${word}${count === 1 ? '' : 's'}`;
}

/**
 * Waits until every package's tarball answers 200 (a package that answered once is not asked
 * again). Any other answer, a refused connection and a timeout all mean "not yet".
 *
 * @param {object} options
 * @param {string} options.version The published version.
 * @param {(url: string) => Promise<number|string>} options.probe The HTTP status of a URL, or a
 *   description of why there was none.
 * @param {(milliseconds: number) => Promise<void>} options.sleep
 * @param {() => number} options.now Milliseconds, any origin.
 * @param {(line: string) => void} [options.log]
 * @param {readonly string[]} [options.packages]
 * @param {number} [options.intervalMs]
 * @param {number} [options.limitMs]
 * @returns {Promise<{attempts: number, waitedMs: number}>}
 * @throws {QualificationError} naming every package still missing, with its last answer.
 */
async function waitForRegistry({
  version,
  probe,
  sleep,
  now,
  log = () => {},
  packages = PACKAGES,
  intervalMs = POLL_INTERVAL_MS,
  limitMs = WAIT_LIMIT_MS,
}) {
  expect(Number.isFinite(intervalMs) && intervalMs > 0, 'the polling interval must be positive');
  expect(Number.isFinite(limitMs) && limitMs >= 0, 'the wait limit must not be negative');
  const urls = new Map(packages.map((name) => [name, tarballUrl(name, version)]));
  const lastAnswer = new Map();
  let missing = [...packages];
  const started = now();
  let attempts = 0;
  for (;;) {
    attempts += 1;
    const stillMissing = [];
    for (const name of missing) {
      const answer = await probe(urls.get(name));
      lastAnswer.set(name, answer);
      if (answer !== 200) {
        stillMissing.push(name);
      }
    }
    missing = stillMissing;
    const waitedMs = now() - started;
    if (missing.length === 0) {
      log(`all ${packages.length} tarballs of ${version} answer 200 (after ${Math.round(waitedMs / 1000)} s, ${plural(attempts, 'attempt')})`);
      return { attempts, waitedMs };
    }
    const description = missing.map((name) => `${name}@${version} (${lastAnswer.get(name)})`).join(', ');
    // Another poll would start after the limit: stop now and say what is still missing.
    if (waitedMs + intervalMs > limitMs) {
      throw new QualificationError(
        `the registry still does not serve ${description} after ${Math.round(waitedMs / 1000)} s ` +
          `(${plural(attempts, 'attempt')}, limit ${Math.round(limitMs / 1000)} s). ` +
          'npm had not yet served the tarball, or the publish did not include it: read the Release run before re-running.',
      );
    }
    log(`not yet served: ${description}; asking again in ${Math.round(intervalMs / 1000)} s`);
    await sleep(intervalMs);
  }
}

/**
 * Asks the real registry for a tarball's status with a HEAD request. A redirect is not
 * followed: the registry answers a tarball directly, so anything but 200 is "not yet".
 */
async function headStatus(url, fetchImpl = fetch) {
  try {
    const response = await fetchImpl(url, {
      method: 'HEAD',
      headers: { 'user-agent': USER_AGENT },
      redirect: 'manual',
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    });
    return response.status;
  } catch (error) {
    if (error && error.name === 'TimeoutError') {
      return `no answer within ${REQUEST_TIMEOUT_MS / 1000} s`;
    }
    return `no answer: ${(error && error.cause && error.cause.code) || (error && error.name) || 'error'}`;
  }
}

module.exports = { POLL_INTERVAL_MS, REGISTRY_ORIGIN, REQUEST_TIMEOUT_MS, WAIT_LIMIT_MS, headStatus, tarballUrl, waitForRegistry };
