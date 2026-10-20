//! `tasks.json`: the task ledger (SDD-RUST §4.5; FR-4.3; ADR-0005, ADR-0008).
//!
//! Contract: a ledger is consistent as a whole. Ids are unique, every
//! dependency names a task in the ledger and never the task itself, and a task
//! cannot be `done` while its review artifacts are missing or a gate blocks it
//! ([`Task::can_close`]).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::id::{GateId, MemoId, TaskId};
use crate::names::{AgentId, check_len};
use crate::refs::{AgoraPath, Ref};
use crate::time::Timestamp;

/// Where a task stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Not started.
    Todo,
    /// Being worked.
    InProgress,
    /// Waiting on a gate or a dependency.
    Blocked,
    /// Waiting for review.
    Review,
    /// Finished.
    Done,
    /// No progress for longer than the stall threshold.
    Stalled,
}

/// A review a task requires before it may close (FR-7.2, FR-7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewKind {
    /// A slide deck.
    Deck,
    /// A decision memo.
    Memo,
}

/// What a task produced.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Artifacts {
    /// Its deck, in the Odeon.
    pub deck: Option<AgoraPath>,
    /// Its memos.
    pub memos: Vec<MemoId>,
    /// Where its result is.
    pub result_ref: Option<Ref>,
}

/// Where a task came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Source {
    /// How it arrived (`propose`), 1 to 32 characters.
    pub kind: String,
    /// The message or log line it came from.
    #[serde(rename = "ref")]
    pub reference: Ref,
}

/// One task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Task {
    /// Its id.
    pub id: TaskId,
    /// Its title, 1 to 200 characters.
    pub title: String,
    /// Its specification, at most 20000 characters.
    pub spec: String,
    /// Who works it.
    pub assignee: Option<AgentId>,
    /// Where it stands.
    pub status: TaskStatus,
    /// 0 (lowest) to 9.
    pub priority: u8,
    /// Tasks it waits on, at most 64.
    pub deps: Vec<TaskId>,
    /// Reviews it requires.
    pub review: BTreeSet<ReviewKind>,
    /// Gates that block it, at most 32.
    pub gates: Vec<GateId>,
    /// What it produced.
    pub artifacts: Artifacts,
    /// Where it came from.
    pub source: Source,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
}

impl Task {
    /// Whether the task may be marked `done`: every review it requires has its
    /// artifact, and no gate blocks it. The ledger refuses a `done` that this
    /// says no to.
    pub fn can_close(&self) -> bool {
        let deck_ok = !self.review.contains(&ReviewKind::Deck) || self.artifacts.deck.is_some();
        let memo_ok = !self.review.contains(&ReviewKind::Memo) || !self.artifacts.memos.is_empty();
        deck_ok && memo_ok && self.gates.is_empty()
    }
}

/// `tasks.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Tasks {
    /// [`Tasks::VERSION`].
    pub schema_version: u32,
    /// At most 10000 tasks.
    pub tasks: Vec<Task>,
}

impl Tasks {
    /// An empty ledger.
    pub fn empty() -> Tasks {
        Tasks {
            schema_version: Self::VERSION,
            tasks: Vec::new(),
        }
    }
}

