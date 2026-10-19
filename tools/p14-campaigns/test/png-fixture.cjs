'use strict';

// Writes the small PNGs the campaign's tests read (#173): 8-bit, one IDAT,
// rows filtered with none or sub. Test-only.

const zlib = require('node:zlib');

function crc32(bytes) {
  let crc = ~0;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return (~crc) >>> 0;
}

function chunk(type, body) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(body.length);
  const named = Buffer.concat([Buffer.from(type, 'latin1'), body]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(named));
  return Buffer.concat([length, named, crc]);
}

/** A PNG of `pixels` (`channels` bytes a pixel) whose rows use `filter` (0 or 1: none or sub). */
function png(width, height, colour, channels, pixels, filter = 0) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = colour;
  const stride = width * channels;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = filter;
    for (let x = 0; x < stride; x += 1) {
      const value = pixels[y * stride + x];
      const left = filter === 1 && x >= channels ? pixels[y * stride + x - channels] : 0;
      raw[y * (stride + 1) + 1 + x] = (value - left) & 0xff;
    }
  }
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', header), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

module.exports = { png };
