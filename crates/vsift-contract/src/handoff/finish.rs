//! `handoff finish` (0.3.0, M5b; ADR 0023 note of 2026-11-09): the skill's
//! closing steps in one call.
//!
//! In the recorded agent trials the closing steps (`handoff check`, then
//! `session retain`, then `bundle validate`, or `session close`) took three
//! to four round trips with no decision between them: every input was known
//! before the first. `handoff finish` checks the draft with the session's
//! records, and only when the draft passes and states the lifecycle action
//! asked for does a host make at most one write, the standalone write
//! itself. This module holds the rule that decides whether the write may
//! happen ([`HandoffFinishRequest::decide`]) and the published data
//! ([`HandoffFinishData`]); the host reads the session and makes the write.
//!
//! **Retention stays explicit** (ADR 0013): only a request that names the
//! new directory is [`HandoffFinishRequest::Retain`], and a draft that does
//! not pass, or that states another lifecycle action, writes nothing.

use serde::Serialize;

use super::{HandoffCheck, HandoffCheckData};
use crate::{BundleData, StatusData};

/// The one write a `handoff finish` request asks for, if any.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffFinishRequest {
    /// Check the draft with the session's records; write nothing.
    Check,
    /// Then retain the session to the new directory the caller named
    /// (`--retain <new-directory>`).
    Retain,
    /// Then close the session (`--close`).
    Close,
}

impl HandoffFinishRequest {
    /// The lifecycle action a draft must state for this request's write, if
    /// it asks for one.
    #[must_use]
    pub const fn required_action(self) -> Option<HandoffLifecycleAction> {
        match self {
            Self::Check => None,
            Self::Retain => Some(HandoffLifecycleAction::Retained),
            Self::Close => Some(HandoffLifecycleAction::Closed),
        }
    }

    /// Whether the write may happen after `check`.
    ///
    /// A draft with any error stops at [`HandoffFinishStop::Draft`]; a write
    /// whose action the draft does not state stops at
    /// [`HandoffFinishStop::Lifecycle`], so a report never claims a
    /// retention or a close that did not happen, and nothing happens that
    /// the report does not claim. A request without a write compares no
    /// lifecycle: an agent may have retained or closed with the standalone
    /// command before.
    #[must_use]
    pub fn decide(self, check: &HandoffCheck) -> HandoffFinishDecision {
        if !check.is_valid() {
            return HandoffFinishDecision::Stop(HandoffFinishStop::Draft);
        }
        match self.required_action() {
            Some(required) if stated_action(check) != Some(required) => {
                HandoffFinishDecision::Stop(HandoffFinishStop::Lifecycle)
            }
            _ => HandoffFinishDecision::Proceed,
        }
    }
}

/// What [`HandoffFinishRequest::decide`] allows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffFinishDecision {
    /// Make the requested write, if any.
    Proceed,
    /// Write nothing, for this reason.
    Stop(HandoffFinishStop),
}

/// Why a `handoff finish` stopped before any write: `data.stopped_at`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffFinishStop {
    /// The draft has errors; `data.check.errors` lists them.
    Draft,
    /// The draft's `lifecycle.action` is not the requested write's.
    Lifecycle,
}

/// A handoff's `lifecycle.action`, as the handoff schema spells it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffLifecycleAction {
    /// The session was left to expire.
    LeftOpen,
    /// The session was closed.
    Closed,
    /// The session was retained to a bundle.
    Retained,
    /// No session was opened.
    NotOpened,
    /// The session expired before the work finished.
    Expired,
}

impl HandoffLifecycleAction {
    /// The schema's word for `text`, or `None` for any other text. Only a
    /// closed word is ever published, so `data.lifecycle.stated` never
    /// repeats draft text (SEC-16).
    fn parse(text: &str) -> Option<Self> {
        match text {
            "left_open" => Some(Self::LeftOpen),
            "closed" => Some(Self::Closed),
            "retained" => Some(Self::Retained),
            "not_opened" => Some(Self::NotOpened),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

/// The draft's `lifecycle.action`, read in the schema's letter case.
fn stated_action(check: &HandoffCheck) -> Option<HandoffLifecycleAction> {
    check
        .handoff()
        .and_then(|handoff| handoff.pointer("/lifecycle/action"))
        .and_then(serde_json::Value::as_str)
        .and_then(HandoffLifecycleAction::parse)
}

/// `data.lifecycle`: the action the draft states against the one asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct HandoffFinishLifecycle {
    stated: Option<HandoffLifecycleAction>,
    requested: Option<HandoffLifecycleAction>,
    agrees: bool,
}

impl HandoffFinishLifecycle {
    fn new(check: &HandoffCheck, request: HandoffFinishRequest) -> Self {
        let stated = stated_action(check);
        let requested = request.required_action();
        Self {
            stated,
            requested,
            agrees: requested.is_none_or(|required| stated == Some(required)),
        }
    }
}

/// The `data` of a complete `handoff.finish` result
/// (`schemas/v1/handoff-finish-data.schema.json`).
///
/// `finished` is whether every requested step was done; a script tests it,
/// not the exit status, before relying on a write: a stop before any write
/// is an answer, `complete` with exit 0, as a draft's verdict is (ADR 0023
/// decision H1). A write that fails is not data: the host answers with that
/// write's own failure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandoffFinishData {
    finished: bool,
    stopped_at: Option<HandoffFinishStop>,
    requested: HandoffFinishRequest,
    check: HandoffCheckData,
    lifecycle: HandoffFinishLifecycle,
    retain: Option<BundleData>,
    bundle_validated: bool,
    close: Option<StatusData>,
}

impl HandoffFinishData {
    fn new(check: HandoffCheck, request: HandoffFinishRequest) -> Self {
        let lifecycle = HandoffFinishLifecycle::new(&check, request);
        Self {
            finished: false,
            stopped_at: None,
            requested: request,
            check: HandoffCheckData::new(check),
            lifecycle,
            retain: None,
            bundle_validated: false,
            close: None,
        }
    }

