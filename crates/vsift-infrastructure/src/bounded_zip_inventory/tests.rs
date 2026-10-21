use std::{
    fmt::Write as _,
    fs,
    io::{Cursor, Write as _},
};

use cap_std::{ambient_authority, fs::Dir};
use flate2::{Compression, write::DeflateEncoder};
use sha2::{Digest, Sha256};
use vsift_domain::ArtifactIntegrity;

use super::{
    ZipInventoryError, inspect_zip_inventory, inspect_zip_selected_files, stage_zip_selected_files,
};
use crate::{
    ArchiveEntryKind, ArchiveInventoryBounds, ArchiveInventoryError, ReviewedArchiveFile,
    test_folder::TestFolder,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const TOOL: &[u8] = b"MZ reviewed tool bytes, repeated: tool tool tool tool tool tool tool\n";
const LIBRARY: &[u8] = b"reviewed library bytes";
const NOTICE: &[u8] = b"licence text that is not selected";

#[derive(Clone, Copy, Eq, PartialEq)]
enum Descriptor {
    None,
    Signed,
    Unsigned,
}

/// One entry the test writer lays out.
#[derive(Clone)]
struct Spec {
    name: String,
    content: Vec<u8>,
    method: u16,
    descriptor: Descriptor,
    host: u8,
    /// A Unix mode (host 3 or 19) or MS-DOS attributes (host 0 or 10).
    attributes: u32,
    flags: u16,
    local_extra: Vec<u8>,
    central_extra: Vec<u8>,
}

impl Spec {
    fn file(name: &str, content: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            content: content.to_vec(),
            method: 8,
            descriptor: Descriptor::None,
            host: 3,
            attributes: 0o100_755,
            flags: 0,
            // An Info-ZIP style extended-timestamp block.
            local_extra: vec![0x55, 0x54, 5, 0, 1, 0, 0, 0, 0],
            central_extra: Vec::new(),
        }
    }

    fn directory(name: &str) -> Self {
        Self {
            method: 0,
            attributes: 0o040_755,
            local_extra: Vec::new(),
            ..Self::file(name, b"")
        }
    }

    fn stored(mut self) -> Self {
        self.method = 0;
        self
    }

    fn descriptor(mut self, descriptor: Descriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    fn windows(mut self) -> Self {
        self.host = 0;
        self.attributes = 0x20;
        self
    }
}

/// The parts of an archive, kept apart so that a test can damage one.
struct Archive {
    /// Each entry's local header, data and descriptor, in order.
    records: Vec<Vec<u8>>,
    /// Each entry's central directory record, in order.
    central: Vec<Vec<u8>>,
}

impl Archive {
    fn new(specs: &[Spec]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut records = Vec::new();
        let mut central = Vec::new();
        let mut offset = 0_u32;
        for spec in specs {
            let data = if spec.method == 8 {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
                encoder.write_all(&spec.content)?;
                encoder.finish()?
            } else {
                spec.content.clone()
            };
            let mut crc = flate2::Crc::new();
            crc.update(&spec.content);
            let crc32 = crc.sum();
            let compressed = u32::try_from(data.len())?;
            let expanded = u32::try_from(spec.content.len())?;
            let name_length = u16::try_from(spec.name.len())?;
            let flags = spec.flags
                | if spec.descriptor == Descriptor::None {
                    0
                } else {
                    0x0008
                };
            let (local_crc, local_compressed, local_expanded) =
                if spec.descriptor == Descriptor::None {
                    (crc32, compressed, expanded)
                } else {
                    (0, 0, 0)
                };
            let version: u16 = if spec.method == 8 { 20 } else { 10 };

            let mut record = Vec::new();
            record.extend(0x0403_4b50_u32.to_le_bytes());
            record.extend(version.to_le_bytes());
            record.extend(flags.to_le_bytes());
            record.extend(spec.method.to_le_bytes());
            record.extend([0_u8; 4]);
            record.extend(local_crc.to_le_bytes());
            record.extend(local_compressed.to_le_bytes());
            record.extend(local_expanded.to_le_bytes());
            record.extend(name_length.to_le_bytes());
            record.extend(u16::try_from(spec.local_extra.len())?.to_le_bytes());
            record.extend(spec.name.as_bytes());
            record.extend(&spec.local_extra);
            record.extend(&data);
            if spec.descriptor == Descriptor::Signed {
                record.extend(0x0807_4b50_u32.to_le_bytes());
            }
            if spec.descriptor != Descriptor::None {
                record.extend(crc32.to_le_bytes());
                record.extend(compressed.to_le_bytes());
                record.extend(expanded.to_le_bytes());
            }

            let mut entry = Vec::new();
            entry.extend(0x0201_4b50_u32.to_le_bytes());
            entry.extend(u16::from_le_bytes([20, spec.host]).to_le_bytes());
            entry.extend(version.to_le_bytes());
            entry.extend(flags.to_le_bytes());
            entry.extend(spec.method.to_le_bytes());
            entry.extend([0_u8; 4]);
            entry.extend(crc32.to_le_bytes());
            entry.extend(compressed.to_le_bytes());
            entry.extend(expanded.to_le_bytes());
            entry.extend(name_length.to_le_bytes());
            entry.extend(u16::try_from(spec.central_extra.len())?.to_le_bytes());
            entry.extend(0_u16.to_le_bytes());
            entry.extend(0_u16.to_le_bytes());
            entry.extend(0_u16.to_le_bytes());
            let external = if matches!(spec.host, 3 | 19) {
                spec.attributes << 16
            } else {
                spec.attributes
            };
            entry.extend(external.to_le_bytes());
            entry.extend(offset.to_le_bytes());
            entry.extend(spec.name.as_bytes());
            entry.extend(&spec.central_extra);

            offset += u32::try_from(record.len())?;
            records.push(record);
            central.push(entry);
        }
        Ok(Self { records, central })
    }

    fn bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.bytes_with(|_| {})
    }

    /// The archive, with `end` applied to the end record once written.
    fn bytes_with(
        &self,
        end: impl FnOnce(&mut Vec<u8>),
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut bytes: Vec<u8> = self.records.concat();
        let central_offset = u32::try_from(bytes.len())?;
        let directory = self.central.concat();
        bytes.extend(&directory);
        let count = u16::try_from(self.central.len())?;
        let mut record = Vec::new();
        record.extend(0x0605_4b50_u32.to_le_bytes());
        record.extend(0_u16.to_le_bytes());
        record.extend(0_u16.to_le_bytes());
        record.extend(count.to_le_bytes());
        record.extend(count.to_le_bytes());
        record.extend(u32::try_from(directory.len())?.to_le_bytes());
        record.extend(central_offset.to_le_bytes());
        record.extend(0_u16.to_le_bytes());
        end(&mut record);
        bytes.extend(record);
        Ok(bytes)
    }
}

