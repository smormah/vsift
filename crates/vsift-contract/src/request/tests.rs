//! Decoding, validation and digest of `job-request` v1.

use vsift_domain::FailureCode;

use super::{
    BatchLine, BundleName, MAX_REQUEST_STEPS, RequestDeadline, RequestDurability, RequestRejection,
    WORK_REQUEST_LIMITS, WorkStep, WorkStepKind, WorkTarget, decode_batch_line,
    decode_work_request,
};
use crate::BundleSourceInclusion;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const INGEST: &str = r#"{"schema_version":"1","operation_id":"op_0123456789abcdef","durability":"ephemeral","deadline_ms":600000,"target":{"ingest":{"source":"clips/F01.mp4","transcript":{"path":"clips/F01.srt","offset_us":-500000}}},"steps":[{"retranscribe":{"range":{"from_us":0,"to_us":6000000}}},{"candidates":{"range":null}},{"retain":{"bundle_name":"f01-review","include_source":false}},{"close":{}}]}"#;

fn with(replacement: (&str, &str)) -> String {
    INGEST.replacen(replacement.0, replacement.1, 1)
}

fn rejection(text: &str) -> Option<RequestRejection> {
    decode_work_request(text.as_bytes()).err()
}

#[test]
fn a_complete_request_decodes_to_typed_values() -> TestResult {
    let request = decode_work_request(INGEST.as_bytes())?;
    assert_eq!(request.operation_id().as_str(), "op_0123456789abcdef");
    assert_eq!(request.durability(), RequestDurability::Ephemeral);
    assert_eq!(
        request.deadline().map(RequestDeadline::as_millis),
        Some(600_000)
    );
    let WorkTarget::Ingest { source, transcript } = request.target() else {
        return Err("not an ingest target".into());
    };
    assert_eq!(source.as_str(), "clips/F01.mp4");
    let transcript = transcript.as_ref().ok_or("no transcript")?;
    assert_eq!(transcript.path().as_str(), "clips/F01.srt");
    assert_eq!(transcript.offset().as_micros(), -500_000);
    let kinds: Vec<_> = request.steps().iter().map(WorkStep::kind).collect();
    assert_eq!(
        kinds,
        [
            WorkStepKind::Retranscribe,
            WorkStepKind::Candidates,
            WorkStepKind::Retain,
            WorkStepKind::Close
        ]
    );
    assert!(matches!(
        &request.steps()[2],
        WorkStep::Retain { bundle, source: BundleSourceInclusion::EvidenceOnly }
            if bundle.as_str() == "f01-review"
    ));
    assert_eq!(request.digest().as_str().len(), 64);
    Ok(())
}

#[test]
fn a_session_target_decodes() -> TestResult {
    let request = decode_work_request(
        br#"{"schema_version":"1","operation_id":"op_0123456789abcdef","durability":"durable","target":{"session_id":"ses_0123456789abcdef"},"steps":[{"close":{}}]}"#,
    )?;
    assert!(
        matches!(request.target(), WorkTarget::Session(session) if session.as_str() == "ses_0123456789abcdef")
    );
    assert_eq!(request.durability(), RequestDurability::Durable);
    assert_eq!(request.deadline(), None);
    Ok(())
}

