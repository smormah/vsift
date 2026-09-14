//! Bounded whole-artifact verification before private staging may be activated.

use std::{
    error::Error,
    fmt,
    io::{self, Read, Write},
};

use sha2::{Digest, Sha256};
use vsift_domain::ArtifactIntegrity;

const TRANSFER_BUFFER_BYTES: usize = 16 * 1024;

/// A typed reason a candidate artifact must not be activated.
#[derive(Debug)]
pub enum ArtifactTransferError {
    /// The source ended before the reviewed byte count.
    Truncated,
    /// The source continued beyond the reviewed byte count.
    Oversized,
    /// The complete source did not match the reviewed SHA-256.
    DigestMismatch,
    /// The source could not be read to completion.
    SourceIo(io::ErrorKind),
    /// The staging destination could not accept or flush the candidate.
    DestinationIo(io::ErrorKind),
}

impl fmt::Display for ArtifactTransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Truncated => "artifact ended before its reviewed size",
            Self::Oversized => "artifact exceeded its reviewed size",
            Self::DigestMismatch => "artifact SHA-256 differs from reviewed bytes",
            Self::SourceIo(_) => "artifact source read failed",
            Self::DestinationIo(_) => "artifact staging write failed",
        })
    }
}

impl Error for ArtifactTransferError {}

/// Streams exactly the reviewed bytes into an unactivated staging destination.
///
/// The caller must discard the destination on **every** error. This function does
/// not open a URL, select a source, extract an archive or activate any runtime.
/// It reads one extra byte after the expected size to detect a changed response.
///
/// # Errors
///
/// Returns a typed size, digest, source-I/O or destination-I/O failure.
pub fn transfer_verified<Source, Destination>(
    mut source: Source,
    mut destination: Destination,
    integrity: ArtifactIntegrity,
) -> Result<(), ArtifactTransferError>
where
    Source: Read,
    Destination: Write,
{
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; TRANSFER_BUFFER_BYTES];
    let mut copied = 0_u64;
    while copied < integrity.bytes() {
        let remaining = integrity.bytes() - copied;
        let limit = usize::try_from(remaining).map_or(TRANSFER_BUFFER_BYTES, |bytes| {
            bytes.min(TRANSFER_BUFFER_BYTES)
        });
        let read = read_retry(&mut source, &mut buffer[..limit])?;
        if read == 0 {
            return Err(ArtifactTransferError::Truncated);
        }
        destination
            .write_all(&buffer[..read])
            .map_err(|error| ArtifactTransferError::DestinationIo(error.kind()))?;
        digest.update(&buffer[..read]);
        copied += read as u64;
    }
    let mut extra = [0_u8; 1];
    if read_retry(&mut source, &mut extra)? != 0 {
        return Err(ArtifactTransferError::Oversized);
    }
    let observed: [u8; 32] = digest.finalize().into();
    if observed != integrity.sha256() {
        return Err(ArtifactTransferError::DigestMismatch);
    }
    destination
        .flush()
        .map_err(|error| ArtifactTransferError::DestinationIo(error.kind()))
}

fn read_retry(source: &mut impl Read, buffer: &mut [u8]) -> Result<usize, ArtifactTransferError> {
    loop {
        match source.read(buffer) {
            Ok(read) => return Ok(read),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ArtifactTransferError::SourceIo(error.kind())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read, Write};

    use vsift_domain::ArtifactIntegrity;

    use super::{ArtifactTransferError, transfer_verified};

    fn fixture_integrity() -> Result<ArtifactIntegrity, Box<dyn std::error::Error>> {
        Ok(ArtifactIntegrity::from_sha256_hex(
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )?)
    }

    #[test]
    fn accepts_only_exact_complete_bytes() -> Result<(), Box<dyn std::error::Error>> {
        let mut output = Vec::new();
        transfer_verified(Cursor::new(b"abc"), &mut output, fixture_integrity()?)?;
        assert_eq!(output, b"abc");
        Ok(())
    }

    struct FragmentedSource {
        offset: usize,
        interrupted: bool,
    }

    impl Read for FragmentedSource {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let bytes = b"abc";
            if self.offset == bytes.len() {
                return Ok(0);
            }
            buffer[0] = bytes[self.offset];
            self.offset += 1;
            Ok(1)
        }
    }

    #[test]
    fn fragmented_source_with_one_interruption_still_verifies()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut output = Vec::new();
        transfer_verified(
            FragmentedSource {
                offset: 0,
                interrupted: false,
            },
            &mut output,
            fixture_integrity()?,
        )?;
        assert_eq!(output, b"abc");
        Ok(())
    }

    #[test]
    fn rejects_truncation_excess_and_digest_mismatch() -> Result<(), Box<dyn std::error::Error>> {
        let integrity = fixture_integrity()?;
        assert!(matches!(
            transfer_verified(Cursor::new(b"ab"), Vec::new(), integrity),
            Err(ArtifactTransferError::Truncated)
        ));
        assert!(matches!(
            transfer_verified(Cursor::new(b"abcd"), Vec::new(), integrity),
            Err(ArtifactTransferError::Oversized)
        ));
        assert!(matches!(
            transfer_verified(Cursor::new(b"abd"), Vec::new(), integrity),
            Err(ArtifactTransferError::DigestMismatch)
        ));
        Ok(())
    }

    struct BrokenSource;

    impl Read for BrokenSource {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("source failure"))
        }
    }

    struct BrokenDestination;

    impl Write for BrokenDestination {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("destination failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn distinguishes_source_and_destination_io_failures() -> Result<(), Box<dyn std::error::Error>>
    {
        let integrity = fixture_integrity()?;
        assert!(matches!(
            transfer_verified(BrokenSource, Vec::new(), integrity),
            Err(ArtifactTransferError::SourceIo(io::ErrorKind::Other))
        ));
        assert!(matches!(
            transfer_verified(Cursor::new(b"abc"), BrokenDestination, integrity),
            Err(ArtifactTransferError::DestinationIo(io::ErrorKind::Other))
        ));
        Ok(())
    }
}