fn integrity(content: &[u8]) -> Result<ArtifactIntegrity, Box<dyn std::error::Error>> {
    let digest: [u8; 32] = Sha256::digest(content).into();
    let hex = digest.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(content.len())?,
        &hex,
    )?)
}

fn bounds() -> Result<ArchiveInventoryBounds, ArchiveInventoryError> {
    ArchiveInventoryBounds::new(8, 4_096)
}

/// A Windows-style archive: a version-named root with `bin/`, the tool, a
/// library beside it and a notice that is not selected.
fn release_archive() -> Vec<Spec> {
    vec![
        Spec::directory("root/"),
        Spec::directory("root/bin/"),
        Spec::file("root/bin/tool.exe", TOOL),
        Spec::file("root/bin/library.dll", LIBRARY).stored(),
        Spec::file("root/LICENSE.txt", NOTICE),
    ]
}

fn selection() -> Result<Vec<(String, ArtifactIntegrity)>, Box<dyn std::error::Error>> {
    Ok(vec![
        (String::from("root/bin/tool.exe"), integrity(TOOL)?),
        (String::from("root/bin/library.dll"), integrity(LIBRARY)?),
    ])
}

fn reviewed(selection: &[(String, ArtifactIntegrity)]) -> Vec<ReviewedArchiveFile<'_>> {
    selection
        .iter()
        .map(|(path, integrity)| ReviewedArchiveFile {
            path,
            integrity: *integrity,
        })
        .collect()
}

fn check(archive: &[u8]) -> Result<(), ZipInventoryError> {
    let selection = selection().map_err(|_| ZipInventoryError::InvalidLimits)?;
    let bounds = bounds().map_err(ZipInventoryError::Inventory)?;
    inspect_zip_selected_files(
        Cursor::new(archive),
        u64::try_from(archive.len()).unwrap_or(u64::MAX),
        bounds,
        &reviewed(&selection),
    )
    .map(|_| ())
}

