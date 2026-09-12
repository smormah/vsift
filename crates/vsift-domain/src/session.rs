//! Deterministic disposable-session lifecycle values.

use std::{error::Error, fmt};

/// A session's committed ability to accept new work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionPhase {
    /// The source is bound and the session accepts work.
    Open,
    /// The caller closed the session; only retention and cleanup remain.
    Closed,
}

/// Media evidence forms that P04 can publish within a P05 session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionArtifactKind {
    /// A source-grounded extracted frame encoded as PNG.
    FramePng,
    /// A bounded mono signed-16-bit PCM audio segment.
    AudioPcm,
}

impl SessionArtifactKind {
    /// Stable manifest and public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FramePng => "frame_png",
            Self::AudioPcm => "audio_pcm",
        }
    }

    /// Fixed contained filename extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::FramePng => "png",
            Self::AudioPcm => "pcm",
        }
    }
}

impl SessionPhase {
    /// Stable identifier used in stored and public contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

/// An explicit transition rejected by the session policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionTransitionError {
    /// A deadline was invalid or outside the bounded retention policy.
    InvalidExpiry,
    /// The session has already been closed or expired.
    NotOpen,
}

impl fmt::Display for SessionTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExpiry => formatter.write_str("session expiry is outside policy"),
            Self::NotOpen => formatter.write_str("session is not open"),
        }
    }
}

impl Error for SessionTransitionError {}

/// The bounded time policy for one disposable investigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionLifetime {
    opened_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

impl SessionLifetime {
    /// Default idle interval; renewal never extends beyond the hard maximum.
    pub const IDLE_SECONDS: u64 = 24 * 60 * 60;
    /// Hard limit measured from the initial open, including all renewals.
    pub const MAX_SECONDS: u64 = 7 * 24 * 60 * 60;

    /// Opens a session with the accepted desktop expiry policy.
    ///
    /// # Errors
    ///
    /// Rejects a clock value whose bounded expiry cannot be represented.
    pub fn open(now_unix_seconds: u64) -> Result<Self, SessionTransitionError> {
        let expires_at_unix_seconds = now_unix_seconds
            .checked_add(Self::IDLE_SECONDS)
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        now_unix_seconds
            .checked_add(Self::MAX_SECONDS)
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        Ok(Self {
            opened_at_unix_seconds: now_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    /// Validates an untrusted persisted lifetime before using it for cleanup.
    ///
    /// # Errors
    ///
    /// Rejects malformed or overlong deadlines.
    pub fn from_record(
        opened_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self, SessionTransitionError> {
        let hard = opened_at_unix_seconds
            .checked_add(Self::MAX_SECONDS)
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        let initial = opened_at_unix_seconds
            .checked_add(Self::IDLE_SECONDS)
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        if expires_at_unix_seconds < initial || expires_at_unix_seconds > hard {
            return Err(SessionTransitionError::InvalidExpiry);
        }
        Ok(Self {
            opened_at_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    /// Returns the original open time.
    #[must_use]
    pub const fn opened_at_unix_seconds(self) -> u64 {
        self.opened_at_unix_seconds
    }

    /// Returns the current committed expiry.
    #[must_use]
    pub const fn expires_at_unix_seconds(self) -> u64 {
        self.expires_at_unix_seconds
    }

    /// Reports whether a session is eligible for expiry at an injected clock value.
    #[must_use]
    pub const fn expired(self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.expires_at_unix_seconds
    }

    /// Renews an unexpired session up to the seven-day hard limit.
    ///
    /// # Errors
    ///
    /// Rejects a renewal after expiry or a clock overflow.
    pub fn renew(self, now_unix_seconds: u64) -> Result<Self, SessionTransitionError> {
        if self.expired(now_unix_seconds) {
            return Err(SessionTransitionError::NotOpen);
        }
        let hard = self
            .opened_at_unix_seconds
            .checked_add(Self::MAX_SECONDS)
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        let proposed = now_unix_seconds
            .checked_add(Self::IDLE_SECONDS)
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        Ok(Self {
            opened_at_unix_seconds: self.opened_at_unix_seconds,
            expires_at_unix_seconds: proposed.min(hard).max(self.expires_at_unix_seconds),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionLifetime, SessionTransitionError};

    #[test]
    fn expiry_boundary_and_hard_cap_are_deterministic() -> Result<(), SessionTransitionError> {
        let original = SessionLifetime::open(1_000)?;
        assert!(!original.expired(original.expires_at_unix_seconds() - 1));
        assert!(original.expired(original.expires_at_unix_seconds()));
        let renewed = original.renew(original.expires_at_unix_seconds() - 1)?;
        assert_eq!(
            renewed.expires_at_unix_seconds(),
            1_000 + SessionLifetime::IDLE_SECONDS * 2 - 1
        );
        assert_eq!(
            original.renew(original.expires_at_unix_seconds()),
            Err(SessionTransitionError::NotOpen)
        );
        assert_eq!(
            SessionLifetime::from_record(1_000, 1_000 + SessionLifetime::MAX_SECONDS + 1),
            Err(SessionTransitionError::InvalidExpiry)
        );
        assert_eq!(
            SessionLifetime::from_record(1_000, 1_000 + SessionLifetime::IDLE_SECONDS - 1),
            Err(SessionTransitionError::InvalidExpiry)
        );
        assert_eq!(original.renew(900)?, original);
        let mut capped = original;
        for _ in 0..10 {
            capped = capped.renew(capped.expires_at_unix_seconds() - 1)?;
        }
        assert_eq!(
            capped.expires_at_unix_seconds(),
            1_000 + SessionLifetime::MAX_SECONDS
        );
        Ok(())
    }
}
