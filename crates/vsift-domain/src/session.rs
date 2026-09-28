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

/// Artifact kinds a session publishes: P04 media evidence, P07 transcript
/// records, P08 visual-index records and P09 evidence clips and records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionArtifactKind {
    /// A source-grounded extracted frame encoded as PNG.
    FramePng,
    /// A bounded mono signed-16-bit PCM audio segment.
    AudioPcm,
    /// One immutable transcript revision, stored as versioned JSON (P07).
    TranscriptRecord,
    /// One immutable visual-candidate index revision, stored as versioned
    /// JSON (P08). Each revision is a superset of the one before it.
    VisualIndexRecord,
    /// An evidence audio clip: a WAV file of 16 kHz mono signed 16-bit PCM,
    /// at most 30 s (P09, ADR 0019 D5).
    AudioWav,
    /// The lineage of one evidence call: its request, the frames or clip it
    /// selected and the items it extracted, stored as versioned JSON (P09).
    EvidenceRecord,
}

impl SessionArtifactKind {
    /// Stable manifest and public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::FramePng => "frame_png",
            Self::AudioPcm => "audio_pcm",
            Self::TranscriptRecord => "transcript_record",
            Self::VisualIndexRecord => "visual_index_record",
            Self::AudioWav => "audio_wav",
            Self::EvidenceRecord => "evidence_record",
        }
    }

    /// Fixed contained filename extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::FramePng => "png",
            Self::AudioPcm => "pcm",
            Self::AudioWav => "wav",
            Self::TranscriptRecord | Self::VisualIndexRecord | Self::EvidenceRecord => "json",
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

/// How long the sessions of one worker workspace live (P11, ADR 0021
/// maintainer decision D2): at least one hour, 168 hours unless the operator
/// sets it, at most 720 hours.
///
/// The value is an operator policy recorded once in the workspace's marker;
/// a request can never choose or raise it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceRetention(u64);

impl WorkspaceRetention {
    /// The shortest retention: one hour, so a workspace session always
    /// outlives the requests that use it.
    pub const MIN: Self = Self(3_600);
    /// The retention of a workspace whose operator sets none: 168 hours.
    pub const DEFAULT: Self = Self(168 * 3_600);
    /// The longest retention, and the hard limit of every workspace
    /// session's life including renewals: 720 hours.
    pub const MAX: Self = Self(720 * 3_600);

    /// A retention of `seconds`.
    ///
    /// # Errors
    ///
    /// [`SessionTransitionError::InvalidExpiry`] outside one to 720 hours.
    pub const fn from_seconds(seconds: u64) -> Result<Self, SessionTransitionError> {
        if seconds < Self::MIN.0 || seconds > Self::MAX.0 {
            return Err(SessionTransitionError::InvalidExpiry);
        }
        Ok(Self(seconds))
    }

    /// A retention of whole `hours`, as an operator writes it.
    ///
    /// # Errors
    ///
    /// [`SessionTransitionError::InvalidExpiry`] outside 1 to 720 hours.
    pub const fn from_hours(hours: u64) -> Result<Self, SessionTransitionError> {
        match hours.checked_mul(3_600) {
            Some(seconds) => Self::from_seconds(seconds),
            None => Err(SessionTransitionError::InvalidExpiry),
        }
    }

    /// The retention in seconds.
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// The operator policy of one worker workspace (P11, ADR 0021 D1, D2):
/// how its sessions publish, how much work its root admits at once and how
/// long its sessions live. It is recorded once, when the workspace is
/// created, and never changes afterwards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspacePolicy {
    durability: crate::DurabilityRequirement,
    admission_capacity: std::num::NonZeroU16,
    retention: WorkspaceRetention,
}

/// A workspace policy outside its bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspacePolicyError {
    /// An admission capacity above [`WorkspacePolicy::MAX_ADMISSION_CAPACITY`].
    AdmissionCapacity,
}

impl fmt::Display for WorkspacePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdmissionCapacity => {
                formatter.write_str("the workspace admission capacity is out of range")
            }
        }
    }
}

impl Error for WorkspacePolicyError {}

