// What the hand-written guide pages promise about the code, held to the code
// (P14 PR 9b; the second CI check of docs/planning/user-guide-spec.md):
//
// - `docs/guide/index.md` says which version the guide was checked against, and it is the
//   release of the binary under test (spec rule 7): `0.2.0` for `0.2.0-rc.1` and for `0.2.0`,
//   so the stable commit needs no guide change (the release process, section 6.8);
// - `docs/guide/troubleshooting.md` has a row for every failure code of the v1 schema, with
//   the exit status the contract gives that code;
// - every relative link in the guide leads to a file that exists;
// - every page of the guide is in the public-claims registry's scanned documents (spec rule 2).
//
// Each function returns a list of problems; an empty list means the promise holds.

'use strict';

const fs = require('node:fs');
const path = require('node:path');

// A forward-slash path, so messages read the same on every system.
const GUIDE = 'docs/guide';
const GUIDE_PATH = GUIDE;
const VERSION_MARKER = /<!--\s*guide-version:\s*(\S+)\s*-->/;

function readText(file) {
  return fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n');
}

/** Every Markdown file below `directory`, as paths relative to `root`, sorted. */
function markdownPages(root, directory) {
  const found = [];
  const visit = (relative) => {
    for (const entry of fs.readdirSync(path.join(root, relative), { withFileTypes: true })) {
      const child = path.join(relative, entry.name);
      if (entry.isDirectory()) visit(child);
      else if (entry.name.endsWith('.md')) found.push(child.split(path.sep).join('/'));
    }
  };
  if (fs.existsSync(path.join(root, directory))) visit(directory);
  return found.sort();
}

/** The version `docs/guide/index.md` says the guide was checked against. */
function guideVersion(root) {
  const file = path.join(root, GUIDE, 'index.md');
  if (!fs.existsSync(file)) return { problem: `${GUIDE_PATH}/index.md does not exist` };
  const match = VERSION_MARKER.exec(readText(file));
  if (!match) return { problem: `${GUIDE_PATH}/index.md has no <!-- guide-version: X --> line saying which version the guide was checked against` };
  if (!/^\d+\.\d+\.\d+$/.test(match[1])) {
    return { problem: `${GUIDE_PATH}/index.md names ${match[1]} in <!-- guide-version: X -->; write the release without a candidate suffix (for example 0.2.0, not 0.2.0-rc.1), because the stable commit may not change this page` };
  }
  return { version: match[1] };
}

