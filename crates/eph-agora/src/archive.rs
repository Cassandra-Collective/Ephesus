//! The reader of `tasks-archive.jsonl`, the done tasks the ledger's retention
//! moved out (SDD-RUST §4.5; DECISIONS-LOG 2026-10-25, *R1.4b — Architect
//! decisions, second round*, items 3 and 6).
//!
//! Contract: [`read`] returns every archived task the ledger does not hold.
//! The newest archive line wins a repeat, and the ledger wins over the archive,
//! because a ledger write cut short between the two files leaves a task in
//! both (R1.4a). Its line counts are the book's: a torn last line is skipped
//! and said so, a line a newer build wrote is counted as unknown, and any other
//! line that is not a valid archive line is counted as damaged. Nothing is ever
//! written here.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;

use eph_schema::document::{Versioned, parse};
use eph_schema::id::TaskId;
use eph_schema::json::strict_value;
use eph_schema::tasks::{ArchivedTasks, Task};
use serde_json::Value;

use crate::store::Damage;

/// What the archive holds that the ledger does not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Archived {
    /// The archived tasks, each id once, in the order the archive first held
    /// them.
    pub tasks: Vec<Task>,
    /// Lines a newer build wrote: kept on disk, counted.
    pub unknown: usize,
    /// Lines that are not archive lines: damage, counted.
    pub damaged: usize,
    /// Whether the archive ended mid-line: a write cut short, skipped.
    pub torn_tail: bool,
}

/// Why the archive could not be read.
#[derive(Debug)]
pub enum ArchiveError {
    /// The ledger is damaged, so which archived tasks it holds is unknown.
    Ledger(Damage),
    /// The archive could not be read.
    Io(io::Error),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArchiveError::Ledger(damage) => write!(
                f,
                "the task archive is read against the ledger, which is damaged: {}",
                damage.detail()
            ),
            ArchiveError::Io(error) => write!(f, "the task archive could not be read: {error}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

/// Reads the archive's `bytes` against the ids the ledger `held`. See the
/// module contract.
pub fn read(bytes: &[u8], held: &HashSet<TaskId>) -> Archived {
    let mut archived = Archived::default();
    let mut at: HashMap<TaskId, usize> = HashMap::new();
    let mut tasks: Vec<Task> = Vec::new();
    for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
        let Some(line) = piece.strip_suffix(b"\n") else {
            archived.torn_tail = true;
            break;
        };
        let line = String::from_utf8_lossy(line);
        // A line a newer build wrote is told apart by its version before it
        // is read as this build's, as the book's reader does.
        let version = strict_value(&line)
            .ok()
            .and_then(|value| value.get("schemaVersion").and_then(Value::as_u64));
        if version.is_some_and(|version| version > u64::from(ArchivedTasks::VERSION)) {
            archived.unknown += 1;
            continue;
        }
        let Ok(archive) = parse::<ArchivedTasks>(&line) else {
            archived.damaged += 1;
            continue;
        };
        for task in archive.tasks {
            match at.get(&task.id) {
                Some(&index) => tasks[index] = task,
                None => {
                    at.insert(task.id.clone(), tasks.len());
                    tasks.push(task);
                }
            }
        }
    }
    archived.tasks = tasks
        .into_iter()
        .filter(|task| !held.contains(&task.id))
        .collect();
    archived
}

#[cfg(test)]
mod tests {
    use eph_schema::tasks::render_archived;
    use eph_schema::time::Timestamp;

    use super::*;
    use crate::retention::tests::task;

    fn line(tasks: Vec<Task>) -> String {
        let mut text = render_archived(&ArchivedTasks::new(
            tasks,
            Timestamp::from_millis(5_000).unwrap(),
        ))
        .unwrap();
        text.push('\n');
        text
    }

    fn titles(archived: &Archived) -> Vec<&str> {
        archived
            .tasks
            .iter()
            .map(|task| task.title.as_str())
            .collect()
    }

    #[test]
    fn every_task_the_ledger_does_not_hold_the_newest_line_winning_a_repeat() {
        let mut renamed = task(1, true, 9_000);
        renamed.title = "task 1, archived again".into();
        let text = [
            line(vec![task(1, true, 2_000), task(2, true, 2_000)]),
            line(vec![task(3, true, 3_000), renamed]),
        ]
        .concat();
        let held: HashSet<TaskId> = [task(2, true, 2_000).id].into_iter().collect();
        let archived = read(text.as_bytes(), &held);
        assert_eq!(
            titles(&archived),
            ["task 1, archived again", "task 3"],
            "task 2 is the ledger's; task 1 keeps its first place and its newest line"
        );
        assert_eq!(
            (archived.unknown, archived.damaged, archived.torn_tail),
            (0, 0, false)
        );
        assert_eq!(read(b"", &HashSet::new()), Archived::default());
    }

    #[test]
    fn a_torn_tail_is_skipped_and_damaged_and_newer_lines_are_counted() {
        let whole = line(vec![task(1, true, 2_000)]);
        let newer = line(vec![task(2, true, 2_000)]).replacen(
            "\"schemaVersion\":2",
            "\"schemaVersion\":9",
            1,
        );
        let older = line(vec![task(3, true, 2_000)]).replacen(
            "\"schemaVersion\":2",
            "\"schemaVersion\":1",
            1,
        );
        let torn = line(vec![task(4, true, 2_000)]);
        let text = format!(
            "{whole}not a line\n\n{newer}{older}{}",
            &torn[..torn.len() / 2]
        );
        let none = HashSet::new();
        let counts = |text: &str| {
            let read = read(text.as_bytes(), &none);
            (read.unknown, read.damaged)
        };
        assert_eq!(counts(&newer), (1, 0), "a newer line is unknown");
        assert_eq!(counts(&older), (0, 1), "an older line is damage");
        assert_eq!(counts(&whole), (0, 0), "a line of this version is read");
        let archived = read(text.as_bytes(), &HashSet::new());
        assert_eq!(titles(&archived), ["task 1"]);
        assert_eq!(archived.unknown, 1, "a newer build's line is kept, counted");
        assert_eq!(
            archived.damaged, 3,
            "a line that is not one, a blank line, and an older version"
        );
        assert!(archived.torn_tail);
        let mut todo = task(5, false, 2_000);
        todo.title = "not done".into();
        let invalid = format!(
            "{{\"schemaVersion\":2,\"archivedAt\":\"1970-01-01T00:00:05.000Z\",\"tasks\":[{}]}}\n",
            serde_json::to_string(&todo).unwrap()
        );
        assert_eq!(
            read(invalid.as_bytes(), &HashSet::new()).damaged,
            1,
            "a line holding a task that is not done breaks the archive's rule"
        );
    }

    #[test]
    fn an_archive_error_names_what_to_do() {
        let error = ArchiveError::Io(io::Error::other("denied")).to_string();
        assert!(error.contains("could not be read: denied"), "{error}");
    }
}
