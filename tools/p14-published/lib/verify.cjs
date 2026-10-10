'use strict';

// The credential-free second verification of a publish (RQ-19; ADR 0024:
// "dist-tags and provenance of all four packages, `npm audit signatures`,
// `gh attestation verify` of every release file and tarball, checksums, the
// release's flags and asset count; for the stable also the candidate-to-stable
// delta ... and `latest` on all four").
//
// This module is the pure half: given the facts read from the registry and
// from GitHub, say what is wrong. verify-release.cjs gathers the facts and does
// the parts that need files and `gh` (downloads, checksums, attestations).
//
// **The stable extension.** A stable version adds two checks, registered in
// `STABLE_CHECKS`: the candidate-to-stable delta (the record the plan job wrote
// says that only version strings and documents that ship inside the artifacts
// differ, and it names this release's commits and the accepted candidate) and
// `latest` on all four packages with the GitHub release marked latest. They
// were registered **after the stable tag** (P14 PR 13a, a small pull request
// that had to land within seven days of the stable publish, while the Release
// run's `publish-plan` artifact exists): a change under `tools/` between the
// candidate and the stable commit is refused by the candidate-to-stable check
// (docs/operations/release.md 6.8), and `P14 verify release` is dispatched from
// `main` (`--ref main`), so the code that verifies a stable release is `main`'s
// at that moment, not the tag's. A stable version whose required check is not
// registered still fails by name (`runStableChecks`): a verification of a stable
// release that silently skipped them would be a green result about the wrong
// thing.
//
// What the delta check reads is P14 PR 8's record: an enforced stable plan
// writes `release-delta.json` into the `publish-plan` artifact of the Release
// run that published, which GitHub keeps for seven days. `context.releaseRunId`
// names that run (the one npm's provenance names; see `releaseRunId`), so the
// check fetches the record without guessing which run it came from. After the
// seven days the artifact is gone and the check fails by name and says where the
// record was copied (the evidence ledger's `release_delta`): an expired input is
// never a pass.
//
// The checks do no input or output themselves. What they read from GitHub
// arrives in `context.readers` (lib/release.cjs `githubStableReaders`), so the
// tests give them fakes and need no network.

const {
  PACKAGES,
  QualificationError,
  REPOSITORY,
  expect,
} = require('./common.cjs');
const {
  PLAN_ARTIFACT,
  RELEASE_DELTA_FILE,
  acceptedCandidateTag,
  candidateTagPrefix,
  expectedAssets,
  isStable,
  parseVersion,
  tagOf,
} = require('./release.cjs');

/** The checks a stable version needs: `runStableChecks` fails a stable version by name for each that is not registered. */
const REQUIRED_STABLE_CHECKS = Object.freeze(['candidate-to-stable-delta', 'latest-on-all-four-packages']);

/**
 * The registered stable checks, by name: each is `(context) => string` (the
 * detail of a pass) and throws a QualificationError when it fails. The
 * function declarations below are hoisted, so the registry can sit up here.
 */
const STABLE_CHECKS = new Map([
  ['candidate-to-stable-delta', candidateToStableDelta],
  ['latest-on-all-four-packages', latestOnAllFourPackages],
]);

/** The evidence ledger the delta record is copied into (P14 PR 13), which the expired-artifact message points at. */
const LEDGER = 'docs/planning/p14-evidence-ledger.json';

/** The placeholder `latest` every package carries until a stable release. */
const PLACEHOLDER = '0.0.0';

/** The one version without a suffix that was published as a GitHub pre-release (P13), before the version alone decided the channel. */
const FIRST_PRE_RELEASE = '0.1.0';

/**
 * Which kind of publish a version is, from the version and the GitHub release
 * (release.md sections 6.1 and 6.7: a version with a pre-release suffix is
 * published under `next` as a GitHub pre-release, and a version without one is
 * stable and moves `latest`). A version whose release flag contradicts that is
 * an error, not a pre-release by default. The one exception is 0.1.0, published
 * as a pre-release before that rule existed.
 *
 * @returns {'pre-release' | 'stable'}
 */
