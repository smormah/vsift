'use strict';

// The Guide workflow is read-only and holds no secret (P14 PR 9b). These tests read the workflow
// file the way `tools/p14-published/test/pins.test.cjs` reads the P14 ones: nothing here can
// publish, tag, log in or write, every action is pinned to the commit the Release workflow uses,
// and no expression of the event reaches a shell.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const repository = path.join(__dirname, '..', '..', '..');
const read = (relative) => fs.readFileSync(path.join(repository, relative), 'utf8');
const workflow = read('.github/workflows/guide.yml');
/** The workflow without its comment lines. */
const code = workflow.replace(/^\s*#.*$/gm, '');

test('the workflow grants nothing at the top and one read scope to each job', () => {
  assert.match(code, /^permissions: \{\}$/m, 'no empty top-level permissions');
  const jobs = code.slice(code.indexOf('\njobs:'));
  const names = [...jobs.matchAll(/^ {2}([a-z][a-z-]*):\n {4}name:/gm)].map((match) => match[1]);
  assert.deepEqual(names, ['reference', 'examples']);
  for (const name of names) {
    const start = jobs.indexOf(`\n  ${name}:\n`);
    const next = names.map((other) => jobs.indexOf(`\n  ${other}:\n`)).filter((index) => index > start).sort((a, b) => a - b)[0];
    const job = jobs.slice(start, next);
    assert.match(job, /^ {4}permissions:\n {6}contents: read\n/m, `${name} does not name contents: read`);
    assert.equal((job.match(/^ {6}[a-z-]+: (read|write|none)$/gm) || []).length, 1, `${name} names another scope`);
  }
});

test('nothing in the workflow can publish, tag, log in or write, and no secret or token is named', () => {
  assert.ok(!/id-token/.test(code), 'names id-token');
  assert.ok(!/secrets\./.test(code), 'names a secret');
  assert.ok(!/GITHUB_TOKEN|github\.token|GH_TOKEN/.test(code), 'names a token');
  assert.ok(!/:\s*write\b/.test(code), 'grants a write scope');
  assert.ok(!/environment:/.test(code), 'names an environment');
  assert.ok(!/pull_request_target/.test(code), 'uses pull_request_target');
  assert.ok(!/npm (publish|dist-tag|login|adduser|unpublish|deprecate|owner|access|token)/.test(code), 'runs a publishing npm command');
  assert.ok(!/gh (release|workflow run|api)\b/.test(code), 'runs a gh command');
  assert.ok(!/git (tag|push)/.test(code), 'tags or pushes');
});

test('every action is pinned to the commit the Release workflow uses, and a checkout keeps no credential', () => {
  const pins = (text) => new Map([...text.matchAll(/uses:\s+([\w.-]+\/[\w./-]+)@([0-9a-f]{40})\s+#\s*(\S+)/g)].map((match) => [match[1], `${match[2]} ${match[3]}`]));
  const release = pins(read('.github/workflows/release.yml'));
  const own = pins(workflow);
  assert.ok(own.size >= 2);
  for (const [action, pin] of own) {
    assert.equal(pin, release.get(action), `${action} is not pinned as the Release workflow pins it`);
  }
  // Every `uses:` is a pin: no tag or branch reference slipped past the pattern above.
  assert.equal([...code.matchAll(/^\s*-?\s*uses:/gm)].length, [...code.matchAll(/uses:\s+[\w.-]+\/[\w./-]+@[0-9a-f]{40}\s+#/g)].length);
  const checkouts = [...code.matchAll(/actions\/checkout@[0-9a-f]{40}[^\n]*\n((?: {8,}[^\n]*\n)*)/g)];
  assert.ok(checkouts.length >= 2);
  for (const [, options] of checkouts) {
    assert.match(options, /persist-credentials: false/);
  }
});

test('no expression reaches a shell: the only expressions are in the concurrency group', () => {
  const lines = code.split('\n').filter((line) => line.includes('${{'));
  assert.ok(lines.length >= 1);
  for (const line of lines) {
    assert.match(line, /^\s+(group|cancel-in-progress): /, `an expression outside the concurrency block: ${line.trim()}`);
  }
});

test('the workflow reads the repository only: it triggers on pull requests and pushes to main, and by hand', () => {
  assert.match(code, /^on:\n {2}pull_request:\n {4}branches:\n {6}- main\n/m);
  assert.match(code, /^ {2}push:\n {4}branches:\n {6}- main\n/m);
  assert.match(code, /^ {2}workflow_dispatch:\n/m);
  assert.ok(!/schedule:/.test(code));
});
