//! The finite request file of `job batch` (P11 PR 4, ADR 0021 section 5):
//! opened once, checked to the end through the same handle, then read one
//! bounded line at a time.
//!
//! **Pre-scan.** Before any request runs, the whole file is read once with a
//! fixed buffer to count its lines: a file of more than the batch's line
//! limit is refused before anything starts, so a supervisor never has to
//! guess which of its lines ran. The count holds no line in memory, and the
//! scan stops as soon as the limit is passed.
//!
//! **Streaming.** The reader then seeks back to the start and hands out one
//! line per call, holding at most the line bound plus one byte of it: a
//! longer line is consumed to its line feed without being kept and is
//! reported as [`BatchLine::TooLong`], so its caller can refuse that line
//! alone. The caller asks for the next line only when it can start it
//! (backpressure, X-08), so the file is never held in memory. A file that
//! grows between the scan and the read is still held to the line limit.
//!
//! A line is the bytes up to a line feed; a final line without one counts,
//! an empty file has no line. Only a regular file is read: a pipe or device
//! could block the scan for ever or could not be read twice.

use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

/// Bytes read at a time by the scan and the line reader.
const READ_BUFFER_BYTES: usize = 8 * 1024;

/// Why a batch file could not be read, or read further.
#[derive(Debug)]
pub enum BatchFileError {
    /// The file could not be opened, is not a regular file, or a read or
    /// seek failed. The error kind only; never the path.
    Unreadable(io::ErrorKind),
    /// The file holds more lines than the limit.
    TooManyLines,
}

impl std::fmt::Display for BatchFileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(kind) => {
                write!(formatter, "the batch file could not be read ({kind})")
            }
            Self::TooManyLines => formatter.write_str("the batch file holds too many lines"),
        }
    }
}

impl std::error::Error for BatchFileError {}

impl From<io::Error> for BatchFileError {
    fn from(error: io::Error) -> Self {
        Self::Unreadable(error.kind())
    }
}

/// One line handed out by [`BatchLines::next_line`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchLine {
    /// A line within the bound, without its line feed.
    Line {
        /// Its 1-based number in the file.
        number: u32,
        /// Its bytes, at most the line bound.
        bytes: Vec<u8>,
    },
    /// A line longer than the bound: consumed, not kept.
    TooLong {
        /// Its 1-based number in the file.
        number: u32,
    },
}

impl BatchLine {
    /// The line's 1-based number in the file.
    #[must_use]
    pub const fn number(&self) -> u32 {
        match self {
            Self::Line { number, .. } | Self::TooLong { number } => *number,
        }
    }
}

/// The lines of a batch source, counted once and then read one at a time.
///
/// Generic over the source so the same code reads a file and, in tests and
/// fuzzing, a buffer.
#[derive(Debug)]
pub struct BatchLines<Source> {
    reader: BufReader<Source>,
    lines: u32,
    max_lines: u32,
    max_line_bytes: usize,
    read: u32,
    ended: bool,
}

/// A batch file opened for reading.
pub type BatchFile = BatchLines<File>;

impl BatchLines<File> {
    /// Opens the regular file at `path` and counts its lines (see the module
    /// documentation).
    ///
    /// # Errors
    ///
    /// [`BatchFileError::Unreadable`] when the file cannot be opened, is not
    /// a regular file, or cannot be read to its end;
    /// [`BatchFileError::TooManyLines`] beyond `max_lines`.
    pub fn open(
        path: &Path,
        max_lines: u32,
        max_line_bytes: usize,
    ) -> Result<Self, BatchFileError> {
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(BatchFileError::Unreadable(io::ErrorKind::InvalidInput));
        }
        Self::scan(file, max_lines, max_line_bytes)
    }
}

impl<Source: Read + Seek> BatchLines<Source> {
    /// Counts the lines of `source` from its start, then rewinds it.
    ///
    /// # Errors
    ///
    /// As [`BatchLines::open`].
    pub fn scan(
        mut source: Source,
        max_lines: u32,
        max_line_bytes: usize,
    ) -> Result<Self, BatchFileError> {
        source.seek(SeekFrom::Start(0))?;
        let mut buffer = [0_u8; READ_BUFFER_BYTES];
        let mut lines: u32 = 0;
        let mut open_line = false;
        loop {
            let read = match source.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            for byte in buffer.iter().take(read) {
                if !open_line {
                    lines = lines.saturating_add(1);
                    if lines > max_lines {
                        return Err(BatchFileError::TooManyLines);
                    }
                }
                open_line = *byte != b'\n';
            }
        }
        source.seek(SeekFrom::Start(0))?;
        Ok(Self {
            reader: BufReader::with_capacity(READ_BUFFER_BYTES, source),
            lines,
            max_lines,
            max_line_bytes,
            read: 0,
            ended: false,
        })
    }

    /// How many lines the scan counted.
    #[must_use]
    pub const fn lines(&self) -> u32 {
        self.lines
    }

    /// How many lines were handed out so far.
    #[must_use]
    pub const fn lines_read(&self) -> u32 {
        self.read
    }