/// What staging did, and the names left in the folder.
type Staged = (Result<(), ZipInventoryError>, Vec<String>);

/// Stages `archive` into a fresh folder and returns the outcome with the
/// names left in the folder.
fn stage(
    archive: &[u8],
    selected: &[ReviewedArchiveFile<'_>],
) -> Result<Staged, Box<dyn std::error::Error>> {
    let folder = TestFolder::new("vsift-zip-stage")?;
    let artifact_path = folder.join("artifact.zip");
    fs::write(&artifact_path, archive)?;
    let staging_path = folder.join("payload");
    fs::create_dir(&staging_path)?;
    let staging = Dir::open_ambient_dir(&staging_path, ambient_authority())?;
    let outcome = stage_zip_selected_files(
        fs::File::open(&artifact_path)?,
        u64::try_from(archive.len())?,
        bounds()?,
        selected,
        &staging,
    )
    .map(|_| ());
    let mut names = fs::read_dir(&staging_path)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    drop(staging);
    Ok((outcome, names))
}

#[test]
fn a_release_archive_is_indexed_and_only_its_selected_files_are_staged() -> TestResult {
    let archive = Archive::new(&release_archive())?.bytes()?;
    let entries = inspect_zip_inventory(
        Cursor::new(&archive),
        u64::try_from(archive.len())?,
        bounds()?,
    )?;
    let index: Vec<(&str, &ArchiveEntryKind, u64)> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), &entry.kind, entry.bytes))
        .collect();
    assert_eq!(
        index,
        [
            ("root/", &ArchiveEntryKind::Directory, 0),
            ("root/bin/", &ArchiveEntryKind::Directory, 0),
            ("root/bin/tool.exe", &ArchiveEntryKind::Regular, 69),
            ("root/bin/library.dll", &ArchiveEntryKind::Regular, 22),
            ("root/LICENSE.txt", &ArchiveEntryKind::Regular, 33),
        ]
    );

    let folder = TestFolder::new("vsift-zip-stage")?;
    let artifact_path = folder.join("artifact.zip");
    fs::write(&artifact_path, &archive)?;
    let staging_path = folder.join("payload");
    fs::create_dir(&staging_path)?;
    let staging = Dir::open_ambient_dir(&staging_path, ambient_authority())?;
    let selection = selection()?;
    stage_zip_selected_files(
        fs::File::open(&artifact_path)?,
        u64::try_from(archive.len())?,
        bounds()?,
        &reviewed(&selection),
        &staging,
    )?;
    drop(staging);
    assert_eq!(fs::read(staging_path.join("tool.exe"))?, TOOL);
    assert_eq!(fs::read(staging_path.join("library.dll"))?, LIBRARY);
    assert_eq!(fs::read_dir(&staging_path)?.count(), 2);
    Ok(())
}

#[test]
fn descriptors_and_windows_attributes_are_read_as_their_writers_write_them() -> TestResult {
    for descriptor in [Descriptor::None, Descriptor::Signed, Descriptor::Unsigned] {
        let specs: Vec<Spec> = release_archive()
            .into_iter()
            .map(|spec| {
                if spec.name.ends_with('/') {
                    spec
                } else {
                    spec.descriptor(descriptor).windows()
                }
            })
            .collect();
        check(&Archive::new(&specs)?.bytes()?)?;
    }
    // A writer on macOS records its Unix modes under host 19; NTFS (10)
    // carries MS-DOS attributes.
    let mut specs = release_archive();
    specs[2].host = 19;
    specs[3].host = 10;
    specs[3].attributes = 0x20;
    check(&Archive::new(&specs)?.bytes()?)?;
    Ok(())
}

