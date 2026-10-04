// Renders the JSON reference page from the v1 schemas (P14 PR 9b,
// docs/planning/user-guide-spec.md): one section per schema file with its
// members, their types and limits, and a note where it adds conditional rules.
//
// The page is generated, never written by hand. It describes shape: which
// members a result has, of what kind, within what bounds. The schemas are the
// authority (a link per section), and `docs/contracts/cli-v1.md` says what each
// command promises. A CI check fails when the committed page is not what this
// module renders from `schemas/v1/`.

'use strict';

const fs = require('node:fs');
const path = require('node:path');

const SCHEMA_SUFFIX = '.schema.json';
// How deep an inline object is expanded into dotted members before the table
// stops and leaves the rest to the schema.
const MAX_DEPTH = 5;
const MAX_HOPS = 16;

/** Every `*.schema.json` of `directory`, parsed, sorted by file name. */
function loadSchemas(directory) {
  return fs
    .readdirSync(directory)
    .filter((name) => name.endsWith(SCHEMA_SUFFIX))
    .sort()
    .map((file) => ({ file, schema: JSON.parse(fs.readFileSync(path.join(directory, file), 'utf8')) }));
}

function anchorOfFile(file) {
  return file.replace(/\./g, '').toLowerCase();
}

/** The node a JSON pointer such as `/$defs/range` names inside `document`. */
function pointer(document, text) {
  let node = document;
  for (const part of text.split('/').filter(Boolean)) {
    if (node === null || typeof node !== 'object') return undefined;
    node = node[part.replace(/~1/g, '/').replace(/~0/g, '~')];
  }
  return node;
}

/** Resolves a `$ref` seen in schema file `file`: `{ file, node, whole }`. */
function resolveRef(ref, file, byFile) {
  const [target, fragment = ''] = ref.split('#');
  const targetFile = target === '' ? file : target;
  const document = byFile.get(targetFile);
  if (document === undefined) return { file: targetFile, node: undefined, whole: false };
  return {
    file: targetFile,
    node: fragment === '' ? document : pointer(document, fragment),
    // A reference to a whole other schema file: that file has its own section.
    whole: fragment === '' && targetFile !== file,
  };
}

/**
 * What a schema node amounts to once its references and `allOf` are followed:
 * `{ file, type, enum, const, alternatives, items, properties, required,
 * bounds, description, external, unresolved }`. `properties` maps a name to
 * `{ node, file }` so a member defined in another file keeps its own context.
 */
function effective(node, file, byFile, hops = 0) {
  const result = {
    file,
    type: undefined,
    enum: undefined,
    const: undefined,
    alternatives: undefined,
    items: undefined,
    properties: new Map(),
    required: new Set(),
    bounds: {},
    description: undefined,
    external: undefined,
    unresolved: undefined,
  };
  if (node === null || typeof node !== 'object' || hops > MAX_HOPS) return result;
  const absorb = (inner) => {
    for (const key of ['type', 'enum', 'const', 'alternatives', 'items', 'description', 'external', 'unresolved']) {
      if (inner[key] !== undefined) result[key] = inner[key];
    }
    for (const [name, value] of inner.properties) result.properties.set(name, value);
    for (const name of inner.required) result.required.add(name);
    Object.assign(result.bounds, inner.bounds);
  };
  if (node.$ref !== undefined) {
    const target = resolveRef(node.$ref, file, byFile);
    if (target.node === undefined) result.unresolved = node.$ref;
    else if (target.whole) result.external = target.file;
    else absorb(effective(target.node, target.file, byFile, hops + 1));
  }
  for (const part of Array.isArray(node.allOf) ? node.allOf : []) absorb(effective(part, file, byFile, hops + 1));
  if (node.type !== undefined) result.type = Array.isArray(node.type) ? node.type.join(' or ') : node.type;
  if (node.const !== undefined) result.const = node.const;
  if (Array.isArray(node.enum)) result.enum = node.enum;
  if (node.description !== undefined) result.description = node.description;
  const alternatives = node.oneOf || node.anyOf;
  if (Array.isArray(alternatives)) result.alternatives = alternatives.map((alternative) => ({ node: alternative, file }));
  if (node.items !== undefined && node.items !== null && typeof node.items === 'object') {
    result.items = { node: node.items, file };
  }
  for (const [name, value] of Object.entries(node.properties || {})) result.properties.set(name, { node: value, file });
  for (const name of Array.isArray(node.required) ? node.required : []) result.required.add(name);
  for (const key of ['minimum', 'maximum', 'minLength', 'maxLength', 'minItems', 'maxItems', 'minProperties', 'maxProperties', 'pattern', 'format', 'uniqueItems']) {
    if (node[key] !== undefined) result.bounds[key] = node[key];
  }
  return result;
}

