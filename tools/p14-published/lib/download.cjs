'use strict';

// A bounded, verified HTTPS download, for fetching the reviewed artifacts of
// the managed catalogue into a folder before the offline install (RQ-03). It
// follows AGENTS.md's rule for downloads: HTTPS only, a pinned provenance (the
// URL, the size and the SHA-256 come from the published binary's own plan), a
// bounded size and a cryptographic check before the file is kept.
//
// Requests carry only the neutral identifier of lib/common.cjs (USER_AGENT): no
// person, machine or credential.

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const { QualificationError, USER_AGENT, expect } = require('./common.cjs');

/** The hosts the reviewed catalogue's redirects may end on (install.md section 9). */
const ALLOWED_HOSTS = Object.freeze([
  'github.com',
  'release-assets.githubusercontent.com',
  'objects.githubusercontent.com',
  'huggingface.co',
  'us.aws.cdn.hf.co',
  'cdn-lfs.huggingface.co',
]);

/** The most redirects a download follows (install.md section 9: more than three leave the reviewed route). */
const MAXIMUM_REDIRECTS = 3;

/** The most a single artifact may be (the schema's ceiling, 1 GiB). */
const MAXIMUM_BYTES = 1024 * 1024 * 1024;

/** The file name an artifact has: the last segment of its reviewed URL. */
function artifactName(url) {
  const { pathname } = new URL(url);
  const name = decodeURIComponent(pathname.split('/').filter(Boolean).pop() || '');
  expect(/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(name) && name !== '.' && name !== '..', `the URL ${url} does not end in a plain file name`);
  return name;
}

/** Whether a URL is HTTPS on one of the allowed hosts, with no credentials in it. */
function isAllowed(url, hosts = ALLOWED_HOSTS) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  return parsed.protocol === 'https:' && parsed.username === '' && parsed.password === '' && hosts.includes(parsed.hostname);
}

/**
 * Downloads `url` to `destination` and keeps it only if it has exactly `bytes`
 * bytes and the SHA-256 `sha256`. A file that fails either check is removed.
 *
 * @returns {Promise<{finalUrl: string, bytes: number}>}
 */
async function downloadVerified({ url, bytes, sha256, destination, hosts = ALLOWED_HOSTS, fetchImpl = fetch }) {
  expect(isAllowed(url, hosts), `${url} is not an HTTPS address on a reviewed host`);
  expect(Number.isSafeInteger(bytes) && bytes > 0 && bytes <= MAXIMUM_BYTES, `the reviewed size ${bytes} is out of bounds`);
  expect(/^[0-9a-f]{64}$/.test(sha256), 'the reviewed SHA-256 is not 64 hex digits');
  // Every hop is checked: a redirect may only lead to another reviewed host,
  // over HTTPS, and at most three times (the route VSift itself allows).
  let finalUrl = url;
  let response;
  for (let hop = 0; ; hop += 1) {
    response = await fetchImpl(finalUrl, { headers: { 'user-agent': USER_AGENT }, redirect: 'manual' });
    if (![301, 302, 303, 307, 308].includes(response.status)) {
      break;
    }
    expect(hop < MAXIMUM_REDIRECTS, `${artifactName(url)}: more than ${MAXIMUM_REDIRECTS} redirects`);
    const location = response.headers.get('location');
    expect(location !== null, `${artifactName(url)}: a redirect without a location`);
    finalUrl = new URL(location, finalUrl).href;
    expect(isAllowed(finalUrl, hosts), `${artifactName(url)} redirected off the reviewed route`);
  }
  expect(response.ok, `${artifactName(url)}: HTTP ${response.status}`);
  const declared = response.headers.get('content-length');
  expect(declared === null || Number(declared) === bytes, `${artifactName(url)}: the server declares ${declared} bytes, the review says ${bytes}`);
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  const hash = crypto.createHash('sha256');
  const file = fs.openSync(destination, 'wx', 0o600);
  let received = 0;
  try {
    for await (const chunk of response.body) {
      received += chunk.length;
      expect(received <= bytes, `${artifactName(url)}: more than the reviewed ${bytes} bytes arrived`);
      hash.update(chunk);
      fs.writeSync(file, chunk);
    }
  } catch (error) {
    fs.closeSync(file);
    fs.rmSync(destination, { force: true });
    throw error;
  }
  fs.closeSync(file);
  const digest = hash.digest('hex');
  if (received !== bytes || digest !== sha256) {
    fs.rmSync(destination, { force: true });
    throw new QualificationError(
      `${artifactName(url)}: received ${received} bytes with SHA-256 ${digest}; the review says ${bytes} bytes and ${sha256}`,
    );
  }
  return { finalUrl, bytes: received };
}

module.exports = { ALLOWED_HOSTS, MAXIMUM_BYTES, MAXIMUM_REDIRECTS, artifactName, downloadVerified, isAllowed };
