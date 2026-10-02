'use strict';

// Tests of the pure half of the second verification (lib/verify.cjs): the rules
// that say what is wrong with the facts read from the registry and GitHub, and
// the place the stable extension is registered (not yet: see STABLE_CHECKS).

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');

const { PACKAGES } = require('../lib/common.cjs');
const { expectedAssets } = require('../lib/release.cjs');
const verify = require('../lib/verify.cjs');

const COMMIT = '011bc4da1af62a837d7ac5f319fc0ee55c9cb2aa';

function integrity(name) {
  return `sha512-${crypto.createHash('sha512').update(name).digest('base64')}`;
}

function provenanceDocument(name, version, overrides = {}) {
  const statement = {
    _type: 'https://in-toto.io/Statement/v1',
    subject: [{ name: verify.packageUrl(name, version), digest: { sha512: verify.integrityToHex(integrity(name)) } }],
    predicateType: 'https://slsa.dev/provenance/v1',
    predicate: {
      buildDefinition: {
        externalParameters: {
          workflow: { ref: `refs/tags/v${version}`, repository: 'https://github.com/smormah/vsift', path: '.github/workflows/release.yml' },
        },
        internalParameters: { github: { event_name: 'workflow_dispatch' } },
        resolvedDependencies: [{ uri: `git+https://github.com/smormah/vsift@refs/tags/v${version}`, digest: { gitCommit: COMMIT } }],
      },
      runDetails: {
        builder: { id: 'https://github.com/actions/runner/github-hosted' },
        metadata: { invocationId: 'https://github.com/smormah/vsift/actions/runs/36931487439/attempts/1' },
      },
    },
  };
  const merged = JSON.parse(JSON.stringify(statement));
  for (const [path, value] of Object.entries(overrides)) {
    const keys = path.split('.');
    let target = merged;
    for (const key of keys.slice(0, -1)) {
      target = target[key];
    }
    target[keys[keys.length - 1]] = value;
  }
  const publish = { predicateType: 'https://github.com/npm/attestation/tree/main/specs/publish/v0.1', bundle: { dsseEnvelope: { payload: Buffer.from('{}').toString('base64') } } };
  const provenance = {
    predicateType: 'https://slsa.dev/provenance/v1',
    bundle: { dsseEnvelope: { payload: Buffer.from(JSON.stringify(merged)).toString('base64') } },
  };
  return { attestations: [publish, provenance] };
}

function goodFacts(version = '0.1.0', overrides = {}) {
  const packages = {};
  for (const name of PACKAGES) {
    packages[name] = {
      distTags: { latest: '0.0.0', next: version },
      versions: ['0.0.0', version],
      manifest: { version, dist: { integrity: integrity(name) } },
      provenance: verify.parseProvenance(provenanceDocument(name, version)),
    };
  }
  return {
    version,
    tagCommit: COMMIT,
    packages,
    release: {
      tagName: `v${version}`,
      isDraft: false,
      isPrerelease: true,
      assets: expectedAssets(version).map((name) => ({ name, size: 10, digest: `sha256:${'a'.repeat(64)}` })),
    },
    latestRelease: null,
    ...overrides,
  };
}

const failures = (results) => results.filter((result) => !result.ok);

test('the facts of the published 0.1.0 pass', () => {
  const results = verify.evaluateRelease(goodFacts());
  assert.deepEqual(failures(results), []);
  assert.equal(results.length, 1 + 3 * PACKAGES.length + 2);
});

test('a kind of publish is read from the version and the release flag, and a contradiction is an error', () => {
  assert.equal(verify.channelOf('0.1.0', { isPrerelease: true }), 'pre-release');
  assert.equal(verify.channelOf('0.2.0-rc.1', { isPrerelease: true }), 'pre-release');
  assert.equal(verify.channelOf('0.2.0', { isPrerelease: false }), 'stable');
  assert.equal(verify.channelOf('1.0.0', { isPrerelease: false }), 'stable');
  assert.throws(() => verify.channelOf('0.2.0-rc.1', { isPrerelease: false }), /not marked as a pre-release/);
  // Since the version alone decides the channel (PR 8), a version without a suffix that is
  // marked as a pre-release is a mistake, except 0.1.0, published before that rule.
  assert.throws(() => verify.channelOf('0.2.0', { isPrerelease: true }), /so it is a stable version/);
  assert.throws(() => verify.channelOf('1.0.0', { isPrerelease: true }), /so it is a stable version/);
  const mismatch = goodFacts('0.2.0-rc.1');
  mismatch.release.isPrerelease = false;
  assert.ok(failures(verify.evaluateRelease(mismatch)).some((result) => /agree on the kind/.test(result.name)));
});

