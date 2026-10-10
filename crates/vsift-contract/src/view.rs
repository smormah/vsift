//! The view of a read: how much of each item a caller asks for (#365).
//!
//! A transcript segment is about a thousand characters of JSON and a visual
//! candidate about six hundred, and nearly half of each is the same on every
//! item of one revision (or one visual index): the two SHA-256 digests, the
//! source and source-segment identities, the model and decoding profiles, the
//! provider name, the displayed size. The answer already states them once, in
//! its revision (or index) block. The `brief` view leaves them out of the
//! items, so a page of fifty fits where a page of fifty full items did not
//! (a client that saved a 54,000-character page and read only its first
//! 43,000 lost nine items and the page's cursor, with no error from `VSift`).
//!
//! **A view is a presentation, never a different result.** Both views are made
//! from the one full item, so a brief item can only hold fields the full item
//! holds, with the same values (each brief type is projected from its full
//! type, and a contract test compares them for every example). The view
//! never reaches a stored record, a retained bundle or an evidence stream: the
//! records of `--events jsonl` are published evidence that an indexer upserts
//! by key, and a reduced record under the same key would replace a complete
//! one, so a stream is always full and a brief view of one is refused
//! ([`BRIEF_VIEW_STREAM_REMEDIATION`]).

/// Which presentation of a read the caller asked for.
///
/// The default is [`ResultView::Full`], the answer every release gave, so an
/// answer to a request that does not name a view is byte for byte what it was.
/// A `view` member appears in `data` only for [`ResultView::Brief`]; a reader
/// that finds none has the full view.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResultView {
    /// Every published field of every item.
    #[default]
    Full,
    /// Each item without what its revision or index block already says once.
    Brief,
}

impl ResultView {
    /// Every view, in declaration order.
    ///
    /// Contract tests compare this list with the `view` values the published
    /// schemas allow, so a view cannot be added on one side only.
    pub const ALL: [Self; 2] = [Self::Full, Self::Brief];

    /// The stable identifier written to the `view` member and given to the
    /// `--view` option.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Brief => "brief",
        }
    }
}

/// The value of the `view` member of a brief answer: the one view that names
/// itself, so a full answer stays exactly what it always was.
pub(crate) const BRIEF_VIEW: &str = ResultView::Brief.identifier();

/// Remediation when the brief view is asked for with `--events jsonl`.
///
/// Says why, so that a caller does not simply retry with another spelling: an
/// evidence stream's records are published evidence stored by their `key`, and
/// a reduced record under a key would replace the complete one an index holds.
/// Fixed prose, no path, no argument text.
pub const BRIEF_VIEW_STREAM_REMEDIATION: &str = "The brief view leaves fields out of each item, and the records of an --events jsonl stream are published evidence that an index stores by key, so a reduced record would replace a complete one. Nothing was read. Ask for the stream without --view, or ask for the brief view with --json.";

#[cfg(test)]
mod tests {
    use super::{BRIEF_VIEW, BRIEF_VIEW_STREAM_REMEDIATION, ResultView};

    #[test]
    fn the_default_is_the_full_view_and_identifiers_are_distinct() {
        assert_eq!(ResultView::default(), ResultView::Full);
        assert_eq!(ResultView::Full.identifier(), "full");
        assert_eq!(BRIEF_VIEW, "brief");
        assert_ne!(
            ResultView::Full.identifier(),
            ResultView::Brief.identifier()
        );
        assert_eq!(ResultView::ALL.len(), 2);
    }

    #[test]
    fn the_stream_refusal_is_bounded_fixed_prose_without_a_path() {
        assert!(BRIEF_VIEW_STREAM_REMEDIATION.len() <= 1_024);
        assert!(!BRIEF_VIEW_STREAM_REMEDIATION.contains(['/', '\\']));
    }
}
