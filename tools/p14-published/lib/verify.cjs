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
// **The stable extension.** A stable version adds two checks: the
// candidate-to-stable delta (only version strings and documents that ship
// inside the artifacts differ) and `latest` on all four packages with the
// GitHub release marked latest. They are NOT built here: no stable release has
// ever been published, so neither could be tried on real bytes. `STABLE_CHECKS`
// is where they are registered, before the stable publish (P14 PR 12's
// preparation; docs/planning/p14-qualification.md section 15), and a stable
// version fails verification by name until they are: a verification of a stable
// release that silently skipped them would be a green result about the wrong
// thing.
//
// What the delta check reads is P14 PR 8's record: an enforced stable plan
// writes `release-delta.json` into the `publish-plan` artifact of the Release
// run that published, which GitHub keeps for seven days. `context.releaseRunId`
// names that run (the one npm's provenance names; see `releaseRunId`), so a
// check fetches the record without guessing which run it came from.

const {
  PACKAGES,
  QualificationError,
  REPOSITORY,
  expect,
} = require('./common.cjs');
const { expectedAssets, parseVersion, tagOf } = require('./release.cjs');

/** The checks a stable version needs and nobody has registered yet. */
const REQUIRED_STABLE_CHECKS = Object.freeze(['candidate-to-stable-delta', 'latest-on-all-four-packages']);

/**
 * The registered stable checks, by name: each is `(context) => string` and
 * throws a QualificationError when it fails. It is empty today; the two checks
 * are added before the stable publish (a test in `test/verify.test.cjs` shows
 * the shape).
 */
const STABLE_CHECKS = new Map();

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

/**
 * Runs the checks a stable version needs. A stable version whose required
 * checks are not all registered fails here by name, so nothing passes by
 * leaving them out. A pre-release needs none.
 *
 * @param {'pre-release' | 'stable'} channel the kind of publish (channelOf)
 * @param {object} context what the checks are given (the facts, the downloaded files)
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
          'not registered yet: it goes into STABLE_CHECKS (tools/p14-published/lib/verify.cjs) before the stable publish, ' +
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
  evaluateRelease,
  integrityToHex,
  parseProvenance,
  runStableChecks,
};
