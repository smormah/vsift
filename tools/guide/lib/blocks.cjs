// Finds the checked examples of a guide page (P14 PR 9b, docs/planning/user-guide-spec.md rule 3).
//
// A checked example is a fenced `console` block that follows a marker comment:
//
//   <!-- check -->
//   <!-- check: needs speech -->
//   <!-- check: exit 2 -->
//
// Inside the block, a line that starts with `$ ` is a command and the lines after it, up to the
// next `$ ` line, are the output the page shows for it. A block without a marker is an
// illustration: it may hold output that depends on the machine (the setup check names your own
// tools), and the page says so. Blocks run in page order, in one sandbox per page.

'use strict';

const MARKER = /^<!--\s*check(?::\s*(.*?))?\s*-->$/;
const OPEN_FENCE = /^(`{3,})\s*console\b.*$/;

/** The options a marker line carries: `{ needs: Set<string>, exit: number }`. */
function parseMarker(text) {
  const options = { needs: new Set(), exit: 0 };
  for (const part of (text || '').split(';').map((item) => item.trim()).filter(Boolean)) {
    const needs = /^needs\s+(\w+)$/.exec(part);
    const exit = /^exit\s+(\d+)$/.exec(part);
    if (needs) options.needs.add(needs[1]);
    else if (exit) options.exit = Number(exit[1]);
    else throw new Error(`unknown check option "${part}"`);
  }
  return options;
}

/** The lines of a fenced block as commands with the output shown for each. */
function parseCommands(lines, firstLine) {
  const commands = [];
  for (const [offset, line] of lines.entries()) {
    if (line.startsWith('$ ')) {
      commands.push({ line: firstLine + offset, text: line.slice(2).trim(), expected: [] });
    } else if (commands.length > 0) {
      commands[commands.length - 1].expected.push(line.replace(/[ \t]+$/, ''));
    } else if (line.trim() !== '') {
      throw new Error(`line ${firstLine + offset}: output before the first command: ${line}`);
    }
  }
  for (const command of commands) {
    while (command.expected.length > 0 && command.expected[command.expected.length - 1] === '') command.expected.pop();
  }
  return commands;
}

/** Every checked block of a page, in order: `{ line, options, commands }`. */
function extractBlocks(markdown) {
  const lines = markdown.replace(/\r\n/g, '\n').split('\n');
  const blocks = [];
  for (let index = 0; index < lines.length; index += 1) {
    const marker = MARKER.exec(lines[index].trim());
    if (!marker) continue;
    let fence = index + 1;
    while (fence < lines.length && lines[fence].trim() === '') fence += 1;
    const open = OPEN_FENCE.exec(lines[fence] || '');
    if (!open) throw new Error(`line ${index + 1}: a check marker must be followed by a console block`);
    let close = fence + 1;
    while (close < lines.length && !lines[close].startsWith(open[1])) close += 1;
    if (close >= lines.length) throw new Error(`line ${fence + 1}: the console block is not closed`);
    let options;
    try {
      options = parseMarker(marker[1]);
    } catch (error) {
      throw new Error(`line ${index + 1}: ${error.message}`);
    }
    const commands = parseCommands(lines.slice(fence + 1, close), fence + 2);
    if (commands.length === 0) throw new Error(`line ${index + 1}: a checked block holds no command ("$ vsift ...")`);
    blocks.push({ line: index + 1, options, commands });
    index = close;
  }
  return blocks;
}

module.exports = { extractBlocks, parseCommands, parseMarker };
