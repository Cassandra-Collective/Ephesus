//! The terminal plane for Ephesus (FR-1.1, ADR-0014).

pub mod path;
pub mod redaction;
mod session;

pub use session::{ExitStatus, Output, Session, ShutdownScope, SpawnPlan};
