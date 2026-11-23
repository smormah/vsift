//! Starting a process detached from this one that inherits no handle of it but
//! its own null standard streams, through `CreateProcessW` with an explicit
//! inheritance list (amendment of 2026-10-27 to ADR 0026, known limit L-229).
//!
//! `std::process::Command` asks Windows to pass the new process every
//! inheritable handle of its caller. A background runner started that way
//! holds, among others, the caller's standard output when the caller's parent
//! gave it a pipe, and that parent then waits for end-of-file until the
//! runner ends. Here the one handle the process may inherit is a `NUL` handle
//! this module opens for it, named in `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
//!
//! Each call into Windows is one operation in its own block, with the
//! invariants it relies on written beside it.

use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io,
    marker::PhantomData,
    os::windows::{
        ffi::OsStrExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
        process::ExitStatusExt as _,
    },
    path::Path,
    process::ExitStatus,
    ptr,
};

use windows_sys::Win32::{
    Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CreateProcessW,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, STARTUPINFOW, TerminateProcess, UpdateProcThreadAttribute,
        WaitForSingleObject,
    },
};

use crate::{
    command_line::{command_line, is_batch_file},
    create::nul_terminated,
};

/// Whether the new process leaves the job object its caller runs in.
///
/// A terminal or an IDE may end its job object, and every process in it, when
/// it closes; a runner that left it survives that. A job object can forbid
/// leaving it, and then starting a process that asks to leave fails with
/// `ERROR_ACCESS_DENIED` (known limit L-220).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallersJob {
    /// Ask to leave it (`CREATE_BREAKAWAY_FROM_JOB`).
    Leave,
    /// Stay in it.
    Stay,
}

/// A process started by [`start_detached`].
///
/// It owns the process handle, which is closed when this drops. Dropping it
/// never ends the process. It answers what `std::process::Child` answers for
/// a child started without pipes.
#[derive(Debug)]
pub struct DetachedProcess {
    process: OwnedHandle,
    id: u32,
    /// The exit status, once seen: a process handle keeps the status, but
    /// asking for it again is a call that need not be made.
    status: Option<ExitStatus>,
}

impl DetachedProcess {
    /// The operating system's identifier of the process.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// The exit status, if the process has ended, without waiting.
    ///
    /// # Errors
    ///
    /// The operating system's failure to query the process.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() && self.wait_for(0)? {
            self.status = Some(self.exit_status()?);
        }
        Ok(self.status)
    }

    /// Waits until the process has ended and returns its exit status.
    ///
    /// # Errors
    ///
    /// The operating system's failure to wait for or query the process.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        if !self.wait_for(INFINITE)? {
            return Err(io::Error::other(
                "a process wait without a timeout timed out",
            ));
        }
        let status = self.exit_status()?;
        self.status = Some(status);
        Ok(status)
    }

    /// Ends the process, if it is still running, with exit code 1, as
    /// `std::process::Child::kill` does. It does not wait for the end; call
    /// [`Self::wait`] for that.
    ///
    /// # Errors
    ///
    /// The operating system's failure to end a process that is still running.
    pub fn kill(&mut self) -> io::Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        #[allow(
            unsafe_code,
            reason = "ADR 0026 amendment (L-229): ending the runner that did not take its job"
        )]
        // SAFETY: `self.process` is the open process handle `CreateProcessW`
        // returned, owned by `self` and alive for the call; it has every
        // access right, `PROCESS_TERMINATE` among them. `TerminateProcess`
        // only reads it.
        let ended = unsafe { TerminateProcess(self.process.as_raw_handle(), 1) };
        if ended == 0 {
            let error = io::Error::last_os_error();
            // It may have ended by itself in between, which Windows reports
            // as a refusal.
            if self.try_wait()?.is_some() {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }

    /// Waits up to `milliseconds` (`INFINITE` for no limit) for the process
    /// to end, and says whether it has.
    fn wait_for(&self, milliseconds: u32) -> io::Result<bool> {
        #[allow(
            unsafe_code,
            reason = "ADR 0026 amendment (L-229): whether the runner has ended"
        )]
        // SAFETY: `self.process` is the open process handle `CreateProcessW`
        // returned, owned by `self` and alive for the call; a process handle
        // with `SYNCHRONIZE` access is a waitable object. The call only reads
        // it.
        let waited = unsafe { WaitForSingleObject(self.process.as_raw_handle(), milliseconds) };
        match waited {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// The exit code of a process that has ended, as an exit status.
    fn exit_status(&self) -> io::Result<ExitStatus> {
        let mut code = 0_u32;
        #[allow(
            unsafe_code,
            reason = "ADR 0026 amendment (L-229): the ended runner's exit code"
        )]
        // SAFETY: `self.process` is the open process handle `CreateProcessW`
        // returned, owned by `self` and alive for the call, with
        // `PROCESS_QUERY_INFORMATION` access. `code` is a valid, writable
        // `u32` on this frame. Called only after the process has ended, so
        // the code is not `STILL_ACTIVE`'s placeholder.
        let read = unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &raw mut code) };
        if read == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ExitStatus::from_raw(code))
    }
}

