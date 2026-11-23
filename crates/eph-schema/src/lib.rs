//! Every persisted Ephesus format as a typed, validated document
//! (docs/sdd/SDD-RUST.md §4, approved 2026-10-20).
//!
//! This crate does no I/O (RUST-PLAN §3.2, enforced by `cargo xtask
//! invariants`). It turns text into a validated value and a value back into
//! text, and refuses anything §4's rules do not allow, with a message that says
//! which file, which field and what is allowed. Reading and writing files is
//! `eph-fs`'s job, and deciding what a damaged file means is `eph-agora`'s
//! store.
//!
//! The rules every format follows (SDD-RUST §4.0) live here:
//! - one version field ([`document`]),
//! - one time format ([`time`]),
//! - one id grammar ([`id`]),
//! - one reference grammar ([`refs`]).
//!
//! R1.1 types what R1 needs (Architect, 2026-10-20). Every other model is
//! typed by the phase that builds its subsystem.

pub mod authority;
pub mod config;
pub mod degradation;
pub mod document;
pub mod error;
pub mod gates;
pub mod github_app;
pub mod home;
pub mod id;
pub mod json;
pub mod known_targets;
pub mod log;
pub mod message;
pub mod names;
pub mod refs;
pub mod registry;
pub mod restart;
pub mod tasks;
pub mod time;

pub use document::{Versioned, parse, render};
pub use error::SchemaError;
