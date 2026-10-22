//! Text recognition: each detected box cut out, scaled to the recogniser's
//! height, read in batches, and its per-step character probabilities decoded.
//!
//! The recogniser (`PP-OCR`'s CTC model) answers, for each horizontal step of a
//! line image, a probability for every character of its dictionary and for
//! "blank". The text is the most probable character at each step, with repeats
//! and blanks removed; its confidence is the mean probability of the characters
//! kept.

use std::fmt;

use crate::{
    detection::{DetectedBox, bgr_tensor},
    image::{Rect, RgbImage, ceil_index, index_to_f32, ratio},
    tensor::Tensor,
};

/// The recogniser's input height in pixels.
pub const LINE_HEIGHT: usize = 48;
/// The narrowest input width; a batch is as wide as its widest line needs.
pub const MIN_LINE_WIDTH: usize = 320;
/// Lines read in one inference call: one, so a line's reading never depends on
/// the other lines of its frame. In a batch every line is padded to the widest
/// one's width, and that padding changes what the recogniser writes; the hosted
/// cross-check showed the reference pipeline's own readings change with it
/// (`P16 OCR runner`, run 39616094342; L-204).
pub const BATCH_SIZE: usize = 1;
/// The widest line image accepted, in pixels after scaling to [`LINE_HEIGHT`].
pub const MAX_LINE_WIDTH: usize = 48 * 128;

/// Why recognition could not run or its output could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecognitionError {
    /// A box does not fit the frame it came from.
    InvalidBox,
    /// The recogniser's output is not a probability table of the expected shape.
    InvalidOutput,
}

impl fmt::Display for RecognitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBox => "a detected box does not fit the frame",
            Self::InvalidOutput => {
                "the text recogniser's output is not a probability table of the expected shape"
            }
        })
    }
}

impl std::error::Error for RecognitionError {}

/// The recogniser's dictionary: index 0 is "blank", then the model's characters, then a space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dictionary {
    characters: Vec<String>,
}

/// A dictionary that is empty or holds a character that is not one character.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DictionaryError;

impl fmt::Display for DictionaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the recogniser's character list is empty or malformed")
    }
}

impl std::error::Error for DictionaryError {}

impl Dictionary {
    /// Most characters a dictionary may hold.
    pub const MAX_CHARACTERS: usize = 32_768;

    /// The dictionary of a model whose character list is `listed`, one entry a line.
    ///
    /// # Errors
    ///
    /// [`DictionaryError`] when the list is empty, too long, or has an entry that is
    /// not exactly one character (a line ending is not part of an entry).
    pub fn from_lines(listed: &str) -> Result<Self, DictionaryError> {
        let mut characters = vec![String::new()];
        for entry in listed.split('\n') {
            let entry = entry.strip_suffix('\r').unwrap_or(entry);
            if entry.is_empty() {
                continue;
            }
            if entry.chars().count() != 1 || entry.chars().any(char::is_control) {
                return Err(DictionaryError);
            }
            characters.push(entry.to_owned());
        }
        if characters.len() < 2 || characters.len() > Self::MAX_CHARACTERS {
            return Err(DictionaryError);
        }
        characters.push(String::from(" "));
        Ok(Self { characters })
    }

    /// Classes the recogniser must answer: blank, the characters and the space.
    #[must_use]
    pub fn classes(&self) -> usize {
        self.characters.len()
    }

    /// Greedy CTC decoding of one line's steps (`steps` x [`Self::classes`] probabilities).
    #[must_use]
    pub fn decode(&self, steps: &[f32]) -> (String, f32) {
        let classes = self.classes();
        let mut text = String::new();
        let mut confidences = Vec::new();
        let mut previous = None;
        for step in steps.chunks_exact(classes) {
            let (best, probability) = step.iter().enumerate().fold(
                (0_usize, f32::MIN),
                |(best, high), (index, value)| {
                    if *value > high {
                        (index, *value)
                    } else {
                        (best, high)
                    }
                },
            );
            if best != 0
                && previous != Some(best)
                && let Some(character) = self.characters.get(best)
            {
                text.push_str(character);
                confidences.push(probability);
            }
            previous = Some(best);
        }
        let confidence = if confidences.is_empty() {
            0.0
        } else {
            confidences.iter().sum::<f32>() / index_to_f32(confidences.len())
        };
        (text, confidence)
    }
}

