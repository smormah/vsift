'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { checkBlocks, makeSandbox, rememberIdentifiers, substitute } = require('../lib/runner.cjs');

const SESSION = 'ses_0123456789abcdef0123456789abcdef';
const SEGMENT = 'tsg_fedcba9876543210fedcba9876543210';
const FRAME = 'evd_aaaabbbbccccddddeeeeffff00001111';
const CROP = 'evd_11110000ffffeeeeddddccccbbbbaaaa';

/** A stand-in for `vsift`: what each command prints, and the status it ends with. */
function fakeVsift(table) {
  const calls = [];
  const run = (args) => {
    calls.push(args.join(' '));
    const answer = table[args.join(' ')];
    if (answer === undefined) return { status: 2, stdout: '', stderr: `no answer for ${args.join(' ')}\n`, output: `no answer for ${args.join(' ')}\n` };
    const [status, text] = answer;
    return { status, stdout: text, stderr: '', output: text };
  };
  return { run, calls };
}

function sandbox() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'guide-runner-test-'));
  return { root, sandbox: makeSandbox(root) };
}

function repositoryWith(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'guide-repo-test-'));
  for (const [name, text] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), text);
  }
  return root;
}

const PAGE = [
  '<!-- check -->',
  '```console',
  '$ vsift ingest ./clip.mp4',
  `Opened session ${SESSION.slice(0, 12)}…`,
  '$ vsift search <session> --query "order 1017"',
  `${SEGMENT.slice(0, 12)}…  00:00:00.775000 --> 00:00:02.850000`,
  '```',
].join('\n');

test('commands run in order with the identifiers earlier output printed, and output is compared by kind', () => {
  const { run, calls } = fakeVsift({
    'ingest ./clip.mp4': [0, `Opened session ${SESSION}\n`],
    [`search ${SESSION} --query order 1017`]: [0, `${SEGMENT}  00:00:00.775000 --> 00:00:02.850000\n`],
  });
  const env = sandbox();
  const repository = repositoryWith({ 'fixtures/corpus/generated/clip.mp4': 'bytes' });
  const result = checkBlocks('page.md', PAGE, { repository, binary: 'unused', sandbox: env.sandbox, speechReady: true, run });
  assert.deepEqual(result.failures, []);
  assert.equal(result.ran, 2);
  assert.deepEqual(calls, ['ingest ./clip.mp4', `search ${SESSION} --query order 1017`]);
  // The file the command named was copied in from the corpus.
  assert.ok(fs.existsSync(path.join(env.sandbox.dirs.cwd, 'clip.mp4')));
});

test('a page that shows the wrong output, or expects the wrong status, fails with the real output', () => {
  const { run } = fakeVsift({
    'ingest ./clip.mp4': [0, `Opened session ${SESSION}\n`],
    [`search ${SESSION} --query order 1017`]: [0, `${SEGMENT}  00:00:09.000000 --> 00:00:10.000000\n`],
  });
  const env = sandbox();
  const result = checkBlocks('page.md', PAGE, { repository: repositoryWith({}), binary: 'unused', sandbox: env.sandbox, speechReady: true, run });
  assert.equal(result.failures.length, 1);
  assert.match(result.failures[0], /page\.md:5: .*printed something other than the page shows/);
  assert.match(result.failures[0], /00:00:09\.000000/);

  const failing = fakeVsift({ 'ingest ./clip.mp4': [7, 'Error: Storage or output I/O prevented completion. (STORAGE_IO)\n'] });
  const second = checkBlocks('page.md', PAGE, { repository: repositoryWith({}), binary: 'unused', sandbox: sandbox().sandbox, speechReady: true, run: failing.run });
  assert.match(second.failures[0], /exited 7, the page expects 0/);
  // After a failure the rest of the page does not run on a state that is not what it expects.
  assert.equal(second.ran, 1);
});

test('an example of a failure names its exit status, and needs-speech examples are skipped when speech is not ready', () => {
  const page = [
    '<!-- check: exit 3 -->',
    '```console',
    '$ vsift ingest ./x.mp4',
    'Error: The source is invalid or unsupported. (INVALID_SOURCE)',
    '```',
    '<!-- check: needs speech -->',
    '```console',
    '$ vsift transcript retranscribe <session>',
    '```',
  ].join('\n');
  const { run, calls } = fakeVsift({ 'ingest ./x.mp4': [3, 'Error: The source is invalid or unsupported. (INVALID_SOURCE)\n'] });
  const result = checkBlocks('p.md', page, { repository: repositoryWith({}), binary: 'unused', sandbox: sandbox().sandbox, speechReady: false, run });
  assert.deepEqual(result.failures, []);
  assert.equal(result.ran, 1);
  assert.equal(result.skipped, 1);
  assert.deepEqual(calls, ['ingest ./x.mp4']);
});

test('a placeholder with no value yet, shell syntax and a program that is not vsift are failures, not guesses', () => {
  const failures = (text) =>
    checkBlocks('p.md', `<!-- check -->\n\`\`\`console\n$ ${text}\n\`\`\`\n`, { repository: repositoryWith({}), binary: 'unused', sandbox: sandbox().sandbox, speechReady: true, run: fakeVsift({}).run }).failures;
  assert.match(failures('vsift transcript get <session> --from 0 --to 1')[0], /placeholder <session> has no value yet/);
  assert.match(failures('vsift search a | head')[0], /uses shell syntax/);
  assert.match(failures('ls -l')[0], /starts with vsift/);
});

test('placeholders name the first identifier of a kind, an index picks another, and labelled commands set their own', () => {
  const known = new Map();
  rememberIdentifiers(`${SESSION}\nvcd_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1 vcd_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa2 vcd_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa3`, known);
  assert.equal(substitute('vsift frame get <session> --candidate <candidate:3>', known), `vsift frame get ${SESSION} --candidate vcd_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa3`);
  assert.equal(substitute('<candidate>', known), 'vcd_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1');
  assert.equal(substitute('<unknown> <1>', known), '<unknown> <1>');
  assert.throws(() => substitute('<revision>', known), /no value yet/);

  const { run } = fakeVsift({
    [`frame get ${SESSION} --at 1`]: [0, `Frame of session ${SESSION}\n  ${FRAME} frame at 00:00:01\n`],
    [`crop ${SESSION} ${FRAME} --rect 0,0,4,4`]: [0, `Crop of ${FRAME} in session ${SESSION}\n  ${CROP} crop 4x4\n`],
    [`frame neighbours ${SESSION} ${CROP}`]: [0, 'ok\n'],
  });
  const page = [
    '<!-- check -->',
    '```console',
    `$ vsift frame get ${SESSION} --at 1`,
    '$ vsift crop ' + SESSION + ' <frame> --rect 0,0,4,4',
    '$ vsift frame neighbours ' + SESSION + ' <crop>',
    '```',
  ].join('\n');
  const result = checkBlocks('p.md', page, { repository: repositoryWith({}), binary: 'unused', sandbox: sandbox().sandbox, speechReady: true, run });
  assert.deepEqual(result.failures, []);
  assert.equal(result.ran, 3);
});