    /// The next line, or `None` at the end of the source.
    ///
    /// # Errors
    ///
    /// [`BatchFileError::Unreadable`] when a read fails;
    /// [`BatchFileError::TooManyLines`] when the source grew past the limit
    /// since it was scanned. Either way nothing more is read.
    pub fn next_line(&mut self) -> Result<Option<BatchLine>, BatchFileError> {
        if self.ended {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        let mut too_long = false;
        let mut consumed_any = false;
        loop {
            let available = match self.reader.fill_buf() {
                Ok(available) => available,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.ended = true;
                    return Err(error.into());
                }
            };
            if available.is_empty() {
                self.ended = true;
                if !consumed_any {
                    return Ok(None);
                }
                break;
            }
            consumed_any = true;
            let (take, found_end) = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or((available.len(), false), |end| (end, true));
            if !too_long {
                let room = self.max_line_bytes.saturating_sub(bytes.len());
                if take > room {
                    too_long = true;
                    bytes = Vec::new();
                } else if let Some(part) = available.get(..take) {
                    bytes.extend_from_slice(part);
                }
            }
            let used = if found_end { take + 1 } else { take };
            self.reader.consume(used);
            if found_end {
                break;
            }
        }
        self.read = self.read.saturating_add(1);
        if self.read > self.max_lines {
            self.ended = true;
            return Err(BatchFileError::TooManyLines);
        }
        let number = self.read;
        Ok(Some(if too_long {
            BatchLine::TooLong { number }
        } else {
            BatchLine::Line { number, bytes }
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{BatchFileError, BatchLine, BatchLines};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn all(
        source: &[u8],
        max_lines: u32,
        max_bytes: usize,
    ) -> Result<Vec<BatchLine>, BatchFileError> {
        let mut lines = BatchLines::scan(Cursor::new(source.to_vec()), max_lines, max_bytes)?;
        let mut out = Vec::new();
        while let Some(line) = lines.next_line()? {
            out.push(line);
        }
        Ok(out)
    }

    fn line(number: u32, bytes: &[u8]) -> BatchLine {
        BatchLine::Line {
            number,
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn lines_are_counted_then_read_in_order() -> TestResult {
        let source = b"a\n\nbc\r\nd";
        let scanned = BatchLines::scan(Cursor::new(source.to_vec()), 10, 8)?;
        assert_eq!(scanned.lines(), 4);
        assert_eq!(
            all(source, 10, 8)?,
            [line(1, b"a"), line(2, b""), line(3, b"bc\r"), line(4, b"d")]
        );
        assert_eq!(all(b"", 10, 8)?, []);
        assert_eq!(all(b"\n", 10, 8)?, [line(1, b"")]);
        assert_eq!(all(b"x\n", 10, 8)?, [line(1, b"x")]);
        Ok(())
    }

    #[test]
    fn a_file_over_the_line_limit_is_refused_before_any_line() {
        assert!(matches!(
            BatchLines::scan(Cursor::new(b"1\n2\n3\n".to_vec()), 2, 8),
            Err(BatchFileError::TooManyLines)
        ));
        assert!(BatchLines::scan(Cursor::new(b"1\n2\n".to_vec()), 2, 8).is_ok());
        assert!(matches!(
            BatchLines::scan(Cursor::new(b"1\n2\n3".to_vec()), 2, 8),
            Err(BatchFileError::TooManyLines)
        ));
    }

    /// A long line is consumed without being kept; the next line is intact,
    /// whatever the read buffer's boundaries.
    #[test]
    fn a_line_over_the_bound_is_reported_alone() -> TestResult {
        let long = vec![b'x'; 20_000];
        let mut source = b"ok\n".to_vec();
        source.extend_from_slice(&long);
        source.extend_from_slice(b"\nnext\n");
        assert_eq!(
            all(&source, 10, 16_384)?,
            [
                line(1, b"ok"),
                BatchLine::TooLong { number: 2 },
                line(3, b"next")
            ]
        );
        // Exactly at the bound is a line; one byte more is not.
        assert_eq!(all(b"12345678\n", 10, 8)?, [line(1, b"12345678")]);
        assert_eq!(
            all(b"123456789\nz", 10, 8)?,
            [BatchLine::TooLong { number: 1 }, line(2, b"z")]
        );
        Ok(())
    }

    #[test]
    fn a_source_that_grew_is_still_held_to_the_limit() -> TestResult {
        let mut lines = BatchLines::scan(Cursor::new(b"a\n".to_vec()), 1, 8)?;
        // The scan saw one line; replace the source with a longer one, as a
        // writer appending after the scan would.
        let mut grown = BatchLines::scan(Cursor::new(b"a\n".to_vec()), 1, 8)?;
        grown.reader = std::io::BufReader::new(Cursor::new(b"a\nb\n".to_vec()));
        assert_eq!(grown.next_line()?, Some(line(1, b"a")));
        assert!(matches!(
            grown.next_line(),
            Err(BatchFileError::TooManyLines)
        ));
        assert_eq!(grown.next_line()?, None);
        assert_eq!(lines.next_line()?, Some(line(1, b"a")));
        assert_eq!(lines.next_line()?, None);
        assert_eq!(lines.lines_read(), 1);
        Ok(())
    }
}
