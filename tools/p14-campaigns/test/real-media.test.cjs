'use strict';

// The real-media manifest and its fetch-and-verify tool (#363). Every transfer
// here reads local bytes through a stand-in transport: no test touches the
// network, and no real media is used.

const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Readable } = require('node:stream');
const test = require('node:test');
const media = require('../lib/real-media.cjs');

const MANIFEST = path.join(__dirname, '..', 'real-media', 'manifest.json');
const SCHEMA = path.join(__dirname, '..', 'real-media', 'manifest.schema.json');
const BYTES = Buffer.from('VSift real-media test bytes, not media. '.repeat(4096));
const hex = (algorithm, bytes) => crypto.createHash(algorithm).update(bytes).digest('hex');

function item(overrides = {}) {
  return {
    id: 'RM01',
    status: 'approved',
    title: 'A stand-in recording',
    source_page: 'https://commons.wikimedia.org/wiki/File:Stand-in.webm',
    url: 'https://upload.wikimedia.org/wikipedia/commons/a/ab/Stand-in.webm',
    file_name: 'RM01-stand-in.webm',
    bytes: BYTES.length,
    sha256: hex('sha256', BYTES),
    source_checksum: { algorithm: 'sha1', value: hex('sha1', BYTES) },
    licence: {
      id: 'CC-BY-SA-4.0',
      url: 'https://creativecommons.org/licenses/by-sa/4.0/',
      statement: 'https://commons.wikimedia.org/wiki/File:Stand-in.webm',
    },
    author: 'A stand-in author',
    attribution: '"A stand-in recording" by a stand-in author, CC BY-SA 4.0, via Wikimedia Commons',
    media: { duration_s: 12.5, container: 'webm', video: 'vp9 1280x720', audio: 'opus' },
    covers: ['conversational-pauses'],
    tests: ['#353'],
    approval: { date: '2026-10-18', by: 'maintainer' },
    ...overrides,
  };
}

function manifest(items) {
  return { schema_version: '1', set_id: 'vsift-real-media', items };
}

/** A stand-in transport that serves `bytes` and records every URL it was asked for. */
function serving(bytes, { length = bytes.length } = {}) {
  const asked = [];
  const open = async (url) => {
    asked.push(url);
    const chunks = [];
    for (let offset = 0; offset < bytes.length; offset += 7000) chunks.push(bytes.subarray(offset, offset + 7000));
    return { stream: Readable.from(chunks), length };
  };
  return { open, asked };
}

function scratch(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift-real-media-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const repository = path.join(root, 'repository');
  const destination = path.join(root, 'media');
  fs.mkdirSync(repository);
  fs.mkdirSync(destination);
  return { repository, destination: media.checkDestination(destination, repository) };
}

const listing = (folder) => fs.readdirSync(folder).sort();

test('an approved, pinned item is fetched, verified and placed with its attribution', async (t) => {
  const { destination } = scratch(t);
  const { open, asked } = serving(BYTES);
  assert.equal(await media.fetchItem(item(), destination, open), 'fetched');
  assert.deepEqual(asked, [item().url]);
  const folder = path.join(destination, 'RM01');
  assert.deepEqual(listing(folder), ['ATTRIBUTION.txt', 'RM01-stand-in.webm']);
  assert.ok(fs.readFileSync(path.join(folder, 'RM01-stand-in.webm')).equals(BYTES));
  const attribution = fs.readFileSync(path.join(folder, 'ATTRIBUTION.txt'), 'utf8');
  assert.match(attribution, /Attribution: "A stand-in recording" by a stand-in author, CC BY-SA 4\.0/);
  assert.match(attribution, /Licence: CC-BY-SA-4\.0 \(https:\/\/creativecommons\.org\/licenses\/by-sa\/4\.0\/\)/);
});

