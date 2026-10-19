'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const { decide, WORKFLOW } = require('../job-guard.cjs');

const ALLOWED = {
  GITHUB_REPOSITORY: 'smormah/vsift',
  GITHUB_EVENT_NAME: 'schedule',
  GITHUB_REF: 'refs/heads/main',
  GITHUB_WORKFLOW_REF: `${WORKFLOW}@refs/heads/main`,
};

test('the overnight workflow on a branch, by schedule or dispatch, may run', () => {
  assert.equal(decide(ALLOWED).ok, true);
  assert.equal(decide({
    ...ALLOWED,
    GITHUB_EVENT_NAME: 'workflow_dispatch',
    GITHUB_REF: 'refs/heads/fix/443-root-race',
    GITHUB_WORKFLOW_REF: `${WORKFLOW}@refs/heads/fix/443-root-race`,
  }).ok, true);
});

test('every other job is refused before its first step', () => {
  for (const change of [
    { GITHUB_EVENT_NAME: 'pull_request' },
    { GITHUB_EVENT_NAME: 'pull_request_target' },
    { GITHUB_EVENT_NAME: 'push' },
    { GITHUB_EVENT_NAME: 'workflow_run' },
    { GITHUB_EVENT_NAME: 'issue_comment' },
    { GITHUB_EVENT_NAME: undefined },
    { GITHUB_REPOSITORY: 'someone/vsift' },
    { GITHUB_REPOSITORY: undefined },
    { GITHUB_REF: 'refs/pull/7/merge', GITHUB_WORKFLOW_REF: `${WORKFLOW}@refs/pull/7/merge` },
    { GITHUB_REF: 'refs/tags/v0.2.2', GITHUB_WORKFLOW_REF: `${WORKFLOW}@refs/tags/v0.2.2` },
    { GITHUB_REF: 'refs/heads/', GITHUB_WORKFLOW_REF: `${WORKFLOW}@refs/heads/` },
    { GITHUB_WORKFLOW_REF: 'smormah/vsift/.github/workflows/ci.yml@refs/heads/main' },
    { GITHUB_WORKFLOW_REF: 'someone/vsift/.github/workflows/loki-overnight.yml@refs/heads/main' },
    { GITHUB_WORKFLOW_REF: `${WORKFLOW}@refs/heads/other` },
    { GITHUB_WORKFLOW_REF: undefined },
  ]) {
    const decision = decide({ ...ALLOWED, ...change });
    assert.equal(decision.ok, false, JSON.stringify(change));
    assert.ok(decision.reason.length > 0);
  }
});
