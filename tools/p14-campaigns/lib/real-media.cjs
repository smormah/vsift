'use strict';

// The real-media set (#363, docs/planning/real-media-set.md): its pinned
// manifest, and fetching an approved item over HTTPS into a folder outside the
// repository, bounded by its pinned size and verified by its pinned SHA-256
// (and by the checksum its source publishes) before it is placed. The media is
// never committed; the manifest is. The command line is real-media.cjs.
//
// Everything here is pure or takes its network transport as an argument, so
// the tests fetch from local bytes and never touch the network.

const crypto = require('node:crypto');
const fs = require('node:fs');
const https = require('node:https');
const path = require('node:path');
const { Readable } = require('node:stream');

/** The manifest's own version; a reader refuses any other. */
const SCHEMA_VERSION = '1';
const SET_ID = 'vsift-real-media';

/** No item may be larger: the set is meant to stay small (aim: a few hundred MB in all). */
const MAX_ITEM_BYTES = 300 * 1024 * 1024;
/** Every item that is not withdrawn, together. */
const MAX_SET_BYTES = 750 * 1024 * 1024;
/**
 * The set itself, its approved items: at most nine, and 500 MB together (the maintainer's limits of 2026-10-19;
 * nine since the approval of 2026-10-21 to add a recording of two or three speakers, RM11).
 */
const MAX_APPROVED_ITEMS = 9;
const MAX_APPROVED_BYTES = 500_000_000;
const MAX_REDIRECTS = 5;
/** A transfer that sends nothing for this long is abandoned. */
const IDLE_TIMEOUT_MS = 60_000;
/**
 * The client name every request sends. It names the tool only: never a person,
 * an e-mail address or an account (the maintainer's rule; a source's courtesy
 * convention of contact details in the user agent is not followed).
 */
const USER_AGENT = 'vsift-real-media-fetch/1';

/**
 * The hosts an item's source `url` may name: sources with stable, versioned
 * URLs (docs/planning/real-media-set.md section 3). A host is added in the
 * reviewed change that adds an item from it.
 */
const SOURCE_HOSTS = Object.freeze(['upload.wikimedia.org', 'archive.org', 'images-assets.nasa.gov']);
/** The Internet Archive's storage nodes (ia*.us, dn*.ca, dn*.eu ...), to which archive.org/download redirects. */
const SOURCE_HOST_SUFFIXES = Object.freeze(['.archive.org']);
/**
 * Sources that refuse the tool's neutral user agent (Wikimedia answers its
 * downloads with HTTP 429 until a client gives contact details, which the
 * maintainer declined on 2026-10-19). Their `url` is kept as provenance only and
 * is never contacted: such an item is fetched from the mirror, or pinned from a
 * copy the maintainer downloaded by hand (`pin --from-file`). A source that
 * refuses is never retried with another user agent.
 */
const PROVENANCE_ONLY_HOSTS = Object.freeze(['upload.wikimedia.org']);
/** The project's own test-media release, the only mirror; every hosted run fetches from it. */
const MIRROR_PREFIX = 'https://github.com/smormah/vsift/releases/download/test-media-';
/** Where GitHub sends a release-asset download. */
const MIRROR_REDIRECT_HOSTS = Object.freeze(['release-assets.githubusercontent.com', 'objects.githubusercontent.com']);
/** Every host the tool contacts, for the first request or a redirect. */
const ALLOWED_HOSTS = Object.freeze([...SOURCE_HOSTS.filter((host) => !PROVENANCE_ONLY_HOSTS.includes(host)), 'github.com', ...MIRROR_REDIRECT_HOSTS]);

const STATUSES = Object.freeze(['proposed', 'approved', 'withdrawn']);
/**
 * The shapes of real media the set must cover, each tied to the findings it is
 * meant to settle (docs/planning/real-media-set.md section 2).
 */
const COVERS = Object.freeze([
  'conversational-pauses',
  'long-speech-seams',
  'screen-ui-small-text',
  'noisy-speech',
  'non-english-or-accented',
  'leading-silence',
  'lecture-or-meeting',
  'two-or-three-speakers',
  'vertical-variable-frame-rate',
]);
/** The shapes the set must hold before it is used as evidence; the vertical phone video is wanted but optional. */
const REQUIRED_COVERS = Object.freeze(COVERS.filter((cover) => cover !== 'vertical-variable-frame-rate'));
const SOURCE_ALGORITHMS = Object.freeze({ sha1: 40, md5: 32 });