test('a file already in place with the pinned digest is kept and not fetched again', async (t) => {
  const { destination } = scratch(t);
  await media.fetchItem(item(), destination, serving(BYTES).open);
  const again = serving(BYTES);
  assert.equal(await media.fetchItem(item(), destination, again.open), 'present');
  assert.deepEqual(again.asked, []);
});

test('a different file already in place is refused and left as it is', async (t) => {
  const { destination } = scratch(t);
  const folder = path.join(destination, 'RM01');
  fs.mkdirSync(folder);
  fs.writeFileSync(path.join(folder, 'RM01-stand-in.webm'), 'not the pinned bytes');
  await assert.rejects(media.fetchItem(item(), destination, serving(BYTES).open), { code: media.ErrorCode.EXISTING_DIFFERS });
  assert.equal(fs.readFileSync(path.join(folder, 'RM01-stand-in.webm'), 'utf8'), 'not the pinned bytes');
});

test('bytes that do not match the pinned SHA-256 are never placed, and the partial file is removed', async (t) => {
  const { destination } = scratch(t);
  const other = Buffer.from(BYTES);
  other[100] ^= 1;
  const pinned = item({ source_checksum: { algorithm: 'sha1', value: hex('sha1', other) } });
  await assert.rejects(media.fetchItem(pinned, destination, serving(other).open), { code: media.ErrorCode.DIGEST_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), ['ATTRIBUTION.txt']);
});

test('bytes that do not match the checksum the source publishes are refused', async (t) => {
  const { destination } = scratch(t);
  const wrongSource = item({ source_checksum: { algorithm: 'md5', value: '0'.repeat(32) } });
  await assert.rejects(media.fetchItem(wrongSource, destination, serving(BYTES).open), { code: media.ErrorCode.DIGEST_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), ['ATTRIBUTION.txt']);
});

test('a transfer is stopped at the pinned size, and a short one is refused', async (t) => {
  const { destination } = scratch(t);
  const longer = Buffer.concat([BYTES, Buffer.from('more')]);
  await assert.rejects(media.fetchItem(item(), destination, serving(longer, { length: null }).open), { code: media.ErrorCode.SIZE_MISMATCH });
  const shorter = BYTES.subarray(0, BYTES.length - 1);
  await assert.rejects(media.fetchItem(item(), destination, serving(shorter, { length: null }).open), { code: media.ErrorCode.SIZE_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), ['ATTRIBUTION.txt']);
});

test('a server that keeps sending is cut off just past the pinned size, not read to its end', async (t) => {
  const { destination } = scratch(t);
  let produced = 0;
  const endless = async () => ({
    stream: Readable.from((function* chunks() {
      while (produced < 1000) {
        produced += 1;
        yield BYTES.subarray(0, 7000);
      }
    })()),
    length: null,
  });
  await assert.rejects(media.fetchItem(item(), destination, endless), { code: media.ErrorCode.SIZE_MISMATCH });
  assert.ok(produced <= Math.ceil(BYTES.length / 7000) + 2, `read ${produced} chunks`);
  assert.deepEqual(listing(path.join(destination, 'RM01')), ['ATTRIBUTION.txt']);
});

test('an announced length other than the pinned size is refused before anything is written', async (t) => {
  const { destination } = scratch(t);
  await assert.rejects(media.fetchItem(item(), destination, serving(BYTES, { length: BYTES.length + 1 }).open), { code: media.ErrorCode.SIZE_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), ['ATTRIBUTION.txt']);
});

test('a partial file of an earlier run is never overwritten', async (t) => {
  const { destination } = scratch(t);
  const folder = path.join(destination, 'RM01');
  fs.mkdirSync(folder);
  fs.writeFileSync(path.join(folder, '.RM01-stand-in.webm.partial'), 'earlier');
  await assert.rejects(media.fetchItem(item(), destination, serving(BYTES).open), { code: media.ErrorCode.PARTIAL_PRESENT });
  assert.equal(fs.readFileSync(path.join(folder, '.RM01-stand-in.webm.partial'), 'utf8'), 'earlier');
});

