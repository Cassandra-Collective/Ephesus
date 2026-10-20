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

    /// Every rule that holds for one task wherever it is kept, the ledger and
    /// each archive line, so an archived task is held to the ledger's rules
    /// (R1.4a review, 2026-10-20). `file` names the file in the refusal.
    fn check(&self, file: &'static str) -> Result<(), SchemaError> {
        let field = |name: &str| format!("tasks.{}.{name}", self.id);
        check_len(file, &field("title"), &self.title, 1, 200)?;
        check_len(file, &field("spec"), &self.spec, 0, 20_000)?;
        check_len(file, &field("source.kind"), &self.source.kind, 1, 32)?;
        if self.priority > 9 {
            return Err(SchemaError::invalid(
                file,
                field("priority"),
                "must be 0 to 9",
            ));
        }
        if self.deps.len() > 64 || self.gates.len() > 32 || self.artifacts.memos.len() > 64 {
            return Err(SchemaError::invalid(
                file,
                field("deps"),
                "at most 64 dependencies, 32 gates and 64 memos",
            ));
        }
        if self.deps.contains(&self.id) {
            return Err(SchemaError::invalid(
                file,
                field("deps"),
                "a task cannot depend on itself",
            ));
        }
        if self.updated_at < self.created_at {
            return Err(SchemaError::invalid(
                file,
                field("updatedAt"),
                "must not be before createdAt",
            ));
        }
        if self.status == TaskStatus::Done && !self.can_close() {
            return Err(SchemaError::invalid(
                file,
                field("status"),
                "is done while a required review artifact is missing or a gate still blocks it",
            ));
        }
        Ok(())
    }
}

/// The most tasks a ledger, or one archive line, holds.
pub const MAX_TASKS: usize = 10_000;

/// The ids of `tasks`, refusing, in `file`'s name, more than [`MAX_TASKS`] or
/// two tasks with one id.
fn unique_ids<'a>(
    file: &'static str,
    tasks: &'a [Task],
) -> Result<BTreeSet<&'a TaskId>, SchemaError> {
    if tasks.len() > MAX_TASKS {
        return Err(SchemaError::invalid(
            file,
            "tasks",
            format!("must hold at most {MAX_TASKS} tasks"),
        ));
    }
    let ids: BTreeSet<&TaskId> = tasks.iter().map(|t| &t.id).collect();
    if ids.len() != tasks.len() {
        return Err(SchemaError::invalid(
            file,
            "tasks",
            "every task id must be unique",
        ));
    }
    Ok(ids)
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
        let ids = unique_ids(Self::FILE, &self.tasks)?;
        for task in &self.tasks {
            task.check(Self::FILE)?;
            if let Some(dep) = task.deps.iter().find(|dep| !ids.contains(dep)) {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    format!("tasks.{}.deps", task.id),
                    format!("names `{dep}`, which is not in the ledger"),
                ));
            }
        }
        Ok(())
    }
}

/// One line of `tasks-archive.jsonl`: the done tasks one retention run moved out
/// of the ledger (SDD-RUST §4.5; Architect, 2026-10-20). Append-only. One line
/// per run, not per task: each line is one synced append, and 8,001 of them took
/// 62 s on win32 (R1.4a, 2026-10-20). Each task is held to the ledger's own
/// per-task rules, and ids are unique within the line. A dependency is not
/// checked: it may name a task in the ledger or on an earlier line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArchivedTasks {
    /// [`ArchivedTasks::VERSION`].
    pub schema_version: u32,
    /// When they left the ledger.
    pub archived_at: Timestamp,
    /// The tasks as the ledger last held them, every one `done`.
    pub tasks: Vec<Task>,
}

impl ArchivedTasks {
    /// `tasks`, archived at `at`.
    pub fn new(tasks: Vec<Task>, at: Timestamp) -> ArchivedTasks {
        ArchivedTasks {
            schema_version: Self::VERSION,
            archived_at: at,
            tasks,
        }
    }
}

