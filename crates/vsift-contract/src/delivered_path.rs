//! How a delivered file's path is written in `files[].path` (ADR 0019 D2 and
//! its 2026-09-29 note, issue #210).
//!
//! The engine verifies an artifact through the session root it canonicalised,
//! which on Windows is the extended-length form `\\?\C:\...`. That form is
//! valid, but common agent file tools and their permission rules refuse it,
//! so the result writes the plain form `C:\...` whenever the plain form names
//! the same file: it fits within the legacy `MAX_PATH` limit and no component
//! would be changed by Win32 path normalisation. Otherwise it keeps the
//! extended-length form, which is always exact.
//!
//! This is presentation only. The engine's verification, containment checks
//! and file operations keep the path they verified; only the JSON text
//! changes. Unix and macOS paths are written unchanged.

use std::path::Path;

/// The extended-length prefix of a Windows path that the file APIs pass to
/// the file system without normalisation.
const EXTENDED_LENGTH_PREFIX: &str = r"\\?\";

/// The legacy Win32 path limit, in UTF-16 code units including the
/// terminating NUL: a plain path must be shorter than this.
const WINDOWS_MAX_PATH: usize = 260;

/// The text a result writes for a delivered file's `path`, or `None` when the
/// path is not valid UTF-8 and so cannot be written as JSON text.
pub(crate) fn delivered_path_text(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    // `cfg!` rather than `#[cfg]` so the decision is compiled, linted and
    // unit-tested on every platform, while only Windows output changes.
    if cfg!(windows) {
        Some(plain_windows_form(text).unwrap_or(text).to_owned())
    } else {
        Some(text.to_owned())
    }
}

/// The plain form `C:\...` of an extended-length drive path `\\?\C:\...`,
/// when that plain form names the same file for every Windows file API.
///
/// Returns `None`, so the caller keeps the extended-length form, when the
/// path is not an extended-length drive path (a UNC or volume path, for
/// example), when the plain form would reach `MAX_PATH`, or when any
/// component is one that Win32 normalisation changes or reinterprets: an
/// empty component, `.` or `..`, a trailing dot or space, a reserved device
/// name, or a character that is not valid in a Windows file name. Those are
/// exactly the cases where only the extended-length form is exact.
pub(crate) fn plain_windows_form(path: &str) -> Option<&str> {
    let plain = path.strip_prefix(EXTENDED_LENGTH_PREFIX)?;
    let mut drive = plain.chars();
    let names_a_drive = drive
        .next()
        .is_some_and(|letter| letter.is_ascii_alphabetic())
        && drive.next() == Some(':')
        && drive.next() == Some('\\');
    if !names_a_drive || plain.encode_utf16().count() >= WINDOWS_MAX_PATH {
        return None;
    }
    let below_drive = drive.as_str();
    let unchanged = !below_drive.is_empty()
        && below_drive
            .split('\\')
            .all(component_survives_normalisation);
    unchanged.then_some(plain)
}

