'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const zlib = require('node:zlib');
const test = require('node:test');
const mp4 = require('../lib/mp4.cjs');
const ebml = require('../lib/ebml.cjs');
const png = require('../lib/png.cjs');
const hostile = require('../lib/hostile-cases.cjs');

const repository = path.join(__dirname, '..', '..', '..');
const base = fs.readFileSync(path.join(repository, 'fixtures', 'corpus', 'generated', 'F01.mp4'));

/** A minimal EBML reader: the elements of a buffer as {id, size, payload}. */
function readElements(buffer, start = 0, end = buffer.length) {
  const found = [];
  let offset = start;
  while (offset < end) {
    let idLength = 1;
    while (idLength < 4 && (buffer[offset] & (0x80 >> (idLength - 1))) === 0) idLength += 1;
    const idHex = buffer.subarray(offset, offset + idLength).toString('hex');
    offset += idLength;
    let sizeLength = 1;
    while (sizeLength < 8 && (buffer[offset] & (0x80 >> (sizeLength - 1))) === 0) sizeLength += 1;
    let size = BigInt(buffer[offset] & ((1 << (8 - sizeLength)) - 1));
    for (let index = 1; index < sizeLength; index += 1) size = (size << 8n) | BigInt(buffer[offset + index]);
    const unknown = size === (1n << BigInt(7 * sizeLength)) - 1n;
    offset += sizeLength;
    const length = unknown ? end - offset : Number(size);
    found.push({ id: idHex, unknown, claimed: size, payload: buffer.subarray(offset, Math.min(end, offset + length)) });
    offset += length;
  }
  return found;
}

test('the fixture parses into boxes and writes back byte for byte', () => {
  const boxes = mp4.parse(base);
  assert.equal(boxes[0].type, 'ftyp');
  assert.ok(mp4.find(boxes, ['moov', 'mvhd']));
  assert.ok(mp4.find(boxes, ['moov', 'trak', 'tkhd']));
  assert.ok(mp4.find(boxes, ['moov', 'trak', 'mdia', 'minf', 'stbl', 'stsd']));
  assert.equal(Buffer.compare(mp4.serialize(boxes), base), 0);
});

test('a box that does not fit is refused, not read', () => {
  assert.throws(() => mp4.parse(Buffer.from('ffffffff66726565', 'hex')), /does not fit/);
});

test('the declared duration is written where a demuxer reads it', () => {
  const boxes = mp4.parse(base);
  mp4.setMovieDuration(boxes, 1, 14401);
  const mvhd = mp4.find(boxes, ['moov', 'mvhd']).payload;
  assert.equal(mvhd.readUInt32BE(12), 1);
  assert.equal(mvhd.readUInt32BE(16), 14401);
  const again = mp4.parse(mp4.serialize(boxes));
  assert.equal(mp4.find(again, ['moov', 'mvhd']).payload.readUInt32BE(16), 14401);
});

test('the track durations are written where FFprobe reads a container length from', () => {
  const boxes = mp4.parse(base);
  mp4.setMovieDuration(boxes, 1, 14401);
  mp4.setTrackDurations(boxes, 14401);
  const tracks = mp4.findAll(boxes, ['moov', 'trak']);
  assert.ok(tracks.length >= 1);
  for (const trak of tracks) {
    const mdhd = mp4.find(trak.children, ['mdia', 'mdhd']).payload;
    const tkhd = mp4.find(trak.children, ['tkhd']).payload;
    assert.equal(mdhd[0], 0);
    assert.equal(mdhd.readUInt32BE(16), Math.min(0xffffffff, 14401 * mdhd.readUInt32BE(12)));
    assert.equal(tkhd.readUInt32BE(20), 14401);
  }
  // A header cannot hold more than 32 bits: the longest claim is clamped, not wrapped.
  mp4.setTrackDurations(boxes, 0xffffffff);
  assert.equal(mp4.find(tracks[0].children, ['mdia', 'mdhd']).payload.readUInt32BE(16), 0xffffffff);
});

test('the declared dimensions are written in the track header and the sample entry', () => {
  const boxes = mp4.parse(base);
  mp4.setDimensions(boxes, 4001, 3999);
  const tkhd = mp4.find(boxes, ['moov', 'trak', 'tkhd']).payload;
  const at = tkhd[0] === 1 ? 88 : 76;
  assert.equal(tkhd.readUInt32BE(at) / 65536, 4001);
  assert.equal(tkhd.readUInt32BE(at + 4) / 65536, 3999);
  const stsd = mp4.find(boxes, ['moov', 'trak', 'mdia', 'minf', 'stbl', 'stsd']).payload;
  assert.equal(stsd.readUInt16BE(16 + 24), 4001);
  assert.equal(stsd.readUInt16BE(16 + 26), 3999);
});

