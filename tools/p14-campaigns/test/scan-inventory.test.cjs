'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const inventory = require('../scan-inventory.cjs');

const metadata = {
  packages: [
    { id: 'root', name: 'vsift-cli', version: '0.1.0' },
    { id: 'a', name: 'alpha', version: '1.0.0' },
    { id: 'b', name: 'beta', version: '2.0.0' },
    { id: 'c', name: 'gamma-dev', version: '3.0.0' },
    { id: 'd', name: 'delta-build', version: '4.0.0' },
  ],
  resolve: {
    nodes: [
      { id: 'root', deps: [{ pkg: 'a', dep_kinds: [{ kind: null }] }, { pkg: 'c', dep_kinds: [{ kind: 'dev' }] }, { pkg: 'd', dep_kinds: [{ kind: 'build' }] }] },
      { id: 'a', deps: [{ pkg: 'b', dep_kinds: [{ kind: null }] }] },
      { id: 'b', deps: [] },
      { id: 'c', deps: [] },
      { id: 'd', deps: [] },
    ],
  },
};

test('the resolved graph holds the normal and build dependencies, not the dev ones', () => {
  assert.deepEqual([...inventory.resolved(metadata, 'vsift-cli')].sort(), ['alpha 1.0.0', 'beta 2.0.0', 'delta-build 4.0.0', 'vsift-cli 0.1.0']);
  assert.throws(() => inventory.resolved(metadata, 'nothing'), /no package/);
});

test('the comparison names what each side lacks', () => {
  const sbom = { components: [{ type: 'library', name: 'alpha', version: '1.0.0' }, { type: 'library', name: 'extra', version: '9.9.9' }, { type: 'file', name: 'x', version: '1' }, { type: 'application', name: 'vsift-cli', version: '0.1.0' }] };
  const result = inventory.compare(sbom, metadata, 'vsift-cli');
  assert.deepEqual(result.missingFromSbom, ['beta 2.0.0', 'delta-build 4.0.0']);
  assert.deepEqual(result.notResolved, ['extra 9.9.9']);
  assert.equal(result.sbomComponents, 3);
  assert.equal(result.resolvedCrates, 4);
});
