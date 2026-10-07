'use strict';

// The hostile inputs of the malicious-media campaign (P14 RQ-10), every one
// built by code at run time from the corpus's own small fixtures and a seeded
// generator: nothing is downloaded, no real exploit or malware is used, and
// nothing hostile is committed. Each case says what it is, how it is built,
// what VSift is asked to do with it and what it must come to. The cases follow
// the plan's list: decompression-bomb and resource-abuse variants (huge
// dimensions, many streams, a declared long duration, damaged and truncated
// files, container nesting, external references), huge metadata, extreme
// subtitle and transcript sidecars, and pathological file names.

const fs = require('node:fs');
const path = require('node:path');
const mp4 = require('./mp4.cjs');
const ebml = require('./ebml.cjs');
const png = require('./png.cjs');
const { Random } = require('./requests.cjs');

const MIB = 1024 * 1024;
/** The failures the plan allows for a hostile input. */
const TYPED = ['INVALID_SOURCE', 'RESOURCE_LIMIT', 'DEADLINE_EXCEEDED'];
/** Where the canary lives: outside the input root and the workspace, which no hostile input may read or change. */
const CANARY_PATH = '/var/lib/vsift/outside-canary.txt';

const MEDIA_OPS = ['ingest', 'candidates', 'frame', 'audio', 'recognise'];

/**
 * How the remediation of an `ingest` that has no room for its copy begins.
 * `STORAGE_IO` alone is also the answer of a disk that failed, so the no-room
 * case requires this beside the code. It is the stable first sentence of
 * `SOURCE_NO_ROOM_REMEDIATION` in `crates/vsift-contract/src/storage.rs`, as far
 * as that crate's own test pins it; a test here reads the constant and fails
 * if the two part.
 */
const NO_ROOM_REMEDIATION_START = "The folder that holds VSift's sessions does not have room for a copy";

/**
 * @typedef {object} Case
 * @property {string} id
 * @property {string} group
 * @property {string} description
 * @property {string} file the file name below the hostile folder (or `names/...`)
 * @property {() => Promise<Buffer>|Buffer|object} build what to write: bytes, or a spec of a sparse file, a link, a FIFO or a folder
 * @property {string[]} ops what VSift is asked to do, in order
 * @property {'failure'|'refused'|'any'} expect `failure`: the first operation must fail typed (a sidecar, a path); `refused`: some operation must fail typed (a media file: `ingest` only copies and sniffs the container, the probe and the decoders run in the later operations); `any`: a result or a typed failure
 * @property {string[]} [codes] failures allowed (default: the plan's three)
 * @property {Record<string, { codes: string[], remediation?: string }>} [answers] by operation, the answer the product is known to give, where the case is about that answer: the operation must fail with one of exactly these codes (`codes` above and the follow-up codes do not apply to it) and, when `remediation` is given, with a first remediation that begins so (lib/hostile-judge.cjs)
 * @property {string} [transcript] a sidecar file to import with the ingest (the source is then F10.mp4)
 * @property {number} [tmpfs] a small filesystem of this many MiB, mounted for the case alone; the session roots of the case are folders inside it that do not exist yet (`sessionRoot` in hostile-media.cjs), never the mount point
 * @property {boolean} [human] also run the human output (to check it carries no raw control character)
 */

/** A deterministic flip of `share` of the bytes. */
function flipped(base, seed, share) {
  const random = new Random(seed);
  const copy = Buffer.from(base);
  const count = Math.max(1, Math.floor(copy.length * share));
  for (let index = 0; index < count; index += 1) copy[random.int(copy.length)] ^= 1 << random.int(8);
  return copy;
}

/** A free-form `udta` item of `bytes` bytes: a metadata value a demuxer has to read and keep. */
function metadataBox(bytes) {
  const text = Buffer.alloc(bytes, 0x41);
  const data = mp4.make('data', Buffer.concat([Buffer.from([0, 0, 0, 1, 0, 0, 0, 0]), text]));
  const item = mp4.make('©nam', Buffer.alloc(0), [data]);
  const handler = mp4.make('hdlr', Buffer.concat([Buffer.alloc(8), Buffer.from('mdir'), Buffer.alloc(13)]));
  const meta = mp4.make('meta', Buffer.from([0, 0, 0, 0]), [handler, mp4.make('ilst', Buffer.alloc(0), [item])]);
  return mp4.make('udta', Buffer.alloc(0), [meta]);
}

