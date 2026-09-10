//! Bounded paging and locally scoped continuation cursor contracts.

use std::{error::Error, fmt};

use crate::SessionId;

const CURSOR_VERSION: &str = "v1";
const CURSOR_SEPARATOR: char = '|';
const MAX_CURSOR_BYTES: usize = 512;
const QUERY_DIGEST_LENGTH: usize = 64;
const MAX_LAST_KEY_LENGTH: usize = 64;

/// Number of items requested from one immutable snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageLimit(u16);

impl PageLimit {
    /// Default number of result cards returned by an operation.
    pub const DEFAULT: Self = Self(20);
    /// Maximum number of result cards accepted by R0 contracts.
    pub const MAX: u16 = 100;

    /// Creates a positive limit no larger than the public maximum.
    ///
    /// # Errors
    ///
    /// Returns [`PageLimitError`] when `value` is zero or greater than 100.
    pub fn new(value: u16) -> Result<Self, PageLimitError> {
        if value == 0 {
            return Err(PageLimitError::Zero);
        }
        if value > Self::MAX {
            return Err(PageLimitError::TooLarge);
        }
        Ok(Self(value))
    }

    /// Returns the validated item count.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Why a requested page size was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageLimitError {
    /// Pages cannot request zero items.
    Zero,
    /// The request exceeds the public R0 cap.
    TooLarge,
}

impl fmt::Display for PageLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Zero => "page limit must be positive",
            Self::TooLarge => "page limit exceeds the maximum of 100",
        };
        formatter.write_str(message)
    }
}

impl Error for PageLimitError {}

/// SHA-256 digest of the canonical query that produced a page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryDigest(String);

impl QueryDigest {
    /// Parses a canonical lowercase SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns [`CursorError`] when the digest is not canonical SHA-256 hexadecimal.
    pub fn parse(value: impl Into<String>) -> Result<Self, CursorError> {
        let value = value.into();
        if value.len() != QUERY_DIGEST_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(CursorError::InvalidQueryDigest);
        }
        Ok(Self(value))
    }

    /// Returns the canonical lowercase digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Decoded continuation state for one immutable result snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorToken {
    session_id: SessionId,
    generation: u64,
    query_digest: QueryDigest,
    last_item_key: String,
    expires_at_micros: u64,
}

impl CursorToken {
    /// Creates a scoped continuation token.
    ///
    /// # Errors
    ///
    /// Returns [`CursorError`] when generation, expiry, or the last-item key is invalid.
    pub fn new(
        session_id: SessionId,
        generation: u64,
        query_digest: QueryDigest,
        last_item_key: impl Into<String>,
        expires_at_micros: u64,
    ) -> Result<Self, CursorError> {
        let last_item_key = last_item_key.into();
        validate_last_item_key(&last_item_key)?;
        if generation == 0 || expires_at_micros == 0 {
            return Err(CursorError::InvalidNumber);
        }
        Ok(Self {
            session_id,
            generation,
            query_digest,
            last_item_key,
            expires_at_micros,
        })
    }

    /// Encodes the bounded local cursor without filesystem paths or credentials.
    #[must_use]
    pub fn encode(&self) -> String {
        format!(
            "{CURSOR_VERSION}{CURSOR_SEPARATOR}{}{CURSOR_SEPARATOR}{}{CURSOR_SEPARATOR}{}{CURSOR_SEPARATOR}{}{CURSOR_SEPARATOR}{}",
            self.session_id,
            self.generation,
            self.query_digest.as_str(),
            self.last_item_key,
            self.expires_at_micros
        )
    }

    /// Parses a token without granting it validity for a particular request.
    ///
    /// # Errors
    ///
    /// Returns [`CursorError`] for oversized, malformed, or unsupported tokens.
    pub fn parse(value: &str) -> Result<Self, CursorError> {
        if value.len() > MAX_CURSOR_BYTES {
            return Err(CursorError::TooLarge);
        }
        let mut parts = value.split(CURSOR_SEPARATOR);
        if parts.next() != Some(CURSOR_VERSION) {
            return Err(CursorError::UnsupportedVersion);
        }
        let session_id = parts
            .next()
            .ok_or(CursorError::Malformed)?
            .parse()
            .map_err(|_| CursorError::Malformed)?;
        let generation = parse_positive_number(parts.next())?;
        let query_digest = QueryDigest::parse(parts.next().ok_or(CursorError::Malformed)?)?;
        let last_item_key = parts.next().ok_or(CursorError::Malformed)?;
        let expires_at_micros = parse_positive_number(parts.next())?;
        if parts.next().is_some() {
            return Err(CursorError::Malformed);
        }
        Self::new(
            session_id,
            generation,
            query_digest,
            last_item_key,
            expires_at_micros,
        )
    }

    /// Verifies that the token belongs to the caller's immutable snapshot and query.
    ///
    /// # Errors
    ///
    /// Returns [`CursorError`] when scope, generation, query, or expiry does not match.
    pub fn validate_scope(
        &self,
        session_id: &SessionId,
        generation: u64,
        query_digest: &QueryDigest,
        now_micros: u64,
    ) -> Result<(), CursorError> {
        if now_micros >= self.expires_at_micros {
            return Err(CursorError::Expired);
        }
        if &self.session_id != session_id {
            return Err(CursorError::WrongSession);
        }
        if self.generation != generation {
            return Err(CursorError::WrongGeneration);
        }
        if &self.query_digest != query_digest {
            return Err(CursorError::WrongQuery);
        }
        Ok(())
    }

