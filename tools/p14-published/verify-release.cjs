'use strict';

// RQ-19: a second verification of a publish, from a hosted runner with no
// publishing credential (P14 PR 2; ADR 0024; docs/planning/p14-qualification.md
// section 2; the commands of docs/operations/release.md section 6.4).
//
//   node tools/p14-published/verify-release.cjs --version <published version> --work <scratch folder>
//
// It reads the registry and GitHub as an outsider does and checks that what was
// published is what the release workflow built from the tag: the four packages'
// dist-tags and npm provenance, `npm audit signatures`, `gh attestation verify`
// of every release file and every npm tarball, the checksums, the release's
// flags and its ten files. The pure rules are in lib/verify.cjs.
//
// The only credential in reach is the job's default read-only token, which only
// `gh` sees. Nothing is published, tagged or changed. A stable version also
// needs two checks (the candidate-to-stable delta, read from P14 PR 8's
// `release-delta.json` in the publish run's `publish-plan` artifact while GitHub
// keeps it, and `latest` on all four packages with the GitHub release marked
// latest); they are registered in lib/verify.cjs (`STABLE_CHECKS`), and a stable
// version whose check is not registered fails here by name. A pre-release is not
// asked for either.

const fs = require('node:fs');
const path = require('node:path');

const {
  PACKAGES,
  QualificationError,
  Recorder,
  REPOSITORY,
  USER_AGENT,
  cleanEnvironment,
  describeStatus,
  expect,
  githubEnvironment,
  parseArguments,
  run,
  sha256File,
  succeed,
  writeFile,
} = require('./lib/common.cjs');
const {
  checkChecksums,
  downloadAssets,
  expectedAssets,
  gh,
  githubStableReaders,
  npmReadEnvironment,
  npmView,
  parseVersion,
  tagOf,
  tarballName,
  verifyAttestation,
} = require('./lib/release.cjs');
const {
  STABLE_CHECKS,
  channelOf,
  evaluateRelease,
  integrityToHex,
  parseProvenance,
  releaseRunId,
  runStableChecks,
} = require('./lib/verify.cjs');
const crypto = require('node:crypto');

/** Reads the registry and GitHub for the facts evaluateRelease needs. */
async function gatherFacts(version, env, work) {
  const tag = tagOf(version);
  const tagCommit = succeed('gh', ['api', `repos/${REPOSITORY}/commits/${tag}`, '--jq', '.sha'], githubEnvironment(env), { timeout: 120_000 }).stdout.trim();
  const packages = {};
  for (const name of PACKAGES) {
    const distTags = npmView(name, 'dist-tags', env, work);
    const versions = npmView(name, 'versions', env, work);
    const manifest = npmView(`${name}@${version}`, undefined, env, work);
    const entry = { distTags: distTags || {}, versions: Array.isArray(versions) ? versions : versions ? [versions] : [], manifest };
    const url = manifest && manifest.dist && manifest.dist.attestations && manifest.dist.attestations.url;
    if (url) {
      try {
        const response = await fetch(url, { headers: { 'user-agent': USER_AGENT } });
        expect(response.ok, `HTTP ${response.status}`);
        entry.provenance = parseProvenance(await response.json());
      } catch (error) {
        entry.provenanceError = error.message;
      }
    } else {
      entry.provenanceError = 'the registry lists no attestations for this version';
    }
    packages[name] = entry;
  }
  const view = gh(['release', 'view', tag, '--repo', REPOSITORY, '--json', 'isDraft,isPrerelease,tagName,assets,targetCommitish,url,publishedAt'], {
    env: githubEnvironment(env),
    timeout: 120_000,
  });
  expect(view.status === 0, `gh release view ${tag} exited ${view.status}: ${(view.stderr || '').trim().slice(0, 300)}`);
  const release = JSON.parse(view.stdout);
  const latest = gh(['api', `repos/${REPOSITORY}/releases/latest`, '--jq', '.tag_name'], { env: githubEnvironment(env), timeout: 120_000 });
  let latestRelease = null;
  if (latest.status === 0) {
    latestRelease = latest.stdout.trim();
  } else {
    expect(/404|Not Found/.test(`${latest.stderr}${latest.stdout}`), `gh api releases/latest exited ${latest.status}: ${latest.stderr.slice(0, 300)}`);
  }
  return { version, tag, tagCommit, packages, release, latestRelease };
}

