'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const {
  validateRepro, buildArguments, testExecutable, testArguments, describe, ReproOptionError,
} = require('../lib/repro.cjs');
const { parse } = require('../stress.cjs');

const GOOD = {
  package: 'vsift-infrastructure',
  target: 'session_root_provisioning',
  filter: 'concurrent_processes_create_one_root_and_every_one_adopts_it',
};

test('the repro options default to one exact copy of a non-ignored test, without load', () => {
  assert.deepEqual(validateRepro(GOOD), { ...GOOD, match: 'exact', ignored: 'no', copies: 1, burners: 0 });
  assert.deepEqual(
    validateRepro({ ...GOOD, target: 'lib', filter: 'kill::installs_killed_at_spread_moments', copies: 8, burners: 4, ignored: 'include', match: 'substring' }).copies,
    8,
  );
});

test('repro options of the wrong shape are refused, not passed on', () => {
  for (const bad of [
    { package: 'Vsift' },
    { package: 'vsift; rm -rf /' },
    { package: undefined },
    { target: '../x' },
    { target: '' },
    { filter: 'a b' },
    { filter: '--ignored' },
    { filter: 'x'.repeat(201) },
    { filter: '' },
    { match: 'regex' },
    { ignored: 'yes' },
    { copies: 0 },
    { copies: 9 },
    { copies: 1.5 },
    { burners: -1 },
    { burners: 5 },
  ]) {
    assert.throws(() => validateRepro({ ...GOOD, ...bad }), ReproOptionError, JSON.stringify(bad));
  }
});

test('the build is one cargo test --no-run of the named target, described as JSON', () => {
  const repro = validateRepro(GOOD);
  assert.deepEqual(buildArguments(repro), [
    'test', '--locked', '--no-run', '--message-format=json-render-diagnostics',
    '-p', 'vsift-infrastructure', '--test', 'session_root_provisioning',
  ]);
  assert.deepEqual(buildArguments(validateRepro({ ...GOOD, target: 'lib' })).slice(-1), ['--lib']);
});

test('the test binary is the one test-profile artifact of the asked target', () => {
  const artifact = (name, kind, test, executable) => JSON.stringify({
    reason: 'compiler-artifact',
    manifest_path: '/repo/crates/vsift-infrastructure/Cargo.toml',
    target: { name, kind: [kind] },
    profile: { test },
    executable,
  });
  const stdout = [
    'not json',
    artifact('serde', 'lib', false, null),
    artifact('vsift_infrastructure', 'lib', false, null),
    artifact('vsift_infrastructure', 'lib', true, '/repo/target/debug/deps/vsift_infrastructure-1'),
    artifact('session_root_provisioning', 'test', true, '/repo/target/debug/deps/session_root_provisioning-2'),
    '{"reason":"build-finished","success":true}',
  ].join('\n');
  assert.deepEqual(testExecutable(stdout, validateRepro(GOOD)), {
    executable: '/repo/target/debug/deps/session_root_provisioning-2',
    manifestPath: '/repo/crates/vsift-infrastructure/Cargo.toml',
  });
  assert.equal(
    testExecutable(stdout, validateRepro({ ...GOOD, target: 'lib' })).executable,
    '/repo/target/debug/deps/vsift_infrastructure-1',
  );
  assert.throws(() => testExecutable(stdout, validateRepro({ ...GOOD, target: 'other' })), /described 0 test binaries/);
  assert.throws(() => testExecutable(`${stdout}\n${stdout}`, validateRepro(GOOD)), /described 2 test binaries/);
});

test('each run passes the filter and the libtest switches the options ask for', () => {
  assert.deepEqual(testArguments(validateRepro(GOOD)), [GOOD.filter, '--exact']);
  assert.deepEqual(testArguments(validateRepro({ ...GOOD, match: 'substring', ignored: 'only' })), [GOOD.filter, '--ignored']);
  assert.deepEqual(testArguments(validateRepro({ ...GOOD, ignored: 'include' })), [GOOD.filter, '--exact', '--include-ignored']);
  assert.match(describe(validateRepro({ ...GOOD, copies: 4, burners: 1 })), /4 at once, 1 busy process\(es\) per CPU/);
});

test('the runner reads the repro suite, budget runs and refuses stray options', () => {
  const options = parse([
    '--suite', 'repro', '--runs', '1000', '--out', 'o', '--package', GOOD.package, '--target', GOOD.target,
    '--filter', GOOD.filter, '--copies', '4', '--burners', '1',
  ]);
  assert.equal(options.repro.copies, 4);
  assert.equal(options.until, 'runs');
  assert.equal(options.package, undefined);
  assert.equal(parse(['--suite', 'batch-277', '--runs', '5', '--out', 'o', '--until', 'budget']).until, 'budget');
  assert.throws(() => parse(['--suite', 'locks', '--runs', '5', '--out', 'o', '--copies', '2']), /only the repro suite takes --copies/);
  assert.throws(() => parse(['--suite', 'locks', '--runs', '5', '--out', 'o', '--until', 'forever']), /--until/);
  assert.throws(() => parse(['--suite', 'repro', '--runs', '5', '--out', 'o']), /--package/);
  assert.throws(() => parse(['--suite', 'locks', '--runs', '5', '--out']), /needs a value/);
  assert.throws(() => parse(['--suite', 'nothing', '--runs', '5', '--out', 'o']), /--suite must be one of/);
  assert.throws(() => parse(['--suite', 'constructor', '--runs', '5', '--out', 'o']), /--suite must be one of/);
});
