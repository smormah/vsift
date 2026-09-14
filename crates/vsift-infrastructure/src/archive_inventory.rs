//! Cross-platform archive metadata checks before any extraction or activation.

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
};

/// Hard ceiling for the number of entries in one managed archive.
pub const MAX_ARCHIVE_ENTRIES: usize = 512;
/// Hard ceiling for declared expanded bytes in one managed archive.
pub const MAX_ARCHIVE_EXPANDED_BYTES: u64 = 1_073_741_824;

/// The kind reported by an archive reader, before any entry is materialized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArchiveEntryKind {
    /// An ordinary file whose contents still require bounded extraction.
    Regular,
    /// A directory, which is never trusted as an extraction destination.
    Directory,
    /// A symbolic link that may match a reviewed alias but is never extracted.
    SymbolicLink {
        /// Link target exactly as encoded by the archive.
        target: String,
    },
    /// Any hard link, device, FIFO or otherwise unsupported entry type.
    Other,
}

/// Untrusted metadata supplied by an archive reader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEntry {
    /// Full path as encoded in the archive, with `/` separators.
    pub path: String,
    /// Declared entry kind.
    pub kind: ArchiveEntryKind,
    /// Declared expanded content bytes.
    pub bytes: u64,
}

/// One exact symlink header expected in a reviewed upstream archive.
///
/// Approval only permits inspection of the header. The extractor must create
/// any needed runtime alias as a verified regular-file copy instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewedArchiveAlias<'a> {
    /// Full archive path for the link header.
    pub path: &'a str,
    /// One relative filename in the same directory, never a filesystem path.
    pub target: &'a str,
}

/// Reviewed, bounded archive metadata budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchiveInventoryBounds {
    entries: usize,
    expanded_bytes: u64,
}

impl ArchiveInventoryBounds {
    /// Constructs limits that cannot exceed the hard managed-archive ceiling.
    ///
    /// # Errors
    ///
    /// Zero or excessive limits are rejected.
    pub fn new(entries: usize, expanded_bytes: u64) -> Result<Self, ArchiveInventoryError> {
        if entries == 0
            || entries > MAX_ARCHIVE_ENTRIES
            || expanded_bytes == 0
            || expanded_bytes > MAX_ARCHIVE_EXPANDED_BYTES
        {
            return Err(ArchiveInventoryError::InvalidBounds);
        }
        Ok(Self {
            entries,
            expanded_bytes,
        })
    }

    /// Maximum number of entries approved for this archive.
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Maximum sum of declared expanded entry bytes.
    #[must_use]
    pub const fn expanded_bytes(self) -> u64 {
        self.expanded_bytes
    }
}

/// A typed reason archive metadata cannot proceed to extraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveInventoryError {
    /// The reviewed budget is zero or exceeds the hard policy ceiling.
    InvalidBounds,
    /// The archive has no entries.
    Empty,
    /// Entry count exceeded the reviewed limit.
    TooManyEntries,
    /// Declared expanded bytes exceeded the reviewed limit.
    TooManyBytes,
    /// A path or reviewed alias uses unsafe or nonportable syntax.
    UnsafePath,
    /// Two entries resolve to the same portable case-insensitive path.
    DuplicatePath,
    /// A directory or link declared content bytes.
    UnexpectedContent,
    /// An archive link differs from the exact reviewed alias inventory.
    UnreviewedLink,
    /// An expected reviewed alias was absent from the archive.
    MissingReviewedAlias,
    /// A hard link, device, FIFO or other special entry was encountered.
    SpecialEntry,
}

impl fmt::Display for ArchiveInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBounds => "archive inventory bounds are invalid",
            Self::Empty => "archive has no entries",
            Self::TooManyEntries => "archive entry count exceeds reviewed limit",
            Self::TooManyBytes => "archive expanded bytes exceed reviewed limit",
            Self::UnsafePath => "archive path is unsafe or nonportable",
            Self::DuplicatePath => "archive contains duplicate portable paths",
            Self::UnexpectedContent => "archive non-file entry declares content",
            Self::UnreviewedLink => "archive contains an unreviewed link",
            Self::MissingReviewedAlias => "archive lacks a reviewed alias",
            Self::SpecialEntry => "archive contains a special entry",
        })
    }
}

