'use strict';

// Tests of the pure half of the second verification (lib/verify.cjs): the rules
// that say what is wrong with the facts read from the registry and GitHub, and
// the stable extension (the two checks registered in STABLE_CHECKS, tried here
// against the values of the real 0.2.0 publish and against what each rule
// refuses). Nothing here touches the network: what the checks read from GitHub
// is a fake given in `context.readers`.

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const { PACKAGES, QualificationError } = require('../lib/common.cjs');
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

test('the two stable checks are registered, and a stable version still fails by name for any that is not', () => {
  assert.deepEqual([...verify.REQUIRED_STABLE_CHECKS], ['candidate-to-stable-delta', 'latest-on-all-four-packages']);
  assert.deepEqual([...verify.STABLE_CHECKS.keys()], [...verify.REQUIRED_STABLE_CHECKS], 'every required check is registered, and nothing else is');
  assert.equal(verify.STABLE_CHECKS.get('candidate-to-stable-delta'), verify.candidateToStableDelta);
  assert.equal(verify.STABLE_CHECKS.get('latest-on-all-four-packages'), verify.latestOnAllFourPackages);
  // The guard that makes a registry emptied by mistake fail loudly instead of passing without the checks.
  const unregistered = verify.runStableChecks('stable', {}, new Map());
  assert.equal(unregistered.length, 2);
  for (const result of unregistered) {
    assert.equal(result.ok, false);
    assert.match(result.detail, /not registered: STABLE_CHECKS \(tools\/p14-published\/lib\/verify\.cjs\) has no check of this name/);
  }
  assert.match(verify.runStableChecks('stable', { releaseRunId: '123' }, new Map())[0].detail, /the Release run is 123/);
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
      throw new QualificationError('latest is 0.0.0 on @vsift/linux-x64');
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

// ---------------------------------------------------------------- the stable checks, on the values of the real 0.2.0 publish

const repository = path.join(__dirname, '..', '..', '..');

/** `release-delta.json` exactly as the Release run 37946261087, the publish of 0.2.0 on 2026-10-09, wrote it. */
const REAL_DELTA_TEXT =
  '{"candidate_commit":"83dca856e7a00fc9a71c87baae99f0b1d401dd31","candidate_version":"0.2.0-rc.3",' +
  '"check":{"run_id":37946261087,"type":"workflow_run","workflow":"Release"},"date":"2026-10-09",' +
  '"stable_commit":"eeb2a22a46a85ab10a456f2ab5d6a62e292836c5","stable_version":"0.2.0","verdict":"allowed"}';
const STABLE_VERSION = '0.2.0';
const STABLE_COMMIT = 'eeb2a22a46a85ab10a456f2ab5d6a62e292836c5';
const CANDIDATE_COMMIT = '83dca856e7a00fc9a71c87baae99f0b1d401dd31';
const PUBLISH_RUN = '37946261087';

/** The facts the registry and GitHub gave for 0.2.0: `latest` 0.2.0 and `next` 0.2.0-rc.3 on all four packages, built by the publish run from the tag's commit. */
function stableFacts() {
  const facts = goodFacts(STABLE_VERSION);
  for (const name of PACKAGES) {
    const entry = facts.packages[name];
    entry.distTags = { latest: STABLE_VERSION, next: '0.2.0-rc.3' };
    entry.versions = ['0.0.0', '0.1.0', '0.2.0-rc.1', '0.2.0-rc.2', '0.2.0-rc.3', STABLE_VERSION];
    entry.provenance = verify.parseProvenance(
      provenanceDocument(name, STABLE_VERSION, {
        'predicate.buildDefinition.resolvedDependencies': [
          { uri: 'git+https://github.com/smormah/vsift@refs/tags/v0.2.0', digest: { gitCommit: STABLE_COMMIT } },
        ],
        'predicate.runDetails.metadata.invocationId': `https://github.com/smormah/vsift/actions/runs/${PUBLISH_RUN}/attempts/1`,
      }),
    );
  }
  facts.tagCommit = STABLE_COMMIT;
  facts.release.isPrerelease = false;
  facts.latestRelease = 'v0.2.0';
  return facts;
}

/** What the checks read from GitHub, as fakes: the record, the candidate tags (with noise a strict reader must ignore) and the tags' commits. */
function fakeReaders(overrides = {}) {
  return {
    releaseDelta: () => ({ state: 'found', text: REAL_DELTA_TEXT }),
    tagsWithPrefix: (prefix) => {
      assert.equal(prefix, 'v0.2.0-rc.');
      return ['v0.2.0-rc.2', 'v0.2.0-rc.3', 'v0.2.0-rc.1', 'v0.2.0-rc.03', 'v0.2.0-rc.4-extra', 'v0.2.0-rc.0', 'v0.2.0-rc.'];
    },
    commitOfTag: (tag) => {
      assert.equal(tag, 'v0.2.0-rc.3', 'only the highest candidate tag is resolved');
      return CANDIDATE_COMMIT;
    },
    ...overrides,
  };
}

function stableContext(overrides = {}) {
  const facts = overrides.facts || stableFacts();
  return { version: STABLE_VERSION, releaseRunId: verify.releaseRunId(facts.packages), readers: fakeReaders(), ...overrides, facts };
}

/** Readers whose record is the real one with `changes` applied. */
function withDelta(changes) {
  return fakeReaders({ releaseDelta: () => ({ state: 'found', text: JSON.stringify({ ...JSON.parse(REAL_DELTA_TEXT), ...changes }) }) });
}

/** What a check says when it refuses: its message, once it is known to be a refusal and not a bug. */
function refusal(check, context) {
  try {
    check(context);
  } catch (error) {
    assert.ok(error instanceof QualificationError, `a bug is not a refusal: ${error && error.stack}`);
    return error.message;
  }
  return assert.fail('the check passed, and should have failed');
}

test('the values of the real 0.2.0 publish pass the whole verification, both stable checks included', () => {
  const context = stableContext();
  assert.equal(context.releaseRunId, PUBLISH_RUN, 'the provenance of all four packages names the publish run');
  assert.deepEqual(failures(verify.evaluateRelease(context.facts)), []);
  assert.equal(verify.channelOf(STABLE_VERSION, context.facts.release), 'stable');
  const results = verify.runStableChecks('stable', context);
  assert.deepEqual(
    results.map((result) => [result.name, result.ok]),
    [
      ['stable: candidate-to-stable-delta', true],
      ['stable: latest-on-all-four-packages', true],
    ],
  );
  assert.equal(
    results[0].detail,
    'release-delta.json of Release run 37946261087 (2026-10-09): 0.2.0-rc.3 at 83dca856e7a0, the highest candidate tag, to 0.2.0 at eeb2a22a46a8, ' +
      "the commit of v0.2.0 and the one npm's provenance names on all four packages; verdict allowed",
  );
  assert.equal(
    results[1].detail,
    "latest is 0.2.0 on all four packages (next is 0.2.0-rc.3); the GitHub release v0.2.0 is not a draft, not a pre-release and is GitHub's latest release",
  );
});

test("the delta check refuses a record that is not this release's, and says which value is wrong", () => {
  const cases = [
    ['another stable version', withDelta({ stable_version: '0.2.1' }), /stable_version is 0\.2\.1, not 0\.2\.0/],
    ['a stable commit the tag does not name', withDelta({ stable_commit: 'f'.repeat(40) }), /stable_commit ffffffffffff is not the commit of v0\.2\.0, eeb2a22a46a8/],
    [
      'a stable commit npm did not build',
      withDelta({ stable_commit: 'f'.repeat(40) }),
      /npm's provenance does not name stable_commit ffffffffffff for vsift-cli, @vsift\/win32-x64, @vsift\/darwin-arm64, @vsift\/linux-x64/,
    ],
    ['a candidate of another version', withDelta({ candidate_version: '0.3.0-rc.3' }), /candidate_version is 0\.3\.0-rc\.3, but the highest candidate tag is v0\.2\.0-rc\.3/],
    ['a candidate that is not a candidate', withDelta({ candidate_version: '0.2.0' }), /candidate_version is 0\.2\.0, but the highest candidate tag is v0\.2\.0-rc\.3/],
    ['an earlier candidate than the highest', withDelta({ candidate_version: '0.2.0-rc.2' }), /candidate_version is 0\.2\.0-rc\.2, but the highest candidate tag is v0\.2\.0-rc\.3/],
    ['a candidate commit the tag does not name', withDelta({ candidate_commit: 'a'.repeat(40) }), /candidate_commit aaaaaaaaaaaa is not the commit of v0\.2\.0-rc\.3, 83dca856e7a0/],
    ['a candidate that is the stable commit', withDelta({ candidate_commit: STABLE_COMMIT }), /the candidate and the stable release are the same commit/],
    ['a rejected verdict', withDelta({ verdict: 'rejected' }), /verdict is "rejected", not "allowed"/],
    ['a verdict that is not one', withDelta({ verdict: 'Allowed' }), /verdict is "Allowed", not "allowed"/],
    [
      'the record of another run',
      withDelta({ check: { type: 'workflow_run', workflow: 'Release', run_id: 36959682491 } }),
      /check\.run_id is 36959682491, but npm's provenance names Release run 37946261087/,
    ],
    ['a record of another workflow', withDelta({ check: { type: 'workflow_run', workflow: 'P14 verify release', run_id: 37946261087 } }), /not a workflow_run of Release/],
    ['a record of another kind of check', withDelta({ check: { type: 'manual', workflow: 'Release', run_id: 37946261087 } }), /"manual" of "Release", not a workflow_run of Release/],
    ['a date that is not a date', withDelta({ date: 'yesterday' }), /date "yesterday" is not YYYY-MM-DD/],
  ];
  for (const [what, readers, pattern] of cases) {
    assert.match(refusal(verify.candidateToStableDelta, stableContext({ readers })), pattern, what);
  }
});

test("the delta check compares the record with the tag, the release, npm's provenance and the highest candidate tag", () => {
  const otherTagCommit = stableFacts();
  otherTagCommit.tagCommit = 'c'.repeat(40);
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ facts: otherTagCommit })), /stable_commit eeb2a22a46a8 is not the commit of v0\.2\.0, cccccccccccc/);

  const otherProvenance = stableFacts();
  otherProvenance.packages['@vsift/linux-x64'].provenance.commit = 'b'.repeat(40);
  const message = refusal(verify.candidateToStableDelta, stableContext({ facts: otherProvenance }));
  assert.match(message, /does not name stable_commit eeb2a22a46a8 for @vsift\/linux-x64$/, 'names the one package, and nothing else is wrong');

  // A package whose provenance could not be read leaves no single run to name (the run is read from all four); given the run
  // by hand, the check still does not accept a package that names no commit.
  const unread = stableFacts();
  unread.packages['vsift-cli'].provenance = undefined;
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ facts: unread })), /does not name one Release run/);
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ facts: unread, releaseRunId: PUBLISH_RUN })), /for vsift-cli$/);

  const otherRelease = stableFacts();
  otherRelease.release.tagName = 'v0.1.0';
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ facts: otherRelease })), /the GitHub release is for v0\.1\.0, not v0\.2\.0/);

  const laterCandidate = fakeReaders({
    tagsWithPrefix: () => ['v0.2.0-rc.1', 'v0.2.0-rc.3', 'v0.2.0-rc.4'],
    commitOfTag: (tag) => (tag === 'v0.2.0-rc.4' ? 'd'.repeat(40) : CANDIDATE_COMMIT),
  });
  const later = refusal(verify.candidateToStableDelta, stableContext({ readers: laterCandidate }));
  assert.match(later, /candidate_version is 0\.2\.0-rc\.3, but the highest candidate tag is v0\.2\.0-rc\.4/);
  assert.match(later, /candidate_commit 83dca856e7a0 is not the commit of v0\.2\.0-rc\.4, dddddddddddd/);

  // Only a tag of the stable version's own X.Y.Z with a plain positive number is a candidate.
  for (const tags of [[], ['v0.2.0-rc.0', 'v0.2.0-rc.03', 'v0.2.1-rc.1', 'v0.2.0-rc.1-extra', 'v0.1.0']]) {
    const readers = fakeReaders({ tagsWithPrefix: () => tags });
    assert.match(refusal(verify.candidateToStableDelta, stableContext({ readers })), /no candidate tag v0\.2\.0-rc\.<N> exists/);
  }

  assert.match(refusal(verify.candidateToStableDelta, stableContext({ releaseRunId: null })), /does not name one Release run for all four packages/);
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ readers: undefined })), /no reader for GitHub/);
});

