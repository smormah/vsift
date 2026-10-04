'use strict';

// Guards of the P14 published-artifact workflows and tools: their tool pins are
// the Release workflow's, and nothing in them can publish.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const repository = path.join(__dirname, '..', '..', '..');
const read = (relative) => fs.readFileSync(path.join(repository, relative), 'utf8');
const workflowFiles = fs
  .readdirSync(path.join(repository, '.github', 'workflows'))
  .filter((name) => /^p14-.*\.yml$/.test(name))
  .map((name) => `.github/workflows/${name}`);

/** The top-level `env:` entries of a workflow, as name to value. */
function topLevelEnv(text) {
  const section = /^env:\n((?:[ ]{2}.*\n|\n)+)/m.exec(text);
  assert.ok(section, 'the workflow has a top-level env');
  const entries = {};
  for (const line of section[1].split('\n')) {
    const match = /^[ ]{2}([A-Z_]+):\s*"?([^"#]*?)"?\s*(?:#.*)?$/.exec(line);
    if (match) {
      entries[match[1]] = match[2];
    }
  }
  return entries;
}

/** Every `uses:` action with its pinned commit. */
function actions(text) {
  return new Map([...text.matchAll(/uses:\s+([\w.-]+\/[\w./-]+)@([0-9a-f]{40})\s+#\s*(\S+)/g)].map((match) => [match[1], `${match[2]} ${match[3]}`]));
}

test('there are four P14 published-artifact workflows', () => {
  assert.deepEqual(
    workflowFiles.filter((file) => /p14-(published-artifacts|local-upgrade|verify-release|compatibility)\.yml$/.test(file)).sort(),
    [
      '.github/workflows/p14-compatibility.yml',
      '.github/workflows/p14-local-upgrade.yml',
      '.github/workflows/p14-published-artifacts.yml',
      '.github/workflows/p14-verify-release.yml',
    ],
  );
});

test('the tool pins are the P13 npm qualification\'s, in the Release workflow', () => {
  const release = topLevelEnv(read('.github/workflows/release.yml'));
  for (const [file, names] of [
    ['.github/workflows/p14-published-artifacts.yml', ['NODE_VERSION', 'BUN_VERSION', 'PNPM_VERSION', 'YARN_VERSION']],
    ['.github/workflows/p14-local-upgrade.yml', ['NODE_VERSION', 'VERDACCIO_VERSION']],
    ['.github/workflows/p14-verify-release.yml', ['NODE_VERSION']],
  ]) {
    const own = topLevelEnv(read(file));
    for (const name of names) {
      assert.equal(own[name], release[name], `${file}: ${name} is ${own[name]}, the Release workflow's is ${release[name]}`);
    }
  }
});

test('every action is pinned to the commit the Release workflow, or else every other workflow, uses', () => {
  const release = actions(read('.github/workflows/release.yml'));
  // An action the Release workflow does not use (actions/setup-python, for the journeys driver) must
  // be pinned to the one commit that every other workflow of the repository uses for it.
  const elsewhere = new Map();
  for (const name of fs.readdirSync(path.join(repository, '.github', 'workflows'))) {
    if (!/\.yml$/.test(name) || name === 'release.yml' || /^p14-/.test(name)) {
      continue;
    }
    for (const [action, pin] of actions(read(`.github/workflows/${name}`))) {
      elsewhere.set(action, new Set([...(elsewhere.get(action) ?? []), pin]));
    }
  }
  for (const file of workflowFiles) {
    for (const [action, pin] of actions(read(file))) {
      if (release.has(action)) {
        assert.equal(pin, release.get(action), `${file}: ${action}`);
        continue;
      }
      const others = elsewhere.get(action);
      assert.ok(others, `${file} uses ${action}, which neither the Release workflow nor any other workflow does`);
      assert.deepEqual([...others], [pin], `${file}: ${action} is pinned to ${pin}, the other workflows' pin is ${[...others].join(' or ')}`);
    }
  }
});

test('the Ubuntu image of the offline job is the one ci.yml already pins', () => {
  const image = /ubuntu@sha256:[0-9a-f]{64}/.exec(read('.github/workflows/ci.yml'));
  assert.ok(image);
  assert.equal(topLevelEnv(read('.github/workflows/p14-published-artifacts.yml')).UBUNTU_IMAGE, image[0]);
});

test('no workflow can publish, tag, log in or write: read scopes only, no secret, no OIDC token', () => {
  for (const file of workflowFiles) {
    const text = read(file);
    const code = text.replace(/^\s*#.*$/gm, '');
    assert.ok(!/id-token/.test(code), `${file} names id-token`);
    assert.ok(!/secrets\./.test(code), `${file} names a secret`);
    assert.ok(!/:\s*write\b/.test(code), `${file} grants a write scope`);
    assert.ok(!/environment:/.test(code), `${file} names an environment`);
    assert.ok(!/npm (publish|dist-tag|login|adduser|unpublish|deprecate|owner|access|token)/.test(code), `${file} runs a publishing or account npm command`);
    assert.ok(!/gh (release (create|edit|delete|upload)|workflow run|api\b.*(-X|--method))/.test(code), `${file} runs a writing gh command`);
    assert.ok(!/git (tag|push)/.test(code), `${file} tags or pushes`);
    assert.ok(/^permissions: \{\}$/m.test(code), `${file} has no empty top-level permissions`);
    assert.ok(!/pull_request_target/.test(code), `${file} uses pull_request_target`);
  }
});

test('no tool can publish: the only npm publish is the loopback registry\'s, given a loopback registry', () => {
  const tools = path.join(repository, 'tools', 'p14-published');
  const sources = [];
  const walk = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (/\.(cjs|ps1)$/.test(entry.name) && !full.includes(`${path.sep}test${path.sep}`)) {
        sources.push(full);
      }
    }
  };
  walk(tools);
  assert.ok(sources.length >= 10);
  for (const file of sources) {
    const code = fs.readFileSync(file, 'utf8').replace(/^\s*\/\/.*$/gm, '');
    const relative = path.relative(tools, file).split(path.sep).join('/');
    // `dist-tags` (reading them) is fine; the `dist-tag` command is not.
    for (const forbidden of [/dist-tag(?!s)/, /npm login/, /adduser/, /unpublish/, /deprecate(?!d)/, /--force/, /release create/, /release edit/, /release delete/, /git tag/, /git push/]) {
      assert.ok(!forbidden.test(code), `${relative} names ${forbidden}`);
    }
    if (relative !== 'lib/local-registry.cjs') {
      assert.ok(!/'publish'/.test(code), `${relative} runs npm publish`);
    }
  }
  const local = fs.readFileSync(path.join(tools, 'lib', 'local-registry.cjs'), 'utf8');
  assert.match(local, /'--registry', LOCAL_REGISTRY/, 'the local publish names the loopback registry');
  assert.match(local, /const LOCAL_REGISTRY = 'http:\/\/127\.0\.0\.1:4873\/'/);
});
