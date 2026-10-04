'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const { extractBlocks, parseMarker } = require('../lib/blocks.cjs');
const { splitCommand, usesShellSyntax } = require('../lib/shellwords.cjs');

const PAGE = [
  '# A page',
  '',
  'Text.',
  '',
  '<!-- check -->',
  '```console',
  '$ vsift ingest ./a.mp4',
  'Opened session ses_1234567890abcdef1234567890abcdef',
  '',
  'Lifecycle: ephemeral',
  '$ vsift session close <session>',
  'Closed session ses_1234567890abcdef1234567890abcdef',
  '```',
  '',
  'An illustration, never run:',
  '',
  '```console',
  '$ vsift setup check',
  'Status: ready',
  '```',
  '',
  '<!-- check: needs speech; exit 3 -->',
  '',
  '```console',
  '$ vsift transcript retranscribe <session>',
  '```',
].join('\n');

test('only marked console blocks are examples, each command with the output shown for it', () => {
  const blocks = extractBlocks(PAGE);
  assert.equal(blocks.length, 2);
  assert.equal(blocks[0].line, 5);
  assert.deepEqual(blocks[0].options.exit, 0);
  assert.equal(blocks[0].commands.length, 2);
  assert.equal(blocks[0].commands[0].text, 'vsift ingest ./a.mp4');
  // A blank line inside the output is kept; blank lines at its end are not.
  assert.deepEqual(blocks[0].commands[0].expected, ['Opened session ses_1234567890abcdef1234567890abcdef', '', 'Lifecycle: ephemeral']);
  assert.deepEqual(blocks[0].commands[1].expected, ['Closed session ses_1234567890abcdef1234567890abcdef']);
  assert.equal(blocks[1].options.exit, 3);
  assert.ok(blocks[1].options.needs.has('speech'));
  assert.deepEqual(blocks[1].commands[0].expected, []);
});

test('a marker needs a console block, and a block needs a command', () => {
  assert.throws(() => extractBlocks('<!-- check -->\n\ntext only\n'), /must be followed by a console block/);
  assert.throws(() => extractBlocks('<!-- check -->\n```console\n```\n'), /holds no command/);
  assert.throws(() => extractBlocks('<!-- check -->\n```console\nstray output\n$ vsift x\n```\n'), /output before the first command/);
  assert.throws(() => extractBlocks('<!-- check -->\n```console\n$ vsift x\n'), /not closed/);
});

test('marker options are closed', () => {
  assert.deepEqual([...parseMarker('needs speech').needs], ['speech']);
  assert.equal(parseMarker('exit 7').exit, 7);
  assert.equal(parseMarker(undefined).exit, 0);
  assert.throws(() => parseMarker('sometimes'), /unknown check option/);
});

test('a command line splits like a shell would for words and quotes, and shell syntax is spotted', () => {
  assert.deepEqual(splitCommand('vsift search ses_1 --query "order 1017"'), ['vsift', 'search', 'ses_1', '--query', 'order 1017']);
  assert.deepEqual(splitCommand("vsift crop a b --rect '0,0,400,100'"), ['vsift', 'crop', 'a', 'b', '--rect', '0,0,400,100']);
  assert.deepEqual(splitCommand('vsift x ""'), ['vsift', 'x', '']);
  assert.throws(() => splitCommand('vsift "unclosed'), /unclosed quote/);
  for (const line of ['vsift a | head', 'vsift a > out', 'vsift a && vsift b', 'vsift a $HOME', 'vsift a `b`']) {
    assert.ok(usesShellSyntax(line), line);
  }
  assert.ok(!usesShellSyntax('vsift search s --query "a | b"'));
});
