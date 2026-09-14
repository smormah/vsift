//! Opt-in read-only check of the pinned upstream Ubuntu whisper.cpp archive.

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs::File,
    io::{self, Read},
};

use sha2::{Digest, Sha256};
use vsift_infrastructure::{
    ArchiveInventoryBounds, ReviewedArchiveAlias, inspect_gzip_tar_inventory,
};

const EXPECTED_BYTES: u64 = 9_497_583;
const EXPECTED_SHA256: &str = "46811a3ecf584307480a220b9ef5ff81b7b22dc41577cbc274ce3afc61f753b1";
const ROOT: &str = "whisper-bin-ubuntu-x64";

#[test]
#[ignore = "opt-in pinned publisher archive; set VSIFT_P06_WHISPER_ARCHIVE to its local path"]
fn pinned_upstream_archive_passes_read_only_inventory() -> Result<(), Box<dyn Error>> {
    let path = env::var_os("VSIFT_P06_WHISPER_ARCHIVE")
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

    let aliases = [
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libwhisper.so"),
            target: "libwhisper.so.1",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libwhisper.so.1"),
            target: "libwhisper.so.1.9.2",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libparakeet.so"),
            target: "libparakeet.so.1",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libparakeet.so.1"),
            target: "libparakeet.so.1.9.2",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml.so"),
            target: "libggml.so.0",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml.so.0"),
            target: "libggml.so.0.18.1",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml-base.so"),
            target: "libggml-base.so.0",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml-base.so.0"),
            target: "libggml-base.so.0.18.1",
        },
    ];
    let entries = inspect_gzip_tar_inventory(
        File::open(path)?,
        EXPECTED_BYTES,
        30_000_000,
        ArchiveInventoryBounds::new(44, 24_519_182)?,
        &aliases,
    )?;
    assert_eq!(entries.len(), 44);
    assert!(entries.iter().all(|entry| entry.path.starts_with(ROOT)));
    Ok(())
}
