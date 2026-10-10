'use strict';

// What a published VSift release looks like, and how to read it (install.md
// sections 3 and 6, release.md section 6.4). Pure functions are separate from
// the ones that run `gh` or `npm`, so the first can be tested without a network.

const fs = require('node:fs');
const path = require('node:path');

const {
  PACKAGES,
  QualificationError,
  REGISTRY,
  REPOSITORY,
  SIGNER_WORKFLOW,
  TARGETS,
  expect,
  githubEnvironment,
  run,
  sha256File,
  succeed,
} = require('./common.cjs');

/** The three archive targets of a release, in a fixed order. */
const ARCHIVE_TARGETS = Object.freeze(Object.values(TARGETS).map((target) => target.archiveTarget).sort());

const VERSION_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?$/;

/** Parses `x.y.z[-pre]`; anything else (build metadata, a range, a tag) is refused. */
function parseVersion(text) {
  const match = VERSION_PATTERN.exec(text);
  if (match === null) {
    throw new QualificationError(`${JSON.stringify(text)} is not a version of the form x.y.z or x.y.z-prerelease`);
  }
  return { major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]), pre: match[4] === undefined ? [] : match[4].split('.') };
}

/** SemVer precedence: -1, 0 or 1. */
function compareVersions(left, right) {
  const a = parseVersion(left);
  const b = parseVersion(right);
  for (const field of ['major', 'minor', 'patch']) {
    if (a[field] !== b[field]) {
      return a[field] < b[field] ? -1 : 1;
    }
  }
  if (a.pre.length === 0 || b.pre.length === 0) {
    return a.pre.length === b.pre.length ? 0 : a.pre.length === 0 ? 1 : -1;
  }
  for (let index = 0; index < Math.max(a.pre.length, b.pre.length); index += 1) {
    const x = a.pre[index];
    const y = b.pre[index];
    if (x === undefined || y === undefined) {
      return x === undefined ? -1 : 1;
    }
    const numeric = /^\d+$/.test(x) && /^\d+$/.test(y);
    if (numeric && Number(x) !== Number(y)) {
      return Number(x) < Number(y) ? -1 : 1;
    }
    if (!numeric && x !== y) {
      const xNumeric = /^\d+$/.test(x);
      const yNumeric = /^\d+$/.test(y);
      if (xNumeric !== yNumeric) {
        return xNumeric ? -1 : 1;
      }
      return x < y ? -1 : 1;
    }
  }
  return 0;
}

/** The highest of `versions` by SemVer precedence. */
function highestVersion(versions) {
  expect(versions.length > 0, 'no version to choose from');
  return [...versions].sort(compareVersions)[versions.length - 1];
}

/** A stable version has no pre-release part. */
function isStable(version) {
  return parseVersion(version).pre.length === 0;
}

/** The tag a version is released from. */
function tagOf(version) {
  parseVersion(version);
  return `v${version}`;
}

/** What every candidate tag of a stable version starts with: `v0.2.0-rc.` for `0.2.0`. */
function candidateTagPrefix(stableVersion) {
  expect(isStable(stableVersion), `${stableVersion} is not a stable version, so it has no candidate tags`);
  return `v${stableVersion}-rc.`;
}

/**
 * The accepted candidate of a stable version: the highest-numbered
 * `v<X.Y.Z>-rc.<N>` among `tags`, for the stable version's own `X.Y.Z` and a
 * positive `N` written without a leading zero. Any other tag is ignored, never
 * read as a candidate. This is the rule of `vsift-release candidate-delta`
 * (tools/vsift-release/src/candidate.rs, `candidate_tags`; release.md 6.8),
 * restated here because the check that reads the delta record must name the
 * same candidate that the plan job compared with, from the tags alone.
 *
 * @param {string} stableVersion for example `0.2.0`
 * @param {string[]} tags tag names, in any order
 * @returns {{tag: string, version: string} | null} `null` when no candidate tag exists
 */
