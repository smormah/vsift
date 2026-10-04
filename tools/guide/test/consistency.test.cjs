'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const consistency = require('../lib/consistency.cjs');

const CONTRACT = [
  '# Contract',
  '',
  '## Exit and error taxonomy',
  '',
  '| Exit | Category | Representative codes |',
  '| ---: | --- | --- |',
  '| 0 | complete | none |',
  '| 2 | usage | `INVALID_ARGUMENT`, `MISSING_CAPABILITY` |',
  '| 7 | storage | `STORAGE_IO` |',
  '',
  '## Next section',
].join('\n');

const SCHEMA = { $defs: { error: { properties: { code: { enum: ['INVALID_ARGUMENT', 'MISSING_CAPABILITY', 'STORAGE_IO'] } } } } };

/** A small repository with a guide that is consistent; a test then breaks one thing. */
function repository(overrides = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'guide-consistency-test-'));
  const files = {
    'docs/contracts/cli-v1.md': CONTRACT,
    'schemas/v1/operation-response.schema.json': JSON.stringify(SCHEMA),
    'docs/planning/public-claims.json': JSON.stringify({ documents: ['docs/guide/index.md', 'docs/guide/troubleshooting.md', 'docs/guide/other.md'] }),
    'docs/guide/index.md': '# Guide\n\n<!-- guide-version: 0.9.0 -->\n\nSee [other](other.md#a-heading) and [here](#guide).\n',
    'docs/guide/other.md': '# Other\n\n## A heading\n\n## `CODE`: a heading\n\nSee [back](index.md).\n',
    'docs/guide/troubleshooting.md': [
      '# Troubleshooting',
      '',
      '| Code | Exit | Meaning |',
      '| --- | ---: | --- |',
      '| `INVALID_ARGUMENT` | 2 | wrong |',
      '| `MISSING_CAPABILITY` | 2 | missing |',
      '| `STORAGE_IO` | 7 | file |',
    ].join('\n'),
    ...overrides,
  };
  for (const [name, text] of Object.entries(files)) {
    if (text === null) continue;
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), text);
  }
  return root;
}

test('a consistent guide has no problem', () => {
  assert.deepEqual(consistency.check(repository(), '0.9.0'), []);
});

test('the version the guide says it was checked against must be the binary\'s', () => {
  const problems = consistency.check(repository(), '0.10.0');
  assert.equal(problems.length, 1);
  assert.match(problems[0], /checked against 0\.9\.0, but the binary under test is 0\.10\.0/);
  assert.match(consistency.check(repository({ 'docs/guide/index.md': '# Guide\n' }), '0.9.0')[0], /no <!-- guide-version: X -->/);
});

test('the version the guide names is a release, not a candidate, so the stable cut needs no guide change', () => {
  const candidate = repository({ 'docs/guide/index.md': '# Guide\n\n<!-- guide-version: 0.9.0-rc.1 -->\n' });
  const problems = consistency.check(candidate, '0.9.0');
  assert.equal(problems.length, 1);
  assert.match(problems[0], /write the release without a candidate suffix/);
});

test('every failure code of the schema needs a row with the exit status the contract gives it', () => {
  const missing = repository({ 'docs/guide/troubleshooting.md': '| `INVALID_ARGUMENT` | 2 | x |\n| `MISSING_CAPABILITY` | 2 | x |\n' });
  assert.match(consistency.troubleshootingProblems(missing)[0], /no row for the failure code STORAGE_IO/);
  const wrong = repository({ 'docs/guide/troubleshooting.md': '| `INVALID_ARGUMENT` | 2 | x |\n| `MISSING_CAPABILITY` | 3 | x |\n| `STORAGE_IO` | 7 | x |\n' });
  assert.match(consistency.troubleshootingProblems(wrong)[0], /MISSING_CAPABILITY exit status 3, but the contract's taxonomy says 2/);
  const invented = repository({ 'docs/guide/troubleshooting.md': '| `INVALID_ARGUMENT` | 2 | x |\n| `MISSING_CAPABILITY` | 2 | x |\n| `STORAGE_IO` | 7 | x |\n| `MADE_UP` | 1 | x |\n' });
  assert.match(consistency.troubleshootingProblems(invented)[0], /MADE_UP, which is not a v1 failure code/);
  assert.deepEqual([...consistency.contractExitStatuses(repository())], [['INVALID_ARGUMENT', 2], ['MISSING_CAPABILITY', 2], ['STORAGE_IO', 7]]);
});

test('a relative link must lead to a file, and a fragment to a heading of that file', () => {
  const broken = repository({ 'docs/guide/other.md': '# Other\n\n[x](gone.md) [y](index.md#nope) [z](#absent) [ok](https://example.test/a#b)\n`[code](not-a-link.md)`\n' });
  const problems = consistency.linkProblems(broken);
  assert.equal(problems.length, 4, problems.join('\n'));
  assert.ok(problems.some((problem) => /gone\.md, which does not exist/.test(problem)));
  assert.ok(problems.some((problem) => /index\.md#nope, but that page has no heading/.test(problem)));
  assert.ok(problems.some((problem) => /#absent, but that page has no heading/.test(problem)));
  // index.md still links to a heading that other.md no longer has.
  assert.ok(problems.some((problem) => /other\.md#a-heading/.test(problem)));
});

test('a heading offers the anchor the common sites give it', () => {
  assert.equal(consistency.slugOf('9. Where to go next'), '9-where-to-go-next');
  assert.equal(consistency.slugOf('`MISSING_CAPABILITY`: a tool or model is missing'), 'missing_capability-a-tool-or-model-is-missing');
  assert.equal(consistency.slugOf('5.2 Windows, macOS and other machines: bring your own'), '52-windows-macos-and-other-machines-bring-your-own');
  assert.equal(consistency.slugOf('audio-data.schema.json'), 'audio-dataschemajson');
  const anchors = consistency.anchorsOf('# A\n\n## Same\n\n## Same\n\n```\n# not a heading\n```\n<b id="custom">x</b>\n');
  assert.deepEqual([...anchors].sort(), ['a', 'custom', 'same', 'same-1']);
});

test('every guide page must be in the claims registry\'s scanned documents', () => {
  const root = repository({ 'docs/guide/new-page.md': '# New\n' });
  const problems = consistency.registryProblems(root);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /docs\/guide\/new-page\.md is not in the documents of docs\/planning\/public-claims\.json/);
});
