//! Storage guarantees and immutable generation identity.

use std::{error::Error, fmt};

/// Persistence behavior requested by a caller for one workspace operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurabilityRequirement {
    /// Preserve atomic visibility across process failure without claiming OS-crash persistence.
    Ephemeral,
    /// Acknowledge only after the qualified OS and storage stack persists the publication.
    Durable,
}

impl DurabilityRequirement {
    /// Returns the minimum publication guarantee that can satisfy this requirement.
    #[must_use]
    pub const fn required_guarantee(self) -> PublicationGuarantee {
        match self {
            Self::Ephemeral => PublicationGuarantee::ProcessCrashConsistent,
            Self::Durable => PublicationGuarantee::OsCrashDurable,
        }
    }

    /// Returns the stable identifier used by configuration and machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::Durable => "durable",
        }
    }
}

/// Strongest publication behavior qualified for a storage adapter and host profile.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PublicationGuarantee {
    /// Committed generations remain atomic across process failure.
    ProcessCrashConsistent,
    /// Committed generations satisfy the qualified OS/storage crash protocol.
    OsCrashDurable,
}

impl PublicationGuarantee {
    /// Reports whether this guarantee is at least as strong as the requirement.
    #[must_use]
    pub const fn satisfies(self, requirement: DurabilityRequirement) -> bool {
        matches!(
            (self, requirement),
            (
                Self::ProcessCrashConsistent | Self::OsCrashDurable,
                DurabilityRequirement::Ephemeral
            ) | (Self::OsCrashDurable, DurabilityRequirement::Durable)
        )
    }

    /// Returns the stable identifier used in diagnostics and machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ProcessCrashConsistent => "process_crash_consistent",
            Self::OsCrashDurable => "os_crash_durable",
        }
    }
}

/// Monotonic identity of one immutable committed session manifest.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageGeneration(u64);

impl StorageGeneration {
    /// First committed generation in a new session workspace.
    pub const INITIAL: Self = Self(0);

    /// Reconstructs a generation read from validated stored metadata.
    #[must_use]
    pub const fn from_value(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stored numeric generation.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Calculates the next generation without wrapping stale-write protection.
    ///
    /// # Errors
    ///
    /// Returns [`GenerationError::Exhausted`] if no later generation can be represented.
    pub const fn successor(self) -> Result<Self, GenerationError> {
        match self.0.checked_add(1) {
            Some(value) => Ok(Self(value)),
            None => Err(GenerationError::Exhausted),
        }
    }
}

impl fmt::Display for StorageGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Failure to allocate a later immutable storage generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationError {
    /// The current generation is the largest value representable by the contract.
    Exhausted,
}

impl fmt::Display for GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("storage generation space is exhausted")
    }
}

impl Error for GenerationError {}

#[cfg(test)]
mod tests {
    use super::{DurabilityRequirement, GenerationError, PublicationGuarantee, StorageGeneration};

    #[test]
    fn publication_guarantees_never_silently_downgrade_durable_requests() {
        assert!(
            PublicationGuarantee::ProcessCrashConsistent
                .satisfies(DurabilityRequirement::Ephemeral)
        );
        assert!(
            !PublicationGuarantee::ProcessCrashConsistent.satisfies(DurabilityRequirement::Durable)
        );
        assert!(PublicationGuarantee::OsCrashDurable.satisfies(DurabilityRequirement::Ephemeral));
        assert!(PublicationGuarantee::OsCrashDurable.satisfies(DurabilityRequirement::Durable));
    }

    #[test]
    fn generation_successor_is_monotonic_and_never_wraps() {
        assert_eq!(
            StorageGeneration::INITIAL.successor(),
            Ok(StorageGeneration::from_value(1))
        );
        assert_eq!(
            StorageGeneration::from_value(u64::MAX).successor(),
            Err(GenerationError::Exhausted)
        );
    }

    #[test]
    fn storage_contract_identifiers_are_stable() {
        assert_eq!(DurabilityRequirement::Ephemeral.identifier(), "ephemeral");
        assert_eq!(DurabilityRequirement::Durable.identifier(), "durable");
        assert_eq!(
            PublicationGuarantee::ProcessCrashConsistent.identifier(),
            "process_crash_consistent"
        );
        assert_eq!(
            PublicationGuarantee::OsCrashDurable.identifier(),
            "os_crash_durable"
        );
    }
}
