'use strict';

// Tests of the release helpers (lib/release.cjs) and the shared helpers
// (lib/common.cjs): version precedence, the ten release files, SHA256SUMS and
// the environment given to `gh`.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const common = require('../lib/common.cjs');
const release = require('../lib/release.cjs');

test('version precedence follows SemVer', () => {
  const ascending = ['0.0.0', '0.1.0-alpha', '0.1.0-alpha.1', '0.1.0-alpha.beta', '0.1.0-beta', '0.1.0-beta.2', '0.1.0-beta.11', '0.1.0-rc.1', '0.1.0', '0.1.1-p14local.1', '0.1.1', '0.2.0-rc.1', '0.2.0', '1.0.0'];
  for (let low = 0; low < ascending.length; low += 1) {
    for (let high = 0; high < ascending.length; high += 1) {
      const expected = low === high ? 0 : low < high ? -1 : 1;
      assert.equal(release.compareVersions(ascending[low], ascending[high]), expected, `${ascending[low]} against ${ascending[high]}`);
    }
  }
  assert.equal(release.highestVersion(['0.0.0', '0.2.0-rc.1', '0.1.0', '0.2.0-rc.2']), '0.2.0-rc.2');
  assert.throws(() => release.highestVersion([]), /no version/);
});

test('only plain versions are accepted', () => {
  for (const bad of ['v0.1.0', '0.1', '0.1.0.0', '01.1.0', '0.1.0+build', 'next', '', '0.1.0-', '0.1.0-01', '../0.1.0', '0.1.0 ']) {
    assert.throws(() => release.parseVersion(bad), /not a version/, JSON.stringify(bad));
  }
  assert.equal(release.isStable('0.2.0'), true);
  assert.equal(release.isStable('0.2.0-rc.1'), false);
  assert.equal(release.tagOf('0.1.0'), 'v0.1.0');
  assert.throws(() => release.tagOf('latest'));
});

test('a release has ten files and four tarballs', () => {
  const assets = release.expectedAssets('0.1.0');
  assert.equal(assets.length, 10);
  assert.deepEqual(assets, [...assets].sort());
  assert.ok(assets.includes('SHA256SUMS'));
  assert.ok(assets.includes('vsift-0.1.0-x86_64-pc-windows-msvc.tar.gz'));
  assert.ok(assets.includes('vsift-0.1.0-aarch64-apple-darwin.cdx.json'));
  assert.ok(assets.includes('vsift-0.1.0-x86_64-unknown-linux-gnu.THIRD-PARTY-NOTICES.txt'));
  assert.deepEqual(
    common.PACKAGES.map((name) => release.tarballName(name, '0.1.0')),
    ['vsift-cli-0.1.0.tgz', 'vsift-win32-x64-0.1.0.tgz', 'vsift-darwin-arm64-0.1.0.tgz', 'vsift-linux-x64-0.1.0.tgz'],
  );
});

test('SHA256SUMS is read strictly and every archive is held to its digest', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 sums '));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const lines = [];
  for (const target of release.ARCHIVE_TARGETS) {
    const name = release.archiveName('0.1.0', target);
    fs.writeFileSync(path.join(directory, name), `archive ${target}`);
    lines.push(`${common.sha256Hex(`archive ${target}`)}  ${name}`);
  }
  fs.writeFileSync(path.join(directory, 'SHA256SUMS'), `${lines.join('\n')}\n`);
  assert.equal(release.checkChecksums(directory, '0.1.0'), 3);

  fs.appendFileSync(path.join(directory, release.archiveName('0.1.0', release.ARCHIVE_TARGETS[0])), 'x');
  assert.throws(() => release.checkChecksums(directory, '0.1.0'), /is not the listed/);

  fs.writeFileSync(path.join(directory, 'SHA256SUMS'), `${lines.slice(0, 2).join('\n')}\n`);
  assert.throws(() => release.checkChecksums(directory, '0.1.0'), /expected exactly/);
  assert.throws(() => release.parseChecksums('not a line\n'), /is not "<sha256>  <file>"/);
  assert.throws(() => release.parseChecksums(`${lines[0]}\n${lines[0]}\n`), /twice/);
  assert.equal(release.parseChecksums(`${'a'.repeat(64)} *star.bin\n`).get('star.bin'), 'a'.repeat(64));
});

