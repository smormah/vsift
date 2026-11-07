//! The value measurement, phase 1 (`docs/planning/value-measurement/method.md`):
//! the same assistant answering the same questions about the same
//! recordings with FFmpeg, FFprobe and whisper-cli alone (R0, R1) and with
//! `VSift` (V0, V1).
//!
//! It is a mode of its own beside the skill and cold trials, which it does
//! not change: its own task files ([`task`]), one preparation for all four
//! arms ([`prepare`]), a grader that reads every arm's report the same way
//! ([`claims`]) and the measures of the method ([`grade`]). `run` is shared;
//! for a value trial it puts the tools on `PATH`, applies the task's budget
//! and, before a second phase, writes the first answer to the workspace.

pub mod claims;
pub mod grade;
pub mod prepare;
pub mod summary;
pub mod task;
