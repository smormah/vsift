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
// before it installs or packs. A package counts as served only when BOTH of these hold:
//
//   - its tarball answers 200 (what `npm pack` and an install fetch), and
//   - its metadata document (the packument that `npm install` reads to resolve the version
//     and to decide whether an optional native package can be installed) lists the version.
//     The packument is cached separately from the tarball and can lag it, and npm skips an
//     optional package silently when it does not find it there.
//
// Both are polled every 30 seconds for at most 15 minutes; then the wait fails naming what is
// missing. The clock, the sleep and both requests are passed in, so the tests run with no
// network and no real time. Requests carry only the neutral identifier of lib/common.cjs
// (`USER_AGENT`) and, for the metadata, the abbreviated-document `Accept` header npm itself
// sends: no person, machine or credential. The metadata body is untrusted: it is read up to a
// fixed size, parsed as JSON, and only a verdict leaves this module, never its content.

const { PACKAGES, QualificationError, USER_AGENT, expect } = require('./common.cjs');
const { parseVersion } = require('./release.cjs');

/** The registry the published packages are read from, with no trailing slash. */
const REGISTRY_ORIGIN = 'https://registry.npmjs.org';
/** How often the registry is asked again. */
const POLL_INTERVAL_MS = 30_000;
/** How long the wait lasts at most (the issue's 15 minutes). */
const WAIT_LIMIT_MS = 15 * 60_000;
/** The most one request, its body included, may take before it counts as "not yet". */
const REQUEST_TIMEOUT_MS = 20_000;
/** The most of a metadata document that is read; a larger one counts as "not yet". */
const PACKUMENT_LIMIT_BYTES = 5 * 1024 * 1024;
/** The abbreviated metadata document, the form `npm install` itself asks for. */
const PACKUMENT_ACCEPT = 'application/vnd.npm.install-v1+json';

function expectPackageName(packageName) {
  expect(/^(@[a-z0-9-]+\/)?[a-z0-9][a-z0-9-]*$/.test(packageName), `${JSON.stringify(packageName)} is not a package name of this project`);
}

/**
 * The address of a package's tarball on the registry: `@vsift/win32-x64` at `0.3.0-rc.1`
 * is `https://registry.npmjs.org/@vsift/win32-x64/-/win32-x64-0.3.0-rc.1.tgz`, and the
 * unscoped launcher is `.../vsift-cli/-/vsift-cli-0.3.0-rc.1.tgz`.
 */
function tarballUrl(packageName, version) {
  parseVersion(version);
  expectPackageName(packageName);
  const base = packageName.slice(packageName.lastIndexOf('/') + 1);
  return `${REGISTRY_ORIGIN}/${packageName}/-/${base}-${version}.tgz`;
}

/**
 * The address of a package's metadata document, written as npm writes it: the slash of a
 * scoped name is encoded (`https://registry.npmjs.org/@vsift%2Fwin32-x64`).
 */
function packumentUrl(packageName) {
  expectPackageName(packageName);
  return `${REGISTRY_ORIGIN}/${packageName.replace('/', '%2F')}`;
}

function plural(count, word) {
  return `${count} ${word}${count === 1 ? '' : 's'}`;
}

/**
 * Judges a metadata document: `200` when it is a JSON object whose `versions` holds the
 * version, otherwise a short fixed description. Nothing of the document is returned.
 *
 * @param {string} text The body, already size-bounded by the caller.
 * @param {string} version
 * @returns {200|string}
 */
function judgePackument(text, version) {
  let document;
  try {
    document = JSON.parse(text);
  } catch {
    return 'the document is not JSON';
  }
  const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
  if (!isObject(document) || !isObject(document.versions)) {
    return 'the document lists no versions';
  }
  return Object.hasOwn(document.versions, version) ? 200 : 'the version is not listed';
}

/**
 * Waits until every package is served (see the head of this file; a package that has been
 * served is not asked again, tarball and metadata each). Any other answer, a refused
 * connection and a timeout all mean "not yet".
 *
 * @param {object} options
 * @param {string} options.version The published version.
 * @param {(url: string) => Promise<number|string>} options.probe The HTTP status of a tarball
 *   URL, or a description of why there was none.
 * @param {(url: string, version: string) => Promise<number|string>} options.probePackument 200
 *   when the metadata document at the URL lists the version, otherwise a description.
 * @param {(milliseconds: number) => Promise<void>} options.sleep
 * @param {() => number} options.now Milliseconds, any origin.
 * @param {(line: string) => void} [options.log]
 * @param {readonly string[]} [options.packages]
 * @param {number} [options.intervalMs]
 * @param {number} [options.limitMs]
 * @returns {Promise<{attempts: number, waitedMs: number}>}
 * @throws {QualificationError} naming every package still missing, with its last answers.
 */