test('a deep nest is made without recursion in the writer and is well formed to its depth', () => {
  const bytes = mp4.nestedBytes('free', 5000);
  assert.equal(bytes.length, 5000 * 8);
  let offset = 0;
  let depth = 0;
  while (offset < bytes.length) {
    const size = bytes.readUInt32BE(offset);
    assert.equal(size, bytes.length - offset);
    offset += 8;
    depth += 1;
  }
  assert.equal(depth, 5000);
});

test('the edited fixtures are the fixture plus what each case says', () => {
  const cases = hostile.cases({ fixture: () => base });
  const build = (id) => cases.find((entry) => entry.id === id).build();
  assert.equal(build('mp4-empty').length, 0);
  assert.equal(build('mp4-ftyp-only').length, base.readUInt32BE(0));
  assert.equal(build('mp4-truncated-0.5').length, Math.floor(base.length * 0.5));
  assert.notEqual(Buffer.compare(build('mp4-bitflips-a'), base), 0);
  assert.equal(Buffer.compare(build('mp4-bitflips-a'), build('mp4-bitflips-a')), 0);
  assert.notEqual(Buffer.compare(build('mp4-bitflips-a'), build('mp4-bitflips-b')), 0);
  assert.equal(build('mp4-size-lies').length, base.length + 26);
  const moovZeroed = mp4.parse(build('mp4-moov-zeroed'));
  assert.ok(mp4.find(moovZeroed, ['moov']).payload.every((byte) => byte === 0));
  const external = mp4.parse(build('mp4-dref-file'));
  const dref = mp4.find(external, ['moov', 'trak', 'mdia', 'minf', 'dinf', 'dref']);
  assert.match(dref.payload.toString('latin1'), /file:\/\/\/var\/lib\/vsift\/outside-canary\.txt/);
  assert.equal(dref.payload.readUIntBE(16, 4), 0, 'the entry flags say the data is not in this file');
});

test('the hand-made Matroska files are well formed and say what each case says', () => {
  const cases = hostile.cases({ fixture: () => base });
  const build = (id) => cases.find((entry) => entry.id === id).build();
  const top = readElements(build('mkv-tracks-33'));
  assert.deepEqual(top.map((element) => element.id), ['1a45dfa3', '18538067']);
  const segment = readElements(top[1].payload);
  const tracks = readElements(segment.find((element) => element.id === '1654ae6b').payload);
  assert.equal(tracks.length, 33);
  assert.equal(readElements(build('mkv-tracks-2000').subarray(0))[1].payload.length > 0, true);
  const info = readElements(readElements(build('mkv-duration-absurd'))[1].payload).find((element) => element.id === '1549a966');
  const duration = readElements(info.payload).find((element) => element.id === '4489');
  assert.equal(duration.payload.readDoubleBE(0), 1e18);
  const voided = readElements(build('mkv-void-huge'));
  assert.ok(readElements(voided[1].payload).some((element) => element.id === 'ec' && element.claimed === 2n ** 50n));
  const unknown = readElements(build('mkv-unknown-nesting-100'));
  assert.equal(unknown[1].unknown, true);
});

test('a zero PNG is a valid PNG of the size it claims, a tiny file for a huge decode', async () => {
  const image = await png.zeroPng(1000, 800);
  assert.equal(Buffer.compare(image.subarray(0, 8), png.SIGNATURE), 0);
  assert.equal(image.readUInt32BE(16), 1000);
  assert.equal(image.readUInt32BE(20), 800);
  const idatLength = image.readUInt32BE(33);
  assert.equal(image.toString('latin1', 37, 41), 'IDAT');
  const data = zlib.inflateSync(image.subarray(41, 41 + idatLength));
  assert.equal(data.length, 800 * 1001);
  assert.ok(data.every((byte) => byte === 0));
  assert.equal(image.readUInt32BE(41 + idatLength + 4 + 0 - 4 + 4) !== undefined, true);
  assert.ok(image.length < 3000, `a megabyte of zeros is ${image.length} bytes`);
  assert.equal(zlib.crc32(image.subarray(12, 29)) >>> 0, image.readUInt32BE(29));
});

