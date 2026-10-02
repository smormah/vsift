'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Readable } = require('node:stream');

const download = require('../lib/download.cjs');

const BODY = Buffer.from('reviewed artifact bytes');
const SHA = crypto.createHash('sha256').update(BODY).digest('hex');

function fakeFetch({ body = BODY, status = 200, redirects = [], headers = {} } = {}) {
  const hops = [...redirects];
  return async (requested, options) => {
    assert.equal(options.headers['user-agent'], 'vsift-p14-qualification', 'only the neutral identifier is sent');
    assert.deepEqual(Object.keys(options.headers), ['user-agent']);
    assert.equal(options.redirect, 'manual');
    if (hops.length > 0) {
      return { ok: false, status: 302, headers: { get: (name) => (name === 'location' ? hops.shift() : null) }, body: Readable.from([]) };
    }
    return {
      ok: status >= 200 && status < 300,
      status,
      headers: { get: (name) => headers[name.toLowerCase()] ?? null },
      body: Readable.from([body]),
    };
  };
}

function scratch(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 download '));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

test('the file name is the last segment of the reviewed URL, and only a plain name is accepted', () => {
  assert.equal(download.artifactName('https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild/ffmpeg-n9.0.1-linux64-gpl.tar.xz'), 'ffmpeg-n9.0.1-linux64-gpl.tar.xz');
  assert.equal(download.artifactName('https://huggingface.co/ggerganov/whisper.cpp/resolve/abc/ggml-base.bin'), 'ggml-base.bin');
  for (const bad of ['https://github.com/', 'https://github.com/a/%2e%2e', 'https://github.com/a/b%2Fc', 'https://github.com/a/-x', 'https://github.com/a/.hidden']) {
    assert.throws(() => download.artifactName(bad), /plain file name/, bad);
  }
});

test('only HTTPS addresses on a reviewed host, without credentials, are allowed', () => {
  assert.equal(download.isAllowed('https://github.com/a/b'), true);
  assert.equal(download.isAllowed('https://release-assets.githubusercontent.com/x'), true);
  for (const bad of ['http://github.com/a', 'https://evil.example/a', 'https://user:pass@github.com/a', 'https://github.com.evil.example/a', 'ftp://github.com/a', 'not a url']) {
    assert.equal(download.isAllowed(bad), false, bad);
  }
});

test('a download is kept only when its size and SHA-256 are the reviewed ones', async (t) => {
  const destination = path.join(scratch(t), 'artifact.bin');
  const result = await download.downloadVerified({
    url: 'https://github.com/a/artifact.bin',
    bytes: BODY.length,
    sha256: SHA,
    destination,
    fetchImpl: fakeFetch({ redirects: ['https://release-assets.githubusercontent.com/x'], headers: { 'content-length': String(BODY.length) } }),
  });
  assert.equal(result.bytes, BODY.length);
  assert.deepEqual(fs.readFileSync(destination), BODY);
});

test('a wrong digest, a wrong size, an off-route redirect and an HTTP error leave no file behind', async (t) => {
  const directory = scratch(t);
  const attempt = (name, options, fetchOptions) =>
    download.downloadVerified({
      url: 'https://github.com/a/artifact.bin',
      bytes: BODY.length,
      sha256: SHA,
      destination: path.join(directory, name),
      fetchImpl: fakeFetch(fetchOptions),
      ...options,
    });
  await assert.rejects(attempt('digest', {}, { body: Buffer.from('reviewed artifact byteS') }), /SHA-256/);
  await assert.rejects(attempt('size', { bytes: BODY.length + 1 }, {}), /received/);
  await assert.rejects(attempt('long', { bytes: 4 }, {}), /more than the reviewed/);
  await assert.rejects(attempt('route', {}, { redirects: ['https://evil.example/x'] }), /redirected off the reviewed route/);
  await assert.rejects(attempt('downgrade', {}, { redirects: ['http://github.com/x'] }), /redirected off the reviewed route/);
  await assert.rejects(attempt('relative', {}, { redirects: ['//evil.example/x'] }), /redirected off the reviewed route/);
  await assert.rejects(attempt('hops', {}, { redirects: Array(4).fill('https://github.com/y') }), /more than 3 redirects/);
  await assert.rejects(attempt('status', {}, { status: 404 }), /HTTP 404/);
  await assert.rejects(attempt('declared', {}, { headers: { 'content-length': '5' } }), /declares 5 bytes/);
  await assert.rejects(attempt('http', { url: 'http://github.com/a/x' }, {}), /not an HTTPS address/);
  await assert.rejects(attempt('huge', { bytes: download.MAXIMUM_BYTES + 1 }, {}), /out of bounds/);
  assert.deepEqual(fs.readdirSync(directory), [], 'nothing was kept');
});
