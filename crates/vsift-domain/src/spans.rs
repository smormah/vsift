//! Half-open spans of source time, in microseconds, and the arithmetic on lists
//! of them that coverage and the gaps of a recognition run share.
//!
//! One home for it: a list that is "merged" is sorted, holds no empty span and
//! has no two spans that overlap or touch, which is the form both
//! [`subtract`] and the readers of a coverage list expect.

use std::collections::BTreeMap;

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
///
/// Two spans on opposite sides of `keep_apart` are never joined: `spans` holds
/// none that overlaps it, and a join across it would. When only such a pair is
/// left the list is returned longer than `limit`, which a caller that gives a
/// `limit` of at least two for a range it keeps apart never meets.
pub(crate) fn join_nearest(
    mut spans: Vec<Span>,
    limit: usize,
    keep_apart: Option<Span>,
) -> Vec<Span> {
    let limit = limit.max(1);
    while spans.len() > limit {
        let nearest = spans
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| {
                keep_apart.is_none_or(|(start, end)| !(pair[0].1 <= start && end <= pair[1].0))
            })
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

/// A growing set of spans, kept merged as spans are added, that answers which
/// parts of a span it does not cover.
///
/// The seam merge asks that of every segment it dropped, against a set that
/// grows as it restores some of them, and a run can hold hundreds of thousands
/// of segments (1,024 chunks of up to 256 each). Rebuilding a merged list and
/// subtracting from it for each would be quadratic; keeping the spans in a
/// tree makes both the insertion and the question logarithmic in the number
/// of spans plus the number of spans touched.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpanSet {
    /// Start to end, disjoint and never touching, so every span is maximal.
    spans: BTreeMap<u64, u64>,
}

impl SpanSet {
    /// Adds `span`, joining every span it overlaps or touches.
    pub(crate) fn insert(&mut self, (start, end): Span) {
        if start >= end {
            return;
        }
        let (mut start, mut end) = (start, end);
        if let Some((&before_start, &before_end)) = self.spans.range(..=start).next_back()
            && before_end >= start
        {
            start = before_start;
            end = end.max(before_end);
            self.spans.remove(&before_start);
        }
        while let Some((&next_start, &next_end)) = self.spans.range(start..=end).next() {
            end = end.max(next_end);
            self.spans.remove(&next_start);
        }
        self.spans.insert(start, end);
    }

    /// The parts of `(start, end)` that no span of the set covers, in start
    /// order.
    pub(crate) fn gaps_within(&self, (start, end): Span) -> Vec<Span> {
        if start >= end {
            return Vec::new();
        }
        let mut gaps = Vec::new();
        let mut cursor = start;
        // The span that starts at or before `start` may reach into it; every
        // later span that matters starts inside it.
        let first = self
            .spans
            .range(..=start)
            .next_back()
            .map_or(start, |(&span_start, _)| span_start);
        for (&span_start, &span_end) in self.spans.range(first..end) {
            if span_end <= cursor {
                continue;
            }
            if span_start > cursor {
                gaps.push((cursor, span_start));
            }
            cursor = cursor.max(span_end);
            if cursor >= end {
                return gaps;
            }
        }
        if cursor < end {
            gaps.push((cursor, end));
        }
        gaps
    }
}

