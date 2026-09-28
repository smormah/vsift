//! Bounded, strict decoding of JSON documents a caller hands to `VSift`.
//!
//! P01 froze this decoder in the CLI before any command read a request file;
//! P11 moves it here because the worker request ([`crate::decode_work_request`])
//! is a published contract every host must decode identically. A document is
//! measured and its nesting checked on the raw bytes before `serde` sees it,
//! so a hostile file cannot make the deserializer allocate or recurse beyond
//! the budget. Each target type denies unknown fields itself.

use std::{error::Error, fmt};

use serde::de::DeserializeOwned;
use vsift_domain::FailureCode;

/// Byte and nesting budget of one JSON document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonLimits {
    /// Most bytes the document may hold.
    pub max_bytes: usize,
    /// Most nested objects and arrays.
    pub max_nesting: usize,
}

impl JsonLimits {
    /// The general budget of a request or configuration document (CLI
    /// contract "Configuration and compatibility"): 1 MiB and 64 levels. The
    /// saved `setup plan` a `setup install` accepts is read with it.
    pub const DOCUMENT: Self = Self {
        max_bytes: 1_048_576,
        max_nesting: 64,
    };
}

/// Why a JSON document was refused before it was used.
#[derive(Debug)]
pub enum StrictJsonError {
    /// The document is larger than its byte budget.
    TooLarge,
    /// Objects and arrays nest deeper than the budget.
    TooDeep,
    /// The bytes are not a valid instance of the strict target type.
    Malformed(serde_json::Error),
}

impl StrictJsonError {
    /// The public failure code: every refusal is the caller's input.
    #[must_use]
    pub const fn code(&self) -> FailureCode {
        FailureCode::InvalidArgument
    }
}

impl fmt::Display for StrictJsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "JSON document exceeds the byte limit",
            Self::TooDeep => "JSON document exceeds the nesting limit",
            Self::Malformed(_) => "JSON document does not match its strict schema",
        })
    }
}

impl Error for StrictJsonError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Malformed(error) => Some(error),
            Self::TooLarge | Self::TooDeep => None,
        }
    }
}

/// Decodes one bounded strict document. `T` must deny unknown fields.
///
/// # Errors
///
/// [`StrictJsonError::TooLarge`] or [`StrictJsonError::TooDeep`] before any
/// deserialization, then [`StrictJsonError::Malformed`].
pub fn decode_strict_json<T>(bytes: &[u8], limits: JsonLimits) -> Result<T, StrictJsonError>
where
    T: DeserializeOwned,
{
    if bytes.len() > limits.max_bytes {
        return Err(StrictJsonError::TooLarge);
    }
    check_nesting(bytes, limits.max_nesting)?;
    serde_json::from_slice(bytes).map_err(StrictJsonError::Malformed)
}

/// Counts container depth outside strings. Unbalanced closers are left to
/// the parser, which rejects them as malformed.
fn check_nesting(bytes: &[u8], max_nesting: usize) -> Result<(), StrictJsonError> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                if depth > max_nesting {
                    return Err(StrictJsonError::TooDeep);
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use vsift_domain::SessionId;

    use super::{JsonLimits, StrictJsonError, decode_strict_json};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TestRequest {
        #[serde(rename = "schema_version")]
        _schema_version: String,
        action: TestAction,
        session_id: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum TestAction {
        Inspect,
    }

    #[test]
    fn a_valid_document_is_decoded() -> TestResult {
        let request: TestRequest = decode_strict_json(
            br#"{"schema_version":"1","action":"inspect","session_id":"ses_0123456789abcdef"}"#,
            JsonLimits::DOCUMENT,
        )?;
        assert!(matches!(request.action, TestAction::Inspect));
        assert!(SessionId::parse(request.session_id).is_ok());
        Ok(())
    }

    #[test]
    fn missing_unknown_and_invalid_enum_fields_are_refused() {
        for bytes in [
            br#"{"schema_version":"1","action":"inspect"}"#.as_slice(),
            br#"{"schema_version":"1","action":"inspect","session_id":"ses_0123456789abcdef","extra":true}"#.as_slice(),
            br#"{"schema_version":"1","action":"execute","session_id":"ses_0123456789abcdef"}"#.as_slice(),
        ] {
            assert!(matches!(
                decode_strict_json::<TestRequest>(bytes, JsonLimits::DOCUMENT),
                Err(StrictJsonError::Malformed(_))
            ));
        }
    }

    #[test]
    fn byte_and_nesting_budgets_fail_before_deserialization() {
        let limits = JsonLimits {
            max_bytes: 64,
            max_nesting: 4,
        };
        assert!(matches!(
            decode_strict_json::<serde_json::Value>(&[b' '; 65], limits),
            Err(StrictJsonError::TooLarge)
        ));
        let deep = format!("{}0{}", "[".repeat(5), "]".repeat(5));
        assert!(matches!(
            decode_strict_json::<serde_json::Value>(deep.as_bytes(), limits),
            Err(StrictJsonError::TooDeep)
        ));
        let within = format!("{}0{}", "[".repeat(4), "]".repeat(4));
        assert!(decode_strict_json::<serde_json::Value>(within.as_bytes(), limits).is_ok());
    }

    #[test]
    fn brackets_inside_strings_do_not_count_as_nesting() -> TestResult {
        let value: serde_json::Value = decode_strict_json(
            br#"{"value":"[[[{{{\\\""}"#,
            JsonLimits {
                max_bytes: 64,
                max_nesting: 1,
            },
        )?;
        assert!(value.is_object());
        Ok(())
    }

    #[test]
    fn unbalanced_closers_are_malformed() {
        assert!(matches!(
            decode_strict_json::<serde_json::Value>(b"]]{}", JsonLimits::DOCUMENT),
            Err(StrictJsonError::Malformed(_))
        ));
    }
}