impl Versioned for ArchivedTasks {
    const FILE: &'static str = "tasks-archive.jsonl";
    /// The ledger's: a line holds the ledger's [`Task`], so a change to a task's
    /// shape is a new version of both files.
    const VERSION: u32 = Tasks::VERSION;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if self.tasks.is_empty() {
            return Err(SchemaError::invalid(
                Self::FILE,
                "tasks",
                "an archive line holds at least one task",
            ));
        }
        unique_ids(Self::FILE, &self.tasks)?;
        for task in &self.tasks {
            task.check(Self::FILE)?;
            if task.status != TaskStatus::Done {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    format!("tasks.{}.status", task.id),
                    "only a done task is archived",
                ));
            }
        }
        Ok(())
    }
}

/// An archive line as one line of JSON, validated first.
pub fn render_archived(line: &ArchivedTasks) -> Result<String, SchemaError> {
    line.validate()?;
    serde_json::to_string(line).map_err(|error| SchemaError::Shape {
        file: ArchivedTasks::FILE,
        detail: error.to_string(),
    })
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
    fn an_archive_line_is_one_line_of_done_tasks() {
        let mut one = task(1);
        one.status = TaskStatus::Done;
        let mut two = task(2);
        two.status = TaskStatus::Done;
        let at = Timestamp::from_millis(9_000).unwrap();
        let line = ArchivedTasks::new(vec![one.clone(), two.clone()], at);
        let text = render_archived(&line).unwrap();
        assert!(!text.contains('\n'), "{text}");
        assert!(
            text.starts_with("{\"schemaVersion\":2,\"archivedAt\":"),
            "the ledger's version: {text}"
        );
        assert_eq!(parse::<ArchivedTasks>(&text).unwrap(), line);
        let open = render_archived(&ArchivedTasks::new(vec![one, task(3)], at)).unwrap_err();
        assert!(
            open.to_string().contains(&format!(
                "tasks.{}.status: only a done task is archived",
                id(3)
            )),
            "{open}"
        );
        let empty = render_archived(&ArchivedTasks::new(Vec::new(), at)).unwrap_err();
        assert!(empty.to_string().contains("at least one task"), "{empty}");
        let mut wrong = ArchivedTasks::new(vec![two], at);
        wrong.schema_version = 1;
        assert!(render_archived(&wrong).is_err());
    }

    /// R1.4a review D1 (2026-10-20): an archived task was checked only for
    /// being done, so a task the ledger would refuse could reach the archive.
    #[test]
    fn an_archive_line_holds_each_task_to_the_ledgers_rules() {
        let at = Timestamp::from_millis(9_000).unwrap();
        let done = |n: u8| Task {
            status: TaskStatus::Done,
            ..task(n)
        };
        let mut gated = done(1);
        gated
            .gates
            .push(GateId::mint(Timestamp::from_millis(5).unwrap(), "abcdef01").unwrap());
        let mut untitled = done(2);
        untitled.title = String::new();
        let mut self_dep = done(3);
        self_dep.deps = vec![id(3)];
        for (bad, says) in [
            (gated, "gate still blocks"),
            (untitled, ".title"),
            (self_dep, "depend on itself"),
        ] {
            let error = render_archived(&ArchivedTasks::new(vec![bad], at)).unwrap_err();
            assert!(
                error.to_string().starts_with("tasks-archive.jsonl: "),
                "{error}"
            );
            assert!(error.to_string().contains(says), "{says}: {error}");
        }
        let twice = render_archived(&ArchivedTasks::new(vec![done(4), done(4)], at)).unwrap_err();
        assert!(twice.to_string().contains("unique"), "{twice}");
        let mut huge: Vec<Task> = (0..=u32::try_from(MAX_TASKS).unwrap())
            .map(|n| Task {
                id: wide_id(n),
                ..done(1)
            })
            .collect();
        let error = render_archived(&ArchivedTasks::new(huge.clone(), at)).unwrap_err();
        assert!(error.to_string().contains("at most 10000"), "{error}");
        huge.pop();
        assert!(render_archived(&ArchivedTasks::new(huge, at)).is_ok());
        let mut elsewhere = done(5);
        elsewhere.deps = vec![id(9)];
        assert!(
            render_archived(&ArchivedTasks::new(vec![elsewhere], at)).is_ok(),
            "a dependency may be in the ledger or on an earlier line"
        );
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
