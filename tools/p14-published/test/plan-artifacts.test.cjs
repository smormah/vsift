'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Readable } = require('node:stream');

const { MOVED_PREFIXES, fetchPlanArtifacts, plansAMovedAddress } = require('../lib/plan-artifacts.cjs');

const BODY = Buffer.from('the bytes the plan describes');
const SHA = crypto.createHash('sha256').update(BODY).digest('hex');
const OLD = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz';
const NEW = 'https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz';
const CDN = 'https://release-assets.githubusercontent.com/github-production-release-asset/292087234/file?sig=redacted';
const MODEL = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/80da2d8bfee42b0e836fc3a9890373e5defc00a6/ggml-base.bin';

/** A server that answers each address as the repository move does: old address, then the new owner's, then the CDN. */
function movedServer(requests, body = BODY) {
  const redirects = new Map([
    [OLD, NEW],
    [NEW, CDN],
  ]);
  return async (requested, options) => {
    requests.push({ requested, headers: options.headers });
    const next = redirects.get(requested);
    if (next !== undefined) {
      return { ok: false, status: requested === OLD ? 301 : 302, headers: { get: (name) => (name === 'location' ? next : null) }, body: Readable.from([]) };
    }
    return { ok: true, status: 200, headers: { get: () => null }, body: Readable.from([body]) };
  };
}

function scratch(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 plan files '));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

const action = (url, bytes = BODY.length, sha256 = SHA) => ({ component: 'x', source_url: url, bytes, sha256 });

test('a plan that names the moved repository is recognised, and a plan that does not is left alone', () => {
  assert.deepEqual(MOVED_PREFIXES, ['https://github.com/BtbN/FFmpeg-Builds/']);
  assert.equal(plansAMovedAddress([action(MODEL), action(OLD)]), true);
  assert.equal(plansAMovedAddress([action(NEW), action(MODEL)]), false, 'the new address does not redirect');
  assert.equal(plansAMovedAddress([action('https://github.com/BtbN/FFmpeg-Builds-other/x')]), false, 'only the exact repository prefix counts');
  assert.throws(() => plansAMovedAddress(undefined), /no actions/);
});

test('the plan files are fetched by following the move, and kept only with the size and SHA-256 the plan carries', async (t) => {
  const folder = path.join(scratch(t), 'files');
  const requests = [];
  const notes = await fetchPlanArtifacts({ actions: [action(OLD)], folder, fetchImpl: movedServer(requests) });
  assert.deepEqual(requests.map((request) => request.requested), [OLD, NEW, CDN], 'the old address, the new owner, the CDN');
  for (const request of requests) {
    assert.deepEqual(Object.keys(request.headers), ['user-agent'], 'only the neutral identifier is sent');
    assert.equal(request.headers['user-agent'], 'vsift-p14-qualification');
  }
  assert.equal(fs.readFileSync(path.join(folder, 'ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz')).toString(), BODY.toString());
  assert.match(notes[0], /via release-assets\.githubusercontent\.com/);
});

test('a file whose bytes differ from the plan is refused and not kept', async (t) => {
  const folder = path.join(scratch(t), 'files');
  const altered = Buffer.from('the bytes the plan describes!');
  await assert.rejects(
    fetchPlanArtifacts({ actions: [action(OLD, altered.length, crypto.createHash('sha256').update(BODY).digest('hex'))], folder, fetchImpl: movedServer([], altered) }),
    /the review says/,
  );
  assert.deepEqual(fs.readdirSync(folder), [], 'nothing is left in the folder');
  await assert.rejects(fetchPlanArtifacts({ actions: [action(OLD, BODY.length + 1)], folder, fetchImpl: movedServer([]) }), /more than|the review says/);
  assert.deepEqual(fs.readdirSync(folder), []);
});

test('a move that leaves the reviewed hosts is refused', async (t) => {
  const folder = path.join(scratch(t), 'files');
  const elsewhere = async () => ({ ok: false, status: 302, headers: { get: () => 'https://example.test/file' }, body: Readable.from([]) });
  await assert.rejects(fetchPlanArtifacts({ actions: [action(OLD)], folder, fetchImpl: elsewhere }), /redirected off the reviewed route/);
  assert.deepEqual(fs.readdirSync(folder), []);
});

test('an empty plan fetches nothing', async (t) => {
  await assert.rejects(fetchPlanArtifacts({ actions: [], folder: path.join(scratch(t), 'files') }), /no actions/);
});
