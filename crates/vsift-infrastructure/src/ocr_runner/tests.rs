use vsift_application::EngineReading;

use super::{
    MAX_RUNNER_OUTPUT_BYTES, RunnerOutputError, parse_runner_description, parse_runner_output,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const HELLO: &str = r#"{"record":"runner","protocol":1,"runner_version":"0.1.0"}"#;

fn stream(records: &[&str]) -> Vec<u8> {
    let mut text = String::from(HELLO);
    for record in records {
        text.push('\n');
        text.push_str(record);
    }
    text.push('\n');
    text.into_bytes()
}

const LINE: &str = r#"{"record":"line","image":0,"x":4,"y":5,"width":60,"height":12,"text":"Total","text_cut":false,"confidence":0.9877}"#;
const SIZE: (u32, u32) = (1280, 720);
const IMAGE_ONE: &str =
    r#"{"record":"image","image":0,"width":1280,"height":720,"lines":1,"truncated":false}"#;

#[test]
fn a_run_gives_each_image_its_lines_or_its_refusal() -> TestResult {
    let output = stream(&[
        LINE,
        IMAGE_ONE,
        r#"{"record":"error","image":1,"code":"image_invalid"}"#,
        r#"{"record":"image","image":2,"width":1280,"height":720,"lines":0,"truncated":true}"#,
        r#"{"record":"error","image":3,"code":"inference_failed"}"#,
    ]);
    let readings = parse_runner_output(&output, &[SIZE; 4])?;
    let Some(EngineReading::Lines {
        lines,
        truncated: false,
    }) = readings.first()
    else {
        return Err("the first image's lines".into());
    };
    assert_eq!(
        lines
            .first()
            .map(|line| (line.text.as_str(), line.confidence_basis_points)),
        Some(("Total", 9_877))
    );
    assert_eq!(readings.get(1), Some(&EngineReading::Rejected));
    assert_eq!(
        readings.get(2),
        Some(&EngineReading::Lines {
            lines: Vec::new(),
            truncated: true
        })
    );
    assert_eq!(readings.get(3), Some(&EngineReading::Failed));
    Ok(())
}

#[test]
fn every_departure_from_the_protocol_is_refused() {
    let wrong_protocol =
        String::from(r#"{"record":"runner","protocol":2,"runner_version":"0.1.0"}"#).into_bytes();
    let cases: Vec<(Vec<u8>, usize, RunnerOutputError)> = vec![
        (
            stream(&[
                r#"{"record":"image","image":0,"width":640,"height":720,"lines":0,"truncated":false}"#,
            ]),
            1,
            RunnerOutputError::Inconsistent,
        ),
        (Vec::new(), 1, RunnerOutputError::Protocol),
        (wrong_protocol, 0, RunnerOutputError::Protocol),
        (stream(&[IMAGE_ONE]), 1, RunnerOutputError::Inconsistent),
        (stream(&[LINE]), 1, RunnerOutputError::Inconsistent),
        (
            stream(&[LINE, IMAGE_ONE]),
            2,
            RunnerOutputError::Inconsistent,
        ),
        (
            stream(&[LINE, IMAGE_ONE, IMAGE_ONE]),
            1,
            RunnerOutputError::Inconsistent,
        ),
        (
            stream(&[
                r#"{"record":"line","image":0,"x":4,"y":5,"width":60,"height":12,"text":"x","text_cut":false,"confidence":1.5}"#,
            ]),
            1,
            RunnerOutputError::OverBound,
        ),
        (
            stream(&[
                r#"{"record":"line","image":0,"x":4,"y":5,"width":60,"height":12,"text":"x","text_cut":false,"confidence":0.5,"extra":1}"#,
            ]),
            1,
            RunnerOutputError::Malformed,
        ),
        (
            stream(&[r#"{"record":"error","image":null,"code":"model_unavailable"}"#]),
            1,
            RunnerOutputError::Inconsistent,
        ),
        (
            stream(&[r#"{"record":"error","image":0,"code":"something"}"#]),
            1,
            RunnerOutputError::Malformed,
        ),
        (stream(&["not json"]), 1, RunnerOutputError::Malformed),
        (vec![0xff, 0xfe], 1, RunnerOutputError::Malformed),
        (
            vec![b' '; MAX_RUNNER_OUTPUT_BYTES + 1],
            1,
            RunnerOutputError::TooLarge,
        ),
    ];
    for (bytes, images, expected) in cases {
        assert_eq!(
            parse_runner_output(&bytes, &vec![SIZE; images]),
            Err(expected),
            "{}",
            String::from_utf8_lossy(bytes.get(..bytes.len().min(160)).unwrap_or_default())
        );
    }
}

#[test]
fn a_line_cut_by_the_runner_marks_its_reading_truncated() -> TestResult {
    let cut = LINE.replace("\"text_cut\":false", "\"text_cut\":true");
    let readings = parse_runner_output(&stream(&[&cut, IMAGE_ONE]), &[SIZE])?;
    assert!(matches!(
        readings.first(),
        Some(EngineReading::Lines {
            truncated: true,
            ..
        })
    ));
    Ok(())
}

#[test]
fn more_lines_than_a_frame_holds_are_refused() {
    let line = r#"{"record":"line","image":0,"x":1,"y":1,"width":2,"height":2,"text":"a","text_cut":false,"confidence":0.9}"#;
    let records: Vec<&str> =
        std::iter::repeat_n(line, vsift_domain::MAX_SCREEN_TEXT_LINES_PER_FRAME + 1).collect();
    assert_eq!(
        parse_runner_output(&stream(&records), &[SIZE]),
        Err(RunnerOutputError::OverBound)
    );
}

#[test]
fn the_description_names_the_runtime_profile_and_models() -> TestResult {
    let good = format!(
        r#"{{"record":"describe","protocol":1,"runner_version":"0.1.0","runtime_version":"1.31.0","profile":"ppocrv5-latin-v1","detector_sha256":"{}","recognizer_sha256":"{}"}}"#,
        "a".repeat(64),
        "b".repeat(64)
    );
    let description = parse_runner_description(good.as_bytes())?;
    assert_eq!(description.runtime_version.as_str(), "1.31.0");
    assert!(
        parse_runner_description(good.replace("ppocrv5-latin-v1", "other").as_bytes()).is_err()
    );
    assert!(
        parse_runner_description(good.replace("\"protocol\":1", "\"protocol\":9").as_bytes())
            .is_err()
    );
    assert!(parse_runner_description(good.replace(&"a".repeat(64), "A").as_bytes()).is_err());
    Ok(())
}

/// The recorded runs under `tests/data/ocr_runner/` (also the fuzz seeds of
/// `ocr_runner_output`) parse: four images, the third's line cut by the runner.
#[test]
fn the_recorded_runs_parse() -> TestResult {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/ocr_runner");
    let readings =
        parse_runner_output(&std::fs::read(data.join("four-images.jsonl"))?, &[SIZE; 4])?;
    assert_eq!(readings.len(), 4);
    assert!(matches!(
        readings.first(),
        Some(EngineReading::Lines { lines, truncated: false }) if lines.len() == 2
    ));
    assert_eq!(readings.get(1), Some(&EngineReading::Rejected));
    assert!(matches!(
        readings.get(2),
        Some(EngineReading::Lines {
            truncated: true,
            ..
        })
    ));
    assert_eq!(readings.get(3), Some(&EngineReading::Failed));
    let description = parse_runner_description(&std::fs::read(data.join("describe.json"))?)?;
    assert_eq!(description.runtime_version.as_str(), "1.31.0");
    Ok(())
}
