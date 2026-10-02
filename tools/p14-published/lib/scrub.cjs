'use strict';

// The scrubbed environment of the published-artifact jobs (ADR 0024, RQ-01:
// "scrubbed environment (`cargo`, `rustc`, `git` and `python` must not
// resolve, recorded in the step)").
//
// Hosted runners are not clean machines: they carry a Rust toolchain, Git and
// Python on `PATH`, and a user who installs VSift from a package manager may
// have none of them. A job therefore runs every install and every `vsift`
// command with a `PATH` from which those programs cannot be found, and proves
// it by resolving each name the way a shell would, before anything is
// installed. What this shows is "no hidden dependency on a toolchain, a
// checkout or a developer's PATH"; it is not a clean machine (the plan says so
// wherever it matters).
//
// How a name is hidden:
//
// - A `PATH` directory that holds none of the hidden programs is kept as is.
// - On Windows a directory that holds one is dropped (the Rust, Git and Python
//   directories are their own; `C:\Windows\System32` holds none).
// - Elsewhere such a directory is usually a shared one (`/usr/bin`,
//   `/opt/homebrew/bin`), so it is replaced by a directory of links to every
//   program in it but the hidden ones.
//
// Relative and empty `PATH` entries are dropped on every system: they mean the
// current directory, which VSift itself refuses to search (SEC-02) and which
// the SEC-02 check plants a tool in on purpose.

const fs = require('node:fs');
const path = require('node:path');

/** The programs a no-Node step must not find: the runtimes and their package managers. */
const NODE_RUNTIME = Object.freeze(['node', 'nodejs', 'npm', 'npx', 'corepack', 'bun', 'bunx', 'pnpm', 'pnpx', 'yarn', 'yarnpkg']);

/** The programs that must not resolve in a scrubbed environment, by group. */
const TOOLCHAIN = Object.freeze({
  rust: ['cargo', 'rustc', 'rustup', 'rustdoc'],
  git: ['git'],
  python: ['python', 'python3', 'pythonw', 'pip', 'pip3', 'py'],
  node: NODE_RUNTIME,
});

/** File extensions a Windows shell appends when it looks a program up. */
const WINDOWS_EXTENSIONS = Object.freeze(['.com', '.exe', '.bat', '.cmd', '.ps1']);

/** The hidden names for a set of toolchain groups. */
function namesOf(groups) {
  return groups.flatMap((group) => {
    const names = TOOLCHAIN[group];
    if (names === undefined) {
      throw new Error(`unknown toolchain group ${group}`);
    }
    return names;
  });
}

/**
 * Whether the directory entry `file` is one of the hidden programs: the name
 * alone, or on Windows the name with an executable extension, or a versioned
 * name of the Python family (`python3.12`, `pip3.12`).
 */
function isHidden(file, hidden, platform = process.platform) {
  const caseless = platform === 'win32';
  let name = caseless ? file.toLowerCase() : file;
  if (caseless) {
    const extension = path.extname(name);
    if (WINDOWS_EXTENSIONS.includes(extension)) {
      name = name.slice(0, -extension.length);
    }
  }
  if (hidden.includes(name)) {
    return true;
  }
  // Versioned names of the Python family (`python3.12`, `pip3.12`).
  if (/^(python|pip)[0-9][0-9.]*$/.test(name) && (hidden.includes('python') || hidden.includes('pip'))) {
    return true;
  }
  // The rest of a Rust toolchain's own directory (`cargo-clippy`, `rustfmt`, `rust-analyzer`).
  return hidden.includes('cargo') && /^(cargo-|rustfmt|rust-|clippy-)/.test(name);
}

/** The key under which `env` holds the search path (`Path` on Windows, usually). */
function pathKey(env) {
  return Object.keys(env).find((key) => key.toLowerCase() === 'path') || 'PATH';
}

/** Splits a search path into its entries. */
function splitPath(value, platform = process.platform) {
  return value.split(platform === 'win32' ? ';' : ':');
}

function joinPath(entries, platform = process.platform) {
  return entries.join(platform === 'win32' ? ';' : ':');
}

/**
 * Finds the first file `name` resolves to in the search path `value`, as a
 * shell would, or null. Relative and empty entries are not searched.
 */