function channelOf(version, release) {
  const parsed = parseVersion(version);
  if (parsed.pre.length > 0) {
    expect(release.isPrerelease === true, `${version} has a pre-release part, but its GitHub release is not marked as a pre-release`);
    return 'pre-release';
  }
  if (release.isPrerelease === true) {
    expect(
      version === FIRST_PRE_RELEASE,
      `${version} has no pre-release suffix, so it is a stable version, but its GitHub release is marked as a pre-release (only ${FIRST_PRE_RELEASE} was, before that rule)`,
    );
    return 'pre-release';
  }
  return 'stable';
}

/**
 * The id of the Release run that built a version, from the provenance of its
 * packages. Every package of one publish names the same run URL
 * (`https://github.com/<repository>/actions/runs/<id>/attempts/<n>`); a
 * disagreement, a missing provenance or a URL of another shape gives `null`,
 * so a check that needs the run says so rather than fetching the wrong one.
 *
 * @param {Object<string, {provenance?: {runUrl?: string}}>} packages per package, as in evaluateRelease
 * @returns {string | null}
 */
function releaseRunId(packages) {
  const prefix = `https://github.com/${REPOSITORY}/actions/runs/`;
  const ids = Object.values(packages).map((entry) => {
    const url = (entry && entry.provenance && entry.provenance.runUrl) || '';
    const match = url.startsWith(prefix) ? /^(\d+)(?:\/attempts\/\d+)?$/.exec(url.slice(prefix.length)) : null;
    return match === null ? null : match[1];
  });
  const [first] = ids;
  return first !== undefined && first !== null && ids.every((id) => id === first) ? first : null;
}

/** The package URL npm gives a package in a provenance subject (`@` of a scope is percent-encoded). */
function packageUrl(name, version) {
  return `pkg:npm/${name.replace('@', '%40')}@${version}`;
}

/** An `sha512-<base64>` integrity string as lower-case hex. */
function integrityToHex(integrity) {
  const match = /^sha512-([A-Za-z0-9+/]+={0,2})$/.exec(integrity || '');
  if (match === null) {
    throw new QualificationError(`${JSON.stringify(integrity)} is not an sha512 integrity string`);
  }
  return Buffer.from(match[1], 'base64').toString('hex');
}

/**
 * Reads the SLSA provenance statement out of a registry attestations document
 * (`/-/npm/v1/attestations/<package>@<version>`). It does not verify a
 * signature: `npm audit signatures` and `gh attestation verify` do that.
 */
function parseProvenance(document) {
  const attestations = Array.isArray(document && document.attestations) ? document.attestations : [];
  const entry = attestations.find((candidate) => candidate.predicateType === 'https://slsa.dev/provenance/v1');
  if (entry === undefined) {
    throw new QualificationError('the registry holds no SLSA provenance attestation');
  }
  const payload = entry.bundle && entry.bundle.dsseEnvelope && entry.bundle.dsseEnvelope.payload;
  if (typeof payload !== 'string') {
    throw new QualificationError('the provenance attestation has no DSSE payload');
  }
  let statement;
  try {
    statement = JSON.parse(Buffer.from(payload, 'base64').toString('utf8'));
  } catch {
    throw new QualificationError('the provenance payload is not JSON');
  }
  const predicate = statement.predicate || {};
  const definition = predicate.buildDefinition || {};
  const workflow = (definition.externalParameters || {}).workflow || {};
  const github = (definition.internalParameters || {}).github || {};
  const dependencies = definition.resolvedDependencies || [];
  const subject = (statement.subject || [])[0] || {};
  const invocation = ((predicate.runDetails || {}).metadata || {}).invocationId || '';
  return {
    subjectName: subject.name,
    subjectSha512: (subject.digest || {}).sha512,
    workflowRepository: workflow.repository,
    workflowPath: workflow.path,
    workflowRef: workflow.ref,
    event: github.event_name,
    commit: (dependencies[0] && dependencies[0].digest && dependencies[0].digest.gitCommit) || undefined,
    builder: ((predicate.runDetails || {}).builder || {}).id,
    runUrl: invocation,
  };
}