test('a pre-release that moved latest, lost next or was deprecated fails', () => {
  const moved = goodFacts();
  moved.packages['@vsift/linux-x64'].distTags.latest = '0.1.0';
  assert.match(failures(verify.evaluateRelease(moved))[0].detail, /latest moved/);

  const lostNext = goodFacts();
  lostNext.packages['vsift-cli'].distTags.next = '0.0.0';
  assert.match(failures(verify.evaluateRelease(lostNext))[0].detail, /next is 0.0.0/);

  const deprecated = goodFacts();
  deprecated.packages['vsift-cli'].manifest.deprecated = 'do not use';
  assert.match(failures(verify.evaluateRelease(deprecated))[0].detail, /deprecated/);

  const missing = goodFacts();
  missing.packages['@vsift/win32-x64'].versions = ['0.0.0'];
  assert.match(failures(verify.evaluateRelease(missing))[0].detail, /not among the published versions/);

  const scripted = goodFacts();
  scripted.packages['vsift-cli'].manifest.scripts = { postinstall: 'node x' };
  assert.match(failures(verify.evaluateRelease(scripted))[0].detail, /declares scripts/);
});

test('provenance must name this repository, the Release workflow, the tag, its commit and a hosted builder', () => {
  const cases = {
    'predicate.buildDefinition.externalParameters.workflow.repository': ['https://github.com/someone/else', /repository/],
    'predicate.buildDefinition.externalParameters.workflow.path': ['.github/workflows/other.yml', /workflow/],
    'predicate.buildDefinition.externalParameters.workflow.ref': ['refs/heads/main', /ref/],
    'predicate.buildDefinition.internalParameters.github.event_name': ['push', /event/],
    'predicate.runDetails.builder.id': ['https://example.test/self-hosted', /builder/],
    'subject': [[{ name: 'pkg:npm/other@0.1.0', digest: { sha512: '00' } }], /subject/],
  };
  for (const [path, [value, pattern]] of Object.entries(cases)) {
    const facts = goodFacts();
    facts.packages['@vsift/darwin-arm64'].provenance = verify.parseProvenance(provenanceDocument('@vsift/darwin-arm64', '0.1.0', { [path]: value }));
    const found = failures(verify.evaluateRelease(facts));
    assert.equal(found.length, 1, path);
    assert.match(found[0].detail, pattern, path);
  }
  const otherCommit = goodFacts();
  otherCommit.tagCommit = 'f'.repeat(40);
  assert.equal(failures(verify.evaluateRelease(otherCommit)).length, PACKAGES.length, 'every package built from a commit the tag does not name');
  const noProvenance = goodFacts();
  noProvenance.packages['vsift-cli'].provenance = undefined;
  noProvenance.packages['vsift-cli'].provenanceError = 'HTTP 404';
  assert.match(failures(verify.evaluateRelease(noProvenance))[0].detail, /HTTP 404/);
  assert.throws(() => verify.parseProvenance({ attestations: [] }), /no SLSA provenance/);
});

test('the scoped package URL percent-encodes the scope', () => {
  assert.equal(verify.packageUrl('vsift-cli', '0.1.0'), 'pkg:npm/vsift-cli@0.1.0');
  assert.equal(verify.packageUrl('@vsift/win32-x64', '0.1.0'), 'pkg:npm/%40vsift/win32-x64@0.1.0');
});

test('the release must not be a draft, must not be the latest release when it is a pre-release, and must hold ten named files', () => {
  const draft = goodFacts();
  draft.release.isDraft = true;
  assert.match(failures(verify.evaluateRelease(draft))[0].detail, /draft/);

  const latest = goodFacts();
  latest.latestRelease = 'v0.1.0';
  assert.match(failures(verify.evaluateRelease(latest))[0].detail, /latest release/);

  const nine = goodFacts();
  nine.release.assets.pop();
  assert.match(failures(verify.evaluateRelease(nine))[0].detail, /holds 9 files/);

  const renamed = goodFacts();
  renamed.release.assets[0].name = 'surprise.bin';
  assert.match(failures(verify.evaluateRelease(renamed))[0].detail, /surprise.bin/);

  const undigested = goodFacts();
  undigested.release.assets[1].digest = null;
  assert.match(failures(verify.evaluateRelease(undigested))[0].detail, /no GitHub digest/);

  const wrongTag = goodFacts();
  wrongTag.release.tagName = 'v0.0.9';
  assert.match(failures(verify.evaluateRelease(wrongTag))[0].detail, /v0.0.9/);
});