async function waitForRegistry({
  version,
  probe,
  probePackument,
  sleep,
  now,
  log = () => {},
  packages = PACKAGES,
  intervalMs = POLL_INTERVAL_MS,
  limitMs = WAIT_LIMIT_MS,
}) {
  expect(Number.isFinite(intervalMs) && intervalMs > 0, 'the polling interval must be positive');
  expect(Number.isFinite(limitMs) && limitMs >= 0, 'the wait limit must not be negative');
  const tarballs = new Map(packages.map((name) => [name, tarballUrl(name, version)]));
  const packuments = new Map(packages.map((name) => [name, packumentUrl(name)]));
  /** What is still to be seen for each package, with the last answer for each part. */
  const state = new Map(packages.map((name) => [name, { tarball: null, metadata: null }]));
  const started = now();
  let attempts = 0;
  for (;;) {
    attempts += 1;
    for (const [name, parts] of state) {
      if (parts.tarball !== 200) {
        parts.tarball = await probe(tarballs.get(name));
      }
      if (parts.metadata !== 200) {
        parts.metadata = await probePackument(packuments.get(name), version);
      }
    }
    const waitedMs = now() - started;
    const missing = [...state].filter(([, parts]) => parts.tarball !== 200 || parts.metadata !== 200);
    if (missing.length === 0) {
      log(
        `all ${packages.length} packages of ${version} are served: tarball answers 200 and the metadata lists the version ` +
          `(after ${Math.round(waitedMs / 1000)} s, ${plural(attempts, 'attempt')})`,
      );
      return { attempts, waitedMs };
    }
    const description = missing
      .map(([name, parts]) => {
        const what = [];
        if (parts.tarball !== 200) {
          what.push(`tarball: ${parts.tarball}`);
        }
        if (parts.metadata !== 200) {
          what.push(`metadata: ${parts.metadata}`);
        }
        return `${name}@${version} (${what.join('; ')})`;
      })
      .join(', ');
    // Another poll would start after the limit: stop now and say what is still missing.
    if (waitedMs + intervalMs > limitMs) {
      throw new QualificationError(
        `the registry still does not serve ${description} after ${Math.round(waitedMs / 1000)} s ` +
          `(${plural(attempts, 'attempt')}, limit ${Math.round(limitMs / 1000)} s). ` +
          'npm had not yet served the tarball or the metadata, or the publish did not include it: read the Release run before re-running.',
      );
    }
    log(`not yet served: ${description}; asking again in ${Math.round(intervalMs / 1000)} s`);
    await sleep(intervalMs);
  }
}

/** A short description of why a request produced no answer. */
function describeFailure(error) {
  if (error && error.name === 'TimeoutError') {
    return `no answer within ${REQUEST_TIMEOUT_MS / 1000} s`;
  }
  return `no answer: ${(error && error.cause && error.cause.code) || (error && error.name) || 'error'}`;
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
    return describeFailure(error);
  }
}

/**
 * Reads at most `limitBytes` of a response body as text; `null` when there is more. The
 * declared length is checked first, and the stream is cancelled as soon as the limit is
 * passed, so a hostile or broken document is never held in memory.
 */
async function readBounded(response, limitBytes) {
  const declared = Number(response.headers.get('content-length'));
  if (Number.isFinite(declared) && declared > limitBytes) {
    return null;
  }
  if (response.body === null) {
    return '';
  }
  const reader = response.body.getReader();
  const chunks = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    total += value.byteLength;
    if (total > limitBytes) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks).toString('utf8');
}

/**
 * Asks the real registry for a package's abbreviated metadata document with a GET request and
 * judges it for the version: 200 when it lists the version, otherwise the status or a fixed
 * description. No redirect is followed and no credential is sent.
 */
async function packumentStatus(url, version, fetchImpl = fetch, limitBytes = PACKUMENT_LIMIT_BYTES) {
  try {
    const response = await fetchImpl(url, {
      method: 'GET',
      headers: { 'user-agent': USER_AGENT, accept: PACKUMENT_ACCEPT },
      redirect: 'manual',
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    });
    if (response.status !== 200) {
      await response.body?.cancel();
      return response.status;
    }
    const text = await readBounded(response, limitBytes);
    return text === null ? `the document is larger than ${limitBytes} bytes` : judgePackument(text, version);
  } catch (error) {
    return describeFailure(error);
  }
}

module.exports = {
  PACKUMENT_ACCEPT,
  PACKUMENT_LIMIT_BYTES,
  POLL_INTERVAL_MS,
  REGISTRY_ORIGIN,
  REQUEST_TIMEOUT_MS,
  WAIT_LIMIT_MS,
  headStatus,
  judgePackument,
  packumentStatus,
  packumentUrl,
  tarballUrl,
  waitForRegistry,
};
