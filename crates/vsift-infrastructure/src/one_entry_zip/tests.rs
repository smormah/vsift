use std::{
    fmt::Write as _,
    fs,
    io::{Cursor, Write as _},
};

use cap_std::{ambient_authority, fs::Dir};
use flate2::{Compression, write::DeflateEncoder};
use sha2::{Digest, Sha256};
use vsift_domain::ArtifactIntegrity;

use super::{OneEntryZipError, ReviewedZipEntry, read_one_entry_zip, stage_one_entry_zip};
use crate::test_folder::TestFolder;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CONTENT: &[u8] =
    b"#!/runtime\nreviewed runtime bytes, repeated: runtime runtime runtime runtime\n";

/// How the test writer lays out an archive.
#[derive(Clone, Copy)]
struct Layout {
    method: u16,
    descriptor: Descriptor,
    host: u8,
    mode: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Descriptor {
    None,
    Signed,
    Unsigned,
}

const UNIX_DEFLATED: Layout = Layout {
    method: 8,
    descriptor: Descriptor::None,
    host: 3,
    mode: 0o100_755,
};

/// The parts of one archive, kept apart so a test can change one.
struct Parts {
    local: Vec<u8>,
    data: Vec<u8>,
    descriptor: Vec<u8>,
    central: Vec<u8>,
}

impl Parts {
    fn new(name: &str, content: &[u8], layout: Layout) -> Result<Self, Box<dyn std::error::Error>> {
        let data = if layout.method == 8 {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
            encoder.write_all(content)?;
            encoder.finish()?
        } else {
            content.to_vec()
        };
        let mut crc = flate2::Crc::new();
        crc.update(content);
        let crc32 = crc.sum();
        let compressed = u32::try_from(data.len())?;
        let expanded = u32::try_from(content.len())?;
        let name_length = u16::try_from(name.len())?;
        let flags: u16 = if layout.descriptor == Descriptor::None {
            0
        } else {
            0x0008
        };
        let (local_crc, local_compressed, local_expanded) = if layout.descriptor == Descriptor::None
        {
            (crc32, compressed, expanded)
        } else {
            (0, 0, 0)
        };
        let version: u16 = if layout.method == 8 { 20 } else { 10 };

        let mut local = Vec::new();
        local.extend(0x0403_4b50_u32.to_le_bytes());
        local.extend(version.to_le_bytes());
        local.extend(flags.to_le_bytes());
        local.extend(layout.method.to_le_bytes());
        local.extend([0_u8; 4]);
        local.extend(local_crc.to_le_bytes());
        local.extend(local_compressed.to_le_bytes());
        local.extend(local_expanded.to_le_bytes());
        local.extend(name_length.to_le_bytes());
        local.extend(4_u16.to_le_bytes());
        local.extend(name.as_bytes());
        local.extend([0xca, 0xfe, 0, 0]);

        let mut descriptor = Vec::new();
        if layout.descriptor == Descriptor::Signed {
            descriptor.extend(0x0807_4b50_u32.to_le_bytes());
        }
        if layout.descriptor != Descriptor::None {
            descriptor.extend(crc32.to_le_bytes());
            descriptor.extend(compressed.to_le_bytes());
            descriptor.extend(expanded.to_le_bytes());
        }

        let mut central = Vec::new();
        central.extend(0x0201_4b50_u32.to_le_bytes());
        central.extend(u16::from_le_bytes([20, layout.host]).to_le_bytes());
        central.extend(version.to_le_bytes());
        central.extend(flags.to_le_bytes());
        central.extend(layout.method.to_le_bytes());
        central.extend([0_u8; 4]);
        central.extend(crc32.to_le_bytes());
        central.extend(compressed.to_le_bytes());
        central.extend(expanded.to_le_bytes());
        central.extend(name_length.to_le_bytes());
        central.extend(0_u16.to_le_bytes());
        central.extend(0_u16.to_le_bytes());
        central.extend(0_u16.to_le_bytes());
        central.extend(0_u16.to_le_bytes());
        // A Unix mode sits in the upper half; MS-DOS attributes in the lower.
        let external = if layout.host == 3 {
            layout.mode << 16
        } else {
            layout.mode
        };
        central.extend(external.to_le_bytes());
        central.extend(0_u32.to_le_bytes());
        central.extend(name.as_bytes());
        Ok(Self {
            local,
            data,
            descriptor,
            central,
        })
    }