/// Starts `executable` with `arguments`, detached from this process, and
/// returns it.
///
/// - `executable` is passed to Windows by its absolute path
///   (`lpApplicationName`): nothing is looked up on `PATH` or in the current
///   folder. The command line is quoted as `std::process::Command` quotes it.
/// - The working folder is `working_directory`; the environment is this
///   process's, as `Command` gives it when none is set.
/// - The process has a hidden console of its own (`CREATE_NO_WINDOW`) and a
///   process group of its own (`CREATE_NEW_PROCESS_GROUP`), and leaves its
///   caller's job object when `callers_job` says so.
/// - Its standard input, output and error are one handle to the `NUL`
///   device, and that handle is the only one it inherits: no other handle of
///   this process reaches it, whatever its inherit flag.
///
/// # Errors
///
/// `InvalidInput`, before anything is started, when `executable` is relative
/// or a batch file (which Windows would run through `cmd.exe`, with other
/// quoting rules), a value holds a NUL, or the program name holds a quote.
/// Otherwise the operating system's error: `ERROR_ACCESS_DENIED` when
/// `callers_job` is [`CallersJob::Leave`] and the caller's job object forbids
/// leaving it.
pub fn start_detached(
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
    callers_job: CallersJob,
) -> io::Result<DetachedProcess> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a detached process needs an absolute executable path",
        ));
    }
    if executable
        .file_name()
        .is_some_and(|name| is_batch_file(&name.to_string_lossy()))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a batch file cannot be started as a detached process",
        ));
    }
    let application = nul_terminated(executable.as_os_str())?;
    let mut line = command_line(
        executable.as_os_str().encode_wide(),
        arguments.iter().map(|argument| argument.encode_wide()),
    )?;
    line.push(0);
    let directory = nul_terminated(working_directory.as_os_str())?;

    let null = inheritable_null()?;
    let inherited: [HANDLE; 1] = [null.as_raw_handle()];
    let list = AttributeList::inheriting_only(&inherited)?;
    let startup = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: u32::try_from(size_of::<STARTUPINFOEXW>())
                .map_err(|_| io::Error::other("STARTUPINFOEXW does not fit its length field"))?,
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: inherited[0],
            hStdOutput: inherited[0],
            hStdError: inherited[0],
            ..STARTUPINFOW::default()
        },
        lpAttributeList: list.as_ptr(),
    };
    let flags = CREATE_NO_WINDOW
        | CREATE_NEW_PROCESS_GROUP
        | EXTENDED_STARTUPINFO_PRESENT
        | match callers_job {
            CallersJob::Leave => CREATE_BREAKAWAY_FROM_JOB,
            CallersJob::Stay => 0,
        };
    let mut information = PROCESS_INFORMATION::default();
    #[allow(
        unsafe_code,
        reason = "ADR 0026 amendment (L-229): the one call that starts the runner"
    )]
    // SAFETY: `application` and `directory` are NUL-terminated UTF-16 strings
    // without an interior NUL, and `line` is a NUL-terminated, writable
    // UTF-16 buffer (`CreateProcessW` may change it in place), all owned by
    // this frame and alive for the call. No security attributes and no
    // environment block are passed (null: the defaults and this process's
    // environment). `startup` is a fully initialised `STARTUPINFOEXW` whose
    // `cb` is its own size, read as one through the pointer to its first
    // field, which `EXTENDED_STARTUPINFO_PRESENT` declares; its standard
    // handles are the inheritable `null`, alive until after the call, and its
    // attribute list is `list`, initialised and alive until after the call,
    // whose handle list points at `inherited`, which `list` borrows.
    // `information` is a valid, writable `PROCESS_INFORMATION` on this frame.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            flags,
            ptr::null(),
            directory.as_ptr(),
            (&raw const startup).cast::<STARTUPINFOW>(),
            &raw mut information,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    let thread = adopt(information.hThread);
    let process = adopt(information.hProcess);
    // The thread's handle is not needed; closing it does not end the thread.
    drop(thread);
    drop(list);
    drop(null);
    Ok(DetachedProcess {
        process: process?,
        id: information.dwProcessId,
        status: None,
    })
}

