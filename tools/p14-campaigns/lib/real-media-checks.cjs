'use strict';

// The property checks of the real-media campaign (#363,
// docs/planning/real-media-set.md section 7). A real recording has no frozen
// transcript, so nothing here compares words with a truth: each check is a
// property of VSift's own answers (no failure on valid speech, coverage that
// adds up, no words lost where two chunks meet, repeats at seams, how far two
// chunk plans disagree on the same speech). Everything is pure, so the tests
// feed it hand-made answers; the runner is real-media-campaign.cjs.

/** Half a second: `SEAM_COVERAGE_TOLERANCE_MICROS`, the merge's own bound (cli-v1.md). */
const SEAM_TOLERANCE_US = 500_000;
/** How far beyond a seam's overlap the comparison looks, each side. */
const SEAM_MARGIN_US = 3_000_000;
/** How far the other revision's words may lie from a word and still count as the same speech. */
const MATCH_SLACK_US = 6_000_000;
/** A run of this many words missing at one seam is reported as a finding whatever the recall. */
const LOST_RUN_FINDING_WORDS = 8;
/** Seam recall this far below the recall away from seams is a finding. */
const RECALL_GAP_FINDING = 0.10;
/** The commands whose failure on valid media is a finding of its own kind (#353, #391). */
const NEVER_ON_VALID_MEDIA = Object.freeze(['MISSING_CAPABILITY', 'INTEGRITY_FAILURE', 'INVALID_SOURCE', 'INTERNAL']);

/**
 * The words of a text, normalised a little like `search` does: lowercase,
 * letters and digits only (any other character separates words). Accents are
 * kept: the comparison is between two runs of the same recogniser.
 * @param {string} text
 * @returns {string[]}
 */
function words(text) {
  return String(text).toLowerCase().split(/[^\p{L}\p{N}]+/u).filter((word) => word.length > 0);
}

/**
 * Every word of the segments, with a time spread evenly over its segment
 * (the recogniser gives segment times only), and the chunk it came from.
 * @param {Array<{ start_us: number, end_us: number, text: string, segment_id?: string, alignment?: { chunk_index?: number } }>} segments
 * @returns {Array<{ word: string, at_us: number, segment_id: string | null, chunk: number | null }>}
 */
function timeline(segments) {
  const out = [];
  for (const segment of [...segments].sort((a, b) => a.start_us - b.start_us)) {
    const list = words(segment.text);
    const span = Math.max(0, segment.end_us - segment.start_us);
    list.forEach((word, index) => {
      out.push({
        word,
        at_us: Math.round(segment.start_us + (span * (index + 0.5)) / list.length),
        segment_id: segment.segment_id || null,
        chunk: segment.alignment && Number.isInteger(segment.alignment.chunk_index) ? segment.alignment.chunk_index : null,
      });
    });
  }
  return out;
}

/**
 * The chunk plan a run used, read from its own segments' alignment (each
 * local-ASR segment records its chunk's index and audio window). Segments a
 * revision carried from an earlier one are left out: they belong to that
 * revision's plan.
 * @param {Array<object>} segments
 * @returns {Array<{ index: number, from_us: number, to_us: number }>}
 */
function chunkPlan(segments) {
  const chunks = new Map();
  for (const segment of segments) {
    const alignment = segment.alignment || {};
    if (segment.carried_from || alignment.origin !== 'local_asr' || !Number.isInteger(alignment.chunk_index)) continue;
    chunks.set(alignment.chunk_index, {
      index: alignment.chunk_index,
      from_us: alignment.chunk_audio_start_us,
      to_us: alignment.chunk_audio_end_us,
    });
  }
  return [...chunks.values()].sort((a, b) => a.from_us - b.from_us);
}

/**
 * Where consecutive chunks of a plan overlap: each seam is the stretch both
 * chunks heard, where the merge chose between their segments.
 * @param {Array<{ index: number, from_us: number, to_us: number }>} plan
 * @returns {Array<{ from_us: number, to_us: number, chunks: [number, number] }>}
 */
