'use strict';

// A small Matroska/EBML writer for hostile files made by code (P14 RQ-10):
// the elements a demuxer reads first (the header, the segment's info and
// tracks, one cluster with a block), so a variant can lie about one field or
// repeat one element thousands of times. It writes what the format describes;
// it is not a muxer.

/** An EBML element id as bytes (ids keep their length marker). */
function id(hex) {
  return Buffer.from(hex, 'hex');
}

/** The EBML variable-length size of `value` in the fewest bytes (1 to 8). */
function vint(value) {
  const big = BigInt(value);
  for (let length = 1; length <= 8; length += 1) {
    // All ones is reserved for "unknown size", so a size needs one value less.
    const limit = (1n << BigInt(7 * length)) - 1n;
    if (big < limit) {
      let remaining = big | (1n << BigInt(7 * length));
      const bytes = Buffer.alloc(length);
      for (let index = length - 1; index >= 0; index -= 1) {
        bytes[index] = Number(remaining & 0xffn);
        remaining >>= 8n;
      }
      return bytes;
    }
  }
  throw new Error(`${value} does not fit an EBML size`);
}

/** The "unknown size" marker of 8 bytes. */
const UNKNOWN_SIZE = Buffer.from('01ffffffffffffff', 'hex');

/** An element: its id, its size and its payload. */
function element(elementId, payload) {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  return Buffer.concat([id(elementId), vint(body.length), body]);
}

/** An element whose size field claims `claimed` bytes whatever follows. */
function elementClaiming(elementId, claimed, payload) {
  return Buffer.concat([id(elementId), vint(claimed), payload]);
}

/** An element with the unknown-size marker, as a live stream writes its segment and clusters. */
function unknownSized(elementId, payload) {
  return Buffer.concat([id(elementId), UNKNOWN_SIZE, payload]);
}

/** An unsigned integer element payload in the fewest bytes. */
function uint(value) {
  let big = BigInt(value);
  const bytes = [];
  do {
    bytes.unshift(Number(big & 0xffn));
    big >>= 8n;
  } while (big > 0n);
  return Buffer.from(bytes);
}

/** A 64-bit float element payload. */
function float64(value) {
  const bytes = Buffer.alloc(8);
  bytes.writeDoubleBE(value, 0);
  return bytes;
}

/** A 32-bit float element payload. */
function float32(value) {
  const bytes = Buffer.alloc(4);
  bytes.writeFloatBE(value, 0);
  return bytes;
}

const IDS = {
  ebml: '1a45dfa3', ebmlVersion: '4286', ebmlReadVersion: '42f7', ebmlMaxIdLength: '42f2', ebmlMaxSizeLength: '42f3',
  docType: '4282', docTypeVersion: '4287', docTypeReadVersion: '4285',
  segment: '18538067', info: '1549a966', timestampScale: '2ad7b1', muxingApp: '4d80', writingApp: '5741', duration: '4489',
  tracks: '1654ae6b', trackEntry: 'ae', trackNumber: 'd7', trackUid: '73c5', trackType: '83', flagLacing: '9c', codecId: '86', codecPrivate: '63a2',
  video: 'e0', pixelWidth: 'b0', pixelHeight: 'ba', audio: 'e1', samplingFrequency: 'b5', channels: '9f', bitDepth: '6264',
  cluster: '1f43b675', timestamp: 'e7', simpleBlock: 'a3', void: 'ec', tags: '1254c367', tag: '7373', simpleTag: '67c8', tagName: '45a3', tagString: '4487',
};

/** The EBML header of a Matroska file. */
function header() {
  return element(IDS.ebml, Buffer.concat([
    element(IDS.ebmlVersion, uint(1)),
    element(IDS.ebmlReadVersion, uint(1)),
    element(IDS.ebmlMaxIdLength, uint(4)),
    element(IDS.ebmlMaxSizeLength, uint(8)),
    element(IDS.docType, 'matroska'),
    element(IDS.docTypeVersion, uint(4)),
    element(IDS.docTypeReadVersion, uint(2)),
  ]));
}

/** Segment info with a timestamp scale of 1 ms and the given duration (in those units). */
function info(durationUnits) {
  return element(IDS.info, Buffer.concat([
    element(IDS.timestampScale, uint(1_000_000)),
    element(IDS.muxingApp, 'p14'),
    element(IDS.writingApp, 'p14'),
    element(IDS.duration, float64(durationUnits)),
  ]));
}

/**
 * One track entry.
 * @param {{ number: number, type: 1|2, codec: string, width?: number, height?: number, codecPrivate?: Buffer, sampleRate?: number, channels?: number }} spec
 */
function track(spec) {
  const parts = [
    element(IDS.trackNumber, uint(spec.number)),
    element(IDS.trackUid, uint(spec.number)),
    element(IDS.trackType, uint(spec.type)),
    element(IDS.flagLacing, uint(0)),
    element(IDS.codecId, spec.codec),
  ];
  if (spec.codecPrivate) parts.push(element(IDS.codecPrivate, spec.codecPrivate));
  if (spec.type === 1) {
    parts.push(element(IDS.video, Buffer.concat([
      element(IDS.pixelWidth, uint(spec.width || 16)),
      element(IDS.pixelHeight, uint(spec.height || 16)),
    ])));
  } else {
    parts.push(element(IDS.audio, Buffer.concat([
      element(IDS.samplingFrequency, float32(spec.sampleRate || 16000)),
      element(IDS.channels, uint(spec.channels || 1)),
    ])));
  }
  return element(IDS.trackEntry, Buffer.concat(parts));
}

/** A cluster holding one keyframe block for track `trackNumber`. */
function cluster(trackNumber, frame) {
  const block = Buffer.concat([vint(trackNumber), Buffer.from([0, 0, 0x80]), frame]);
  return element(IDS.cluster, Buffer.concat([element(IDS.timestamp, uint(0)), element(IDS.simpleBlock, block)]));
}

/** A whole file: header, then a segment of the given parts. */
function file(parts, { unknownSize = false } = {}) {
  const body = Buffer.concat(parts);
  return Buffer.concat([header(), unknownSize ? unknownSized(IDS.segment, body) : element(IDS.segment, body)]);
}

/** A BITMAPINFOHEADER of the `V_MS/VFW/FOURCC` codec private data, naming a fourcc (for example `MPNG`). */
function bitmapInfo(width, height, fourcc) {
  const bytes = Buffer.alloc(40);
  bytes.writeUInt32LE(40, 0);
  bytes.writeInt32LE(width, 4);
  bytes.writeInt32LE(height, 8);
  bytes.writeUInt16LE(1, 12);
  bytes.writeUInt16LE(24, 14);
  bytes.write(fourcc, 16, 'latin1');
  return bytes;
}

module.exports = {
  IDS, vint, UNKNOWN_SIZE, element, elementClaiming, unknownSized, uint, float64, float32, header, info, track, cluster, file, bitmapInfo,
};