/// A handle to the `NUL` device, opened by the standard library and marked
/// inheritable, so that it can be named in an inheritance list.
///
/// Only this handle's flag changes, never one of the caller's. While it is
/// inheritable, a process another thread starts with the standard library
/// can inherit it too, which gives that process a handle to `NUL` and
/// nothing else: it holds no pipe and no file open.
fn inheritable_null() -> io::Result<File> {
    let null = OpenOptions::new()
        .read(true)
        .write(true)
        .open(Path::new(r"\\.\NUL"))?;
    #[allow(
        unsafe_code,
        reason = "ADR 0026 amendment (L-229): the runner's null handle made inheritable"
    )]
    // SAFETY: `null` is an open file handle owned by this frame and alive for
    // the call. `SetHandleInformation` changes only its inherit flag, which
    // nothing else in this process relies on.
    let marked = unsafe {
        SetHandleInformation(
            null.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        )
    };
    if marked == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(null)
}

/// Takes ownership of a handle `CreateProcessW` returned, so that it is
/// closed exactly once, when the `OwnedHandle` drops.
fn adopt(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::other(
            "Windows returned no process or thread handle",
        ));
    }
    #[allow(
        unsafe_code,
        reason = "ADR 0026 amendment (L-229): owning a handle CreateProcessW returned"
    )]
    // SAFETY: `handle` is a non-null process or thread handle that
    // `CreateProcessW` opened for this process and that nothing else owns or
    // closes: `start_detached` adopts each of the two exactly once and never
    // uses the raw value again.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle) };
    Ok(owned)
}

/// A process-thread attribute list holding one attribute: the handles a new
/// process may inherit. Deleted when it drops.
///
/// It borrows the handle array for its whole life, because Windows keeps a
/// pointer to it and reads it when the process is created.
struct AttributeList<'h> {
    /// The list, inside `_buffer`.
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
    /// The list's memory, sized by Windows and aligned for pointers. It is
    /// never resized, so `list` stays valid while this lives.
    _buffer: Vec<usize>,
    _handles: PhantomData<&'h [HANDLE]>,
}

impl<'h> AttributeList<'h> {
    /// A list whose only attribute names `handles` as the only handles a new
    /// process inherits.
    fn inheriting_only(handles: &'h [HANDLE; 1]) -> io::Result<Self> {
        let mut size = 0_usize;
        #[allow(
            unsafe_code,
            reason = "ADR 0026 amendment (L-229): the size of a list of one attribute"
        )]
        // SAFETY: a null list with a valid, writable `size` on this frame is
        // the documented way to ask for the size a list of one attribute
        // needs; the call writes only `size` (and fails, as documented, with
        // `ERROR_INSUFFICIENT_BUFFER`).
        let _ = unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_usize; size.div_ceil(size_of::<usize>())];
        let list: LPPROC_THREAD_ATTRIBUTE_LIST = buffer.as_mut_ptr().cast();
        #[allow(
            unsafe_code,
            reason = "ADR 0026 amendment (L-229): initialising the attribute list"
        )]
        // SAFETY: `list` points at `buffer`, which is writable, aligned for
        // pointers and at least `size` bytes, the size Windows asked for one
        // attribute; `size` is a valid, writable `usize` on this frame.
        let initialised = unsafe { InitializeProcThreadAttributeList(list, 1, 0, &raw mut size) };
        if initialised == 0 {
            return Err(io::Error::last_os_error());
        }
        // From here `Drop` deletes the list, also on the error below.
        let this = Self {
            list,
            _buffer: buffer,
            _handles: PhantomData,
        };
        let attribute = usize::try_from(PROC_THREAD_ATTRIBUTE_HANDLE_LIST)
            .map_err(|_| io::Error::other("the attribute identifier does not fit"))?;
        #[allow(
            unsafe_code,
            reason = "ADR 0026 amendment (L-229): naming the only inherited handle"
        )]
        // SAFETY: `this.list` is an initialised list with room for one
        // attribute, and this is its first. The value is `handles`, an array
        // of open, inheritable handles whose size in bytes is passed with it;
        // `this` borrows it for `'h`, so it outlives every use Windows makes
        // of the pointer. No previous value or returned size is asked for.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                this.list,
                0,
                attribute,
                handles.as_ptr().cast(),
                size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(this)
    }

    const fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.list
    }
}

impl Drop for AttributeList<'_> {
    fn drop(&mut self) {
        #[allow(
            unsafe_code,
            reason = "ADR 0026 amendment (L-229): deleting the attribute list"
        )]
        // SAFETY: `self.list` was initialised by
        // `InitializeProcThreadAttributeList` (the only constructor returns a
        // value only after that succeeded) and its buffer is still alive;
        // `Self` is neither `Clone` nor `Copy`, so it is deleted once.
        unsafe {
            DeleteProcThreadAttributeList(self.list);
        };
    }
}

#[cfg(test)]
mod tests;
