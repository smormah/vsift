//! The `--view` option of `transcript get`, `search` and `candidates` at the
//! host (#365): which view an answer is built in, and the refusal of the brief
//! view for an evidence stream.
//!
//! The views themselves, and what each leaves out, are the contract crate's
//! ([`vsift_contract::ResultView`]); this module only decides, from the output
//! mode, which of them a command line gets.

use vsift::FailureCode;
use vsift_contract::{BRIEF_VIEW_STREAM_REMEDIATION, ResultView};

use crate::{CommandFailure, command::ViewArgument, output::OutputMode};

/// The view an answer is built in: the one asked for in `--json`, and the full
/// one in readable text.
///
/// The readable text is made from the full answer and prints only what a
/// person reads (it never repeats the hashes the brief view leaves out of each
/// item), so there it is the same for both views; `--view` shapes the JSON.
/// `--events jsonl` never reaches this: a stream is always full, and the brief
/// view is refused for it by [`refuse_brief_stream`].
pub(crate) const fn presented_view(mode: OutputMode, requested: ViewArgument) -> ResultView {
    match mode {
        OutputMode::Human => ResultView::Full,
        OutputMode::Json | OutputMode::JsonLines => requested.result_view(),
    }
}

/// Refuses the brief view for an evidence stream, before anything is read or
/// analysed: the records of `--events jsonl` are published evidence that an
/// index stores by key, and a reduced record under a key would replace a
/// complete one (#365). The answer is the usual one terminal failure event,
/// `INVALID_ARGUMENT`, whose remediation says why.
pub(crate) fn refuse_brief_stream(requested: ViewArgument) -> Result<(), CommandFailure> {
    match requested.result_view() {
        ResultView::Full => Ok(()),
        ResultView::Brief => Err(CommandFailure::with_remediation(
            FailureCode::InvalidArgument,
            BRIEF_VIEW_STREAM_REMEDIATION.to_owned(),
        )),
    }
}