/// Whitespace, member order and an omitted or null deadline do not change
/// the digest; the operation id is not part of it; anything requested is.
#[test]
fn the_digest_is_canonical() -> TestResult {
    let compact = decode_work_request(INGEST.as_bytes())?;
    let spaced = INGEST.replace(',', " ,\n\t").replace(':', " : ");
    let spaced = decode_work_request(spaced.as_bytes())?;
    assert_eq!(compact.digest(), spaced.digest());

    let reordered = r#"{"steps":[{"retranscribe":{"range":{"to_us":6000000,"from_us":0}}},{"candidates":{"range":null}},{"retain":{"include_source":false,"bundle_name":"f01-review"}},{"close":{}}],"target":{"ingest":{"transcript":{"offset_us":-500000,"path":"clips/F01.srt"},"source":"clips/F01.mp4"}},"deadline_ms":600000,"durability":"ephemeral","operation_id":"op_fedcba9876543210","schema_version":"1"}"#;
    assert_eq!(
        decode_work_request(reordered.as_bytes())?.digest(),
        compact.digest()
    );

    let omitted = with((r#""deadline_ms":600000,"#, ""));
    let null = with((r#""deadline_ms":600000"#, r#""deadline_ms":null"#));
    assert_eq!(
        decode_work_request(omitted.as_bytes())?.digest(),
        decode_work_request(null.as_bytes())?.digest()
    );

    for changed in [
        with(("600000", "600001")),
        with(("ephemeral", "durable")),
        with(("F01.mp4", "F02.mp4")),
        with(("-500000", "500000")),
        with(("6000000", "6000001")),
        with(("f01-review", "f01-other")),
        with(("false", "true")),
        with((r#",{"close":{}}"#, "")),
        omitted,
    ] {
        assert_ne!(
            decode_work_request(changed.as_bytes())?.digest(),
            compact.digest(),
            "{changed}"
        );
    }
    Ok(())
}

#[test]
fn a_newer_major_is_unsupported_even_with_new_members() {
    for text in [
        with((r#""schema_version":"1""#, r#""schema_version":"2""#)),
        r#"{"schema_version":"2","future_member":{"x":1}}"#.to_owned(),
    ] {
        let rejection = rejection(&text);
        assert_eq!(rejection, Some(RequestRejection::UnsupportedSchemaVersion));
        assert_eq!(
            rejection.map(RequestRejection::failure_code),
            Some(FailureCode::UnsupportedSchema)
        );
    }
}

#[test]
fn strict_shape_violations_are_malformed() {
    for text in [
        String::new(),
        "[]".to_owned(),
        "not json".to_owned(),
        with((r#""durability":"ephemeral","#, "")),
        with((r#""operation_id""#, r#""extra":1,"operation_id""#)),
        with((r#""durability":"ephemeral""#, r#""durability":"sometimes""#)),
        with((
            r#","transcript":{"path":"clips/F01.srt","offset_us":-500000}"#,
            "",
        )),
        with((r#"{"range":null}"#, "{}")),
        with((r#"{"close":{}}"#, r#"{"close":{"now":true}}"#)),
        with((r#"{"close":{}}"#, r#"{"close":{},"retain":{}}"#)),
        with((r#"{"close":{}}"#, r#"{"search":{}}"#)),
        with((r#""target":{"ingest""#, r#""target":{"path""#)),
        with((r#""schema_version":"1""#, r#""schema_version":1"#)),
        with((r#""deadline_ms":600000"#, r#""deadline_ms":-1"#)),
        with((r#""from_us":0"#, r#""from_us":0.5"#)),
        format!("{INGEST}{INGEST}"),
    ] {
        assert_eq!(
            rejection(&text),
            Some(RequestRejection::Malformed),
            "{text}"
        );
    }
}

#[test]
fn every_value_is_validated() {
    for (text, expected) in [
        (
            with(("op_0123456789abcdef", "op_short")),
            RequestRejection::InvalidOperationId,
        ),
        (
            with(("op_0123456789abcdef", "../../etc")),
            RequestRejection::InvalidOperationId,
        ),
        (
            with(("clips/F01.mp4", "/etc/passwd")),
            RequestRejection::InvalidPath,
        ),
        (
            with(("clips/F01.mp4", "../F01.mp4")),
            RequestRejection::InvalidPath,
        ),
        (
            with(("clips/F01.srt", r"C:\\F01.srt")),
            RequestRejection::InvalidPath,
        ),
        (
            with(("-500000", "-86400000001")),
            RequestRejection::InvalidTranscriptOffset,
        ),
        (with(("600000", "0")), RequestRejection::InvalidDeadline),
        (
            with(("600000", "86400001")),
            RequestRejection::InvalidDeadline,
        ),
        (
            with((r#""to_us":6000000"#, r#""to_us":0"#)),
            RequestRejection::InvalidRange,
        ),
        (
            with(("f01-review", "F01")),
            RequestRejection::InvalidBundleName,
        ),
        (
            with(("f01-review", "-f01")),
            RequestRejection::InvalidBundleName,
        ),
        (
            with(("f01-review", "")),
            RequestRejection::InvalidBundleName,
        ),
        (
            with(("f01-review", &"a".repeat(65))),
            RequestRejection::InvalidBundleName,
        ),
    ] {
        assert_eq!(rejection(&text), Some(expected), "{text}");
    }
    assert_eq!(
        rejection(
            r#"{"schema_version":"1","operation_id":"op_0123456789abcdef","durability":"durable","target":{"session_id":"session-7"},"steps":[{"close":{}}]}"#
        ),
        Some(RequestRejection::InvalidSessionId)
    );
    assert!(BundleName::parse(&"a".repeat(64)).is_ok());
}

#[test]
fn the_step_order_is_enforced() {
    let session = |steps: &str| {
        format!(
            r#"{{"schema_version":"1","operation_id":"op_0123456789abcdef","durability":"ephemeral","target":{{"session_id":"ses_0123456789abcdef"}},"steps":[{steps}]}}"#
        )
    };
    let retain = r#"{"retain":{"bundle_name":"b","include_source":true}}"#;
    let close = r#"{"close":{}}"#;
    let candidates = r#"{"candidates":{"range":null}}"#;
    for (steps, expected) in [
        (
            format!("{close},{candidates}"),
            Some(RequestRejection::StepOrder),
        ),
        (
            format!("{close},{close}"),
            Some(RequestRejection::StepOrder),
        ),
        (
            format!("{retain},{retain}"),
            Some(RequestRejection::StepOrder),
        ),
        (
            format!("{retain},{candidates}"),
            Some(RequestRejection::StepOrder),
        ),
        (
            format!("{close},{retain}"),
            Some(RequestRejection::StepOrder),
        ),
        (String::new(), Some(RequestRejection::NoWork)),
        (
            [candidates; MAX_REQUEST_STEPS + 1].join(","),
            Some(RequestRejection::TooManySteps),
        ),
        ([candidates; MAX_REQUEST_STEPS].join(","), None),
        (format!("{candidates},{candidates},{retain},{close}"), None),
        (format!("{retain},{close}"), None),
        (close.to_owned(), None),
    ] {
        assert_eq!(rejection(&session(&steps)), expected, "{steps}");
    }
    // An ingest alone is work: it opens a session.
    assert_eq!(
        rejection(&INGEST.replace(
            r#"[{"retranscribe":{"range":{"from_us":0,"to_us":6000000}}},{"candidates":{"range":null}},{"retain":{"bundle_name":"f01-review","include_source":false}},{"close":{}}]"#,
            "[]"
        )),
        None
    );
}

#[test]
fn budgets_hold_before_anything_is_parsed() {
    let padded = format!("{INGEST}{}", " ".repeat(WORK_REQUEST_LIMITS.max_bytes));
    assert_eq!(rejection(&padded), Some(RequestRejection::TooLarge));
    let deep = format!(
        "{}{}",
        "[".repeat(WORK_REQUEST_LIMITS.max_nesting + 1),
        "]".repeat(WORK_REQUEST_LIMITS.max_nesting + 1)
    );
    assert_eq!(rejection(&deep), Some(RequestRejection::TooDeep));
    let at_limit = format!(
        "{INGEST}{}",
        " ".repeat(WORK_REQUEST_LIMITS.max_bytes - INGEST.len())
    );
    assert_eq!(rejection(&at_limit), None);
}

#[test]
fn batch_lines_decode_blank_and_carriage_return_lines() -> TestResult {
    assert_eq!(decode_batch_line(b"")?, BatchLine::Blank);
    assert_eq!(decode_batch_line(b" \t\r")?, BatchLine::Blank);
    let line = format!("{INGEST}\r");
    assert!(matches!(
        decode_batch_line(line.as_bytes())?,
        BatchLine::Request(request) if request.operation_id().as_str() == "op_0123456789abcdef"
    ));
    let two = format!("{INGEST}\n{INGEST}");
    assert_eq!(
        decode_batch_line(two.as_bytes()),
        Err(RequestRejection::Malformed)
    );
    Ok(())
}

#[test]
fn every_rejection_has_an_identifier_code_and_remediation() {
    let mut identifiers = std::collections::BTreeSet::new();
    for rejection in RequestRejection::ALL {
        assert!(identifiers.insert(rejection.identifier()));
        assert!(
            rejection
                .identifier()
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        );
        let remediation = rejection.remediation();
        assert!(!remediation.is_empty() && remediation.len() <= 1_024);
        assert!(matches!(
            rejection.failure_code(),
            FailureCode::InvalidArgument | FailureCode::UnsupportedSchema
        ));
    }
}
