'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const help = require('../lib/help.cjs');
const jsonReference = require('../lib/json-reference.cjs');

const ROOT_HELP = [
  'Sift technical video into agent-ready evidence',
  '',
  'Usage: vsift.exe [OPTIONS] [COMMAND]',
  '',
  'Commands:',
  '  setup   Inspect and manage the local `VSift` setup',
  '  ingest  Prepare a local video investigation session',
  '',
  'Options:',
  '      --json  Emit one versioned JSON terminal result',
  '',
  'A typical investigation (every command takes --json):',
  '  1. vsift setup check --json',
].join('\r\n');

const HELPS = new Map([
  ['', ROOT_HELP],
  ['setup', 'Inspect and manage\n\nUsage: vsift.exe setup [OPTIONS] <COMMAND>\n\nCommands:\n  check  Inspect dependencies without changing the machine  \n  help   Print this message or the help of the given subcommand(s)\n\nOptions:\n  -h, --help  Print help\n'],
  ['setup check', 'Inspect dependencies\n\n\n\nUsage: vsift.exe setup check [OPTIONS]\n'],
  ['ingest', 'Prepare a session\n\nUsage: vsift.exe ingest [OPTIONS] <SOURCE>\n'],
]);

const helpOf = (args) => {
  assert.equal(args[args.length - 1], '--help');
  const text = HELPS.get(args.slice(0, -1).join(' '));
  assert.notEqual(text, undefined, args.join(' '));
  return text;
};

test('a help text loses the program name of the machine, trailing spaces and extra blank lines', () => {
  const text = help.normaliseHelp(HELPS.get('setup check'));
  assert.equal(text, 'Inspect dependencies\n\nUsage: vsift setup check [OPTIONS]');
});

test('the commands a help lists are read from its Commands section, without the implicit help command', () => {
  assert.deepEqual(help.listedCommands(help.normaliseHelp(ROOT_HELP)), [
    { name: 'setup', summary: 'Inspect and manage the local `VSift` setup' },
    { name: 'ingest', summary: 'Prepare a local video investigation session' },
  ]);
  assert.deepEqual(help.listedCommands(help.normaliseHelp(HELPS.get('setup'))), [
    { name: 'check', summary: 'Inspect dependencies without changing the machine' },
  ]);
  assert.deepEqual(help.listedCommands('No commands here'), []);
});

test('a candidate and its stable release are the same release, so the stable cut changes no guide page', () => {
  assert.equal(help.releaseOf('0.2.0-rc.1'), '0.2.0');
  assert.equal(help.releaseOf('0.2.0'), '0.2.0');
  assert.equal(help.releaseOf('10.20.30+build5'), '10.20.30');
  assert.throws(() => help.releaseOf('0.2'), /not a version/);
});

test('the whole tree is walked, and the page lists every command with its own help', () => {
  const tree = help.readCommandTreeWith(helpOf);
  assert.deepEqual(help.flatten(tree).map(help.headingOf), ['vsift', 'vsift setup', 'vsift setup check', 'vsift ingest']);
  const page = help.renderCommands(tree, '0.9.0');
  assert.match(page, /^# Command reference/);
  assert.match(page, /from vsift 0\.9\.0/);
  assert.match(page, /- \[vsift setup check\]\(#vsift-setup-check\): Inspect dependencies without changing the machine/);
  assert.match(page, /^## vsift ingest\n\nPrepare a local video investigation session\.\n\n```text\nPrepare a session/m);
  assert.ok(!page.includes('vsift.exe'));
  assert.ok(!page.includes('\r'));
  // Rendering twice gives the same bytes: the check compares files.
  assert.equal(page, help.renderCommands(help.readCommandTreeWith(helpOf), '0.9.0'));
});

const SCHEMAS = [
  {
    file: 'operation-response.schema.json',
    schema: {
      title: 'Operation response v1',
      $ref: '#/$defs/operation',
      $defs: {
        operation: {
          type: 'object',
          required: ['status', 'error'],
          properties: {
            status: { enum: ['complete', 'failed'] },
            error: { oneOf: [{ type: 'null' }, { $ref: '#/$defs/error' }] },
            data: { oneOf: [{ type: 'null' }, { type: 'object' }] },
          },
          allOf: [{ if: { properties: { status: { const: 'failed' } } }, then: { required: ['error'] } }],
        },
        error: {
          type: 'object',
          required: ['code'],
          properties: { code: { enum: ['BUSY', 'INTERNAL'] }, retry_after_ms: { oneOf: [{ type: 'null' }, { type: 'integer', minimum: 1, maximum: 86400000 }] } },
        },
      },
    },
  },
  {
    file: 'ingest-data.schema.json',
    schema: {
      title: 'Ingest data v1',
      description: 'The data member.',
      type: 'object',
      required: ['session_id', 'transcript'],
      properties: {
        session_id: { type: 'string', pattern: '^ses_[a-z0-9]{16,64}$' },
        transcript: { $ref: 'transcript-revision.schema.json' },
        ranges: { type: 'array', maxItems: 4, items: { type: 'object', required: ['from'], properties: { from: { type: 'integer', minimum: 0 } } } },
        selections: { allOf: [{ $ref: '#/$defs/list' }], maxItems: 1 },
      },
      $defs: { list: { type: 'array', items: { type: 'string' } } },
    },
  },
  { file: 'transcript-revision.schema.json', schema: { title: 'Revision', type: 'object', properties: { revision_id: { type: 'string' } } } },
];

test('the JSON reference lists members with their kinds and limits, and nested members by dotted name', () => {
  const page = jsonReference.renderJson(SCHEMAS, '0.9.0');
  assert.match(page, /### ingest-data\.schema\.json/);
  assert.match(page, /\| `session_id` \| string \| yes \| pattern `\^ses_\[a-z0-9\]\{16,64\}\$` \|/);
  assert.match(page, /\| `transcript` \| object, see \[transcript-revision\.schema\.json\]\(#transcript-revisionschemajson\) \| yes \|/);
  assert.match(page, /\| `ranges\[\]\.from` \| integer \| yes \| at least 0 \|/);
  assert.match(page, /\| `selections` \| array of string \| no \| at most 1 items \|/);
  // A reference is followed, a union with null is shown as "or null", an enum is listed.
  assert.match(page, /\| `error\.code` \| "BUSY" \\\| "INTERNAL" \| yes \|/);
  assert.match(page, /\| `error` \| object or null \| yes \|/);
  assert.match(page, /\| `error\.retry_after_ms` \| integer or null \| no \| 1 to 86400000 \|/);
  assert.match(page, /1 conditional rule/);
  assert.match(page, /## Failure codes\n[\s\S]*- `BUSY`\n- `INTERNAL`/);
  assert.equal(page, jsonReference.renderJson(SCHEMAS, '0.9.0'));
});

test('the failure codes are read from the terminal response schema', () => {
  const byFile = new Map(SCHEMAS.map(({ file, schema }) => [file, schema]));
  assert.deepEqual(jsonReference.failureCodes(byFile), ['BUSY', 'INTERNAL']);
  assert.deepEqual(jsonReference.failureCodes(new Map()), []);
});

test('bounds read as plain phrases', () => {
  assert.deepEqual(jsonReference.boundNotes({ minimum: 1, maximum: 1 }), ['exactly 1']);
  assert.deepEqual(jsonReference.boundNotes({ minLength: 1 }), ['at least 1 characters']);
  assert.deepEqual(jsonReference.boundNotes({ maxItems: 100, uniqueItems: true }), ['at most 100 items', 'items are unique']);
});