impl WorkspacePolicy {
    /// The largest admission capacity of any root, in weight units.
    pub const MAX_ADMISSION_CAPACITY: u16 = 64;

    /// A policy.
    ///
    /// # Errors
    ///
    /// [`WorkspacePolicyError::AdmissionCapacity`] above 64 units.
    pub const fn new(
        durability: crate::DurabilityRequirement,
        admission_capacity: std::num::NonZeroU16,
        retention: WorkspaceRetention,
    ) -> Result<Self, WorkspacePolicyError> {
        if admission_capacity.get() > Self::MAX_ADMISSION_CAPACITY {
            return Err(WorkspacePolicyError::AdmissionCapacity);
        }
        Ok(Self {
            durability,
            admission_capacity,
            retention,
        })
    }

    /// How every session of the workspace publishes.
    #[must_use]
    pub const fn durability(self) -> crate::DurabilityRequirement {
        self.durability
    }

    /// The root's admission capacity, in weight units.
    #[must_use]
    pub const fn admission_capacity(self) -> std::num::NonZeroU16 {
        self.admission_capacity
    }

    /// How long the workspace's sessions live.
    #[must_use]
    pub const fn retention(self) -> WorkspaceRetention {
        self.retention
    }

    /// The lifetime rules of the workspace's sessions.
    #[must_use]
    pub const fn lifetime_policy(self) -> SessionLifetimePolicy {
        SessionLifetimePolicy::Workspace(self.retention)
    }
}

/// Which rules bound a session's life.
///
/// A desktop session (any root that is not a worker workspace) idles out
/// after 24 hours and lives at most seven days. A worker workspace session
/// lives the workspace's retention after it opens or after its latest
/// renewal, and never beyond [`WorkspaceRetention::MAX`] from its opening:
/// a supervisor that keeps using a session renews it, and an abandoned one
/// still disappears within the operator's retention (ADR 0021 D2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionLifetimePolicy {
    /// An ordinary disposable desktop session.
    Desktop,
    /// A session of an explicitly initialised worker workspace.
    Workspace(WorkspaceRetention),
}

impl SessionLifetimePolicy {
    /// How long an unrenewed session lives.
    #[must_use]
    pub const fn idle_seconds(self) -> u64 {
        match self {
            Self::Desktop => SessionLifetime::IDLE_SECONDS,
            Self::Workspace(retention) => retention.seconds(),
        }
    }

    /// The hard limit measured from the opening, including every renewal.
    #[must_use]
    pub const fn max_seconds(self) -> u64 {
        match self {
            Self::Desktop => SessionLifetime::MAX_SECONDS,
            Self::Workspace(_) => WorkspaceRetention::MAX.seconds(),
        }
    }
}