test('several wrong values are all named in one failure', () => {
  const readers = withDelta({ stable_version: '0.2.1', verdict: 'rejected', date: '' });
  const message = refusal(verify.candidateToStableDelta, stableContext({ readers }));
  assert.match(message, /^release-delta\.json of Release run 37946261087 is not this release's: /);
  assert.match(message, /stable_version is 0\.2\.1, not 0\.2\.0; .*verdict is "rejected".*; date "" is not YYYY-MM-DD$/);
});

test("a record that is not the plan job's shape is refused, however it is wrong", () => {
  const real = JSON.parse(REAL_DELTA_TEXT);
  const without = (field) => {
    const copy = { ...real };
    delete copy[field];
    return JSON.stringify(copy);
  };
  const cases = [
    ['{', /release-delta\.json is not JSON/],
    ['', /release-delta\.json is not JSON/],
    ['[]', /not an object with a check object/],
    ['null', /not an object with a check object/],
    [without('check'), /not an object with a check object/],
    [JSON.stringify({ ...real, check: 'Release' }), /not an object with a check object/],
    [without('verdict'), /no text field verdict/],
    [without('stable_commit'), /no text field stable_commit/],
    [JSON.stringify({ ...real, candidate_version: 3 }), /no text field candidate_version/],
    [JSON.stringify({ ...real, check: { type: 'workflow_run', workflow: 'Release' } }), /no positive whole number check\.run_id/],
    [JSON.stringify({ ...real, check: { type: 'workflow_run', workflow: 'Release', run_id: '37946261087' } }), /no positive whole number check\.run_id/],
    [JSON.stringify({ ...real, check: { type: 'workflow_run', workflow: 'Release', run_id: 0 } }), /no positive whole number check\.run_id/],
    [JSON.stringify({ ...real, check: { type: 'workflow_run', workflow: 3, run_id: 1 } }), /no text field check\.workflow/],
  ];
  for (const [text, pattern] of cases) {
    const readers = fakeReaders({ releaseDelta: () => ({ state: 'found', text }) });
    assert.match(refusal(verify.candidateToStableDelta, stableContext({ readers })), pattern, JSON.stringify(text));
  }
  assert.equal(verify.parseReleaseDelta(REAL_DELTA_TEXT).check.run_id, 37946261087);
});

test("the shape the plan job's own test holds its record to is the shape this tool reads", () => {
  const example = fs.readFileSync(path.join(repository, 'tools', 'vsift-release', 'tests', 'release-delta.example.json'), 'utf8');
  const record = verify.parseReleaseDelta(example);
  assert.deepEqual(Object.keys(record).sort(), Object.keys(JSON.parse(REAL_DELTA_TEXT)).sort(), 'the same fields as the record the publish wrote');
});

test('an expired publish-plan artifact fails the delta check and says where the record was copied', () => {
  const expired = fakeReaders({ releaseDelta: () => ({ state: 'expired' }) });
  const message = refusal(verify.candidateToStableDelta, stableContext({ readers: expired }));
  assert.match(message, /release-delta\.json cannot be read: the publish-plan artifact of Release run 37946261087 has expired \(GitHub keeps a run's files 7 days\)/);
  assert.match(message, /docs\/planning\/p14-evidence-ledger\.json as release_delta \(P14 PR 13\)/);
  assert.match(message, /node -p "require\('\.\/docs\/planning\/p14-evidence-ledger\.json'\)\.release_delta" and compare it by hand$/);
  assert.ok(message.length < 400, `the job summary cuts a detail at 400 characters, and this one is ${message.length}`);
  // The ledger the message points at is there and has the field it names, and the command it gives reads it from the repository's root.
  assert.ok('release_delta' in require(path.join(repository, 'docs', 'planning', 'p14-evidence-ledger.json')), 'the ledger has a top-level release_delta');
  // In the verification it is a failed result, never a pass and never a skip, and the other stable check is unaffected.
  const results = verify.runStableChecks('stable', stableContext({ readers: expired }));
  assert.deepEqual(
    results.map((result) => [result.name, result.ok]),
    [
      ['stable: candidate-to-stable-delta', false],
      ['stable: latest-on-all-four-packages', true],
    ],
  );
  assert.match(results[0].detail, /has expired/);
});

test('a run with no publish-plan artifact, or one without the record, fails the delta check', () => {
  const noArtifact = fakeReaders({ releaseDelta: () => ({ state: 'no-artifact' }) });
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ readers: noArtifact })), /Release run 37946261087 has no artifact named publish-plan/);
  const noRecord = fakeReaders({ releaseDelta: () => ({ state: 'no-record' }) });
  assert.match(
    refusal(verify.candidateToStableDelta, stableContext({ readers: noRecord })),
    /holds no release-delta\.json: only an enforced stable plan writes one, so this run did not publish a stable version/,
  );
  const odd = fakeReaders({ releaseDelta: () => ({ state: 'maybe' }) });
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ readers: odd })), /the reader answered "maybe"/);
  const unreadable = fakeReaders({
    releaseDelta: () => {
      throw new QualificationError('gh api repos/smormah/vsift/actions/runs/37946261087/artifacts exited 1: HTTP 403');
    },
  });
  assert.match(refusal(verify.candidateToStableDelta, stableContext({ readers: unreadable })), /HTTP 403/);
});

