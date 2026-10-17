#!/usr/bin/env node
'use strict';

// #367: how many calls and characters it takes to shortlist the screens of a long
// recording with `vsift candidates`, through the binary, before and after a change.
//
//   node candidates-shortlist.cjs build <corpus generated dir> <out.mp4>
//   node candidates-shortlist.cjs measure <vsift> <video> <work dir> whole|skill|omit
//   node candidates-shortlist.cjs project <vsift> <work dir of a "skill" run>
//
// build: a 20:57 synthetic walkthrough from the committed corpus, never a private
//   recording: F01 to F08 scaled and padded to 1920x1080 at 20 fps, each held on its
//   last frame for 40 s, joined, and that sequence three times. Needs FFmpeg on PATH.
// measure: a fresh session root and per-user folders under <work dir> (it is emptied
//   first; never point it at a folder that matters), one `ingest`, then either
//   "whole" (one range for the whole video, the default page size, the cursor to null:
//   what the issue's agent did) or "skill" (the 0.2.1 skill: ranges of at most five
//   minutes, --limit 100 --view brief, each range's cursor to null, a range read again
//   while it reports not_analyzed), or "omit" (the same with --omit periodic, 0.2.2,
//   which leaves out the periodic-only candidates). Prints calls, characters, seconds and
//   the reasons.
// project: warm reads of a "skill" run's session (no analysis) and what each filter
//   option would cost on the same items.
//
// Node.js 22, no dependency. `vsift` is the binary named, by path, never one on PATH.

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const FIXTURES = ['F01', 'F02', 'F03', 'F04', 'F05', 'F06', 'F07', 'F08'];
const FIVE_MINUTES_US = 300e6;

function build(corpus, out) {
  const scratch = `${out}.parts`;
  fs.mkdirSync(scratch, { recursive: true });
  const inputs = FIXTURES.flatMap((name) => ['-i', path.join(corpus, `${name}.mp4`)]);
  const legs = FIXTURES.map((_, index) => `[${index}:v]scale=1920:1080:force_original_aspect_ratio=decrease,`
    + 'pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=20,tpad=stop_mode=clone:stop_duration=40'
    + `[v${index}]`).join(';');
  const joined = `${legs};${FIXTURES.map((_, index) => `[v${index}]`).join('')}concat=n=${FIXTURES.length}:v=1:a=0[seq]`;
  const loop = path.join(scratch, 'loop.mp4');
  ffmpeg([...inputs, '-filter_complex', joined, '-map', '[seq]', '-c:v', 'libx264', '-preset', 'veryfast',
    '-pix_fmt', 'yuv420p', '-an', loop]);
  const list = path.join(scratch, 'list.txt');
  fs.writeFileSync(list, `file '${loop}'\n`.repeat(3));
  ffmpeg(['-f', 'concat', '-safe', '0', '-i', list, '-c', 'copy', out]);
  fs.rmSync(scratch, { recursive: true, force: true });
}

function ffmpeg(args) {
  const result = spawnSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', ...args], { stdio: 'inherit' });
  if (result.status !== 0) throw new Error(`ffmpeg failed (${result.status})`);
}

function runner(vsift, work) {
  const root = path.join(work, 'root');
  const local = path.join(work, 'local');
  const env = { ...process.env, LOCALAPPDATA: local, APPDATA: path.join(local, 'roaming'),
    XDG_CONFIG_HOME: path.join(local, 'config'), XDG_CACHE_HOME: path.join(local, 'cache') };
  return (args) => {
    const started = process.hrtime.bigint();
    const result = spawnSync(vsift, ['--session-root', root, ...args, '--json'],
      { encoding: 'utf8', maxBuffer: 1 << 28, env });
    if (result.error) throw result.error;
    const seconds = Number(process.hrtime.bigint() - started) / 1e9;
    return { seconds, characters: result.stdout.length, json: JSON.parse(result.stdout) };
  };
}

function walk(run, session, from, to, extra, calls, items) {
  for (let round = 0; round < 20; round += 1) {
    const before = items.length;
    let cursor = null;
    let data;
    do {
      const args = ['candidates', session, '--from', String(from), '--to', String(to), ...extra];
      if (cursor) args.push('--cursor', cursor);
      const answer = run(args);
      data = answer.json.data || {};
      const gaps = (data.coverage && data.coverage.gaps) || [];
      calls.push({ from, to, seconds: answer.seconds, characters: answer.characters, status: answer.json.status,
        code: answer.json.error && answer.json.error.code, items: (data.items || []).length,
        pending: gaps.filter((gap) => gap.reason === 'not_analyzed' || gap.reason === 'deadline_exceeded').length,
        dropped: gaps.reduce((sum, gap) => sum + (gap.dropped_candidates || 0), 0) });
      items.push(...(data.items || []));
      cursor = data.next_cursor;
    } while (cursor);
    if (!calls[calls.length - 1].pending) return data;
    items.length = before;
  }
  throw new Error('the range stayed not analysed for 20 rounds');
}

