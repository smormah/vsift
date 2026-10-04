'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ancestry = require('../ffmpeg-ancestry.cjs');

const FULL = 'fd3ee52fab34d98a95b787d0b5ff45685766200c';
const OTHER = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const FIXTURES = path.join(__dirname, 'fixtures', 'ffmpeg-ancestry');

test('a record names its fix commits in all three URL forms, abbreviated or not, and nothing else', () => {
  const cve = {
    references: [
      { url: `https://code.ffmpeg.org/FFmpeg/FFmpeg/commit/${FULL}` },
      { url: 'https://code.ffmpeg.org/FFmpeg/FFmpeg/pulls/23773' },
      { url: 'https://git.ffmpeg.org/gitweb/ffmpeg.git/commit/8439e0203744a30d280668fcd086f74ed5001da1' },
      { url: 'https://github.com/FFmpeg/FFmpeg/commit/8970658472d1' },
      { url: `https://code.ffmpeg.org/FFmpeg/FFmpeg/commit/${FULL}` },
      { url: `https://github.com/someone/FFmpeg/commit/${OTHER}` },
      { url: 'https://github.com/FFmpeg/FFmpeg/commit/abc12' },
      { url: 'https://example.com/FFmpeg/FFmpeg/commit/bbbbbbbbbbbb' },
    ],
  };
  assert.deepEqual(ancestry.fixCommits(cve), [FULL, '8439e0203744a30d280668fcd086f74ed5001da1', '8970658472d1']);
  assert.deepEqual(ancestry.fixCommits({}), []);
});

test('a cherry-pick line names the commit it came from, and a revert line the commit it undoes', () => {
  const message = 'avcodec/tdsc: unref the reference frame\n\nSigned-off-by: someone\n(cherry picked from commit fd3ee52fab34d98a95b787d0b5ff45685766200c)\n(cherry picked from commit 0123456)\n';
  assert.deepEqual(ancestry.cherryPicks(message), [FULL, '0123456']);
  assert.deepEqual(ancestry.cherryPicks('no trailer'), []);
  assert.deepEqual(ancestry.cherryPicks('cherry picked from commit fd3ee52'), []);
  assert.deepEqual(ancestry.reverts(`Revert "x"\n\nThis reverts commit ${FULL}.\n`), [FULL]);
  assert.deepEqual(ancestry.reverts('reverts nothing'), []);
});

test('an abbreviated hash matches the full one, but a very short prefix matches nothing', () => {
  assert.equal(ancestry.sameCommit(FULL, FULL.slice(0, 10)), true);
  assert.equal(ancestry.sameCommit(FULL.slice(0, 7), FULL), true);
  assert.equal(ancestry.sameCommit(FULL, OTHER), false);
  assert.equal(ancestry.sameCommit(FULL, FULL.slice(0, 6)), false);
});

test('ancestor beats cherry-pick, a trailer is found, a later revert is seen, and none of them is "absent"', () => {
  const pick = 'b'.repeat(40);
  const revert = 'c'.repeat(40);
  const branchOnly = [
    { sha: pick, date: '2026-07-21', picks: [FULL], reverts: [] },
    { sha: 'd'.repeat(40), date: '2026-08-12', picks: [], reverts: [] },
  ];
  assert.deepEqual(ancestry.classify(FULL, 'ahead', branchOnly), { state: 'ancestor' });
  assert.deepEqual(ancestry.classify(FULL, 'identical', []), { state: 'ancestor' });
  assert.deepEqual(ancestry.classify(FULL, 'diverged', branchOnly), { state: 'cherry-pick', commit: pick, date: '2026-07-21' });
  assert.deepEqual(ancestry.classify(OTHER, 'diverged', branchOnly), { state: 'absent' });
  assert.deepEqual(ancestry.classify(OTHER, 'behind', branchOnly), { state: 'absent' });
  const withRevert = [...branchOnly, { sha: revert, date: '2026-09-02', picks: [], reverts: [pick.slice(0, 12)] }];
  assert.deepEqual(ancestry.classify(FULL, 'diverged', withRevert), { state: 'reverted', commit: pick, date: '2026-07-21', revertedBy: revert });
  // A revert that comes before the pick (oldest first) does not undo it.
  const earlier = [{ sha: revert, date: '2026-07-01', picks: [], reverts: [pick] }, ...branchOnly];
  assert.equal(ancestry.classify(FULL, 'diverged', earlier).state, 'cherry-pick');
});