test('integrity strings are converted to hex and anything else is refused', () => {
  assert.equal(verify.integrityToHex(`sha512-${Buffer.from('abc').toString('base64')}`), '616263');
  for (const bad of ['sha1-abc', 'sha512-', '', undefined, 'sha512-@@@']) {
    assert.throws(() => verify.integrityToHex(bad), /integrity/);
  }
});

test('a pre-release needs no stable check, and a stable version fails by name until its two are registered', () => {
  assert.deepEqual(verify.runStableChecks('pre-release', {}), []);
  assert.deepEqual([...verify.REQUIRED_STABLE_CHECKS], ['candidate-to-stable-delta', 'latest-on-all-four-packages']);
  assert.equal(verify.STABLE_CHECKS.size, 0, 'none is registered yet: they are added before the stable publish');
  const unregistered = verify.runStableChecks('stable', {});
  assert.equal(unregistered.length, 2);
  for (const result of unregistered) {
    assert.equal(result.ok, false);
    assert.match(result.detail, /not registered yet: it goes into STABLE_CHECKS/);
  }
  assert.match(verify.runStableChecks('stable', { releaseRunId: '123' })[0].detail, /the Release run is 123/);
});

test('the Release run is read from the provenance of the four packages, and any disagreement gives none', () => {
  const withRun = (url) => {
    const packages = {};
    for (const name of PACKAGES) {
      packages[name] = { provenance: { runUrl: url } };
    }
    return packages;
  };
  const run = 'https://github.com/smormah/vsift/actions/runs/36959682491';
  assert.equal(verify.releaseRunId(withRun(`${run}/attempts/1`)), '36959682491');
  assert.equal(verify.releaseRunId(withRun(run)), '36959682491');
  assert.equal(verify.releaseRunId(withRun('https://github.com/someone/else/actions/runs/36959682491/attempts/1')), null);
  assert.equal(verify.releaseRunId(withRun(`${run}/attempts/x`)), null);
  assert.equal(verify.releaseRunId(withRun('')), null);
  const split = withRun(`${run}/attempts/1`);
  split['vsift-cli'] = { provenance: { runUrl: 'https://github.com/smormah/vsift/actions/runs/1/attempts/1' } };
  assert.equal(verify.releaseRunId(split), null, 'the packages of one publish are built by one run');
  const unread = withRun(`${run}/attempts/1`);
  unread['vsift-cli'] = { provenanceError: 'HTTP 404' };
  assert.equal(verify.releaseRunId(unread), null);
  assert.equal(verify.releaseRunId({}), null);
});

test('registered stable checks run with the context, and one that throws fails the verification', () => {
  const registry = new Map([
    ['candidate-to-stable-delta', (context) => `delta of ${context.version} allowed`],
    ['latest-on-all-four-packages', () => {
      throw new (require('../lib/common.cjs').QualificationError)('latest is 0.0.0 on @vsift/linux-x64');
    }],
  ]);
  const results = verify.runStableChecks('stable', { version: '0.2.0' }, registry);
  assert.deepEqual(results.map((result) => [result.name, result.ok]), [
    ['stable: candidate-to-stable-delta', true],
    ['stable: latest-on-all-four-packages', false],
  ]);
  assert.equal(results[0].detail, 'delta of 0.2.0 allowed');
  assert.match(results[1].detail, /latest is 0.0.0/);
});

test('a stable version gets no pre-release dist-tag rule: that is the stable extension\'s', () => {
  const facts = goodFacts('0.2.0');
  facts.release.isPrerelease = false;
  for (const name of PACKAGES) {
    facts.packages[name].distTags = { latest: '0.2.0' };
  }
  const results = verify.evaluateRelease(facts);
  assert.deepEqual(failures(results), []);
  assert.ok(results.some((result) => /read by the stable checks/.test(result.detail)));
});