function describe(items) {
  const reasons = {};
  let runs = 0;
  let previous = null;
  for (const item of items) {
    for (const reason of item.reasons) reasons[reason] = (reasons[reason] || 0) + 1;
    if (item.visual_hash !== previous) runs += 1;
    previous = item.visual_hash;
  }
  return { candidates: items.length, distinct_hashes: new Set(items.map((item) => item.visual_hash)).size,
    hash_runs: runs, reasons };
}

function measure(vsift, video, work, mode) {
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const run = runner(vsift, work);
  const session = run(['ingest', video]).json.data.session_id;
  const calls = [];
  const items = [];
  let omitted = 0;
  if (mode === 'whole') {
    walk(run, session, 0, 24 * 3600e6, [], calls, items);
  } else if (mode === 'skill' || mode === 'omit') {
    const extra = ['--limit', '100', '--view', 'brief', ...(mode === 'omit' ? ['--omit', 'periodic'] : [])];
    let duration = Infinity;
    for (let from = 0; from < duration; from += FIVE_MINUTES_US) {
      const data = walk(run, session, from, Math.min(duration, from + FIVE_MINUTES_US), extra, calls, items);
      duration = data.index.duration_us;
      if (data.omitted) omitted += data.omitted.count;
    }
  } else {
    throw new Error(`unknown mode ${mode}`);
  }
  const total = (key) => calls.reduce((sum, call) => sum + call[key], 0);
  console.log(JSON.stringify({ mode, calls: calls.length, characters: total('characters'),
    seconds: Number(total('seconds').toFixed(1)),
    longest_call_seconds: Number(Math.max(...calls.map((call) => call.seconds)).toFixed(1)),
    ...describe(items),
    omitted_by_request: omitted,
    per_call: calls.map((call) => `${(call.from / 60e6).toFixed(0)}-${Math.min(call.to / 60e6, 1440).toFixed(0)} min `
      + `${call.status}${call.code ? ` ${call.code}` : ''}: ${call.items} items, ${call.characters} characters, `
      + `${call.seconds.toFixed(1)} s, pending ${call.pending}, dropped ${call.dropped}`) }, null, 1));
}

function project(vsift, work) {
  const run = runner(vsift, work);
  const session = run(['session', 'list']).json.data.items[0].session_id;
  const calls = [];
  const items = [];
  let duration = Infinity;
  for (let from = 0; from < duration; from += FIVE_MINUTES_US) {
    const data = walk(run, session, from, Math.min(duration, from + FIVE_MINUTES_US),
      ['--limit', '100', '--view', 'brief'], calls, items);
    duration = data.index.duration_us;
  }
  const itemCharacters = items.reduce((sum, item) => sum + JSON.stringify(item).length + 1, 0);
  const envelope = (calls.reduce((sum, call) => sum + call.characters, 0) - itemCharacters) / calls.length;
  const ranges = Math.ceil(duration / FIVE_MINUTES_US);
  const cost = (name, kept) => {
    const pages = Math.max(ranges, Math.ceil(kept.length / 100));
    const characters = Math.round(envelope * pages + kept.reduce((sum, item) => sum + JSON.stringify(item).length + 1, 0));
    console.log(`${name}: ${kept.length} items, about ${characters} characters, at least ${pages} calls`);
  };
  const periodicOnly = (item) => item.reasons.length === 1 && item.reasons[0] === 'periodic_coverage';
  const firstOfRun = () => {
    let previous = null;
    return (item) => {
      const keep = item.visual_hash !== previous;
      previous = item.visual_hash;
      return keep;
    };
  };
  cost('today', items);
  cost('without periodic-only items', items.filter((item) => !periodicOnly(item)));
  cost('one item per run of equal visual_hash', items.filter(firstOfRun()));
  cost('both', items.filter((item) => !periodicOnly(item)).filter(firstOfRun()));
  const perMinute = {};
  for (const item of items) {
    const minute = Math.floor(item.representative_us / 60e6);
    perMinute[minute] = (perMinute[minute] || 0) + 1;
  }
  console.log(`candidates per minute: ${Object.values(perMinute).join(' ')}`);
}

const [command, ...args] = process.argv.slice(2);
if (command === 'build' && args.length === 2) build(...args);
else if (command === 'measure' && args.length === 4) measure(...args);
else if (command === 'project' && args.length === 2) project(...args);
else {
  console.error('usage: build <corpus> <out.mp4> | measure <vsift> <video> <work> whole|skill|omit | project <vsift> <work>');
  process.exitCode = 2;
}
