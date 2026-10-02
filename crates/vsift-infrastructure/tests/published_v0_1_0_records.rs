//! The current readers decode the stored records of a 0.1.0 session
//! (P14 PR 2, evidence item RQ-04; ADR 0024).
//!
//! A session's transcript revisions, evidence lineage and visual index are
//! stored as versioned JSON records, and a retained bundle carries them
//! unchanged. The frozen examples `bundle-transcript-record*.json`,
//! `bundle-evidence-record.json` and `bundle-visual-index-record.json` under
//! `schemas/v1/frozen/v0.1.0/examples/` are those records exactly as the tag
//! `v0.1.0` published them (the `vsift-contract` test
//! `published_compatibility` proves the copy is the tag's, and holds the same
//! files to the current schemas). Each must still decode and revalidate with the
//! readers a newer build uses: a session a newer `VSift` opens is read through
//! these functions, so a decoder that lost a version, tightened a rule or
//! started to refuse an old field fails here.
//!
//! What this does not show: the rest of a session (its manifest, its artifacts,
//! the generation counter), or the commands over a session. Those cross an
//! upgrade in the `P14 published artifacts` and `P14 local upgrade` workflows,
//! on real binaries.

use std::{error::Error, fs, path::PathBuf};

use serde_json::Value;
use vsift_domain::SessionId;
use vsift_infrastructure::{
    decode_evidence_record, decode_transcript_record, decode_visual_index_record,
};

type TestResult = Result<(), Box<dyn Error>>;

fn frozen(name: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/frozen/v0.1.0/examples")
            .join(name),
    )?)
}

/// The session a stored record names, which its decoder requires the caller to
/// know.
fn session_of(bytes: &[u8]) -> Result<SessionId, Box<dyn Error>> {
    let record: Value = serde_json::from_slice(bytes)?;
    let identifier = record["session_id"]
        .as_str()
        .ok_or("the record names no session")?;
    Ok(SessionId::parse(identifier)?)
}

#[test]
fn an_imported_transcript_revision_of_0_1_0_decodes() -> TestResult {
    let revision = decode_transcript_record(&frozen("bundle-transcript-record.json")?)?;
    assert!(!revision.segments().is_empty());
    Ok(())
}

#[test]
fn a_local_asr_transcript_revision_of_0_1_0_decodes() -> TestResult {
    let revision = decode_transcript_record(&frozen("bundle-transcript-record.asr.json")?)?;
    assert!(!revision.segments().is_empty());
    Ok(())
}

#[test]
fn the_evidence_record_of_0_1_0_decodes_and_its_identities_still_derive() -> TestResult {
    let bytes = frozen("bundle-evidence-record.json")?;
    let record = decode_evidence_record(&bytes, &session_of(&bytes)?)?;
    assert!(!record.items().is_empty());
    Ok(())
}

#[test]
fn the_visual_index_record_of_0_1_0_decodes() -> TestResult {
    let bytes = frozen("bundle-visual-index-record.json")?;
    let index = decode_visual_index_record(&bytes, &session_of(&bytes)?)?;
    assert!(!index.windows().is_empty());
    Ok(())
}
