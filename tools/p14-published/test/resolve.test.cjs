'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');

const { FIRST_RELEASE, chooseVersions } = require('../resolve-release.cjs');

test('with no input the highest published version is qualified against the first release', () => {
  assert.deepEqual(chooseVersions({ inputVersion: '', inputFromVersion: undefined, published: ['0.0.0', '0.1.0'] }), {
    version: '0.1.0',
    fromVersion: FIRST_RELEASE,
  });
  assert.equal(chooseVersions({ inputVersion: '', inputFromVersion: '', published: ['0.0.0', '0.1.0', '0.2.0-rc.1', '0.2.0-rc.2'] }).version, '0.2.0-rc.2');
  assert.equal(chooseVersions({ inputVersion: '', inputFromVersion: '', published: ['0.0.0', '0.1.0', '0.2.0-rc.1', '0.2.0'] }).version, '0.2.0');
});

test('inputs are used when given, and must be published and plain versions', () => {
  const published = ['0.0.0', '0.1.0', '0.2.0-rc.1'];
  const chosen = chooseVersions({ inputVersion: ' 0.1.0 ', inputFromVersion: '0.0.0', published });
  assert.equal(chosen.version, '0.1.0');
  assert.equal(chosen.fromVersion, '0.0.0');
  assert.throws(() => chooseVersions({ inputVersion: '0.3.0', inputFromVersion: '', published }), /is not published/);
  assert.throws(() => chooseVersions({ inputVersion: '', inputFromVersion: '0.0.9', published }), /is not published/);
  for (const hostile of ['latest', 'next', '0.1.0; rm -rf /', '$(id)', '../0.1.0', '0.1.0\n0.2.0', '^0.1.0', '0.1']) {
    assert.throws(() => chooseVersions({ inputVersion: hostile, inputFromVersion: '', published }), /not a version/, JSON.stringify(hostile));
    assert.throws(() => chooseVersions({ inputVersion: '', inputFromVersion: hostile, published }), /not a version/, JSON.stringify(hostile));
  }
  assert.throws(() => chooseVersions({ inputVersion: '', inputFromVersion: '', published: [] }), /lists no version/);
});
