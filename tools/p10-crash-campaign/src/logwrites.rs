//! Reading and replaying a dm-log-writes log (layer A, power loss at every
//! flush).
//!
//! The kernel's `log-writes` target (`drivers/md/dm-log-writes.c`, format
//! version 1) records every write to the logged device, in completion
//! order, on a separate log device. A write that completed is logged only
//! when a later flush or FUA write reaches the device, just before that
//! flush's own entry, and a mark (`dmsetup message <dev> 0 mark <text>`) is
//! logged at once. The log therefore lists writes in the order they became
//! durable, and replaying its first `n` entries onto a copy of the device as
//! it was when logging began reproduces the device after a power loss just
//! after entry `n`.
//!
//! Layout, in the logged device's logical sectors (`sectorsize` bytes):
//!
//! - sector 0: the super block (`magic`, `version`, `nr_entries`,
//!   `sectorsize`, little endian);
//! - then, for each entry, one sector holding its header (`sector`,
//!   `nr_sectors`, `flags`, `data_len`), followed by `nr_sectors` sectors of
//!   written data (none for flushes, discards and marks). A mark's text is
//!   stored inside its header sector, after the header.

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
};

/// The log's magic number (`WRITE_LOG_MAGIC`).
pub const LOG_MAGIC: u64 = 0x006a_7366_7773_6872;
/// The only log format version the kernel writes.
pub const LOG_VERSION: u64 = 1;
/// A flush (`REQ_PREFLUSH`).
pub const FLUSH: u64 = 1;
/// A forced-unit-access write (`REQ_FUA`).
pub const FUA: u64 = 1 << 1;
/// A discard.
pub const DISCARD: u64 = 1 << 2;
/// A mark.
pub const MARK: u64 = 1 << 3;
/// Most entries the reader accepts: far above any campaign run.
pub const MAX_ENTRIES: u64 = 50_000_000;
const HEADER_BYTES: u64 = 32;

/// Why a log was refused.
#[derive(Debug)]
pub enum LogError {
    /// The log could not be read.
    Io(io::Error),
    /// The super block's magic or version is not the kernel's.
    NotALog,
    /// The sector size is not a power of two from 512 to 4096.
    SectorSize(u32),
    /// More entries than [`MAX_ENTRIES`].
    TooManyEntries(u64),
    /// Entry `index` runs past the end of the log or is inconsistent.
    Entry(u64),
}

impl fmt::Display for LogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "reading the log: {error}"),
            Self::NotALog => formatter.write_str("not a dm-log-writes version 1 log"),
            Self::SectorSize(size) => write!(formatter, "unsupported sector size {size}"),
            Self::TooManyEntries(count) => write!(formatter, "{count} entries is too many"),
            Self::Entry(index) => write!(formatter, "entry {index} is malformed"),
        }
    }
}

impl Error for LogError {}

impl From<io::Error> for LogError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// One logged entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    /// First sector written (logical sectors of the logged device).
    pub sector: u64,
    /// Sectors written or discarded.
    pub sectors: u64,
    /// `FLUSH`, `FUA`, `DISCARD` and `MARK` bits.
    pub flags: u64,
    /// Byte offset of the written data in the log.
    pub data_offset: u64,
    /// A mark's text.
    pub mark: Option<String>,
}

impl Entry {
    /// Whether the entry is a durability point: a flush or a FUA write.
    #[must_use]
    pub const fn is_durability_point(&self) -> bool {
        self.flags & (FLUSH | FUA) != 0
    }
}

/// A parsed log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteLog {
    /// The logged device's logical sector size in bytes.
    pub sector_size: u64,
    /// Every entry, in log order.
    pub entries: Vec<Entry>,
}

fn le_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let slice = bytes.get(at..at + 8)?;
    let mut array = [0_u8; 8];
    array.copy_from_slice(slice);
    Some(u64::from_le_bytes(array))
}

