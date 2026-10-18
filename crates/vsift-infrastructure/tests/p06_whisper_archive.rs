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

const EXPECTED_BYTES: u64 = 10_364_195;
const EXPECTED_SHA256: &str = "a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c";
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
            target: "libwhisper.so.1.9.5",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libparakeet.so"),
            target: "libparakeet.so.1",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libparakeet.so.1"),
            target: "libparakeet.so.1.9.5",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml.so"),
            target: "libggml.so.0",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml.so.0"),
            target: "libggml.so.0.26.0",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml-base.so"),
            target: "libggml-base.so.0",
        },
        ReviewedArchiveAlias {
            path: concat!("whisper-bin-ubuntu-x64", "/libggml-base.so.0"),
            target: "libggml-base.so.0.26.0",
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
            "110a858e128d1fbc5a75899b42a1de8a7a4c10856bfe45dad1adc278f6459d14",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml.so.0.26.0",
            55_184,
            "25406a47ddc7a6b45adb4995ad5efbc6a7ef6efbc43e64a506d978d11dd6fec8",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-base.so.0.26.0",
            931_232,
            "8cd57970d5ba71529442758c0b42cb820ed319e03be0691aead74ef298d3f51f",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-x64.so",
            942_168,
            "c19a80e868a34bdb197e2901c8f785a77640d6e56055134514b5fa2807171af5",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libwhisper.so.1.9.5",
            615_800,
            "1517f17d5188f7ef70cbb66a2b8ce258c510d20c25018d18e2801351de13cd55",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-alderlake.so",
            1_280_320,
            "d33aa9c10d716534dff77655da446f09516ee4583b695e8cc1aa447e57848dac",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-cannonlake.so",
            1_362_352,
            "0cb6e59a34c9798a512398e820942ee18ce99c1e0b308ec593e78655759eaafe",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-cascadelake.so",
            1_374_344,
            "a0286798b10092eabaf5f81ce9ae664aa78d3c609ed2138fa8a7ca22da73ef7d",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-cooperlake.so",
            1_374_344,
            "4169f4b9c975e46ddc386b695f64fb3a38a3dacd40bbb15465a2a5dbcabb9a1f",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-haswell.so",
            1_284_416,
            "a63377026e388671d7925e729fced52d9dd85d1bca780dce97bc2c74817053d4",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-icelake.so",
            1_374_344,
            "3f28f5a44dd3640cf179d4f678ea9bdacc536ad69b6b5fb5efd155b6b91c49c8",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-ivybridge.so",
            1_213_104,
            "8b22efecdde26d41d3d864f2d5f6e29c31e0951d6453465d1f4fbda50b9eaa4b",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-piledriver.so",
            1_209_008,
            "e04b72d641d4ae129c05a2ca167dae4843eb0ecae5d40a31a5d98ae480641b11",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-sandybridge.so",
            1_217_200,
            "806218b034614ad31dbe4fbae48bcc0282c966bbe9f0085380d1c49b6b0f7d0b",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-sapphirerapids.so",
            1_636_576,
            "d190abfe2ad900b24da89c883f633dc621f69ba12b2f5664659826374c1f4cfb",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-skylakex.so",
            1_362_352,
            "66d8e2a22ef39225eb2d80e3a1dd9c8a7dd2be41966760c27f22e79623611dd5",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-sse42.so",
            950_184,
            "58e6f62fa6bc2732c5a8b6323377ae62ba6cc1ebad9dfd298b6548819bd94375",
        )?,
        selected(
            "whisper-bin-ubuntu-x64/libggml-cpu-zen4.so",
            1_374_344,
            "42bb25dff93816eb12327a1b82c04ae436dd17eebf30da556143ec9acffd9582",
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
        ArchiveInventoryBounds::new(47, 28_128_870)?,
        &aliases,
        &files,
        staging.directory()?,
    )?;
    assert_eq!(entries.len(), 47);
    assert!(entries.iter().all(|entry| entry.path.starts_with(ROOT)));
    let mut names = std::fs::read_dir(staging.path())?
        .map(|entry| entry.map(|item| item.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    assert_eq!(
        names,
        [
            "LICENSE",
            "libggml-base.so.0.26.0",
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
            "libggml.so.0.26.0",
            "libwhisper.so.1.9.5",
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
        55_184
    );
    runtime.discard()?;
    payload.discard()?;
    artifact.discard()?;
    Ok(())
}