function acceptedCandidateTag(stableVersion, tags) {
  const prefix = candidateTagPrefix(stableVersion);
  // The number is held as a BigInt because the tool this mirrors reads a u64, and a tag of
  // `rc.99999999999999999999` must be ignored by both rather than rounded by one.
  const limit = 2n ** 64n - 1n;
  let best = null;
  for (const tag of tags) {
    if (!tag.startsWith(prefix)) {
      continue;
    }
    const digits = tag.slice(prefix.length);
    if (!/^[1-9][0-9]*$/.test(digits) || BigInt(digits) > limit) {
      continue;
    }
    const number = BigInt(digits);
    if (best === null || number > best.number) {
      best = { number, tag };
    }
  }
  return best === null ? null : { tag: best.tag, version: best.tag.slice(1) };
}

/** The name of the archive for a target. */
function archiveName(version, archiveTarget) {
  return `vsift-${version}-${archiveTarget}.tar.gz`;
}

/** The ten files of a release, sorted: three archives, three SBOMs, three notices and SHA256SUMS. */
function expectedAssets(version) {
  const names = ['SHA256SUMS'];
  for (const target of ARCHIVE_TARGETS) {
    names.push(`vsift-${version}-${target}.tar.gz`, `vsift-${version}-${target}.cdx.json`, `vsift-${version}-${target}.THIRD-PARTY-NOTICES.txt`);
  }
  return names.sort();
}

/** The file name of an npm tarball: `vsift-cli-0.1.0.tgz`, `vsift-win32-x64-0.1.0.tgz`. */
function tarballName(packageName, version) {
  return `${packageName.replace(/^@/, '').replace('/', '-')}-${version}.tgz`;
}

/** Parses a `sha256sum` list (`<hex>  <name>` or `<hex> *<name>`) into a map. */
function parseChecksums(text) {
  const sums = new Map();
  for (const line of text.split(/\r?\n/)) {
    if (line.trim() === '') {
      continue;
    }
    const match = /^([0-9a-f]{64}) [ *](.+)$/.exec(line);
    if (match === null) {
      throw new QualificationError(`a line of SHA256SUMS is not "<sha256>  <file>": ${JSON.stringify(line.slice(0, 120))}`);
    }
    if (sums.has(match[2])) {
      throw new QualificationError(`SHA256SUMS lists ${match[2]} twice`);
    }
    sums.set(match[2], match[1]);
  }
  return sums;
}

/**
 * Requires every archive in `directory` to be listed in `SHA256SUMS` with its
 * digest and the list to name nothing else.
 */
function checkChecksums(directory, version) {
  const sums = parseChecksums(fs.readFileSync(path.join(directory, 'SHA256SUMS'), 'utf8'));
  const archives = ARCHIVE_TARGETS.map((target) => archiveName(version, target));
  const listed = [...sums.keys()].sort();
  expect(
    JSON.stringify(listed) === JSON.stringify([...archives].sort()),
    `SHA256SUMS lists ${listed.join(', ')}; expected exactly ${archives.join(', ')}`,
  );
  for (const archive of archives) {
    const actual = sha256File(path.join(directory, archive));
    expect(actual === sums.get(archive), `${archive}: SHA-256 ${actual} is not the listed ${sums.get(archive)}`);
  }
  return archives.length;
}

/** Requires one archive in `directory` to be listed in `SHA256SUMS` with its digest. */
function checkArchiveChecksum(directory, name) {
  const sums = parseChecksums(fs.readFileSync(path.join(directory, 'SHA256SUMS'), 'utf8'));
  expect(sums.has(name), `SHA256SUMS does not list ${name}`);
  const actual = sha256File(path.join(directory, name));
  expect(actual === sums.get(name), `${name}: SHA-256 ${actual} is not the listed ${sums.get(name)}`);
}

// ---------------------------------------------------------------- gh

/** Runs `gh` with the job's read-only token and nothing else of a secret kind. */
function gh(args, options = {}) {
  return run('gh', args, options.env || githubEnvironment(), options);
}

/**
 * `gh attestation verify` as install.md section 3 and release.md section 6.4
 * give it: this repository's Release workflow, the tag's ref, a GitHub-hosted
 * runner. Returns the number of verified attestations it reports, or fails.
 */
