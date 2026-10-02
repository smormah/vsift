'use strict';

// Pure helpers of the upgrade check (RQ-04): where VSift keeps its own folders
// on each system (install.md section 8.3), how to find what an uninstall left,
// and how to compare what an older build returned with what a newer one does.

const fs = require('node:fs');
const path = require('node:path');

/**
 * The folders VSift writes for a user, as install.md section 8 lists them, for
 * the user state described by `env` (HOME, LOCALAPPDATA, XDG_*): its
 * configuration, its disposable sessions and (Linux only) its managed tools.
 */
function vsiftFolders(platform, env) {
  const home = env.HOME || env.USERPROFILE;
  if (platform === 'win32') {
    return { config: path.join(env.LOCALAPPDATA, 'vsift'), sessions: path.join(env.LOCALAPPDATA, 'VSift-sessions'), managed: null };
  }
  if (platform === 'darwin') {
    return {
      config: path.join(home, 'Library', 'Application Support', 'vsift'),
      sessions: path.join(home, 'Library', 'Caches', 'VSift-sessions'),
      managed: null,
    };
  }
  const xdg = (name, fallback) => (env[name] && path.isAbsolute(env[name]) ? env[name] : path.join(home, fallback));
  return {
    config: path.join(xdg('XDG_CONFIG_HOME', '.config'), 'vsift'),
    sessions: path.join(xdg('XDG_CACHE_HOME', '.cache'), 'vsift-sessions'),
    managed: path.join(xdg('XDG_DATA_HOME', '.local/share'), 'vsift', 'managed-v1'),
  };
}

/** Whether a directory holds nothing but (empty) directories. */
function isEmptyTree(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (!entry.isDirectory() || !isEmptyTree(path.join(directory, entry.name))) {
      return false;
    }
  }
  return true;
}

/**
 * Every path under `root` whose name mentions vsift (a match is reported once
 * and not entered). Directories that hold nothing are not reported when
 * `ignoreEmpty` is set: a package manager may leave an empty scope folder.
 */
function findVsiftEntries(root, { ignoreEmpty = false } = {}) {
  const found = [];
  const walk = (directory) => {
    let entries;
    try {
      entries = fs.readdirSync(directory, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(directory, entry.name);
      if (/vsift/i.test(entry.name)) {
        if (!(ignoreEmpty && entry.isDirectory() && isEmptyTree(full))) {
          found.push(full);
        }
        continue;
      }
      if (entry.isDirectory()) {
        walk(full);
      }
    }
  };
  walk(root);
  return found;
}

/**
 * Whether everything `before` holds is still in `after`, with the same value:
 * objects may have gained members (v1 is additive), arrays keep their length
 * and order, scalars are equal. Returns the differences, as JSON pointers.
 */
function subsetProblems(before, after, pointer = '') {
  if (before === null || typeof before !== 'object') {
    return before === after ? [] : [`${pointer || '/'}: ${JSON.stringify(before)} became ${JSON.stringify(after)}`];
  }
  if (Array.isArray(before)) {
    if (!Array.isArray(after) || after.length !== before.length) {
      return [`${pointer || '/'}: an array of ${before.length} became ${Array.isArray(after) ? `${after.length} items` : JSON.stringify(after)}`];
    }
    return before.flatMap((item, index) => subsetProblems(item, after[index], `${pointer}/${index}`));
  }
  if (after === null || typeof after !== 'object' || Array.isArray(after)) {
    return [`${pointer || '/'}: an object became ${JSON.stringify(after)}`];
  }
  return Object.keys(before).flatMap((key) =>
    Object.hasOwn(after, key) ? subsetProblems(before[key], after[key], `${pointer}/${key}`) : [`${pointer}/${key}: removed`],
  );
}

module.exports = { findVsiftEntries, isEmptyTree, subsetProblems, vsiftFolders };
