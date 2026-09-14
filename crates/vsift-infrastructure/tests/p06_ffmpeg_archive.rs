//! Opt-in read-only check of the pinned upstream Ubuntu `FFmpeg` build archive.

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs::File,
    io::{self, Read},
};

use sha2::{Digest, Sha256};
use vsift_infrastructure::{ArchiveInventoryBounds, inspect_xz_tar_inventory};

const EXPECTED_BYTES: u64 = 113_372_924;
const EXPECTED_SHA256: &str = "204fc02692b11249c3e688ad18538ce2939129a1fc6abc32a6b2638a024496cf";
const ROOT: &str = "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0";

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

    let entries = inspect_xz_tar_inventory(
        File::open(path)?,
        EXPECTED_BYTES,
        400_000_000,
        ArchiveInventoryBounds::new(73, 370_667_773)?,
        &[],
    )?;
    assert_eq!(entries.len(), 73);
    assert!(entries.iter().all(|entry| entry.path.starts_with(ROOT)));
    assert_eq!(
        entries.iter().map(|entry| entry.bytes).sum::<u64>(),
        370_667_773
    );
    Ok(())
}