test('the environments given to children hold no credential and no package-manager setting', () => {
  const source = {
    PATH: '/usr/bin',
    GITHUB_TOKEN: 'secret-token',
    GH_TOKEN: 'another',
    NODE_AUTH_TOKEN: 'npm-secret',
    ACTIONS_ID_TOKEN_REQUEST_TOKEN: 'oidc',
    ACTIONS_RUNTIME_TOKEN: 'runtime',
    npm_config_registry: 'http://elsewhere/',
    PNPM_HOME: '/x',
    YARN_NPM_AUTH_TOKEN: 'yarn-secret',
    BUN_CONFIG_REGISTRY: 'http://elsewhere/',
    NODE_OPTIONS: '--require x',
    KEEP: 'yes',
  };
  const clean = common.cleanEnvironment(source);
  assert.deepEqual(Object.keys(clean).sort(), ['KEEP', 'PATH']);
  const forGh = common.githubEnvironment(source, source);
  assert.equal(forGh.GH_TOKEN, 'another', 'gh gets the job token and nothing else of a secret kind');
  assert.equal(forGh.NODE_AUTH_TOKEN, undefined);
  assert.equal(forGh.GITHUB_TOKEN, undefined);
  assert.equal(forGh.KEEP, 'yes');
  const none = common.githubEnvironment({ PATH: 'x' }, {});
  assert.equal(none.GH_TOKEN, undefined);
});

test('arguments are parsed strictly', () => {
  const spec = { required: ['version'], optional: ['work'], flags: ['quiet'] };
  assert.deepEqual(common.parseArguments(['--version', '0.1.0', '--quiet'], spec), { version: '0.1.0', quiet: true });
  assert.throws(() => common.parseArguments([], spec), /--version is required/);
  assert.throws(() => common.parseArguments(['--version'], spec), /needs a value/);
  assert.throws(() => common.parseArguments(['--version', '--work'], spec), /needs a value/);
  assert.throws(() => common.parseArguments(['--version', '1', '--other', 'x'], spec), /unknown option/);
  assert.throws(() => common.parseArguments(['version'], spec), /unexpected argument/);
});

test('two directory trees compare by file names and bytes', (t) => {
  const make = (files) => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 tree '));
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    for (const [name, text] of Object.entries(files)) {
      fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
      fs.writeFileSync(path.join(root, name), text);
    }
    return root;
  };
  const left = make({ 'a.md': 'one', 'sub/b.md': 'two' });
  assert.deepEqual(common.compareTrees(left, make({ 'a.md': 'one', 'sub/b.md': 'two' })), { equal: true, files: 2, problems: [] });
  const different = common.compareTrees(left, make({ 'a.md': 'ONE', 'c.md': 'x' }));
  assert.equal(different.equal, false);
  assert.equal(different.problems.length, 3);
});

test('a recorder reports checks, observations and failures as a table and sets the exit status', async () => {
  const recorder = new common.Recorder();
  const quiet = process.stdout.write;
  process.stdout.write = () => true;
  try {
    assert.equal(await recorder.check('passes', () => 'fine'), true);
    recorder.observe('observed', 'a | b');
    assert.equal(await recorder.check('fails', () => { throw new common.QualificationError('went wrong'); }), false);
  } finally {
    process.stdout.write = quiet;
  }
  const text = recorder.markdown('Title');
  assert.match(text, /\| passes \| pass \| fine \|/);
  assert.match(text, /\| observed \| observed \| a \\\| b \|/);
  assert.match(text, /\| fails \| \*\*FAIL\*\* \| went wrong \|/);
  assert.equal(recorder.ok, false);
});

// ---------------------------------------------------------------- what the stable checks read from GitHub

const repository = path.join(__dirname, '..', '..', '..');

test('the accepted candidate is the highest plain rc number of the stable version\'s own X.Y.Z, as vsift-release reads it', () => {
  // The list of tools/vsift-release/src/candidate.rs's own test of `candidate_tags`, and the same answer.
  const listed = [
    'v0.2.0-rc.1',
    'v0.2.0-rc.10',
    'v0.2.0-rc.9',
    'v0.2.0-rc.0',
    'v0.2.0-rc.01',
    'v0.2.0-rc.',
    'v0.2.0-rc.1-extra',
    'v0.2.0-rc.1.2',
    'v0.2.0-beta.1',
    'v0.2.1-rc.1',
    'v0.2.00-rc.1',
    'v0.2.0',
    '0.2.0-rc.2',
    'v0.2.0-rc.99999999999999999999999999',
  ];
  assert.deepEqual(release.acceptedCandidateTag('0.2.0', listed), { tag: 'v0.2.0-rc.10', version: '0.2.0-rc.10' });
  assert.deepEqual(release.acceptedCandidateTag('0.2.0', ['v0.2.0-rc.3', 'v0.2.0-rc.1', 'v0.2.0-rc.2']), { tag: 'v0.2.0-rc.3', version: '0.2.0-rc.3' });
  assert.deepEqual(release.acceptedCandidateTag('0.2.0', ['v0.2.0-rc.18446744073709551615']), {
    tag: 'v0.2.0-rc.18446744073709551615',
    version: '0.2.0-rc.18446744073709551615',
  });
  assert.equal(release.acceptedCandidateTag('0.2.0', ['v0.2.0-rc.18446744073709551616']), null, 'a number a u64 cannot hold is ignored, as the tool ignores it');
  assert.equal(release.acceptedCandidateTag('0.2.0', []), null);
  assert.equal(release.acceptedCandidateTag('0.2.0', ['v0.2.1-rc.1', 'v0.2.0-rc.0', 'v0.2.0-rc.01']), null);
  assert.equal(release.candidateTagPrefix('0.2.0'), 'v0.2.0-rc.');
  assert.throws(() => release.candidateTagPrefix('0.2.0-rc.3'), /not a stable version/);
  assert.throws(() => release.acceptedCandidateTag('0.2.0-rc.3', ['v0.2.0-rc.3']), /not a stable version/);
});