test('a proposed or unpinned item is never fetched', async (t) => {
  const { destination } = scratch(t);
  const proposed = item({ status: 'proposed' });
  delete proposed.approval;
  const transport = serving(BYTES);
  await assert.rejects(media.fetchItem(proposed, destination, transport.open), { code: media.ErrorCode.NOT_APPROVED });
  await assert.rejects(media.fetchItem(item({ sha256: null }), destination, transport.open), { code: media.ErrorCode.NOT_PINNED });
  assert.deepEqual(transport.asked, []);
  assert.throws(() => media.selectApproved(manifest([proposed]), ['RM01']), { code: media.ErrorCode.NOT_APPROVED });
  assert.throws(() => media.selectApproved(manifest([item()]), ['RM02']), { code: media.ErrorCode.NOT_APPROVED });
  assert.deepEqual(media.selectApproved(manifest([proposed]), []), []);
});

test('pinning verifies the source checksum, keeps the file and returns its SHA-256', async (t) => {
  const { destination } = scratch(t);
  const unpinned = item({ sha256: null });
  assert.equal(await media.pinItem(unpinned, destination, serving(BYTES).open), hex('sha256', BYTES));
  assert.ok(fs.readFileSync(path.join(destination, 'RM01', 'RM01-stand-in.webm')).equals(BYTES));
  await assert.rejects(media.pinItem(item(), destination, serving(BYTES).open), { code: media.ErrorCode.ALREADY_PINNED });
});

test('pinning refuses bytes the source checksum does not match', async (t) => {
  const { destination } = scratch(t);
  const other = Buffer.from(BYTES);
  other[0] ^= 1;
  await assert.rejects(media.pinItem(item({ sha256: null }), destination, serving(other).open), { code: media.ErrorCode.DIGEST_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), ['ATTRIBUTION.txt']);
});

test('an item whose source publishes no checksum is pinned on first use, and later fetches are held to that pin', async (t) => {
  const { destination } = scratch(t);
  const transcode = item({ sha256: null, source_checksum: null });
  const sha256 = await media.pinItem(transcode, destination, serving(BYTES).open);
  assert.equal(sha256, hex('sha256', BYTES));
  const other = Buffer.from(BYTES);
  other[0] ^= 1;
  const elsewhere = scratch(t).destination;
  await assert.rejects(media.fetchItem({ ...transcode, sha256 }, elsewhere, serving(other).open), { code: media.ErrorCode.DIGEST_MISMATCH });
});

test('the destination must be absolute, exist, and lie outside the repository', (t) => {
  const { repository, destination } = scratch(t);
  assert.throws(() => media.checkDestination('media', repository), { code: media.ErrorCode.DESTINATION_REFUSED });
  assert.throws(() => media.checkDestination(path.join(destination, 'missing'), repository), { code: media.ErrorCode.DESTINATION_REFUSED });
  const within = path.join(repository, 'media');
  fs.mkdirSync(within);
  assert.throws(() => media.checkDestination(within, repository), { code: media.ErrorCode.DESTINATION_REFUSED });
  assert.throws(() => media.checkDestination(repository, repository), { code: media.ErrorCode.DESTINATION_REFUSED });
  assert.throws(() => media.checkDestination(path.dirname(repository), repository), { code: media.ErrorCode.DESTINATION_REFUSED });
  const file = path.join(destination, 'file');
  fs.writeFileSync(file, '');
  assert.throws(() => media.checkDestination(file, repository), { code: media.ErrorCode.DESTINATION_REFUSED });
});