impl WriteLog {
    /// Parses the log in `source`, whose length is `length` bytes.
    ///
    /// # Errors
    ///
    /// [`LogError`] for an unreadable, foreign or inconsistent log.
    pub fn read(source: &mut (impl Read + Seek), length: u64) -> Result<Self, LogError> {
        // magic, version, nr_entries (u64 each) and sectorsize (u32).
        let mut super_block = [0_u8; 28];
        source.seek(SeekFrom::Start(0))?;
        source.read_exact(&mut super_block)?;
        let magic = le_u64(&super_block, 0).ok_or(LogError::NotALog)?;
        let version = le_u64(&super_block, 8).ok_or(LogError::NotALog)?;
        let count = le_u64(&super_block, 16).ok_or(LogError::NotALog)?;
        let mut size = [0_u8; 4];
        size.copy_from_slice(super_block.get(24..28).ok_or(LogError::NotALog)?);
        let sector_size = u32::from_le_bytes(size);
        if magic != LOG_MAGIC || version != LOG_VERSION {
            return Err(LogError::NotALog);
        }
        if !(512..=4096).contains(&sector_size) || !sector_size.is_power_of_two() {
            return Err(LogError::SectorSize(sector_size));
        }
        if count > MAX_ENTRIES {
            return Err(LogError::TooManyEntries(count));
        }
        let sector = u64::from(sector_size);
        let mut header =
            vec![0_u8; usize::try_from(sector).map_err(|_| LogError::SectorSize(sector_size))?];
        let mut entries = Vec::with_capacity(usize::try_from(count.min(1 << 20)).unwrap_or(0));
        let mut position = sector;
        for index in 0..count {
            if position.checked_add(sector).is_none_or(|end| end > length) {
                return Err(LogError::Entry(index));
            }
            source.seek(SeekFrom::Start(position))?;
            source.read_exact(&mut header)?;
            let (Some(first), Some(sectors), Some(flags), Some(data_len)) = (
                le_u64(&header, 0),
                le_u64(&header, 8),
                le_u64(&header, 16),
                le_u64(&header, 24),
            ) else {
                return Err(LogError::Entry(index));
            };
            let mark = if flags & MARK == 0 {
                // Inline data belongs to DAX devices, which the campaign
                // never logs.
                if data_len != 0 {
                    return Err(LogError::Entry(index));
                }
                None
            } else {
                let end = HEADER_BYTES
                    .checked_add(data_len)
                    .filter(|end| *end <= sector && sectors == 0)
                    .ok_or(LogError::Entry(index))?;
                let text = header
                    .get(
                        usize::try_from(HEADER_BYTES).unwrap_or(0)
                            ..usize::try_from(end).unwrap_or(0),
                    )
                    .ok_or(LogError::Entry(index))?;
                let text = text.split(|byte| *byte == 0).next().unwrap_or_default();
                Some(String::from_utf8_lossy(text).into_owned())
            };
            let data_bytes = if flags & (DISCARD | MARK) == 0 {
                sectors.checked_mul(sector).ok_or(LogError::Entry(index))?
            } else {
                0
            };
            let data_offset = position + sector;
            position = data_offset
                .checked_add(data_bytes)
                .filter(|end| *end <= length)
                .ok_or(LogError::Entry(index))?;
            entries.push(Entry {
                sector: first,
                sectors,
                flags,
                data_offset,
                mark,
            });
        }
        Ok(Self {
            sector_size: sector,
            entries,
        })
    }

    /// Writes entries `range` onto `target`, the device image being
    /// rebuilt: written data at its sector, zeros over a discard.
    ///
    /// # Errors
    ///
    /// I/O failures and entries whose target offset overflows.
    pub fn apply(
        &self,
        log: &mut File,
        target: &mut File,
        range: std::ops::Range<usize>,
    ) -> Result<(), LogError> {
        let mut buffer = Vec::new();
        for (index, entry) in self
            .entries
            .iter()
            .enumerate()
            .take(range.end)
            .skip(range.start)
        {
            let bad = || LogError::Entry(u64::try_from(index).unwrap_or(u64::MAX));
            if entry.flags & MARK != 0 || entry.sectors == 0 {
                continue;
            }
            let bytes = entry
                .sectors
                .checked_mul(self.sector_size)
                .ok_or_else(bad)?;
            let offset = entry.sector.checked_mul(self.sector_size).ok_or_else(bad)?;
            buffer.resize(usize::try_from(bytes).map_err(|_| bad())?, 0);
            if entry.flags & DISCARD == 0 {
                log.seek(SeekFrom::Start(entry.data_offset))?;
                log.read_exact(&mut buffer)?;
            } else {
                buffer.fill(0);
            }
            target.seek(SeekFrom::Start(offset))?;
            target.write_all(&buffer)?;
        }
        Ok(())
    }
}

/// One replay point: the device after a power loss just after `entry`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    /// Entries replayed: every entry before this index.
    pub replayed: usize,
    /// How many acknowledgements (in mark order) must hold here.
    pub required: usize,
    /// The highest command whose start mark is among the replayed entries
    /// (managed workload only; `None` in a log without start marks). The
    /// first acknowledgement beyond `required` is in flight here exactly
    /// when its command started at or before this one.
    pub started: Option<u64>,
}

