'use strict';

// Hostile file names and arguments for the installed launcher (ADR 0024, RQ-01:
// "hostile file names (quotes, newlines, leading dashes, spaces, Unicode)
// through the installed launcher"; threat SEC-01, injection through a path).
//
// Two kinds of case, both chosen for what the file system of the platform
// allows:
//
// - A **file name**: a draft report with this exact name is read by
//   `vsift handoff check --file <name>`. The command succeeds only if the name
//   reached VSift unchanged, so a launcher or a package-manager shim that
//   quotes, splits, expands or drops a character fails it.
// - An **argument**: text that is not a file, every one of which a shell would
//   act on (command substitution, separators, redirections). It goes to
//   `vsift ingest -- <argument>`, which fails the same way for every value
//   (the source does not exist). The check is that the installed command
//   answers exactly as VSift run directly does, and that no command the
//   argument spells ran: each one would create the marker file.
//
// A leading dash goes after `--`, so it is a value and not an option.

/** The file the injected commands would create if anything interpreted them. */
const MARKER = 'pwned-marker';

/** Characters Windows cannot have in a file name. */
const WINDOWS_FORBIDDEN = /[<>:"/\\|?*\u0000-\u001f]/;

/** Draft file names valid on `platform`, each hostile to some quoting rule. */
function hostileFileNames(platform) {
  const common = [
    ['spaces', 'a draft with  several   spaces.md'],
    ['leading dash', '-leading-dash.md'],
    ['double leading dash', '--help.md'],
    ['single quote', "it's a draft.md"],
    ['accents and CJK', 'dräft ü 日本語 café.md'],
    ['emoji and combining mark', 'dráft 😀.md'],
    ['dollar and parentheses', '$(touch pwned-marker) ${HOME} (x).md'],
    ['semicolon, ampersand and caret', 'a;b&c^d.md'],
    ['brackets and braces', '[x] {y} #z.md'],
    ['backtick', '`touch pwned-marker`.md'],
    ['exclamation and tilde', '!bang ~tilde.md'],
    ['equals sign', 'a=b.md'],
    ['percent sign', '100% done.md'],
    ['comma', 'one,two.md'],
  ];
  if (platform === 'win32') {
    return common.filter(([, name]) => !WINDOWS_FORBIDDEN.test(name));
  }
  return [
    ...common,
    ['double quote', 'say "hello".md'],
    ['both quotes', `it's "quoted".md`],
    ['newline', 'line one\nline two.md'],
    ['carriage return', 'a\rb.md'],
    ['tab', 'a\tb.md'],
    ['backslash', 'back\\slash.md'],
    ['asterisk and question mark', 'star*question?.md'],
    ['pipe, less-than and greater-than', 'a|b<c>d.md'],
    ['colon', 'time 12:30.md'],
  ];
}

/** Arguments that are not files and that a shell would act on, valid on `platform`. */
function hostileArguments(platform) {
  const posix = [
    ['command substitution', `$(touch ${MARKER})`],
    ['backticks', `\`touch ${MARKER}\``],
    ['semicolon', `a;touch ${MARKER}`],
    ['pipe', `a|touch ${MARKER}`],
    ['ampersand', `a&touch ${MARKER}`],
    ['and-and', `a && touch ${MARKER}`],
    ['newline', `a\ntouch ${MARKER}`],
    ['breaking out of double quotes', `"; touch ${MARKER}; "`],
    ['breaking out of single quotes', `'; touch ${MARKER}; '`],
    ['redirection', `a > ${MARKER}`],
    ['glob', '*'],
    ['tilde and variable', '~/$HOME'],
    ['an option after the separator', '--help'],
    ['an option-like value', '-version'],
  ];
  if (platform !== 'win32') {
    return posix;
  }
  return [
    ['ampersand', `a&echo x>${MARKER}`],
    ['pipe', `a|echo x>${MARKER}`],
    ['quote break-out', `a"&echo x>${MARKER}&"b`],
    ['escaped ampersand', `a^&echo x>${MARKER}`],
    ['percent variable', '%COMSPEC%'],
    ['percent path', '%PATH%'],
    ['delayed expansion', '!PATH!'],
    ['parentheses', `(a)&(echo x>${MARKER})`],
    ['semicolon and comma', `a;echo,x>${MARKER}`],
    ['an option after the separator', '--help'],
    ['an option-like value', '-version'],
  ];
}

/** The text of the draft every file-name case reads: it has no handoff block, which is a complete check. */
const DRAFT = '# Report\n\nNo handoff block yet.\n';

module.exports = { DRAFT, MARKER, hostileArguments, hostileFileNames };
