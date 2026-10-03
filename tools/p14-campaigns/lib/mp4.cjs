'use strict';

// A small ISO base media (MP4) box tree, for making hostile variants of a real
// file by code (P14 RQ-10): parse a fixture into boxes, change fields or add
// boxes, write it out with every size recomputed. Nothing here is a media
// decoder; a box this module does not know is kept as opaque bytes. The
// hostile files are generated at run time and never committed.

/** Boxes whose payload is a sequence of boxes. */
const CONTAINERS = new Set(['moov', 'trak', 'mdia', 'minf', 'stbl', 'dinf', 'edts', 'udta']);

/**
 * @typedef {{ type: string, payload: Buffer, children: Box[]|null }} Box
 */

/** Reads the boxes of `buffer` from `start` to `end`. Large (64-bit) sizes are read; a size of 0 runs to the end. */
function parse(buffer, start = 0, end = buffer.length) {
  const boxes = [];
  let offset = start;
  while (offset + 8 <= end) {
    let size = buffer.readUInt32BE(offset);
    const type = buffer.toString('latin1', offset + 4, offset + 8);
    let header = 8;
    if (size === 1) {
      size = Number(buffer.readBigUInt64BE(offset + 8));
      header = 16;
    } else if (size === 0) {
      size = end - offset;
    }
    if (size < header || offset + size > end) {
      throw new Error(`box ${type} at ${offset} has the size ${size}, which does not fit`);
    }
    const payload = buffer.subarray(offset + header, offset + size);
    boxes.push({ type, payload: Buffer.from(payload), children: CONTAINERS.has(type) ? parse(buffer, offset + header, offset + size) : null });
    offset += size;
  }
  if (offset !== end) throw new Error(`${end - offset} bytes are left after the last box`);
  return boxes;
}

/** Writes boxes back, with 32-bit sizes (a payload past 4 GiB is not made here). */
function serialize(boxes) {
  const parts = [];
  for (const box of boxes) {
    const payload = box.children ? serialize(box.children) : box.payload;
    const header = Buffer.alloc(8);
    header.writeUInt32BE(payload.length + 8, 0);
    header.write(box.type, 4, 'latin1');
    parts.push(header, payload);
  }
  return Buffer.concat(parts);
}

/** The first box at `path` (types, from the top), or null. */
function find(boxes, path) {
  let level = boxes;
  let found = null;
  for (const type of path) {
    found = (level || []).find((box) => box.type === type) || null;
    if (!found) return null;
    level = found.children;
  }
  return found;
}

/** Every box at `path` (the last step may match many). */
function findAll(boxes, path) {
  if (path.length === 1) return boxes.filter((box) => box.type === path[0]);
  return boxes.filter((box) => box.type === path[0]).flatMap((box) => findAll(box.children || [], path.slice(1)));
}

/** A box of `type` holding `payload` (bytes) or `children` (boxes). */
function make(type, payload = Buffer.alloc(0), children = null) {
  return { type, payload, children };
}

/** Sets the movie header's timescale and duration (version 0 or 1). */
function setMovieDuration(boxes, timescale, duration) {
  const mvhd = find(boxes, ['moov', 'mvhd']);
  if (!mvhd) throw new Error('the file has no movie header');
  if (mvhd.payload[0] === 1) {
    mvhd.payload.writeUInt32BE(timescale, 20);
    mvhd.payload.writeBigUInt64BE(BigInt(duration), 24);
  } else {
    mvhd.payload.writeUInt32BE(timescale, 12);
    mvhd.payload.writeUInt32BE(Number(duration), 16);
  }
}

/**
 * Declares `seconds` as the length of every track: the media header (in the track's own timescale) and
 * the track header (in the movie's). FFprobe reports a container's length from its tracks, so a lie
 * confined to the movie header changes nothing; this one reaches the number the product reads.
 * Durations are clamped to what a version 0 header can hold.
 */
function setTrackDurations(boxes, seconds) {
  const limit = 0xffffffff;
  const movie = find(boxes, ['moov', 'mvhd']);
  if (!movie) throw new Error('the file has no movie header');
  const movieScale = movie.payload.readUInt32BE(movie.payload[0] === 1 ? 20 : 12);
  for (const trak of findAll(boxes, ['moov', 'trak'])) {
    const mdhd = find(trak.children, ['mdia', 'mdhd']);
    const tkhd = find(trak.children, ['tkhd']);
    if (mdhd && mdhd.payload[0] === 0) {
      mdhd.payload.writeUInt32BE(Math.min(limit, Math.round(seconds * mdhd.payload.readUInt32BE(12))), 16);
    }
    if (tkhd && tkhd.payload[0] === 0) {
      tkhd.payload.writeUInt32BE(Math.min(limit, Math.round(seconds * movieScale)), 20);
    }
  }
}

/** Sets every video track's declared size: the track header (16.16 fixed point) and the sample entry. */
function setDimensions(boxes, width, height) {
  for (const tkhd of findAll(boxes, ['moov', 'trak', 'tkhd'])) {
    const at = tkhd.payload[0] === 1 ? 88 : 76;
    tkhd.payload.writeUInt32BE((width * 65536) >>> 0, at);
    tkhd.payload.writeUInt32BE((height * 65536) >>> 0, at + 4);
  }
  for (const stsd of findAll(boxes, ['moov', 'trak', 'mdia', 'minf', 'stbl', 'stsd'])) {
    // version/flags, entry count, then the first entry's size and type, 6 reserved, 2 reference, 2 + 2 + 12 predefined/reserved.
    const entry = 8 + 8;
    if (stsd.payload.length >= entry + 28) {
      stsd.payload.writeUInt16BE(Math.min(width, 65535), entry + 24);
      stsd.payload.writeUInt16BE(Math.min(height, 65535), entry + 26);
    }
  }
}

/** The 4 bytes of a box type's name plus the size, as an opaque box at the end of a container's children. */
function appendChild(parent, child) {
  if (!parent.children) throw new Error(`${parent.type} is not a container`);
  parent.children.push(child);
}

/** `depth` boxes of `type` nested one inside the next, the innermost empty. */
function nested(type, depth) {
  let box = make(type);
  for (let level = 1; level < depth; level += 1) box = make(type, Buffer.alloc(0), [box]);
  // A container type holds its children; an unknown one holds them as bytes.
  return box;
}

/** A nested chain of `depth` unknown-type boxes, written as bytes so no container list has to be recursive. */
function nestedBytes(type, depth) {
  // Build from the inside out: each level is an 8-byte header around the previous level.
  let inner = Buffer.alloc(0);
  for (let level = 0; level < depth; level += 1) {
    const header = Buffer.alloc(8);
    header.writeUInt32BE(inner.length + 8, 0);
    header.write(type, 4, 'latin1');
    inner = Buffer.concat([header, inner]);
  }
  return inner;
}

module.exports = { CONTAINERS, parse, serialize, find, findAll, make, setMovieDuration, setTrackDurations, setDimensions, appendChild, nested, nestedBytes };