#[cfg(test)]
mod tests {
    use super::{SpanSet, join_nearest, merged, overlaps_any, subtract};

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
        assert_eq!(join_nearest(spans.clone(), 4, None), spans);
        // The nearest pair is (10, 11) and (12, 13), a stretch of one.
        assert_eq!(
            join_nearest(spans.clone(), 3, None),
            vec![(0, 1), (10, 13), (30, 31)]
        );
        // Then (0, 1) and (10, 13), nine apart, are nearer than the last, 17 away.
        assert_eq!(
            join_nearest(spans.clone(), 2, None),
            vec![(0, 13), (30, 31)]
        );
        assert_eq!(join_nearest(spans.clone(), 1, None), vec![(0, 31)]);
        // A limit of nothing still leaves one, and an empty list stays empty.
        assert_eq!(join_nearest(spans, 0, None), vec![(0, 31)]);
        assert_eq!(join_nearest(Vec::new(), 3, None), Vec::new());
    }

    /// A range kept apart is never crossed: the nearest pair is the two spans a
    /// short range splits, and they are not joined, whatever the limit. Without
    /// the range the same list does join them, and the join overlaps it.
    #[test]
    fn spans_on_either_side_of_a_range_kept_apart_are_never_joined() {
        // A range 60-70 splits the span 55-75 into 55-60 and 70-75, ten apart.
        let spans = vec![(0, 25), (55, 60), (70, 75), (105, 125), (155, 175)];
        let kept_apart = Some((60, 70));
        let crossed = join_nearest(spans.clone(), 4, None);
        assert!(crossed.contains(&(55, 75)), "{crossed:?}");
        let joined = join_nearest(spans.clone(), 4, kept_apart);
        assert_eq!(joined, vec![(0, 60), (70, 75), (105, 125), (155, 175)]);
        for limit in [3, 2] {
            let joined = join_nearest(spans.clone(), limit, kept_apart);
            assert!(joined.len() <= limit, "{limit}: {joined:?}");
            assert!(
                joined.iter().all(|&(start, end)| end <= 60 || start >= 70),
                "{limit}: {joined:?}"
            );
            assert!(subtract(&spans, &joined).is_empty(), "{limit}");
        }
        // With one span a side and a limit of one, nothing can be joined.
        assert_eq!(
            join_nearest(vec![(0, 10), (80, 90)], 1, Some((20, 70))),
            vec![(0, 10), (80, 90)]
        );
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
            let joined = join_nearest(spans.clone(), limit, None);
            assert!(joined.len() <= limit.max(1));
            assert_eq!(merged(joined.clone()), joined);
            assert!(subtract(&spans, &joined).is_empty(), "{limit}");
        }
    }

    /// The set is the merged list of what was inserted, whatever the order and
    /// the overlaps, and its gaps are the subtraction of that list.
    #[test]
    fn a_span_set_agrees_with_merging_and_subtracting_lists() {
        let mut inserted: Vec<(u64, u64)> = Vec::new();
        let mut set = SpanSet::default();
        // A fixed pseudo-random walk, so the test needs no generator and a
        // failure repeats.
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        for step in 0..400_u64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let start = state % 500;
            let length = 1 + (state >> 20) % 12;
            set.insert((start, start + length));
            inserted.push((start, start + length));
            let list = merged(inserted.clone());
            let probe_start = (state >> 32) % 520;
            let probe = (probe_start, probe_start + 1 + (state >> 40) % 60);
            assert_eq!(
                set.gaps_within(probe),
                subtract(&[probe], &list),
                "step {step}, probe {probe:?}"
            );
        }
        // Nothing, and nothing over a degenerate span.
        assert_eq!(SpanSet::default().gaps_within((5, 9)), vec![(5, 9)]);
        assert!(set.gaps_within((7, 7)).is_empty());
        assert!(set.gaps_within((9, 7)).is_empty());
    }

    /// Spans that touch or overlap become one, so a gap is never reported
    /// between two spans that meet.
    #[test]
    fn spans_that_meet_are_one_span() {
        let mut set = SpanSet::default();
        set.insert((10, 20));
        set.insert((20, 30));
        assert_eq!(set.gaps_within((10, 30)), Vec::<(u64, u64)>::new());
        set.insert((40, 50));
        set.insert((5, 45));
        assert_eq!(set.gaps_within((0, 60)), vec![(0, 5), (50, 60)]);
        set.insert((0, 0));
        set.insert((9, 3));
        assert_eq!(set.gaps_within((0, 60)), vec![(0, 5), (50, 60)]);
    }
}