async function main() {
  const options = parseArguments(process.argv.slice(2), { required: ['version', 'work'] });
  const { version } = options;
  parseVersion(version);
  const work = path.resolve(options.work);
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const recorder = new Recorder();
  const title = `RQ-19 second verification of vsift-cli ${version} (no publishing credential)`;
  const env = cleanEnvironment(process.env);
  let facts;

  const gathered = await recorder.check('read the registry and GitHub', async () => {
    facts = await gatherFacts(version, env, work);
    return `tag ${facts.tag} names ${facts.tagCommit.slice(0, 12)}; ${PACKAGES.length} packages read; release ${facts.release.url}`;
  });
  if (!gathered) {
    recorder.finish(title);
    return;
  }

  for (const result of evaluateRelease(facts)) {
    await recorder.check(result.name, () => {
      if (!result.ok) {
        throw new QualificationError(result.detail);
      }
      return result.detail;
    });
  }

  // The release's files: downloaded, matched to GitHub's own digests and to SHA256SUMS, each attested.
  const directory = path.join(work, 'release');
  const downloaded = await recorder.check('download the release files and require GitHub\'s digest and the checksums to match', () => {
    downloadAssets(version, directory, [], githubEnvironment(env));
    const names = fs.readdirSync(directory).sort();
    expect(JSON.stringify(names) === JSON.stringify(expectedAssets(version)), `downloaded ${names.join(', ')}`);
    for (const asset of facts.release.assets) {
      const actual = `sha256:${sha256File(path.join(directory, asset.name))}`;
      expect(actual === asset.digest, `${asset.name}: ${actual} is not the digest GitHub records, ${asset.digest}`);
    }
    const archives = checkChecksums(directory, version);
    return `${names.length} files match GitHub's digests; SHA256SUMS holds ${archives} archives`;
  });
  if (downloaded) {
    await recorder.check('gh attestation verify: every release file came from the Release workflow at the tag', () => {
      for (const name of expectedAssets(version)) {
        verifyAttestation(path.join(directory, name), version, githubEnvironment(env));
      }
      return `${expectedAssets(version).length} of ${expectedAssets(version).length} files verified`;
    });
  }

  // The four npm tarballs: packed from the registry, matched to its integrity, each attested.
  await recorder.check("gh attestation verify: every npm tarball is the registry's and came from the Release workflow at the tag", () => {
    const tarballs = path.join(work, 'tarballs');
    fs.mkdirSync(tarballs, { recursive: true });
    const project = path.join(work, 'pack');
    writeFile(path.join(project, 'package.json'), `${JSON.stringify({ name: 'vsift-qualification', version: '0.0.0', private: true }, null, 2)}\n`);
    const readEnv = npmReadEnvironment(env, work);
    for (const name of PACKAGES) {
      succeed('npm', ['pack', `${name}@${version}`, '--pack-destination', tarballs, '--ignore-scripts'], readEnv, { cwd: project });
      const file = path.join(tarballs, tarballName(name, version));
      expect(fs.existsSync(file), `npm pack did not write ${file}`);
      const sha512 = crypto.createHash('sha512').update(fs.readFileSync(file)).digest('hex');
      expect(sha512 === integrityToHex(facts.packages[name].manifest.dist.integrity), `${name}: the packed tarball is not the registry's integrity`);
      verifyAttestation(file, version, githubEnvironment(env));
    }
    return `${PACKAGES.length} of ${PACKAGES.length} tarballs verified`;
  });

  await recorder.check('npm audit signatures: registry signatures and provenance attestations verify', () => {
    const project = path.join(work, 'audit');
    writeFile(path.join(project, 'package.json'), `${JSON.stringify({ name: 'vsift-qualification', version: '0.0.0', private: true }, null, 2)}\n`);
    const readEnv = { ...npmReadEnvironment(env, work), npm_config_cache: path.join(work, 'audit-cache') };
    succeed('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund', `vsift-cli@${version}`], readEnv, { cwd: project });
    const audit = run('npm', ['audit', 'signatures'], readEnv, { cwd: project, timeout: 300_000 });
    const text = `${audit.stdout}${audit.stderr}`;
    expect(audit.status === 0, `${describeStatus(audit)}: ${text.trim().slice(0, 600)}`);
    const signatures = /(\d+) packages? ha(?:s|ve) verified registry signatures/.exec(text);
    const attestations = /(\d+) packages? ha(?:s|ve) verified attestations/.exec(text);
    expect(
      signatures && attestations && signatures[1] === attestations[1] && Number(signatures[1]) >= 2,
      `unexpected output: ${text.trim().slice(0, 600)}`,
    );
    return `${signatures[1]} packages with verified registry signatures, ${attestations[1]} with verified attestations`;
  });

  let channel = 'pre-release';
  try {
    channel = channelOf(version, facts.release);
  } catch {
    // The disagreement is already a failed check above; nothing stable is claimed.
  }
  const context = { facts, version, env, work, releaseRunId: releaseRunId(facts.packages), readers: githubStableReaders(env, work) };
  for (const result of runStableChecks(channel, context, STABLE_CHECKS)) {
    await recorder.check(result.name, () => {
      if (!result.ok) {
        throw new QualificationError(result.detail);
      }
      return result.detail;
    });
  }

  recorder.finish(title);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
  process.exitCode = 1;
});
