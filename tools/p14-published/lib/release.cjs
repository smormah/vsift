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
  archiveName,
  checkArchiveChecksum,
  checkChecksums,
  compareVersions,
  downloadAssets,
  expectedAssets,
  gh,
  highestVersion,
  isStable,
  npmReadEnvironment,
  npmView,
  parseChecksums,
  parseVersion,
  tagOf,
  tarballName,
  verifyAttestation,
};
