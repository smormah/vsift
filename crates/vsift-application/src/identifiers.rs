//! Fresh opaque identifiers as an injected port.
//!
//! New session and operation identities must be unguessable in production but
//! predictable in tests. Generating them behind a port keeps randomness out of the
//! use cases and lets a host or test supply its own source.

use std::{error::Error, fmt};

use vsift_domain::{OperationId, SessionId};

/// Why a fresh identifier could not be produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierGenerationError {
    /// The source of unpredictable bytes was unavailable.
    EntropyUnavailable,
    /// The produced value was not a canonical identifier of the requested type.
    NonCanonical,
}

impl fmt::Display for IdentifierGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EntropyUnavailable => "identifier entropy source is unavailable",
            Self::NonCanonical => "generated identifier is not canonical",
        })
    }
}

impl Error for IdentifierGenerationError {}

/// Port that issues fresh, never-reused opaque identifiers.
///
/// Identifiers name sessions and operations in public results and on disk, so
/// an implementation must not repeat a value within the lifetime of a session
/// root. Production sources are random; test sources may be sequential.
pub trait IdentifierSource: Send + Sync {
    /// Returns a fresh session identity.
    ///
    /// # Errors
    ///
    /// Returns [`IdentifierGenerationError`] when no identifier can be issued.
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError>;

    /// Returns a fresh operation identity.
    ///
    /// # Errors
    ///
    /// Returns [`IdentifierGenerationError`] when no identifier can be issued.
    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError>;
}
