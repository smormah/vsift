//! The rank of hits across retained bundles (ADR 0035 decision 5).
//!
//! Search stays literal and deterministic, with no scoring. Every phrase hit
//! comes before any all-terms hit; inside a tier the hits of one bundle stay
//! together, in the order of the bundles; inside a bundle they run by time, a
//! transcript segment before on-screen text at the same time.

use crate::MediaTime;

use super::SearchMatch;

/// What kind of evidence a hit is; earlier kinds rank first at the same time.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BundleHitKind {
    /// A transcript segment.
    Transcript,
    /// An occurrence of on-screen text.
    ScreenText,
}

impl BundleHitKind {
    /// Stable identifier written to a hit's `kind`.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::ScreenText => "screen_text",
        }
    }
}

/// The order of one hit among the hits of its bundle.
///
/// The derived order is the rank: tier, then time, then kind, then the ordinal
/// that separates two hits of one kind at the same time.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BundleHitKey {
    /// The tier it matched in.
    pub tier: SearchMatch,
    /// A segment's start, or the first frame of a line.
    pub time: MediaTime,
    /// What kind of evidence it is.
    pub kind: BundleHitKind,
    /// Its ordinal within its kind and tier.
    pub ordinal: u32,
}

/// The rank position of a hit across bundles, as a cursor records it.
///
/// The derived order is the rank: tier, then bundle, then the hit's place
/// among that bundle's hits of that tier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BundleHitPosition {
    tier: SearchMatch,
    bundle: u16,
    sequence: u32,
}

impl BundleHitPosition {
    /// The position `sequence` (from 1) of `bundle`'s hits in `tier`.
    #[must_use]
    pub const fn new(tier: SearchMatch, bundle: u16, sequence: u32) -> Self {
        Self {
            tier,
            bundle,
            sequence,
        }
    }

    /// The tier of the hit.
    #[must_use]
    pub const fn tier(self) -> SearchMatch {
        self.tier
    }

    /// The rank of the hit's bundle among the bundles searched, from 0.
    #[must_use]
    pub const fn bundle(self) -> u16 {
        self.bundle
    }

    /// The hit's place among its bundle's hits of its tier, from 1.
    #[must_use]
    pub const fn sequence(self) -> u32 {
        self.sequence
    }
}

/// Orders the hits of one bundle and gives each its position across bundles.
#[must_use]
pub fn rank_bundle_hits<T>(
    bundle: u16,
    mut hits: Vec<(BundleHitKey, T)>,
) -> Vec<(BundleHitPosition, T)> {
    hits.sort_by_key(|(key, _)| *key);
    let mut tier = None;
    let mut sequence = 0_u32;
    hits.into_iter()
        .map(|(key, hit)| {
            if tier != Some(key.tier) {
                tier = Some(key.tier);
                sequence = 0;
            }
            sequence = sequence.saturating_add(1);
            (BundleHitPosition::new(key.tier, bundle, sequence), hit)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        BundleHitKey, BundleHitKind, BundleHitPosition, MediaTime, SearchMatch, rank_bundle_hits,
    };

    fn key(tier: SearchMatch, seconds: u64, kind: BundleHitKind, ordinal: u32) -> BundleHitKey {
        BundleHitKey {
            tier,
            time: MediaTime::from_micros(seconds * 1_000_000),
            kind,
            ordinal,
        }
    }

    /// Phrases first; by time inside a tier, whatever the kind; a transcript
    /// segment before on-screen text at the same time.
    #[test]
    fn hits_of_a_bundle_rank_by_tier_then_time_then_kind() {
        let ranked = rank_bundle_hits(
            3,
            vec![
                (
                    key(SearchMatch::AllTerms, 1, BundleHitKind::Transcript, 1),
                    "all-1",
                ),
                (
                    key(SearchMatch::Phrase, 9, BundleHitKind::Transcript, 2),
                    "phrase-9",
                ),
                (
                    key(SearchMatch::Phrase, 5, BundleHitKind::ScreenText, 1),
                    "screen-5",
                ),
                (
                    key(SearchMatch::Phrase, 5, BundleHitKind::Transcript, 1),
                    "speech-5",
                ),
            ],
        );
        let order: Vec<(BundleHitPosition, &str)> = ranked;
        assert_eq!(
            order,
            vec![
                (
                    BundleHitPosition::new(SearchMatch::Phrase, 3, 1),
                    "speech-5"
                ),
                (
                    BundleHitPosition::new(SearchMatch::Phrase, 3, 2),
                    "screen-5"
                ),
                (
                    BundleHitPosition::new(SearchMatch::Phrase, 3, 3),
                    "phrase-9"
                ),
                (BundleHitPosition::new(SearchMatch::AllTerms, 3, 1), "all-1"),
            ]
        );
    }

    /// Every phrase hit of a later bundle ranks before any all-terms hit of an
    /// earlier one.
    #[test]
    fn the_tier_outranks_the_bundle() {
        assert!(
            BundleHitPosition::new(SearchMatch::Phrase, 9, 40)
                < BundleHitPosition::new(SearchMatch::AllTerms, 0, 1)
        );
        assert!(
            BundleHitPosition::new(SearchMatch::Phrase, 1, 40)
                < BundleHitPosition::new(SearchMatch::Phrase, 2, 1)
        );
    }
}
