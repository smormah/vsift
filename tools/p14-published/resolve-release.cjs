'use strict';

// Chooses the versions a published-artifact run is about (P14 PR 2), and writes
// them to the job outputs.
//
//   INPUT_VERSION=<version or empty> INPUT_FROM_VERSION=<version or empty> \
//     node tools/p14-published/resolve-release.cjs --work <scratch folder>
//
// - `version` defaults to the highest version of `vsift-cli` on the real
//   registry (the placeholder `0.0.0` counts only if nothing else is
//   published), so a pull request or a dispatch with no input qualifies
//   whatever was published last.
// - `from_version` defaults to the first release, 0.1.0 (the baseline of the
//   upgrade evidence).
// - Both must be published, and the tag of `version` must name a commit.
//
// The inputs arrive through the environment, never through the command line:
// a workflow input is text an outsider may choose.

const fs = require('node:fs');
const path = require('node:path');

const { QualificationError, REPOSITORY, cleanEnvironment, expect, githubEnvironment, parseArguments, succeed } = require('./lib/common.cjs');
const { highestVersion, npmView, parseVersion, tagOf } = require('./lib/release.cjs');

/** The first release, the baseline every upgrade starts from. */
const FIRST_RELEASE = '0.1.0';

/**
 * The versions to qualify, from the inputs (empty means "choose") and the
 * published versions of the launcher package.
 */
function chooseVersions({ inputVersion, inputFromVersion, published }) {
  expect(published.length > 0, 'the registry lists no version of vsift-cli');
  const version = inputVersion && inputVersion.trim() !== '' ? inputVersion.trim() : highestVersion(published);
  const fromVersion = inputFromVersion && inputFromVersion.trim() !== '' ? inputFromVersion.trim() : FIRST_RELEASE;
  parseVersion(version);
  parseVersion(fromVersion);
  expect(published.includes(version), `vsift-cli@${version} is not published (published: ${published.join(', ')})`);
  expect(published.includes(fromVersion), `vsift-cli@${fromVersion} is not published (published: ${published.join(', ')})`);
  return { version, fromVersion };
}

function main() {
  const options = parseArguments(process.argv.slice(2), { required: ['work'] });
  const work = path.resolve(options.work);
  fs.mkdirSync(work, { recursive: true });
  const env = cleanEnvironment(process.env);
  const versions = npmView('vsift-cli', 'versions', env, work);
  const published = Array.isArray(versions) ? versions : versions ? [versions] : [];
  const { version, fromVersion } = chooseVersions({
    inputVersion: process.env.INPUT_VERSION,
    inputFromVersion: process.env.INPUT_FROM_VERSION,
    published,
  });
  const commitOf = (name) => {
    const sha = succeed('gh', ['api', `repos/${REPOSITORY}/commits/${name}`, '--jq', '.sha'], githubEnvironment(env), { timeout: 120_000 }).stdout.trim();
    expect(/^[0-9a-f]{40}$/.test(sha), `the tag ${name} does not name a commit (${JSON.stringify(sha)})`);
    return sha;
  };
  const tag = tagOf(version);
  const commit = commitOf(tag);
  const fromCommit = commitOf(tagOf(fromVersion));
  const lines = [`version=${version}`, `from_version=${fromVersion}`, `tag=${tag}`, `commit=${commit}`, `from_commit=${fromCommit}`];
  process.stdout.write(`${lines.join('\n')}\n(published: ${published.join(', ')})\n`);
  if (process.env.GITHUB_OUTPUT) {
    fs.appendFileSync(process.env.GITHUB_OUTPUT, `${lines.join('\n')}\n`);
  }
  if (process.env.GITHUB_STEP_SUMMARY) {
    fs.appendFileSync(
      process.env.GITHUB_STEP_SUMMARY,
      `### Versions\n\nQualifying \`vsift-cli\` **${version}** (tag \`${tag}\`, commit \`${commit.slice(0, 12)}\`); upgrade baseline **${fromVersion}** (commit \`${fromCommit.slice(0, 12)}\`); published: ${published.join(', ')}.\n\n`,
    );
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error instanceof QualificationError ? error.message : error && error.stack}\n`);
    process.exitCode = 1;
  }
}

module.exports = { FIRST_RELEASE, chooseVersions };
