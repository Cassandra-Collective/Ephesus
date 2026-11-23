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
//! R1.4b adds the book of record and the degradation channel to the same
//! crate. It writes only through `eph-fs` and runs git only through `eph-git`'s
//! committer, so it claims no exemption of its own (RUST-PLAN §3.2).

pub mod agora;
pub mod retention;
pub mod store;