/// A line of text the recogniser read inside a box.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadLine {
    /// The box on the frame.
    pub rect: Rect,
    /// The text, as decoded.
    pub text: String,
    /// The mean probability of its characters (0 to 1).
    pub confidence: f32,
}

/// The line image of one box: cut from the frame, turned upright when much taller
/// than wide, as the measured pipeline did.
///
/// # Errors
///
/// [`RecognitionError::InvalidBox`] when the box does not fit the frame.
pub fn line_image(frame: &RgbImage, detected: &DetectedBox) -> Result<RgbImage, RecognitionError> {
    let crop = frame
        .cropped(detected.rect)
        .map_err(|_| RecognitionError::InvalidBox)?;
    if index_to_f32(crop.height()) >= 1.5 * index_to_f32(crop.width()) {
        Ok(crop.rotated_anticlockwise())
    } else {
        Ok(crop)
    }
}

/// The batches to read: indices into `lines`, narrowest first, at most [`BATCH_SIZE`] each.
#[must_use]
pub fn batches(lines: &[RgbImage]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|left, right| {
        let aspect = |index: &usize| {
            lines
                .get(*index)
                .map_or(0.0, |line| ratio(line.width(), line.height()))
        };
        aspect(left).total_cmp(&aspect(right))
    });
    order.chunks(BATCH_SIZE).map(<[usize]>::to_vec).collect()
}

/// The recogniser's input for one batch: each line scaled to [`LINE_HEIGHT`] and
/// padded to the batch's width.
///
/// # Errors
///
/// [`RecognitionError::InvalidBox`] when a line is empty or wider than [`MAX_LINE_WIDTH`] once scaled.
pub fn recognition_input(lines: &[&RgbImage]) -> Result<Tensor, RecognitionError> {
    let min_aspect = ratio(MIN_LINE_WIDTH, LINE_HEIGHT);
    let widest = lines
        .iter()
        .map(|line| ratio(line.width(), line.height()))
        .fold(min_aspect, f32::max);
    let width = crate::image::floor_index(index_to_f32(LINE_HEIGHT) * widest);
    if width > MAX_LINE_WIDTH {
        return Err(RecognitionError::InvalidBox);
    }
    let mut scaled = Vec::with_capacity(lines.len());
    for line in lines {
        let wanted = ceil_index(index_to_f32(LINE_HEIGHT) * ratio(line.width(), line.height()));
        let resized = line
            .resized(wanted.clamp(1, width), LINE_HEIGHT)
            .map_err(|_| RecognitionError::InvalidBox)?;
        scaled.push(resized);
    }
    let refs: Vec<&RgbImage> = scaled.iter().collect();
    bgr_tensor(&refs, width).map_err(|_| RecognitionError::InvalidBox)
}

