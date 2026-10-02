//! Named-client agent trials for the `VSift` skill (P12, ADR 0022 decision 7).
//!
//! The harness prepares a trial workspace for one scenario, runs Claude Code
//! or Codex in it through an explicit executable and argument list (never a
//! shell), and grades the client's event stream into two separate results:
//! a **mechanical** result a program decides (handoff validity, citations
//! resolved in the retained bundle, truth windows, command policy, budgets,
//! the image check, canaries, report text) and an **interpretation** result
//! (key facts of the frozen truth, scenario expectations and a slot for a
//! human reviewer). Model prose can never turn a mechanical failure into a
//! pass.
//!
//! The command policy and the budgets are parsed from the skill's own
//! references ([`policy`]), and every truth window and key fact from the
//! corpus manifest ([`truth`]), so neither can drift from its source.
//!
//! This crate sends no prompt anywhere by itself: `run` starts the client
//! the operator names, and its tests use a stand-in client that replays a
//! recorded stream (`src/bin/stub_client.rs`).

pub mod bundle;
pub mod calls;
pub mod campaign;
pub mod claude_trust;
pub mod client_warnings;
pub mod cold;
pub mod error;
pub mod evaluate;
pub mod freeze;
pub mod grade;
pub mod handoff;
pub mod holdout;
pub mod install;
pub mod layout;
pub mod leak_check;
pub mod policy;
pub mod prepare;
pub mod record;
pub mod roots;
pub mod run;
pub mod scenario;
pub mod shell;
pub mod shim;
pub mod skill;
pub mod summary;
pub mod trace;
pub mod truth;
pub mod usage_limit;
pub mod vsift_cli;

pub use error::TrialError;
