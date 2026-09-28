//! The relative input path grammar of a worker request (ADR 0021).
//!
//! A worker request names its source and supplied transcript relative to the
//! operator's `--input-root`; the host opens them through a capability
//! directory handle of that root, never by joining strings. The grammar is
//! deliberately narrower than any one platform's so the same request means
//! the same file on Linux, macOS and Windows, and so nothing in it can
//! climb out of the root, name a device, pick an alternate data stream or
//! alias another name after Windows strips a trailing dot or space.

use std::fmt;

/// Most bytes of one relative input path.
pub const MAX_INPUT_PATH_BYTES: usize = 1_024;
/// Most components of one relative input path.
pub const MAX_INPUT_PATH_COMPONENTS: usize = 32;

/// Device names Windows resolves in every directory, with or without an
/// extension, in any case.
const RESERVED_WINDOWS_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Characters no component may hold: separators and drive or stream markers
/// (`\`, `:`), and the characters Windows forbids in names.
const FORBIDDEN_CHARACTERS: [char; 7] = ['\\', ':', '<', '>', '"', '|', '?'];

/// A validated path relative to the operator's input root, with `/` between
/// components.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RelativeInputPath(String);

impl RelativeInputPath {
    /// Validates `value` against the grammar.
    ///
    /// # Errors
    ///
    /// [`InputPathError`] naming the first rule it breaks.
    pub fn parse(value: &str) -> Result<Self, InputPathError> {
        if value.is_empty() {
            return Err(InputPathError::Empty);
        }
        if value.len() > MAX_INPUT_PATH_BYTES {
            return Err(InputPathError::TooLong);
        }
        let mut components = 0_usize;
        for component in value.split('/') {
            components += 1;
            if components > MAX_INPUT_PATH_COMPONENTS {
                return Err(InputPathError::TooManyComponents);
            }
            validate_component(component)?;
        }
        Ok(Self(value.to_owned()))
    }

    /// The path as written, components separated by `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The components, in order; none is empty, `.` or `..`.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

fn validate_component(component: &str) -> Result<(), InputPathError> {
    // An empty component is a leading `/` (an absolute path), a trailing `/`
    // or a doubled separator.
    if component.is_empty() {
        return Err(InputPathError::EmptyComponent);
    }
    if component == "." || component == ".." {
        return Err(InputPathError::DotComponent);
    }
    if component
        .chars()
        .any(|character| character.is_control() || FORBIDDEN_CHARACTERS.contains(&character))
        || component.contains('*')
    {
        return Err(InputPathError::ForbiddenCharacter);
    }
    // Windows drops a trailing dot or space, so `a.` would open `a`.
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(InputPathError::TrailingDotOrSpace);
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ');
    if RESERVED_WINDOWS_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        || is_superscript_device(stem)
    {
        return Err(InputPathError::ReservedName);
    }
    Ok(())
}

/// `COM¹`..`COM³` and `LPT¹`..`LPT³` are device names on current Windows too.
fn is_superscript_device(stem: &str) -> bool {
    let mut characters = stem.chars();
    let prefix: String = characters.by_ref().take(3).collect();
    let rest: Vec<char> = characters.collect();
    (prefix.eq_ignore_ascii_case("com") || prefix.eq_ignore_ascii_case("lpt"))
        && matches!(rest.as_slice(), ['\u{b9}' | '\u{b2}' | '\u{b3}'])
}

/// Which rule of the relative input path grammar a path broke.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputPathError {
    /// The path is empty.
    Empty,
    /// The path is longer than [`MAX_INPUT_PATH_BYTES`].
    TooLong,
    /// The path has more than [`MAX_INPUT_PATH_COMPONENTS`] components.
    TooManyComponents,
    /// A component is empty: an absolute path, a trailing or doubled `/`.
    EmptyComponent,
    /// A component is `.` or `..`.
    DotComponent,
    /// A component holds a control character, `\`, `:` or a character
    /// Windows forbids in names (drive prefixes and alternate data streams).
    ForbiddenCharacter,
    /// A component ends with a dot or a space, which Windows strips.
    TrailingDotOrSpace,
    /// A component is a Windows device name (`CON`, `NUL`, `COM1`, ...).
    ReservedName,
}

