//! Opt-in owned staging check of the pinned Ubuntu multilingual base model.

mod support;

use std::{env, error::Error, fs::File, io};

use vsift_application::ManagedSetupAction;
use vsift_domain::ManagedComponent;
use vsift_infrastructure::{ManagedArtifactStore, ReviewedUbuntuAction, accepted_ubuntu_catalogue};

use support::PrivateStaging;

#[test]
#[ignore = "opt-in pinned publisher model; set VSIFT_P06_BASE_MODEL to its local path"]
fn pinned_model_passes_owned_raw_payload_and_runtime_recheck() -> Result<(), Box<dyn Error>> {
    let path = env::var_os("VSIFT_P06_BASE_MODEL")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "candidate model path missing"))?;
    let accepted = accepted_ubuntu_catalogue()?
        .artifacts
        .into_iter()
        .find(|entry| entry.component == ManagedComponent::WhisperModel)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "model review missing"))?;
    let reviewed = ReviewedUbuntuAction::from_accepted_action(&ManagedSetupAction {
        id: String::from("install-whisper_model"),
        artifact: accepted,
    })?;
    let staging = PrivateStaging::new("vsift-p06-model-stage")?;
    let store = ManagedArtifactStore::at(staging.path().join("managed"))?;
    let artifact =
        store.import_verified(File::open(path)?, reviewed.publisher_source().integrity())?;
    assert!(staging.directory()?.try_exists("managed")?);
    let payload = reviewed.stage_payload(&artifact)?;
    let runtime = reviewed.prepare_runtime(&payload)?;
    assert_eq!(runtime.reviewed_names(), ["ggml-base.bin"]);
    runtime.recheck_all()?;
    assert_eq!(
        runtime
            .open_reviewed_file("ggml-base.bin")?
            .metadata()?
            .len(),
        reviewed.publisher_source().integrity().bytes()
    );
    runtime.discard()?;
    payload.discard()?;
    artifact.discard()?;
    Ok(())
}