function verifyAttestation(file, version, env = githubEnvironment()) {
  const result = succeed(
    'gh',
    [
      'attestation',
      'verify',
      file,
      '--repo',
      REPOSITORY,
      '--signer-workflow',
      SIGNER_WORKFLOW,
      '--source-ref',
      `refs/tags/${tagOf(version)}`,
      '--deny-self-hosted-runners',
    ],
    env,
    { timeout: 180_000 },
  );
  return `${result.stdout}${result.stderr}`;
}

/** Downloads every asset of a release into `directory` with `gh release download`. */
function downloadAssets(version, directory, patterns = [], env = githubEnvironment()) {
  fs.mkdirSync(directory, { recursive: true });
  // With no pattern `gh` downloads every asset: the release's ten files.
  const args = ['release', 'download', tagOf(version), '--repo', REPOSITORY, '--dir', directory];
  for (const pattern of patterns) {
    args.push('--pattern', pattern);
  }
  succeed('gh', args, env, { timeout: 600_000 });
}

// ---------------------------------------------------------------- what the stable checks read from GitHub

/** The artifact the Release workflow's plan job uploads (release.yml, `name: publish-plan`); GitHub keeps it 7 days. */
const PLAN_ARTIFACT = 'publish-plan';
/** The record an enforced stable plan writes into that artifact (tools/vsift-release/src/publish.rs, `RELEASE_DELTA`). */
const RELEASE_DELTA_FILE = 'release-delta.json';
/** The record is a few hundred bytes; a file this large is not it, and is not read. */
const MAXIMUM_RELEASE_DELTA_BYTES = 64 * 1024;

/**
 * Whether a run's artifact listing holds the named artifact, and whether it is
 * still there. GitHub keeps an expired artifact in the listing with
 * `expired: true` and refuses its download, so the listing, not a failed
 * download, is how an expiry is told apart from a mistake.
 *
 * @param {string} text one JSON object per line, `{"name": ..., "expired": ...}`, as the `gh api --jq` of
 *   `readReleaseDelta` prints them
 * @param {string} name the artifact's name
 * @returns {'available' | 'expired' | 'absent'}
 */
function parseArtifactListing(text, name) {
  const entries = text
    .split(/\r?\n/)
    .filter((line) => line.trim() !== '')
    .map((line) => {
      try {
        return JSON.parse(line);
      } catch {
        throw new QualificationError(`a line of the artifact listing is not JSON: ${JSON.stringify(line.slice(0, 120))}`);
      }
    })
    .filter((entry) => entry !== null && typeof entry === 'object' && entry.name === name);
  if (entries.length === 0) {
    return 'absent';
  }
  return entries.some((entry) => entry.expired === false) ? 'available' : 'expired';
}

/**
 * Reads `release-delta.json` out of the `publish-plan` artifact of a Release
 * run, with `gh` and the job's read-only token: it lists the run's artifacts,
 * and downloads the artifact only when it is there and not expired.
 *
 * @param {string} runId the Release run (digits)
 * @param {string} directory an empty folder the artifact is unpacked into
 * @param {(args: string[]) => string} runGh runs `gh` and returns what it printed; a failure throws
 * @returns {{state: 'found', text: string} | {state: 'expired' | 'no-artifact' | 'no-record'}}
 */
function readReleaseDelta(runId, directory, runGh) {
  expect(/^[0-9]+$/.test(runId), `${JSON.stringify(runId)} is not a workflow run id`);
  const state = parseArtifactListing(
    runGh(['api', `repos/${REPOSITORY}/actions/runs/${runId}/artifacts`, '--paginate', '--jq', '.artifacts[] | {name, expired}']),
    PLAN_ARTIFACT,
  );
  if (state !== 'available') {
    return { state: state === 'expired' ? 'expired' : 'no-artifact' };
  }
  runGh(['run', 'download', runId, '--repo', REPOSITORY, '--name', PLAN_ARTIFACT, '--dir', directory]);
  const file = path.join(directory, RELEASE_DELTA_FILE);
  let stat;
  try {
    stat = fs.lstatSync(file);
  } catch {
    return { state: 'no-record' };
  }
  expect(stat.isFile(), `${RELEASE_DELTA_FILE} in the ${PLAN_ARTIFACT} artifact is not a regular file`);
  expect(stat.size <= MAXIMUM_RELEASE_DELTA_BYTES, `${RELEASE_DELTA_FILE} in the ${PLAN_ARTIFACT} artifact is ${stat.size} bytes, far larger than the record`);
  return { state: 'found', text: fs.readFileSync(file, 'utf8') };
}

