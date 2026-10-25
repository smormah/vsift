//! The whole reading of one frame: detect, cut out, recognise, decode, filter.
//!
//! Inference itself is behind [`Inference`], so that everything around the
//! models is tested without them, and so that the runtime is replaceable.

use std::fmt;

use crate::{
    detection::{DetectionError, DetectionSettings, detected_boxes, detection_input},
    image::RgbImage,
    recognition::{
        Dictionary, ReadLine, RecognitionError, batches, decode_batch, line_image,
        recognition_input,
    },
    tensor::Tensor,
};

/// Lines below this confidence are dropped, as in the measured pipeline.
pub const MIN_CONFIDENCE: f32 = 0.5;
/// Most characters kept of one line; a longer line is cut and marked.
pub const MAX_LINE_CHARACTERS: usize = 512;

/// The two models a frame passes through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Model {
    /// The text detector.
    Detector,
    /// The text recogniser.
    Recognizer,
}

/// A failure of the inference backend, with a short, fixed description.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InferenceError {
    /// Which model failed.
    pub model: Model,
}

impl fmt::Display for InferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.model {
            Model::Detector => "the text detector failed to run",
            Model::Recognizer => "the text recogniser failed to run",
        })
    }
}

impl std::error::Error for InferenceError {}

/// Runs a model on one input tensor and returns its single output tensor.
pub trait Inference {
    /// Runs `model` on `input`.
    ///
    /// # Errors
    ///
    /// [`InferenceError`] when the runtime cannot run the model on this input.
    fn run(&mut self, model: Model, input: &Tensor) -> Result<Tensor, InferenceError>;
}

/// Why a frame could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    /// Detection failed.
    Detection(DetectionError),
    /// Recognition failed.
    Recognition(RecognitionError),
    /// The runtime failed.
    Inference(InferenceError),
}

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Detection(error) => error.fmt(formatter),
            Self::Recognition(error) => error.fmt(formatter),
            Self::Inference(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ReadError {}

/// A line read off a frame, with whether its text was cut at [`MAX_LINE_CHARACTERS`].
#[derive(Clone, Debug, PartialEq)]
pub struct FrameLine {
    /// The line.
    pub line: ReadLine,
    /// True when the text was longer than [`MAX_LINE_CHARACTERS`] and was cut.
    pub cut: bool,
}

/// What one frame gave: its lines in reading order, and whether more were found than kept.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameReading {
    /// The lines kept, at most the caller's limit.
    pub lines: Vec<FrameLine>,
    /// True when the frame held more lines than the limit.
    pub truncated: bool,
}