function quote(value) {
  return JSON.stringify(value);
}

/** The limits a node states, as short phrases. */
function boundNotes(bounds) {
  const notes = [];
  const range = (low, high, unit) => {
    if (low !== undefined && high !== undefined) return low === high ? `exactly ${low}${unit}` : `${low} to ${high}${unit}`;
    if (low !== undefined) return `at least ${low}${unit}`;
    if (high !== undefined) return `at most ${high}${unit}`;
    return undefined;
  };
  for (const text of [
    range(bounds.minimum, bounds.maximum, ''),
    range(bounds.minLength, bounds.maxLength, ' characters'),
    range(bounds.minItems, bounds.maxItems, ' items'),
    range(bounds.minProperties, bounds.maxProperties, ' properties'),
  ]) {
    if (text !== undefined) notes.push(text);
  }
  if (bounds.pattern !== undefined) notes.push(`pattern \`${bounds.pattern}\``);
  if (bounds.format !== undefined) notes.push(`format ${bounds.format}`);
  if (bounds.uniqueItems === true) notes.push('items are unique');
  return notes;
}

/** `{ type, notes }` for a node: a short statement of its kind and limits. */
function describe(node, file, byFile, hops = 0) {
  const merged = effective(node, file, byFile);
  if (merged.external !== undefined) {
    return { type: `object, see [${merged.external}](#${anchorOfFile(merged.external)})`, notes: [] };
  }
  if (merged.unresolved !== undefined) return { type: `see \`${merged.unresolved}\``, notes: [] };
  if (merged.const !== undefined) return { type: `constant ${quote(merged.const)}`, notes: [] };
  if (merged.enum !== undefined) return { type: merged.enum.map(quote).join(' | '), notes: [] };
  if (merged.alternatives !== undefined && hops < MAX_HOPS) {
    const described = merged.alternatives.map((alternative) => describe(alternative.node, alternative.file, byFile, hops + 1));
    const nullable = described.some((entry) => entry.type === 'null');
    const others = described.filter((entry) => entry.type !== 'null');
    const type = others.map((entry) => entry.type).join(' or ') || 'null';
    return { type: nullable && others.length > 0 ? `${type} or null` : type, notes: others.flatMap((entry) => entry.notes) };
  }
  const notes = boundNotes(merged.bounds);
  if (merged.type === 'array' || (merged.type === undefined && merged.items !== undefined)) {
    const item = merged.items ? describe(merged.items.node, merged.items.file, byFile, hops + 1) : { type: 'any', notes: [] };
    return { type: `array of ${item.type}`, notes: [...notes, ...item.notes.map((note) => `each: ${note}`)] };
  }
  if (merged.type === undefined) return { type: merged.properties.size > 0 ? 'object' : 'any', notes };
  return { type: merged.type, notes };
}

/** The objects a node can be: itself, or each object alternative of a union. */
function objectShapes(node, file, byFile) {
  const merged = effective(node, file, byFile);
  if (merged.properties.size > 0) return [merged];
  if (merged.alternatives !== undefined) {
    return merged.alternatives.flatMap((alternative) => objectShapes(alternative.node, alternative.file, byFile));
  }
  if (merged.items !== undefined && merged.type === 'array') {
    return objectShapes(merged.items.node, merged.items.file, byFile);
  }
  return [];
}

/** Table rows for the members of `shape`, nested members flattened into dotted names. */
function collectRows(shape, byFile, prefix, depth, ancestors, rows) {
  for (const [name, member] of shape.properties) {
    const described = describe(member.node, member.file, byFile);
    const dotted = prefix === '' ? name : `${prefix}.${name}`;
    const merged = effective(member.node, member.file, byFile);
    rows.push({
      name: dotted,
      type: described.type,
      required: shape.required.has(name),
      notes: [...(merged.description ? [merged.description] : []), ...described.notes],
    });
    if (depth >= MAX_DEPTH) continue;
    const key = `${member.file}:${JSON.stringify(member.node)}`;
    if (ancestors.has(key)) continue;
    const nested = merged.type === 'array' ? `${dotted}[]` : dotted;
    const next = new Set(ancestors).add(key);
    for (const inner of objectShapes(member.node, member.file, byFile)) collectRows(inner, byFile, nested, depth + 1, next, rows);
  }
}

