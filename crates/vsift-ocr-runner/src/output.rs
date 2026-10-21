//! The runner's output: one JSON object a line, versioned, bounded.
//!
//! `VSift` reads this protocol as untrusted provider output (ADR 0030): every line
//! is one of the records below, every number is bounded, and a reading always
//! ends with an `image` or an `error` record, so a cut-off stream is visible.

use serde::Serialize;

use crate::pipeline::{FrameLine, FrameReading};

/// The protocol's version, written in the first record.
pub const PROTOCOL: u32 = 1;

/// The first record: who is answering.
#[derive(Debug, Serialize)]
pub struct Hello<'a> {
    /// Always `runner`.
    pub record: &'static str,
    /// [`PROTOCOL`].
    pub protocol: u32,
    /// The runner's own version.
    pub runner_version: &'a str,
}

/// A line of text read off image `image` (its index on the command line).
#[derive(Debug, Serialize)]
pub struct LineRecord<'a> {
    /// Always `line`.
    pub record: &'static str,
    /// The image's index, from 0.
    pub image: usize,
    /// The box's left edge, in the image's pixels.
    pub x: usize,
    /// The box's top edge.
    pub y: usize,
    /// The box's width.
    pub width: usize,
    /// The box's height.
    pub height: usize,
    /// The text read.
    pub text: &'a str,
    /// True when the text was cut at the runner's line length bound.
    pub text_cut: bool,
    /// The recogniser's mean character probability, rounded to four places.
    pub confidence: f64,
}

/// The end of one image's lines.
#[derive(Debug, Serialize)]
pub struct ImageRecord {
    /// Always `image`.
    pub record: &'static str,
    /// The image's index, from 0.
    pub image: usize,
    /// The image's width.
    pub width: usize,
    /// The image's height.
    pub height: usize,
    /// How many `line` records the image gave.
    pub lines: usize,
    /// True when the image held more lines than the bound and the rest were not written.
    pub truncated: bool,
}

/// An image the runner could not read, or a failure that ends the run.
#[derive(Debug, Serialize)]
pub struct ErrorRecord {
    /// Always `error`.
    pub record: &'static str,
    /// The image's index, or none for a failure of the whole run.
    pub image: Option<usize>,
    /// A fixed code: see [`crate::RunnerFailure::code`].
    pub code: &'static str,
}

/// The records of one image's reading, in order: its lines, then its `image` record.
#[must_use]
pub fn reading_records(
    image: usize,
    width: usize,
    height: usize,
    reading: &FrameReading,
) -> Vec<String> {
    let mut records: Vec<String> = reading
        .lines
        .iter()
        .map(|line| line_record(image, line))
        .collect();
    records.push(to_line(&ImageRecord {
        record: "image",
        image,
        width,
        height,
        lines: reading.lines.len(),
        truncated: reading.truncated,
    }));
    records
}

fn line_record(image: usize, line: &FrameLine) -> String {
    to_line(&LineRecord {
        record: "line",
        image,
        x: line.line.rect.x,
        y: line.line.rect.y,
        width: line.line.rect.width,
        height: line.line.rect.height,
        text: &line.line.text,
        text_cut: line.cut,
        confidence: (f64::from(line.line.confidence.clamp(0.0, 1.0)) * 10_000.0).round() / 10_000.0,
    })
}

/// One record as a JSON line without its newline. Serialising these plain records
/// cannot fail; an impossible failure is written as an `error` record, never dropped.
#[must_use]
pub fn to_line(record: &impl Serialize) -> String {
    serde_json::to_string(record).unwrap_or_else(|_| {
        String::from(r#"{"record":"error","image":null,"code":"output_failed"}"#)
    })
}

#[cfg(test)]
mod tests {
    use super::{ErrorRecord, Hello, reading_records, to_line};
    use crate::{
        image::Rect,
        pipeline::{FrameLine, FrameReading},
        recognition::ReadLine,
    };

    #[test]
    fn a_reading_is_its_lines_then_its_image_record() {
        let reading = FrameReading {
            lines: vec![FrameLine {
                line: ReadLine {
                    rect: Rect {
                        x: 4,
                        y: 5,
                        width: 60,
                        height: 12,
                    },
                    text: String::from("Total \"4,731\""),
                    confidence: 0.987_654,
                },
                cut: false,
            }],
            truncated: true,
        };
        let records = reading_records(2, 1280, 720, &reading);
        assert_eq!(
            records,
            vec![
                r#"{"record":"line","image":2,"x":4,"y":5,"width":60,"height":12,"text":"Total \"4,731\"","text_cut":false,"confidence":0.9877}"#.to_owned(),
                r#"{"record":"image","image":2,"width":1280,"height":720,"lines":1,"truncated":true}"#.to_owned(),
            ]
        );
    }

    #[test]
    fn the_hello_and_error_records_are_fixed() {
        assert_eq!(
            to_line(&Hello {
                record: "runner",
                protocol: 1,
                runner_version: "0.1.0"
            }),
            r#"{"record":"runner","protocol":1,"runner_version":"0.1.0"}"#
        );
        assert_eq!(
            to_line(&ErrorRecord {
                record: "error",
                image: Some(0),
                code: "image_invalid"
            }),
            r#"{"record":"error","image":0,"code":"image_invalid"}"#
        );
    }
}
