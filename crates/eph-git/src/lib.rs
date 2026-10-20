//! `eph-git`: the one crate that runs git (RUST-PLAN §3.2; invariant 4,
//! ADR-0004).
//!
//! - [`runner::Git`] starts every git process, under one of two profiles: the
//!   Agora's, which reads no machine configuration, and a target repository's,
//!   which reads the user's.
//! - [`committer::Committer`] is the Agora's single committer: one queue, work
//!   batched, a failed git retried on a fixed schedule, a stale `index.lock`
//!   cleared only while the committer holds its own lock, and a reconcile after
//!   a restart.
//! - [`worktrees::Worktrees`] gives an agent its own checkout of a target
//!   repository and never deletes work to do it (FR-1.5).
//! - [`remotes::read_remotes`] reads a checkout's remotes.
//!
//! Agents never run any of this. They write plain files (ADR-0003), and the
//! harness runs git before an agent exists and after it is gone.

pub mod committer;
pub mod remotes;
pub mod runner;
pub mod worktrees;