/// Whether Win32 normalisation leaves `component` exactly as it is and reads
/// it as an ordinary file or directory name.
fn component_survives_normalisation(component: &str) -> bool {
    !component.is_empty()
        && !component.ends_with(['.', ' '])
        && !component.chars().any(|character| {
            character.is_control()
                || matches!(character, '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && !is_reserved_device_name(component)
}

/// Whether `component` names a DOS device in a plain path. Legacy Windows
/// reads the device name before any extension and ignores spaces before the
/// dot (`CON .txt`), so the check does too; newer builds are narrower, and
/// the wider rule only ever keeps the always-exact extended-length form.
fn is_reserved_device_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|device| {
        stem.strip_prefix(device).is_some_and(|number| {
            matches!(
                number,
                "0" | "1"
                    | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "\u{b9}"
                    | "\u{b2}"
                    | "\u{b3}"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{WINDOWS_MAX_PATH, delivered_path_text, plain_windows_form};

    const ARTIFACT: &str = r"sessions\ses_0123\artifacts\artifact-0a1b.png";

    fn verbatim(folder: &str) -> String {
        format!(r"\\?\C:\{folder}\{ARTIFACT}")
    }

    #[test]
    fn a_short_ordinary_drive_path_is_written_plainly() {
        let path = verbatim(r"Users\agent\AppData\Local\Temp\vsift root");
        assert_eq!(
            plain_windows_form(&path),
            Some(
                r"C:\Users\agent\AppData\Local\Temp\vsift root\sessions\ses_0123\artifacts\artifact-0a1b.png"
            )
        );
    }

    #[test]
    fn a_trailing_dot_or_space_keeps_the_extended_length_form() {
        for folder in ["root.", "root ", "root. ", r"root.\inner", r"root \inner"] {
            assert_eq!(plain_windows_form(&verbatim(folder)), None, "{folder:?}");
        }
    }

    #[test]
    fn reserved_device_names_keep_the_extended_length_form() {
        for folder in [
            "CON",
            "con",
            "PRN",
            "aux",
            "NUL",
            "nul.txt",
            "CON .txt",
            "COM1",
            "com9",
            "LPT1",
            "lpt0",
            "COM\u{b9}",
            "LPT\u{b3}",
            "CONIN$",
            "conout$",
            "CLOCK$",
        ] {
            assert_eq!(plain_windows_form(&verbatim(folder)), None, "{folder:?}");
        }
    }

    #[test]
    fn names_that_only_resemble_devices_are_written_plainly() {
        for folder in ["CONSOLE", "COM10", "LPT", "nulls", "auxiliary.txt", "COMA"] {
            assert!(
                plain_windows_form(&verbatim(folder)).is_some(),
                "{folder:?}"
            );
        }
    }

    #[test]
    fn components_normalisation_would_change_keep_the_extended_length_form() {
        for folder in [
            ".",
            "..",
            r"root\\inner",
            "a/b",
            "file:stream",
            "star*",
            "what?",
            "quote\"",
            "less<",
            "more>",
            "pipe|",
            "bell\u{7}",
        ] {
            assert_eq!(plain_windows_form(&verbatim(folder)), None, "{folder:?}");
        }
    }

    #[test]
    fn only_a_path_below_max_path_is_written_plainly() {
        let fixed = verbatim("").chars().count() - r"\\?\".len();
        let fits = "a".repeat(WINDOWS_MAX_PATH - 1 - fixed);
        let reaches = "a".repeat(WINDOWS_MAX_PATH - fixed);
        assert!(plain_windows_form(&verbatim(&fits)).is_some());
        assert_eq!(plain_windows_form(&verbatim(&reaches)), None);
        // The limit counts UTF-16 code units, not bytes: a character outside
        // the Basic Multilingual Plane is two.
        let wide = "\u{1f600}".repeat(fits.len() / 2 + 1);
        assert_eq!(plain_windows_form(&verbatim(&wide)), None);
    }

    #[test]
    fn only_an_extended_length_drive_path_changes() {
        for path in [
            r"C:\already\plain.png",
            r"\\?\UNC\server\share\artifact.png",
            r"\\?\Volume{0a1b}\artifact.png",
            r"\\?\C:",
            r"\\?\C:\",
            r"\\?\1:\artifact.png",
            "/home/agent/vsift/artifact.png",
        ] {
            assert_eq!(plain_windows_form(path), None, "{path:?}");
        }
    }

    #[test]
    fn delivered_text_changes_only_on_windows() {
        let path = verbatim("root");
        let expected = if cfg!(windows) {
            path.trim_start_matches(r"\\?\").to_owned()
        } else {
            path.clone()
        };
        assert_eq!(
            delivered_path_text(std::path::Path::new(&path)),
            Some(expected)
        );
        assert_eq!(
            delivered_path_text(std::path::Path::new("/vsift-session-root/a.png")),
            Some("/vsift-session-root/a.png".to_owned())
        );
    }
}
