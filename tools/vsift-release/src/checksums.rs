//! `SHA256SUMS`: one line per archive in the format `sha256sum --check` and
//! `shasum -a 256 --check` read, sorted by file name.

use std::{
    collections::BTreeMap,
    fmt::{self, Write},
};

use sha2::{Digest, Sha256};

/// Why a checksum list could not be written.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ChecksumError {
    /// No file was given.
    Empty,
    /// Two inputs share a file name, which one list cannot tell apart.
    DuplicateName(String),
    /// A name holds a path separator, a line break or leading whitespace,
    /// which would change how a checker reads its line.
    UnsafeName(String),
}

impl fmt::Display for ChecksumError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "no file to list"),
            Self::DuplicateName(name) => write!(formatter, "{name:?} is given twice"),
            Self::UnsafeName(name) => {
                write!(formatter, "{name:?} cannot be written on a checksum line")
            }
        }
    }
}

impl std::error::Error for ChecksumError {}

/// The `SHA256SUMS` text for `files`, each a file name and its bytes.
pub(crate) fn checksum_list<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Result<String, ChecksumError> {
    let mut lines = BTreeMap::new();
    for (name, bytes) in files {
        let unsafe_name = name.is_empty()
            || name.contains(['/', '\\', '\n', '\r'])
            || name.starts_with(char::is_whitespace);
        if unsafe_name {
            return Err(ChecksumError::UnsafeName(name.to_owned()));
        }
        let digest = Sha256::digest(bytes);
        let hex = digest.iter().fold(String::new(), |mut hex, byte| {
            let _infallible = write!(hex, "{byte:02x}");
            hex
        });
        if lines.insert(name.to_owned(), hex).is_some() {
            return Err(ChecksumError::DuplicateName(name.to_owned()));
        }
    }
    if lines.is_empty() {
        return Err(ChecksumError::Empty);
    }
    Ok(lines.iter().fold(String::new(), |mut list, (name, hex)| {
        let _infallible = writeln!(list, "{hex}  {name}");
        list
    }))
}

#[cfg(test)]
mod tests {
    use super::{ChecksumError, checksum_list};

    #[test]
    fn lines_are_sorted_and_in_sha256sum_format() {
        let list = checksum_list([("b.tar.gz", b"b".as_slice()), ("a.tar.gz", b"".as_slice())]);
        assert_eq!(
            list,
            Ok(String::from(
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  a.tar.gz\n\
                 3e23e8160039594a33894f6564e1b1348bbd7a0088d42c4acb73eeaed59c009d  b.tar.gz\n"
            ))
        );
    }

    #[test]
    fn ambiguous_or_unsafe_names_are_refused() {
        assert_eq!(checksum_list([]), Err(ChecksumError::Empty));
        assert_eq!(
            checksum_list([("a", b"1".as_slice()), ("a", b"2".as_slice())]),
            Err(ChecksumError::DuplicateName(String::from("a")))
        );
        for name in ["", "dir/a", "a\nb", " a", "c:\\a"] {
            assert_eq!(
                checksum_list([(name, b"1".as_slice())]),
                Err(ChecksumError::UnsafeName(name.to_owned())),
                "{name:?}"
            );
        }
    }
}