test('an artifact listing says whether the artifact is there, expired or absent', () => {
  const line = (name, expired) => JSON.stringify({ name, expired });
  assert.equal(release.parseArtifactListing([line('npm-packages', false), line('publish-plan', false)].join('\n'), 'publish-plan'), 'available');
  assert.equal(release.parseArtifactListing(`${line('publish-plan', true)}\n`, 'publish-plan'), 'expired');
  assert.equal(release.parseArtifactListing([line('publish-plan', true), line('publish-plan', false)].join('\r\n'), 'publish-plan'), 'available');
  assert.equal(release.parseArtifactListing(line('npm-packages', false), 'publish-plan'), 'absent');
  assert.equal(release.parseArtifactListing('', 'publish-plan'), 'absent');
  assert.equal(release.parseArtifactListing('\n\n', 'publish-plan'), 'absent');
  assert.throws(() => release.parseArtifactListing('<html>rate limited</html>', 'publish-plan'), /not JSON/);
  // An entry without `expired: false` is not taken for a live artifact.
  assert.equal(release.parseArtifactListing(JSON.stringify({ name: 'publish-plan' }), 'publish-plan'), 'expired');
});

/** A `gh` that answers from a script and records its arguments; `download` stands for what `gh run download` writes into `--dir`. */
function scriptedGh({ listing, download }) {
  const calls = [];
  const runGh = (args) => {
    calls.push(args);
    if (args[0] === 'api') {
      return listing;
    }
    assert.equal(args[0], 'run');
    download(args[args.indexOf('--dir') + 1]);
    return '';
  };
  return { calls, runGh };
}

const RECORD = '{"stable_version":"0.2.0"}';
const AVAILABLE = `${JSON.stringify({ name: 'publish-plan', expired: false })}\n`;

test('the delta record is read from the publish-plan artifact of the named run, and only when the artifact is there', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 delta '));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const directory = path.join(root, 'plan');

  const found = scriptedGh({
    listing: AVAILABLE,
    download: (into) => {
      fs.mkdirSync(into, { recursive: true });
      fs.writeFileSync(path.join(into, 'release-delta.json'), RECORD);
      fs.writeFileSync(path.join(into, 'publish-plan.md'), 'unrelated files of the artifact are not read');
    },
  });
  assert.deepEqual(release.readReleaseDelta('37946261087', directory, found.runGh), { state: 'found', text: RECORD });
  assert.deepEqual(found.calls, [
    ['api', 'repos/smormah/vsift/actions/runs/37946261087/artifacts', '--paginate', '--jq', '.artifacts[] | {name, expired}'],
    ['run', 'download', '37946261087', '--repo', 'smormah/vsift', '--name', 'publish-plan', '--dir', directory],
  ]);

  const expired = scriptedGh({
    listing: `${JSON.stringify({ name: 'publish-plan', expired: true })}\n`,
    download: () => assert.fail('an expired artifact must not be downloaded'),
  });
  assert.deepEqual(release.readReleaseDelta('37946261087', directory, expired.runGh), { state: 'expired' });
  assert.equal(expired.calls.length, 1, 'only the listing was asked for');

  const absent = scriptedGh({ listing: `${JSON.stringify({ name: 'npm-packages', expired: false })}\n`, download: () => assert.fail('nothing to download') });
  assert.deepEqual(release.readReleaseDelta('37946261087', directory, absent.runGh), { state: 'no-artifact' });

  const withoutRecord = scriptedGh({
    listing: AVAILABLE,
    download: (into) => {
      fs.mkdirSync(into, { recursive: true });
      fs.writeFileSync(path.join(into, 'publish-plan.md'), 'a plan with no record');
    },
  });
  assert.deepEqual(release.readReleaseDelta('37946261087', path.join(root, 'other'), withoutRecord.runGh), { state: 'no-record' });

  assert.throws(() => release.readReleaseDelta('37946261087; rm -rf', directory, found.runGh), /not a workflow run id/);
  assert.throws(() => release.readReleaseDelta('', directory, found.runGh), /not a workflow run id/);
});