const TOP_KEYS = Object.freeze(['schema_version', 'set_id', 'items']);
const ITEM_REQUIRED = Object.freeze([
  'id', 'status', 'title', 'source_page', 'url', 'file_name', 'bytes', 'sha256',
  'source_checksum', 'licence', 'author', 'attribution', 'media', 'covers', 'tests',
]);
const ITEM_OPTIONAL = Object.freeze(['source_revision', 'mirror_url', 'approval', 'labels', 'notes']);
/** A private item (the maintainer's own recording, local use only): no source, licence or title is recorded. */
const PRIVATE_REQUIRED = Object.freeze(['id', 'status', 'private', 'file_name', 'bytes', 'sha256', 'covers', 'approval']);
const PRIVATE_OPTIONAL = Object.freeze(['labels', 'notes', 'tests']);
const PRIVATE_SET_ID = 'vsift-real-media-private';
const PRIVATE_ID = /^P[0-9]{2}$/;
const LICENCE_KEYS = Object.freeze(['id', 'url', 'statement']);
const LICENCE_ID = /^(CC0-1\.0|CC-BY-[1-4]\.0|CC-BY-SA-[1-4]\.0|public-domain|public-domain-us-government)$/;
const MEDIA_KEYS = Object.freeze(['duration_s', 'container', 'video', 'audio']);
const ITEM_ID = /^RM[0-9]{2}$/;
const FILE_NAME = /^RM[0-9]{2}-[a-z0-9]+(?:-[a-z0-9]+)*\.(webm|ogv|mp4|mkv|mov|m4a|ogg|opus|wav|flac|mp3)$/;
const SHA256 = /^[0-9a-f]{64}$/;
const DATE = /^[0-9]{4}-[0-9]{2}-[0-9]{2}$/;
const TEST_REFERENCE = /^(#[0-9]{1,5}|L-[0-9]{3})$/;

/** The kinds of refusal; each is stable so a caller and a test can tell them apart. */
const ErrorCode = Object.freeze({
  MANIFEST_INVALID: 'manifest_invalid',
  NOT_APPROVED: 'not_approved',
  NOT_PINNED: 'not_pinned',
  ALREADY_PINNED: 'already_pinned',
  DESTINATION_REFUSED: 'destination_refused',
  TRANSPORT_REFUSED: 'transport_refused',
  SIZE_MISMATCH: 'size_mismatch',
  DIGEST_MISMATCH: 'digest_mismatch',
  EXISTING_DIFFERS: 'existing_differs',
  PARTIAL_PRESENT: 'partial_present',
  SOURCE_FILE_REFUSED: 'source_file_refused',
  PRIVATE_REFUSED: 'private_refused',
  NOT_FOUND: 'not_found',
});

/** A typed refusal: `code` is one of `ErrorCode`, the message is fixed prose plus the item id. */
class RealMediaError extends Error {
  /**
   * @param {string} code one of ErrorCode
   * @param {string} message
   */
  constructor(code, message) {
    super(message);
    this.name = 'RealMediaError';
    this.code = code;
  }
}

function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function unknownKeys(value, allowed) {
  return Object.keys(value).filter((key) => !allowed.includes(key));
}

/**
 * Whether a URL may be fetched or redirected to: HTTPS, no credentials, no
 * port other than the default, and a host on the allowlist.
 * @param {string} text
 * @returns {boolean}
 */
function parsedHttps(text) {
  let url;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  if (url.protocol !== 'https:' || url.username || url.password || url.port) return null;
  return url;
}

function archiveHost(host) {
  return SOURCE_HOST_SUFFIXES.some((suffix) => host.endsWith(suffix));
}

/**
 * Whether a URL may be named as an item's source: HTTPS, no credentials, no
 * port, on a source host (a provenance-only host included).
 * @param {string} text
 * @returns {boolean}
 */
function sourceUrl(text) {
  const url = parsedHttps(text);
  if (!url) return false;
  const host = url.hostname.toLowerCase();
  return SOURCE_HOSTS.includes(host) || archiveHost(host);
}

/**
 * Whether a URL may be contacted, as a first request or a redirect: HTTPS, no
 * credentials, no port, on a host that accepts the tool (never a
 * provenance-only one); on github.com only the test-media release.
 * @param {string} text
 * @returns {boolean}
 */
function allowedUrl(text) {
  const url = parsedHttps(text);
  if (!url) return false;
  const host = url.hostname.toLowerCase();
  if (host === 'github.com') return url.href.startsWith(MIRROR_PREFIX);
  return ALLOWED_HOSTS.includes(host) || archiveHost(host);
}

/** The URL a fetch contacts: the mirror when there is one, else the source. */
function fetchUrl(item) {
  return item.mirror_url || item.url;
}

function httpsPage(text) {
  try {
    const url = new URL(text);
    return url.protocol === 'https:' && !url.username && !url.password;
  } catch {
    return false;
  }
}

function checkItem(item, index, problems) {
  const where = `items[${index}]`;
  if (!isObject(item)) {
    problems.push(`${where}: not an object`);
    return;
  }
  for (const key of ITEM_REQUIRED) if (!(key in item)) problems.push(`${where}: missing ${key}`);
  for (const key of unknownKeys(item, [...ITEM_REQUIRED, ...ITEM_OPTIONAL])) problems.push(`${where}: unknown member ${key}`);
  const id = typeof item.id === 'string' && ITEM_ID.test(item.id) ? item.id : null;
  if (!id) problems.push(`${where}: id must match RM followed by two digits`);
  const label = id || where;
  if (!STATUSES.includes(item.status)) problems.push(`${label}: status must be one of ${STATUSES.join(', ')}`);
  for (const key of ['title', 'author', 'attribution']) {
    if (typeof item[key] !== 'string' || item[key].trim().length === 0) problems.push(`${label}: ${key} must be a non-empty string`);
  }
  if (typeof item.source_page !== 'string' || !httpsPage(item.source_page)) problems.push(`${label}: source_page must be an HTTPS URL`);
  if (typeof item.url !== 'string' || !sourceUrl(item.url)) problems.push(`${label}: url must be HTTPS on an allowed host`);
  if ('mirror_url' in item && (typeof item.mirror_url !== 'string' || !item.mirror_url.startsWith(MIRROR_PREFIX)
    || !allowedUrl(item.mirror_url) || !item.mirror_url.endsWith(`/${item.file_name}`))) {
    problems.push(`${label}: mirror_url must be the test-media release's asset of the item's file_name`);
  }
  if (typeof item.file_name !== 'string' || !FILE_NAME.test(item.file_name)) {
    problems.push(`${label}: file_name must be the item id, a lowercase slug and a media extension`);
  } else if (id && !item.file_name.startsWith(`${id}-`)) {
    problems.push(`${label}: file_name must start with the item id`);
  }
  if (!Number.isSafeInteger(item.bytes) || item.bytes < 1 || item.bytes > MAX_ITEM_BYTES) {
    problems.push(`${label}: bytes must be an integer from 1 to ${MAX_ITEM_BYTES}`);
  }
  if (item.sha256 !== null && (typeof item.sha256 !== 'string' || !SHA256.test(item.sha256))) {
    problems.push(`${label}: sha256 must be null or 64 lowercase hexadecimal digits`);
  }
  const checksum = item.source_checksum;
  if (checksum !== null) {
    if (!isObject(checksum) || unknownKeys(checksum, ['algorithm', 'value']).length > 0
      || !(checksum.algorithm in SOURCE_ALGORITHMS) || typeof checksum.value !== 'string'
      || !new RegExp(`^[0-9a-f]{${SOURCE_ALGORITHMS[checksum.algorithm] || 0}}$`).test(checksum.value)) {
      problems.push(`${label}: source_checksum must be null or {algorithm: sha1|md5, value: lowercase hex}`);
    }
  }
  const licence = item.licence;
  if (!isObject(licence) || unknownKeys(licence, LICENCE_KEYS).length > 0 || !LICENCE_ID.test(String(licence.id))
    || !httpsPage(String(licence.url)) || !httpsPage(String(licence.statement))) {
    problems.push(`${label}: licence must be {id, url, statement} with a permitted id and HTTPS links`);
  }
  const media = item.media;
  if (!isObject(media) || unknownKeys(media, MEDIA_KEYS).length > 0 || typeof media.duration_s !== 'number'
    || !(media.duration_s > 0) || typeof media.container !== 'string'
    || (media.video !== null && typeof media.video !== 'string') || (media.audio !== null && typeof media.audio !== 'string')) {
    problems.push(`${label}: media must be {duration_s > 0, container, video (string or null), audio (string or null)}`);
  }
  if (!Array.isArray(item.covers) || item.covers.length === 0 || item.covers.some((cover) => !COVERS.includes(cover))
    || new Set(item.covers).size !== item.covers.length) {
    problems.push(`${label}: covers must be a non-empty list of distinct known shapes`);
  }
  if (!Array.isArray(item.tests) || item.tests.length === 0 || item.tests.some((reference) => typeof reference !== 'string' || !TEST_REFERENCE.test(reference))) {
    problems.push(`${label}: tests must list issue numbers (#n) or register entries (L-nnn)`);
  }
  if ('source_revision' in item && typeof item.source_revision !== 'string') problems.push(`${label}: source_revision must be a string`);
  if ('notes' in item && typeof item.notes !== 'string') problems.push(`${label}: notes must be a string`);
  if (item.status === 'approved') {
    const approval = item.approval;
    if (!isObject(approval) || unknownKeys(approval, ['date', 'by']).length > 0 || !DATE.test(String(approval.date))
      || approval.by !== 'maintainer') {
      problems.push(`${label}: an approved item needs approval {date: YYYY-MM-DD, by: "maintainer"}`);
    }
  } else if ('approval' in item) {
    problems.push(`${label}: only an approved item carries approval`);
  }
  if ('labels' in item) {
    const labels = item.labels;
    if (!isObject(labels) || unknownKeys(labels, ['speech_start_us', 'changes_us', 'reference_transcript', 'note']).length > 0
      || ('speech_start_us' in labels && !(Number.isSafeInteger(labels.speech_start_us) && labels.speech_start_us >= 0))
      || ('changes_us' in labels && !(Array.isArray(labels.changes_us) && labels.changes_us.every((at) => Number.isSafeInteger(at) && at >= 0)))
      || ('reference_transcript' in labels && !httpsPage(String(labels.reference_transcript)))
      || ('note' in labels && typeof labels.note !== 'string')) {
      problems.push(`${label}: labels must be {speech_start_us?, changes_us?, reference_transcript?, note?}`);
    }
  }
}

const PRIVATE_FILE_NAME = /^[^\\/:*?"<>|\x00-\x1f]{1,200}$/;

/**
 * The problems of a private item. Its messages name members only, never a
 * value: a private manifest's file names and notes must not reach a log.
 */
function checkPrivateItem(item, index, problems) {
  const where = `items[${index}]`;
  if (!isObject(item)) {
    problems.push(`${where}: not an object`);
    return;
  }
  for (const key of PRIVATE_REQUIRED) if (!(key in item)) problems.push(`${where}: missing ${key}`);
  for (const key of unknownKeys(item, [...PRIVATE_REQUIRED, ...PRIVATE_OPTIONAL])) {
    problems.push(`${where}: a member not allowed in a private item (only ${[...PRIVATE_REQUIRED, ...PRIVATE_OPTIONAL].join(', ')})`);
    break;
  }
  const label = typeof item.id === 'string' && PRIVATE_ID.test(item.id) ? item.id : where;
  if (label === where) problems.push(`${where}: a private id is P followed by two digits`);
  if (item.private !== true) problems.push(`${label}: private must be true`);
  if (item.status !== 'approved') problems.push(`${label}: a private item is approved (the maintainer's consent) or left out`);
  const approval = item.approval;
  if (!isObject(approval) || unknownKeys(approval, ['date', 'by']).length > 0 || !DATE.test(String(approval.date)) || approval.by !== 'maintainer') {
    problems.push(`${label}: approval must be {date: YYYY-MM-DD, by: "maintainer"}`);
  }
  if (typeof item.file_name !== 'string' || !PRIVATE_FILE_NAME.test(item.file_name) || item.file_name.startsWith('.') || item.file_name.trim() !== item.file_name) {
    problems.push(`${label}: file_name must be a plain file name beside the private manifest`);
  }
  if (!Number.isSafeInteger(item.bytes) || item.bytes < 1) problems.push(`${label}: bytes must be a positive integer`);
  if (item.sha256 !== null && (typeof item.sha256 !== 'string' || !SHA256.test(item.sha256))) problems.push(`${label}: sha256 must be null or 64 lowercase hexadecimal digits`);
  if (!Array.isArray(item.covers) || item.covers.length === 0 || item.covers.some((cover) => !COVERS.includes(cover))) problems.push(`${label}: covers must list known shapes`);
  if ('notes' in item && typeof item.notes !== 'string') problems.push(`${label}: notes must be a string`);
}

/**
 * Every problem with a manifest, as fixed prose naming the member; an empty
 * list means it is valid. The rules are those of
 * `real-media/manifest.schema.json` (a test keeps the required members of both
 * in step) plus what a schema cannot say: unique ids and file names, the set's
 * total size and what an approval needs.
 * @param {unknown} manifest the parsed JSON
 * @returns {string[]}
 */
function manifestProblems(manifest, { privateSet = false } = {}) {
  const problems = [];
  if (!isObject(manifest)) return ['the manifest must be a JSON object'];
  for (const key of TOP_KEYS) if (!(key in manifest)) problems.push(`missing ${key}`);
  for (const key of unknownKeys(manifest, TOP_KEYS)) problems.push(`unknown member ${key}`);
  if (manifest.schema_version !== SCHEMA_VERSION) problems.push(`schema_version must be "${SCHEMA_VERSION}"`);
  const setId = privateSet ? PRIVATE_SET_ID : SET_ID;
  if (manifest.set_id !== setId) problems.push(`set_id must be "${setId}"`);
  if (!Array.isArray(manifest.items)) return [...problems, 'items must be a list'];
  if (privateSet) {
    manifest.items.forEach((item, index) => checkPrivateItem(item, index, problems));
    for (const key of ['id', 'file_name']) {
      const values = manifest.items.filter(isObject).map((item) => item[key]);
      if (new Set(values).size !== values.length) problems.push(`a ${key} is used twice`);
    }
    return problems;
  }
  manifest.items.forEach((item, index) => {
    if (isObject(item) && 'private' in item) problems.push(`items[${index}]: a private item belongs in a private manifest outside the repository, never here`);
    else checkItem(item, index, problems);
  });
  const items = manifest.items.filter(isObject);
  for (const key of ['id', 'file_name', 'url']) {
    const seen = new Set();
    for (const item of items) {
      if (seen.has(item[key])) problems.push(`${key} ${item[key]} is used twice`);
      seen.add(item[key]);
    }
  }
  const total = items.filter((item) => item.status !== 'withdrawn' && Number.isSafeInteger(item.bytes))
    .reduce((sum, item) => sum + item.bytes, 0);
  if (total > MAX_SET_BYTES) problems.push(`the items that are not withdrawn hold ${total} bytes, more than ${MAX_SET_BYTES}`);
  const approved = items.filter((item) => item.status === 'approved' && Number.isSafeInteger(item.bytes));
  if (approved.length > MAX_APPROVED_ITEMS) problems.push(`${approved.length} items are approved, more than ${MAX_APPROVED_ITEMS}`);
  const approvedBytes = approved.reduce((sum, item) => sum + item.bytes, 0);
  if (approvedBytes > MAX_APPROVED_BYTES) problems.push(`the approved items hold ${approvedBytes} bytes, more than ${MAX_APPROVED_BYTES}`);
  return problems;
}

/**
 * The required shapes that no item that is not withdrawn covers.
 * @param {{ items: Array<{ status: string, covers: string[] }> }} manifest a valid manifest
 * @returns {string[]}
 */
function uncoveredShapes(manifest) {
  const covered = new Set(manifest.items.filter((item) => item.status !== 'withdrawn').flatMap((item) => item.covers));
  return REQUIRED_COVERS.filter((cover) => !covered.has(cover));
}

/**
 * Reads and validates a manifest file.
 * @param {string} file
 * @returns {object}
 */
function readManifest(file) {
  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    throw new RealMediaError(ErrorCode.MANIFEST_INVALID, 'the manifest could not be read as JSON');
  }
  const problems = manifestProblems(manifest);
  if (problems.length > 0) throw new RealMediaError(ErrorCode.MANIFEST_INVALID, `the manifest is invalid: ${problems.join('; ')}`);
  return manifest;
}

/**
 * Reads and validates a private manifest. It must be named by an absolute
 * path that lies outside the repository, and its problems are reported by
 * member, never with a value.
 * @param {string} file
 * @param {string} repository the repository's root
 * @returns {object}
 */
function readPrivateManifest(file, repository) {
  if (typeof file !== 'string' || !path.isAbsolute(file)) throw new RealMediaError(ErrorCode.PRIVATE_REFUSED, 'the private manifest must be named by an absolute path');
  let real;
  try {
    real = fs.realpathSync.native(file);
  } catch {
    throw new RealMediaError(ErrorCode.PRIVATE_REFUSED, 'the private manifest could not be read');
  }
  const root = fs.realpathSync.native(repository);
  if (inside(real, root)) throw new RealMediaError(ErrorCode.PRIVATE_REFUSED, 'the private manifest must lie outside the repository');
  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(real, 'utf8'));
  } catch {
    throw new RealMediaError(ErrorCode.MANIFEST_INVALID, 'the private manifest could not be read as JSON');
  }
  const problems = manifestProblems(manifest, { privateSet: true });
  if (problems.length > 0) throw new RealMediaError(ErrorCode.MANIFEST_INVALID, `the private manifest is invalid: ${problems.join('; ')}`);
  return manifest;
}

