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
    url: 'https://archive.org/download/stand-in/Stand-in.webm',
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
  assert.deepEqual(listing(path.join(destination, 'RM01')), []);
});

test('bytes that do not match the checksum the source publishes are refused', async (t) => {
  const { destination } = scratch(t);
  const wrongSource = item({ source_checksum: { algorithm: 'md5', value: '0'.repeat(32) } });
  await assert.rejects(media.fetchItem(wrongSource, destination, serving(BYTES).open), { code: media.ErrorCode.DIGEST_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), []);
});

test('a transfer is stopped at the pinned size, and a short one is refused', async (t) => {
  const { destination } = scratch(t);
  const longer = Buffer.concat([BYTES, Buffer.from('more')]);
  await assert.rejects(media.fetchItem(item(), destination, serving(longer, { length: null }).open), { code: media.ErrorCode.SIZE_MISMATCH });
  const shorter = BYTES.subarray(0, BYTES.length - 1);
  await assert.rejects(media.fetchItem(item(), destination, serving(shorter, { length: null }).open), { code: media.ErrorCode.SIZE_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), []);
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
  assert.deepEqual(listing(path.join(destination, 'RM01')), []);
});

test('an announced length other than the pinned size is refused before anything is written', async (t) => {
  const { destination } = scratch(t);
  await assert.rejects(media.fetchItem(item(), destination, serving(BYTES, { length: BYTES.length + 1 }).open), { code: media.ErrorCode.SIZE_MISMATCH });
  assert.deepEqual(listing(path.join(destination, 'RM01')), []);
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
  assert.deepEqual(listing(path.join(destination, 'RM01')), []);
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
  assert.ok(media.sourceUrl('https://upload.wikimedia.org/wikipedia/commons/a/ab/x.webm'));
  assert.equal(media.allowedUrl('https://upload.wikimedia.org/wikipedia/commons/a/ab/x.webm'), false, 'a provenance-only source is never contacted');
  assert.ok(media.allowedUrl('https://images-assets.nasa.gov/video/x/x~medium.mp4'));
  assert.ok(media.allowedUrl(`${media.MIRROR_PREFIX}2026-10-19/RM01-x.webm`));
  assert.ok(media.allowedUrl('https://release-assets.githubusercontent.com/github-production-release-asset/1/2'));
  assert.equal(media.allowedUrl('https://github.com/someone/else/releases/download/test-media-1/x.webm'), false);
  assert.equal(media.allowedUrl('https://github.com/smormah/vsift/releases/download/v0.2.2/vsift.tgz'), false);
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

test('a refusal page is shown as its reason, and never with the address it echoes back', () => {
  const page = '<!DOCTYPE html><html><head><title>Wikimedia Error</title><style>body { font: 15px sans-serif; }</style></head>'
    + '<body><h1>Error</h1><p>Too many requests. Please contact the operators.</p><p>Sensitive client information</p>'
    + '<p>IP address: 203.0.113.7</p></body></html>';
  const said = media.refusalText(page);
  assert.match(said, /^Wikimedia Error Error Too many requests\. Please contact the operators\.$/);
  assert.doesNotMatch(media.refusalText('client 198.51.100.4 and 2001:db8::1 were refused'), /198\.51|2001:db8/);
  assert.ok(media.refusalText('x'.repeat(1000)).length <= 300);
});

test('a copy downloaded by hand is pinned only when its size and the source checksum match, and is left as it is', async (t) => {
  const { destination } = scratch(t);
  const hand = path.join(path.dirname(destination), 'downloads');
  fs.mkdirSync(hand);
  const copy = path.join(hand, 'Editing_basics_-_watchlists.webm');
  fs.writeFileSync(copy, BYTES);
  const unpinned = item({ sha256: null });
  assert.deepEqual(await media.pinFromFile(unpinned, destination, copy), { sha256: hex('sha256', BYTES), outcome: 'pinned' });
  assert.deepEqual(listing(path.join(destination, 'RM01')), ['ATTRIBUTION.txt', 'RM01-stand-in.webm']);
  assert.ok(fs.readFileSync(copy).equals(BYTES), 'the copy given is not moved or changed');
  const again = scratch(t).destination;
  assert.deepEqual(await media.pinFromFile(item(), again, copy), { sha256: hex('sha256', BYTES), outcome: 'fetched' });
});

test('a hand-downloaded copy of the wrong size, with the wrong checksum, or of an item with no published checksum is refused', async (t) => {
  const { destination } = scratch(t);
  const hand = path.join(path.dirname(destination), 'downloads');
  fs.mkdirSync(hand);
  const other = Buffer.from(BYTES);
  other[9] ^= 1;
  const wrong = path.join(hand, 'wrong.webm');
  fs.writeFileSync(wrong, other);
  await assert.rejects(media.pinFromFile(item({ sha256: null }), destination, wrong), { code: media.ErrorCode.DIGEST_MISMATCH });
  const short = path.join(hand, 'short.webm');
  fs.writeFileSync(short, BYTES.subarray(1));
  await assert.rejects(media.pinFromFile(item({ sha256: null }), destination, short), { code: media.ErrorCode.SIZE_MISMATCH });
  await assert.rejects(media.pinFromFile(item({ sha256: null, source_checksum: null }), destination, wrong), { code: media.ErrorCode.SOURCE_FILE_REFUSED });
  await assert.rejects(media.pinFromFile(item({ sha256: null }), destination, 'relative.webm'), { code: media.ErrorCode.SOURCE_FILE_REFUSED });
  await assert.rejects(media.pinFromFile(item({ sha256: null }), destination, hand), { code: media.ErrorCode.SOURCE_FILE_REFUSED });
  assert.deepEqual(listing(path.join(destination, 'RM01')), []);
});

test('a source that refuses the tool is never contacted, and a mirror is the test-media release asset of the item', async () => {
  await assert.rejects(media.httpsOpen('https://upload.wikimedia.org/wikipedia/commons/e/e6/x.webm'), /never contacted/);
  const mirrored = item({ mirror_url: `${media.MIRROR_PREFIX}2026-10-19/RM01-stand-in.webm` });
  assert.deepEqual(media.manifestProblems(manifest([mirrored])), []);
  assert.equal(media.fetchUrl(mirrored), mirrored.mirror_url);
  assert.equal(media.fetchUrl(item()), item().url);
  for (const bad of [`${media.MIRROR_PREFIX}2026-10-19/RM01-other.webm`, 'https://github.com/smormah/vsift/releases/download/v0.2.2/RM01-stand-in.webm',
    'https://example.com/test-media-1/RM01-stand-in.webm']) {
    assert.ok(media.manifestProblems(manifest([item({ mirror_url: bad })])).some((problem) => /mirror_url/.test(problem)), bad);
  }
});

function privateItem(overrides = {}) {
  return { id: 'P01', status: 'approved', private: true, file_name: 'Secret Meeting With Alice.mp4', bytes: 10, sha256: null,
    covers: ['conversational-pauses'], approval: { date: '2026-10-19', by: 'maintainer' }, ...overrides };
}

test('a private manifest is valid only outside the repository, with private items only, and never echoes a value', (t) => {
  const { repository } = scratch(t);
  const outside = path.join(path.dirname(repository), 'private');
  fs.mkdirSync(outside);
  const file = path.join(outside, 'private-manifest.json');
  const write = (items, setId = 'vsift-real-media-private') => fs.writeFileSync(file, JSON.stringify({ schema_version: '1', set_id: setId, items }));
  write([privateItem(), privateItem({ id: 'P02', file_name: 'paid tutorial.mkv' })]);
  assert.equal(media.readPrivateManifest(file, repository).items.length, 2);
  write([privateItem({ title: 'Alice and Bob', bytes: 0 }), privateItem({ id: 'P01' })]);
  assert.throws(() => media.readPrivateManifest(file, repository), (error) => {
    assert.equal(error.code, media.ErrorCode.MANIFEST_INVALID);
    assert.doesNotMatch(error.message, /Alice|Bob|Secret|Meeting/);
    return true;
  });
  write([item()]);
  assert.throws(() => media.readPrivateManifest(file, repository), { code: media.ErrorCode.MANIFEST_INVALID });
  const within = path.join(repository, 'private-manifest.json');
  fs.copyFileSync(file, within);
  assert.throws(() => media.readPrivateManifest(within, repository), { code: media.ErrorCode.PRIVATE_REFUSED });
  assert.throws(() => media.readPrivateManifest('private-manifest.json', repository), { code: media.ErrorCode.PRIVATE_REFUSED });
  assert.ok(media.manifestProblems(manifest([privateItem()])).some((problem) => /belongs in a private manifest/.test(problem)));
});

test('private items run on a local machine only', () => {
  assert.throws(() => media.refuseHostedPrivateRun({ GITHUB_ACTIONS: 'true' }), { code: media.ErrorCode.PRIVATE_REFUSED });
  assert.throws(() => media.refuseHostedPrivateRun({ CI: '1' }), { code: media.ErrorCode.PRIVATE_REFUSED });
  assert.doesNotThrow(() => media.refuseHostedPrivateRun({}));
});

test('a pin is anchored at the source, and a fetch of a pinned item contacts the mirror', async (t) => {
  const mirror = `${media.MIRROR_PREFIX}2026-10-19/RM01-stand-in.webm`;
  const pinning = serving(BYTES);
  await media.pinItem(item({ sha256: null, url: 'https://archive.org/download/x/x.webm', mirror_url: mirror }), scratch(t).destination, pinning.open);
  assert.deepEqual(pinning.asked, ['https://archive.org/download/x/x.webm']);
  const fetching = serving(BYTES);
  await media.fetchItem(item({ mirror_url: mirror }), scratch(t).destination, fetching.open);
  assert.deepEqual(fetching.asked, [mirror]);
});

test('the set holds at most eight approved items and 500 MB', () => {
  const nine = Array.from({ length: 9 }, (_, n) => item({ id: `RM${String(n + 1).padStart(2, '0')}`, file_name: `RM${String(n + 1).padStart(2, '0')}-x.webm`, url: `https://archive.org/download/x/${n}.webm`, bytes: 1000 }));
  assert.ok(media.manifestProblems(manifest(nine)).some((problem) => /9 items are approved, more than 8/.test(problem)));
  const heavy = [item({ bytes: 300_000_000 }), item({ id: 'RM02', file_name: 'RM02-x.webm', url: 'https://archive.org/download/x/2.webm', bytes: 200_000_001 })];
  assert.ok(media.manifestProblems(manifest(heavy)).some((problem) => /approved items hold 500000001 bytes, more than 500000000/.test(problem)));
});

test('the release is prepared from verbatim, pinned files only, as a pre-release on a tag no release workflow uses', (t) => {
  const { destination } = scratch(t);
  const mirrored = (id, bytes) => item({ id, file_name: `${id}-stand-in.webm`, url: `https://archive.org/download/x/${id}.webm`, bytes: bytes.length,
    sha256: hex('sha256', bytes), source_checksum: { algorithm: 'sha1', value: hex('sha1', bytes) }, mirror_url: `${media.MIRROR_PREFIX}2026-10-19/${id}-stand-in.webm` });
  const second = Buffer.from('second stand-in');
  const items = [mirrored('RM01', BYTES), mirrored('RM02', second)];
  for (const [entry, bytes] of [[items[0], BYTES], [items[1], second]]) {
    fs.mkdirSync(path.join(destination, entry.id));
    fs.writeFileSync(path.join(destination, entry.id, entry.file_name), bytes);
  }
  const plan = media.releasePlan(manifest(items), destination);
  assert.equal(plan.tag, 'test-media-2026-10-19');
  const command = plan.command('C:\out\NOTES.md', 'C:\out\ATTRIBUTION.txt');
  assert.match(command, /^gh release create test-media-2026-10-19 --repo smormah\/vsift --target main --prerelease --latest=false /);
  assert.ok(command.includes(`"${path.join(destination, 'RM02', 'RM02-stand-in.webm')}"`));
  assert.match(plan.attribution, /RM01: A stand-in recording[\s\S]*SHA-256: [0-9a-f]{64}/);
  assert.match(plan.notes, /\*\*This is not a VSift release\.\*\*/);
  fs.writeFileSync(path.join(destination, 'RM02', 'RM02-stand-in.webm'), 'changed');
  assert.throws(() => media.releasePlan(manifest(items), destination), { code: media.ErrorCode.EXISTING_DIFFERS });
  assert.throws(() => media.releasePlan(manifest([items[0], { ...items[1], mirror_url: `${media.MIRROR_PREFIX}2026-10-20/RM02-stand-in.webm` }]), destination), { code: media.ErrorCode.MANIFEST_INVALID });
});

test('the committed manifest\'s mirror is one test-media release, outside the release tags (v*)', () => {
  const committed = media.readManifest(MANIFEST);
  const tags = new Set(committed.items.filter((entry) => entry.status === 'approved').map((entry) => entry.mirror_url.split('/releases/download/')[1].split('/')[0]));
  assert.equal(tags.size, 1);
  const [tag] = tags;
  assert.match(tag, /^test-media-\d{4}-\d{2}-\d{2}$/);
  assert.doesNotMatch(tag, /^v/);
});

test('a partial release takes the items pinned and present, names the rest, and gives the later upload', (t) => {
  const { destination } = scratch(t);
  const mirror = (id) => `${media.MIRROR_PREFIX}2026-10-19/${id}-stand-in.webm`;
  const ready = item({ id: 'RM06', file_name: 'RM06-stand-in.webm', url: 'https://archive.org/download/x/6.webm', mirror_url: mirror('RM06') });
  const unpinned = item({ id: 'RM01', file_name: 'RM01-stand-in.webm', url: 'https://upload.wikimedia.org/wikipedia/commons/a/ab/1.webm', sha256: null, mirror_url: mirror('RM01') });
  const absent = item({ id: 'RM03', file_name: 'RM03-stand-in.webm', url: 'https://archive.org/download/x/3.webm', mirror_url: mirror('RM03') });
  fs.mkdirSync(path.join(destination, 'RM06'));
  fs.writeFileSync(path.join(destination, 'RM06', 'RM06-stand-in.webm'), BYTES);
  const set = manifest([unpinned, absent, ready]);
  assert.throws(() => media.releasePlan(set, destination), { code: media.ErrorCode.NOT_PINNED });
  const plan = media.releasePlan(set, destination, { partial: true });
  assert.deepEqual(plan.present, ['RM06']);
  assert.deepEqual(plan.toCome, ['RM01', 'RM03']);
  assert.deepEqual(plan.assets, [path.join(destination, 'RM06', 'RM06-stand-in.webm')]);
  assert.match(plan.notes, /1 of 3 recordings/);
  assert.match(plan.notes, /\*\*Still to come:\*\* RM01 \(RM01-stand-in\.webm, \d+ bytes\), RM03/);
  assert.match(plan.attribution, /Still to come \(not yet in this release\): RM01, RM03\./);
  assert.doesNotMatch(plan.attribution, /RM01: /);
  const upload = plan.upload('C:\out\ATTRIBUTION.txt');
  assert.equal(upload, `gh release upload test-media-2026-10-19 --repo smormah/vsift --clobber "${path.join(destination, 'RM01', 'RM01-stand-in.webm')}" "${path.join(destination, 'RM03', 'RM03-stand-in.webm')}" "C:\out\ATTRIBUTION.txt"`);
  fs.writeFileSync(path.join(destination, 'RM06', 'RM06-stand-in.webm'), 'changed');
  assert.throws(() => media.releasePlan(set, destination, { partial: true }), { code: media.ErrorCode.EXISTING_DIFFERS }, 'a file that differs from its pin is never an asset');
  fs.writeFileSync(path.join(destination, 'RM06', 'RM06-stand-in.webm'), BYTES);
  assert.equal(media.releasePlan(manifest([ready]), destination, { partial: true }).upload('C:\out\ATTRIBUTION.txt'), null, 'nothing still to come, no upload');
});

test('a source that refuses the tool is pinned from the mirror against its published checksum, and only then', async (t) => {
  const mirror = `${media.MIRROR_PREFIX}2026-10-19/RM01-stand-in.webm`;
  const wikimedia = item({ url: 'https://upload.wikimedia.org/wikipedia/commons/a/ab/1.webm', sha256: null, mirror_url: mirror });
  assert.equal(media.pinUrl(wikimedia), mirror);
  assert.equal(media.pinUrl(item()), item().url);
  assert.throws(() => media.pinUrl({ ...wikimedia, source_checksum: null }), { code: media.ErrorCode.NOT_PINNED });
  assert.throws(() => media.pinUrl({ ...wikimedia, mirror_url: undefined }), { code: media.ErrorCode.NOT_PINNED });
  const transport = serving(BYTES);
  assert.equal(await media.pinItem(wikimedia, scratch(t).destination, transport.open), hex('sha256', BYTES));
  assert.deepEqual(transport.asked, [mirror]);
  const other = Buffer.from(BYTES);
  other[3] ^= 1;
  await assert.rejects(media.pinItem(wikimedia, scratch(t).destination, serving(other).open), { code: media.ErrorCode.DIGEST_MISMATCH });
});

test('an asset the release does not hold yet is "not yet mirrored", which is not a failure; anything else is refused', () => {
  const mirrored = item({ mirror_url: `${media.MIRROR_PREFIX}2026-10-19/RM01-stand-in.webm` });
  assert.equal(media.crossCheckOutcome(mirrored, new media.RealMediaError(media.ErrorCode.NOT_FOUND, 'HTTP 404')), 'not_yet_mirrored');
  assert.equal(media.crossCheckOutcome(item(), new media.RealMediaError(media.ErrorCode.NOT_FOUND, 'HTTP 404')), 'refused');
  assert.equal(media.crossCheckOutcome(mirrored, new media.RealMediaError(media.ErrorCode.DIGEST_MISMATCH, 'x')), 'refused');
  assert.equal(media.crossCheckOutcome(mirrored, new Error('x')), 'refused');
});