function seams(plan) {
  const out = [];
  for (let index = 1; index < plan.length; index += 1) {
    const before = plan[index - 1];
    const after = plan[index];
    if (after.from_us < before.to_us) out.push({ from_us: after.from_us, to_us: before.to_us, chunks: [before.index, after.index] });
  }
  return out;
}

function trigramsNear(list, from, to) {
  const set = new Set();
  for (let index = 0; index + 2 < list.length; index += 1) {
    if (list[index].at_us >= from && list[index + 2].at_us <= to) set.add(`${list[index].word} ${list[index + 1].word} ${list[index + 2].word}`);
  }
  return set;
}

/**
 * How much of what `reference` says in a stretch the `subject` also says:
 * each reference word counts as found when one of the word triples it is part
 * of appears among the subject's triples nearby. Runs of words not found are
 * returned with their times (an estimate inside their segment).
 * @returns {{ words: number, found: number, lost: Array<{ from_us: number, to_us: number, words: string[] }> }}
 */
function recallIn(reference, subject, from, to) {
  const inside = reference.map((entry, index) => ({ entry, index })).filter(({ entry }) => entry.at_us >= from && entry.at_us < to);
  if (inside.length === 0) return { words: 0, found: 0, lost: [] };
  const theirs = trigramsNear(subject, from - MATCH_SLACK_US, to + MATCH_SLACK_US);
  const triple = (index) => (index >= 0 && index + 2 < reference.length
    ? `${reference[index].word} ${reference[index + 1].word} ${reference[index + 2].word}` : null);
  let found = 0;
  const lost = [];
  let run = null;
  for (const { entry, index } of inside) {
    const hit = [index - 2, index - 1, index].some((start) => {
      const key = triple(start);
      return key !== null && theirs.has(key);
    });
    if (hit) {
      found += 1;
      if (run) lost.push(run);
      run = null;
    } else if (run) {
      run.to_us = entry.at_us;
      run.words.push(entry.word);
    } else {
      run = { from_us: entry.at_us, to_us: entry.at_us, words: [entry.word] };
    }
  }
  if (run) lost.push(run);
  return { words: inside.length, found, lost };
}

/** The writing system most of a list's letters belong to: `latin`, `cyrillic`, `other`, or null with no letter. */
function script(list) {
  const counts = { latin: 0, cyrillic: 0, other: 0 };
  for (const entry of list) {
    for (const letter of entry.word.match(/\p{L}/gu) || []) {
      if (/\p{Script=Latin}/u.test(letter)) counts.latin += 1;
      else if (/\p{Script=Cyrillic}/u.test(letter)) counts.cyrillic += 1;
      else counts.other += 1;
    }
  }
  const [best, count] = Object.entries(counts).sort((a, b) => b[1] - a[1])[0];
  return count > 0 ? best : null;
}

/**
 * What a run of words one revision has and the other lacks most likely is.
 * `repetition`: the run repeats itself (few distinct word triples), as a
 * recogniser's loop on noise or silence does; `script`: the other revision
 * heard the same stretch in another writing system (a chunk whose language the
 * recogniser detected differently); otherwise `loss`, a candidate for words the
 * merge lost, to be confirmed by listening.
 */
function lostKind(run, subject) {
  const triples = [];
  for (let index = 0; index + 2 < run.words.length; index += 1) triples.push(run.words.slice(index, index + 3).join(' '));
  if (run.words.length >= 6 && new Set(triples).size / triples.length < 0.5) return 'repetition';
  const theirs = subject.filter((entry) => entry.at_us >= run.from_us - 1_000_000 && entry.at_us <= run.to_us + 1_000_000);
  const mine = script(run.words.map((word) => ({ word })));
  const other = script(theirs);
  if (mine !== null && other !== null && mine !== other) return 'script';
  return 'loss';
}