    fn bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.bytes_with(|_| {}, 1, 0, &[])
    }

    /// The archive, with `end` applied to the end record's fields after
    /// they are written (entry count, comment length) and `gap` bytes put
    /// between the data and the central directory.
    fn bytes_with(
        &self,
        edit_end: impl FnOnce(&mut Vec<u8>),
        entries: u16,
        comment: u16,
        gap: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut archive = Vec::new();
        archive.extend(&self.local);
        archive.extend(&self.data);
        archive.extend(&self.descriptor);
        archive.extend(gap);
        let central_offset = u32::try_from(archive.len())?;
        archive.extend(&self.central);
        let mut end = Vec::new();
        end.extend(0x0605_4b50_u32.to_le_bytes());
        end.extend(0_u16.to_le_bytes());
        end.extend(0_u16.to_le_bytes());
        end.extend(entries.to_le_bytes());
        end.extend(entries.to_le_bytes());
        end.extend(u32::try_from(self.central.len())?.to_le_bytes());
        end.extend(central_offset.to_le_bytes());
        end.extend(comment.to_le_bytes());
        edit_end(&mut end);
        archive.extend(end);
        Ok(archive)
    }
}

fn reviewed(content: &[u8]) -> Result<ArtifactIntegrity, Box<dyn std::error::Error>> {
    let digest = Sha256::digest(content);
    let mut hex = String::new();
    for byte in digest {
        write!(&mut hex, "{byte:02x}")?;
    }
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(content.len())?,
        &hex,
    )?)
}

fn read(archive: &[u8], entry: ReviewedZipEntry<'_>) -> Result<Vec<u8>, OneEntryZipError> {
    let mut output = Vec::new();
    read_one_entry_zip(
        Cursor::new(archive),
        u64::try_from(archive.len()).unwrap_or(u64::MAX),
        1024 * 1024,
        entry,
        &mut output,
    )?;
    Ok(output)
}

fn entry(name: &str) -> Result<ReviewedZipEntry<'_>, Box<dyn std::error::Error>> {
    Ok(ReviewedZipEntry {
        path: name,
        integrity: reviewed(CONTENT)?,
    })
}

#[test]
fn stored_and_deflated_entries_read_back_exactly_with_or_without_a_descriptor() -> TestResult {
    for method in [0, 8] {
        for descriptor in [Descriptor::None, Descriptor::Signed, Descriptor::Unsigned] {
            for (host, mode) in [(3, 0o100_755), (3, 0), (0, 0x20)] {
                let layout = Layout {
                    method,
                    descriptor,
                    host,
                    mode,
                };
                let archive = Parts::new("deno", CONTENT, layout)?.bytes()?;
                assert_eq!(read(&archive, entry("deno")?)?, CONTENT);
            }
        }
    }
    Ok(())
}

#[test]
fn the_entry_is_staged_under_the_callers_name_and_removed_on_failure() -> TestResult {
    let folder = TestFolder::new("vsift-one-entry-zip")?;
    let staging = Dir::open_ambient_dir(&*folder, ambient_authority())?;
    let good = folder.join("good.zip");
    fs::write(&good, Parts::new("deno", CONTENT, UNIX_DEFLATED)?.bytes()?)?;
    let length = fs::metadata(&good)?.len();
    stage_one_entry_zip(
        fs::File::open(&good)?,
        length,
        1024,
        entry("deno")?,
        &staging,
        "runtime",
    )?;
    assert_eq!(fs::read(folder.join("runtime"))?, CONTENT);
    // The name exists now: nothing is overwritten.
    assert_eq!(
        stage_one_entry_zip(
            fs::File::open(&good)?,
            length,
            1024,
            entry("deno")?,
            &staging,
            "runtime",
        ),
        Err(OneEntryZipError::InvalidStaging)
    );

    let bad = folder.join("bad.zip");
    fs::write(
        &bad,
        Parts::new("deno", b"other bytes", UNIX_DEFLATED)?.bytes()?,
    )?;
    let length = fs::metadata(&bad)?.len();
    assert_eq!(
        stage_one_entry_zip(
            fs::File::open(&bad)?,
            length,
            1024,
            entry("deno")?,
            &staging,
            "second"
        ),
        Err(OneEntryZipError::EntryMismatch)
    );
    assert!(!folder.join("second").exists());
    for unsafe_name in ["", "..", "a/b", "CON", "name."] {
        assert_eq!(
            stage_one_entry_zip(
                fs::File::open(&good)?,
                fs::metadata(&good)?.len(),
                1024,
                entry("deno")?,
                &staging,
                unsafe_name
            ),
            Err(OneEntryZipError::InvalidStaging),
            "{unsafe_name:?}"
        );
    }
    drop(staging);
    Ok(())
}