impl Error for ArchiveInventoryError {}

/// Checks the complete archive index against bounds and reviewed link headers.
///
/// This has no filesystem effects. The caller must inspect every archive member,
/// preserve its exact path and type, then separately validate selected file names,
/// sizes and digests while extracting into a private contained staging directory.
/// A passing metadata check does not authorize extraction or activation.
///
/// # Errors
///
/// Rejects invalid budgets, paths, duplicates, unexpected links or special
/// entries, missing aliases and excessive declared expansion.
pub fn validate_archive_inventory(
    entries: &[ArchiveEntry],
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    bounds: ArchiveInventoryBounds,
) -> Result<(), ArchiveInventoryError> {
    if entries.is_empty() {
        return Err(ArchiveInventoryError::Empty);
    }
    if entries.len() > bounds.entries {
        return Err(ArchiveInventoryError::TooManyEntries);
    }
    let mut expected = HashMap::new();
    for alias in reviewed_aliases {
        if !safe_archive_path(alias.path, false) || !safe_alias_target(alias.target) {
            return Err(ArchiveInventoryError::UnsafePath);
        }
        if expected
            .insert(alias.path.to_ascii_lowercase(), alias.target)
            .is_some()
        {
            return Err(ArchiveInventoryError::DuplicatePath);
        }
    }
    let mut seen = HashSet::new();
    let mut expanded = 0_u64;
    for entry in entries {
        let directory = entry.kind == ArchiveEntryKind::Directory;
        if !safe_archive_path(&entry.path, directory) {
            return Err(ArchiveInventoryError::UnsafePath);
        }
        let normalized = entry.path.trim_end_matches('/').to_ascii_lowercase();
        if !seen.insert(normalized.clone()) {
            return Err(ArchiveInventoryError::DuplicatePath);
        }
        expanded = expanded
            .checked_add(entry.bytes)
            .ok_or(ArchiveInventoryError::TooManyBytes)?;
        if expanded > bounds.expanded_bytes {
            return Err(ArchiveInventoryError::TooManyBytes);
        }
        match &entry.kind {
            ArchiveEntryKind::Regular => {
                if expected.contains_key(&normalized) {
                    return Err(ArchiveInventoryError::UnreviewedLink);
                }
            }
            ArchiveEntryKind::Directory => {
                if entry.bytes != 0 {
                    return Err(ArchiveInventoryError::UnexpectedContent);
                }
            }
            ArchiveEntryKind::SymbolicLink { target } => {
                if entry.bytes != 0 {
                    return Err(ArchiveInventoryError::UnexpectedContent);
                }
                match expected.remove(&normalized) {
                    Some(reviewed) if reviewed == target => {}
                    _ => return Err(ArchiveInventoryError::UnreviewedLink),
                }
            }
            ArchiveEntryKind::Other => return Err(ArchiveInventoryError::SpecialEntry),
        }
    }
    if !expected.is_empty() {
        return Err(ArchiveInventoryError::MissingReviewedAlias);
    }
    Ok(())
}

