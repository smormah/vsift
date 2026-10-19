'use strict';

// The `repro` suite of the repetition runner (stress.cjs): one named test, run
// many times, several copies at once, optionally with every CPU kept busy. It is
// how a rare failure seen once under load gets a rate (#439, #443): the overnight
// machine LOKI runs it (docs/operations/loki-runner.md), never a developer's
// machine. Everything here is pure, so it is tested without running cargo.

/** The bounds of the repro options. */
const LIMITS = Object.freeze({
  copiesMax: 8,
  burnersMax: 4,
  filterLength: 200,
});

/** How libtest matches the filter, and which tests it may run. */
const MATCHES = Object.freeze(['exact', 'substring']);
const IGNORED = Object.freeze(['no', 'only', 'include']);

const PACKAGE = /^[a-z][a-z0-9-]*$/;
const TARGET = /^[A-Za-z0-9_]+$/;
const FILTER = /^[A-Za-z0-9_:]+$/;

class ReproOptionError extends Error {
  constructor(message) {
    super(message);
    this.name = 'ReproOptionError';
  }
}

/**
 * Checks the repro options and returns them normalised. The values come from a
 * manual dispatch; they are passed to cargo and the test binary as separate
 * arguments, never through a shell, but they are still held to the shapes a
 * package, a test target and a test path have.
 * @param {{ package?: string, target?: string, filter?: string, match?: string,
 *   ignored?: string, copies?: number, burners?: number }} options
 */
function validateRepro(options) {
  const repro = {
    package: options.package,
    target: options.target,
    filter: options.filter,
    match: options.match === undefined ? 'exact' : options.match,
    ignored: options.ignored === undefined ? 'no' : options.ignored,
    copies: options.copies === undefined ? 1 : options.copies,
    burners: options.burners === undefined ? 0 : options.burners,
  };
  if (typeof repro.package !== 'string' || !PACKAGE.test(repro.package)) {
    throw new ReproOptionError('--package must name a workspace package (lower-case letters, digits and hyphens)');
  }
  if (typeof repro.target !== 'string' || !(repro.target === 'lib' || TARGET.test(repro.target))) {
    throw new ReproOptionError('--target must be `lib` or the name of an integration-test binary');
  }
  if (typeof repro.filter !== 'string' || !FILTER.test(repro.filter) || repro.filter.length > LIMITS.filterLength) {
    throw new ReproOptionError(`--filter must be a test path of letters, digits, \`_\` and \`::\`, at most ${LIMITS.filterLength} characters`);
  }
  if (!MATCHES.includes(repro.match)) throw new ReproOptionError(`--match must be one of ${MATCHES.join(', ')}`);
  if (!IGNORED.includes(repro.ignored)) throw new ReproOptionError(`--ignored must be one of ${IGNORED.join(', ')}`);
  if (!Number.isInteger(repro.copies) || repro.copies < 1 || repro.copies > LIMITS.copiesMax) {
    throw new ReproOptionError(`--copies must be an integer from 1 to ${LIMITS.copiesMax}`);
  }
  if (!Number.isInteger(repro.burners) || repro.burners < 0 || repro.burners > LIMITS.burnersMax) {
    throw new ReproOptionError(`--burners must be an integer from 0 to ${LIMITS.burnersMax}`);
  }
  return repro;
}

/**
 * The cargo arguments that build the test binary once and describe it on
 * standard output as JSON messages.
 * @param {ReturnType<typeof validateRepro>} repro
 */
function buildArguments(repro) {
  const selection = repro.target === 'lib' ? ['--lib'] : ['--test', repro.target];
  return ['test', '--locked', '--no-run', '--message-format=json-render-diagnostics', '-p', repro.package, ...selection];
}

/**
 * The test binary cargo built, from its JSON messages: the one test-profile
 * artifact with an executable of the asked target.
 * @param {string} stdout cargo's standard output
 * @param {ReturnType<typeof validateRepro>} repro
 * @returns {{ executable: string, manifestPath: string }}
 */
function testExecutable(stdout, repro) {
  const found = [];
  for (const line of String(stdout).split(/\r?\n/)) {
    if (!line.startsWith('{')) continue;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      continue;
    }
    if (message.reason !== 'compiler-artifact' || !message.executable) continue;
    if (!message.profile || message.profile.test !== true || !message.target) continue;
    const kinds = Array.isArray(message.target.kind) ? message.target.kind : [];
    const wanted = repro.target === 'lib'
      ? kinds.some((kind) => kind === 'lib' || kind === 'rlib')
      : kinds.includes('test') && message.target.name === repro.target;
    if (wanted) found.push({ executable: message.executable, manifestPath: message.manifest_path });
  }
  if (found.length !== 1) {
    throw new ReproOptionError(`cargo described ${found.length} test binaries for ${repro.package} ${repro.target}; expected exactly one`);
  }
  return found[0];
}

/**
 * The arguments of one run of the test binary.
 * @param {ReturnType<typeof validateRepro>} repro
 */
function testArguments(repro) {
  const args = [repro.filter];
  if (repro.match === 'exact') args.push('--exact');
  if (repro.ignored === 'only') args.push('--ignored');
  if (repro.ignored === 'include') args.push('--include-ignored');
  return args;
}

/** A one-line description of the repro suite for the record. */
function describe(repro) {
  const ignored = repro.ignored === 'no' ? '' : `, ignored tests: ${repro.ignored}`;
  const load = repro.burners > 0 ? `, ${repro.burners} busy process(es) per CPU` : '';
  return `Repro: ${repro.package} ${repro.target} ${repro.filter} (${repro.match}${ignored}), ${repro.copies} at once${load}`;
}

module.exports = {
  LIMITS, MATCHES, IGNORED, ReproOptionError, validateRepro, buildArguments, testExecutable, testArguments, describe,
};
