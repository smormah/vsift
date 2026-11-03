//! Opt-in contained-staging check of the pinned Ubuntu `FFmpeg` build archive.

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
    ArchiveInventoryBounds, ManagedArtifactStore, ReviewedArchiveFile, ReviewedUbuntuAction,
    accepted_ubuntu_catalogue, stage_xz_tar_selected_files,
};

use support::PrivateStaging;

const EXPECTED_BYTES: u64 = 136_027_168;
const EXPECTED_SHA256: &str = "6046d6731b26f58887e953561551d254a6eec9fbca4100dfeab56e19af8328d5";
const ROOT: &str = "ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-lgpl-9.0";

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
fn pinned_upstream_archive_passes_contained_staging() -> Result<(), Box<dyn Error>> {
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
            "ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-lgpl-9.0/LICENSE.txt",
            7_651,
            "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768",
        )?,
        selected(
            "ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-lgpl-9.0/bin/ffmpeg",
            140_488_520,
            "d372f20463a8ffb9eef1675242cdbf9aae19b647503b073fc9b5b1f42b91c94a",
        )?,
        selected(
            "ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-lgpl-9.0/bin/ffprobe",
            140_259_080,
            "9a7f735d3df4f56dd8406cd7a03a227686bec55b1edc496c25bd8b1867e44498",
        )?,
    ];
    let staging = PrivateStaging::new("vsift-p06-ffmpeg-stage")?;
    let entries = stage_xz_tar_selected_files(
        File::open(&path)?,
        EXPECTED_BYTES,
        450_000_000,
        ArchiveInventoryBounds::new(73, 444_067_173)?,
        &[],
        &files,
        staging.directory()?,
    )?;
    assert_eq!(entries.len(), 73);
    assert!(entries.iter().all(|entry| entry.path.starts_with(ROOT)));
    assert_eq!(
        entries.iter().map(|entry| entry.bytes).sum::<u64>(),
        444_067_173
    );
    let mut names = std::fs::read_dir(staging.path())?
        .map(|entry| entry.map(|item| item.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    assert_eq!(names, ["LICENSE.txt", "ffmpeg", "ffprobe"]);

    let owned = ManagedArtifactStore::at(staging.path().join("managed"))?;
    let artifact = owned.import_verified(
        File::open(&path)?,
        ArtifactIntegrity::from_sha256_hex(EXPECTED_BYTES, EXPECTED_SHA256)?,
    )?;
    let accepted = accepted_ubuntu_catalogue()?
        .artifacts
        .into_iter()
        .find(|entry| entry.component.identifier() == "ffmpeg_ffprobe")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "media review missing"))?;
    let reviewed = ReviewedUbuntuAction::from_accepted_action(&ManagedSetupAction {
        id: String::from("install-ffmpeg_ffprobe"),
        artifact: accepted,
    })?;
    let payload = reviewed.stage_payload(&artifact)?;
    let runtime = reviewed.prepare_runtime(&payload)?;
    assert_eq!(
        runtime.reviewed_names(),
        ["LICENSE.txt", "ffmpeg", "ffprobe"]
    );
    runtime.recheck_all()?;
    runtime.discard()?;
    payload.discard()?;
    artifact.discard()?;
    Ok(())
}