#[test]
fn a_selection_that_differs_from_the_archive_is_refused() -> TestResult {
    let archive = Archive::new(&release_archive())?.bytes()?;
    let wrong = [ReviewedArchiveFile {
        path: "root/bin/tool.exe",
        integrity: integrity(LIBRARY)?,
    }];
    assert_eq!(
        stage(&archive, &wrong)?,
        (Err(ZipInventoryError::SelectedFileMismatch), Vec::new())
    );
    let same_size_other_bytes = {
        let mut changed = TOOL.to_vec();
        changed[0] = b'Z';
        integrity(&changed)?
    };
    let altered = [ReviewedArchiveFile {
        path: "root/bin/tool.exe",
        integrity: same_size_other_bytes,
    }];
    assert_eq!(
        stage(&archive, &altered)?,
        (Err(ZipInventoryError::SelectedFileMismatch), Vec::new())
    );
    let other_case = [ReviewedArchiveFile {
        path: "root/BIN/tool.exe",
        integrity: integrity(TOOL)?,
    }];
    assert_eq!(
        stage(&archive, &other_case)?.0,
        Err(ZipInventoryError::SelectedFileMismatch)
    );
    let missing = [ReviewedArchiveFile {
        path: "root/bin/absent.exe",
        integrity: integrity(TOOL)?,
    }];
    assert_eq!(
        stage(&archive, &missing)?.0,
        Err(ZipInventoryError::MissingSelectedFile)
    );
    let directory = [ReviewedArchiveFile {
        path: "root/bin",
        integrity: integrity(TOOL)?,
    }];
    assert_eq!(
        stage(&archive, &directory)?.0,
        Err(ZipInventoryError::MissingSelectedFile)
    );
    for unsafe_path in [
        "../tool.exe",
        "root\\bin\\tool.exe",
        "C:/tool.exe",
        "root/bin/CON",
    ] {
        let selected = [ReviewedArchiveFile {
            path: unsafe_path,
            integrity: integrity(TOOL)?,
        }];
        assert_eq!(
            stage(&archive, &selected)?.0,
            Err(ZipInventoryError::InvalidSelection),
            "{unsafe_path}"
        );
    }
    let twice = [
        ReviewedArchiveFile {
            path: "root/bin/tool.exe",
            integrity: integrity(TOOL)?,
        },
        ReviewedArchiveFile {
            path: "root/TOOL.EXE",
            integrity: integrity(TOOL)?,
        },
    ];
    assert_eq!(
        stage(&archive, &twice)?.0,
        Err(ZipInventoryError::InvalidSelection)
    );
    assert_eq!(
        stage(&archive, &[])?.0,
        Err(ZipInventoryError::InvalidSelection)
    );
    Ok(())
}

#[test]
fn a_failure_after_the_first_staged_file_removes_what_was_created() -> TestResult {
    let archive = Archive::new(&release_archive())?.bytes()?;
    let selected = [
        ReviewedArchiveFile {
            path: "root/bin/tool.exe",
            integrity: integrity(TOOL)?,
        },
        ReviewedArchiveFile {
            path: "root/bin/library.dll",
            integrity: integrity(TOOL)?,
        },
    ];
    // The library comes after the tool in the archive and differs from its
    // review, so the tool was written first and must be gone.
    assert_eq!(
        stage(&archive, &selected)?,
        (Err(ZipInventoryError::SelectedFileMismatch), Vec::new())
    );
    Ok(())
}

#[test]
fn links_special_entries_and_unsafe_paths_are_refused_by_the_shared_policy() -> TestResult {
    let refused =
        |change: &dyn Fn(&mut Vec<Spec>)| -> Result<ZipInventoryError, Box<dyn std::error::Error>> {
            let mut specs = release_archive();
            change(&mut specs);
            match check(&Archive::new(&specs)?.bytes()?) {
                Ok(()) => Err("the archive was accepted".into()),
                Err(error) => Ok(error),
            }
        };
    let inventory = ZipInventoryError::Inventory;
    assert_eq!(
        refused(&|specs| specs[4].attributes = 0o120_777)?,
        inventory(ArchiveInventoryError::SpecialEntry)
    );
    assert_eq!(
        refused(&|specs| specs[4].attributes = 0o020_644)?,
        inventory(ArchiveInventoryError::SpecialEntry)
    );
    for name in [
        "root\\LICENSE.txt",
        "../LICENSE.txt",
        "/root/LICENSE.txt",
        "C:LICENSE.txt",
        "root//LICENSE.txt",
        "root/./LICENSE.txt",
        "root/LICENSE\u{1b}.txt",
    ] {
        assert_eq!(
            refused(&|specs| specs[4].name = name.to_owned())?,
            inventory(ArchiveInventoryError::UnsafePath),
            "{name}"
        );
    }
    assert_eq!(
        refused(&|specs| specs[4].name = String::from("root/bin/TOOL.exe"))?,
        inventory(ArchiveInventoryError::DuplicatePath)
    );
    assert_eq!(
        refused(&|specs| specs[0].content = b"x".to_vec())?,
        inventory(ArchiveInventoryError::UnexpectedContent)
    );
    // A Unix regular mode on a name that ends in `/`, or a directory mode
    // on one that does not, says two things at once.
    assert_eq!(
        refused(&|specs| specs[0].attributes = 0o100_644)?,
        ZipInventoryError::Malformed
    );
    assert_eq!(
        refused(&|specs| specs[4].attributes = 0o040_755)?,
        ZipInventoryError::Malformed
    );
    Ok(())
}