/**
 * The seam comparison of two revisions of the same audio whose chunk plans
 * are shifted against each other, so that every seam of one lies inside a
 * chunk of the other. At each seam of `subject`, what `reference` (mid-chunk
 * there) says is looked for in `subject`; the same is done at control points
 * far from both plans' seams, so the recogniser's own disagreement between two
 * cuts is measured too. A seam loss is then recall well below the control's,
 * or a long run of words lost at one seam.
 * @param {Array<object>} subjectSegments the revision whose seams are tested
 * @param {Array<object>} referenceSegments the shifted revision
 * @param {{ from_us: number, to_us: number }} compared the stretch both revisions transcribed afresh
 * @returns {object}
 */
function seamComparison(subjectSegments, referenceSegments, compared) {
  const subject = timeline(subjectSegments);
  const reference = timeline(referenceSegments);
  const subjectSeams = seams(chunkPlan(subjectSegments)).filter((seam) => seam.from_us >= compared.from_us && seam.to_us <= compared.to_us);
  const perSeam = subjectSeams.map((seam) => {
    const from = seam.from_us - SEAM_MARGIN_US;
    const to = seam.to_us + SEAM_MARGIN_US;
    const result = recallIn(reference, subject, from, to);
    return { from_us: seam.from_us, to_us: seam.to_us, chunks: seam.chunks, ...result };
  });
  const width = subjectSeams.length > 0 ? (subjectSeams[0].to_us - subjectSeams[0].from_us) + 2 * SEAM_MARGIN_US : 11_000_000;
  const controls = [];
  // Control stretches lie wholly inside one chunk of the subject, away from its
  // seams; a seam of the reference there only removes reference words, which
  // cannot count as a loss of the subject.
  const step = 1_000_000;
  for (let at = compared.from_us; at + width <= compared.to_us;) {
    const clear = !subjectSeams.some((seam) => at < seam.to_us + SEAM_MARGIN_US && at + width > seam.from_us - SEAM_MARGIN_US);
    if (clear) {
      controls.push(recallIn(reference, subject, at, at + width));
      at += width;
    } else {
      at += step;
    }
  }
  const sum = (list, key) => list.reduce((total, entry) => total + entry[key], 0);
  const seamWords = sum(perSeam, 'words');
  const controlWords = sum(controls, 'words');
  const seamRecall = seamWords > 0 ? sum(perSeam, 'found') / seamWords : null;
  const controlRecall = controlWords > 0 ? sum(controls, 'found') / controlWords : null;
  const lostRuns = perSeam.flatMap((seam) => seam.lost.map((run) => ({ ...run, seam: [seam.from_us, seam.to_us], kind: lostKind(run, subject) })))
    .filter((run) => run.words.length >= 3)
    .sort((a, b) => b.words.length - a.words.length);
  // Words in a run the recogniser explains (a loop, another script) are not
  // counted against the merge: they say how two cuts of the same audio differ.
  const explained = lostRuns.filter((run) => run.kind !== 'loss').reduce((total, run) => total + run.words.length, 0);
  const mergeRecall = seamWords > 0 ? (sum(perSeam, 'found') + explained) / seamWords : null;
  const findings = [];
  if (mergeRecall !== null && controlRecall !== null && controlRecall - mergeRecall > RECALL_GAP_FINDING) {
    findings.push(`seam recall ${mergeRecall.toFixed(3)} (runs the recogniser explains counted as found) is more than ${RECALL_GAP_FINDING} below the control's ${controlRecall.toFixed(3)}`);
  }
  for (const run of lostRuns.filter((entry) => entry.kind === 'loss' && entry.words.length >= LOST_RUN_FINDING_WORDS)) {
    findings.push(`${run.words.length} words heard mid-chunk are missing at the seam ${run.seam[0]}-${run.seam[1]} us (about ${run.from_us}-${run.to_us} us): listen`);
  }
  const kinds = (kind) => lostRuns.filter((run) => run.kind === kind).length;
  return {
    seams: perSeam.length,
    seam_words: seamWords,
    seam_recall: seamRecall,
    seam_recall_merge: mergeRecall,
    control_stretches: controls.length,
    control_words: controlWords,
    control_recall: controlRecall,
    lost_runs: lostRuns.slice(0, 20).map((run) => ({ from_us: run.from_us, to_us: run.to_us, words: run.words.length, seam: run.seam, kind: run.kind })),
    lost_run_kinds: { loss: kinds('loss'), repetition: kinds('repetition'), script: kinds('script') },
    findings,
    passed: findings.length === 0,
  };
}

