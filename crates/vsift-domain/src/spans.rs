//! Half-open spans of source time, in microseconds, and the arithmetic on lists
//! of them that coverage and the gaps of a recognition run share.
//!
//! One home for it: a list that is "merged" is sorted, holds no empty span and
//! has no two spans that overlap or touch, which is the form both
//! [`subtract`] and the readers of a coverage list expect.

use crate::{MediaTime, TimeRange};

/// A half-open span in microseconds; always `start < end` where built here.
pub(crate) type Span = (u64, u64);

pub(crate) fn span(range: TimeRange) -> Span {
    (range.start().as_micros(), range.end().as_micros())
}

pub(crate) fn to_range((start, end): Span) -> Option<TimeRange> {
    TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end)).ok()
}

pub(crate) fn to_ranges(spans: &[Span]) -> Vec<TimeRange> {
    spans.iter().copied().filter_map(to_range).collect()
}

/// Sorts `spans` and merges every overlapping or touching pair.
pub(crate) fn merged(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|(start, end)| start < end);
    spans.sort_unstable();
    let mut result: Vec<Span> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match result.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => result.push((start, end)),
        }
    }
    result
}

/// The merged spans of `spans` minus every span of `removed` (both merged).
pub(crate) fn subtract(spans: &[Span], removed: &[Span]) -> Vec<Span> {
    let mut result = Vec::new();
    for &(start, end) in spans {
        let mut cursor = start;
        for &(removed_start, removed_end) in removed {
            if removed_end <= cursor {
                continue;
            }
            if removed_start >= end {
                break;
            }
            if removed_start > cursor {
                result.push((cursor, removed_start));
            }
            cursor = cursor.max(removed_end);
            if cursor >= end {
                break;
            }
        }
        if cursor < end {
            result.push((cursor, end));
        }
    }
    result
}