/// The replay points of `log`: after every flush and FUA entry, and after
/// the whole log.
///
/// An acknowledgement's mark is logged when it was made; the device then
/// held exactly the entries logged before it. So an acknowledgement whose
/// mark lies before the next durability point after `p` was made while the
/// device held no more than `p`'s prefix: it must hold at `p` and at every
/// later point. `acks` are the acknowledgement sequence numbers in the order
/// the workload made them.
///
/// A command's start mark (managed workload) is logged before it writes, so
/// a point whose prefix holds the start mark of the next acknowledged
/// command may hold part of that command too: [`Point::started`] says so.
///
/// # Errors
///
/// The sequence number of an acknowledgement whose mark is not in the log.
pub fn plan(log: &WriteLog, acks: &[u64]) -> Result<Vec<Point>, u64> {
    let mut mark_positions = Vec::with_capacity(acks.len());
    let mut marks = std::collections::BTreeMap::new();
    let mut starts: Vec<(usize, u64)> = Vec::new();
    for (index, entry) in log.entries.iter().enumerate() {
        let Some(mark) = entry.mark.as_deref() else {
            continue;
        };
        if let Some(seq) = crate::protocol::seq_of_mark(mark) {
            marks.insert(seq, index);
        } else if let Some(seq) = crate::protocol::seq_of_start_mark(mark) {
            starts.push((index, seq));
        }
    }
    for seq in acks {
        mark_positions.push(*marks.get(seq).ok_or(*seq)?);
    }
    // The device as logging began (nothing replayed) is the first point.
    let mut ends: Vec<usize> = std::iter::once(0)
        .chain(
            log.entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.is_durability_point())
                .map(|(index, _)| index + 1),
        )
        .collect();
    if ends.last() != Some(&log.entries.len()) {
        ends.push(log.entries.len());
    }
    let points = ends
        .iter()
        .enumerate()
        .map(|(index, replayed)| {
            let limit = ends.get(index + 1).copied();
            let required = limit.map_or(acks.len(), |next| {
                mark_positions
                    .iter()
                    .take_while(|position| **position < next)
                    .count()
            });
            let started = starts
                .iter()
                .take_while(|(position, _)| position < replayed)
                .map(|(_, seq)| *seq)
                .max();
            Point {
                replayed: *replayed,
                required,
                started,
            }
        })
        .collect();
    Ok(points)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{DISCARD, FLUSH, FUA, LOG_MAGIC, LogError, MARK, Point, WriteLog, plan};

    const SECTOR: usize = 512;

    fn header(sector: u64, sectors: u64, flags: u64, mark: &str) -> Vec<u8> {
        let mut block = vec![0_u8; SECTOR];
        let fields = [sector, sectors, flags, mark.len() as u64];
        for (index, value) in fields.iter().enumerate() {
            block[index * 8..index * 8 + 8].copy_from_slice(&value.to_le_bytes());
        }
        block[32..32 + mark.len()].copy_from_slice(mark.as_bytes());
        block
    }

    /// A log of: write 1 sector at 4, flush, mark ack-1, FUA write of 2
    /// sectors at 8, discard of 3 sectors at 4.
    fn sample() -> Vec<u8> {
        let mut log = vec![0_u8; SECTOR];
        log[0..8].copy_from_slice(&LOG_MAGIC.to_le_bytes());
        log[8..16].copy_from_slice(&1_u64.to_le_bytes());
        log[16..24].copy_from_slice(&5_u64.to_le_bytes());
        log[24..28].copy_from_slice(&512_u32.to_le_bytes());
        log.extend(header(4, 1, 0, ""));
        log.extend(vec![0xaa; SECTOR]);
        log.extend(header(0, 0, FLUSH, ""));
        log.extend(header(0, 0, MARK, "ack-1"));
        log.extend(header(8, 2, FUA, ""));
        log.extend(vec![0xbb; 2 * SECTOR]);
        log.extend(header(4, 3, DISCARD, ""));
        log
    }

    #[test]
    fn a_log_is_read_entry_by_entry() -> Result<(), LogError> {
        let bytes = sample();
        let length = bytes.len() as u64;
        let log = WriteLog::read(&mut Cursor::new(bytes), length)?;
        assert_eq!(log.sector_size, 512);
        assert_eq!(log.entries.len(), 5);
        assert_eq!(log.entries[0].data_offset, 1024);
        assert_eq!(log.entries[2].mark.as_deref(), Some("ack-1"));
        assert_eq!(log.entries[3].data_offset, 512 * 6);
        assert!(log.entries[1].is_durability_point() && log.entries[3].is_durability_point());
        Ok(())
    }

    #[test]
    fn a_truncated_or_foreign_log_is_refused() {
        let bytes = sample();
        let short = bytes[..bytes.len() - SECTOR].to_vec();
        let length = short.len() as u64;
        assert!(matches!(
            WriteLog::read(&mut Cursor::new(short), length),
            Err(LogError::Entry(4))
        ));
        let mut foreign = sample();
        foreign[0] ^= 1;
        let length = foreign.len() as u64;
        assert!(matches!(
            WriteLog::read(&mut Cursor::new(foreign), length),
            Err(LogError::NotALog)
        ));
    }

    #[test]
    fn an_acknowledgement_binds_from_the_last_point_before_its_mark() -> Result<(), LogError> {
        let bytes = sample();
        let length = bytes.len() as u64;
        let log = WriteLog::read(&mut Cursor::new(bytes), length)?;
        // Points before anything (0 entries), after the flush (2), the FUA
        // write (4) and the end (5). ack-1 was made after the flush and
        // before the FUA write, so it binds already at the flush.
        assert_eq!(
            plan(&log, &[1]),
            Ok(vec![
                Point {
                    replayed: 0,
                    required: 0,
                    started: None
                },
                Point {
                    replayed: 2,
                    required: 1,
                    started: None
                },
                Point {
                    replayed: 4,
                    required: 1,
                    started: None
                },
                Point {
                    replayed: 5,
                    required: 1,
                    started: None
                },
            ])
        );
        assert_eq!(plan(&log, &[1, 2]), Err(2));
        Ok(())
    }

    /// A managed log: command 1 (start, write, flush, ack); commands 2 and 3
    /// failed (start, write, flush each); command 4 (start, write, flush,
    /// write, flush, ack).
    fn managed_sample() -> Vec<u8> {
        let mut log = vec![0_u8; SECTOR];
        log[0..8].copy_from_slice(&LOG_MAGIC.to_le_bytes());
        log[8..16].copy_from_slice(&1_u64.to_le_bytes());
        log[16..24].copy_from_slice(&16_u64.to_le_bytes());
        log[24..28].copy_from_slice(&512_u32.to_le_bytes());
        let write = |log: &mut Vec<u8>, sector: u64, fill: u8| {
            log.extend(header(sector, 1, 0, ""));
            log.extend(vec![fill; SECTOR]);
        };
        log.extend(header(0, 0, MARK, "start-1")); // 0
        write(&mut log, 4, 0xaa); // 1
        log.extend(header(0, 0, FLUSH, "")); // 2
        log.extend(header(0, 0, MARK, "ack-1")); // 3
        log.extend(header(0, 0, MARK, "start-2")); // 4
        write(&mut log, 5, 0xbb); // 5
        log.extend(header(0, 0, FLUSH, "")); // 6
        log.extend(header(0, 0, MARK, "start-3")); // 7
        write(&mut log, 6, 0xcc); // 8
        log.extend(header(0, 0, FLUSH, "")); // 9
        log.extend(header(0, 0, MARK, "start-4")); // 10
        write(&mut log, 7, 0xdd); // 11
        log.extend(header(0, 0, FLUSH, "")); // 12
        write(&mut log, 8, 0xee); // 13
        log.extend(header(0, 0, FLUSH, "")); // 14
        log.extend(header(0, 0, MARK, "ack-4")); // 15
        log
    }

    /// Regression (run 36793177930): a point records the last command that
    /// had started, so the replay can tell the command in flight there (its
    /// writes may be in the prefix, its acknowledgement not yet made) from
    /// one that had not begun.
    #[test]
    fn a_point_records_the_last_command_started_before_it() -> Result<(), LogError> {
        let bytes = managed_sample();
        let length = bytes.len() as u64;
        let log = WriteLog::read(&mut Cursor::new(bytes), length)?;
        assert_eq!(log.entries.len(), 16);
        let summary = plan(&log, &[1, 4]).map(|points| {
            points
                .iter()
                .map(|point| (point.replayed, point.required, point.started))
                .collect::<Vec<_>>()
        });
        // (replayed, required, started). At 7 and 10 the failed commands 2
        // and 3 had started but not command 4, the next acknowledged one: it
        // cannot be in flight. At 13 command 4 had started and written, and
        // its acknowledgement does not bind yet: it is in flight. From 15 it
        // binds.
        assert_eq!(
            summary,
            Ok(vec![
                (0, 0, None),
                (3, 1, Some(1)),
                (7, 1, Some(2)),
                (10, 1, Some(3)),
                (13, 1, Some(4)),
                (15, 2, Some(4)),
                (16, 2, Some(4)),
            ])
        );
        Ok(())
    }
}