/**
 * Private items run on a local machine only: a run in CI (the CI or
 * GITHUB_ACTIONS variable set) is refused before anything is read.
 * @param {Record<string, string | undefined>} env
 */
function refuseHostedPrivateRun(env) {
  if (env.CI || env.GITHUB_ACTIONS) throw new RealMediaError(ErrorCode.PRIVATE_REFUSED, 'private items run on a local machine only, never in CI');
}

/**
 * A transport over a local file the maintainer downloaded by hand, for
 * `pin --from-file`: the same size bound and checksums apply as to a download.
 * The source must be an existing regular file named by an absolute path.
 * @param {string} file
 */
function localOpen(file) {
  if (typeof file !== 'string' || !path.isAbsolute(file)) throw new RealMediaError(ErrorCode.SOURCE_FILE_REFUSED, 'the local file must be named by an absolute path');
  let stat;
  try {
    stat = fs.statSync(file);
  } catch {
    throw new RealMediaError(ErrorCode.SOURCE_FILE_REFUSED, 'the local file could not be read');
  }
  if (!stat.isFile()) throw new RealMediaError(ErrorCode.SOURCE_FILE_REFUSED, 'the local file must be a regular file');
  // The file is opened only when the transfer is read, so a copy refused on its
  // size before reading leaves no open handle behind.
  async function* chunks() {
    const handle = await fs.promises.open(file, 'r');
    try {
      const buffer = Buffer.alloc(1 << 20);
      for (;;) {
        const { bytesRead } = await handle.read(buffer, 0, buffer.length, null);
        if (bytesRead === 0) return;
        yield Buffer.from(buffer.subarray(0, bytesRead));
      }
    } finally {
      await handle.close();
    }
  }
  return async () => ({ stream: Readable.from(chunks()), length: stat.size });
}

