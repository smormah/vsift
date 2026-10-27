# ADR 0026: Windows directories created with their security descriptor (the one `unsafe` crate)

- Status: **Accepted, 2026-10-19**, by the maintainer's decision: that day the maintainer chose
  this design, option 1 of #443 ("an isolated `unsafe` crate"), recorded on
  [#443](https://github.com/smormah/vsift/issues/443). This is the narrow exception that
  `AGENTS.md` allows to its rule that `unsafe` is forbidden in VSift crates.
- Date: 2026-10-19
- Tracking: [#443](https://github.com/smormah/vsift/issues/443) (with
  [#206](https://github.com/smormah/vsift/issues/206))
- Refines: the private-root rule of the [threat model](../planning/security-threat-model.md)
  (SEC-18, 2026-09-24) and the [P05 record](../planning/p05-session-qualification.md)
  "private roots under a permissive parent"; ends known limit L-005 and changes L-175.

## Context

Every directory VSift creates for itself (the per-user configuration folder and its missing
ancestors, the session root and its created parent, each retained bundle, the managed-data
folder) must be private to the current user whatever its parent grants. On Unix the creation
mode does that. On Windows, from 2026-09-24 until this ADR, the directory was created with the
standard library (inheriting its parent's entries) and its DACL was then replaced through
`windows-acl` by a sequence of path-based writes: three additions, the removals, a read-back,
and a repeat of the sequence when the read-back was not private (#206).

**What went wrong (#443, shown).** A path-based DACL write through `SetNamedSecurityInfoW`
makes Windows rewrite the DACL of every folder inside the target, from what it read of each one
a moment before, without coordinating with anyone else changing them. When several processes
used a new session root for the first time at once, one created the missing parent (`cache`)
and restricted it while another had already created the root inside it and was restricting the
root. The instrumented race (4,000 runs of `concurrent_processes_create_one_root_and_every_one_adopts_it`
under load, logging every DACL step; 2026-10-18) showed the parent's writes landing inside the
root's own restriction in 2,857 runs; in 9 the root's DACL was taken back to its inherited
entries between its additions and its removals, the removal of the user's inherited copy then
deleted the user's only entry, and every racer that opened the root in the next ~20 ms was
refused `STORAGE_IO` ("session storage root is unavailable"); the creator's second pass
repaired those 9. When such a write lands after the creator's last read-back, the root keeps a
DACL with only `SYSTEM` and Administrators, which no later command can open: one such root,
left by a reproduction on 2026-10-17, is kept on the maintainer's desktop (L-175).

**Why the standard library cannot do it.** The only way to give a Windows directory its DACL
as it comes into being is to pass a security descriptor to `CreateDirectoryW` (or
`NtCreateFile`). `std::fs::DirBuilder`, `cap-std` and `windows-acl` expose no such parameter:
they create first and change the DACL afterwards, which is the sequence that failed. The
`windows` and `windows-sys` crates bind `CreateDirectoryW`, but every binding is an `unsafe fn`.
No crate in `Cargo.lock` offers a safe wrapper.

## Decision

1. **One isolated crate holds VSift's only `unsafe` code:**
   [`crates/vsift-windows-security`](../../crates/vsift-windows-security). It exposes one safe
   function, `create_directory(path, &PrivateDirectoryDescriptor)` (Windows only), and the
   descriptor type (every platform, plain Rust, tested everywhere).
   - The descriptor is built only from a validated string SID (`S-1-<authority>-<sub>...`,
     decimal, bounded; anything else is refused before it reaches the SDDL text):
     `O:<user>D:P(A;OICI;FA;;;<user>)(A;OICI;FA;;;S-1-5-18)(A;OICI;FA;;;S-1-5-32-544)`: owned by
     the current user, a **protected** DACL (nothing inherited from the parent), full control
     inherited by everything created inside, for the user, `LocalSystem` and Administrators.
     These are exactly the entries the earlier restriction ended with and the principals the
     validator trusts.
   - Three `unsafe` blocks, each one operation with a `// SAFETY:` comment naming every
     invariant: `ConvertStringSecurityDescriptorToSecurityDescriptorW` (NUL-terminated input,
     valid out-pointer), `CreateDirectoryW` (NUL-terminated path without an interior NUL, a fully
     initialised `SECURITY_ATTRIBUTES` whose descriptor outlives the call), and `LocalFree` in
     the descriptor's `Drop` (the one owner of the pointer Windows allocated; freed once).
   - The dependency is `windows-sys` 0.61.2, already in the lock through `fs4`, `process-wrap`,
     `tokio` and others, with four features; no new crate is downloaded or locked.
2. **Every directory VSift creates as private on Windows is created this way**
   (`private_user_root::create_private_child`), at every level it creates, and **nothing writes a
   new directory's permissions afterwards**. The post-creation validation still runs, and existing
   directories are never changed: their checks (and the short wait for a creator of an earlier
   version) stay as they were.
3. **A session root that has lost its user's own entry is refused, never repaired.** Its DACL
   is still readable by its owner, so VSift tells it apart (`SessionRootError::OwnAccessMissing`)
   and answers `STORAGE_IO`, the code the case had, with a remediation that says how to recover
   the folder (the `icacls` grant of the user's entry, or deleting the folder). VSift cannot tell
   who removed the entry, so it does not give it back itself.
4. **The rule is enforced.** The workspace keeps `unsafe_code = "forbid"`; every other package
   takes the workspace's lints, and its `src/lib.rs` and `src/main.rs` carry
   `#![forbid(unsafe_code)]` (for the crates under `crates/`; the tools rely on the workspace
   lint, which no target can relax). The exception crate repeats the workspace's lints with
   `unsafe_code = "deny"` (allowed only at each block), `unsafe_op_in_unsafe_fn = "deny"`, and
   Clippy's `undocumented_unsafe_blocks` and `multiple_unsafe_ops_per_block` denied, and its root
   carries `#![deny(unsafe_op_in_unsafe_fn)]` and `#![deny(clippy::undocumented_unsafe_blocks)]`.
   The governance check (`tools/vsift-governance/src/unsafe_code.rs`) refuses the `unsafe`
   keyword in any Rust source of `crates/`, `tools/` or `fuzz/` outside the exception crate,
   refuses any of the lint settings above being dropped, requires a `// SAFETY:` comment before
   every `unsafe` in the crate, and bounds them at three. Its tests feed broken copies of the real
   files.

## The review rule for any change to the crate

A change to `crates/vsift-windows-security` (its code, its manifest, or the bound in the
governance check) is a security change: it needs the maintainer's review, names this ADR in its
description, keeps each `unsafe` block to one operation with its `// SAFETY:` comment, adds no
function that is not needed by a decision recorded here (a new one is a note on this ADR), and
adds no dependency beyond `windows-sys`. Raising the bound of three is such a change.

## Alternatives rejected

- **Create under a temporary name, restrict, then rename into place** (option 2 of #443). No
  `unsafe`, and the final name would only appear once private. Rejected: `std::fs::rename`
  replaces an existing **empty** directory on Windows (shown on 2026-10-18: an empty directory
  renamed onto another empty one succeeded, onto a non-empty one failed with error 145), so
  choosing one creator among racers would rest on subtle replace semantics; a crash between the
  steps leaves temporary folders to sweep; and the restriction's path-based writes, the cause of
  #443, would remain (only their timing would change).
- **Let racers wait on an access-denied root** (option 3, the stop-gap). It would hide the
  transient refusals the instrumented runs showed, but not the permanent state, and it would add
  a wait for a state VSift itself created.
- **Keep the restriction and accept the race** (L-175 as it stood): a new session root could be
  left unusable for every later command, with a remediation that did not say why.
- **A third-party safe wrapper.** None in the lock; `windows-permissions` 0.2.4 was found but
  not reviewed, and a new dependency for three calls would be a larger review than the calls.

## Consequences

- No moment exists in which a directory VSift creates on Windows has its parent's entries:
  L-005's window (a principal the parent trusted could open a handle to the still-empty folder)
  is gone, and no later DACL write by VSift can reach a folder another VSift process is creating.
  L-005 is deleted; L-175 says what remains (roots already broken by earlier versions, which the
  new refusal names, and the releases up to `0.2.2`, which keep the race).
- The old restriction (`restrict_new_directory`, its read-back and repeat, and its model tests)
  is removed; the validators and the wait for an earlier version's creator stay.
- VSift has one crate with `unsafe` code, under the review rule above.

## Verification

- `vsift-windows-security`: the descriptor's tests (every platform: the exact SDDL, protection,
  the SIDs accepted and refused), and on Windows the call's tests (the stored DACL is exactly the
  three explicit entries under a parent that grants `BUILTIN\Users`, a child inherits only them,
  a second creation is `AlreadyExists`, a missing parent, a relative path and a NUL are refused).
- `private_user_root`: `a_late_parent_write_cannot_take_a_new_folder_back_to_an_earlier_state`
  makes #443's late propagation on demand; it fails with the earlier create-then-restrict order
  (`BUILTIN\Users` comes back) and passes with this one.
  `a_folder_without_the_users_own_entry_is_recognised` and the CLI's
  `session_root_own_access_cli_contract` pin the refusal and its remediation, and that VSift
  leaves the folder unchanged.
- The governance check's broken-copy tests.
- Load: the race test's rate before and after is to be measured on a separate machine (the pull
  request lists the command and the load shape); the deterministic tests above are the
  regression evidence.

## Amendment, 2026-10-27: a second function, starting the background runner with only the handles it may inherit (L-229)

The accepted text above stands unchanged. This note adds one function to the crate, under the review
rule above, for one recorded decision.

**Status of this note:** checked by the planner on 2026-10-27 against the review rule above
(accepted, with the two conditions on the environment and on a job object that forbids leaving it,
covered below); the maintainer reviews the pull request. L-229 is deleted from the register by the
same change.

**The decision it serves.** On 2026-10-23 the maintainer decided that known limit
L-229 is fixed with one narrowly scoped function in this crate
that starts a process with an explicit list of the handles it may inherit, recorded as an amendment
to this ADR, and not with a third-party crate or by shipping the limit. L-229: `launch_detached`
(`crates/vsift-infrastructure/src/detached_runner.rs`, [ADR 0033](0033-progress-during-long-transcriptions.md)
section 5) starts `job start`'s background runner with its standard streams set to null, but the
standard library's `CreateProcessW` call asks for handle inheritance without a list, so Windows
passes the runner every inheritable handle of its caller. When the caller's own standard output and
error are pipes, as they are for an AI assistant's tool call, a test harness or a script's command
substitution, the runner holds the write end of the pipe its caller's parent reads, and that parent
waits for end-of-file until the transcription ends. Unix is not affected.

**The function.** `start_detached(executable, arguments, working_directory, job_object)` (Windows
only), returning a `DetachedProcess`:

- It starts `executable`, by its absolute path given as `lpApplicationName` (no search of `PATH` or
  of the current folder), with `arguments` quoted as the standard library quotes them (the program
  name always in quotes; an argument in quotes when it is empty or holds a space or a tab; a run of
  backslashes doubled before a quote or before the closing quote; a quote escaped). The quoting is
  plain Rust and tested on every platform. A NUL in any value, a relative path, a quote in the
  program name, and a `.bat` or `.cmd` executable (which Windows would run through `cmd.exe`, with
  other quoting rules) are refused with `InvalidInput` before anything is started.
- Its working folder is `working_directory`, given explicitly as `lpCurrentDirectory`, and its
  environment is inherited (`lpEnvironment` null), which is exactly what `std::process::Command`
  gave the runner before (it passes no environment block when none is set). So the runner sees the
  same folder and environment on every platform as before: Unix keeps the standard library.
- Its creation flags are fixed inside the crate: `CREATE_NO_WINDOW`, `CREATE_NEW_PROCESS_GROUP` and
  `EXTENDED_STARTUPINFO_PRESENT`, plus `CREATE_BREAKAWAY_FROM_JOB` when `job_object` is
  `JobObject::Leave` (`JobObject::Stay` leaves it out). The caller cannot pass any other flag. The
  infrastructure keeps today's order: leave the job, and on `ERROR_ACCESS_DENIED` start again
  inside it, reporting `in_callers_job` (L-220, unchanged).
- Its standard input, output and error are one handle to the `NUL` device, opened by the function
  with the standard library and marked inheritable, and **that handle is the only entry of the
  process's inheritance list** (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`). Windows passes the runner no
  other handle of the caller, whatever its inherit flag. The function closes its own copy of the
  `NUL` handle and the new thread's handle before it returns.
- `DetachedProcess` owns the process handle (closed when it drops; dropping never ends the process)
  and offers what `DetachedRunner` already promises, with the standard library's types: `id()`,
  `try_wait() -> io::Result<Option<ExitStatus>>`, `kill()` (ends a process that is still running)
  and `wait()`. No raw handle is ever returned.

**The Win32 calls,** each in its own `unsafe` block with its `// SAFETY:` comment: `SetHandleInformation`
(the function's own `NUL` handle becomes inheritable), `InitializeProcThreadAttributeList` twice
(the size of a list of one attribute, then the list in a buffer of that size, aligned for pointers),
`UpdateProcThreadAttribute` (the handle list; the one-element array it points to is borrowed by the
list, so it outlives every use), `CreateProcessW` (with `bInheritHandles` true, which the list then
restricts), `DeleteProcThreadAttributeList` (in the list's `Drop`, once), `OwnedHandle::from_raw_handle`
(one helper that takes ownership of the process and thread handles `CreateProcessW` returns, so the
standard library closes each exactly once), `WaitForSingleObject` (one helper, with a zero timeout for
`try_wait` and an infinite one for `wait`), `GetExitCodeProcess` and `TerminateProcess`. That is ten
new blocks.

**The bound.** The governance check's bound on `unsafe` blocks in this crate rises **from three to
thirteen**, by the ten above and no more; its broken-copy tests cover the new bound. The dependency
stays `windows-sys` 0.61.2, already in the lock; it gains the one feature `Win32_System_Threading`
(the process and attribute-list functions and types). No crate is added to the lock.

**The infrastructure side.** `launch_detached` keeps its signature and its promises (the handshake,
the owner lock, the kill on a handshake timeout, `detachment`). `DetachedRunner` holds a
`std::process::Child` on Unix, started as today, and a `DetachedProcess` on Windows; both answer the
same three calls (`try_wait`, `kill`, `wait`), so `vsift-infrastructure` stays free of `unsafe` and
the CLI does not change.

**Alternatives rejected.**

- **Clearing `HANDLE_FLAG_INHERIT` on the caller's standard handles before the launch** (the finding's
  second option). It changes process-wide state that belongs to the caller: a library host that later
  starts a child of its own, expecting its standard streams to be passed on, would find them not
  inherited; and another thread starting a process at the same moment would see the flags half
  changed. Restoring them afterwards cannot be made atomic. It would also cover only the three
  standard handles, not any other inheritable handle the host holds.
- **`bInheritHandles` false.** No handle at all would be inherited, with fewer calls. But the
  standard handles named in `STARTUPINFO` are then not valid in the runner, so the runner's standard
  streams would no longer be connected to nothing: they would be its hidden console's, a change for
  the runner and for every provider that takes its streams, and the maintainer's decision names an
  explicit list.
- **A third-party crate** that starts processes with a handle list: the maintainer's decision rules
  it out, and none in the lock offers it.

**Tests.**

- In this crate, on Windows (`src/detached/tests.rs`): a test process starts a stand-in caller (the
  test executable again) whose standard output is a pipe the test created, and so inheritable in the
  caller; the caller starts a long-lived child with `start_detached` and exits; the test sees
  end-of-file while the child is still alive. A control in the same shape, with the child started by
  the standard library, sees no end-of-file until the child ends, so the test can tell. A child
  started each way records its working folder, environment and arguments, and the two records are
  equal (the planner's first condition), with awkward arguments that exercise the quoting. `kill`,
  `wait` and `try_wait`, and the refusals, are covered too. The quoting and the batch-file rule run on
  every platform (`src/command_line.rs`).
- In `vsift-infrastructure` (`tests/detached_runner.rs`), through `launch_detached`: the same
  three-process shape on every platform, which waited for the runner's end on Windows before the fix
  (the evidence is in the pull request); and on Windows a caller started inside a job object that
  forbids leaving it (`process-wrap`'s, as the process supervisor uses) gets `in_callers_job`, is not
  held up, and its runner ends when the job is closed: L-220's documented behaviour (the planner's
  second condition).
- The CLI-level test of the defect (`job start` answering within seconds while a queued job's runner
  waits behind a live one) needs the queued mode, ADR 0033's fifth pull request (#512), and comes with
  whichever of the two lands second.
- The existing `job start`, runner and L-220 tests do not change.
