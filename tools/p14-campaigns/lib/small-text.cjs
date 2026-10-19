'use strict';

// The small-text measurement of the real-media campaign (#173, register L-018;
// docs/planning/real-media-set.md section 7, check 9). VSift's `crop` returns
// a frame's own pixels at native size and never scales them; whether a reader
// can make out real interface text then depends on how tall that text is in
// pixels and how hard the recording was compressed. This module measures the
// first mechanically on the crops VSift returned: it reads each PNG (the
// subset VSift writes) and estimates the height of every line of text in it.
// Legibility itself is still a person's yes or no (check 9); these numbers say
// what size of text that judgement is about. Everything is pure.

const zlib = require('node:zlib');

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
/** Channels per colour type of the PNGs read here: grey, RGB, grey and alpha, RGBA (8 bits each). */
const CHANNELS = Object.freeze({ 0: 1, 2: 3, 4: 2, 6: 4 });
/** Largest image read: a 4K frame. */
const MAX_PIXELS = 3840 * 2160;
/** A horizontal step of at least this many grey levels is an edge (glyph strokes on their background). */
const EDGE_LEVELS = 48;
/** A row is a text row when it holds at least this many edges, and at least one per this many pixels. */
const MIN_ROW_EDGES = 6;
const PIXELS_PER_ROW_EDGE = 40;
/** Bands of text rows outside these heights are rules, borders or pictures, not a line of text. */
const MIN_LINE_PX = 4;
const MAX_LINE_PX = 64;

class PngError extends Error {}

function paeth(left, up, upLeft) {
  const estimate = left + up - upLeft;
  const toLeft = Math.abs(estimate - left);
  const toUp = Math.abs(estimate - up);
  const toUpLeft = Math.abs(estimate - upLeft);
  if (toLeft <= toUp && toLeft <= toUpLeft) return left;
  return toUp <= toUpLeft ? up : upLeft;
}

/**
 * Reads an 8-bit, non-interlaced PNG (grey, RGB, grey with alpha or RGBA:
 * what VSift's evidence encoder writes) into its luma, one byte a pixel
 * (BT.601 weights, rounded; alpha ignored). Anything else is refused.
 * @param {Buffer} bytes
 * @returns {{ width: number, height: number, luma: Uint8Array }}
 */
function readPng(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length < 33 || !bytes.subarray(0, 8).equals(PNG_SIGNATURE)) throw new PngError('not a PNG');
  let offset = 8;
  let header = null;
  const data = [];
  while (offset + 12 <= bytes.length) {
    const length = bytes.readUInt32BE(offset);
    const type = bytes.toString('latin1', offset + 4, offset + 8);
    const body = bytes.subarray(offset + 8, offset + 8 + length);
    if (body.length !== length) throw new PngError('a chunk runs past the end');
    if (type === 'IHDR') {
      header = { width: body.readUInt32BE(0), height: body.readUInt32BE(4), depth: body[8], colour: body[9], interlace: body[12] };
    } else if (type === 'IDAT') {
      data.push(body);
    } else if (type === 'IEND') {
      break;
    }
    offset += 12 + length;
  }
  if (!header) throw new PngError('no header');
  const channels = CHANNELS[header.colour];
  if (header.depth !== 8 || !channels || header.interlace !== 0) throw new PngError('only 8-bit, non-interlaced grey, RGB, grey-alpha and RGBA images are read');
  const { width, height } = header;
  if (width < 1 || height < 1 || width * height > MAX_PIXELS) throw new PngError('the image size is out of bounds');
  const stride = width * channels;
  const raw = zlib.inflateSync(Buffer.concat(data), { maxOutputLength: (stride + 1) * height });
  if (raw.length !== (stride + 1) * height) throw new PngError('the image data has the wrong length');
  const pixels = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    for (let x = 0; x < stride; x += 1) {
      const value = raw[y * (stride + 1) + 1 + x];
      const left = x >= channels ? pixels[y * stride + x - channels] : 0;
      const up = y > 0 ? pixels[(y - 1) * stride + x] : 0;
      const upLeft = x >= channels && y > 0 ? pixels[(y - 1) * stride + x - channels] : 0;
      let predicted;
      if (filter === 0) predicted = 0;
      else if (filter === 1) predicted = left;
      else if (filter === 2) predicted = up;
      else if (filter === 3) predicted = (left + up) >> 1;
      else if (filter === 4) predicted = paeth(left, up, upLeft);
      else throw new PngError(`unknown row filter ${filter}`);
      pixels[y * stride + x] = (value + predicted) & 0xff;
    }
  }
  const luma = new Uint8Array(width * height);
  for (let index = 0; index < width * height; index += 1) {
    const base = index * channels;
    luma[index] = channels >= 3
      ? Math.round(0.299 * pixels[base] + 0.587 * pixels[base + 1] + 0.114 * pixels[base + 2])
      : pixels[base];
  }
  return { width, height, luma };
}