/**
 * Pins (or, when pinned already, verifies and places) an approved item from a
 * local copy downloaded by hand. A local copy is accepted only against the
 * checksum its source publishes, so an item without one is refused.
 * @param {object} item
 * @param {string} destination a folder `checkDestination` accepted
 * @param {string} file the local copy
 * @returns {Promise<{ sha256: string, outcome: string }>}
 */
async function pinFromFile(item, destination, file) {
  if (item.status !== 'approved') throw new RealMediaError(ErrorCode.NOT_APPROVED, `${item.id} is not approved`);
  if (item.source_checksum === null) {
    throw new RealMediaError(ErrorCode.SOURCE_FILE_REFUSED, `${item.id}: its source publishes no checksum, so a local copy cannot be checked against it`);
  }
  const open = localOpen(file);
  if (item.sha256 === null) return { sha256: await pinItem(item, destination, open, { local: true }), outcome: 'pinned' };
  return { sha256: item.sha256, outcome: await fetchItem(item, destination, open) };
}

/**
 * The items a fetch may take: each named id must exist and be approved; with
 * no id named, every approved item.
 * @param {object} manifest a valid manifest
 * @param {string[]} ids
 * @returns {object[]}
 */
function selectApproved(manifest, ids) {
  if (ids.length === 0) return manifest.items.filter((item) => item.status === 'approved');
  return ids.map((id) => {
    const item = manifest.items.find((candidate) => candidate.id === id);
    if (!item || item.status !== 'approved') {
      throw new RealMediaError(ErrorCode.NOT_APPROVED, `${id} is not an approved item; only an item the maintainer approved is fetched`);
    }
    return item;
  });
}

