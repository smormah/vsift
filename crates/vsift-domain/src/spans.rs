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

/// Whether `candidate` overlaps (shares more than a point with) any span of
/// `spans`, which is merged: sorted, so that the ends are sorted too.
pub(crate) fn overlaps_any(spans: &[Span], (start, end): Span) -> bool {
    let first_ending_after = spans.partition_point(|&(_, span_end)| span_end <= start);
    spans
        .get(first_ending_after)
        .is_some_and(|&(span_start, _)| span_start < end)
}

/// Joins the two spans of `spans` that lie nearest each other, and again,
/// until at most `limit` (at least one) remain. `spans` is merged. The result
/// covers everything the input did and the stretches between the spans it
/// joined, so a list bounded this way describes more than it was given, never
/// less.
pub(crate) fn join_nearest(mut spans: Vec<Span>, limit: usize) -> Vec<Span> {
    let limit = limit.max(1);
    while spans.len() > limit {
        let nearest = spans
            .windows(2)
            .enumerate()
            .min_by_key(|(_, pair)| pair[1].0.saturating_sub(pair[0].1))
            .map(|(index, _)| index);
        let Some(index) = nearest else {
            break;
        };
        let next = spans.remove(index + 1);
        if let Some(joined) = spans.get_mut(index) {
            joined.1 = joined.1.max(next.1);
        }
    }
    spans
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

#[cfg(test)]
mod tests {
    use super::{join_nearest, merged, overlaps_any, subtract};

    #[test]
    fn overlap_means_sharing_more_than_a_point() {
        let spans = [(10, 20), (30, 40)];
        assert!(overlaps_any(&spans, (19, 21)));
        assert!(overlaps_any(&spans, (0, 100)));
        assert!(overlaps_any(&spans, (35, 36)));
        // Touching an edge, or lying between or outside the spans, does not.
        assert!(!overlaps_any(&spans, (20, 30)));
        assert!(!overlaps_any(&spans, (40, 50)));
        assert!(!overlaps_any(&spans, (0, 10)));
        assert!(!overlaps_any(&[], (0, 10)));
    }

    #[test]
    fn the_nearest_spans_are_joined_until_the_list_is_short_enough() {
        let spans = vec![(0, 1), (10, 11), (12, 13), (30, 31)];
        assert_eq!(join_nearest(spans.clone(), 4), spans);
        // The nearest pair is (10, 11) and (12, 13), a stretch of one.
        assert_eq!(
            join_nearest(spans.clone(), 3),
            vec![(0, 1), (10, 13), (30, 31)]
        );
        // Then (0, 1) and (10, 13), nine apart, are nearer than the last, 17 away.
        assert_eq!(join_nearest(spans.clone(), 2), vec![(0, 13), (30, 31)]);
        assert_eq!(join_nearest(spans.clone(), 1), vec![(0, 31)]);
        // A limit of nothing still leaves one, and an empty list stays empty.
        assert_eq!(join_nearest(spans, 0), vec![(0, 31)]);
        assert_eq!(join_nearest(Vec::new(), 3), Vec::new());
    }

    /// Joining only adds: every instant of the input is in the output, the
    /// output is merged, and it is no longer than the limit.
    #[test]
    fn joining_never_drops_an_instant() {
        let spans = merged(
            (0..50_u64)
                .map(|index| (index * 7, index * 7 + 1 + index % 3))
                .collect(),
        );
        for limit in [1_usize, 2, 5, 17, 49, 50, 80] {
            let joined = join_nearest(spans.clone(), limit);
            assert!(joined.len() <= limit.max(1));
            assert_eq!(merged(joined.clone()), joined);
            assert!(subtract(&spans, &joined).is_empty(), "{limit}");
        }
    }
}