#[test]
fn an_entry_that_differs_from_the_review_is_refused() -> TestResult {
    let archive = Parts::new("deno", CONTENT, UNIX_DEFLATED)?.bytes()?;
    assert_eq!(
        read(&archive, entry("other")?),
        Err(OneEntryZipError::EntryMismatch)
    );
    let other = Parts::new("deno", b"different bytes", UNIX_DEFLATED)?.bytes()?;
    assert_eq!(
        read(&other, entry("deno")?),
        Err(OneEntryZipError::EntryMismatch)
    );
    // Same size, different bytes: only the digest tells.
    let mut flipped = CONTENT.to_vec();
    flipped[0] ^= 1;
    let same_size = Parts::new("deno", &flipped, UNIX_DEFLATED)?.bytes()?;
    assert_eq!(
        read(&same_size, entry("deno")?),
        Err(OneEntryZipError::EntryMismatch)
    );
    // A wrong CRC in both records.
    let mut parts = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    parts.local[14] ^= 1;
    parts.central[16] ^= 1;
    assert_eq!(
        read(&parts.bytes()?, entry("deno")?),
        Err(OneEntryZipError::EntryMismatch)
    );
    Ok(())
}

#[test]
fn a_bomb_or_a_short_stream_is_refused_by_its_declared_sizes() -> TestResult {
    // The data expands to far more than the records declare.
    let large = vec![b'a'; 512 * 1024];
    let mut parts = Parts::new("deno", &large, UNIX_DEFLATED)?;
    let declared = u32::try_from(CONTENT.len())?.to_le_bytes();
    parts.local[22..26].copy_from_slice(&declared);
    parts.central[24..28].copy_from_slice(&declared);
    assert_eq!(
        read(&parts.bytes()?, entry("deno")?),
        Err(OneEntryZipError::EntryMismatch)
    );

    // The deflate stream ends before its declared compressed size: bytes
    // ride along inside the entry's data.
    let mut parts = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    parts.data.extend(b"hidden");
    let compressed = u32::try_from(parts.data.len())?.to_le_bytes();
    parts.local[18..22].copy_from_slice(&compressed);
    parts.central[20..24].copy_from_slice(&compressed);
    assert_eq!(
        read(&parts.bytes()?, entry("deno")?),
        Err(OneEntryZipError::EntryMismatch)
    );

    // A corrupt deflate stream.
    let mut parts = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    for byte in &mut parts.data {
        *byte = 0xff;
    }
    assert_eq!(
        read(&parts.bytes()?, entry("deno")?),
        Err(OneEntryZipError::EntryMismatch)
    );
    Ok(())
}

#[test]
fn anything_but_one_regular_file_is_refused() -> TestResult {
    for (host, mode) in [
        (3, 0o120_777),
        (3, 0o040_755),
        (3, 0o020_644),
        (0, 0x10),
        (11, 0),
    ] {
        let layout = Layout {
            method: 8,
            descriptor: Descriptor::None,
            host,
            mode,
        };
        let archive = Parts::new("deno", CONTENT, layout)?.bytes()?;
        assert_eq!(
            read(&archive, entry("deno")?),
            Err(OneEntryZipError::NotOneRegularFile),
            "{host} {mode:o}"
        );
    }
    let directory = Parts::new("deno/", CONTENT, UNIX_DEFLATED)?.bytes()?;
    assert_eq!(
        read(&directory, entry("deno/")?),
        Err(OneEntryZipError::NotOneRegularFile)
    );
    let parts = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    for entries in [0, 2] {
        assert_eq!(
            read(&parts.bytes_with(|_| {}, entries, 0, &[])?, entry("deno")?),
            Err(OneEntryZipError::NotOneRegularFile)
        );
    }
    Ok(())
}