/**
 * Evaluates the facts of one published version.
 *
 * @param {object} facts
 * @param {string} facts.version
 * @param {string} facts.tagCommit the commit the tag names
 * @param {Object<string, object>} facts.packages per package: `distTags`, `versions`, `manifest` (the registry's metadata
 *   of this version), `provenance` (see parseProvenance) or `provenanceError`
 * @param {object} facts.release `isDraft`, `isPrerelease`, `tagName`, `assets` ([{name, size, digest}])
 * @param {string|null} facts.latestRelease the tag GitHub calls the latest release, or null when there is none
 * @returns {{name: string, ok: boolean, detail: string}[]}
 */
function evaluateRelease(facts) {
  const results = [];
  const record = (name, action) => {
    try {
      results.push({ name, ok: true, detail: action() || '' });
    } catch (error) {
      results.push({ name, ok: false, detail: error instanceof QualificationError ? error.message : `${error && error.stack}` });
    }
  };
  const { version } = facts;
  let stable = false;
  record('the version and the release flag agree on the kind of publish', () => {
    const channel = channelOf(version, facts.release);
    stable = channel === 'stable';
    return channel;
  });

  for (const name of PACKAGES) {
    const entry = facts.packages[name];
    record(`${name}: published at ${version} with the dist-tags a ${stable ? 'stable' : 'pre-release'} publish leaves`, () => {
      expect(entry !== undefined, 'the registry was not read');
      expect(entry.versions.includes(version), `${version} is not among the published versions ${JSON.stringify(entry.versions)}`);
      expect(entry.manifest && entry.manifest.version === version, `the registry's metadata is for ${entry.manifest && entry.manifest.version}`);
      expect(!entry.manifest.deprecated, `${version} is deprecated: ${entry.manifest.deprecated}`);
      if (stable) {
        // What `latest` and `next` should be for a stable version is the stable
        // extension's: see STABLE_CHECKS.
        return `${version} is published; dist-tags ${JSON.stringify(entry.distTags)} are read by the stable checks`;
      }
      expect(entry.distTags.next === version, `next is ${entry.distTags.next}, not ${version}`);
      // `latest` must not be this pre-release. While no stable release exists
      // it is the maintainer's empty `${PLACEHOLDER}` placeholder (install.md
      // section 2); once one does, it is that release, so the placeholder is
      // only noted, not required.
      expect(entry.distTags.latest !== version, `latest moved to the pre-release ${version}`);
      expect(entry.versions.includes(entry.distTags.latest), `latest is ${entry.distTags.latest}, which is not a published version`);
      return `next ${entry.distTags.next}, latest ${entry.distTags.latest}${entry.distTags.latest === PLACEHOLDER ? ' (the placeholder)' : ''}`;
    });
    record(`${name}: the tarball has no install script and a recorded integrity`, () => {
      const scripts = entry.manifest.scripts;
      expect(scripts === undefined || Object.keys(scripts).length === 0, `declares scripts ${JSON.stringify(scripts)}`);
      const hex = integrityToHex(entry.manifest.dist && entry.manifest.dist.integrity);
      return `sha512 ${hex.slice(0, 16)}...`;
    });
    record(`${name}: npm provenance names this repository's Release workflow, the tag and the tag's commit`, () => {
      expect(entry.provenance, `no provenance could be read: ${entry.provenanceError || 'missing'}`);
      const provenance = entry.provenance;
      expect(provenance.subjectName === packageUrl(name, version), `the subject is ${provenance.subjectName}, not ${packageUrl(name, version)}`);
      expect(
        provenance.subjectSha512 === integrityToHex(entry.manifest.dist.integrity),
        'the provenance subject digest is not the registry tarball integrity',
      );
      expect(provenance.workflowRepository === `https://github.com/${REPOSITORY}`, `repository ${provenance.workflowRepository}`);
      expect(provenance.workflowPath === '.github/workflows/release.yml', `workflow ${provenance.workflowPath}`);
      expect(provenance.workflowRef === `refs/tags/${tagOf(version)}`, `ref ${provenance.workflowRef}`);
      expect(provenance.event === 'workflow_dispatch', `event ${provenance.event}`);
      expect(provenance.commit === facts.tagCommit, `built from ${provenance.commit}, but the tag names ${facts.tagCommit}`);
      expect(/github-hosted$/.test(provenance.builder || ''), `builder ${provenance.builder}`);
      return `${provenance.runUrl.replace('https://github.com/', '')}`;
    });
  }

  record('the release flags: not a draft, a pre-release exactly when the version is one, on the tag', () => {
    const { release } = facts;
    expect(release.tagName === tagOf(version), `the release is for ${release.tagName}`);
    expect(release.isDraft === false, 'the release is a draft');
    expect(release.isPrerelease === !stable, `isPrerelease is ${release.isPrerelease} for a ${stable ? 'stable' : 'pre-release'} version`);
    if (!stable) {
      expect(facts.latestRelease !== tagOf(version), `GitHub marks ${tagOf(version)} as the latest release`);
    }
    return `draft ${release.isDraft}, pre-release ${release.isPrerelease}, GitHub's latest release ${facts.latestRelease || 'none'}`;
  });
  record('the release holds exactly ten files, named as a release names them', () => {
    const names = facts.release.assets.map((asset) => asset.name).sort();
    const expected = expectedAssets(version);
    expect(
      JSON.stringify(names) === JSON.stringify(expected),
      `holds ${names.length} files: ${names.join(', ')}; expected ${expected.length}: ${expected.join(', ')}`,
    );
    for (const asset of facts.release.assets) {
      expect(/^sha256:[0-9a-f]{64}$/.test(asset.digest || ''), `${asset.name} has no GitHub digest`);
      expect(asset.size > 0, `${asset.name} is empty`);
    }
    return `${names.length} files`;
  });
  return results;
}

