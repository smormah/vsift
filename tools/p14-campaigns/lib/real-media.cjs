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

/** The manifest's own version; a reader refuses any other. */
const SCHEMA_VERSION = '1';
const SET_ID = 'vsift-real-media';

/** No item may be larger: the set is meant to stay small (aim: a few hundred MB in all). */
const MAX_ITEM_BYTES = 300 * 1024 * 1024;
/** Every item that is not withdrawn, together. */
const MAX_SET_BYTES = 750 * 1024 * 1024;
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
 * The hosts a file may be fetched from, and the hosts a redirect may lead to.
 * Each is a source with stable, versioned URLs (docs/planning/real-media-set.md
 * section 3). A host is added in the reviewed change that adds an item from it.
 */
const ALLOWED_HOSTS = Object.freeze(['upload.wikimedia.org', 'archive.org']);
/** Host suffixes for the Internet Archive's storage nodes, to which archive.org/download redirects. */
const ALLOWED_HOST_SUFFIXES = Object.freeze(['.us.archive.org']);

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
const ITEM_OPTIONAL = Object.freeze(['source_revision', 'approval', 'labels', 'notes']);
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
function allowedUrl(text) {
  let url;
  try {
    url = new URL(text);
  } catch {
    return false;
  }
  if (url.protocol !== 'https:' || url.username || url.password || url.port) return false;
  const host = url.hostname.toLowerCase();
  return ALLOWED_HOSTS.includes(host) || ALLOWED_HOST_SUFFIXES.some((suffix) => host.endsWith(suffix));
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
  if (typeof item.url !== 'string' || !allowedUrl(item.url)) problems.push(`${label}: url must be HTTPS on an allowed host`);
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

/**
 * Every problem with a manifest, as fixed prose naming the member; an empty
 * list means it is valid. The rules are those of
 * `real-media/manifest.schema.json` (a test keeps the required members of both
 * in step) plus what a schema cannot say: unique ids and file names, the set's
 * total size and what an approval needs.
 * @param {unknown} manifest the parsed JSON
 * @returns {string[]}
 */
function manifestProblems(manifest) {
  const problems = [];
  if (!isObject(manifest)) return ['the manifest must be a JSON object'];
  for (const key of TOP_KEYS) if (!(key in manifest)) problems.push(`missing ${key}`);
  for (const key of unknownKeys(manifest, TOP_KEYS)) problems.push(`unknown member ${key}`);
  if (manifest.schema_version !== SCHEMA_VERSION) problems.push(`schema_version must be "${SCHEMA_VERSION}"`);
  if (manifest.set_id !== SET_ID) problems.push(`set_id must be "${SET_ID}"`);
  if (!Array.isArray(manifest.items)) return [...problems, 'items must be a list'];
  manifest.items.forEach((item, index) => checkItem(item, index, problems));
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
 * The transport the tool uses: Node's HTTPS client, refusing any URL or
 * redirect that `allowedUrl` refuses, following at most `MAX_REDIRECTS`.
 * @param {string} url
 * @returns {Promise<{ stream: import('node:stream').Readable, length: number | null }>}
 */
function httpsOpen(url) {
  return new Promise((resolve, reject) => {
    const follow = (current, redirects) => {
      if (!allowedUrl(current)) {
        reject(new RealMediaError(ErrorCode.TRANSPORT_REFUSED, 'a request or redirect left HTTPS or the allowed hosts'));
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
          response.resume();
          reject(new RealMediaError(ErrorCode.TRANSPORT_REFUSED, `the server answered HTTP ${status}`));
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

/**
 * Streams one file to `<destination>/<id>/.<file>.partial`, never more than the
 * pinned size, hashing as it goes; returns the digests. The partial file is
 * created exclusively and removed again on any failure.
 */
async function download(item, partial, open) {
  const { stream, length } = await open(item.url);
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
  fs.writeFileSync(path.join(folder, 'ATTRIBUTION.txt'), attributionText(item));
  if (fs.existsSync(final)) {
    if (fs.statSync(final).size === item.bytes && digestOfFile(final, 'sha256') === item.sha256) return 'present';
    throw new RealMediaError(ErrorCode.EXISTING_DIFFERS, `${item.id}: a different file is already at its place; it is left as it is`);
  }
  const partial = path.join(folder, `.${item.file_name}.partial`);
  const digests = await download(item, partial, open);
  if (digests.sha256 !== item.sha256) {
    fs.rmSync(partial, { force: true });
    throw new RealMediaError(ErrorCode.DIGEST_MISMATCH, `${item.id}: the SHA-256 does not match the pinned digest`);
  }
  fs.renameSync(partial, final);
  return 'fetched';
}

/**
 * Pins an approved item that has no SHA-256 yet: fetches it bounded by its
 * size, verifies the checksum its source publishes where it publishes one,
 * keeps the file and returns the SHA-256 for the reviewer to write into the
 * manifest. Without a source checksum (a Commons transcode, for which the
 * source publishes none) the pin is trust on first use over HTTPS, which the
 * design records; every later fetch is held to it.
 * @param {object} item an approved item with no sha256
 * @param {string} destination a folder `checkDestination` accepted
 * @param {(url: string) => Promise<{ stream: AsyncIterable<Buffer> & { destroy(): void }, length: number | null }>} open
 * @returns {Promise<string>} the SHA-256
 */
async function pinItem(item, destination, open) {
  if (item.status !== 'approved') throw new RealMediaError(ErrorCode.NOT_APPROVED, `${item.id} is not approved`);
  if (item.sha256 !== null) throw new RealMediaError(ErrorCode.ALREADY_PINNED, `${item.id} is pinned already; fetch it instead`);
  const folder = path.join(destination, item.id);
  const final = path.join(folder, item.file_name);
  fs.mkdirSync(folder, { recursive: true, mode: 0o700 });
  if (fs.existsSync(final)) throw new RealMediaError(ErrorCode.EXISTING_DIFFERS, `${item.id}: a file is already at its place; it is left as it is`);
  fs.writeFileSync(path.join(folder, 'ATTRIBUTION.txt'), attributionText(item));
  const partial = path.join(folder, `.${item.file_name}.partial`);
  const digests = await download(item, partial, open);
  fs.renameSync(partial, final);
  return digests.sha256;
}

module.exports = {
  SCHEMA_VERSION,
  SET_ID,
  MAX_ITEM_BYTES,
  MAX_SET_BYTES,
  USER_AGENT,
  ALLOWED_HOSTS,
  COVERS,
  REQUIRED_COVERS,
  TOP_KEYS,
  ITEM_REQUIRED,
  ITEM_OPTIONAL,
  ErrorCode,
  RealMediaError,
  allowedUrl,
  manifestProblems,
  uncoveredShapes,
  readManifest,
  selectApproved,
  checkDestination,
  httpsOpen,
  attributionText,
  fetchItem,
  pinItem,
};