/// The bounded time policy for one disposable investigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionLifetime {
    policy: SessionLifetimePolicy,
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
    pub const fn open(now_unix_seconds: u64) -> Result<Self, SessionTransitionError> {
        Self::open_under(SessionLifetimePolicy::Desktop, now_unix_seconds)
    }

    /// Opens a session under `policy`: it expires one idle interval later.
    ///
    /// # Errors
    ///
    /// Rejects a clock value whose bounded expiry cannot be represented.
    pub const fn open_under(
        policy: SessionLifetimePolicy,
        now_unix_seconds: u64,
    ) -> Result<Self, SessionTransitionError> {
        let Some(expires_at_unix_seconds) = now_unix_seconds.checked_add(policy.idle_seconds())
        else {
            return Err(SessionTransitionError::InvalidExpiry);
        };
        if now_unix_seconds.checked_add(policy.max_seconds()).is_none() {
            return Err(SessionTransitionError::InvalidExpiry);
        }
        Ok(Self {
            policy,
            opened_at_unix_seconds: now_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    /// Validates an untrusted persisted desktop lifetime before using it for cleanup.
    ///
    /// # Errors
    ///
    /// Rejects malformed or overlong deadlines.
    pub const fn from_record(
        opened_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self, SessionTransitionError> {
        Self::from_record_under(
            SessionLifetimePolicy::Desktop,
            opened_at_unix_seconds,
            expires_at_unix_seconds,
        )
    }

    /// Validates an untrusted persisted lifetime recorded under `policy`:
    /// the expiry lies between one idle interval and the hard maximum after
    /// the opening.
    ///
    /// # Errors
    ///
    /// Rejects malformed or overlong deadlines.
    pub const fn from_record_under(
        policy: SessionLifetimePolicy,
        opened_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self, SessionTransitionError> {
        let (Some(hard), Some(initial)) = (
            opened_at_unix_seconds.checked_add(policy.max_seconds()),
            opened_at_unix_seconds.checked_add(policy.idle_seconds()),
        ) else {
            return Err(SessionTransitionError::InvalidExpiry);
        };
        if expires_at_unix_seconds < initial || expires_at_unix_seconds > hard {
            return Err(SessionTransitionError::InvalidExpiry);
        }
        Ok(Self {
            policy,
            opened_at_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    /// The rules this session's life follows.
    #[must_use]
    pub const fn policy(self) -> SessionLifetimePolicy {
        self.policy
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

    /// Renews an unexpired session by one idle interval of its policy, up
    /// to the policy's hard limit; an expiry never moves earlier.
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
            .checked_add(self.policy.max_seconds())
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        let proposed = now_unix_seconds
            .checked_add(self.policy.idle_seconds())
            .ok_or(SessionTransitionError::InvalidExpiry)?;
        Ok(Self {
            policy: self.policy,
            opened_at_unix_seconds: self.opened_at_unix_seconds,
            expires_at_unix_seconds: proposed.min(hard).max(self.expires_at_unix_seconds),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SessionLifetime, SessionLifetimePolicy, SessionTransitionError, WorkspaceRetention,
    };

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

    /// ADR 0021 D2: a workspace session lives its workspace's retention
    /// after opening or renewal, never beyond 720 hours from its opening,
    /// and its stored lifetime validates only under its own policy.
    #[test]
    fn a_workspace_session_lives_its_retention_within_the_hard_limit()
    -> Result<(), SessionTransitionError> {
        let retention = WorkspaceRetention::from_hours(168)?;
        assert_eq!(retention, WorkspaceRetention::DEFAULT);
        let policy = SessionLifetimePolicy::Workspace(retention);
        let opened = SessionLifetime::open_under(policy, 1_000)?;
        assert_eq!(opened.policy(), policy);
        assert_eq!(opened.expires_at_unix_seconds(), 1_000 + 168 * 3_600);
        let renewed = opened.renew(1_000 + 100 * 3_600)?;
        assert_eq!(renewed.expires_at_unix_seconds(), 1_000 + 268 * 3_600);
        let mut capped = renewed;
        for _ in 0..10 {
            capped = capped.renew(capped.expires_at_unix_seconds() - 1)?;
        }
        assert_eq!(
            capped.expires_at_unix_seconds(),
            1_000 + WorkspaceRetention::MAX.seconds()
        );
        assert_eq!(
            SessionLifetime::from_record_under(policy, 1_000, capped.expires_at_unix_seconds()),
            Ok(capped)
        );
        // A desktop reading of a workspace lifetime is refused, and back.
        assert_eq!(
            SessionLifetime::from_record(1_000, capped.expires_at_unix_seconds()),
            Err(SessionTransitionError::InvalidExpiry)
        );
        assert_eq!(
            SessionLifetime::from_record_under(
                policy,
                1_000,
                1_000 + SessionLifetime::IDLE_SECONDS
            ),
            Err(SessionTransitionError::InvalidExpiry)
        );
        Ok(())
    }

    #[test]
    fn a_retention_is_one_to_seven_hundred_and_twenty_hours() {
        for hours in [0, 721, u64::MAX] {
            assert_eq!(
                WorkspaceRetention::from_hours(hours),
                Err(SessionTransitionError::InvalidExpiry)
            );
        }
        assert_eq!(
            WorkspaceRetention::from_hours(1),
            Ok(WorkspaceRetention::MIN)
        );
        assert_eq!(
            WorkspaceRetention::from_hours(720),
            Ok(WorkspaceRetention::MAX)
        );
        assert_eq!(
            WorkspaceRetention::from_seconds(3_599),
            Err(SessionTransitionError::InvalidExpiry)
        );
    }
}