/** The tag names that start with `prefix` (`v0.2.0-rc.`), from GitHub's own list of tag references. */
function listTagsWithPrefix(prefix, runGh) {
  expect(/^v[0-9]+\.[0-9]+\.[0-9]+-rc\.$/.test(prefix), `${JSON.stringify(prefix)} is not the start of a candidate tag`);
  const lines = runGh(['api', `repos/${REPOSITORY}/git/matching-refs/tags/${prefix}`, '--paginate', '--jq', '.[].ref']);
  return lines
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.startsWith('refs/tags/'))
    .map((line) => line.slice('refs/tags/'.length));
}

/** The commit a tag names, peeled through an annotated tag, the way `gatherFacts` reads the stable tag's. */
function commitOfTag(tag, runGh) {
  const sha = runGh(['api', `repos/${REPOSITORY}/commits/${tag}`, '--jq', '.sha']).trim();
  expect(/^[0-9a-f]{40}$/.test(sha), `the tag ${tag} does not name a commit (${JSON.stringify(sha.slice(0, 60))})`);
  return sha;
}

/**
 * What the stable checks of lib/verify.cjs read from GitHub, for a verification
 * run: `gh` with the job's read-only token and nothing else of a secret kind.
 * The checks are given this object, never `gh` itself, so their tests need no
 * network.
 *
 * @param {object} env the scrubbed environment of the run
 * @param {string} work the run's scratch folder
 */
function githubStableReaders(env, work) {
  const runGh = (args) => succeed('gh', args, githubEnvironment(env), { timeout: 180_000 }).stdout;
  return {
    releaseDelta: (runId) => readReleaseDelta(runId, path.join(work, `release-plan-${runId}`), runGh),
    tagsWithPrefix: (prefix) => listTagsWithPrefix(prefix, runGh),
    commitOfTag: (tag) => commitOfTag(tag, runGh),
  };
}

// ---------------------------------------------------------------- npm

/** The environment `npm` runs in for a read of the public registry: no token, no setting of the runner's. */
function npmReadEnvironment(base, work) {
  return {
    ...base,
    npm_config_registry: REGISTRY,
    npm_config_userconfig: path.join(work, 'read.npmrc'),
    npm_config_globalconfig: path.join(work, 'read-global.npmrc'),
    npm_config_cache: path.join(work, 'read-cache'),
    npm_config_update_notifier: 'false',
    npm_config_audit: 'false',
    npm_config_fund: 'false',
  };
}

/** `npm view <spec> [field]` as JSON, or null when the registry has no such thing. */
function npmView(spec, field, env, work) {
  const args = ['view', spec];
  if (field) {
    args.push(field);
  }
  args.push('--json');
  const result = run('npm', args, npmReadEnvironment(env, work), { timeout: 120_000 });
  if (result.status !== 0) {
    if (/E404|404 Not Found|is not in this registry/.test(`${result.stderr}${result.stdout}`)) {
      return null;
    }
    throw new QualificationError(`npm view ${spec} ${field || ''} exited ${result.status}: ${(result.stderr || '').trim().slice(0, 400)}`);
  }
  const text = result.stdout.trim();
  return text === '' ? null : JSON.parse(text);
}

module.exports = {
  ARCHIVE_TARGETS,
  PACKAGES,
  PLAN_ARTIFACT,
  RELEASE_DELTA_FILE,
  acceptedCandidateTag,
  archiveName,
  candidateTagPrefix,
  checkArchiveChecksum,
  checkChecksums,
  commitOfTag,
  compareVersions,
  downloadAssets,
  expectedAssets,
  gh,
  githubStableReaders,
  highestVersion,
  isStable,
  listTagsWithPrefix,
  npmReadEnvironment,
  npmView,
  parseArtifactListing,
  parseChecksums,
  parseVersion,
  readReleaseDelta,
  tagOf,
  tarballName,
  verifyAttestation,
};