    /// Returns the stable key after which the next page starts.
    #[must_use]
    pub fn last_item_key(&self) -> &str {
        &self.last_item_key
    }
}

/// Why a continuation token was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorError {
    /// The encoded token exceeds the input budget.
    TooLarge,
    /// The token uses an unknown major format.
    UnsupportedVersion,
    /// The token is syntactically invalid.
    Malformed,
    /// A required generation or expiry value is zero or invalid.
    InvalidNumber,
    /// The query digest is not canonical SHA-256 hexadecimal.
    InvalidQueryDigest,
    /// The page key is empty, too large, or outside its safe alphabet.
    InvalidLastItemKey,
    /// The token's validity window has ended.
    Expired,
    /// The token belongs to a different session.
    WrongSession,
    /// The immutable result generation changed.
    WrongGeneration,
    /// The token belongs to a different canonical query.
    WrongQuery,
}

impl fmt::Display for CursorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TooLarge => "cursor exceeds the maximum encoded size",
            Self::UnsupportedVersion => "cursor version is unsupported",
            Self::Malformed => "cursor is malformed",
            Self::InvalidNumber => "cursor contains an invalid numeric field",
            Self::InvalidQueryDigest => "cursor query digest is invalid",
            Self::InvalidLastItemKey => "cursor item key is invalid",
            Self::Expired => "cursor has expired",
            Self::WrongSession => "cursor belongs to a different session",
            Self::WrongGeneration => "cursor snapshot generation is no longer current",
            Self::WrongQuery => "cursor belongs to a different query",
        };
        formatter.write_str(message)
    }
}

impl Error for CursorError {}

fn parse_positive_number(value: Option<&str>) -> Result<u64, CursorError> {
    let parsed = value
        .ok_or(CursorError::Malformed)?
        .parse::<u64>()
        .map_err(|_| CursorError::InvalidNumber)?;
    if parsed == 0 {
        return Err(CursorError::InvalidNumber);
    }
    Ok(parsed)
}

fn validate_last_item_key(value: &str) -> Result<(), CursorError> {
    if value.is_empty()
        || value.len() > MAX_LAST_KEY_LENGTH
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(CursorError::InvalidLastItemKey);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CursorError, CursorToken, PageLimit, QueryDigest};
    use crate::SessionId;

    const QUERY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const QUERY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn session(value: &str) -> Result<SessionId, Box<dyn std::error::Error>> {
        Ok(SessionId::parse(value)?)
    }

    #[test]
    fn page_limits_enforce_zero_default_maximum_and_maximum_plus_one() {
        assert!(PageLimit::new(0).is_err());
        assert_eq!(PageLimit::new(1).map(PageLimit::get), Ok(1));
        assert_eq!(PageLimit::DEFAULT.get(), 20);
        assert_eq!(PageLimit::new(PageLimit::MAX).map(PageLimit::get), Ok(100));
        assert!(PageLimit::new(PageLimit::MAX + 1).is_err());
    }

    #[test]
    fn cursor_round_trip_preserves_snapshot_position() -> Result<(), Box<dyn std::error::Error>> {
        let token = CursorToken::new(
            session("ses_0123456789abcdef")?,
            7,
            QueryDigest::parse(QUERY_A)?,
            "candidate_0042",
            10_000,
        )?;

        let decoded = CursorToken::parse(&token.encode())?;

        assert_eq!(decoded, token);
        assert_eq!(decoded.last_item_key(), "candidate_0042");
        Ok(())
    }

    #[test]
    fn cursor_scope_rejects_reuse_across_query_session_generation_and_expiry()
    -> Result<(), Box<dyn std::error::Error>> {
        let expected_session = session("ses_0123456789abcdef")?;
        let expected_query = QueryDigest::parse(QUERY_A)?;
        let token = CursorToken::new(
            expected_session.clone(),
            7,
            expected_query.clone(),
            "candidate_0042",
            10_000,
        )?;

        assert_eq!(
            token.validate_scope(&session("ses_fedcba9876543210")?, 7, &expected_query, 9_000),
            Err(CursorError::WrongSession)
        );
        assert_eq!(
            token.validate_scope(&expected_session, 8, &expected_query, 9_000),
            Err(CursorError::WrongGeneration)
        );
        assert_eq!(
            token.validate_scope(&expected_session, 7, &QueryDigest::parse(QUERY_B)?, 9_000),
            Err(CursorError::WrongQuery)
        );
        assert_eq!(
            token.validate_scope(&expected_session, 7, &expected_query, 10_000),
            Err(CursorError::Expired)
        );
        Ok(())
    }

    #[test]
    fn cursor_rejects_unknown_version_path_data_and_oversize() {
        assert_eq!(
            CursorToken::parse("v2|ses_0123456789abcdef|1|bad|key|2"),
            Err(CursorError::UnsupportedVersion)
        );
        assert_eq!(
            CursorToken::parse(&"x".repeat(513)),
            Err(CursorError::TooLarge)
        );
        assert!(CursorToken::parse(
            "v1|ses_0123456789abcdef|1|aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|../../secret|2"
        )
        .is_err());
    }
}