function inside(child, parent) {
  const relative = path.relative(parent, child);
  return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
}

/**
 * The real path of the folder the media goes into. It must be given as an
 * absolute path, exist, be a directory and lie outside the repository, so
 * that a fetched file can never be committed by accident.
 * @param {string} destination
 * @param {string} repository the repository's root
 * @returns {string}
 */
function checkDestination(destination, repository) {
  if (typeof destination !== 'string' || !path.isAbsolute(destination)) {
    throw new RealMediaError(ErrorCode.DESTINATION_REFUSED, 'the destination must be an absolute path');
  }
  let real;
  try {
    real = fs.realpathSync.native(destination);
  } catch {
    throw new RealMediaError(ErrorCode.DESTINATION_REFUSED, 'the destination must be an existing directory');
  }
  if (!fs.statSync(real).isDirectory()) throw new RealMediaError(ErrorCode.DESTINATION_REFUSED, 'the destination must be an existing directory');
  const root = fs.realpathSync.native(repository);
  if (inside(real, root) || inside(root, real)) {
    throw new RealMediaError(ErrorCode.DESTINATION_REFUSED, 'the destination must lie outside the repository, and must not contain it');
  }
  return real;
}

/**
 * What a refusal page says, as one short line of plain text: styles, scripts
 * and markup dropped, control and non-ASCII characters folded, and anything
 * the page echoes back about the client removed (Wikimedia's error page prints
 * the client's IP address), so a log never repeats it. It is data, never
 * followed.
 * @param {string} page
 * @returns {string}
 */
function refusalText(page) {
  return String(page)
    .replace(/<(style|script)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, ' ')
    .replace(/<[^>]*>/g, ' ')
    .replace(/[^\x20-\x7e]+/g, ' ')
    .replace(/Sensitive client information[\s\S]*$/i, ' ')
    .replace(/\b\d{1,3}(?:\.\d{1,3}){3}\b/g, '[address]')
    .replace(/\b[0-9a-f]{0,4}(?::[0-9a-f]{0,4}){2,7}\b/gi, '[address]')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 300);
}

/**
 * The transport the tool uses: Node's HTTPS client, refusing any URL or
 * redirect that `allowedUrl` refuses, following at most `MAX_REDIRECTS`.
 * @param {string} url
 * @returns {Promise<{ stream: import('node:stream').Readable, length: number | null }>}
 */