pub(crate) fn safe_archive_path(path: &str, directory: bool) -> bool {
    let path = if directory {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    !path.is_empty()
        && path.len() <= 240
        && path
            .bytes()
            .all(|byte| (0x20..=0x7e).contains(&byte) && byte != b'\\' && byte != b':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn safe_alias_target(target: &str) -> bool {
    safe_archive_path(target, false) && !target.contains('/')
}

#[cfg(test)]
mod tests {
    use super::{
        ArchiveEntry, ArchiveEntryKind, ArchiveInventoryBounds, ArchiveInventoryError,
        ReviewedArchiveAlias, validate_archive_inventory,
    };

    fn bounds() -> Result<ArchiveInventoryBounds, ArchiveInventoryError> {
        ArchiveInventoryBounds::new(5, 100)
    }

    fn regular(path: &str, bytes: u64) -> ArchiveEntry {
        ArchiveEntry {
            path: path.to_owned(),
            kind: ArchiveEntryKind::Regular,
            bytes,
        }
    }

    #[test]
    fn accepts_reviewed_alias_header_without_materializing_it() -> Result<(), ArchiveInventoryError>
    {
        let entries = [
            ArchiveEntry {
                path: "root/".to_owned(),
                kind: ArchiveEntryKind::Directory,
                bytes: 0,
            },
            regular("root/lib.so.1", 10),
            ArchiveEntry {
                path: "root/lib.so".to_owned(),
                kind: ArchiveEntryKind::SymbolicLink {
                    target: "lib.so.1".to_owned(),
                },
                bytes: 0,
            },
        ];
        validate_archive_inventory(
            &entries,
            &[ReviewedArchiveAlias {
                path: "root/lib.so",
                target: "lib.so.1",
            }],
            bounds()?,
        )
    }

    #[test]
    fn rejects_cross_platform_traversal_and_duplicate_names() -> Result<(), ArchiveInventoryError> {
        for path in [
            "/root/file",
            "C:/root/file",
            "root/../file",
            "root/./file",
            "root\\file",
            "root//file",
            "root/file/",
            "root/\u{7f}file",
        ] {
            assert_eq!(
                validate_archive_inventory(&[regular(path, 1)], &[], bounds()?),
                Err(ArchiveInventoryError::UnsafePath),
                "{path}"
            );
        }
        assert_eq!(
            validate_archive_inventory(
                &[regular("root/File", 1), regular("root/file", 1)],
                &[],
                bounds()?
            ),
            Err(ArchiveInventoryError::DuplicatePath)
        );
        Ok(())
    }

    #[test]
    fn rejects_unreviewed_or_changed_links_and_special_entries() -> Result<(), ArchiveInventoryError>
    {
        let link = ArchiveEntry {
            path: "root/lib.so".to_owned(),
            kind: ArchiveEntryKind::SymbolicLink {
                target: "../outside".to_owned(),
            },
            bytes: 0,
        };
        assert_eq!(
            validate_archive_inventory(&[link], &[], bounds()?),
            Err(ArchiveInventoryError::UnreviewedLink)
        );
        assert_eq!(
            validate_archive_inventory(
                &[regular("root/file", 1)],
                &[ReviewedArchiveAlias {
                    path: "root/lib.so",
                    target: "lib.so.1",
                }],
                bounds()?
            ),
            Err(ArchiveInventoryError::MissingReviewedAlias)
        );
        assert_eq!(
            validate_archive_inventory(
                &[ArchiveEntry {
                    path: "root/device".to_owned(),
                    kind: ArchiveEntryKind::Other,
                    bytes: 0,
                }],
                &[],
                bounds()?
            ),
            Err(ArchiveInventoryError::SpecialEntry)
        );
        Ok(())
    }

    #[test]
    fn enforces_entry_and_expanded_byte_budgets() -> Result<(), ArchiveInventoryError> {
        assert_eq!(
            validate_archive_inventory(
                &[regular("root/a", 1), regular("root/b", 1)],
                &[],
                ArchiveInventoryBounds::new(1, 100)?
            ),
            Err(ArchiveInventoryError::TooManyEntries)
        );
        assert_eq!(
            validate_archive_inventory(&[regular("root/a", 101)], &[], bounds()?),
            Err(ArchiveInventoryError::TooManyBytes)
        );
        assert_eq!(
            ArchiveInventoryBounds::new(0, 100),
            Err(ArchiveInventoryError::InvalidBounds)
        );
        Ok(())
    }
}