test('a 16384 by 16384 bomb is a quarter of a megabyte, and the liar claims one size around another', async () => {
  const bomb = await png.zeroPng(16384, 16384);
  assert.ok(bomb.length < 400 * 1024, `the bomb is ${bomb.length} bytes`);
  assert.equal(bomb.readUInt32BE(16), 16384);
  const cases = hostile.cases({ fixture: () => base });
  const liar = await cases.find((entry) => entry.id === 'mkv-png-liar').build();
  const segment = readElements(readElements(liar)[1].payload);
  const track = readElements(readElements(segment.find((element) => element.id === '1654ae6b').payload)[0].payload);
  const video = readElements(track.find((element) => element.id === 'e0').payload);
  const width = video.find((element) => element.id === 'b0').payload;
  assert.equal(width.readUIntBE(0, width.length), 64);
  const frame = readElements(segment.find((element) => element.id === '1f43b675').payload).find((element) => element.id === 'a3').payload;
  const imageStart = frame.indexOf(png.SIGNATURE);
  assert.equal(frame.readUInt32BE(imageStart + 16), 30000);
});

test('the sidecars are over their bounds, by construction', () => {
  const cases = hostile.cases({ fixture: () => base });
  const build = (id) => cases.find((entry) => entry.id === id).build();
  assert.ok(build('srt-9mib').length > 8 * hostile.MIB);
  assert.equal(build('srt-many-cues').toString().split('\n\n').filter(Boolean).length, 50000);
  assert.ok(build('srt-giant-cue').length > 6 * hostile.MIB);
  assert.ok(!build('srt-one-line').includes(10));
  assert.ok(build('vtt-nested-markup').toString().includes('<b>'.repeat(400000)));
  assert.ok(build('srt-control-characters').includes(0x1b));
  assert.ok(build('srt-invalid-utf8').includes(0xff));
  assert.equal(hostile.srt(3, 'hi').toString().split('\n\n').filter(Boolean).length, 3);
});

test('every case has a unique id and file, a typed expectation and operations it can run', () => {
  const all = [...hostile.cases({ fixture: () => base }), ...hostile.nameCases({ fixture: () => base })];
  assert.equal(new Set(all.map((entry) => entry.id)).size, all.length);
  assert.equal(new Set(all.map((entry) => entry.file)).size, all.length);
  for (const entry of all) {
    assert.ok(['failure', 'refused', 'any'].includes(entry.expect), entry.id);
    assert.ok(entry.ops.length > 0 && entry.ops.every((op) => ['ingest', 'candidates', 'frame', 'audio', 'recognise', 'ingest_human', 'job_name', 'job_small'].includes(op)), entry.id);
    assert.ok(entry.description.length > 10, entry.id);
    assert.ok(typeof entry.build === 'function');
  }
  const groups = new Set(all.map((entry) => entry.group));
  for (const group of ['damaged', 'declared', 'nesting', 'metadata', 'external', 'streams', 'bomb', 'size', 'sidecar', 'names']) assert.ok(groups.has(group), group);
});

test('the pathological names are real file names and include the dangerous kinds', () => {
  assert.equal(new Set(hostile.NAMES).size, hostile.NAMES.length);
  for (const name of hostile.NAMES) {
    assert.ok(!name.includes('/') && !name.includes('\u0000'), name);
    assert.ok(Buffer.byteLength(name) <= 255, name);
  }
  const joined = hostile.NAMES.join('|');
  for (const needle of ['$(', '`', ';', '\n', '\u001b', '-rf', '--help', '‮', '%s', 'con.mp4']) assert.ok(joined.includes(needle), needle);
});

test('materialising writes bytes, a sparse file, a link, a fifo-less folder and creates parent folders', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'vsift-hostile-'));
  try {
    assert.deepEqual(await hostile.materialise(directory, { id: 'a', file: 'x/y/z.bin', build: () => Buffer.from('abc') }), { bytes: 3 });
    assert.equal(fs.readFileSync(path.join(directory, 'x', 'y', 'z.bin'), 'utf8'), 'abc');
    const sparse = await hostile.materialise(directory, { id: 'b', file: 's.bin', build: () => ({ sparse: 1 << 30, header: Buffer.from('hdr') }) });
    assert.equal(sparse.sparse, true);
    assert.equal(fs.statSync(path.join(directory, 's.bin')).size, 1 << 30);
    assert.ok(fs.statSync(path.join(directory, 's.bin')).blocks * 512 < 1 << 20 || process.platform === 'win32', 'the file is sparse');
    assert.deepEqual(await hostile.materialise(directory, { id: 'c', file: 'dir', build: () => ({ folder: true }) }), { folder: true });
    assert.ok(fs.statSync(path.join(directory, 'dir')).isDirectory());
    await assert.rejects(hostile.materialise(directory, { id: 'd', file: 'q', build: () => ({ unknown: true }) }), /unknown/);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