function httpsOpen(url) {
  return new Promise((resolve, reject) => {
    const follow = (current, redirects) => {
      if (!allowedUrl(current)) {
        const host = parsedHttps(current) ? new URL(current).hostname.toLowerCase() : '';
        reject(new RealMediaError(ErrorCode.TRANSPORT_REFUSED, PROVENANCE_ONLY_HOSTS.includes(host)
          ? 'this source refuses the tool, so it is never contacted: fetch the item from the test-media release (mirror_url) or pin a copy downloaded by hand with pin --from-file'
          : 'a request or redirect left HTTPS or the allowed hosts'));
        return;
      }
      const request = https.get(current, { headers: { 'User-Agent': USER_AGENT }, timeout: IDLE_TIMEOUT_MS }, (response) => {
        const status = response.statusCode || 0;
        if (status >= 300 && status < 400 && response.headers.location) {
          response.resume();
          if (redirects >= MAX_REDIRECTS) {
            reject(new RealMediaError(ErrorCode.TRANSPORT_REFUSED, 'too many redirects'));
            return;
          }
          follow(new URL(response.headers.location, current).href, redirects + 1);
          return;
        }
        if (status !== 200) {
          // A refusal's own text says why (a rate limit names its policy); it is
          // read up to 32 KiB and shown on one line as data, never followed.
          let text = '';
          response.setEncoding('utf8');
          response.on('data', (part) => {
            if (text.length < 32768) text += part;
          });
          response.on('end', () => {
            const retry = response.headers['retry-after'];
            const said = refusalText(text);
            // Not found is its own kind: on the mirror it means the asset is not uploaded yet.
            reject(new RealMediaError(status === 404 ? ErrorCode.NOT_FOUND : ErrorCode.TRANSPORT_REFUSED,
              `the server answered HTTP ${status}${retry ? ` (Retry-After: ${String(retry).slice(0, 40)})` : ''}${said ? `: ${said}` : ''}`));
          });
          return;
        }
        const header = response.headers['content-length'];
        response.setTimeout(IDLE_TIMEOUT_MS, () => response.destroy(new RealMediaError(ErrorCode.TRANSPORT_REFUSED, 'the transfer stalled')));
        resolve({ stream: response, length: header === undefined ? null : Number(header) });
      });
      request.on('timeout', () => request.destroy(new RealMediaError(ErrorCode.TRANSPORT_REFUSED, 'the server did not answer')));
      request.on('error', (error) => reject(error instanceof RealMediaError ? error
        : new RealMediaError(ErrorCode.TRANSPORT_REFUSED, 'the HTTPS transfer failed')));
    };
    follow(url, 0);
  });
}

function digestOfFile(file, algorithm) {
  const hash = crypto.createHash(algorithm);
  const descriptor = fs.openSync(file, 'r');
  try {
    const buffer = Buffer.alloc(1 << 20);
    let read;
    while ((read = fs.readSync(descriptor, buffer, 0, buffer.length, null)) > 0) hash.update(buffer.subarray(0, read));
  } finally {
    fs.closeSync(descriptor);
  }
  return hash.digest('hex');
}

/**
 * The text written beside each fetched file, so the attribution and licence
 * travel with the media wherever the folder is copied.
 * @param {object} item
 * @returns {string}
 */
function attributionText(item) {
  return [
    `${item.id}: ${item.title}`,
    `Attribution: ${item.attribution}`,
    `Author: ${item.author}`,
    `Licence: ${item.licence.id} (${item.licence.url}); stated at ${item.licence.statement}`,
    `Source: ${item.source_page}`,
    `File: ${item.url}${item.source_revision ? ` (revision ${item.source_revision})` : ''}`,
    'Fetched unchanged for testing VSift; see docs/planning/real-media-set.md.',
    '',
  ].join('\n');
}

/** Writes the attribution beside a placed file; nothing is written for a file that was refused. */
function writeAttribution(folder, item) {
  fs.writeFileSync(path.join(folder, 'ATTRIBUTION.txt'), attributionText(item));
}

/**
 * Streams one file to `<destination>/<id>/.<file>.partial`, never more than the
 * pinned size, hashing as it goes; returns the digests. The partial file is
 * created exclusively and removed again on any failure.
 */
async function download(item, partial, open, url) {
  const { stream, length } = await open(url);
  if (length !== null && length !== item.bytes) {
    stream.destroy();
    throw new RealMediaError(ErrorCode.SIZE_MISMATCH, `${item.id}: the server announced ${length} bytes, the manifest pins ${item.bytes}`);
  }
  const sha256 = crypto.createHash('sha256');
  const source = item.source_checksum ? crypto.createHash(item.source_checksum.algorithm) : null;
  let received = 0;
  let descriptor;
  try {
    descriptor = fs.openSync(partial, 'wx', 0o600);
  } catch {
    stream.destroy();
    throw new RealMediaError(ErrorCode.PARTIAL_PRESENT, `${item.id}: a partial file of an earlier run is in the way; remove it and retry`);
  }
  try {
    for await (const chunk of stream) {
      received += chunk.length;
      if (received > item.bytes) {
        stream.destroy();
        throw new RealMediaError(ErrorCode.SIZE_MISMATCH, `${item.id}: the transfer exceeds the pinned ${item.bytes} bytes`);
      }
      sha256.update(chunk);
      if (source) source.update(chunk);
      for (let written = 0; written < chunk.length;) written += fs.writeSync(descriptor, chunk, written);
    }
    fs.fsyncSync(descriptor);
  } catch (error) {
    fs.closeSync(descriptor);
    fs.rmSync(partial, { force: true });
    throw error instanceof RealMediaError ? error : new RealMediaError(ErrorCode.TRANSPORT_REFUSED, `${item.id}: the transfer failed`);
  }
  fs.closeSync(descriptor);
  if (received !== item.bytes) {
    fs.rmSync(partial, { force: true });
    throw new RealMediaError(ErrorCode.SIZE_MISMATCH, `${item.id}: received ${received} bytes, the manifest pins ${item.bytes}`);
  }
  const digests = { sha256: sha256.digest('hex'), source: source ? source.digest('hex') : null };
  if (item.source_checksum && digests.source !== item.source_checksum.value) {
    fs.rmSync(partial, { force: true });
    throw new RealMediaError(ErrorCode.DIGEST_MISMATCH, `${item.id}: the ${item.source_checksum.algorithm} the source publishes does not match`);
  }
  return digests;
}