/** A commit's first twelve digits, for a message; anything that is not text is shown as it is. */
function short(commit) {
  return typeof commit === 'string' ? commit.slice(0, 12) : String(commit);
}

/** The tag of a stable version. Both stable checks are for a stable version only: a pre-release has no delta record and no `latest`. */
function stableTag(version) {
  expect(isStable(version), `${version} is a pre-release: this check is for a stable version`);
  return tagOf(version);
}

/**
 * Reads the text of a `release-delta.json` and requires the shape an enforced
 * stable plan writes (`tools/vsift-release/tests/release-delta.example.json`
 * is the example the plan's own test holds it to). It checks the shape only:
 * `candidateToStableDelta` says whether the values are this release's.
 *
 * @param {string} text the file's content
 * @returns {object} the record
 */
function parseReleaseDelta(text) {
  let record;
  try {
    record = JSON.parse(text);
  } catch {
    throw new QualificationError(`${RELEASE_DELTA_FILE} is not JSON`);
  }
  const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
  expect(isObject(record) && isObject(record.check), `${RELEASE_DELTA_FILE} is not an object with a check object`);
  for (const field of ['candidate_version', 'candidate_commit', 'stable_version', 'stable_commit', 'verdict', 'date']) {
    expect(typeof record[field] === 'string', `${RELEASE_DELTA_FILE} has no text field ${field}`);
  }
  for (const field of ['type', 'workflow']) {
    expect(typeof record.check[field] === 'string', `${RELEASE_DELTA_FILE} has no text field check.${field}`);
  }
  expect(Number.isSafeInteger(record.check.run_id) && record.check.run_id > 0, `${RELEASE_DELTA_FILE} has no positive whole number check.run_id`);
  return record;
}

/**
 * The stable check `candidate-to-stable-delta` (ADR 0024 decision A; release.md
 * 6.4, 6.7 and 6.8). The plan job of the Release run that published the stable
 * version compared it with its accepted candidate and wrote the verdict into
 * `release-delta.json`. This check reads that record, from the artifact of the
 * run npm's provenance names, and requires it to be about this release: the
 * stable version and commit are the ones the tag, the GitHub release and npm's
 * provenance of all four packages name; the candidate is the highest `-rc.N`
 * tag of the same `X.Y.Z`, at that tag's commit; the verdict is `allowed`; and
 * the record was made by the run that published. It does not repeat the
 * comparison of the files (the plan job did, from the full history, and an
 * enforced plan that failed it published nothing).
 *
 * An expired artifact fails, naming where the record was copied: the input is
 * gone, and a check that passed without its input would be green about the wrong
 * thing.
 *
 * @param {object} context `version`, `facts` (see evaluateRelease), `releaseRunId` and `readers` (lib/release.cjs `githubStableReaders`)
 * @returns {string} what was verified
 */
