'use strict';

// The success rule of RQ-09 (docs/planning/p14-qualification.md section 2;
// verification.md section 5 "Suggested gates"): each phase of the load
// campaign is judged on the measurements it recorded. The thresholds are the
// plan's; throughput is reported as a measurement and never gated.

const THRESHOLDS = Object.freeze({
  coordinatorRssMiB: 256,
  descendantGraceMs: 10_000,
  warmPageP95Ms: 250,
});

/**
 * @typedef {{ name: string, ok: boolean, detail: string }} Check
 */

/** @returns {Check} */
function check(name, ok, detail) {
  return { name, ok: Boolean(ok), detail };
}

/**
 * Checks of one container run's resource curve.
 * @param {string} label
 * @param {ReturnType<import('./cgroup-sampler.cjs').summariseRun>|null} curve
 * @returns {Check[]}
 */
function resourceChecks(label, curve) {
  if (!curve || curve.samples === 0) {
    return [check(`${label}: resource curve`, false, 'the sampler recorded nothing for this run')];
  }
  const rss = curve.coordinatorRssPeakMiB;
  return [
    check(
      `${label}: coordinator memory at most ${THRESHOLDS.coordinatorRssMiB} MiB`,
      rss !== null && rss <= THRESHOLDS.coordinatorRssMiB,
      `peak ${rss} MiB over ${curve.samples} samples (${curve.seconds} s); the container peaked at ${curve.containerMemoryPeakMiB} MiB with its providers`,
    ),
    check(
      `${label}: no monotonic memory growth after warm-up`,
      !curve.coordinatorRssGrowth.monotonic,
      `${curve.coordinatorRssGrowth.reason}; window maxima ${JSON.stringify(curve.coordinatorRssGrowth.maxima.map((value) => Math.round(value)))} MiB`,
    ),
    check(
      `${label}: no monotonic descriptor growth after warm-up`,
      !curve.coordinatorFdsGrowth.monotonic,
      `${curve.coordinatorFdsGrowth.reason}; peak ${curve.coordinatorFdsPeak} descriptors`,
    ),
  ];
}

/**
 * Checks of one batch's events and outputs.
 * @param {string} label
 * @param {{ problems: string[], leaks: string[], batch: object, expectedConcurrency: number, expectedCapacity: number }} run
 * @returns {Check[]}
 */
function streamChecks(label, run) {
  const readiness = run.batch.started || {};
  return [
    check(`${label}: the stream keeps its rules`, run.problems.length === 0, run.problems.slice(0, 3).join('; ') || 'ordered, bounded, ready first'),
    check(`${label}: strict isolation attested`, readiness.isolation === 'strict_linux', `isolation ${readiness.isolation}`),
    check(
      `${label}: concurrency and capacity as set`,
      readiness.concurrency === run.expectedConcurrency && readiness.admission_capacity === run.expectedCapacity,
      `concurrency ${readiness.concurrency}, capacity ${readiness.admission_capacity}`,
    ),
    check(`${label}: never more requests running than the concurrency`, run.batch.peakRunning <= run.expectedConcurrency, `peak ${run.batch.peakRunning} of ${run.expectedConcurrency}`),
    check(`${label}: no path and no sentinel in any output`, run.leaks.length === 0, run.leaks.length ? `found ${JSON.stringify(run.leaks)}` : 'none found'),
  ];
}

/**
 * The page-latency check.
 * @param {number[]} millis
 * @param {number} p95
 */
function pageCheck(millis, p95) {
  return check(
    `a warm candidate page, p95 at most ${THRESHOLDS.warmPageP95Ms} ms`,
    millis.length >= 100 && p95 <= THRESHOLDS.warmPageP95Ms,
    `${millis.length} calls through the binary, p95 ${p95} ms (each includes starting the process)`,
  );
}

/** Whether every check of every phase held. */
function allHold(phases) {
  return phases.every((phase) => phase.checks.every((entry) => entry.ok));
}

/** Markdown of the phases: a table of checks per phase. */
function renderMarkdown(title, phases) {
  const lines = [`## ${title}`, ''];
  for (const phase of phases) {
    const held = phase.checks.every((entry) => entry.ok);
    lines.push(`### ${phase.name}: ${held ? 'held' : 'DID NOT HOLD'}`, '');
    if (phase.measurements) lines.push(...phase.measurements, '');
    lines.push('| Check | Result | Detail |', '| --- | --- | --- |');
    for (const entry of phase.checks) lines.push(`| ${entry.name} | ${entry.ok ? 'pass' : '**FAIL**'} | ${String(entry.detail).replace(/\|/g, '\\|')} |`);
    lines.push('');
  }
  return lines.join('\n');
}

module.exports = { THRESHOLDS, check, resourceChecks, streamChecks, pageCheck, allHold, renderMarkdown };