/** The exit status of each failure code, read from the contract's taxonomy table. */
function contractExitStatuses(root) {
  const contract = readText(path.join(root, 'docs', 'contracts', 'cli-v1.md'));
  const start = contract.indexOf('## Exit and error taxonomy');
  if (start < 0) return new Map();
  const section = contract.slice(start, contract.indexOf('\n## ', start + 1) > 0 ? contract.indexOf('\n## ', start + 1) : undefined);
  const statuses = new Map();
  for (const line of section.split('\n')) {
    const row = /^\|\s*(\d+)\s*\|.*\|\s*([^|]*)\|\s*$/.exec(line);
    if (!row) continue;
    for (const code of row[2].match(/`([A-Z_]+)`/g) || []) statuses.set(code.replace(/`/g, ''), Number(row[1]));
  }
  return statuses;
}

/** The failure codes of the v1 terminal response. */
function schemaFailureCodes(root) {
  const file = path.join(root, 'schemas', 'v1', 'operation-response.schema.json');
  const schema = JSON.parse(fs.readFileSync(file, 'utf8'));
  return schema.$defs.error.properties.code.enum;
}

/** The rows `| `CODE` | 7 | ...` of the troubleshooting page, as code to exit status. */
function troubleshootingRows(root) {
  const file = path.join(root, GUIDE, 'troubleshooting.md');
  if (!fs.existsSync(file)) return undefined;
  const rows = new Map();
  for (const line of readText(file).split('\n')) {
    const row = /^\|\s*`([A-Z_]+)`\s*\|\s*(\d+)\s*\|/.exec(line);
    if (row) rows.set(row[1], Number(row[2]));
  }
  return rows;
}

function troubleshootingProblems(root) {
  const rows = troubleshootingRows(root);
  if (rows === undefined) return [`${GUIDE}/troubleshooting.md does not exist`];
  const contract = contractExitStatuses(root);
  const problems = [];
  for (const code of schemaFailureCodes(root)) {
    if (!rows.has(code)) {
      problems.push(`${GUIDE}/troubleshooting.md has no row for the failure code ${code} (a row starts \`| \`${code}\` | <exit status> |\`)`);
    } else if (contract.has(code) && contract.get(code) !== rows.get(code)) {
      problems.push(`${GUIDE}/troubleshooting.md gives ${code} exit status ${rows.get(code)}, but the contract's taxonomy says ${contract.get(code)}`);
    } else if (!contract.has(code)) {
      problems.push(`the contract's exit taxonomy does not list ${code}`);
    }
  }
  for (const code of rows.keys()) {
    if (!schemaFailureCodes(root).includes(code)) problems.push(`${GUIDE}/troubleshooting.md has a row for ${code}, which is not a v1 failure code`);
  }
  return problems;
}

/** The anchor a Markdown heading gets on the common sites: lower case, words joined by hyphens. */
function slugOf(heading) {
  return heading
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/<[^>]*>/g, '')
    .replace(/[`*]/g, '')
    .toLowerCase()
    .replace(/[^\p{L}\p{N} _-]/gu, '')
    .trim()
    .replace(/ /g, '-');
}

/** Every anchor a page offers: its headings (repeats numbered as the sites do) and explicit ids. */
function anchorsOf(markdown) {
  const anchors = new Set();
  const seen = new Map();
  const outside = markdown.replace(/\r\n/g, '\n').replace(/^```[\s\S]*?^```/gm, '');
  for (const line of outside.split('\n')) {
    const heading = /^#{1,6}\s+(.*?)\s*#*\s*$/.exec(line);
    if (heading) {
      const slug = slugOf(heading[1]);
      const count = seen.get(slug) || 0;
      seen.set(slug, count + 1);
      anchors.add(count === 0 ? slug : `${slug}-${count}`);
    }
  }
  for (const match of outside.matchAll(/\bid="([^"]+)"/g)) anchors.add(match[1]);
  return anchors;
}

/** Relative links of the guide that lead nowhere, or to a heading that is not there. */
function linkProblems(root) {
  const problems = [];
  const anchorCache = new Map();
  const anchorsIn = (file) => {
    if (!anchorCache.has(file)) anchorCache.set(file, anchorsOf(readText(file)));
    return anchorCache.get(file);
  };
  for (const page of markdownPages(root, GUIDE)) {
    const text = readText(path.join(root, page));
    // Fenced blocks and code spans hold examples (a regular expression can look like a link), not links.
    const prose = text.replace(/^```[\s\S]*?^```/gm, '').replace(/`[^`\n]*`/g, '');
    for (const match of prose.matchAll(/\]\(([^)\s]+)\)/g)) {
      const target = match[1];
      if (/^[a-z][a-z0-9+.-]*:/i.test(target)) continue;
      const [file, fragment] = target.split('#');
      const resolved = file === '' ? path.join(root, page) : path.resolve(root, path.dirname(page), decodeURIComponent(file));
      if (!fs.existsSync(resolved)) {
        problems.push(`${page} links to ${target}, which does not exist`);
      } else if (fragment && resolved.endsWith('.md') && !anchorsIn(resolved).has(decodeURIComponent(fragment))) {
        problems.push(`${page} links to ${target}, but that page has no heading with that anchor`);
      }
    }
  }
  return problems;
}

/** Guide pages missing from the claims registry's scanned documents. */
function registryProblems(root) {
  const registry = JSON.parse(readText(path.join(root, 'docs', 'planning', 'public-claims.json')));
  const scanned = new Set(registry.documents);
  return markdownPages(root, GUIDE)
    .filter((page) => !scanned.has(page))
    .map((page) => `${page} is not in the documents of docs/planning/public-claims.json (the claims ladder applies to the guide: user-guide-spec rule 2)`);
}

/** Every problem of the hand-written pages; `version` is the binary's. */
function check(root, version) {
  const problems = [];
  const declared = guideVersion(root);
  if (declared.problem) problems.push(declared.problem);
  else if (declared.version !== version) {
    problems.push(`${GUIDE}/index.md says the guide was checked against ${declared.version}, but the binary under test is ${version}: re-check the guide, then update <!-- guide-version: ${version} -->`);
  }
  problems.push(...troubleshootingProblems(root), ...linkProblems(root), ...registryProblems(root));
  return problems;
}

module.exports = {
  anchorsOf,
  check,
  contractExitStatuses,
  guideVersion,
  linkProblems,
  slugOf,
  markdownPages,
  registryProblems,
  schemaFailureCodes,
  troubleshootingProblems,
  troubleshootingRows,
};