/** Opens the fixture as a box tree, applies `edit` and writes it back. */
function edited(base, edit) {
  const boxes = mp4.parse(base);
  edit(boxes);
  return mp4.serialize(boxes);
}

function mkvWithTracks(count) {
  const tracks = [];
  for (let number = 1; number <= count; number += 1) tracks.push(ebml.track({ number, type: 1, codec: 'V_UNCOMPRESSED', width: 16, height: 16 }));
  return ebml.file([ebml.info(1000), ebml.element(ebml.IDS.tracks, Buffer.concat(tracks)), ebml.cluster(1, Buffer.alloc(16 * 16))]);
}

async function mkvWithPng({ claimedWidth, claimedHeight, imageWidth, imageHeight }) {
  const image = await png.zeroPng(imageWidth, imageHeight);
  const trackEntry = ebml.track({ number: 1, type: 1, codec: 'V_MS/VFW/FOURCC', width: claimedWidth, height: claimedHeight, codecPrivate: ebml.bitmapInfo(claimedWidth, claimedHeight, 'MPNG') });
  return ebml.file([ebml.info(1000), ebml.element(ebml.IDS.tracks, trackEntry), ebml.cluster(1, image)]);
}

/** Nested unknown-sized clusters: each opens another and never closes. */
function mkvNestedUnknown(depth) {
  let body = Buffer.alloc(0);
  for (let level = 0; level < depth; level += 1) body = ebml.unknownSized(ebml.IDS.cluster, Buffer.concat([ebml.element(ebml.IDS.timestamp, ebml.uint(0)), body]));
  return ebml.file([ebml.info(1000), ebml.element(ebml.IDS.tracks, ebml.track({ number: 1, type: 1, codec: 'V_UNCOMPRESSED' })), body], { unknownSize: true });
}

function mkvHugeTags(bytes) {
  const simple = ebml.element(ebml.IDS.simpleTag, Buffer.concat([ebml.element(ebml.IDS.tagName, 'TITLE'), ebml.element(ebml.IDS.tagString, Buffer.alloc(bytes, 0x41))]));
  const tags = ebml.element(ebml.IDS.tags, ebml.element(ebml.IDS.tag, simple));
  return ebml.file([ebml.info(1000), ebml.element(ebml.IDS.tracks, ebml.track({ number: 1, type: 1, codec: 'V_UNCOMPRESSED' })), tags, ebml.cluster(1, Buffer.alloc(256))]);
}

function srt(cues, text) {
  const lines = [];
  for (let index = 1; index <= cues; index += 1) {
    const second = index % 3600;
    const stamp = (value) => `00:${String(Math.floor(value / 60)).padStart(2, '0')}:${String(value % 60).padStart(2, '0')},000`;
    lines.push(String(index), `${stamp(second)} --> ${stamp(second + 1)}`, text, '');
  }
  return Buffer.from(lines.join('\n'));
}

/** The pathological names: valid file names on Linux that a shell, a terminal, a log or a parser may mishandle. */
const NAMES = [
  '$(touch pwned).mp4', ';touch pwned;.mp4', '`touch pwned`.mp4', '|touch pwned.mp4', '&&touch pwned.mp4', "'; touch pwned; '.mp4", '"; touch pwned; ".mp4',
  'line\nbreak.mp4', 'carriage\rreturn.mp4', 'tab\there.mp4', 'esc\u001b[2J\u001b[31mred.mp4', 'osc8\u001b]8;;http:evil.test\u0007link\u001b]8;;\u0007.mp4', 'c1\u009bcontrol.mp4', 'bell\u0007.mp4',
  '-rf.mp4', '--help.mp4', '-.mp4', '--.mp4', ' leading space.mp4', 'trailing space .mp4', '.hidden.mp4', '..mp4', '...', `${'a'.repeat(251)}.mp4`,
  '‮gnp.mp4', 'zero​width.mp4', 'emoji-\u{1F600}.mp4', 'nfd-é.mp4', 'wildcards*?[x].mp4', '%s%s%s%n.mp4', '${HOME}.mp4', '{{7*7}}.mp4', '~root.mp4', 'con.mp4', 'COM1.mp4', 'a:b.mp4', 'a\\b.mp4',
];

/**
 * The cases.
 * @param {{ fixture: (name: string) => Buffer }} context
 * @returns {Case[]}
 */