/** How many `if` rules a schema adds beyond its member table. */
function conditionalRules(schema) {
  const count = (node) => {
    if (Array.isArray(node)) return node.reduce((sum, item) => sum + count(item), 0);
    if (node === null || typeof node !== 'object') return 0;
    return Object.entries(node).reduce((sum, [key, value]) => sum + (key === 'if' ? 1 : 0) + count(value), 0);
  };
  return count(schema);
}

function cell(text) {
  return String(text).replace(/\|/g, '\\|').replace(/\n/g, ' ');
}

/** The Markdown of one schema's section. */
function renderSchema(file, schema, byFile) {
  const lines = [`### ${file}`, ''];
  lines.push(`**${schema.title || file}** ([the schema](../../../schemas/v1/${file}))`, '');
  if (schema.description) lines.push(schema.description, '');
  const rows = [];
  for (const shape of objectShapes(schema, file, byFile)) collectRows(shape, byFile, '', 0, new Set(), rows);
  if (rows.length === 0) {
    const described = describe(schema, file, byFile);
    lines.push(`Shape: ${cell(described.type)}${described.notes.length ? ` (${cell(described.notes.join('; '))})` : ''}.`, '');
  } else {
    lines.push('| Member | Type | Required | Notes |', '| --- | --- | --- | --- |');
    for (const row of rows) {
      lines.push(`| \`${row.name}\` | ${cell(row.type)} | ${row.required ? 'yes' : 'no'} | ${cell(row.notes.join('; '))} |`);
    }
    lines.push('');
  }
  const rules = conditionalRules(schema);
  if (rules > 0) {
    lines.push(`This schema also holds ${rules} conditional rule${rules === 1 ? '' : 's'} (a member that is required, or fixed, when another has a given value); read [the schema](../../../schemas/v1/${file}) for ${rules === 1 ? 'it' : 'them'}.`, '');
  }
  return lines.join('\n');
}

/** The failure codes of the terminal response, from its schema. */
function failureCodes(byFile) {
  const response = byFile.get('operation-response.schema.json');
  const codes = response && response.$defs && response.$defs.error && response.$defs.error.properties.code.enum;
  return Array.isArray(codes) ? codes : [];
}

/** The Markdown of the JSON reference page. */
function renderJson(schemas, version) {
  const byFile = new Map(schemas.map(({ file, schema }) => [file, schema]));
  const lines = [
    '# JSON reference',
    '',
    '<!-- Generated by tools/guide/generate-reference.cjs from schemas/v1/. Do not edit by hand: a CI check fails when this page is out of date. -->',
    '',
    `> **Generated page.** The shape of every JSON document the v1 contract defines, from the schemas in [\`schemas/v1\`](../../../schemas/v1/README.md), as of vsift ${version}, the version this guide was checked against. Do not edit it: change the schema and run \`node tools/guide/generate-reference.cjs --write\`. The schemas are the authority and [the CLI contract](../../contracts/cli-v1.md) says what each command promises; this page is for finding a member quickly. Every command prints one of these documents with \`--json\`, wrapped in the envelope of [operation-response.schema.json](#operation-responseschemajson).`,
    '',
    '## Failure codes',
    '',
    "The `error.code` of a failed result is one of these (the schema's own list). What each one means and what to do is in [troubleshooting](../troubleshooting.md).",
    '',
    ...failureCodes(byFile).map((code) => `- \`${code}\``),
    '',
    '## Contents',
    '',
    ...schemas.map(({ file, schema }) => `- [${file}](#${anchorOfFile(file)}): ${schema.title || file}`),
    '',
    '## Schemas',
    '',
    ...schemas.map(({ file, schema }) => renderSchema(file, schema, byFile)),
  ];
  return lines.join('\n');
}

module.exports = { anchorOfFile, boundNotes, describe, effective, failureCodes, loadSchemas, renderJson, renderSchema };
