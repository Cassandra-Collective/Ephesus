//! The task ledger's retention (SDD-RUST §4.5; Architect, 2026-10-20).
//!
//! Contract: [`split`] is pure. A ledger of at most [`ARCHIVE_PAST`] tasks is
//! returned whole. A larger one keeps every task not `done`, the newest
//! [`DONE_KEPT`] done tasks (by `updatedAt`, then id), and every task a kept
//! task depends on, transitively, because a dependency outside the ledger makes
//! the ledger damaged. Every other done task is returned to be archived. Both
//! halves keep the ledger's order.

use std::collections::{BTreeMap, BTreeSet};

use eph_schema::id::TaskId;
use eph_schema::tasks::{Task, TaskStatus, Tasks};

/// The size past which done tasks are archived: 9,000 of the ledger's 10,000.
pub const ARCHIVE_PAST: usize = 9_000;

/// How many done tasks stay in the ledger when it is trimmed.
pub const DONE_KEPT: usize = 1_000;

/// The ledger to keep and the done tasks to archive, under the retention rule.
pub fn split(ledger: &Tasks) -> (Tasks, Vec<Task>) {
    split_at(ledger, ARCHIVE_PAST, DONE_KEPT)
}

/// [`split`] with its two numbers as parameters, so the rule is tested small.
pub fn split_at(ledger: &Tasks, archive_past: usize, done_kept: usize) -> (Tasks, Vec<Task>) {
    if ledger.tasks.len() <= archive_past {
        return (ledger.clone(), Vec::new());
    }
    let mut done: Vec<&Task> = ledger
        .tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Done)
        .collect();
    done.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    let mut keep: BTreeSet<&TaskId> = ledger
        .tasks
        .iter()
        .filter(|task| task.status != TaskStatus::Done)
        .map(|task| &task.id)
        .chain(done.iter().take(done_kept).map(|task| &task.id))
        .collect();
    let by_id: BTreeMap<&TaskId, &Task> =
        ledger.tasks.iter().map(|task| (&task.id, task)).collect();
    let mut unvisited: Vec<&TaskId> = keep.iter().copied().collect();
    while let Some(id) = unvisited.pop() {
        for dep in by_id.get(id).map_or(&[][..], |task| &task.deps[..]) {
            if keep.insert(dep) {
                unvisited.push(dep);
            }
        }
    }
    let (kept, archived): (Vec<Task>, Vec<Task>) = ledger
        .tasks
        .iter()
        .cloned()
        .partition(|task| keep.contains(&task.id));
    (
        Tasks {
            schema_version: ledger.schema_version,
            tasks: kept,
        },
        archived,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeSet;

    use eph_schema::document::Versioned;
    use eph_schema::tasks::{Artifacts, Source};
    use eph_schema::time::Timestamp;

    use super::*;

    /// Task `n`, `done` or `todo`, last updated at `updated` ms.
    pub(crate) fn task(n: u32, done: bool, updated: i64) -> Task {
        Task {
            id: TaskId::mint(Timestamp::from_millis(1_000).unwrap(), &format!("{n:08x}")).unwrap(),
            title: format!("task {n}"),
            spec: String::new(),
            assignee: None,
            status: if done {
                TaskStatus::Done
            } else {
                TaskStatus::Todo
            },
            priority: 5,
            deps: Vec::new(),
            review: BTreeSet::new(),
            gates: Vec::new(),
            artifacts: Artifacts::default(),
            source: Source {
                kind: "propose".into(),
                reference: "msg:msg-20251019T123456789Z-1a2b3c4d".parse().unwrap(),
            },
            created_at: Timestamp::from_millis(1_000).unwrap(),
            updated_at: Timestamp::from_millis(updated).unwrap(),
        }
    }

    pub(crate) fn ledger(tasks: Vec<Task>) -> Tasks {
        Tasks {
            schema_version: Tasks::VERSION,
            tasks,
        }
    }

    fn ids(tasks: &[Task]) -> Vec<String> {
        tasks.iter().map(|task| task.title.clone()).collect()
    }

    #[test]
    fn the_rules_numbers_are_the_architects() {
        assert_eq!(ARCHIVE_PAST, 9_000);
        assert_eq!(DONE_KEPT, 1_000);
    }

    #[test]
    fn a_ledger_at_or_under_the_threshold_is_kept_whole() {
        let at = ledger(
            (0..5)
                .map(|n| task(n, true, 2_000 + i64::from(n)))
                .collect(),
        );
        let (kept, archived) = split_at(&at, 5, 1);
        assert_eq!(kept, at);
        assert!(archived.is_empty());
    }

    #[test]
    fn past_it_the_newest_done_and_every_open_task_stay() {
        // Five done (updated 2001..2005) and two open; keep the newest two done.
        let mut tasks: Vec<Task> = (0..5)
            .map(|n| task(n, true, 2_001 + i64::from(n)))
            .collect();
        tasks.push(task(10, false, 1_500));
        tasks.push(task(11, false, 1_600));
        let (kept, archived) = split_at(&ledger(tasks), 6, 2);
        assert_eq!(ids(&kept.tasks), ["task 3", "task 4", "task 10", "task 11"]);
        assert_eq!(ids(&archived), ["task 0", "task 1", "task 2"]);
        assert_eq!(kept.schema_version, Tasks::VERSION);
        assert!(kept.validate().is_ok(), "the kept ledger is consistent");
    }

    #[test]
    fn a_done_task_a_kept_task_depends_on_stays_however_far_back_the_chain_runs() {
        // open 10 -> done 2 -> done 1 -> done 0; done 3 is newest and kept anyway.
        let mut zero = task(0, true, 2_000);
        let mut one = task(1, true, 2_001);
        let two = task(2, true, 2_002);
        one.deps = vec![zero.id.clone()];
        let mut two = two;
        two.deps = vec![one.id.clone()];
        zero.deps = Vec::new();
        let mut open = task(10, false, 1_000);
        open.deps = vec![two.id.clone()];
        let tasks = vec![
            zero,
            one,
            two,
            task(3, true, 2_100),
            task(4, true, 1_900),
            open,
        ];
        let (kept, archived) = split_at(&ledger(tasks), 3, 1);
        assert_eq!(
            ids(&kept.tasks),
            ["task 0", "task 1", "task 2", "task 3", "task 10"]
        );
        assert_eq!(ids(&archived), ["task 4"]);
        assert!(kept.validate().is_ok(), "{:?}", kept.validate());
    }

    #[test]
    fn two_done_tasks_updated_at_once_are_ordered_by_id() {
        let tasks = vec![
            task(1, true, 2_000),
            task(2, true, 2_000),
            task(3, true, 2_000),
        ];
        let (kept, archived) = split_at(&ledger(tasks), 2, 1);
        assert_eq!(ids(&kept.tasks), ["task 3"], "the higher id is the newer");
        assert_eq!(ids(&archived), ["task 1", "task 2"]);
    }

    #[test]
    fn the_production_numbers_trim_a_ledger_of_9001() {
        let tasks: Vec<Task> = (0..9_001)
            .map(|n| task(n, true, 2_000 + i64::from(n)))
            .collect();
        let (kept, archived) = split(&ledger(tasks));
        assert_eq!(kept.tasks.len(), 1_000);
        assert_eq!(archived.len(), 8_001);
        assert_eq!(kept.tasks.first().unwrap().title, "task 8001");
        let (whole, none) = split(&ledger((0..9_000).map(|n| task(n, true, 2_000)).collect()));
        assert_eq!(whole.tasks.len(), 9_000);
        assert!(none.is_empty(), "9,000 is the threshold, not past it");
    }
}