/**
 * Fetches one approved, pinned item into `<destination>/<id>/<file_name>` with
 * its `ATTRIBUTION.txt`. A file already there is checked, never overwritten:
 * with the pinned digest it is reported `present`; with another it is refused.
 * @param {object} item an approved item with a pinned sha256
 * @param {string} destination a folder `checkDestination` accepted
 * @param {(url: string) => Promise<{ stream: AsyncIterable<Buffer> & { destroy(): void }, length: number | null }>} open
 * @returns {Promise<'fetched' | 'present'>}
 */
async function fetchItem(item, destination, open) {
  if (item.status !== 'approved') throw new RealMediaError(ErrorCode.NOT_APPROVED, `${item.id} is not approved`);
  if (item.sha256 === null) throw new RealMediaError(ErrorCode.NOT_PINNED, `${item.id} has no pinned sha256 yet; pin it first`);
  const folder = path.join(destination, item.id);
  const final = path.join(folder, item.file_name);
  fs.mkdirSync(folder, { recursive: true, mode: 0o700 });
  if (fs.existsSync(final)) {
    if (fs.statSync(final).size === item.bytes && digestOfFile(final, 'sha256') === item.sha256) {
      writeAttribution(folder, item);
      return 'present';
    }
    throw new RealMediaError(ErrorCode.EXISTING_DIFFERS, `${item.id}: a different file is already at its place; it is left as it is`);
  }
  const partial = path.join(folder, `.${item.file_name}.partial`);
  const digests = await download(item, partial, open, fetchUrl(item));
  if (digests.sha256 !== item.sha256) {
    fs.rmSync(partial, { force: true });
    throw new RealMediaError(ErrorCode.DIGEST_MISMATCH, `${item.id}: the SHA-256 does not match the pinned digest`);
  }
  fs.renameSync(partial, final);
  writeAttribution(folder, item);
  return 'fetched';
}

/**
 * Pins an approved item that has no SHA-256 yet: fetches it from its source
 * (never the mirror: a pin is anchored at the source) bounded by its size, verifies the checksum its source publishes where it publishes one,
 * keeps the file and returns the SHA-256 for the reviewer to write into the
 * manifest. Without a source checksum (a Commons transcode, for which the
 * source publishes none) the pin is trust on first use over HTTPS, which the
 * design records; every later fetch is held to it.
 * @param {object} item an approved item with no sha256
 * @param {string} destination a folder `checkDestination` accepted
 * @param {(url: string) => Promise<{ stream: AsyncIterable<Buffer> & { destroy(): void }, length: number | null }>} open
 * @returns {Promise<string>} the SHA-256
 */
/**
 * Where a pin fetches from: the source, unless the source refuses the tool;
 * then the mirror, which is checked against the checksum the source publishes
 * (so a copy on the mirror is held to the source all the same). An item whose
 * source refuses the tool and publishes no checksum cannot be pinned this way.
 */
function pinUrl(item) {
  if (allowedUrl(item.url)) return item.url;
  if (item.mirror_url && item.source_checksum) return item.mirror_url;
  throw new RealMediaError(ErrorCode.NOT_PINNED, `${item.id}: its source refuses the tool and there is no mirror copy with a published checksum to check; pin a hand-downloaded copy with --from-file`);
}

/**
 * How the hosted cross-check reports an item it could not take: an asset not
 * yet uploaded to the mirror (or a source the tool may not contact, with no
 * mirror copy yet) is `not_yet_mirrored`, which is not a failure; anything else
 * is `refused`.
 * @param {object} item
 * @param {unknown} error
 * @returns {'not_yet_mirrored' | 'refused'}
 */
function crossCheckOutcome(item, error) {
  if (!(error instanceof RealMediaError)) return 'refused';
  if (error.code === ErrorCode.NOT_FOUND && item.mirror_url) return 'not_yet_mirrored';
  return 'refused';
}

async function pinItem(item, destination, open, { local = false } = {}) {
  if (item.status !== 'approved') throw new RealMediaError(ErrorCode.NOT_APPROVED, `${item.id} is not approved`);
  if (item.sha256 !== null) throw new RealMediaError(ErrorCode.ALREADY_PINNED, `${item.id} is pinned already; fetch it instead`);
  const folder = path.join(destination, item.id);
  const final = path.join(folder, item.file_name);
  fs.mkdirSync(folder, { recursive: true, mode: 0o700 });
  if (fs.existsSync(final)) throw new RealMediaError(ErrorCode.EXISTING_DIFFERS, `${item.id}: a file is already at its place; it is left as it is`);
  const partial = path.join(folder, `.${item.file_name}.partial`);
  const digests = await download(item, partial, open, local ? null : pinUrl(item));
  fs.renameSync(partial, final);
  writeAttribution(folder, item);
  return digests.sha256;
}

/**
 * The test-media release, prepared for the maintainer's review (the release is
 * created by a person, never by a tool or a workflow). Every approved item must
 * carry a mirror_url of one and the same `test-media-*` tag. An item is an
 * asset only when it is pinned and present in `source` with exactly its pinned
 * size and SHA-256, so the assets are verbatim copies.
 *
 * Without `partial`, every approved item must be such an asset. With
 * `partial` (decided 2026-10-19: the release starts with the items already
 * pinned and takes the rest later), the items that are not yet pinned or
 * present are listed as still to come, the notes say so, and `upload` gives
 * the `gh release upload` command for them, to run once they are pinned.
 * Returns the attribution and notes texts and the `gh release create` command
 * (a pre-release that is never "latest"; no release or publishing workflow
 * runs on a tag outside `v*`).
 * @param {object} manifest a valid manifest
 * @param {string} source the folder the items were fetched or pinned into
 * @param {{ partial?: boolean }} [options]
 */