#[test]
fn refused_features_are_unsupported() -> TestResult {
    let parts = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    // Encryption, in both records.
    let mut encrypted = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    encrypted.local[6] |= 1;
    encrypted.central[8] |= 1;
    // A compression method other than stored or deflated.
    let mut bzip2 = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    bzip2.local[8] = 12;
    bzip2.central[10] = 12;
    // Version 4.5 (Zip64).
    let mut zip64 = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    zip64.local[4] = 45;
    zip64.central[6] = 45;
    for archive in [encrypted.bytes()?, bzip2.bytes()?, zip64.bytes()?] {
        assert_eq!(
            read(&archive, entry("deno")?),
            Err(OneEntryZipError::Unsupported)
        );
    }
    // Zip64 markers and another disk in the end record.
    let markers = parts.bytes_with(|end| end[16..20].copy_from_slice(&[0xff; 4]), 1, 0, &[])?;
    let disk = parts.bytes_with(|end| end[4] = 1, 1, 0, &[])?;
    for archive in [markers, disk] {
        assert_eq!(
            read(&archive, entry("deno")?),
            Err(OneEntryZipError::Unsupported)
        );
    }
    // A declared comment: refused before its bytes matter.
    let mut commented = parts.bytes_with(|_| {}, 1, 3, &[])?;
    commented.extend(b"hey");
    assert_eq!(
        read(&commented, entry("deno")?),
        Err(OneEntryZipError::Malformed)
    );
    let declared_only = parts.bytes_with(|_| {}, 1, 3, &[])?;
    assert_eq!(
        read(&declared_only, entry("deno")?),
        Err(OneEntryZipError::Unsupported)
    );
    Ok(())
}

#[test]
fn bytes_outside_the_declared_records_are_malformed() -> TestResult {
    let parts = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    let gap = parts.bytes_with(|_| {}, 1, 0, b"gap")?;
    let mut trailing = parts.bytes()?;
    trailing.extend(b"trailing");
    let mut leading = b"lead".to_vec();
    leading.extend(parts.bytes()?);
    for archive in [gap, trailing, leading] {
        assert_eq!(
            read(&archive, entry("deno")?),
            Err(OneEntryZipError::Malformed)
        );
    }
    // A local header that disagrees with the central record.
    let mut disagreeing = Parts::new("deno", CONTENT, UNIX_DEFLATED)?;
    disagreeing.local[22] ^= 1;
    assert_eq!(
        read(&disagreeing.bytes()?, entry("deno")?),
        Err(OneEntryZipError::Malformed)
    );
    // A data descriptor that disagrees.
    let mut descriptor = Parts::new(
        "deno",
        CONTENT,
        Layout {
            descriptor: Descriptor::Signed,
            ..UNIX_DEFLATED
        },
    )?;
    descriptor.descriptor[4] ^= 1;
    assert_eq!(
        read(&descriptor.bytes()?, entry("deno")?),
        Err(OneEntryZipError::Malformed)
    );
    Ok(())
}

#[test]
fn every_truncation_and_every_single_byte_change_is_refused_without_panic() -> TestResult {
    let archive = Parts::new("deno", CONTENT, UNIX_DEFLATED)?.bytes()?;
    for length in 0..archive.len() {
        assert!(
            read(&archive[..length], entry("deno")?).is_err(),
            "{length}"
        );
    }
    for index in 0..archive.len() {
        let mut changed = archive.clone();
        changed[index] ^= 0x5a;
        // A change inside an extra field or the timestamps may be harmless;
        // whatever is accepted must still be exactly the reviewed bytes.
        if let Ok(output) = read(&changed, entry("deno")?) {
            assert_eq!(output, CONTENT, "{index}");
        }
    }
    Ok(())
}

#[test]
fn the_archive_must_be_exactly_its_reviewed_size_and_limits_must_be_sane() -> TestResult {
    let archive = Parts::new("deno", CONTENT, UNIX_DEFLATED)?.bytes()?;
    let length = u64::try_from(archive.len())?;
    let mut output = Vec::new();
    assert_eq!(
        read_one_entry_zip(
            Cursor::new(&archive),
            length - 1,
            1024,
            entry("deno")?,
            &mut output
        ),
        Err(OneEntryZipError::ArchiveTooLarge)
    );
    assert_eq!(
        read_one_entry_zip(
            Cursor::new(&archive),
            length + 1,
            1024,
            entry("deno")?,
            &mut output
        ),
        Err(OneEntryZipError::Malformed)
    );
    for (archive_bytes, max_expanded) in [(0, 1024), (length, 0), (length, 10)] {
        assert_eq!(
            read_one_entry_zip(
                Cursor::new(&archive),
                archive_bytes,
                max_expanded,
                entry("deno")?,
                &mut output
            ),
            Err(OneEntryZipError::InvalidLimits)
        );
    }
    Ok(())
}
