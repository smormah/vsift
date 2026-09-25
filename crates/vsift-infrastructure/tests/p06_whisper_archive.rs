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
use vsift_application::ManagedSetupAction;
use vsift_domain::ArtifactIntegrity;
use vsift_infrastructure::{
    ArchiveInventoryBounds, ManagedArtifactStore, ReviewedArchiveAlias, ReviewedArchiveFile,
    ReviewedUbuntuAction, accepted_ubuntu_catalogue, stage_gzip_tar_selected_files,
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

fn reviewed_files() -> Result<[ReviewedArchiveFile<'static>; 19], Box<dyn Error>> {
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
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-alderlake.so",
            1_013_360,
            "53633ffd2a2c668ae08a283b38821a88c61523de2e407096a0c1b6a1cad1d41e",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-cannonlake.so",
            1_074_912,
            "b28b1ac4709a87c8868921b30eae141aaf8f4d8fb620b584d453cfef5eb698f0",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-cascadelake.so",
            1_070_816,
            "643f51af219a74b86178992eabbcb683d9f37a38b81d1e9ad63605874331477c",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-cooperlake.so",
            1_070_816,
            "e02b7d041ddbf19a9f1a0a0f61d54e6d8bb6072b6ee09c001e2dd0b4c3b74398",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-haswell.so",
            1_017_456,
            "4e6e0cfe5f94806bf38e3f75d8128df6c65ab8c8c9cf279e392bd9c0b6d5c1f7",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-icelake.so",
            1_070_816,
            "89248d02d6aa6f06f64a654139f9c1e7cedf4215407f49c8930af98db2df8bf3",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-ivybridge.so",
            951_704,
            "c7b66231986e403482de7ef7474b59523ef5968f1c923f12519a30fdc4567741",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-piledriver.so",
            951_704,
            "5e0e0bc4c3ab21bc4ff43386d1f683c9b07f9a7ce0f6786b0dab7a19570991d5",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-sandybridge.so",
            959_896,
            "fccf45830c2929fe8261064c987fc94da85afdb7fc57cfaae52132cbbeda825c",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-sapphirerapids.so",
            1_337_176,
            "fb4f9e70b2d83220165afad2073be0ea36ad8e91829f84753c7e9bf7e0b38a54",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-skylakex.so",
            1_074_912,
            "089070b665ba61e8c34f61c9f6d7cf86df5d71847522ecb434d23afb16d466fb",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-sse42.so",
            881_976,
            "f0d5d2cf7dbfd3081be59b6ba97a34399c4954a2f8b1ba8d80d9894545225f8e",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-zen4.so",
            1_070_816,
            "d17c9e99888b523b1699603b9453594fd47d1e2e686c77dff32d125f7b0c6c36",
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
        File::open(&path)?,
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
            "libggml-cpu-alderlake.so",
            "libggml-cpu-cannonlake.so",
            "libggml-cpu-cascadelake.so",
            "libggml-cpu-cooperlake.so",
            "libggml-cpu-haswell.so",
            "libggml-cpu-icelake.so",
            "libggml-cpu-ivybridge.so",
            "libggml-cpu-piledriver.so",
            "libggml-cpu-sandybridge.so",
            "libggml-cpu-sapphirerapids.so",
            "libggml-cpu-skylakex.so",
            "libggml-cpu-sse42.so",
            "libggml-cpu-x64.so",
            "libggml-cpu-zen4.so",
            "libggml.so.0.18.1",
            "libwhisper.so.1.9.2",
            "whisper-cli",
        ]
    );
    let owned = ManagedArtifactStore::at(staging.path().join("managed"))?;
    let artifact = owned.import_verified(
        File::open(&path)?,
        ArtifactIntegrity::from_sha256_hex(EXPECTED_BYTES, EXPECTED_SHA256)?,
    )?;
    let accepted = accepted_ubuntu_catalogue()?
        .artifacts
        .into_iter()
        .find(|entry| entry.component.identifier() == "whisper_cli")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "whisper review missing"))?;
    let reviewed = ReviewedUbuntuAction::from_accepted_action(&ManagedSetupAction {
        id: String::from("install-whisper_cli"),
        artifact: accepted,
    })?;
    let payload = reviewed.stage_payload(&artifact)?;
    assert_eq!(
        payload.open_selected_file("whisper-cli")?.metadata()?.len(),
        976_312
    );
    let runtime = reviewed.prepare_runtime(&payload)?;
    assert_eq!(runtime.reviewed_names().len(), 25);
    runtime.recheck_all()?;
    assert_eq!(
        runtime.open_reviewed_file("libggml.so")?.metadata()?.len(),
        54_936
    );
    runtime.discard()?;
    payload.discard()?;
    artifact.discard()?;
    Ok(())
}
