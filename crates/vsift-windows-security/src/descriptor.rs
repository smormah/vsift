//! The security descriptor of a private directory, as an SDDL string.
//!
//! Plain Rust with no system call, so it builds and is tested on every
//! platform; only `create_directory` needs Windows.

use std::fmt::{self, Write as _};

/// `LocalSystem`.
pub const SYSTEM_SID: &str = "S-1-5-18";
/// The local Administrators group.
pub const ADMINISTRATORS_SID: &str = "S-1-5-32-544";

/// The most sub-authorities a SID holds (`SID_MAX_SUB_AUTHORITIES`).
const MAX_SUB_AUTHORITIES: usize = 15;
/// The largest identifier authority: a SID stores it in six bytes.
const MAX_IDENTIFIER_AUTHORITY: u64 = (1 << 48) - 1;

/// Why a descriptor could not be built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    /// The owner is not a SID in its string form (`S-1-<authority>-<sub>...`,
    /// decimal numbers only). Nothing else may reach the SDDL text.
    InvalidSid,
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSid => formatter.write_str("the owner is not a SID in string form"),
        }
    }
}

impl std::error::Error for DescriptorError {}

/// The security descriptor every directory `VSift` creates as private is given
/// at creation (ADR 0026).
///
/// The owner is the given account, and the DACL is **protected** (nothing is
/// inherited from the parent) with exactly three entries: full control
/// (`FILE_ALL_ACCESS`), inherited by every file and folder created inside
/// (`OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE`), for the owner,
/// `LocalSystem` and the local Administrators group. These are the entries the
/// directories `VSift` made private before ADR 0026 ended with, and the
/// principals its validator trusts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateDirectoryDescriptor {
    sddl: String,
}

impl PrivateDirectoryDescriptor {
    /// The descriptor for a directory private to `owner_sid`, the string SID
    /// of the account that will use it (normally the current user).
    ///
    /// # Errors
    ///
    /// [`DescriptorError::InvalidSid`] when `owner_sid` is not a SID in its
    /// string form; the SID is checked before it is written into the SDDL
    /// text, so no other SDDL can be smuggled in through it.
    pub fn for_owner(owner_sid: &str) -> Result<Self, DescriptorError> {
        if !is_string_sid(owner_sid) {
            return Err(DescriptorError::InvalidSid);
        }
        let mut sddl = format!("O:{owner_sid}D:P");
        for trustee in [owner_sid, SYSTEM_SID, ADMINISTRATORS_SID] {
            // Allow, object and container inherit, file all access.
            let _ = write!(sddl, "(A;OICI;FA;;;{trustee})");
        }
        Ok(Self { sddl })
    }

    /// The descriptor in Security Descriptor Definition Language.
    #[must_use]
    pub fn sddl(&self) -> &str {
        &self.sddl
    }
}

/// Whether `text` is `S-1-<authority>(-<sub-authority>){1,15}` with decimal
/// numbers that fit their fields and no leading zeros.
fn is_string_sid(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("S-1-") else {
        return false;
    };
    let mut parts = rest.split('-');
    let authority_fits = parts
        .next()
        .and_then(decimal)
        .is_some_and(|authority| authority <= MAX_IDENTIFIER_AUTHORITY);
    if !authority_fits {
        return false;
    }
    let mut sub_authorities = 0_usize;
    for part in parts {
        sub_authorities += 1;
        if sub_authorities > MAX_SUB_AUTHORITIES
            || decimal(part).is_none_or(|value| value > u64::from(u32::MAX))
        {
            return false;
        }
    }
    sub_authorities >= 1
}

/// A non-empty run of ASCII digits without a leading zero (except `0`
/// itself), as a number; `None` for anything else or for a run too long.
fn decimal(part: &str) -> Option<u64> {
    let digits_only = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let leading_zero = part.len() > 1 && part.starts_with('0');
    if !digits_only || leading_zero {
        return None;
    }
    part.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        ADMINISTRATORS_SID, DescriptorError, PrivateDirectoryDescriptor, SYSTEM_SID, is_string_sid,
    };

    const USER: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";

    #[test]
    fn the_descriptor_names_the_owner_and_three_protected_inheritable_entries() {
        let descriptor = PrivateDirectoryDescriptor::for_owner(USER);
        assert_eq!(
            descriptor.as_ref().map(PrivateDirectoryDescriptor::sddl),
            Ok(concat!(
                "O:S-1-5-21-1111111111-2222222222-3333333333-1001",
                "D:P",
                "(A;OICI;FA;;;S-1-5-21-1111111111-2222222222-3333333333-1001)",
                "(A;OICI;FA;;;S-1-5-18)",
                "(A;OICI;FA;;;S-1-5-32-544)",
            ))
        );
    }

    /// The DACL is protected (`P`), and nothing in it is inherited (`ID`), so
    /// nothing the parent grants can reach the new directory.
    #[test]
    fn the_dacl_is_protected_and_inherits_nothing() -> Result<(), DescriptorError> {
        let descriptor = PrivateDirectoryDescriptor::for_owner(USER)?;
        let dacl = descriptor
            .sddl()
            .split_once("D:")
            .map_or("", |(_, dacl)| dacl);
        assert!(dacl.starts_with("P("), "{dacl}");
        assert!(!dacl.contains("ID"), "{dacl}");
        assert!(!descriptor.sddl().contains("S:"), "no SACL is set");
        assert_eq!(dacl.matches("(A;OICI;FA;;;").count(), 3);
        for trustee in [USER, SYSTEM_SID, ADMINISTRATORS_SID] {
            assert!(dacl.contains(&format!(";;;{trustee})")), "{trustee}");
        }
        Ok(())
    }

    #[test]
    fn well_known_and_account_sids_are_accepted() {
        for sid in [
            USER,
            SYSTEM_SID,
            ADMINISTRATORS_SID,
            "S-1-0-0",
            "S-1-281474976710655-4294967295",
            "S-1-5-1-2-3-4-5-6-7-8-9-10-11-12-13-14-15",
        ] {
            assert!(is_string_sid(sid), "{sid}");
        }
    }

    /// Anything that is not exactly a string SID is refused before it reaches
    /// the SDDL text: an SDDL fragment, an alias, a hexadecimal authority,
    /// fields that overflow, too many or no sub-authorities.
    #[test]
    fn anything_else_is_refused() {
        for text in [
            "",
            "S-1-5",
            "S-1-5-",
            "S-2-5-18",
            "s-1-5-18",
            "SY",
            "S-1-5-18)(A;OICI;FA;;;WD",
            "S-1-5-18;",
            "S-1-0x5-18",
            "S-1-5--18",
            "S-1-5-018",
            "S-1-5-4294967296",
            "S-1-281474976710656-1",
            "S-1-5-1-2-3-4-5-6-7-8-9-10-11-12-13-14-15-16",
            " S-1-5-18",
            "S-1-5-18\0",
        ] {
            assert!(!is_string_sid(text), "{text:?}");
            assert_eq!(
                PrivateDirectoryDescriptor::for_owner(text),
                Err(DescriptorError::InvalidSid),
                "{text:?}"
            );
        }
    }
}
