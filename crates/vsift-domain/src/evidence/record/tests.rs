use super::{
    AudioRange, CropRegion, EvidenceDetail, EvidenceItem, EvidenceItemParts, EvidenceMedia,
    EvidenceMediaKind, EvidenceRecord, EvidenceRecordError, EvidenceRecordParts, EvidenceRequest,
    EvidenceSelection, EvidenceSubject, FrameRef, MAX_AUDIO_CLIP_MICROS, SelectionRole,
    SourceCheck, TimeBase,
};
use crate::{
    BurstCount, BurstExtent, BurstRange, CropRect, EvidenceId, EvidenceProfile, FrameDimensions,
    FrameSelection, FrameTolerance, MediaTime, OperationKey, SessionId, Sha256Hex, SourceId,
    TimeRange,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn frame(pts: i64, time: u64) -> Result<FrameRef, Box<dyn std::error::Error>> {
    Ok(FrameRef {
        stream_index: 0,
        time_base: TimeBase::new(1, 20)?,
        pts,
        time: MediaTime::from_micros(time),
        dimensions: FrameDimensions::new(1280, 720)?,
    })
}

fn png() -> Result<EvidenceMedia, Box<dyn std::error::Error>> {
    Ok(EvidenceMedia::new(
        EvidenceMediaKind::FramePng,
        Sha256Hex::parse(DIGEST)?,
        100,
    )?)
}

fn item(id: &str, subject: EvidenceSubject) -> Result<EvidenceItem, Box<dyn std::error::Error>> {
    let media = match subject.media_kind() {
        EvidenceMediaKind::FramePng => png()?,
        EvidenceMediaKind::AudioWav => {
            EvidenceMedia::new(EvidenceMediaKind::AudioWav, Sha256Hex::parse(DIGEST)?, 46)?
        }
    };
    Ok(EvidenceItem::new(EvidenceItemParts {
        id: EvidenceId::parse(id)?,
        subject,
        media,
        tool_fingerprint: None,
        source_check: SourceCheck::FullHash,
    })?)
}

fn parts(
    request: EvidenceRequest,
    detail: EvidenceDetail,
    selections: Vec<EvidenceSelection>,
    items: Vec<EvidenceItem>,
) -> Result<EvidenceRecordParts, Box<dyn std::error::Error>> {
    Ok(EvidenceRecordParts {
        request_key: OperationKey::from_sha256(DIGEST)?,
        session_id: SessionId::parse("ses_0123456789abcdef")?,
        source_id: SourceId::from_sha256(DIGEST)?,
        profile: EvidenceProfile::P09R0,
        tool_fingerprint: None,
        request,
        selections,
        detail,
        items,
        partial: None,
    })
}

#[test]
fn identifiers_are_stable() {
    assert_eq!(EvidenceProfile::P09R0.identifier(), "p09-r0-v1");
    assert_eq!(
        SourceCheck::ALL.map(SourceCheck::identifier),
        ["identity", "full_hash"]
    );
    assert_eq!(
        SelectionRole::ALL.map(SelectionRole::identifier),
        ["requested", "before", "after", "target"]
    );
    assert_eq!(
        super::PartialReason::ALL.map(super::PartialReason::identifier),
        [
            "frame_budget",
            "pixel_budget",
            "byte_budget",
            "session_evidence_budget",
            "deadline_exceeded",
            "cancelled"
        ]
    );
}

#[test]
fn items_keep_their_media_kind_and_geometry_consistent() -> TestResult {
    let audio = EvidenceMedia::new(EvidenceMediaKind::AudioWav, Sha256Hex::parse(DIGEST)?, 46)?;
    assert_eq!(
        EvidenceItem::new(EvidenceItemParts {
            id: EvidenceId::parse("evd_0123456789abcdef")?,
            subject: EvidenceSubject::Frame(frame(20, 1_000_000)?),
            media: audio,
            tool_fingerprint: None,
            source_check: SourceCheck::Identity,
        }),
        Err(EvidenceRecordError::InvalidItem)
    );
    let dimensions = FrameDimensions::new(1280, 720)?;
    let inside = CropRect::new(1_000, 700, 280, 20, dimensions)?;
    let crop = EvidenceSubject::Crop {
        frame: frame(20, 1_000_000)?,
        region: CropRegion {
            parent: EvidenceId::parse("evd_1111111111111111")?,
            rect: inside,
            frame_rect: inside,
        },
    };
    assert!(item("evd_0123456789abcdef", crop).is_ok());
    let small = FrameDimensions::new(640, 360)?;
    let other = CropRect::new(0, 0, 10, 10, small)?;
    let mismatched = EvidenceSubject::Crop {
        frame: frame(20, 1_000_000)?,
        region: CropRegion {
            parent: EvidenceId::parse("evd_1111111111111111")?,
            rect: CropRect::new(0, 0, 11, 10, small)?,
            frame_rect: other,
        },
    };
    assert_eq!(
        EvidenceItem::new(EvidenceItemParts {
            id: EvidenceId::parse("evd_0123456789abcdef")?,
            subject: mismatched,
            media: png()?,
            tool_fingerprint: None,
            source_check: SourceCheck::FullHash,
        }),
        Err(EvidenceRecordError::InvalidItem)
    );
    assert_eq!(
        AudioRange::new(TimeRange::new(
            MediaTime::from_micros(0),
            MediaTime::from_micros(MAX_AUDIO_CLIP_MICROS + 1)
        )?),
        Err(EvidenceRecordError::AudioRangeTooLong)
    );
    Ok(())
}

#[test]
fn a_record_selects_every_item_once_and_fits_its_operation() -> TestResult {
    let at = MediaTime::from_micros(1_025_000);
    let request = EvidenceRequest::FrameAt {
        at,
        selection: FrameSelection::AtOrAfter,
        tolerance: FrameTolerance::MAX,
        candidate: None,
    };
    let id = EvidenceId::parse("evd_0123456789abcdef")?;
    let chosen = item(id.as_str(), EvidenceSubject::Frame(frame(21, 1_050_000)?))?;
    let selection = EvidenceSelection::new(
        SelectionRole::Requested,
        id.clone(),
        at,
        MediaTime::from_micros(1_050_000),
    )?;
    assert_eq!(selection.timing().delta_micros(), 25_000);
    let record = EvidenceRecord::new(parts(
        request.clone(),
        EvidenceDetail::Single,
        vec![selection.clone()],
        vec![chosen.clone()],
    )?)?;
    assert_eq!(record.item(&id), Some(&chosen));

    let stray = item(
        "evd_1111111111111111",
        EvidenceSubject::Frame(frame(22, 1_100_000)?),
    )?;
    assert_eq!(
        EvidenceRecord::new(parts(
            request.clone(),
            EvidenceDetail::Single,
            vec![selection.clone()],
            vec![chosen.clone(), stray],
        )?),
        Err(EvidenceRecordError::UnselectedItem)
    );
    assert_eq!(
        EvidenceRecord::new(parts(
            request.clone(),
            EvidenceDetail::Single,
            vec![selection.clone()],
            vec![chosen.clone(), chosen.clone()],
        )?),
        Err(EvidenceRecordError::DuplicateItem)
    );
    let wrong_time = EvidenceSelection::new(
        SelectionRole::Requested,
        id.clone(),
        at,
        MediaTime::from_micros(1_100_000),
    )?;
    assert_eq!(
        EvidenceRecord::new(parts(
            request.clone(),
            EvidenceDetail::Single,
            vec![wrong_time],
            vec![chosen.clone()],
        )?),
        Err(EvidenceRecordError::InvalidTiming)
    );
    assert_eq!(
        EvidenceRecord::new(parts(
            request,
            EvidenceDetail::Audio {
                range_clipped: false
            },
            vec![selection],
            vec![chosen],
        )?),
        Err(EvidenceRecordError::OperationMismatch)
    );
    Ok(())
}

#[test]
fn a_burst_record_stays_inside_its_plan() -> TestResult {
    let range = TimeRange::new(MediaTime::from_micros(0), MediaTime::from_micros(2_000_000))?;
    let request = EvidenceRequest::Burst {
        range: BurstRange::new(range)?,
        max_frames: BurstCount::new(2)?,
    };
    let first = EvidenceId::parse("evd_0123456789abcdef")?;
    let chosen = item(first.as_str(), EvidenceSubject::Frame(frame(0, 0)?))?;
    let target = |at: u64| {
        EvidenceSelection::new(
            SelectionRole::Target,
            first.clone(),
            MediaTime::from_micros(at),
            MediaTime::from_micros(0),
        )
    };
    let detail = EvidenceDetail::Burst {
        planned: range,
        extent: BurstExtent::Requested,
        targets: 2,
        distinct: 1,
    };
    assert!(
        EvidenceRecord::new(parts(
            request.clone(),
            detail,
            vec![target(0)?],
            vec![chosen.clone()],
        )?)
        .is_ok()
    );
    assert_eq!(
        EvidenceRecord::new(parts(
            request,
            detail,
            vec![target(2_000_000)?],
            vec![chosen]
        )?),
        Err(EvidenceRecordError::OperationMismatch)
    );
    Ok(())
}
