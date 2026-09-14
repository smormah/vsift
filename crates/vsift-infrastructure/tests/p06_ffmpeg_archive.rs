//! Opt-in read-only check of the pinned upstream Ubuntu `FFmpeg` build archive.

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs::File,
    io::{self, Read},
};

use sha2::{Digest, Sha256};
use vsift_domain::ArtifactIntegrity;
use vsift_infrastructure::{
    ArchiveInventoryBounds, ReviewedArchiveFile, inspect_xz_tar_selected_files,
};

const EXPECTED_BYTES: u64 = 113_372_924;
const EXPECTED_SHA256: &str = "204fc02692b11249c3e688ad18538ce2939129a1fc6abc32a6b2638a024496cf";
const ROOT: &str = "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0";

fn selected(
    path: &'static str,
    bytes: u64,
    sha256: &str,
) -> Result<ReviewedArchiveFile<'static>, Box<dyn Error>> {
    Ok(ReviewedArchiveFile {
        path,
        integrity: ArtifactIntegrity::from_sha256_hex(bytes, sha256)?,
    })
}

#[test]
#[ignore = "opt-in pinned publisher archive; set VSIFT_P06_FFMPEG_ARCHIVE to its local path"]
fn pinned_upstream_archive_passes_read_only_inventory() -> Result<(), Box<dyn Error>> {
    let path = env::var_os("VSIFT_P06_FFMPEG_ARCHIVE")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "candidate archive path missing"))?;
    let metadata = std::fs::metadata(&path)?;
    assert_eq!(metadata.len(), EXPECTED_BYTES);

    let mut hasher = Sha256::new();
    let mut source = File::open(&path)?;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut digest, "{byte:02x}")?;
    }
    assert_eq!(digest, EXPECTED_SHA256);

    let files = [
        selected(
            "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0/LICENSE.txt",
            7_651,
            "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768",
        )?,
        selected(
            "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0/bin/ffmpeg",
            116_038_416,
            "ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc",
        )?,
        selected(
            "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0/bin/ffprobe",
            115_829_520,
            "0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7",
        )?,
    ];
    let entries = inspect_xz_tar_selected_files(
        File::open(path)?,
        EXPECTED_BYTES,
        400_000_000,
        ArchiveInventoryBounds::new(73, 370_667_773)?,
        &[],
        &files,
    )?;
    assert_eq!(entries.len(), 73);
    assert!(entries.iter().all(|entry| entry.path.starts_with(ROOT)));
    assert_eq!(
        entries.iter().map(|entry| entry.bytes).sum::<u64>(),
        370_667_773
    );
    Ok(())
}
