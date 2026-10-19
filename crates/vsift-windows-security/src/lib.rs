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
//! platform, plain Rust, so it is tested everywhere). Its `unsafe` blocks are
//! three, each one operation with a `// SAFETY:` comment; the lints deny any
//! other, and the governance check refuses `unsafe` in every other crate. Any
//! change here needs the review ADR 0026 sets out.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

#[cfg(windows)]
mod create;
mod descriptor;

#[cfg(windows)]
pub use create::create_directory;
pub use descriptor::{ADMINISTRATORS_SID, DescriptorError, PrivateDirectoryDescriptor, SYSTEM_SID};
