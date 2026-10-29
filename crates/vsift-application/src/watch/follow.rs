//! How the `watch` use case follows the `prepare` job while it runs, without
//! an async runtime (ADR 0037 sections 3 and 7, the planner's call 1 of
//! 2026-10-29).
//!
//! The engine invokes the run's [`WatchProgress`] callback on the job's own
//! task. The callback only records what happened in a [`WatchSignal`] and
//! wakes its follower: it never writes, waits or reads storage, so the job
//! is never held by it. The follower (a second future, polled beside the job
//! by [`join`] on the same task) writes the snapshots and the frames. The
//! application has no runtime, so both are plain `std` futures.

use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::Mutex,
    task::{Poll, Waker},
};

use super::WatchProgress;

/// What happened since the follower last looked.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Notice {
    /// A chunk of speech finished, so a part may have been published.
    pub(super) published: bool,
    /// The visual stream has indexed the whole video.
    pub(super) indexed: bool,
}

#[derive(Debug, Default)]
struct SignalState {
    pending: Notice,
    closed: bool,
    waker: Option<Waker>,
}

/// The run's notices, from the callback to the follower.
#[derive(Debug, Default)]
pub(super) struct WatchSignal {
    state: Mutex<SignalState>,
}

impl WatchSignal {
    fn change(&self, change: impl FnOnce(&mut SignalState)) {
        // A poisoned lock means a follower panicked; the job goes on unheard.
        if let Ok(mut state) = self.state.lock() {
            change(&mut state);
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
    }

    /// No more notices: the job has ended. The follower takes what is
    /// pending, then ends.
    pub(super) fn close(&self) {
        self.change(|state| state.closed = true);
    }

    /// The notices pending since the last call, waiting for one; `None` once
    /// the signal is closed and nothing is pending.
    pub(super) async fn next(&self) -> Option<Notice> {
        poll_fn(|context| {
            let Ok(mut state) = self.state.lock() else {
                return Poll::Ready(None);
            };
            let pending = std::mem::take(&mut state.pending);
            if pending != Notice::default() {
                return Poll::Ready(Some(pending));
            }
            if state.closed {
                return Poll::Ready(None);
            }
            state.waker = Some(context.waker().clone());
            Poll::Pending
        })
        .await
    }
}

impl WatchProgress for WatchSignal {
    fn published(&self) {
        self.change(|state| state.pending.published = true);
    }

    fn visual_indexed(&self) {
        self.change(|state| state.pending.indexed = true);
    }
}

/// Runs both futures on the caller's task to their ends; their outputs.
pub(super) async fn join<A: Future, B: Future>(first: A, second: B) -> (A::Output, B::Output) {
    let mut first = pin!(first);
    let mut second = pin!(second);
    let mut first_output = None;
    let mut second_output = None;
    poll_fn(|context| {
        if first_output.is_none()
            && let Poll::Ready(output) = first.as_mut().poll(context)
        {
            first_output = Some(output);
        }
        if second_output.is_none()
            && let Poll::Ready(output) = second.as_mut().poll(context)
        {
            second_output = Some(output);
        }
        match (first_output.take(), second_output.take()) {
            (Some(first), Some(second)) => Poll::Ready((first, second)),
            (first, second) => {
                first_output = first;
                second_output = second;
                Poll::Pending
            }
        }
    })
    .await
}