function releasePlan(manifest, source, { partial = false } = {}) {
  const approved = manifest.items.filter((item) => item.status === 'approved');
  const tags = new Set(approved.map((item) => (item.mirror_url ? item.mirror_url.slice(MIRROR_PREFIX.length - 'test-media-'.length).split('/')[0] : null)));
  if (approved.length === 0 || tags.size !== 1 || tags.has(null)) {
    throw new RealMediaError(ErrorCode.MANIFEST_INVALID, 'every approved item needs a mirror_url of one and the same test-media release');
  }
  const [tag] = tags;
  const fileOf = (item) => path.join(source, item.id, item.file_name);
  const present = [];
  const toCome = [];
  for (const item of approved) {
    const file = fileOf(item);
    const exists = fs.existsSync(file);
    if (item.sha256 === null || !exists) {
      if (!partial) {
        throw new RealMediaError(item.sha256 === null ? ErrorCode.NOT_PINNED : ErrorCode.EXISTING_DIFFERS,
          item.sha256 === null ? `${item.id} is not pinned yet` : `${item.id}: the file in the media folder is missing or is not the pinned one`);
      }
      toCome.push(item);
      continue;
    }
    // A file that is there but differs from its pin is never an asset, partial or not.
    if (fs.statSync(file).size !== item.bytes || digestOfFile(file, 'sha256') !== item.sha256) {
      throw new RealMediaError(ErrorCode.EXISTING_DIFFERS, `${item.id}: the file in the media folder is missing or is not the pinned one`);
    }
    present.push(item);
  }
  if (present.length === 0) throw new RealMediaError(ErrorCode.NOT_PINNED, 'no approved item is pinned and present yet');
  const assets = present.map(fileOf);
  const attribution = [
    'VSift test media: verbatim copies of freely licensed recordings, kept for testing VSift.',
    'They are not VSift and are not covered by its licence; each keeps its own licence, below.',
    '',
    ...present.map((item) => `${attributionText(item)}SHA-256: ${item.sha256}\n`),
    ...(toCome.length > 0 ? [`Still to come (not yet in this release): ${toCome.map((item) => item.id).join(', ')}.`, ''] : []),
  ].join('\n');
  const notes = [
    `Test media for VSift's opt-in real-media campaign (#363): ${present.length} of ${approved.length} recordings, verbatim, with their attribution in ATTRIBUTION.txt.`,
    '',
    '**This is not a VSift release.** Nothing here is VSift or under its licence; install VSift from npm (`vsift-cli`).',
    '',
    'The pinned manifest, with each file\'s source, licence and SHA-256, is `tools/p14-campaigns/real-media/manifest.json`; the design is `docs/planning/real-media-set.md`.',
    '',
    '| Asset | Bytes | SHA-256 | Licence | Source |',
    '| --- | ---: | --- | --- | --- |',
    ...present.map((item) => `| ${item.file_name} | ${item.bytes} | \`${item.sha256}\` | ${item.licence.id} | ${item.source_page} |`),
    '',
    ...(toCome.length > 0 ? [
      `**Still to come:** ${toCome.map((item) => `${item.id} (${item.file_name}, ${item.bytes} bytes)`).join(', ')}. They are added to this release once they are pinned; until then the hosted campaign reports them as not yet mirrored.`,
      '',
    ] : []),
  ].join('\n');
  const quote = (text) => `"${text}"`;
  const command = (notesFile, attributionFile) => [
    'gh release create', tag, '--repo smormah/vsift', '--target main', '--prerelease', '--latest=false',
    `--title ${quote(`VSift test media ${tag.slice('test-media-'.length)} (not a VSift release)`)}`,
    `--notes-file ${quote(notesFile)}`, ...assets.map(quote), quote(attributionFile),
  ].join(' ');
  // The later upload adds the files still to come and replaces ATTRIBUTION.txt
  // (--clobber touches only the files named: the new ones do not exist yet).
  const upload = (attributionFile) => (toCome.length === 0 ? null : [
    'gh release upload', tag, '--repo smormah/vsift', '--clobber', ...toCome.map((item) => quote(fileOf(item))), quote(attributionFile),
  ].join(' '));
  return { tag, assets, present: present.map((item) => item.id), toCome: toCome.map((item) => item.id), attribution, notes, command, upload };
}

module.exports = {
  SCHEMA_VERSION,
  SET_ID,
  MAX_ITEM_BYTES,
  MAX_SET_BYTES,
  MAX_APPROVED_ITEMS,
  MAX_APPROVED_BYTES,
  USER_AGENT,
  ALLOWED_HOSTS,
  SOURCE_HOSTS,
  PROVENANCE_ONLY_HOSTS,
  MIRROR_PREFIX,
  PRIVATE_REQUIRED,
  PRIVATE_OPTIONAL,
  COVERS,
  REQUIRED_COVERS,
  TOP_KEYS,
  ITEM_REQUIRED,
  ITEM_OPTIONAL,
  ErrorCode,
  RealMediaError,
  allowedUrl,
  sourceUrl,
  fetchUrl,
  readPrivateManifest,
  refuseHostedPrivateRun,
  localOpen,
  pinFromFile,
  pinUrl,
  crossCheckOutcome,
  releasePlan,
  manifestProblems,
  uncoveredShapes,
  readManifest,
  selectApproved,
  checkDestination,
  httpsOpen,
  refusalText,
  attributionText,
  fetchItem,
  pinItem,
};