#[test]
fn refused_features_are_unsupported() -> TestResult {
    let refused =
        |change: &dyn Fn(&mut Vec<Spec>)| -> Result<ZipInventoryError, Box<dyn std::error::Error>> {
            let mut specs = release_archive();
            change(&mut specs);
            match check(&Archive::new(&specs)?.bytes()?) {
                Ok(()) => Err("the archive was accepted".into()),
                Err(error) => Ok(error),
            }
        };
    // Encryption, strong encryption and a reserved bit.
    for flag in [0x0001_u16, 0x0040, 0x2000] {
        assert_eq!(
            refused(&|specs| specs[2].flags = flag)?,
            ZipInventoryError::Unsupported
        );
    }
    // Zip64's extra block, in either header.
    let zip64 = vec![0x01, 0x00, 0x08, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        refused(&|specs| specs[2].local_extra.clone_from(&zip64))?,
        ZipInventoryError::Unsupported
    );
    assert_eq!(
        refused(&|specs| specs[2].central_extra.clone_from(&zip64))?,
        ZipInventoryError::Unsupported
    );
    // An unknown made-by host.
    assert_eq!(
        refused(&|specs| specs[2].host = 7)?,
        ZipInventoryError::Unsupported
    );
    // An extra field whose last block runs past its end.
    assert_eq!(
        refused(&|specs| specs[2].local_extra = vec![0x55, 0x54, 9, 0, 1])?,
        ZipInventoryError::Malformed
    );

    let archive = Archive::new(&release_archive())?;
    // A method other than stored or deflated (bzip2 is 12).
    let mut method = archive.bytes()?;
    let central_at = archive.records.concat().len();
    let tool_central = central_at + archive.central[0].len() + archive.central[1].len();
    method[tool_central + 10] = 12;
    assert_eq!(check(&method), Err(ZipInventoryError::Unsupported));
    // An archive comment, several disks, a central directory comment.
    assert_eq!(
        check(&archive.bytes_with(|end| end[20] = 1)?),
        Err(ZipInventoryError::Unsupported)
    );
    assert_eq!(
        check(&archive.bytes_with(|end| end[4] = 1)?),
        Err(ZipInventoryError::Unsupported)
    );
    let mut comment = archive.bytes()?;
    comment[tool_central + 32] = 1;
    assert_eq!(check(&comment), Err(ZipInventoryError::Unsupported));
    // The Zip64 markers in the end record.
    assert_eq!(
        check(&archive.bytes_with(|end| end[16..20].copy_from_slice(&[0xff; 4]))?),
        Err(ZipInventoryError::Unsupported)
    );
    Ok(())
}

#[test]
fn bytes_outside_the_declared_records_are_malformed() -> TestResult {
    let archive = Archive::new(&release_archive())?;
    let good = archive.bytes()?;

    // A prefix, as a self-extractor has.
    let mut prefixed = vec![0_u8; 4];
    prefixed.extend(&good);
    assert_eq!(check(&prefixed), Err(ZipInventoryError::Malformed));

    // A byte after the end record.
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(check(&trailing), Err(ZipInventoryError::Malformed));

    // A gap between two entries.
    let mut gapped = Archive::new(&release_archive())?;
    gapped.records[2].push(0);
    for (index, record) in gapped.central.iter_mut().enumerate().skip(3) {
        let offset = u32::from_le_bytes([record[42], record[43], record[44], record[45]]) + 1;
        record[42..46].copy_from_slice(&offset.to_le_bytes());
        assert!(index > 2);
    }
    assert_eq!(check(&gapped.bytes()?), Err(ZipInventoryError::Malformed));

    // Two central records for the same local data: the classic overlap.
    let mut overlapping = Archive::new(&release_archive())?;
    let shared = overlapping.central[2][42..46].to_vec();
    overlapping.central[4][42..46].copy_from_slice(&shared);
    assert_eq!(
        check(&overlapping.bytes()?),
        Err(ZipInventoryError::Malformed)
    );

    // A local header that names its entry differently.
    let mut renamed = Archive::new(&release_archive())?;
    let name_at = 30;
    renamed.records[2][name_at] = b'R';
    assert_eq!(check(&renamed.bytes()?), Err(ZipInventoryError::Malformed));

    // A local header whose sizes disagree with the central record.
    let mut sized = Archive::new(&release_archive())?;
    sized.records[2][22] ^= 1;
    assert_eq!(check(&sized.bytes()?), Err(ZipInventoryError::Malformed));

    // The end record miscounting the central directory.
    assert_eq!(
        check(&archive.bytes_with(|end| {
            end[8] = 4;
            end[10] = 4;
        })?),
        Err(ZipInventoryError::Malformed)
    );
    Ok(())
}