function candidateToStableDelta(context) {
  const { facts, readers, releaseRunId: runId, version } = context;
  const tag = stableTag(version);
  expect(
    typeof runId === 'string' && runId !== '',
    `npm's provenance does not name one Release run for all four packages, so there is no run whose ${RELEASE_DELTA_FILE} could be read`,
  );
  expect(readers !== undefined, 'the check was given no reader for GitHub');
  const found = readers.releaseDelta(runId);
  if (found.state === 'expired') {
    throw new QualificationError(
      // Short on purpose: the job summary's table cuts a detail at 400 characters, and the pointer must survive that.
      `${RELEASE_DELTA_FILE} cannot be read: the ${PLAN_ARTIFACT} artifact of Release run ${runId} has expired (GitHub keeps a run's files 7 days). ` +
        `The record is copied into ${LEDGER} as release_delta (P14 PR 13): read it with node -p "require('./${LEDGER}').release_delta" and compare it by hand`,
    );
  }
  expect(found.state !== 'no-artifact', `Release run ${runId} has no artifact named ${PLAN_ARTIFACT}, so there is no ${RELEASE_DELTA_FILE} to read`);
  expect(
    found.state !== 'no-record',
    `the ${PLAN_ARTIFACT} artifact of Release run ${runId} holds no ${RELEASE_DELTA_FILE}: only an enforced stable plan writes one, so this run did not publish a stable version`,
  );
  expect(found.state === 'found', `the reader answered ${JSON.stringify(found.state)}`);
  const record = parseReleaseDelta(found.text);

  const problems = [];
  if (record.stable_version !== version) {
    problems.push(`stable_version is ${record.stable_version}, not ${version}`);
  }
  if (record.stable_commit !== facts.tagCommit) {
    problems.push(`stable_commit ${short(record.stable_commit)} is not the commit of ${tag}, ${short(facts.tagCommit)}`);
  }
  if (facts.release.tagName !== tag) {
    problems.push(`the GitHub release is for ${facts.release.tagName}, not ${tag}`);
  }
  const elsewhere = PACKAGES.filter((name) => {
    const provenance = facts.packages[name] && facts.packages[name].provenance;
    return !provenance || provenance.commit !== record.stable_commit;
  });
  if (elsewhere.length > 0) {
    problems.push(`npm's provenance does not name stable_commit ${short(record.stable_commit)} for ${elsewhere.join(', ')}`);
  }

  const prefix = candidateTagPrefix(version);
  const accepted = acceptedCandidateTag(version, readers.tagsWithPrefix(prefix));
  if (accepted === null) {
    problems.push(`no candidate tag ${prefix}<N> exists`);
  } else {
    if (record.candidate_version !== accepted.version) {
      problems.push(`candidate_version is ${record.candidate_version}, but the highest candidate tag is ${accepted.tag}`);
    }
    const candidateCommit = readers.commitOfTag(accepted.tag);
    if (record.candidate_commit !== candidateCommit) {
      problems.push(`candidate_commit ${short(record.candidate_commit)} is not the commit of ${accepted.tag}, ${short(candidateCommit)}`);
    }
  }
  if (record.candidate_commit === record.stable_commit) {
    problems.push('the candidate and the stable release are the same commit');
  }

  if (record.verdict !== 'allowed') {
    problems.push(`verdict is ${JSON.stringify(record.verdict)}, not "allowed"`);
  }
  if (record.check.type !== 'workflow_run' || record.check.workflow !== 'Release') {
    problems.push(`check is ${JSON.stringify(record.check.type)} of ${JSON.stringify(record.check.workflow)}, not a workflow_run of Release`);
  }
  if (String(record.check.run_id) !== runId) {
    problems.push(`check.run_id is ${record.check.run_id}, but npm's provenance names Release run ${runId}`);
  }
  if (!/^[0-9]{4}-[0-9]{2}-[0-9]{2}$/.test(record.date)) {
    problems.push(`date ${JSON.stringify(record.date)} is not YYYY-MM-DD`);
  }

  expect(problems.length === 0, `${RELEASE_DELTA_FILE} of Release run ${runId} is not this release's: ${problems.join('; ')}`);
  return (
    `${RELEASE_DELTA_FILE} of Release run ${runId} (${record.date}): ${record.candidate_version} at ${short(record.candidate_commit)}, the highest candidate tag, ` +
    `to ${record.stable_version} at ${short(record.stable_commit)}, the commit of ${tag} and the one npm's provenance names on all four packages; verdict ${record.verdict}`
  );
}