/**
 * Word triples that stand twice within four seconds at a seam, the second
 * copy from the other chunk: what the seam guard's restored copies can leave
 * (#380, L-155).
 * @param {Array<object>} segments one revision
 * @returns {{ seams: number, repeats: Array<{ at_us: number, words: string }>, per_hour: number | null }}
 */
function seamRepeats(segments, durationUs) {
  const list = timeline(segments);
  const seamList = seams(chunkPlan(segments));
  const repeats = [];
  for (const seam of seamList) {
    const from = seam.from_us - SEAM_MARGIN_US;
    const to = seam.to_us + SEAM_MARGIN_US;
    const inside = [];
    for (let index = 0; index + 2 < list.length; index += 1) {
      if (list[index].at_us >= from && list[index].at_us < to) {
        inside.push({ key: `${list[index].word} ${list[index + 1].word} ${list[index + 2].word}`, at_us: list[index].at_us, chunk: list[index].chunk, segment: list[index].segment_id });
      }
    }
    // Two chunks' copies interleave in time, so the triples are put in time order first.
    inside.sort((x, y) => x.at_us - y.at_us);
    const seen = new Set();
    for (let first = 0; first < inside.length; first += 1) {
      for (let second = first + 1; second < inside.length; second += 1) {
        const a = inside[first];
        const b = inside[second];
        if (b.at_us - a.at_us > 4_000_000) break;
        if (a.key === b.key && a.segment !== b.segment && a.chunk !== b.chunk && !seen.has(a.key)) {
          seen.add(a.key);
          repeats.push({ at_us: a.at_us, words: a.key });
        }
      }
    }
  }
  return { seams: seamList.length, repeats, per_hour: durationUs > 0 ? (repeats.length * 3_600_000_000) / durationUs : null };
}

function quantile(sorted, q) {
  if (sorted.length === 0) return null;
  const position = Math.min(sorted.length - 1, Math.max(0, Math.round(q * (sorted.length - 1))));
  return sorted[position];
}

/**
 * How far two chunk plans disagree on the times of the same speech (#381,
 * L-156): segments of both fresh runs whose text is the same (at least three
 * words) are matched one to one, and the differences of their starts and ends
 * are summarised.
 * @returns {{ matched: number, start: object, end: object }}
 */
function timingDifferences(segmentsA, segmentsB) {
  const fresh = (list) => list.filter((segment) => !segment.carried_from);
  const key = (segment) => words(segment.text).join(' ');
  const byText = new Map();
  for (const segment of fresh(segmentsB)) {
    const text = key(segment);
    if (text.split(' ').length < 3) continue;
    if (!byText.has(text)) byText.set(text, []);
    byText.get(text).push(segment);
  }
  const starts = [];
  const ends = [];
  for (const segment of fresh(segmentsA)) {
    const candidates = byText.get(key(segment));
    if (!candidates || candidates.length !== 1) continue;
    const other = candidates[0];
    if (Math.abs(other.start_us - segment.start_us) > MATCH_SLACK_US) continue;
    starts.push(Math.abs(other.start_us - segment.start_us));
    ends.push(Math.abs(other.end_us - segment.end_us));
  }
  const summary = (values) => {
    const sorted = [...values].sort((a, b) => a - b);
    return {
      median_us: quantile(sorted, 0.5),
      p95_us: quantile(sorted, 0.95),
      max_us: sorted.length > 0 ? sorted[sorted.length - 1] : null,
      over_tolerance: sorted.filter((value) => value > SEAM_TOLERANCE_US).length,
    };
  };
  return { matched: starts.length, start: summary(starts), end: summary(ends) };
}