#[test]
fn a_bomb_or_a_short_stream_is_refused_by_its_declared_sizes() -> TestResult {
    // A deflated stream that expands past the size its headers declare.
    let mut archive = Archive::new(&release_archive())?;
    for record in [
        &mut archive.records[2][22..26],
        &mut archive.central[2][24..28],
    ] {
        let declared = u32::from_le_bytes([record[0], record[1], record[2], record[3]]) - 1;
        record.copy_from_slice(&declared.to_le_bytes());
    }
    let shorter = [ReviewedArchiveFile {
        path: "root/bin/tool.exe",
        integrity: integrity(&TOOL[..TOOL.len() - 1])?,
    }];
    assert_eq!(
        stage(&archive.bytes()?, &shorter)?,
        (Err(ZipInventoryError::SelectedFileMismatch), Vec::new())
    );
    Ok(())
}

#[test]
fn the_bounds_and_the_reviewed_size_hold() -> TestResult {
    let good = Archive::new(&release_archive())?.bytes()?;
    let selection = selection()?;
    let length = u64::try_from(good.len())?;
    let outcome = |archive_bytes: u64, bounds: ArchiveInventoryBounds| {
        inspect_zip_selected_files(
            Cursor::new(&good),
            archive_bytes,
            bounds,
            &reviewed(&selection),
        )
        .map(|_| ())
    };
    assert_eq!(
        outcome(length - 1, bounds()?),
        Err(ZipInventoryError::ArchiveTooLarge)
    );
    assert_eq!(
        outcome(length + 1, bounds()?),
        Err(ZipInventoryError::Malformed)
    );
    assert_eq!(outcome(0, bounds()?), Err(ZipInventoryError::InvalidLimits));
    assert_eq!(
        outcome(length, ArchiveInventoryBounds::new(4, 4_096)?),
        Err(ZipInventoryError::Inventory(
            ArchiveInventoryError::TooManyEntries
        ))
    );
    assert_eq!(
        outcome(length, ArchiveInventoryBounds::new(8, 100)?),
        Err(ZipInventoryError::Inventory(
            ArchiveInventoryError::TooManyBytes
        ))
    );
    assert_eq!(
        outcome(length, ArchiveInventoryBounds::new(5, 124)?),
        Ok(())
    );
    Ok(())
}

#[test]
fn every_truncation_is_refused_and_no_single_byte_change_panics_or_slips_through() -> TestResult {
    let good = Archive::new(&release_archive())?.bytes()?;
    for length in 0..good.len() {
        assert!(check(&good[..length]).is_err(), "truncated at {length}");
    }
    let selection = selection()?;
    for index in 0..good.len() {
        let mut changed = good.clone();
        changed[index] ^= 0x41;
        // A change the policy cannot see (an unselected file's data, an
        // extra field's contents) may pass; what is staged must still be
        // exactly the reviewed bytes.
        let (outcome, names) = stage(&changed, &reviewed(&selection))?;
        match outcome {
            Ok(()) => assert_eq!(names, ["library.dll", "tool.exe"], "byte {index}"),
            Err(_) => assert_eq!(names, Vec::<String>::new(), "byte {index}"),
        }
    }
    Ok(())
}

/// The shape of the fuzz target's `one-file.zip` seed (`root/tool`, three
/// stored bytes), which the replay test rebuilds and mirrors here.
#[test]
fn the_fuzz_seeds_one_file_shape_is_accepted() -> TestResult {
    let archive = Archive::new(&[Spec::file("root/tool", b"abc").stored()])?.bytes()?;
    let selected = [ReviewedArchiveFile {
        path: "root/tool",
        integrity: integrity(b"abc")?,
    }];
    inspect_zip_selected_files(
        Cursor::new(&archive),
        u64::try_from(archive.len())?,
        bounds()?,
        &selected,
    )?;
    Ok(())
}
