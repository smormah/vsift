// How a page's shown output is compared with what a real run printed (P14 PR 9b).
//
// The pages show real output, so the comparison is exact except for the values that differ on
// every run or machine and say nothing about the command: identifiers, timestamps, digests, the
// size of a frame file, the measured size of a change, a speech confidence, the path of a file
// under the user's own session folder. Each such value is replaced by a fixed word on both sides
// before the lines are compared. A page may shorten an identifier or a digest (`ses_4ad48dbf…`);
// the shortened form masks the same way. Everything else (every word, every number that
// is not on this list) must match, so a renamed field or a changed sentence fails the check.
//
// A line that is only `...` in the shown output stands for any number of lines, which is how a
// page labels trimmed output.

'use strict';

const ELLIPSIS = /^(\.\.\.|…)$/;

const IDENTIFIER = /\b(ses|src_sha256|opk_sha256|trv|tsg|sgm|vcd|vix|evd|job|op|art)_[0-9a-z]{4,}…?/g;
const TIME = /\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\b/g;
const LONG_HEX = /\b[0-9a-f]{16,}\b/g;
const SHORT_HEX = /\b[0-9a-f]{6,}…/g;
const BUILD_COMMIT = /\(([0-9a-f]{12})\)/g;
// The `vsift --version` line. Its version is a candidate's before the release and the release's
// after it, and the stable commit may not change a guide page (release process, section 6.8).
const VERSION_LINE = /^vsift \S+ (\(<commit>\))$/;
const IMAGE_BYTES = /(Image \d+x\d+), \d+ bytes/g;
const CHANGED_BLOCKS = /\d+ blocks changed, the largest by \d+/g;
const CONFIDENCE = /confidence \d+(?:\.\d+)?%/g;
// How many artifacts a session holds, how much they weigh (it depends on the encoder that made a
// frame) and its generation counter depend on which earlier commands a reader ran.
const ARTIFACT_BYTES = /Artifacts: \d+ \(\d+ bytes\)/g;
const GENERATION = /Generation: \d+/g;
// Listing and cleaning walk the session folder in hash buckets, and the next bucket depends on the ids.
const CURSOR = /--cursor \d+/g;
// A text file's size depends on the line endings the checkout wrote (Windows may convert them).
const SUPPLIED_BYTES = /(Supplied file: \w+), \d+ bytes/g;
// A delivered file's path, alone on a line below its `File (<type>):` label.
const FILE_PATH_LINE = /^ {4}\S.*[\\/].*\.(?:png|wav|json)$/;
// Notes that appear only on some systems.
const PLATFORM_NOTE = /^Note: a path starting with \\\\\?\\ /;

// What speech recognition decides: the words it hears, where it cuts them and how many pieces it
// makes. A page that shows a recognition marks its example `needs speech`, and these are masked
// too, because the recogniser may hear a clip a little differently on another processor.
const SPEECH_TEXT_LINE = /^ {2}\| .*$/;
const SPEECH_SPAN = /\d{2}:\d{2}:\d{2}\.\d{6} --> \d{2}:\d{2}:\d{2}\.\d{6}/g;
const SPEECH_COUNT = /\b(segments in all|Segments recognised now|Segments on this page|Hits on this page|segments): \d+/gi;
// The recogniser uses the machine's cores (at most eight).
const SPEECH_THREADS = /\b\d+ threads\b/g;

/** One line with its machine-dependent values replaced by fixed words. */
function maskLine(line, speech = false) {
  if (FILE_PATH_LINE.test(line)) return '    <file path>';
  if (speech && SPEECH_TEXT_LINE.test(line)) return '  | <speech>';
  return (speech ? line.replace(SPEECH_SPAN, '<span>').replace(SPEECH_COUNT, '$1: <n>').replace(SPEECH_THREADS, '<n> threads') : line)
    .replace(IDENTIFIER, (_, prefix) => `${prefix}_<id>`)
    .replace(TIME, '<time>')
    .replace(LONG_HEX, '<hex>')
    .replace(SHORT_HEX, '<hex>')
    .replace(BUILD_COMMIT, '(<commit>)')
    .replace(VERSION_LINE, 'vsift <version> $1')
    .replace(IMAGE_BYTES, '$1, <n> bytes')
    .replace(CHANGED_BLOCKS, '<n> blocks changed, the largest by <n>')
    .replace(CONFIDENCE, 'confidence <pct>')
    .replace(ARTIFACT_BYTES, 'Artifacts: <n> (<n> bytes)')
    .replace(GENERATION, 'Generation: <n>')
    .replace(CURSOR, '--cursor <n>')
    .replace(SUPPLIED_BYTES, '$1, <n> bytes')
    .replace(/[ \t]+$/, '');
}