/** The bins of `seamJoins`, by the gap between one kept segment's end and the next one's start. */
const JOIN_BINS = Object.freeze([
  { name: 'overlap', below_us: 0 },
  { name: 'touch', below_us: 1 },
  { name: 'up_to_100_ms', below_us: 100_001 },
  { name: 'up_to_250_ms', below_us: 250_001 },
  { name: 'up_to_500_ms', below_us: SEAM_TOLERANCE_US + 1 },
  { name: 'up_to_1_s', below_us: 1_000_001 },
  { name: 'over_1_s', below_us: Infinity },
]);

/**
 * How the merge joins two chunks' segments at its seams, against how one
 * chunk's own segments follow each other (#381, L-156). The seam guard leaves a
 * stretch of what the chunks heard uncovered when it is no longer than the
 * tolerance (half a second), so a hole it left is a gap of at most that between
 * a segment of one chunk and the next segment, of the following chunk, at a
 * seam. Pauses make such gaps too, so the same gaps between consecutive segments
 * of one chunk, away from its seams, are the control: if the seam joins have no
 * more gaps of up to half a second than the control, the tolerance leaves
 * nothing beyond ordinary pauses. The times of the seam gaps of up to half a
 * second are listed (`within_tolerance_at_us`, where the gap starts) for a
 * listener: what they hold is not in any answer, because the merge dropped it.
 * A measurement, not a check.
 * @param {Array<{ start_us: number, end_us: number, carried_from?: string, alignment?: object }>} segments one revision
 * @returns {{ seam_joins: object, control_joins: object }}
 */
function seamJoins(segments) {
  const plan = chunkPlan(segments);
  const seamList = seams(plan);
  const fresh = segments
    .filter((segment) => !segment.carried_from && segment.alignment && Number.isInteger(segment.alignment.chunk_index))
    .sort((a, b) => a.start_us - b.start_us || a.end_us - b.end_us);
  const empty = () => Object.fromEntries(JOIN_BINS.map((bin) => [bin.name, 0]));
  const seamCounts = empty();
  const withinTolerance = [];
  const controlCounts = empty();
  const bin = (gap) => JOIN_BINS.find((entry) => gap < entry.below_us).name;
  const nearSeam = (at) => seamList.some((seam) => at >= seam.from_us - SEAM_MARGIN_US && at <= seam.to_us + SEAM_MARGIN_US);
  for (let index = 1; index < fresh.length; index += 1) {
    const before = fresh[index - 1];
    const after = fresh[index];
    const gap = after.start_us - before.end_us;
    const chunks = [before.alignment.chunk_index, after.alignment.chunk_index];
    if (chunks[1] === chunks[0] + 1 && nearSeam(after.start_us)) {
      seamCounts[bin(gap)] += 1;
      if (gap > 0 && gap <= SEAM_TOLERANCE_US) withinTolerance.push(before.end_us);
    } else if (chunks[1] === chunks[0] && !nearSeam(before.end_us) && !nearSeam(after.start_us)) {
      controlCounts[bin(gap)] += 1;
    }
  }
  const total = (counts) => Object.values(counts).reduce((sum, value) => sum + value, 0);
  const share = (counts) => {
    const all = total(counts);
    return all > 0 ? (counts.up_to_100_ms + counts.up_to_250_ms + counts.up_to_500_ms) / all : null;
  };
  return {
    seam_joins: { count: total(seamCounts), bins: seamCounts, within_tolerance_share: share(seamCounts), within_tolerance_at_us: withinTolerance },
    control_joins: { count: total(controlCounts), bins: controlCounts, within_tolerance_share: share(controlCounts) },
  };
}

