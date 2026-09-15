//! Opt-in contained-staging check of the pinned Ubuntu whisper.cpp archive.

mod support;

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
    ArchiveInventoryBounds, ReviewedArchiveAlias, ReviewedArchiveFile,
    stage_gzip_tar_selected_files,
};

use support::PrivateStaging;

const EXPECTED_BYTES: u64 = 9_497_583;
const EXPECTED_SHA256: &str = "46811a3ecf584307480a220b9ef5ff81b7b22dc41577cbc274ce3afc61f753b1";
const ROOT: &str = "whisper-bin-ubuntu-x64";

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

fn reviewed_aliases() -> [ReviewedArchiveAlias<'static>; 8] {
    [
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
    ]
}

fn reviewed_files() -> Result<[ReviewedArchiveFile<'static>; 6], Box<dyn Error>> {
    Ok([
        selected(
            "whisper-bin-ubuntu-x64/LICENSE",
            1_078,
            "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/whisper-cli",
            976_312,
            "61fa94d25ba9a4695118883011f35e8521c158145ec73bcd8805a7c11760e6d7",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml.so.0.18.1",
            54_936,
            "1985fa3dc169a16715a0998da0a075b29be8f68ea2501e3c043be53be7f11857",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-base.so.0.18.1",
            910_680,
            "bc41368cecccc3db8b4f52ad168b51413ee6c005a772b1d3e4f4b3bb47777553",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-x64.so",
            878_024,
            "b7c084e19dc63a83acf9d6dac8d2cba089026996bf805659e10d650d5a51c216",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libwhisper.so.1.9.2",
            611_280,
            "afd9560fa2dd20a7c0f9aa682f9c4f339b2d223f2ad6fa200fc229bc3b1606d6",
        )?,
    ])
}

#[test]
#[ignore = "opt-in pinned publisher archive; set VSIFT_P06_WHISPER_ARCHIVE to its local path"]
fn pinned_upstream_archive_passes_contained_staging() -> Result<(), Box<dyn Error>> {
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

    let aliases = reviewed_aliases();
    let files = reviewed_files()?;
    let staging = PrivateStaging::new("vsift-p06-whisper-stage")?;
    let entries = stage_gzip_tar_selected_files(
        File::open(path)?,
        EXPECTED_BYTES,
        30_000_000,
        ArchiveInventoryBounds::new(44, 24_519_182)?,
        &aliases,
        &files,
        staging.directory()?,
    )?;
    assert_eq!(entries.len(), 44);
    assert!(entries.iter().all(|entry| entry.path.starts_with(ROOT)));
    let mut names = std::fs::read_dir(staging.path())?
        .map(|entry| entry.map(|item| item.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    assert_eq!(
        names,
        [
            "LICENSE",
            "libggml-base.so.0.18.1",
            "libggml-cpu-x64.so",
            "libggml.so.0.18.1",
            "libwhisper.so.1.9.2",
            "whisper-cli",
        ]
    );
    Ok(())
}