/**
 * The heights in pixels of the lines of text in an image: bands of
 * consecutive rows that each hold many strong horizontal edges (glyph
 * strokes against their background), between `MIN_LINE_PX` and
 * `MAX_LINE_PX` tall. A band touching the image's top or bottom edge is cut
 * by the crop and left out. The height of a band is the line's height from
 * its tallest glyph to its lowest descender as compression left it, which is
 * close to the font's pixel size.
 * @param {{ width: number, height: number, luma: Uint8Array }} image
 * @returns {number[]}
 */
function textLineHeights(image) {
  const { width, height, luma } = image;
  const needed = Math.max(MIN_ROW_EDGES, Math.floor(width / PIXELS_PER_ROW_EDGE));
  const texty = [];
  for (let y = 0; y < height; y += 1) {
    let edges = 0;
    for (let x = 1; x < width; x += 1) {
      if (Math.abs(luma[y * width + x] - luma[y * width + x - 1]) >= EDGE_LEVELS) edges += 1;
    }
    texty.push(edges >= needed);
  }
  const lines = [];
  let start = null;
  for (let y = 0; y <= height; y += 1) {
    const on = y < height && texty[y];
    if (on && start === null) start = y;
    if (!on && start !== null) {
      const size = y - start;
      const cut = start === 0 || y === height;
      if (!cut && size >= MIN_LINE_PX && size <= MAX_LINE_PX) lines.push(size);
      start = null;
    }
  }
  return lines;
}

/**
 * A summary of line heights: how many, the smallest, the 10th percentile,
 * the median and the largest (nearest rank), and a histogram.
 * @param {number[]} heights
 */
function summariseHeights(heights) {
  if (heights.length === 0) return { lines: 0, min_px: null, p10_px: null, median_px: null, max_px: null, histogram: {} };
  const sorted = [...heights].sort((a, b) => a - b);
  const rank = (percent) => sorted[Math.max(1, Math.ceil((sorted.length * percent) / 100)) - 1];
  const histogram = {};
  for (const size of sorted) histogram[size] = (histogram[size] || 0) + 1;
  return { lines: sorted.length, min_px: sorted[0], p10_px: rank(10), median_px: rank(50), max_px: sorted[sorted.length - 1], histogram };
}

/**
 * The rectangles a frame is cut into for the measurement: a grid of crops at
 * most `cropWidth` x `cropHeight`, covering the whole frame.
 * @param {number} width
 * @param {number} height
 */
function cropGrid(width, height, cropWidth = 480, cropHeight = 120) {
  const rects = [];
  for (let y = 0; y < height; y += cropHeight) {
    for (let x = 0; x < width; x += cropWidth) {
      rects.push({ x, y, width: Math.min(cropWidth, width - x), height: Math.min(cropHeight, height - y) });
    }
  }
  return rects;
}

module.exports = { PngError, readPng, textLineHeights, summariseHeights, cropGrid, EDGE_LEVELS, MIN_LINE_PX, MAX_LINE_PX };