function mergeRanges(ranges) {
  const sorted = [...ranges].filter((range) => range.to_us > range.from_us).sort((a, b) => a.from_us - b.from_us);
  const out = [];
  for (const range of sorted) {
    const last = out[out.length - 1];
    if (last && range.from_us <= last.to_us) last.to_us = Math.max(last.to_us, range.to_us);
    else out.push({ from_us: range.from_us, to_us: range.to_us });
  }
  return out;
}

const length = (ranges) => mergeRanges(ranges).reduce((total, range) => total + (range.to_us - range.from_us), 0);

function intersection(a, b) {
  const out = [];
  for (const x of mergeRanges(a)) {
    for (const y of mergeRanges(b)) {
      const from = Math.max(x.from_us, y.from_us);
      const to = Math.min(x.to_us, y.to_us);
      if (to > from) out.push({ from_us: from, to_us: to });
    }
  }
  return out;
}

/**
 * Whether a coverage statement adds up (cli-v1.md, "Coverage"): transcribed
 * and untranscribed ranges are disjoint and together are the searched range;
 * no-speech ranges lie inside the transcribed ones; and the result is
 * `partial` exactly when something is untranscribed. A truncated list is only
 * checked for what it does list.
 * @param {object} coverage a `transcript_coverage`
 * @param {string} status the result's status
 * @returns {string[]} what does not add up; empty when it does
 */
function coverageProblems(coverage, status) {
  const problems = [];
  if (!coverage || typeof coverage !== 'object') return ['no transcript_coverage'];
  const searched = coverage.searched_range;
  const transcribed = coverage.transcribed_ranges || [];
  const untranscribed = coverage.untranscribed_ranges || [];
  const noSpeech = coverage.no_speech_ranges || [];
  if (length(intersection(transcribed, untranscribed)) > 0) problems.push('a range is both transcribed and untranscribed');
  if (length(noSpeech) - length(intersection(noSpeech, transcribed)) > 0) problems.push('a no-speech range lies outside the transcribed ranges');
  if (searched && !coverage.ranges_truncated) {
    const total = searched.to_us - searched.from_us;
    const covered = length([...transcribed, ...untranscribed]);
    if (covered !== total) problems.push(`transcribed and untranscribed ranges cover ${covered} of the ${total} us searched`);
  }
  const hasGap = untranscribed.length > 0;
  if (hasGap !== (status === 'partial')) problems.push(`status ${status} but ${hasGap ? 'a' : 'no'} gap is listed`);
  return problems;
}

/**
 * The failure of one command, judged: an error code VSift must never give on
 * valid media is a finding; any other failure is recorded as it is.
 * @param {{ step: string, exit: number | null, json: object | null }} answer
 * @returns {string | null} a finding, or null
 */
function failureFinding(answer) {
  const error = answer.json && answer.json.error;
  if (answer.exit === 0 && !error) return null;
  const code = error && error.code ? error.code : 'no JSON error';
  if (NEVER_ON_VALID_MEDIA.includes(code)) return `${answer.step}: ${code} on valid media (exit ${answer.exit})`;
  return `${answer.step}: failed with ${code} (exit ${answer.exit})`;
}

/**
 * A plain summary of a list of numbers, for timings.
 * @param {number[]} values
 */
function describe(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return { count: sorted.length, median: quantile(sorted, 0.5), max: sorted.length > 0 ? sorted[sorted.length - 1] : null };
}

/**
 * The item's verdict: it passes when no check has a finding. The seam
 * measurements are advisory evidence; their findings still fail the item, so
 * the run says plainly that something needs a listener.
 * @param {Record<string, { passed: boolean, findings?: string[] }>} checks
 */
function itemVerdict(checks) {
  const findings = Object.entries(checks).flatMap(([name, check]) => (check.findings || []).map((finding) => `${name}: ${finding}`));
  return { passed: Object.values(checks).every((check) => check.passed), findings };
}

