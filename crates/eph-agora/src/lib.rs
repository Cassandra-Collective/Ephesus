//! `eph-agora`: the Agora, the company's one git repository (ADR-0004), and the
//! one store every structured record goes through (SDD-RUST §4.0 rule 6).
//!
//! - [`store::Store`] reads a record, telling ABSENT from DAMAGED, and writes
//!   one only after validating it and never over a damaged file.
//! - [`agora::Agora`] opens the Agora in its boot order: the committer, the
//!   reconcile of whatever a killed harness left, the files the harness owns
//!   (`PROTOCOL.md`, `.gitignore`), the seeds, and one commit when anything
//!   changed. It holds the registry, the task ledger with its retention, and
//!   the board.
//!
//! - [`book::Book`] is the book of record: one writer that syncs then
//!   publishes, and readers that page by byte offset across rotation (R1.4b).
//! - [`degradations::Degradations`] is the degradation channel, carrying what
//!   the boot found, every commit nobody waited for that failed, and what an
//!   agent's own ignore file hides from history (R1.4b).
//! - [`archive`] reads back the done tasks the ledger's retention moved out.
//!
//! It writes only through `eph-fs` and runs git only through `eph-git`'s
//! committer. Its one exemption, `process:`, is for its kill tests, which run
//! the test binary again as the harness they kill (GYM-014).

pub mod agora;
pub mod archive;
pub mod book;
pub mod degradations;
pub mod retention;
pub mod store;