impl Versioned for Tasks {
    const FILE: &'static str = "tasks.json";
    const VERSION: u32 = 2;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if self.tasks.len() > 10_000 {
            return Err(SchemaError::invalid(
                Self::FILE,
                "tasks",
                "must hold at most 10000 tasks",
            ));
        }
        let ids: BTreeSet<&TaskId> = self.tasks.iter().map(|t| &t.id).collect();
        if ids.len() != self.tasks.len() {
            return Err(SchemaError::invalid(
                Self::FILE,
                "tasks",
                "every task id must be unique",
            ));
        }
        for task in &self.tasks {
            let field = |name: &str| format!("tasks.{}.{name}", task.id);
            check_len(Self::FILE, &field("title"), &task.title, 1, 200)?;
            check_len(Self::FILE, &field("spec"), &task.spec, 0, 20_000)?;
            check_len(Self::FILE, &field("source.kind"), &task.source.kind, 1, 32)?;
            if task.priority > 9 {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("priority"),
                    "must be 0 to 9",
                ));
            }
            if task.deps.len() > 64 || task.gates.len() > 32 || task.artifacts.memos.len() > 64 {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("deps"),
                    "at most 64 dependencies, 32 gates and 64 memos",
                ));
            }
            for dep in &task.deps {
                if dep == &task.id {
                    return Err(SchemaError::invalid(
                        Self::FILE,
                        field("deps"),
                        "a task cannot depend on itself",
                    ));
                }
                if !ids.contains(dep) {
                    return Err(SchemaError::invalid(
                        Self::FILE,
                        field("deps"),
                        format!("names `{dep}`, which is not in the ledger"),
                    ));
                }
            }
            if task.updated_at < task.created_at {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("updatedAt"),
                    "must not be before createdAt",
                ));
            }
            if task.status == TaskStatus::Done && !task.can_close() {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("status"),
                    "is done while a required review artifact is missing or a gate still blocks it",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{parse, render};

    fn id(n: u8) -> TaskId {
        TaskId::mint(
            Timestamp::from_millis(1_000 + i64::from(n)).unwrap(),
            &format!("{n:08x}"),
        )
        .unwrap()
    }

    fn task(n: u8) -> Task {
        Task {
            id: id(n),
            title: format!("task {n}"),
            spec: String::new(),
            assignee: Some("agent.mason".parse().unwrap()),
            status: TaskStatus::Todo,
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
            updated_at: Timestamp::from_millis(2_000).unwrap(),
        }
    }

    fn ledger(tasks: Vec<Task>) -> Tasks {
        Tasks {
            schema_version: 2,
            tasks,
        }
    }

    #[test]
    fn a_ledger_round_trips_with_its_refs_in_the_one_grammar() {
        let mut b = task(2);
        b.deps = vec![id(1)];
        b.status = TaskStatus::InProgress;
        let tasks = ledger(vec![task(1), b]);
        let text = render(&tasks).unwrap();
        assert!(text.contains("\"ref\": \"msg:msg-20251019T123456789Z-1a2b3c4d\""));
        assert!(text.contains("\"in_progress\""));
        assert_eq!(parse::<Tasks>(&text).unwrap(), tasks);
    }

    #[test]
    fn a_task_closes_only_with_its_reviews_and_no_gates() {
        let mut t = task(1);
        assert!(t.can_close());
        t.review.insert(ReviewKind::Deck);
        assert!(!t.can_close());
        t.artifacts.deck = Some("odeon/decks/x.html".parse().unwrap());
        assert!(t.can_close());
        t.review.insert(ReviewKind::Memo);
        assert!(!t.can_close());
        t.artifacts
            .memos
            .push(MemoId::mint(Timestamp::from_millis(5).unwrap(), "abcdef01").unwrap());
        assert!(t.can_close());
        t.gates
            .push(GateId::mint(Timestamp::from_millis(5).unwrap(), "abcdef01").unwrap());
        assert!(!t.can_close());
        t.status = TaskStatus::Done;
        assert!(
            render(&ledger(vec![t]))
                .unwrap_err()
                .to_string()
                .contains("gate still blocks")
        );
    }

    #[test]
    fn each_ledger_rule_refuses_naming_the_task() {
        assert!(
            render(&ledger(vec![task(1), task(1)]))
                .unwrap_err()
                .to_string()
                .contains("unique")
        );

        let mut self_dep = task(1);
        self_dep.deps = vec![id(1)];
        assert!(
            render(&ledger(vec![self_dep]))
                .unwrap_err()
                .to_string()
                .contains("depend on itself")
        );

        let mut dangling = task(1);
        dangling.deps = vec![id(9)];
        assert!(
            render(&ledger(vec![dangling]))
                .unwrap_err()
                .to_string()
                .contains("not in the ledger")
        );

        let mut backwards = task(1);
        backwards.updated_at = Timestamp::from_millis(10).unwrap();
        assert!(
            render(&ledger(vec![backwards]))
                .unwrap_err()
                .to_string()
                .contains("updatedAt")
        );

        let mut loud = task(1);
        loud.priority = 10;
        assert!(
            render(&ledger(vec![loud]))
                .unwrap_err()
                .to_string()
                .contains("priority")
        );

        let mut untitled = task(1);
        untitled.title = String::new();
        assert!(
            render(&ledger(vec![untitled]))
                .unwrap_err()
                .to_string()
                .contains(".title")
        );
    }

    #[test]
    fn the_status_vocabulary_is_closed() {
        let text = render(&ledger(vec![task(1)]))
            .unwrap()
            .replace("\"todo\"", "\"someday\"");
        assert!(parse::<Tasks>(&text).is_err());
    }

    fn wide_id(n: u32) -> TaskId {
        TaskId::mint(Timestamp::from_millis(1_000).unwrap(), &format!("{n:08x}")).unwrap()
    }

    #[test]
    fn a_ledger_and_each_list_in_a_task_are_capped() {
        let mut huge = ledger(Vec::new());
        huge.tasks = (0..10_001)
            .map(|n| Task {
                id: wide_id(n),
                ..task(1)
            })
            .collect();
        assert!(
            render(&huge)
                .unwrap_err()
                .to_string()
                .contains("must hold at most 10000 tasks")
        );
        huge.tasks.pop();
        assert!(render(&huge).is_ok(), "10000 tasks is the cap, not past it");
        let at = Timestamp::from_millis(5).unwrap();
        let memo = move |n: u32| MemoId::mint(at, &format!("{n:08x}")).unwrap();
        let gate = move |n: u32| GateId::mint(at, &format!("{n:08x}")).unwrap();
        type Edit = Box<dyn Fn(&mut Task)>;
        let cases: [(&str, Edit); 3] = [
            (
                "deps",
                Box::new(|t: &mut Task| t.deps = (0..65).map(wide_id).collect()),
            ),
            (
                "gates",
                Box::new(move |t: &mut Task| t.gates = (0..33).map(gate).collect()),
            ),
            (
                "memos",
                Box::new(move |t: &mut Task| t.artifacts.memos = (0..65).map(memo).collect()),
            ),
        ];
        for (what, edit) in cases {
            let mut t = task(1);
            edit(&mut t);
            let mut tasks = vec![t];
            tasks.extend((0..65).map(|n| Task {
                id: wide_id(n),
                ..task(1)
            }));
            let error = render(&ledger(tasks)).unwrap_err().to_string();
            assert!(
                error.contains("at most 64 dependencies, 32 gates and 64 memos"),
                "{what}: {error}"
            );
        }
        let mut at_cap = task(1);
        at_cap.gates = (0..32).map(gate).collect();
        at_cap.artifacts.memos = (0..64).map(memo).collect();
        assert!(render(&ledger(vec![at_cap])).is_ok());
        assert!(
            parse::<Tasks>(&render(&Tasks::empty()).unwrap())
                .unwrap()
                .tasks
                .is_empty()
        );
    }

    /// Round 1 (2026-10-20): each task cap was tested past its limit, never at.
    #[test]
    fn each_task_bound_admits_its_limit() {
        let mut top = task(1);
        top.priority = 9;
        let mut same_instant = task(2);
        same_instant.updated_at = same_instant.created_at;
        let mut many = task(3);
        many.deps = (0..64).map(wide_id).collect();
        let mut tasks = vec![top, same_instant, many];
        tasks.extend((0..64).map(|n| Task {
            id: wide_id(n),
            ..task(1)
        }));
        assert!(render(&ledger(tasks)).is_ok());
    }
}