/**
 * Stretches the two revisions heard in different writing systems (a chunk
 * whose language the recogniser detected otherwise), from both seam
 * comparisons: the same audio transcribed as two languages depending on where
 * the chunks were cut.
 */
function languageFlips(...comparisons) {
  const runs = comparisons.flatMap((comparison) => comparison.lost_runs.filter((run) => run.kind === 'script'));
  return {
    count: runs.length,
    at_us: runs.map((run) => run.from_us).sort((a, b) => a - b),
    findings: runs.length > 0 ? [`${runs.length} stretch(es) were heard in a different writing system by the two chunk plans (per-chunk language detection)`] : [],
    passed: runs.length === 0,
  };
}

/** How far a person's label of a change may lie outside a candidate's change window and still be its change. */
const LABEL_TOLERANCE_US = 500_000;
/** The loose distance of check 7's first form: a candidate within one second of the label. */
const LOOSE_US = 1_000_000;

/**
 * Check 7 (#175): how many labelled changes (`changes_us`, a person's times)
 * a candidate accounts for. A labelled change is *bracketed* when a change
 * candidate's `change_window` holds it, widened by `LABEL_TOLERANCE_US` each
 * side for the labeller's own timing, or when a `motion_start` candidate's
 * span holds it (the motion is one candidate). The loose figure (any
 * candidate within a second) is kept with its *chance* figure: the share of
 * the recording that lies within a second of some candidate, which a label
 * placed at random would score. On a busy screen recording the chance is
 * high (seven in eight of RM10's still moments, run 39219218493), so the loose figure
 * alone says little; the bracketed one is the check.
 * @param {number[]} changes the labelled times, microseconds
 * @param {Array<{ representative_us: number, reasons?: string[], span?: { from_us: number, to_us: number }, change_window?: { from_us: number, to_us: number } | null }>} candidates
 * @param {number} durationUs
 */
function labelledChanges(changes, candidates, durationUs) {
  const bracketing = (at) => candidates.some((candidate) => {
    const window = candidate.change_window;
    if (window && window.from_us - LABEL_TOLERANCE_US <= at && at <= window.to_us + LABEL_TOLERANCE_US) return true;
    const span = candidate.span;
    return (candidate.reasons || []).includes('motion_start') && Boolean(span) && span.from_us <= at && at < span.to_us;
  });
  const near = (at) => candidates.some((candidate) => Math.abs(candidate.representative_us - at) <= LOOSE_US);
  const reach = mergeRanges(candidates.map((candidate) => ({
    from_us: Math.max(0, candidate.representative_us - LOOSE_US),
    to_us: Math.min(durationUs, candidate.representative_us + LOOSE_US),
  })).filter((range) => range.to_us > range.from_us));
  const covered = reach.reduce((total, range) => total + range.to_us - range.from_us, 0);
  const bracketed = changes.filter(bracketing).length;
  const within = changes.filter(near).length;
  return {
    labelled: changes.length,
    bracketed,
    bracketed_share: changes.length > 0 ? bracketed / changes.length : null,
    within_1s: within,
    within_1s_share: changes.length > 0 ? within / changes.length : null,
    chance_within_1s: durationUs > 0 ? covered / durationUs : null,
  };
}

module.exports = {
  SEAM_TOLERANCE_US,
  languageFlips,
  script,
  SEAM_MARGIN_US,
  LOST_RUN_FINDING_WORDS,
  RECALL_GAP_FINDING,
  NEVER_ON_VALID_MEDIA,
  words,
  timeline,
  chunkPlan,
  seams,
  seamComparison,
  seamRepeats,
  timingDifferences,
  seamJoins,
  JOIN_BINS,
  coverageProblems,
  mergeRanges,
  failureFinding,
  describe,
  itemVerdict,
  labelledChanges,
  LABEL_TOLERANCE_US,
};
