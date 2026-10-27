//! The one crate where `VSift` calls Windows directly, and so the one crate
//! with `unsafe` code ([ADR 0026](https://github.com/smormah/vsift/blob/main/docs/decisions/0026-windows-directories-created-with-their-security-descriptor.md)).
//!
//! `VSift` makes every directory it creates for itself private to the current
//! user. On Unix the mode does that at creation. On Windows it must be the
//! directory's security descriptor, and the standard library, `cap-std` and the
//! `windows-acl` crate can only change a DACL **after** the directory exists,
//! by a sequence of path-based writes. Each such write also makes Windows
//! rewrite the DACLs of the folders inside, without coordinating with anyone
//! else changing them; under concurrent first use that left a new session root
//! with no entry for its own user (#206, #443). Passing the descriptor to
//! `CreateDirectoryW` makes the directory private from its first instant, so
//! nothing has to write its permissions afterwards.
//!
//! The crate exposes one safe function, `create_directory` (Windows only),
//! and the descriptor it takes, [`PrivateDirectoryDescriptor`] (every
//! platform, plain Rust, so it is tested everywhere).
//!
//! Its amendment of 2026-10-27 (known limit L-229) adds a second,
//! `start_detached` (Windows only), with the process it returns,
//! `DetachedProcess`, and its one choice, `CallersJob`. It starts the
//! background runner of `job start` so that the runner inherits no handle of
//! its caller but its own null standard streams, through an explicit
//! inheritance list: the standard library passes a new process every
//! inheritable handle, and a runner that held its caller's output pipe kept
//! the caller's parent waiting for end-of-file until the transcription ended.
//! Its command-line quoting is plain Rust, tested everywhere.
//!
//! The `unsafe` blocks are thirteen, each one operation with a `// SAFETY:`
//! comment; the lints deny any other, and the governance check bounds their
//! number and refuses `unsafe` in every other crate. Any change here needs
//! the review ADR 0026 sets out.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

#[cfg(any(windows, test))]
mod command_line;
#[cfg(windows)]
mod create;
mod descriptor;
#[cfg(windows)]
mod detached;

#[cfg(windows)]
pub use create::create_directory;
pub use descriptor::{ADMINISTRATORS_SID, DescriptorError, PrivateDirectoryDescriptor, SYSTEM_SID};
#[cfg(windows)]
pub use detached::{CallersJob, DetachedProcess, start_detached};