/// Decodes the recogniser's output for a batch of `count` lines.
///
/// # Errors
///
/// [`RecognitionError::InvalidOutput`] unless the output is `[count, steps, classes]`
/// of finite values.
pub fn decode_batch(
    output: &Tensor,
    count: usize,
    dictionary: &Dictionary,
) -> Result<Vec<(String, f32)>, RecognitionError> {
    let [batch, steps, classes] = output.shape() else {
        return Err(RecognitionError::InvalidOutput);
    };
    if *batch != count
        || *classes != dictionary.classes()
        || *steps == 0
        || output.data().iter().any(|value| !value.is_finite())
    {
        return Err(RecognitionError::InvalidOutput);
    }
    Ok(output
        .data()
        .chunks_exact(steps * classes)
        .map(|line| dictionary.decode(line))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{
        BATCH_SIZE, Dictionary, LINE_HEIGHT, batches, decode_batch, line_image, recognition_input,
    };
    use crate::{
        detection::DetectedBox,
        image::{Rect, RgbImage},
        tensor::Tensor,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn one_hot(dictionary: &Dictionary, indices: &[usize], probability: f32) -> Vec<f32> {
        let mut steps = Vec::new();
        for index in indices {
            let mut step = vec![0.0; dictionary.classes()];
            if let Some(slot) = step.get_mut(*index) {
                *slot = probability;
            }
            steps.extend(step);
        }
        steps
    }

    #[test]
    fn decoding_drops_blanks_and_repeats_and_averages_what_it_keeps() -> TestResult {
        let dictionary = Dictionary::from_lines("a\nb\nç\n")?;
        // blank=0, a=1, b=2, ç=3, space=4
        assert_eq!(dictionary.classes(), 5);
        let (text, confidence) =
            dictionary.decode(&one_hot(&dictionary, &[1, 1, 0, 1, 4, 3, 3, 2, 0], 0.8));
        assert_eq!(text, "aa çb");
        assert!((confidence - 0.8).abs() < 1e-6);
        assert_eq!(
            dictionary.decode(&one_hot(&dictionary, &[0, 0], 0.9)),
            (String::new(), 0.0)
        );
        Ok(())
    }

    #[test]
    fn a_malformed_dictionary_is_refused() {
        assert!(Dictionary::from_lines("").is_err());
        assert!(Dictionary::from_lines("ab\n").is_err());
        assert!(Dictionary::from_lines("a\n\u{7}\n").is_err());
        assert!(Dictionary::from_lines("a\r\nb\r\n").is_ok());
    }

    #[test]
    fn a_tall_box_is_turned_upright() -> TestResult {
        let frame = RgbImage::new(20, 40, vec![9; 20 * 40 * 3])?;
        let tall = line_image(
            &frame,
            &DetectedBox {
                rect: Rect {
                    x: 2,
                    y: 2,
                    width: 10,
                    height: 30,
                },
                score: 0.9,
            },
        )?;
        assert_eq!((tall.width(), tall.height()), (30, 10));
        let wide = line_image(
            &frame,
            &DetectedBox {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 20,
                    height: 10,
                },
                score: 0.9,
            },
        )?;
        assert_eq!((wide.width(), wide.height()), (20, 10));
        assert!(
            line_image(
                &frame,
                &DetectedBox {
                    rect: Rect {
                        x: 15,
                        y: 0,
                        width: 10,
                        height: 10
                    },
                    score: 0.9
                }
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn a_batch_is_as_wide_as_its_widest_line_and_never_narrower_than_320() -> TestResult {
        let short = RgbImage::new(24, 12, vec![0; 24 * 12 * 3])?;
        let long = RgbImage::new(400, 20, vec![0; 400 * 20 * 3])?;
        assert_eq!(
            recognition_input(&[&short])?.shape(),
            &[1, 3, LINE_HEIGHT, 320]
        );
        assert_eq!(
            recognition_input(&[&short, &long])?.shape(),
            &[2, 3, LINE_HEIGHT, 960]
        );
        let absurd = RgbImage::new(20_000, 4, vec![0; 20_000 * 4 * 3])?;
        assert!(recognition_input(&[&absurd]).is_err());
        Ok(())
    }

    #[test]
    fn lines_are_batched_narrowest_first() -> TestResult {
        let lines: Vec<RgbImage> = [50, 10, 30, 20, 60, 40, 70, 15]
            .iter()
            .map(|width| RgbImage::new(*width, 10, vec![0; width * 10 * 3]))
            .collect::<Result<_, _>>()?;
        let grouped = batches(&lines);
        assert_eq!(grouped.len(), lines.len().div_ceil(BATCH_SIZE));
        assert!(grouped.iter().all(|batch| batch.len() <= BATCH_SIZE));
        assert_eq!(
            grouped.first().and_then(|batch| batch.first()).copied(),
            Some(1)
        );
        assert_eq!(
            grouped.last().and_then(|batch| batch.last()).copied(),
            Some(6)
        );
        let order: Vec<usize> = grouped.into_iter().flatten().collect();
        assert_eq!(order, vec![1, 7, 3, 2, 5, 0, 4, 6]);
        Ok(())
    }

    #[test]
    fn a_batch_output_of_the_wrong_shape_is_refused() -> TestResult {
        let dictionary = Dictionary::from_lines("a\nb\n")?;
        let good = Tensor::new(
            vec![2, 2, 4],
            [
                one_hot(&dictionary, &[1, 2], 0.9),
                one_hot(&dictionary, &[2, 2], 0.7),
            ]
            .concat(),
        )?;
        let read = decode_batch(&good, 2, &dictionary)?;
        assert_eq!(
            read.iter()
                .map(|(text, _)| text.as_str())
                .collect::<Vec<_>>(),
            vec!["ab", "b"]
        );
        assert!(decode_batch(&good, 3, &dictionary).is_err());
        let wrong_classes = Tensor::new(vec![1, 2, 5], vec![0.1; 10])?;
        assert!(decode_batch(&wrong_classes, 1, &dictionary).is_err());
        let infinite = Tensor::new(vec![1, 1, 4], vec![f32::INFINITY, 0.0, 0.0, 0.0])?;
        assert!(decode_batch(&infinite, 1, &dictionary).is_err());
        Ok(())
    }
}
