'use strict';

// The small-text measurement (#173) on hand-made images: PNGs written here
// with each row filter, and "lines of text" of known heights drawn as
// vertical strokes, so the heights the measurement reports are known.

const assert = require('node:assert/strict');
const test = require('node:test');
const smallText = require('../lib/small-text.cjs');
const { png } = require('./png-fixture.cjs');

/** A light grey image with dark "text lines": one-pixel strokes every 4 pixels, `rows` tall from `top`. */
function page(width, height, lines) {
  const luma = new Uint8Array(width * height).fill(230);
  for (const { top, rows } of lines) {
    for (let y = top; y < top + rows; y += 1) {
      for (let x = 8; x < width - 8; x += 4) luma[y * width + x] = 30;
    }
  }
  return { width, height, luma };
}

test('an RGB PNG is read into its luma, whatever the row filter', () => {
  const rgb = Buffer.from([255, 0, 0, 0, 255, 0, 0, 0, 255, 10, 20, 30]);
  for (const filter of [0, 1]) {
    const image = smallText.readPng(png(2, 2, 2, 3, rgb, filter));
    assert.equal(image.width, 2);
    assert.equal(image.height, 2);
    assert.deepEqual([...image.luma], [76, 150, 29, 18]);
  }
  const grey = smallText.readPng(png(3, 1, 0, 1, Buffer.from([0, 128, 255])));
  assert.deepEqual([...grey.luma], [0, 128, 255]);
});

test('anything but the PNGs VSift writes is refused', () => {
  assert.throws(() => smallText.readPng(Buffer.from('not an image at all, not one bit of it')), smallText.PngError);
  const sixteenBit = png(1, 1, 0, 1, Buffer.from([0]));
  sixteenBit[24] = 16;
  assert.throws(() => smallText.readPng(sixteenBit), smallText.PngError);
});

test('line heights are the bands of text rows, without the bands a crop cuts', () => {
  const image = page(480, 120, [
    { top: 0, rows: 9 }, // cut by the top edge: left out
    { top: 20, rows: 9 },
    { top: 40, rows: 12 },
    { top: 60, rows: 2 }, // a rule, not text
    { top: 70, rows: 6 },
    { top: 112, rows: 8 }, // cut by the bottom edge
  ]);
  assert.deepEqual(smallText.textLineHeights(image), [9, 12, 6]);
});

test('a row with few edges is not text', () => {
  const image = page(480, 40, []);
  for (let y = 10; y < 20; y += 1) for (const x of [100, 200, 300]) image.luma[y * 480 + x] = 0;
  assert.deepEqual(smallText.textLineHeights(image), []);
});

test('line heights are summarised by nearest rank', () => {
  assert.deepEqual(smallText.summariseHeights([12, 9, 9, 30, 11]), {
    lines: 5, min_px: 9, p10_px: 9, median_px: 11, max_px: 30, histogram: { 9: 2, 11: 1, 12: 1, 30: 1 },
  });
  assert.equal(smallText.summariseHeights([]).lines, 0);
});

test('the crop grid covers the frame exactly', () => {
  const grid = smallText.cropGrid(1280, 720);
  assert.equal(grid.length, 18);
  assert.deepEqual(grid[2], { x: 960, y: 0, width: 320, height: 120 });
  const area = grid.reduce((total, rect) => total + rect.width * rect.height, 0);
  assert.equal(area, 1280 * 720);
});