    /// A request that stopped before any write, for `stop`.
    #[must_use]
    pub fn stopped(
        check: HandoffCheck,
        request: HandoffFinishRequest,
        stop: HandoffFinishStop,
    ) -> Self {
        Self {
            stopped_at: Some(stop),
            ..Self::new(check, request)
        }
    }

    /// A [`HandoffFinishRequest::Check`] whose draft passed. A session that
    /// could not be read is `data.check.session.gap`, as for
    /// `handoff check --session`, and not a stop.
    #[must_use]
    pub fn checked(check: HandoffCheck) -> Self {
        Self {
            finished: true,
            ..Self::new(check, HandoffFinishRequest::Check)
        }
    }

    /// A retention done after a passing draft. `bundle` is the result of
    /// the retention's own revalidation (ADR 0013), the check
    /// `bundle validate` runs, so `bundle_validated` is true.
    #[must_use]
    pub fn retained(check: HandoffCheck, bundle: BundleData) -> Self {
        Self {
            finished: true,
            retain: Some(bundle),
            bundle_validated: true,
            ..Self::new(check, HandoffFinishRequest::Retain)
        }
    }

    /// A close done after a passing draft, with the session's status after
    /// it.
    #[must_use]
    pub fn closed(check: HandoffCheck, status: StatusData) -> Self {
        Self {
            finished: true,
            close: Some(status),
            ..Self::new(check, HandoffFinishRequest::Close)
        }
    }

    /// Whether every requested step was done.
    #[must_use]
    pub const fn finished(&self) -> bool {
        self.finished
    }

    /// Why the request stopped before any write, if it did.
    #[must_use]
    pub const fn stopped_at(&self) -> Option<HandoffFinishStop> {
        self.stopped_at
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        HandoffFinishDecision, HandoffFinishLifecycle, HandoffFinishRequest, HandoffFinishStop,
        HandoffLifecycleAction,
    };
    use crate::handoff::HandoffChecker;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn handoff(action: &str) -> serde_json::Value {
        json!({
            "handoff_version": "1",
            "status": "insufficient_evidence",
            "question": "What does the dialog show?",
            "capabilities": {"image_access": "unavailable"},
            "claims": [],
            "citations": [],
            "gaps": [{"kind": "visual", "reason": "not_inspected", "note": "No frame was opened."}],
            "untrusted_instructions": [],
            "lifecycle": {"action": action}
        })
    }

    #[test]
    fn a_write_needs_the_action_the_draft_states() -> TestResult {
        let checker = HandoffChecker::new()?;
        let retained = checker.check_value(handoff("retained"));
        let closed = checker.check_value(handoff("closed"));
        let left_open = checker.check_value(handoff("left_open"));
        for check in [&retained, &closed, &left_open] {
            assert!(check.is_valid(), "{:?}", check.errors());
        }
        assert_eq!(
            HandoffFinishRequest::Retain.decide(&retained),
            HandoffFinishDecision::Proceed
        );
        assert_eq!(
            HandoffFinishRequest::Close.decide(&closed),
            HandoffFinishDecision::Proceed
        );
        for (request, check) in [
            (HandoffFinishRequest::Retain, &left_open),
            (HandoffFinishRequest::Retain, &closed),
            (HandoffFinishRequest::Close, &retained),
            (HandoffFinishRequest::Close, &left_open),
        ] {
            assert_eq!(
                request.decide(check),
                HandoffFinishDecision::Stop(HandoffFinishStop::Lifecycle),
                "{request:?}"
            );
        }
        // Without a write nothing is compared.
        for check in [&retained, &closed, &left_open] {
            assert_eq!(
                HandoffFinishRequest::Check.decide(check),
                HandoffFinishDecision::Proceed
            );
        }
        Ok(())
    }

    #[test]
    fn a_draft_with_errors_stops_before_the_lifecycle_is_compared() -> TestResult {
        let checker = HandoffChecker::new()?;
        let mut value = handoff("retained");
        value["status"] = json!("done");
        let check = checker.check_value(value);
        assert!(!check.is_valid());
        for request in [
            HandoffFinishRequest::Check,
            HandoffFinishRequest::Retain,
            HandoffFinishRequest::Close,
        ] {
            assert_eq!(
                request.decide(&check),
                HandoffFinishDecision::Stop(HandoffFinishStop::Draft)
            );
        }
        Ok(())
    }

    #[test]
    fn the_stated_action_is_published_only_as_a_closed_word() -> TestResult {
        let checker = HandoffChecker::new()?;
        let check = checker.check_value(handoff("kept somewhere safe"));
        let lifecycle = HandoffFinishLifecycle::new(&check, HandoffFinishRequest::Retain);
        let published = serde_json::to_value(lifecycle)?;
        assert_eq!(
            published,
            json!({"stated": null, "requested": "retained", "agrees": false})
        );
        let check = checker.check_value(handoff("Retained"));
        let lifecycle = HandoffFinishLifecycle::new(&check, HandoffFinishRequest::Retain);
        assert_eq!(lifecycle.stated, Some(HandoffLifecycleAction::Retained));
        assert!(lifecycle.agrees);
        Ok(())
    }
}