test('only HTTPS on an allowed host is fetched, and the real transport refuses before connecting', async () => {
  assert.ok(media.allowedUrl('https://upload.wikimedia.org/wikipedia/commons/a/ab/x.webm'));
  assert.ok(media.allowedUrl('https://archive.org/download/item/x.mp4'));
  assert.ok(media.allowedUrl('https://ia800300.us.archive.org/1/items/item/x.mp4'));
  for (const refused of [
    'http://upload.wikimedia.org/x.webm',
    'https://example.com/x.webm',
    'https://upload.wikimedia.org.example.com/x.webm',
    'https://user@upload.wikimedia.org/x.webm',
    'https://upload.wikimedia.org:8443/x.webm',
    'https://notarchive.org/x.mp4',
    'file:///etc/passwd',
    'not a url',
  ]) {
    assert.equal(media.allowedUrl(refused), false, refused);
    await assert.rejects(media.httpsOpen(refused), { code: media.ErrorCode.TRANSPORT_REFUSED }, refused);
  }
  assert.doesNotMatch(media.USER_AGENT, /@|mailto|github\.com/);
});

test('the manifest rules refuse what could escape, mislead or grow the set', () => {
  assert.deepEqual(media.manifestProblems(manifest([item()])), []);
  const refused = (items, pattern) => {
    const problems = media.manifestProblems(manifest(items));
    assert.ok(problems.some((problem) => pattern.test(problem)), `${pattern}: ${problems.join('; ')}`);
  };
  refused([item({ file_name: '../RM01-x.webm' })], /file_name/);
  refused([item({ file_name: 'RM02-x.webm' })], /start with the item id/);
  refused([item({ file_name: 'RM01-x.exe' })], /file_name/);
  refused([item({ url: 'http://upload.wikimedia.org/x.webm' })], /url must be HTTPS/);
  refused([item({ bytes: media.MAX_ITEM_BYTES + 1 })], /bytes/);
  refused([item({ sha256: 'ABC' })], /sha256/);
  refused([item({ licence: { ...item().licence, id: 'CC-BY-NC-4.0' } })], /licence/);
  refused([item({ covers: ['music-video'] })], /covers/);
  refused([item({ tests: ['soon'] })], /tests/);
  refused([item({ extra: true })], /unknown member extra/);
  refused([item({ approval: undefined })], /approval/);
  refused([item({ approval: { date: '2026-10-18', by: 'someone' } })], /approval/);
  refused([item({ status: 'proposed' })], /only an approved item carries approval/);
  refused([item(), item({ url: 'https://upload.wikimedia.org/other.webm', file_name: 'RM01-other.webm' })], /id RM01 is used twice/);
  const big = media.MAX_ITEM_BYTES;
  refused(['01', '02', '03'].map((n) => item({ id: `RM${n}`, file_name: `RM${n}-x.webm`, url: `https://upload.wikimedia.org/${n}.webm`, bytes: big })), /more than/);
  assert.deepEqual(media.manifestProblems({ ...manifest([]), schema_version: '2' }), ['schema_version must be "1"']);
});

test('the schema and the rules name the same members, shapes and limits', () => {
  const schema = JSON.parse(fs.readFileSync(SCHEMA, 'utf8'));
  assert.deepEqual(schema.required, [...media.TOP_KEYS]);
  const itemSchema = schema.$defs.item;
  assert.deepEqual(itemSchema.required, [...media.ITEM_REQUIRED]);
  assert.deepEqual(Object.keys(itemSchema.properties).sort(), [...media.ITEM_REQUIRED, ...media.ITEM_OPTIONAL].sort());
  assert.deepEqual(itemSchema.properties.covers.items.enum, [...media.COVERS]);
  assert.equal(itemSchema.properties.bytes.maximum, media.MAX_ITEM_BYTES);
  assert.match(schema.description, new RegExp(`at most ${media.MAX_SET_BYTES} bytes`));
});

test('the committed manifest is valid, within its size, and covers every required shape', () => {
  const committed = media.readManifest(MANIFEST);
  assert.deepEqual(media.uncoveredShapes(committed), []);
  for (const entry of committed.items) {
    assert.ok(entry.status !== 'approved' || entry.approval, `${entry.id} is approved without a recorded approval`);
  }
});