impl fmt::Display for InputPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "the path is empty",
            Self::TooLong => "the path is too long",
            Self::TooManyComponents => "the path has too many components",
            Self::EmptyComponent => "the path is absolute or has an empty component",
            Self::DotComponent => "the path has a . or .. component",
            Self::ForbiddenCharacter => "the path has a forbidden character",
            Self::TrailingDotOrSpace => "a path component ends with a dot or a space",
            Self::ReservedName => "a path component is a reserved device name",
        })
    }
}

impl std::error::Error for InputPathError {}

#[cfg(test)]
mod tests {
    use super::{InputPathError, MAX_INPUT_PATH_BYTES, RelativeInputPath};

    #[test]
    fn plain_relative_paths_are_accepted() {
        for path in [
            "clip.mp4",
            "recordings/2026/clip one.mp4",
            "a/b/c/d.e.f",
            ".hidden/file",
            "con-tract.mp4",
            "Übersicht/clip.mkv",
            "comet.mp4",
        ] {
            assert!(RelativeInputPath::parse(path).is_ok(), "{path}");
        }
    }

    #[test]
    fn every_escape_and_alias_is_refused() {
        for (path, expected) in [
            ("", InputPathError::Empty),
            ("/etc/passwd", InputPathError::EmptyComponent),
            ("a//b", InputPathError::EmptyComponent),
            ("a/", InputPathError::EmptyComponent),
            ("../outside.mp4", InputPathError::DotComponent),
            ("a/./b", InputPathError::DotComponent),
            ("a/../b", InputPathError::DotComponent),
            ("C:/clip.mp4", InputPathError::ForbiddenCharacter),
            ("C:clip.mp4", InputPathError::ForbiddenCharacter),
            ("dir\\clip.mp4", InputPathError::ForbiddenCharacter),
            ("clip.mp4:stream", InputPathError::ForbiddenCharacter),
            ("clip\u{0}.mp4", InputPathError::ForbiddenCharacter),
            ("clip\n.mp4", InputPathError::ForbiddenCharacter),
            ("clip\u{1b}[31m.mp4", InputPathError::ForbiddenCharacter),
            ("clip?.mp4", InputPathError::ForbiddenCharacter),
            ("clip*.mp4", InputPathError::ForbiddenCharacter),
            ("clip.", InputPathError::TrailingDotOrSpace),
            ("clip ", InputPathError::TrailingDotOrSpace),
            ("CON", InputPathError::ReservedName),
            ("nul.txt", InputPathError::ReservedName),
            ("dir/Com1.mp4", InputPathError::ReservedName),
            ("lpt9", InputPathError::ReservedName),
            ("COM\u{b9}.mp4", InputPathError::ReservedName),
            ("aux .mp4", InputPathError::ReservedName),
        ] {
            assert_eq!(RelativeInputPath::parse(path), Err(expected), "{path:?}");
        }
    }

    #[test]
    fn length_and_component_bounds_hold_at_the_edge() {
        let longest = "a".repeat(MAX_INPUT_PATH_BYTES);
        assert!(RelativeInputPath::parse(&longest).is_ok());
        assert_eq!(
            RelativeInputPath::parse(&format!("{longest}a")),
            Err(InputPathError::TooLong)
        );
        let deepest = vec!["d"; 32].join("/");
        assert!(RelativeInputPath::parse(&deepest).is_ok());
        assert_eq!(
            RelativeInputPath::parse(&format!("{deepest}/d")),
            Err(InputPathError::TooManyComponents)
        );
    }

    #[test]
    fn components_are_the_separated_names() -> Result<(), InputPathError> {
        let path = RelativeInputPath::parse("a/b.mp4")?;
        assert_eq!(path.components().collect::<Vec<_>>(), ["a", "b.mp4"]);
        assert_eq!(path.as_str(), "a/b.mp4");
        Ok(())
    }
}
