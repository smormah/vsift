//! Progress of a long engine operation, for hosts that show it (P11, ADR 0021).
//!
//! A host that wants to see a long operation advance (the CLI's
//! `--events jsonl`, a worker's supervisor) passes a [`ProgressObserver`] in
//! the request. The engine calls it on the operation's own task between
//! units of work, so the callback must return at once: a host queues the
//! observation, and drops it when its queue is full, instead of writing it
//! out there. Progress is advisory; the operation's result stays the only
//! authority on what happened.

use std::{fmt, sync::Arc};

use vsift_application::ProgressSink;
use vsift_domain::{JobId, ProgressUpdate};

/// One observation a host receives: the update and the job doing the work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobProgress {
    /// The recoverable job behind the work, when it runs as one.
    pub job: Option<JobId>,
    /// How far the work has come.
    pub update: ProgressUpdate,
}

/// The callback a [`ProgressObserver`] holds.
type ProgressCallback = dyn Fn(&JobProgress) + Send + Sync;

/// Where an operation reports its progress: a host's callback, or nowhere.
///
/// Cheap to clone; every clone reports to the same callback.
#[derive(Clone, Default)]
pub struct ProgressObserver(Option<Arc<ProgressCallback>>);

impl ProgressObserver {
    /// Reports nowhere: the default for hosts that do not show progress.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    /// Reports every observation to `callback`, which must not block.
    #[must_use]
    pub fn new(callback: impl Fn(&JobProgress) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(callback)))
    }

    /// Reports one observation to the callback, if there is one. The
    /// engine's operations call it; a host may too, for example to test the
    /// callback it installed.
    pub fn report(&self, progress: &JobProgress) {
        if let Some(callback) = &self.0 {
            callback(progress);
        }
    }

    /// The application port for the work of `job`.
    pub(crate) fn for_job(&self, job: &JobId) -> JobProgressSink<'_> {
        JobProgressSink {
            observer: self,
            job: job.clone(),
        }
    }
}

impl fmt::Debug for ProgressObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.0.is_some() {
            "ProgressObserver(callback)"
        } else {
            "ProgressObserver(none)"
        })
    }
}

/// A [`ProgressObserver`] as the application's port, naming one job.
pub(crate) struct JobProgressSink<'observer> {
    observer: &'observer ProgressObserver,
    job: JobId,
}

impl ProgressSink for JobProgressSink<'_> {
    fn report(&self, update: ProgressUpdate) {
        if self.observer.0.is_some() {
            self.observer.report(&JobProgress {
                job: Some(self.job.clone()),
                update,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use vsift_application::ProgressSink;
    use vsift_domain::{JobId, ProgressStage, ProgressUpdate};

    use super::{JobProgress, ProgressObserver};

    #[test]
    fn a_job_sink_reports_to_the_callback_with_its_job() -> Result<(), Box<dyn std::error::Error>> {
        let seen: Arc<Mutex<Vec<JobProgress>>> = Arc::default();
        let recorder = Arc::clone(&seen);
        let observer = ProgressObserver::new(move |progress| {
            if let Ok(mut seen) = recorder.lock() {
                seen.push(progress.clone());
            }
        });
        let job = JobId::parse("job_0123456789abcdef0123456789abcdef")?;
        let update = ProgressUpdate {
            stage: ProgressStage::RecognisingSpeech,
            completed: 1,
            total: Some(2),
        };
        observer.for_job(&job).report(update);
        ProgressObserver::none().for_job(&job).report(update);

        let seen = seen.lock().map_err(|_| "poisoned")?;
        assert_eq!(
            *seen,
            [JobProgress {
                job: Some(job),
                update
            }]
        );
        assert_eq!(format!("{observer:?}"), "ProgressObserver(callback)");
        Ok(())
    }
}
