//! Infrastructure adapters for operating-system and provider boundaries.

#![forbid(unsafe_code)]

mod process_dependency_probe;

pub use process_dependency_probe::ProcessDependencyProbe;
