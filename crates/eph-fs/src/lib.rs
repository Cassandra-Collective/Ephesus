//! `eph-fs`: the file layer every other crate writes through (SDD-RUST §2 and
//! §4.0 rule 5; invariants 3 and 5).
//!
//! - [`atomic::write_atomic`] for any file another process reads: a temporary
//!   file, synced, renamed over the target, with Windows' passing holds waited
//!   out.
//! - [`append::AppendLog`] for the append-only files: one synced line per
//!   append, `seq` recovered at open, a torn tail left alone, rotation by rename.
//! - [`home`] for the harness home: whose a directory is, its marker, its
//!   directories, and the defaults seeded only when absent.
//! - [`lock::Exclusive`] for a lock one holder keeps while it runs, released by
//!   the OS when the process dies: the single committer's lock.
//!
//! This is the one crate RUST-PLAN §3.2 allows to write a file another process
//! reads, and each write says so with an `fs-write:` exemption.

pub mod append;
pub mod atomic;
pub mod home;
pub mod lock;