test('latest must name the version on all four packages, and the GitHub release must be a published, latest release', () => {
  for (const name of PACKAGES) {
    for (const [latest, shown] of [
      ['0.0.0', '0.0.0'],
      ['0.2.0-rc.3', '0.2.0-rc.3'],
      [undefined, 'not set'],
    ]) {
      const facts = stableFacts();
      facts.packages[name].distTags = latest === undefined ? { next: '0.2.0-rc.3' } : { latest, next: '0.2.0-rc.3' };
      assert.equal(refusal(verify.latestOnAllFourPackages, stableContext({ facts })), `latest is ${shown} on ${name}, not 0.2.0`);
    }
  }

  const unlisted = stableFacts();
  unlisted.packages['@vsift/darwin-arm64'].versions = ['0.0.0', '0.2.0-rc.3'];
  assert.equal(refusal(verify.latestOnAllFourPackages, stableContext({ facts: unlisted })), '@vsift/darwin-arm64 does not list 0.2.0 among its versions');

  const cases = [
    ['a pre-release', (facts) => { facts.release.isPrerelease = true; }, /^the GitHub release is marked as a pre-release$/],
    ['a draft', (facts) => { facts.release.isDraft = true; }, /^the GitHub release is a draft$/],
    ['no latest release', (facts) => { facts.latestRelease = null; }, /^GitHub's latest release is none, not v0\.2\.0$/],
    ['another latest release', (facts) => { facts.latestRelease = 'v0.2.0-rc.3'; }, /^GitHub's latest release is v0\.2\.0-rc\.3, not v0\.2\.0$/],
    ['another tag', (facts) => { facts.release.tagName = 'v0.1.0'; }, /^the GitHub release is for v0\.1\.0, not v0\.2\.0$/],
  ];
  for (const [what, change, pattern] of cases) {
    const facts = stableFacts();
    change(facts);
    assert.match(refusal(verify.latestOnAllFourPackages, stableContext({ facts })), pattern, what);
  }

  const many = stableFacts();
  many.packages['vsift-cli'].distTags.latest = '0.0.0';
  many.packages['@vsift/win32-x64'].distTags.latest = '0.0.0';
  many.release.isDraft = true;
  many.latestRelease = null;
  assert.equal(
    refusal(verify.latestOnAllFourPackages, stableContext({ facts: many })),
    "latest is 0.0.0 on vsift-cli, not 0.2.0; latest is 0.0.0 on @vsift/win32-x64, not 0.2.0; the GitHub release is a draft; GitHub's latest release is none, not v0.2.0",
  );

  const unread = stableFacts();
  delete unread.packages['@vsift/linux-x64'];
  assert.match(refusal(verify.latestOnAllFourPackages, stableContext({ facts: unread })), /^@vsift\/linux-x64 was not read$/);
});

test('latest needs nothing of `next`, and a missing `next` is only noted', () => {
  const facts = stableFacts();
  for (const name of PACKAGES) {
    facts.packages[name].distTags = { latest: STABLE_VERSION };
  }
  assert.match(verify.latestOnAllFourPackages(stableContext({ facts })), /\(next is unset\)/);
});

test('a pre-release is not asked for the stable checks, and neither check passes for one if it is called', () => {
  const version = '0.2.0-rc.3';
  const facts = goodFacts(version);
  const never = () => {
    throw new Error('a pre-release must not read anything for a stable check');
  };
  const context = { version, facts, releaseRunId: '1', readers: { releaseDelta: never, tagsWithPrefix: never, commitOfTag: never } };
  assert.equal(verify.channelOf(version, facts.release), 'pre-release');
  assert.deepEqual(verify.runStableChecks(verify.channelOf(version, facts.release), context), []);
  for (const check of verify.STABLE_CHECKS.values()) {
    assert.match(refusal(check, context), /^0\.2\.0-rc\.3 is a pre-release: this check is for a stable version$/);
  }
  // The same for 0.1.0, the one version without a suffix that was published as a pre-release.
  assert.equal(verify.channelOf('0.1.0', { isPrerelease: true }), 'pre-release');
  assert.deepEqual(verify.runStableChecks('pre-release', context), []);
});
