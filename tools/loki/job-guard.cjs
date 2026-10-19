#!/usr/bin/env node
'use strict';

// LOKI's job-started guard (ADR 0025). The runner application runs it before
// every job it is given (ACTIONS_RUNNER_HOOK_JOB_STARTED, through
// job-started.ps1), and a refusal fails the job before any of its steps runs.
//
// It is the run-time half of the self-hosted runner policy. The governance lint
// checks the workflows a change merges, but a pull request's own copy of a
// workflow runs before anyone merges it; this guard lives on LOKI, outside the
// repository's reach, and refuses any job that is not the overnight workflow of
// smormah/vsift started by its schedule or by a dispatch on a branch.
//
// The maintainer copies this file and job-started.ps1 from a reviewed commit to
// a folder outside the runner's application and work folders
// (docs/operations/loki-runner.md). It depends on nothing but Node.js.

/** The repository whose jobs LOKI runs. */
const REPOSITORY = 'smormah/vsift';

/** The one workflow allowed on LOKI. */
const WORKFLOW = `${REPOSITORY}/.github/workflows/loki-overnight.yml`;

/** The events that may start it. */
const EVENTS = Object.freeze(['schedule', 'workflow_dispatch']);

/** A branch: a dispatch may name one; never a tag or a pull request's ref. */
const BRANCH_PREFIX = 'refs/heads/';

/**
 * Whether the job described by the runner's default variables may run.
 * Fail-closed: a variable that is missing refuses the job.
 * @param {Record<string, string|undefined>} env
 * @returns {{ ok: boolean, reason: string }}
 */
function decide(env) {
  const repository = env.GITHUB_REPOSITORY;
  const event = env.GITHUB_EVENT_NAME;
  const ref = env.GITHUB_REF;
  const workflowRef = env.GITHUB_WORKFLOW_REF;
  if (repository !== REPOSITORY) {
    return { ok: false, reason: `the job is from ${repository || 'an unnamed repository'}, not ${REPOSITORY}` };
  }
  if (!EVENTS.includes(event)) {
    return { ok: false, reason: `the job was started by \`${event || 'an unnamed event'}\`; LOKI runs only ${EVENTS.join(' and ')}` };
  }
  if (typeof ref !== 'string' || !ref.startsWith(BRANCH_PREFIX) || ref.length === BRANCH_PREFIX.length) {
    return { ok: false, reason: `the job runs on \`${ref || 'no ref'}\`; LOKI runs only branches` };
  }
  if (workflowRef !== `${WORKFLOW}@${ref}`) {
    return { ok: false, reason: `the workflow is \`${workflowRef || 'unnamed'}\`; LOKI runs only ${WORKFLOW} from the run's own branch` };
  }
  return { ok: true, reason: `${WORKFLOW} on ${ref}, started by ${event}` };
}

if (require.main === module) {
  const decision = decide(process.env);
  // The names in the message come from GitHub's own variables; the log is the
  // job's "Set up runner" step.
  console.log(`LOKI job guard: ${decision.ok ? 'allowed' : 'REFUSED'}: ${decision.reason}`);
  process.exitCode = decision.ok ? 0 : 1;
}

module.exports = { REPOSITORY, WORKFLOW, EVENTS, decide };
