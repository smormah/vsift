//! The command line of a process started on Windows, built as the standard
//! library builds it (amendment of 2026-10-27 to ADR 0026, L-229).
//!
//! Windows passes a new process one string, and the process splits it into
//! arguments itself (the C runtime's rules, which Rust's `std::env::args`
//! follows). `std::process::Command` joins the arguments so that this split
//! gives them back exactly; [`command_line`] repeats its rules, so a process
//! started with `start_detached` receives the arguments `Command` would have
//! given it. The rules are plain Rust over UTF-16 units, so they are tested on
//! every platform.

use std::io;

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;

/// The command line for `program` and `arguments`, without its terminating
/// NUL, as `std::process::Command` builds it on Windows.
///
/// - The program name is always in quotes, and is refused if it holds a quote
///   (a file name cannot, and an escape there would not be read as one).
/// - An argument is in quotes when it is empty or holds a space or a tab.
/// - A quote inside an argument is escaped with a backslash, and the
///   backslashes just before it are doubled; so are the backslashes before
///   the closing quote of a quoted argument. Other backslashes stay as they
///   are.
///
/// # Errors
///
/// `InvalidInput` when any value holds a NUL (Windows would read it as the end
/// of the string) or the program name holds a quote.
pub(crate) fn command_line<P, A, U>(program: P, arguments: A) -> io::Result<Vec<u16>>
where
    P: IntoIterator<Item = u16>,
    A: IntoIterator<Item = U>,
    U: IntoIterator<Item = u16>,
{
    let mut line = vec![QUOTE];
    for unit in program {
        if unit == 0 || unit == QUOTE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a program name holds a NUL or a quote",
            ));
        }
        line.push(unit);
    }
    line.push(QUOTE);
    for argument in arguments {
        line.push(SPACE);
        append_argument(&mut line, argument.into_iter().collect())?;
    }
    Ok(line)
}

/// Appends one argument, quoted and escaped as [`command_line`] describes.
fn append_argument(line: &mut Vec<u16>, argument: Vec<u16>) -> io::Result<()> {
    if argument.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an argument holds a NUL character",
        ));
    }
    let quoted = argument.is_empty() || argument.iter().any(|unit| *unit == SPACE || *unit == TAB);
    if quoted {
        line.push(QUOTE);
    }
    let mut backslashes = 0_usize;
    for unit in argument {
        if unit == BACKSLASH {
            backslashes += 1;
        } else {
            if unit == QUOTE {
                // The run of backslashes is doubled and one more escapes the
                // quote: 2n + 1 in all.
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(unit);
    }
    if quoted {
        // Doubled before the closing quote, so the last one does not escape it.
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        line.push(QUOTE);
    }
    Ok(())
}

/// Whether Windows would run `file_name` through `cmd.exe` (a `.bat` or `.cmd`
/// file), whose quoting rules differ from the ones [`command_line`] follows.
///
/// Trailing dots and spaces are removed first, as Windows removes them when
/// it opens the file, so `run.bat.` and `run.cmd ` are batch files too.
pub(crate) fn is_batch_file(file_name: &str) -> bool {
    std::path::Path::new(file_name.trim_end_matches(['.', ' ']))
        .extension()
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
        })
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io};

    use super::{command_line, is_batch_file};

    fn line(program: &str, arguments: &[&str]) -> io::Result<String> {
        let units = command_line(
            program.encode_utf16(),
            arguments.iter().map(|argument| argument.encode_utf16()),
        )?;
        String::from_utf16(&units).map_err(io::Error::other)
    }

    #[test]
    fn the_program_is_always_quoted_and_plain_arguments_are_not() -> Result<(), Box<dyn Error>> {
        assert_eq!(line(r"C:\vsift\vsift.exe", &[])?, r#""C:\vsift\vsift.exe""#);
        assert_eq!(
            line(r"C:\Program Files\vsift.exe", &["job", "resume", "--json"])?,
            r#""C:\Program Files\vsift.exe" job resume --json"#
        );
        Ok(())
    }

    #[test]
    fn arguments_are_quoted_and_escaped_as_the_standard_library_does() -> Result<(), Box<dyn Error>>
    {
        let cases: [(&str, &str); 9] = [
            ("", r#""""#),
            ("a b", r#""a b""#),
            ("a\tb", "\"a\tb\""),
            (r"a\b", r"a\b"),
            (r#"a"b"#, r#"a\"b"#),
            (r#"a\"b"#, r#"a\\\"b"#),
            (r"C:\dir with space\", r#""C:\dir with space\\""#),
            (r#"say "hi" \\"#, r#""say \"hi\" \\\\""#),
            (r"trailing\\", r"trailing\\"),
        ];
        for (argument, expected) in cases {
            assert_eq!(
                line("p", &[argument])?,
                format!(r#""p" {expected}"#),
                "{argument:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_nul_anywhere_and_a_quote_in_the_program_are_refused() {
        for refused in [
            line("p\0q", &[]),
            line(r#"C:\a"b.exe"#, &[]),
            line("p", &["ok", "a\0b"]),
        ] {
            assert!(
                matches!(&refused, Err(error) if error.kind() == io::ErrorKind::InvalidInput),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn batch_files_are_recognised_whatever_their_case_and_trailing_dots() {
        for batch in ["run.bat", "RUN.CMD", "run.bat.", "run.cmd  ", "run.Bat. ."] {
            assert!(is_batch_file(batch), "{batch}");
        }
        for program in ["vsift.exe", "batch", "run.bat.exe", "cmd", "run.cmdx"] {
            assert!(!is_batch_file(program), "{program}");
        }
    }
}
