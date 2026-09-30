//! Finding the one fenced `vsift-handoff` block of a report and reading it.

use serde_json::Value;

use super::rule::{HandoffFinding, HandoffRule};
use crate::{JsonLimits, StrictJsonError, decode_strict_json};

/// The fenced block's info string.
const FENCE_INFO: &str = "vsift-handoff";

/// How deeply the block's JSON may nest; a handoff nests about six levels.
const MAX_HANDOFF_NESTING: usize = 32;

/// The block's JSON and the report line it starts on.
pub(crate) struct Block {
    /// The parsed JSON.
    pub(crate) value: Value,
}

/// Extracts the one `vsift-handoff` block of `report` and parses it.
///
/// A fence opens with at least three backticks and the info string alone,
/// and closes with at least as many backticks and nothing else, as
/// `CommonMark` says.
pub(crate) fn extract(report: &str, max_bytes: usize) -> Result<Block, HandoffFinding> {
    let mut blocks: Vec<(u32, String)> = Vec::new();
    let mut open: Option<(usize, u32, Vec<&str>)> = None;
    for (index, line) in report.lines().enumerate() {
        let number = line_number(index);
        let trimmed = line.trim_start();
        let ticks = trimmed.chars().take_while(|value| *value == '`').count();
        match open.take() {
            Some((opened, first, body))
                if ticks >= opened && trimmed[ticks..].trim().is_empty() =>
            {
                blocks.push((first, body.join("\n")));
            }
            Some((opened, first, mut body)) => {
                body.push(line);
                open = Some((opened, first, body));
            }
            None if ticks >= 3 && trimmed[ticks..].trim() == FENCE_INFO => {
                open = Some((ticks, number.saturating_add(1), Vec::new()));
            }
            None => {}
        }
    }
    if let Some((_, first, _)) = open {
        return Err(HandoffFinding::on_line(
            Some(first.saturating_sub(1)),
            HandoffRule::HandoffBlockUnclosed,
        ));
    }
    match blocks.as_slice() {
        [] => Err(HandoffFinding::on_line(
            None,
            HandoffRule::HandoffBlockMissing,
        )),
        [(first, body)] => parse(body, *first, max_bytes),
        [_, (second, _), ..] => Err(HandoffFinding::on_line(
            Some(second.saturating_sub(1)),
            HandoffRule::HandoffBlockRepeated,
        )),
    }
}

fn parse(body: &str, first: u32, max_bytes: usize) -> Result<Block, HandoffFinding> {
    let limits = JsonLimits {
        max_bytes,
        max_nesting: MAX_HANDOFF_NESTING,
    };
    match decode_strict_json::<Value>(body.as_bytes(), limits) {
        Ok(value) => Ok(Block { value }),
        Err(StrictJsonError::TooDeep) => Err(HandoffFinding::on_line(
            Some(first),
            HandoffRule::HandoffJsonTooDeep,
        )),
        Err(StrictJsonError::Malformed(error)) => {
            let within = u32::try_from(error.line().max(1)).unwrap_or(u32::MAX);
            Err(HandoffFinding::on_line(
                Some(first.saturating_add(within - 1)),
                HandoffRule::HandoffJsonInvalid,
            ))
        }
        Err(StrictJsonError::TooLarge) => Err(HandoffFinding::on_line(
            Some(first),
            HandoffRule::HandoffJsonInvalid,
        )),
    }
}

/// The 1-based number of the line at `index`.
pub(crate) fn line_number(index: usize) -> u32 {
    u32::try_from(index).map_or(u32::MAX, |index| index.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::HandoffRule;

    fn rule(report: &str) -> Option<HandoffRule> {
        extract(report, 65_536).err().map(|finding| finding.rule())
    }

    #[test]
    fn exactly_one_closed_block_of_json_is_read() {
        let report = "## Problem\n\nx\n\n```vsift-handoff\n{\"handoff_version\": \"1\"}\n```\n";
        assert_eq!(
            extract(report, 65_536)
                .map(|block| block.value["handoff_version"].clone())
                .ok(),
            Some(serde_json::Value::from("1"))
        );
        assert_eq!(rule("no block"), Some(HandoffRule::HandoffBlockMissing));
        assert_eq!(
            rule("```vsift-handoff\n{}\n```\n```vsift-handoff\n{}\n```"),
            Some(HandoffRule::HandoffBlockRepeated)
        );
        assert_eq!(
            rule("```vsift-handoff\n{\n"),
            Some(HandoffRule::HandoffBlockUnclosed)
        );
        assert_eq!(
            rule("```vsift-handoff\n{,}\n```"),
            Some(HandoffRule::HandoffJsonInvalid)
        );
        let deep = format!(
            "```vsift-handoff\n{}{}\n```",
            "[".repeat(40),
            "]".repeat(40)
        );
        assert_eq!(rule(&deep), Some(HandoffRule::HandoffJsonTooDeep));
    }

    #[test]
    fn a_json_error_names_its_report_line() {
        let report = "a\nb\n```vsift-handoff\n{\n\"x\": ,\n}\n```\n";
        let finding = extract(report, 65_536).err();
        assert_eq!(finding.and_then(|finding| finding.line()), Some(5));
    }
}