function cases(context) {
  const base = context.fixture('F01.mp4');
  const list = [];
  const add = (spec) => list.push({ ops: MEDIA_OPS, expect: 'any', ...spec });

  // Damaged and truncated containers.
  add({ id: 'mp4-empty', group: 'damaged', file: 'mp4-empty.mp4', description: 'an empty file', build: () => Buffer.alloc(0), ops: ['ingest'], expect: 'failure' });
  add({ id: 'mp4-ftyp-only', group: 'damaged', file: 'mp4-ftyp-only.mp4', description: 'a file type box and nothing else', build: () => base.subarray(0, base.readUInt32BE(0)), expect: 'refused' });
  for (const share of [0.05, 0.5, 0.9, 0.999]) {
    add({ id: `mp4-truncated-${share}`, group: 'damaged', file: `mp4-truncated-${share}.mp4`, description: `cut after ${share * 100}% of its bytes`, build: () => base.subarray(0, Math.floor(base.length * share)), ops: ['ingest', 'candidates'] });
  }
  for (const seed of ['a', 'b', 'c']) {
    add({ id: `mp4-bitflips-${seed}`, group: 'damaged', file: `mp4-bitflips-${seed}.mp4`, description: 'a fifth of a percent of the bytes flipped (seeded)', build: () => flipped(base, `flip-${seed}`, 0.002) });
  }
  add({ id: 'mp4-moov-zeroed', group: 'damaged', file: 'mp4-moov-zeroed.mp4', description: 'the movie box overwritten with zeros', build: () => edited(base, (boxes) => { const moov = mp4.find(boxes, ['moov']); moov.children = null; moov.payload = Buffer.alloc(moov.payload.length); }), expect: 'refused' });
  add({ id: 'mp4-size-lies', group: 'damaged', file: 'mp4-size-lies.mp4', description: 'a box that claims 4 GiB and a box that claims 2^63 bytes after the real boxes', build: () => Buffer.concat([base, Buffer.from('fffffff066726565' + '6161', 'hex'), Buffer.from('00000001' + '74657374' + '8000000000000000', 'hex')]), ops: ['ingest'] });

  // Absurd declared values.
  add({ id: 'mp4-movie-header-lies', group: 'declared', file: 'mp4-movie-header-lies.mp4', description: 'a movie header that declares 4,294,967,295 seconds (136 years) over 6 seconds of video; the tracks say 6 seconds', build: () => edited(base, (boxes) => mp4.setMovieDuration(boxes, 1, 0xffffffff)), ops: ['ingest', 'candidates', 'frame'] });
  add({ id: 'mp4-duration-max', group: 'declared', file: 'mp4-duration-max.mp4', description: 'every track and the movie declaring the longest a header can hold (136 years of movie time)', build: () => edited(base, (boxes) => { mp4.setMovieDuration(boxes, 1, 0xffffffff); mp4.setTrackDurations(boxes, 0xffffffff); }), expect: 'refused' });
  add({ id: 'mp4-duration-over-4h', group: 'declared', file: 'mp4-duration-14401s.mp4', description: 'every track and the movie declaring one second above the 4 hour limit (observed: the file is processed, not refused, so the container length the product measures is not this header value; the limit itself is tested in the unit tests)', build: () => edited(base, (boxes) => { mp4.setMovieDuration(boxes, 1, 14401); mp4.setTrackDurations(boxes, 14401); }), ops: ['ingest', 'candidates', 'frame'] });
  add({ id: 'mp4-duration-4h', group: 'declared', file: 'mp4-duration-14400s.mp4', description: 'every track and the movie declaring exactly 4 hours over 6 seconds of video (the limit itself)', build: () => edited(base, (boxes) => { mp4.setMovieDuration(boxes, 1, 14400); mp4.setTrackDurations(boxes, 14400); }), ops: ['ingest', 'candidates'] });
  add({ id: 'mp4-duration-zero', group: 'declared', file: 'mp4-duration-zero.mp4', description: 'a declared duration of zero', build: () => edited(base, (boxes) => mp4.setMovieDuration(boxes, 1000, 0)), ops: ['ingest', 'candidates'] });
  add({ id: 'mp4-dims-65535', group: 'declared', file: 'mp4-dims-65535.mp4', description: 'declared dimensions of 65535 by 65535 in the track header and the sample entry', build: () => edited(base, (boxes) => mp4.setDimensions(boxes, 65535, 65535)) });
  add({ id: 'mp4-dims-4001', group: 'declared', file: 'mp4-dims-4001.mp4', description: 'declared dimensions of 4001 by 4001 (16.008 megapixels, just above the 16 megapixel limit)', build: () => edited(base, (boxes) => mp4.setDimensions(boxes, 4001, 4001)) });

  // Container nesting and huge metadata.
  add({ id: 'mp4-nested-5000', group: 'nesting', file: 'mp4-nested-5000.mp4', description: 'an unknown box nested 5,000 deep inside the movie box', build: () => edited(base, (boxes) => mp4.appendChild(mp4.find(boxes, ['moov']), mp4.make('udta', mp4.nestedBytes('free', 5000)))), ops: ['ingest', 'candidates'] });
  add({ id: 'mp4-nested-trak-300', group: 'nesting', file: 'mp4-nested-trak-300.mp4', description: 'track boxes nested 300 deep inside the movie box', build: () => edited(base, (boxes) => mp4.appendChild(mp4.find(boxes, ['moov']), mp4.make('trak', mp4.nestedBytes('trak', 300)))), ops: ['ingest', 'candidates'] });
  add({ id: 'mp4-metadata-32mib', group: 'metadata', file: 'mp4-metadata-32mib.mp4', description: 'a 32 MiB title in the movie box', build: () => edited(base, (boxes) => mp4.appendChild(mp4.find(boxes, ['moov']), metadataBox(32 * MIB))), ops: ['ingest', 'candidates'] });
  add({ id: 'mp4-free-512mib', group: 'metadata', file: 'mp4-free-512mib.mp4', description: 'a 512 MiB free-space box after the movie (a large file of nothing)', build: () => Buffer.concat([base, Buffer.from([0x20, 0, 0, 0]), Buffer.from('free'), Buffer.alloc(512 * MIB - 8)]), ops: ['ingest'] });

  // External references.
  const dref = (flags, location) => (boxes) => {
    for (const box of mp4.findAll(boxes, ['moov', 'trak', 'mdia', 'minf', 'dinf'])) {
      const url = Buffer.concat([Buffer.from([0, flags >> 16 & 0xff, flags >> 8 & 0xff, flags & 0xff]), Buffer.from(`${location}\0`)]);
      const entry = Buffer.alloc(8);
      entry.writeUInt32BE(url.length + 8, 0);
      entry.write('url ', 4, 'latin1');
      const table = Buffer.concat([Buffer.from([0, 0, 0, 0, 0, 0, 0, 1]), entry, url]);
      box.children = [mp4.make('dref', table)];
    }
  };
  add({ id: 'mp4-dref-file', group: 'external', file: 'mp4-dref-file.mp4', description: 'a data reference to a local file (the canary outside the input root)', build: () => edited(base, dref(0, `file://${CANARY_PATH}`)), ops: ['ingest', 'candidates', 'frame'] });
  add({ id: 'mp4-dref-http', group: 'external', file: 'mp4-dref-http.mp4', description: 'a data reference to http://127.0.0.1:9/ (no network exists in the container)', build: () => edited(base, dref(0, 'http://127.0.0.1:9/media.mp4')), ops: ['ingest', 'candidates', 'frame'] });
  add({ id: 'mp4-dref-absolute-path', group: 'external', file: 'mp4-dref-path.mp4', description: 'a data reference by absolute path to the canary', build: () => edited(base, dref(0, CANARY_PATH)), ops: ['ingest', 'candidates', 'frame'] });
  add({ id: 'playlist-hls', group: 'external', file: 'playlist-hls.mp4', description: 'an HLS playlist named .mp4, listing the canary and a loopback address', build: () => Buffer.from(`#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:10,\nfile://${CANARY_PATH}\n#EXTINF:10,\nhttp://127.0.0.1:9/a.ts\n#EXT-X-ENDLIST\n`), ops: ['ingest'], expect: 'failure' });
  add({ id: 'concat-script', group: 'external', file: 'concat-script.mkv', description: 'an FFmpeg concat script named .mkv, naming the canary', build: () => Buffer.from(`ffconcat version 1.0\nfile '${CANARY_PATH}'\nfile 'http://127.0.0.1:9/b.mp4'\n`), ops: ['ingest'], expect: 'failure' });
  add({ id: 'sdp-text', group: 'external', file: 'session.sdp.mp4', description: 'an SDP session description named .mp4', build: () => Buffer.from('v=0\no=- 0 0 IN IP4 127.0.0.1\ns=x\nc=IN IP4 127.0.0.1\nt=0 0\nm=video 5004 RTP/AVP 96\na=rtpmap:96 H264/90000\n'), ops: ['ingest'], expect: 'failure' });
  add({ id: 'text-64mib', group: 'external', file: 'text-64mib.mp4', description: '64 MiB of one letter named .mp4', build: () => Buffer.alloc(64 * MIB, 0x41), ops: ['ingest'], expect: 'failure' });
  add({ id: 'random-1mib', group: 'external', file: 'random-1mib.mkv', description: '1 MiB of seeded random bytes named .mkv', build: () => { const random = new Random('random'); const bytes = Buffer.alloc(MIB); for (let index = 0; index < bytes.length; index += 1) bytes[index] = random.int(256); return bytes; }, ops: ['ingest'], expect: 'failure' });
  add({ id: 'symlink-to-canary', group: 'external', file: 'symlink-to-canary.mp4', description: 'a symbolic link to the canary', build: () => ({ symlink: CANARY_PATH }), ops: ['ingest', 'job_name'], expect: 'failure', codes: [...TYPED, 'INVALID_ARGUMENT'] });
  add({ id: 'fifo', group: 'external', file: 'fifo.mp4', description: 'a named pipe nobody writes to (a read would block for ever)', build: () => ({ fifo: true }), ops: ['ingest', 'job_name'], expect: 'failure', codes: [...TYPED, 'INVALID_ARGUMENT'] });
  add({ id: 'folder', group: 'external', file: 'folder.mp4', description: 'a folder named like a video', build: () => ({ folder: true }), ops: ['ingest', 'job_name'], expect: 'failure', codes: [...TYPED, 'INVALID_ARGUMENT'] });

  // Matroska by hand: many streams, absurd values, nesting, huge tags.
  add({ id: 'mkv-tracks-32', group: 'streams', file: 'mkv-tracks-32.mkv', description: '32 video tracks (the limit)', build: () => mkvWithTracks(32), ops: ['ingest', 'candidates'] });
  add({ id: 'mkv-tracks-33', group: 'streams', file: 'mkv-tracks-33.mkv', description: '33 video tracks (one above the limit)', build: () => mkvWithTracks(33), expect: 'refused' });
  add({ id: 'mkv-tracks-2000', group: 'streams', file: 'mkv-tracks-2000.mkv', description: '2,000 video tracks', build: () => mkvWithTracks(2000), expect: 'refused' });
  add({ id: 'mkv-duration-absurd', group: 'declared', file: 'mkv-duration-absurd.mkv', description: 'a declared duration of 1e18 milliseconds', build: () => ebml.file([ebml.info(1e18), ebml.element(ebml.IDS.tracks, ebml.track({ number: 1, type: 1, codec: 'V_UNCOMPRESSED' })), ebml.cluster(1, Buffer.alloc(256))]), expect: 'refused' });
  add({ id: 'mkv-duration-nan', group: 'declared', file: 'mkv-duration-nan.mkv', description: 'a declared duration that is not a number', build: () => ebml.file([ebml.info(Number.NaN), ebml.element(ebml.IDS.tracks, ebml.track({ number: 1, type: 1, codec: 'V_UNCOMPRESSED' })), ebml.cluster(1, Buffer.alloc(256))]), ops: ['ingest'] });
  add({ id: 'mkv-void-huge', group: 'damaged', file: 'mkv-void-huge.mkv', description: 'a void element that claims 2^50 bytes', build: () => ebml.file([ebml.info(1000), ebml.elementClaiming(ebml.IDS.void, 2 ** 50, Buffer.alloc(16))]), ops: ['ingest'] });
  add({ id: 'mkv-unknown-nesting-100', group: 'nesting', file: 'mkv-unknown-nesting-100.mkv', description: 'clusters of unknown size opened 100 deep', build: () => mkvNestedUnknown(100), ops: ['ingest'] });
  add({ id: 'mkv-tags-64mib', group: 'metadata', file: 'mkv-tags-64mib.mkv', description: 'a 64 MiB tag value', build: () => mkvHugeTags(64 * MIB), ops: ['ingest', 'candidates'] });
  add({ id: 'mkv-png-16384', group: 'bomb', file: 'mkv-png-16384.mkv', description: 'a 16384 by 16384 grayscale image frame (268 megapixels) that is a quarter of a megabyte on disk', build: () => mkvWithPng({ claimedWidth: 16384, claimedHeight: 16384, imageWidth: 16384, imageHeight: 16384 }), expect: 'refused' });
  add({ id: 'mkv-png-3999', group: 'bomb', file: 'mkv-png-3999.mkv', description: 'a 3999 by 3999 image frame (15.99 megapixels, inside the limit) that compresses a thousand to one', build: () => mkvWithPng({ claimedWidth: 3999, claimedHeight: 3999, imageWidth: 3999, imageHeight: 3999 }), ops: ['ingest', 'candidates', 'frame'] });
  add({ id: 'mkv-png-liar', group: 'bomb', file: 'mkv-png-liar.mkv', description: 'a track that says 64 by 64 around an image that says 30000 by 30000 (900 MB decoded)', build: () => mkvWithPng({ claimedWidth: 64, claimedHeight: 64, imageWidth: 30000, imageHeight: 30000 }), ops: ['ingest', 'candidates', 'frame'] });
  add({ id: 'mkv-audio-absurd', group: 'declared', file: 'mkv-audio-absurd.mkv', description: 'an audio track of 255 channels at 3.4e38 Hz', build: () => ebml.file([ebml.info(1000), ebml.element(ebml.IDS.tracks, ebml.track({ number: 1, type: 2, codec: 'A_PCM/INT/LIT', sampleRate: 3.4e38, channels: 255 })), ebml.cluster(1, Buffer.alloc(256))]), ops: ['ingest', 'audio', 'recognise'] });

  // Size: a file above the limit, and one that does not fit the disk. These two cases are about which answer the
  // product gives, so each operation's answer is pinned (`answers`), not only held to the plan's three codes.
  //
  // Over the limit: `ingest` answers INVALID_SOURCE whatever the free space (#310; the room check once ran first and
  // answered STORAGE_IO). A worker workspace checks its 1 GiB reserve before the limit, as it did in 0.1.0, so the
  // request answers RESOURCE_LIMIT on a disk without 31 GiB free (a hosted runner) and INVALID_SOURCE on one with.
  add({ id: 'sparse-30gib', group: 'size', file: 'sparse-30gib.mp4', description: 'a sparse file of 30 GiB (above the 20 GiB limit) that begins like an MP4', build: () => ({ sparse: 30 * 1024 * MIB, header: base.subarray(0, 32) }), ops: ['ingest', 'job_name'], expect: 'failure', answers: { ingest: { codes: ['INVALID_SOURCE'] }, job_name: { codes: ['INVALID_SOURCE', 'RESOURCE_LIMIT'] } } });
  // Within the limit, no room: `ingest` refuses before the copy with STORAGE_IO and the no-room remediation (#266; the
  // code is the CLI path's published one, L-127), and the worker request with RESOURCE_LIMIT (its reserve cannot be met).
  add({ id: 'sparse-no-room', group: 'size', file: 'sparse-600mib.mp4', description: 'a sparse 600 MiB file (within the 20 GiB limit) ingested into a session root on a filesystem of 256 MiB: the copy cannot fit, and neither can a worker workspace\'s 1 GiB reserve', build: () => ({ sparse: 600 * MIB, header: base.subarray(0, 32) }), ops: ['ingest', 'job_small'], expect: 'failure', answers: { ingest: { codes: ['STORAGE_IO'], remediation: NO_ROOM_REMEDIATION_START }, job_small: { codes: ['RESOURCE_LIMIT'] } }, tmpfs: 256 });

  // Sidecars (the source is F10.mp4).
  const sidecar = (id, file, description, build, extra = {}) => add({ id, group: 'sidecar', file, description, build, ops: ['ingest'], expect: 'failure', transcript: file, ...extra });
  sidecar('srt-9mib', 'srt-9mib.srt', 'a subtitle file of 9 MiB (above the 8 MiB bound): one cue, then blank lines', () => Buffer.concat([Buffer.from('1\n00:00:01,000 --> 00:00:02,000\nok\n\n'), Buffer.alloc(9 * MIB, 0x0a)]));
  sidecar('srt-many-cues', 'srt-many-cues.srt', '50,000 one-letter cues (above the cue count bound)', () => srt(50000, 'x'));
  sidecar('srt-giant-cue', 'srt-giant-cue.srt', 'one cue of 6 MiB of text', () => Buffer.concat([Buffer.from('1\n00:00:01,000 --> 00:00:02,000\n'), Buffer.alloc(6 * MIB, 0x61), Buffer.from('\n\n')]));
  sidecar('srt-one-line', 'srt-one-line.srt', 'one line of 7.9 MiB without a line break', () => Buffer.alloc(Math.floor(7.9 * MIB), 0x61));
  sidecar('vtt-nested-markup', 'vtt-nested-markup.vtt', 'a cue with 400,000 nested bold tags', () => Buffer.from(`WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n${'<b>'.repeat(400000)}text${'</b>'.repeat(400000)}\n`));
  sidecar('vtt-note-flood', 'vtt-note-flood.vtt', 'a million NOTE blocks (a valid file of about 8 MB)', () => Buffer.from(`WEBVTT\n\n${'NOTE x\n\n'.repeat(1000000)}00:00:01.000 --> 00:00:02.000\nok\n`), { expect: 'any' });
  sidecar('srt-control-characters', 'srt-control.srt', 'cue text with escape, bell and C1 characters', () => Buffer.from('1\n00:00:01,000 --> 00:00:02,000\n\u001b[2J\u001b]8;;http://x\u0007hi\u009b31m\u0000nul\n\n'));
  sidecar('srt-invalid-utf8', 'srt-invalid-utf8.srt', 'invalid UTF-8', () => Buffer.concat([Buffer.from('1\n00:00:01,000 --> 00:00:02,000\n'), Buffer.from([0xff, 0xfe, 0xc3, 0x28]), Buffer.from('\n\n')]));
  sidecar('srt-timestamp-overflow', 'srt-overflow.srt', 'timestamps of 99999999 hours', () => Buffer.from('1\n99999999:59:59,999 --> 99999999:59:59,999\nx\n\n2\n18446744073709551615:00:00,000 --> 18446744073709551615:00:01,000\ny\n\n'));
  sidecar('vtt-lone-cr', 'vtt-lone-cr.vtt', 'bare carriage returns as line ends (valid WebVTT)', () => Buffer.from('WEBVTT\r\r00:00:01.000 --> 00:00:02.000\rtext\r\r'), { expect: 'any' });
  sidecar('srt-bom-only', 'srt-bom-only.srt', 'a byte order mark and nothing else', () => Buffer.from([0xef, 0xbb, 0xbf]));

  return list;
}