test('a record that is not a small regular file is refused, and a failing gh is a failure, not an absence', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'p14 delta '));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const directoryNamedLikeIt = scriptedGh({
    listing: AVAILABLE,
    download: (into) => fs.mkdirSync(path.join(into, 'release-delta.json'), { recursive: true }),
  });
  assert.throws(() => release.readReleaseDelta('1', path.join(root, 'a'), directoryNamedLikeIt.runGh), /is not a regular file/);

  const huge = scriptedGh({
    listing: AVAILABLE,
    download: (into) => {
      fs.mkdirSync(into, { recursive: true });
      fs.writeFileSync(path.join(into, 'release-delta.json'), 'x'.repeat(64 * 1024 + 1));
    },
  });
  assert.throws(() => release.readReleaseDelta('1', path.join(root, 'b'), huge.runGh), /far larger than the record/);

  const denied = () => {
    throw new common.QualificationError('gh api repos/smormah/vsift/actions/runs/1/artifacts exited 1 (signal null): gh: Resource not accessible by integration (HTTP 403)');
  };
  assert.throws(() => release.readReleaseDelta('1', path.join(root, 'c'), denied), /HTTP 403/);
});

test('candidate tags and their commits are read from the tag references and the commit of a tag', () => {
  const seen = [];
  const refs = ['refs/tags/v0.2.0-rc.1', 'refs/tags/v0.2.0-rc.2', 'refs/tags/v0.2.0-rc.3', '', 'something else'].join('\n');
  const tags = release.listTagsWithPrefix('v0.2.0-rc.', (args) => {
    seen.push(args);
    return `${refs}\n`;
  });
  assert.deepEqual(tags, ['v0.2.0-rc.1', 'v0.2.0-rc.2', 'v0.2.0-rc.3']);
  assert.deepEqual(seen, [['api', 'repos/smormah/vsift/git/matching-refs/tags/v0.2.0-rc.', '--paginate', '--jq', '.[].ref']]);
  assert.deepEqual(release.listTagsWithPrefix('v0.2.0-rc.', () => ''), []);
  for (const bad of ['', 'v0.2.0-rc', 'v0.2.0', 'v0.2.0-rc.1', '../v0.2.0-rc.', 'v0.2.0-rc./x', 'v0.2.0-beta.']) {
    assert.throws(() => release.listTagsWithPrefix(bad, () => assert.fail('nothing may be asked for')), /not the start of a candidate tag/, JSON.stringify(bad));
  }

  const sha = '83dca856e7a00fc9a71c87baae99f0b1d401dd31';
  const asked = [];
  assert.equal(
    release.commitOfTag('v0.2.0-rc.3', (args) => {
      asked.push(args);
      return `${sha}\n`;
    }),
    sha,
  );
  assert.deepEqual(asked, [['api', 'repos/smormah/vsift/commits/v0.2.0-rc.3', '--jq', '.sha']]);
  for (const answer of ['', 'null\n', `${sha.toUpperCase()}\n`, `${sha}0\n`, '<html>']) {
    assert.throws(() => release.commitOfTag('v0.2.0-rc.3', () => answer), /does not name a commit/, JSON.stringify(answer));
  }
});

test('the readers a verification run is given are the three the stable checks use, and nothing runs until one is called', () => {
  const readers = release.githubStableReaders({ PATH: '/usr/bin' }, '/nowhere');
  assert.deepEqual(Object.keys(readers).sort(), ['commitOfTag', 'releaseDelta', 'tagsWithPrefix']);
  for (const reader of Object.values(readers)) {
    assert.equal(typeof reader, 'function');
  }
});

test('the names the stable check relies on are the ones the Release workflow and the plan tool use', () => {
  const workflow = fs.readFileSync(path.join(repository, '.github', 'workflows', 'release.yml'), 'utf8');
  assert.equal(release.PLAN_ARTIFACT, 'publish-plan');
  assert.ok(new RegExp(`^\\s+name: ${release.PLAN_ARTIFACT}$`, 'm').test(workflow), 'the Release workflow uploads an artifact of that name');
  const planTool = fs.readFileSync(path.join(repository, 'tools', 'vsift-release', 'src', 'publish.rs'), 'utf8');
  assert.ok(planTool.includes(`RELEASE_DELTA: &str = "${release.RELEASE_DELTA_FILE}";`), 'the plan tool writes a file of that name');
});
