// Splits a command line of a guide page into arguments, the way a person types it in a
// POSIX shell or PowerShell for the simple cases the pages use: words, single quotes and
// double quotes. The runner starts the program with the argument list and no shell, so nothing
// else (a pipe, a redirection, a variable) is understood, and a command that uses one is an error.

'use strict';

const SHELL_SYNTAX = /[|<>&;$`]/;

/** The arguments of `line`, or an error that says what the check cannot run. */
function splitCommand(line) {
  const words = [];
  let current = '';
  let started = false;
  let quote;
  for (const character of line) {
    if (quote) {
      if (character === quote) quote = undefined;
      else current += character;
    } else if (character === '"' || character === "'") {
      quote = character;
      started = true;
    } else if (/\s/.test(character)) {
      if (started) words.push(current);
      current = '';
      started = false;
    } else {
      current += character;
      started = true;
    }
  }
  if (quote) throw new Error(`an unclosed quote in: ${line}`);
  if (started) words.push(current);
  return words;
}

/** Whether an unquoted word uses shell syntax the check does not run. */
function usesShellSyntax(line) {
  let quote;
  for (const character of line) {
    if (quote) {
      if (character === quote) quote = undefined;
    } else if (character === '"' || character === "'") quote = character;
    else if (SHELL_SYNTAX.test(character)) return true;
  }
  return false;
}

module.exports = { splitCommand, usesShellSyntax };
