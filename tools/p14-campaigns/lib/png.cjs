'use strict';

// A PNG whose pixels are all zero, claiming any size (P14 RQ-10). The image
// data is the zlib deflate of `height * (width + 1)` zero bytes (each row a
// filter byte and `width` pixels), made in pieces so the generator never holds
// the decoded size; a decoder that believes the header must allocate it. The
// file is a few hundred kilobytes for a quarter of a gigabyte: a
// decompression bomb made by code, with nothing from outside.

const zlib = require('node:zlib');

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function chunk(type, data) {
  const typeBytes = Buffer.from(type, 'latin1');
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(zlib.crc32(Buffer.concat([typeBytes, data])) >>> 0, 0);
  return Buffer.concat([length, typeBytes, data, crc]);
}

/** The deflate stream of `total` zero bytes, in `pieceBytes` pieces. */
function deflateZeros(total, pieceBytes = 1 << 20) {
  return new Promise((resolve, reject) => {
    const deflate = zlib.createDeflate({ level: 9 });
    const parts = [];
    deflate.on('data', (part) => parts.push(part));
    deflate.on('error', reject);
    deflate.on('end', () => resolve(Buffer.concat(parts)));
    const piece = Buffer.alloc(pieceBytes);
    let remaining = total;
    const writeMore = () => {
      while (remaining > 0) {
        const size = Math.min(remaining, pieceBytes);
        remaining -= size;
        if (!deflate.write(size === pieceBytes ? piece : piece.subarray(0, size))) {
          deflate.once('drain', writeMore);
          return;
        }
      }
      deflate.end();
    };
    writeMore();
  });
}

/**
 * @param {number} width the width the image header states
 * @param {number} height the height the image header states
 * @returns {Promise<Buffer>} an 8-bit grayscale PNG of that size, every pixel zero
 */
async function zeroPng(width, height) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 0; // grayscale
  const data = await deflateZeros(height * (width + 1));
  return Buffer.concat([SIGNATURE, chunk('IHDR', ihdr), chunk('IDAT', data), chunk('IEND', Buffer.alloc(0))]);
}

module.exports = { SIGNATURE, chunk, zeroPng };