test('a patch is compared by its files and hunks with the line numbers dropped', () => {
  const file = (name, patch) => ({ filename: name, patch });
  const fix = { files: [file('b.c', '@@ -1,2 +1,2 @@ int f(void)\n-a\n+b\n'), file('a.c', '@@ -5,1 +5,1 @@\n-x\n+y\n')] };
  const moved = { files: [file('a.c', '@@ -50,1 +50,1 @@\n-x\n+y\n'), file('b.c', '@@ -9,2 +9,2 @@ int f(void)\n-a\n+b\n')] };
  const changed = { files: [file('a.c', '@@ -5,1 +5,1 @@\n-x\n+z\n'), file('b.c', '@@ -1,2 +1,2 @@ int f(void)\n-a\n+b\n')] };
  const fewer = { files: [file('a.c', '@@ -5,1 +5,1 @@\n-x\n+y\n')] };
  assert.equal(ancestry.comparePatches(fix, moved), 'identical');
  assert.equal(ancestry.comparePatches(fix, changed), 'differs');
  assert.equal(ancestry.comparePatches(fix, fewer), 'differs');
  assert.equal(ancestry.comparePatches(fix, { files: [{ filename: 'a.c' }] }), 'unknown');
  assert.equal(ancestry.comparePatches(fix, {}), 'unknown');
});

/** A fake of the GitHub endpoints the reading uses, from a recorded-shape fixture: no network. */
function fixtureApi(responses) {
  return (request) => {
    if (!(request in responses)) throw new Error(`No fixture for ${request}`);
    return responses[request];
  };
}

test('records are read against two snapshots from fixtures, with every kind of outcome and no network', () => {
  const records = ancestry.loadRecords(path.join(FIXTURES, 'records.json'));
  const responses = JSON.parse(fs.readFileSync(path.join(FIXTURES, 'github.json'), 'utf8'));
  const results = ancestry.readRecords(records, { shipped: 'shipped', candidate: 'candidate' }, fixtureApi(responses));
  assert.deepEqual(ancestry.summary(results, ['shipped', 'candidate']), [
    'CVE-2000-0001 a1a1a1a1a1 shipped=cherry-pick(patch=identical) candidate=cherry-pick(patch=identical)',
    'CVE-2000-0002 a2a2a2a2a2 shipped=ancestor candidate=ancestor',
    'CVE-2000-0003 a3a3a3a3a3 shipped=cherry-pick(patch=differs) candidate=reverted',
    'CVE-2000-0004 unreferenced (read it by hand)',
    'CVE-2000-0005 a5a5a5a5a5 not on the mirror',
    'CVE-2000-0006 a6a6a6a6a6 shipped=absent candidate=absent',
  ]);
  assert.equal(results[0].fixes[0].snapshots.shipped.commit, 'b1'.repeat(20));
  assert.equal(results[2].fixes[0].snapshots.candidate.revertedBy, 'c3'.repeat(20));
  assert.equal(results[3].unreferenced, true);
});

test('a saved response and a plain array of records load the same way', () => {
  const fromResponse = ancestry.loadRecords(path.join(FIXTURES, 'records.json'));
  const directory = fs.mkdtempSync(path.join(require('node:os').tmpdir(), 'ancestry-'));
  try {
    const plain = path.join(directory, 'plain.json');
    fs.writeFileSync(plain, JSON.stringify(fromResponse));
    assert.deepEqual(ancestry.loadRecords(plain), fromResponse);
    const keyed = path.join(directory, 'keyed.json');
    fs.writeFileSync(keyed, JSON.stringify(Object.fromEntries(fromResponse.map((cve) => [cve.id, cve]))));
    assert.deepEqual(ancestry.loadRecords(keyed), fromResponse);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true, maxRetries: 3, retryDelay: 50 });
  }
});

test('the branch listing is followed across pages until a short page ends it', () => {
  const calls = [];
  const full = Array.from({ length: 100 }, (_, index) => ({ sha: String(index).padStart(40, '0'), commit: { committer: { date: '2026-07-01T00:00:00Z' }, message: 'm' } }));
  const api = (request) => {
    calls.push(request);
    return { commits: calls.length === 1 ? full : full.slice(0, 5) };
  };
  assert.equal(ancestry.branchOnlyCommits(api, 'abc1234').length, 105);
  assert.equal(calls.length, 2);
});
