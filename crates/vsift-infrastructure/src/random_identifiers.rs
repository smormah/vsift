//! Operating-system randomness behind the application [`IdentifierSource`] port.

use vsift_application::{IdentifierGenerationError, IdentifierSource};
use vsift_domain::{OperationId, SessionId};

const HEX: &[u8; 16] = b"0123456789abcdef";
/// 128 bits make a collision within one session root negligible.
const RANDOM_BYTES: usize = 16;

/// Issues identifiers with a type prefix and 128 random bits as lowercase hex.
///
/// The value is unguessable, so a public identifier never reveals how many
/// sessions or operations exist and cannot be enumerated.
#[derive(Clone, Copy, Debug, Default)]
pub struct RandomIdentifierSource;

impl RandomIdentifierSource {
    fn random_identifier(prefix: &str) -> Result<String, IdentifierGenerationError> {
        let mut random = [0_u8; RANDOM_BYTES];
        getrandom::fill(&mut random).map_err(|_| IdentifierGenerationError::EntropyUnavailable)?;
        let mut result = String::with_capacity(prefix.len() + RANDOM_BYTES * 2);
        result.push_str(prefix);
        for byte in random {
            result.push(char::from(HEX[usize::from(byte >> 4)]));
            result.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        Ok(result)
    }
}

impl IdentifierSource for RandomIdentifierSource {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        SessionId::parse(Self::random_identifier("ses_")?)
            .map_err(|_| IdentifierGenerationError::NonCanonical)
    }

    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError> {
        OperationId::parse(Self::random_identifier("op_")?)
            .map_err(|_| IdentifierGenerationError::NonCanonical)
    }
}

#[cfg(test)]
mod tests {
    use vsift_application::IdentifierSource;

    use super::RandomIdentifierSource;

    #[test]
    fn identifiers_are_prefixed_lowercase_hex_and_fresh() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = RandomIdentifierSource;
        let first = source.session_id()?;
        let second = source.session_id()?;
        let operation = source.operation_id()?;

        assert_ne!(first, second);
        for (identifier, prefix) in [(first.as_str(), "ses_"), (operation.as_str(), "op_")] {
            let suffix = identifier
                .strip_prefix(prefix)
                .ok_or("identifier lost its type prefix")?;
            assert_eq!(suffix.len(), 32);
            assert!(
                suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            );
        }
        Ok(())
    }
}