/** The lines of an output as compared: masked, without platform-only notes and edge blank lines. */
function maskLines(lines, speech = false) {
  const masked = lines.filter((line) => !PLATFORM_NOTE.test(line)).map((line) => maskLine(line, speech));
  while (masked.length > 0 && masked[masked.length - 1] === '') masked.pop();
  while (masked.length > 0 && masked[0] === '') masked.shift();
  // Blank lines around a dropped note collapse to one.
  return masked.filter((line, index) => !(line === '' && masked[index - 1] === ''));
}

/** Whether `actual` lines match `expected` lines, where an `...` line in `expected` stands for any lines. */
function linesMatch(expected, actual) {
  const memo = new Map();
  const walk = (e, a) => {
    const key = e * 100_000 + a;
    if (memo.has(key)) return memo.get(key);
    let result;
    if (e === expected.length) result = a === actual.length;
    else if (ELLIPSIS.test(expected[e])) {
      result = false;
      for (let skip = a; skip <= actual.length && !result; skip += 1) result = walk(e + 1, skip);
    } else result = a < actual.length && expected[e] === actual[a] && walk(e + 1, a + 1);
    memo.set(key, result);
    return result;
  };
  return walk(0, 0);
}

/** A readable account of the first place two masked outputs differ. */
function explainDifference(expected, actual) {
  const plain = expected.filter((line) => !ELLIPSIS.test(line));
  for (let index = 0; index < Math.max(plain.length, actual.length); index += 1) {
    if (plain[index] !== actual[index]) {
      return `first difference at shown line ${index + 1}: the page has ${JSON.stringify(plain[index])}, the run printed ${JSON.stringify(actual[index])}`;
    }
  }
  return 'the lines match in order but an elision or the line count differs';
}

/** Compares a shown output with a real one: `{ ok, why }`; `speech` masks what recognition decides. */
function compareText(shownLines, actualText, speech = false) {
  const expected = maskLines(shownLines, speech);
  const actual = maskLines(actualText.replace(/\r\n/g, '\n').split('\n'), speech);
  if (linesMatch(expected, actual)) return { ok: true };
  return { ok: false, why: explainDifference(expected, actual), expected, actual };
}

// ---- JSON ----------------------------------------------------------------------------------

function maskValue(value) {
  if (typeof value === 'string') return maskLine(value);
  return value;
}

const OMITTED = '…';

/**
 * Whether every member the page shows is in the real result with the same (masked) value.
 * `"…": …` as a member, or `"…"` as the last array element, says "and more"; `"…"` as a value
 * says "any value". Returns an empty list when the shown JSON holds, else the paths that differ.
 */
function jsonSubset(shown, actual, at = '$') {
  if (shown === OMITTED) return [];
  if (Array.isArray(shown)) {
    if (!Array.isArray(actual)) return [`${at}: the page shows an array`];
    const open = shown[shown.length - 1] === OMITTED;
    const items = open ? shown.slice(0, -1) : shown;
    if (open ? actual.length < items.length : actual.length !== items.length) {
      return [`${at}: the page shows ${items.length}${open ? ' or more' : ''} elements, the run printed ${actual.length}`];
    }
    return items.flatMap((item, index) => jsonSubset(item, actual[index], `${at}[${index}]`));
  }
  if (shown !== null && typeof shown === 'object') {
    if (actual === null || typeof actual !== 'object' || Array.isArray(actual)) return [`${at}: the page shows an object`];
    return Object.entries(shown)
      .filter(([key]) => key !== OMITTED)
      .flatMap(([key, value]) => (key in actual ? jsonSubset(value, actual[key], `${at}.${key}`) : [`${at}.${key}: missing from the run's output`]));
  }
  return maskValue(shown) === maskValue(actual) ? [] : [`${at}: the page shows ${JSON.stringify(shown)}, the run printed ${JSON.stringify(actual)}`];
}

/** Compares shown JSON (lines) with the real result: `{ ok, why }`. */
function compareJson(shownLines, actualText) {
  let shown;
  let actual;
  try {
    shown = JSON.parse(shownLines.join('\n'));
  } catch (error) {
    return { ok: false, why: `the page's JSON does not parse: ${error.message}` };
  }
  try {
    actual = JSON.parse(actualText);
  } catch (error) {
    return { ok: false, why: `the run's stdout is not JSON: ${error.message}` };
  }
  const differences = jsonSubset(shown, actual);
  return differences.length === 0 ? { ok: true } : { ok: false, why: differences.slice(0, 5).join('; ') };
}

module.exports = { compareJson, compareText, jsonSubset, linesMatch, maskLine, maskLines };