/**
 * The stable check `latest-on-all-four-packages` (release.md 6.7, "After the
 * publish"): `latest` names this version on all four packages and the version
 * is published, and the GitHub release of its tag is not a draft, not a
 * pre-release and is the release GitHub calls the latest. All of it was read by
 * `gatherFacts`; every wrong place is named, not only the first.
 *
 * @param {object} context `version` and `facts` (see evaluateRelease)
 * @returns {string} what was verified
 */
function latestOnAllFourPackages(context) {
  const { facts, version } = context;
  const tag = stableTag(version);
  const problems = [];
  for (const name of PACKAGES) {
    const entry = facts.packages[name];
    if (entry === undefined) {
      problems.push(`${name} was not read`);
      continue;
    }
    const latest = (entry.distTags || {}).latest;
    if (latest !== version) {
      problems.push(`latest is ${latest === undefined ? 'not set' : latest} on ${name}, not ${version}`);
    }
    if (!entry.versions.includes(version)) {
      problems.push(`${name} does not list ${version} among its versions`);
    }
  }
  const { release } = facts;
  if (release.tagName !== tag) {
    problems.push(`the GitHub release is for ${release.tagName}, not ${tag}`);
  }
  if (release.isDraft !== false) {
    problems.push('the GitHub release is a draft');
  }
  if (release.isPrerelease !== false) {
    problems.push('the GitHub release is marked as a pre-release');
  }
  if (facts.latestRelease !== tag) {
    problems.push(`GitHub's latest release is ${facts.latestRelease || 'none'}, not ${tag}`);
  }
  expect(problems.length === 0, problems.join('; '));
  const next = [...new Set(PACKAGES.map((name) => (facts.packages[name].distTags || {}).next || 'unset'))].join(' and ');
  return `latest is ${version} on all four packages (next is ${next}); the GitHub release ${tag} is not a draft, not a pre-release and is GitHub's latest release`;
}

/**
 * Runs the checks a stable version needs. A stable version whose required
 * checks are not all registered fails here by name, so nothing passes by
 * leaving them out. A pre-release needs none: it gets no result at all.
 *
 * @param {'pre-release' | 'stable'} channel the kind of publish (channelOf)
 * @param {object} context what the checks are given (the facts, the readers of GitHub)
 * @param {Map<string, Function>} [registry] the registered checks; the module's own by default
 */
function runStableChecks(channel, context, registry = STABLE_CHECKS) {
  if (channel !== 'stable') {
    return [];
  }
  return REQUIRED_STABLE_CHECKS.map((name) => {
    const check = registry.get(name);
    if (check === undefined) {
      return {
        name: `stable: ${name}`,
        ok: false,
        detail:
          'not registered: STABLE_CHECKS (tools/p14-published/lib/verify.cjs) has no check of this name, ' +
          `and a stable version is not verified without it${context.releaseRunId ? ` (the Release run is ${context.releaseRunId})` : ''}`,
      };
    }
    try {
      return { name: `stable: ${name}`, ok: true, detail: check(context) || '' };
    } catch (error) {
      return { name: `stable: ${name}`, ok: false, detail: error instanceof QualificationError ? error.message : `${error && error.stack}` };
    }
  });
}

module.exports = {
  PLACEHOLDER,
  channelOf,
  packageUrl,
  releaseRunId,
  REQUIRED_STABLE_CHECKS,
  STABLE_CHECKS,
  candidateToStableDelta,
  evaluateRelease,
  integrityToHex,
  latestOnAllFourPackages,
  parseProvenance,
  parseReleaseDelta,
  runStableChecks,
};