function resolveOnPath(name, value, platform = process.platform) {
  for (const entry of splitPath(value, platform)) {
    if (entry === '' || !path.isAbsolute(entry)) {
      continue;
    }
    let files;
    try {
      files = fs.readdirSync(entry);
    } catch {
      continue;
    }
    for (const file of files) {
      const stem = platform === 'win32' ? file.toLowerCase() : file;
      const wanted = platform === 'win32' ? name.toLowerCase() : name;
      const matches =
        stem === wanted ||
        (platform === 'win32' && WINDOWS_EXTENSIONS.some((extension) => stem === `${wanted}${extension}`));
      if (matches) {
        return path.join(entry, file);
      }
    }
  }
  return null;
}

/**
 * Builds a scrubbed search path.
 *
 * @param {string} value the original search path
 * @param {string[]} hidden program names that must not resolve in the result
 * @param {{farm: string, platform?: string}} options `farm` is an empty or
 *   absent directory under which a replacement directory is made when a
 *   shared directory has to be filtered (never on Windows)
 * @returns {{path: string, kept: string[], dropped: string[], filtered: string[]}}
 */
function scrubPath(value, hidden, { farm, platform = process.platform }) {
  const kept = [];
  const dropped = [];
  const filtered = [];
  const result = [];
  let index = 0;
  for (const entry of splitPath(value, platform)) {
    if (entry === '' || !path.isAbsolute(entry)) {
      if (entry !== '') {
        dropped.push(entry);
      }
      continue;
    }
    let files;
    try {
      files = fs.readdirSync(entry);
    } catch {
      // A directory that does not exist or cannot be read holds nothing to hide.
      kept.push(entry);
      result.push(entry);
      continue;
    }
    const hiding = files.filter((file) => isHidden(file, hidden, platform));
    if (hiding.length === 0) {
      kept.push(entry);
      result.push(entry);
      continue;
    }
    if (platform === 'win32') {
      dropped.push(entry);
      continue;
    }
    index += 1;
    const replacement = path.join(farm, `${String(index).padStart(2, '0')}-${path.basename(entry) || 'root'}`);
    fs.mkdirSync(replacement, { recursive: true });
    for (const file of files) {
      if (!isHidden(file, hidden, platform)) {
        fs.symlinkSync(path.join(entry, file), path.join(replacement, file));
      }
    }
    filtered.push(`${entry} (without ${hiding.join(', ')})`);
    result.push(replacement);
  }
  return { path: joinPath(result, platform), kept, dropped, filtered };
}

/**
 * Returns a copy of `env` whose search path hides `hidden`, and the facts to
 * record. Throws when a hidden name still resolves.
 */
function scrubbedEnvironment(env, hidden, options) {
  const key = pathKey(env);
  const original = env[key] || '';
  const scrubbed = scrubPath(original, hidden, options);
  const result = { ...env };
  for (const other of Object.keys(result)) {
    if (other.toLowerCase() === 'path') {
      delete result[other];
    }
  }
  result[key] = scrubbed.path;
  const platform = options.platform || process.platform;
  const resolved = hidden.filter((name) => resolveOnPath(name, scrubbed.path, platform) !== null);
  if (resolved.length > 0) {
    throw new Error(`still on PATH after the scrub: ${resolved.join(', ')}`);
  }
  return { env: result, ...scrubbed, key };
}

/** One line per name saying where it resolves in `value`, or that it does not. */
function describeResolution(names, value, platform = process.platform) {
  return names.map((name) => `${name}: ${resolveOnPath(name, value, platform) || 'not found'}`);
}

/** Puts `directories` ahead of the search path in `env` (returns a new object). */
function withPathPrefix(env, directories) {
  const key = pathKey(env);
  const platform = process.platform;
  const result = { ...env };
  result[key] = joinPath([...directories, ...(env[key] ? splitPath(env[key], platform) : [])], platform);
  return result;
}

module.exports = {
  NODE_RUNTIME,
  TOOLCHAIN,
  describeResolution,
  isHidden,
  joinPath,
  namesOf,
  pathKey,
  resolveOnPath,
  scrubPath,
  scrubbedEnvironment,
  splitPath,
  withPathPrefix,
};