/** The pathological file names, each a copy of the base clip. */
function nameCases(context) {
  const base = context.fixture('F01.mp4');
  const list = NAMES.map((name, index) => ({
    id: `name-${String(index).padStart(2, '0')}`,
    group: 'names',
    file: `names/${name}`,
    description: `the file name ${JSON.stringify(name)}`,
    build: () => base,
    ops: ['ingest', 'ingest_human', 'job_name'],
    expect: 'any',
    codes: [...TYPED, 'INVALID_ARGUMENT'],
    human: true,
  }));
  list.push({
    id: 'name-deep',
    group: 'names',
    file: `names/${Array.from({ length: 150 }, () => 'dddddddddd').join('/')}/video.mp4`,
    description: 'a path 150 folders deep (about 1,700 bytes)',
    build: () => base,
    ops: ['ingest', 'job_name'],
    expect: 'any',
    codes: [...TYPED, 'INVALID_ARGUMENT'],
  });
  return list;
}

/** Writes one case's file below `directory`, creating folders as needed. Returns what was made. */
async function materialise(directory, spec) {
  const target = path.join(directory, spec.file);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  const built = await spec.build();
  if (Buffer.isBuffer(built)) {
    fs.writeFileSync(target, built);
    return { bytes: built.length };
  }
  if (built.sparse) {
    const fd = fs.openSync(target, 'w');
    fs.writeSync(fd, built.header, 0, built.header.length, 0);
    fs.ftruncateSync(fd, built.sparse);
    fs.closeSync(fd);
    return { bytes: built.sparse, sparse: true };
  }
  if (built.symlink) {
    fs.symlinkSync(built.symlink, target);
    return { link: built.symlink };
  }
  if (built.fifo) {
    require('node:child_process').execFileSync('mkfifo', [target]);
    return { fifo: true };
  }
  if (built.folder) {
    fs.mkdirSync(target, { recursive: true });
    return { folder: true };
  }
  throw new Error(`case ${spec.id} built something unknown`);
}

module.exports = { MIB, TYPED, CANARY_PATH, MEDIA_OPS, NO_ROOM_REMEDIATION_START, NAMES, cases, nameCases, materialise, flipped, srt };
