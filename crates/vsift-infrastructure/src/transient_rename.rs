//! A rename that Windows refuses for a moment is tried again, within a bound
//! (#439).
//!
//! Windows refuses to rename a folder while any file inside it is open, with
//! `ERROR_ACCESS_DENIED` whatever sharing that handle allows, and refuses to
//! replace a file that another handle holds without delete sharing, with
//! `ERROR_SHARING_VIOLATION`. A scanner or an indexer opens a file a moment
//! after it is written, so the managed store's renames, made just after the
//! runtime's files were copied and run and the selection pointer was written,
//! can meet either refusal for a healthy store. Both end when the other handle
//! closes. A refusal that outlasts the bound is the same failure it was before,
//! so a real permission problem is still reported, later by at most the bound.
//! A POSIX rename is never refused this way and is not tried again.

use std::{
    io, thread,
    time::{Duration, Instant},
};

/// How long a refused rename is tried again before its refusal is final.
/// A scanner holds a new file for milliseconds; the bound absorbs a busy
/// machine and stays well inside the time a command spends on activation.
pub(crate) const TRANSIENT_RENAME_WAIT: Duration = Duration::from_secs(2);
/// The first pause between attempts; it doubles up to [`MAX_BACKOFF`].
const INITIAL_BACKOFF: Duration = Duration::from_millis(5);
/// The longest pause between attempts.
const MAX_BACKOFF: Duration = Duration::from_millis(100);

/// Win32 `ERROR_ACCESS_DENIED`: how a folder rename is refused while a file
/// inside the folder is open.
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;
/// Win32 `ERROR_SHARING_VIOLATION`: how a rename over a file is refused while
/// another handle holds that file without delete sharing.
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// Whether a failed rename is one Windows ends by itself when another
/// handle closes.
#[cfg(windows)]
pub(crate) fn is_transient_rename_refusal(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION)
    )
}

/// A POSIX rename does not depend on other open handles, so none of its
/// failures ends by itself.
#[cfg(not(windows))]
pub(crate) const fn is_transient_rename_refusal(_error: &io::Error) -> bool {
    false
}

/// Runs `rename` until it succeeds, fails in a way that is not transient, or
/// has been refused for `wait`; returns the last outcome.
///
/// `rename` must leave nothing changed when it fails, as a rename does, so
/// another attempt starts from the same state.
pub(crate) fn rename_retrying_transient_refusals(
    wait: Duration,
    is_transient: impl Fn(&io::Error) -> bool,
    mut rename: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let started = Instant::now();
    let mut backoff = INITIAL_BACKOFF;
    loop {
        match rename() {
            Err(error) if is_transient(&error) && started.elapsed() < wait => {
                thread::sleep(backoff);
                backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
            }
            outcome => return outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        io,
        time::{Duration, Instant},
    };

    use super::{is_transient_rename_refusal, rename_retrying_transient_refusals};

    fn refused() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    fn is_refused(error: &io::Error) -> bool {
        error.kind() == io::ErrorKind::PermissionDenied
    }

    #[test]
    fn a_rename_refused_for_a_moment_is_tried_again_until_it_succeeds() {
        let attempts = Cell::new(0_u32);
        let outcome =
            rename_retrying_transient_refusals(Duration::from_secs(2), is_refused, || {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 3 {
                    Err(refused())
                } else {
                    Ok(())
                }
            });
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn a_refusal_that_outlasts_the_bound_is_returned() {
        let attempts = Cell::new(0_u32);
        let wait = Duration::from_millis(60);
        let started = Instant::now();
        let outcome = rename_retrying_transient_refusals(wait, is_refused, || {
            attempts.set(attempts.get() + 1);
            Err(refused())
        });
        assert!(matches!(outcome, Err(error) if is_refused(&error)));
        assert!(started.elapsed() >= wait);
        assert!(attempts.get() > 1);
    }

    #[test]
    fn any_other_failure_is_returned_at_once() {
        let attempts = Cell::new(0_u32);
        let outcome =
            rename_retrying_transient_refusals(Duration::from_secs(2), is_refused, || {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from(io::ErrorKind::NotFound))
            });
        assert!(matches!(outcome, Err(error) if error.kind() == io::ErrorKind::NotFound));
        assert_eq!(attempts.get(), 1);
    }

    /// Only the two refusals Windows ends by itself are transient, and on
    /// Unix none is.
    #[test]
    fn only_the_windows_refusals_of_an_open_handle_are_transient() {
        let access_denied = io::Error::from_raw_os_error(5);
        let sharing_violation = io::Error::from_raw_os_error(32);
        let other = io::Error::from_raw_os_error(2);
        assert_eq!(is_transient_rename_refusal(&access_denied), cfg!(windows));
        assert_eq!(
            is_transient_rename_refusal(&sharing_violation),
            cfg!(windows)
        );
        assert!(!is_transient_rename_refusal(&other));
    }
}