/// Reads `frame`: its text lines in reading order, at most `max_lines`.
///
/// # Errors
///
/// [`ReadError`] when a step fails; nothing partial is returned for the frame.
pub fn read_frame(
    inference: &mut impl Inference,
    frame: &RgbImage,
    dictionary: &Dictionary,
    settings: &DetectionSettings,
    max_lines: usize,
) -> Result<FrameReading, ReadError> {
    let (input, scale) = detection_input(frame, settings).map_err(ReadError::Detection)?;
    let map = inference
        .run(Model::Detector, &input)
        .map_err(ReadError::Inference)?;
    let boxes = detected_boxes(&map, &scale, settings).map_err(ReadError::Detection)?;
    let images = boxes
        .iter()
        .map(|detected| line_image(frame, detected))
        .collect::<Result<Vec<_>, _>>()
        .map_err(ReadError::Recognition)?;
    let mut decoded: Vec<Option<(String, f32)>> = vec![None; images.len()];
    for batch in batches(&images) {
        let members: Vec<&RgbImage> = batch
            .iter()
            .filter_map(|index| images.get(*index))
            .collect();
        let tensor = recognition_input(&members).map_err(ReadError::Recognition)?;
        let output = inference
            .run(Model::Recognizer, &tensor)
            .map_err(ReadError::Inference)?;
        for (index, read) in batch
            .iter()
            .zip(decode_batch(&output, members.len(), dictionary).map_err(ReadError::Recognition)?)
        {
            if let Some(slot) = decoded.get_mut(*index) {
                *slot = Some(read);
            }
        }
    }
    let mut lines = Vec::new();
    let mut found = 0_usize;
    for (detected, read) in boxes.iter().zip(decoded) {
        let Some((text, confidence)) = read else {
            continue;
        };
        if confidence < MIN_CONFIDENCE || text.trim().is_empty() {
            continue;
        }
        found += 1;
        if lines.len() == max_lines {
            continue;
        }
        let cut = text.chars().count() > MAX_LINE_CHARACTERS;
        let text = if cut {
            text.chars().take(MAX_LINE_CHARACTERS).collect()
        } else {
            text
        };
        lines.push(FrameLine {
            line: ReadLine {
                rect: detected.rect,
                text,
                confidence,
            },
            cut,
        });
    }
    Ok(FrameReading {
        truncated: found > lines.len(),
        lines,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{FrameReading, Inference, InferenceError, Model, read_frame};
    use crate::{
        detection::DetectionSettings, image::RgbImage, recognition::Dictionary, tensor::Tensor,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A stand-in runtime: a detector that marks fixed blocks of the map as text, and a
    /// recogniser that answers each line in turn from a script.
    pub(crate) struct Scripted {
        pub(crate) blocks: Vec<(usize, usize, usize, usize)>,
        pub(crate) lines: Vec<(Vec<usize>, f32)>,
        pub(crate) next: usize,
        pub(crate) fail: Option<Model>,
    }

    impl Inference for Scripted {
        fn run(&mut self, model: Model, input: &Tensor) -> Result<Tensor, InferenceError> {
            if self.fail == Some(model) {
                return Err(InferenceError { model });
            }
            match model {
                Model::Detector => {
                    let [_, _, height, width] = input.shape() else {
                        return Err(InferenceError { model });
                    };
                    let mut data = vec![0.0_f32; height * width];
                    for &(left, top, right, bottom) in &self.blocks {
                        for y in top..=bottom.min(height - 1) {
                            for x in left..=right.min(width - 1) {
                                if let Some(slot) = data.get_mut(y * width + x) {
                                    *slot = 0.95;
                                }
                            }
                        }
                    }
                    Tensor::new(vec![1, 1, *height, *width], data)
                        .map_err(|_| InferenceError { model })
                }
                Model::Recognizer => {
                    let batch = input.shape().first().copied().unwrap_or(0);
                    let classes = 5;
                    let steps = 4;
                    let mut data = Vec::new();
                    for _ in 0..batch {
                        let (indices, probability) = self
                            .lines
                            .get(self.next)
                            .cloned()
                            .unwrap_or((Vec::new(), 0.0));
                        self.next += 1;
                        for step in 0..steps {
                            let mut row = vec![0.0; classes];
                            let index = indices.get(step).copied().unwrap_or(0);
                            if let Some(slot) = row.get_mut(index) {
                                *slot = if index == 0 { 1.0 } else { probability };
                            }
                            data.extend(row);
                        }
                    }
                    Tensor::new(vec![batch, steps, classes], data)
                        .map_err(|_| InferenceError { model })
                }
            }
        }
    }

    fn frame() -> Result<RgbImage, Box<dyn std::error::Error>> {
        Ok(RgbImage::new(736, 736, vec![200; 736 * 736 * 3])?)
    }

    fn read(
        scripted: &mut Scripted,
        max_lines: usize,
    ) -> Result<FrameReading, Box<dyn std::error::Error>> {
        let dictionary = Dictionary::from_lines("a\nb\nc\n")?;
        Ok(read_frame(
            scripted,
            &frame()?,
            &dictionary,
            &DetectionSettings::PP_OCR_V5,
            max_lines,
        )?)
    }

    #[test]
    fn a_frame_is_read_line_by_line_in_reading_order() -> TestResult {
        let mut scripted = Scripted {
            blocks: vec![(100, 100, 300, 120), (100, 200, 500, 230)],
            lines: vec![(vec![1, 2], 0.9), (vec![3, 3, 0, 3], 0.8)],
            next: 0,
            fail: None,
        };
        let reading = read(&mut scripted, 10)?;
        let texts: Vec<&str> = reading
            .lines
            .iter()
            .map(|line| line.line.text.as_str())
            .collect();
        // The narrower line is read first in its batch, but reading order is kept.
        assert_eq!(texts, vec!["ab", "cc"]);
        assert!(!reading.truncated);
        assert!(reading.lines.iter().all(|line| !line.cut));
        Ok(())
    }

    #[test]
    fn unsure_and_empty_lines_are_dropped_and_a_limit_is_reported() -> TestResult {
        let mut scripted = Scripted {
            blocks: vec![
                (100, 100, 300, 120),
                (100, 200, 500, 230),
                (100, 300, 400, 330),
            ],
            lines: vec![(vec![1], 0.3), (vec![0], 0.0), (vec![2], 0.9)],
            next: 0,
            fail: None,
        };
        let reading = read(&mut scripted, 10)?;
        assert_eq!(reading.lines.len(), 1);
        let mut limited = Scripted {
            blocks: vec![(100, 100, 300, 120), (100, 200, 500, 230)],
            lines: vec![(vec![1], 0.9), (vec![2], 0.9)],
            next: 0,
            fail: None,
        };
        let reading = read(&mut limited, 1)?;
        assert_eq!(reading.lines.len(), 1);
        assert!(reading.truncated);
        Ok(())
    }

    #[test]
    fn a_runtime_failure_fails_the_frame() {
        for model in [Model::Detector, Model::Recognizer] {
            let mut scripted = Scripted {
                blocks: vec![(100, 100, 300, 120)],
                lines: vec![(vec![1], 0.9)],
                next: 0,
                fail: Some(model),
            };
            assert!(read(&mut scripted, 10).is_err());
        }
    }
}
