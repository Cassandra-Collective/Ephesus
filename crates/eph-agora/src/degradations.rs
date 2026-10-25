//! `Degradations`: the degradation channel, the one place a give-up becomes
//! visible (invariant 7; SDD-RUST §4.5).
//!
//! Its shape is the Architect's of 2026-09-03, carried from the TypeScript
//! build (`src/main/degradations.ts`), and cited rather than re-decided
//! (DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions*):
//! - **One entry per cause, not per occurrence.** A repeat raises the count and
//!   refreshes the detail, never splits the entry and never moves it. The ring
//!   keeps [`LIMIT`] causes and drops the oldest first, so a noisy cause cannot
//!   push a quiet one out: the cap is on how many things are wrong.
//! - **The book of record gets a bounded ladder**: a line at the first
//!   occurrence, at every power of ten, and at the clear
//!   (`eph_schema::degradation::earns_log_line`). The count in the ring is
//!   exact.
//! - **A restart replays what was true when the harness stopped**, marked
//!   [`Freshness::Carried`] ([`Degradations::replay`]): a cleared cause stays
//!   gone, and a live report replaces a carried entry. A replay writes nothing.
//! - **Reporting never fails.** A line the book cannot take is noted in the
//!   ring as `agora/log-unwritable`, without another append, and a rotation the
//!   book skipped is reported as `agora/log-rotation`, whose own skipped
//!   rotation is only counted, so neither can recurse.
//!
//! Lock order: the ring, then the book's writer. Nothing holds either while it
//! waits on a commit, so the committer's thread may report here.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use eph_fs::append::Rotation;
use eph_fs::home::Condition;
use eph_schema::degradation::{Cause, Degradation, DegradationEvent, Source, earns_log_line};
use eph_schema::log::{LogKind, LogLine};
use eph_schema::time::{Duration, Timestamp};

use crate::book::{AppendError, Appended, Book, Draft};

/// How many causes the ring keeps (today's figure).
pub const LIMIT: usize = 50;

/// How many of the book's last lines a boot replays (today's figure,
/// `src/main/index.ts`'s `DEGRADATION_REPLAY_LIMIT`).
pub const REPLAY_LINES: usize = 400;

/// The longest detail a row carries (`eph-schema`'s rule).
pub const MAX_DETAIL: usize = 2000;

/// Whether an entry was seen by this run or came back from the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Reported by this run.
    Live,
    /// True when the harness last stopped, and not checked again since.
    Carried,
}

/// One entry of the ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What it is.
    pub cause: Cause,
    /// The latest wording.
    pub detail: String,
    /// How many times it has occurred.
    pub count: u64,
    /// When it first occurred.
    pub since: Timestamp,
    /// When it last occurred.
    pub last_seen: Timestamp,
    /// Live or carried.
    pub freshness: Freshness,
}

type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The channel. See the module contract.
pub struct Degradations {
    book: Arc<Book>,
    ring: Mutex<Vec<Entry>>,
    clock: Clock,
}

impl fmt::Debug for Degradations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Degradations")
            .field("causes", &guard(&self.ring).len())
            .finish_non_exhaustive()
    }
}

impl Degradations {
    /// A channel writing to `book`, on the system clock.
    pub fn new(book: Arc<Book>) -> Degradations {
        Degradations::with_clock(book, Timestamp::now)
    }

    /// A channel on the clock a test chose.
    pub fn with_clock(
        book: Arc<Book>,
        clock: impl Fn() -> Timestamp + Send + Sync + 'static,
    ) -> Degradations {
        Degradations {
            book,
            ring: Mutex::new(Vec::new()),
            clock: Arc::new(clock),
        }
    }

    /// Records that `cause` holds, with the current wording.
    pub fn report(&self, cause: Cause, detail: &str) {
        let mut ring = guard(&self.ring);
        self.raise(&mut ring, cause, detail);
    }

    /// Records a condition the boot or the home found.
    pub fn report_condition(&self, condition: &Condition) {
        self.report(condition.cause.clone(), &condition.detail);
    }

    /// Records that `cause` no longer holds, with a line saying how long it
    /// held. A cause that is not in the ring is not news, and writes nothing.
    pub fn clear(&self, cause: &Cause) {
        self.clear_where(|held| held == cause);
    }

    /// Clears every cause `matches` accepts, oldest first.
    pub fn clear_where(&self, matches: impl Fn(&Cause) -> bool) {
        let mut ring = guard(&self.ring);
        let mut kept = Vec::with_capacity(ring.len());
        let mut cleared = Vec::new();
        for entry in ring.drain(..) {
            if matches(&entry.cause) {
                cleared.push(entry);
            } else {
                kept.push(entry);
            }
        }
        *ring = kept;
        let now = (self.clock)();
        for entry in cleared {
            let held = now
                .as_millis()
                .saturating_sub(entry.since.as_millis())
                .max(0);
            let row = Degradation {
                cause: entry.cause,
                detail: entry.detail,
                count: entry.count,
                since: entry.since,
                cleared_after: Duration::from_millis(held),
            };
            self.write(&mut ring, &row, now);
        }
    }

    /// Whether `cause` holds.
    pub fn has(&self, cause: &Cause) -> bool {
        guard(&self.ring).iter().any(|entry| &entry.cause == cause)
    }

    /// Every entry, oldest first, so a reader reads "since when" top down.
    pub fn list(&self) -> Vec<Entry> {
        guard(&self.ring).clone()
    }

    /// Rebuilds the ring from the book's lines, oldest first, as carried. A
    /// cleared row removes its cause, and nothing this run reported is
    /// replaced. Writes nothing.
    pub fn replay(&self, lines: &[LogLine]) {
        let mut ring = guard(&self.ring);
        for line in lines {
            if line.kind != LogKind::Degradation {
                continue;
            }
            let Ok(row) = Degradation::from_line(line) else {
                continue;
            };
            let at = ring.iter().position(|entry| entry.cause == row.cause);
            if row.event() == DegradationEvent::Cleared {
                if let Some(at) = at {
                    ring.remove(at);
                }
                continue;
            }
            let entry = Entry {
                cause: row.cause,
                detail: row.detail,
                count: row.count,
                since: row.since,
                last_seen: line.ts,
                freshness: Freshness::Carried,
            };
            match at {
                Some(at) if ring[at].freshness == Freshness::Live => {}
                Some(at) => ring[at] = entry,
                None => {
                    ring.push(entry);
                    evict(&mut ring);
                }
            }
        }
    }

    /// [`Degradations::report`] with the ring held.
    fn raise(&self, ring: &mut Vec<Entry>, cause: Cause, detail: &str) {
        let now = (self.clock)();
        let detail = fit(detail);
        let row = match ring.iter_mut().find(|entry| entry.cause == cause) {
            Some(entry) => {
                entry.count += 1;
                entry.detail.clone_from(&detail);
                entry.last_seen = now;
                entry.freshness = Freshness::Live;
                if !earns_log_line(entry.count) {
                    return;
                }
                row_of(entry)
            }
            None => {
                let entry = Entry {
                    cause,
                    detail,
                    count: 1,
                    since: now,
                    last_seen: now,
                    freshness: Freshness::Live,
                };
                let row = row_of(&entry);
                ring.push(entry);
                evict(ring);
                row
            }
        };
        self.write(ring, &row, now);
    }

    /// Appends any other kind of line to the book, reporting a rotation the
    /// book had to skip: `Agora::append`'s one path.
    pub(crate) fn append(&self, draft: Draft) -> Result<Appended, AppendError> {
        let mut ring = guard(&self.ring);
        let now = (self.clock)();
        self.append_held(&mut ring, draft, false, now)
    }

    /// Appends `row`'s line. A failure is noted.
    fn write(&self, ring: &mut Vec<Entry>, row: &Degradation, now: Timestamp) {
        let appended = row
            .to_line(1, now)
            .map_err(|error| error.to_string())
            .and_then(|line| {
                let draft = Draft {
                    kind: line.kind,
                    event: line.event,
                    source: line.source,
                    refs: line.refs,
                    data: line.data,
                };
                let rotation_row = row.cause == agora("log-rotation");
                self.append_held(ring, draft, rotation_row, now)
                    .map_err(|error| error.to_string())
            });
        if let Err(why) = appended {
            note(
                ring,
                agora("log-unwritable"),
                &format!(
                    "a degradation line could not be written to the book of record ({why}); the channel still holds it"
                ),
                now,
            );
        }
    }

    /// Appends `draft` with the ring held. A rotation the book skipped is
    /// raised, or only counted when the line is that rotation's own report, so
    /// it cannot recurse.
    fn append_held(
        &self,
        ring: &mut Vec<Entry>,
        draft: Draft,
        rotation_row: bool,
        now: Timestamp,
    ) -> Result<Appended, AppendError> {
        let appended = self.book.append(draft)?;
        if let Rotation::Skipped(why) = &appended.rotation {
            let rotation = agora("log-rotation");
            let detail = rotation_detail(why);
            if rotation_row {
                note(ring, rotation, &detail, now);
            } else {
                self.raise(ring, rotation, &detail);
            }
        }
        Ok(appended)
    }
}

/// The row a ring entry writes.
fn row_of(entry: &Entry) -> Degradation {
    Degradation {
        cause: entry.cause.clone(),
        detail: entry.detail.clone(),
        count: entry.count,
        since: entry.since,
        cleared_after: None,
    }
}

/// Records `cause` in the ring without writing a line. An entry already held
/// keeps its wording, which may be the boot's fuller account of the same
/// cause (`agora/log-unwritable`); only its count moves.
fn note(ring: &mut Vec<Entry>, cause: Cause, detail: &str, now: Timestamp) {
    let detail = fit(detail);
    if let Some(entry) = ring.iter_mut().find(|entry| entry.cause == cause) {
        entry.count += 1;
        entry.last_seen = now;
        entry.freshness = Freshness::Live;
        return;
    }
    ring.push(Entry {
        cause,
        detail,
        count: 1,
        since: now,
        last_seen: now,
        freshness: Freshness::Live,
    });
    evict(ring);
}

/// Drops the oldest causes past [`LIMIT`].
fn evict(ring: &mut Vec<Entry>) {
    let surplus = ring.len().saturating_sub(LIMIT);
    ring.drain(..surplus);
}

/// `detail` cut to [`MAX_DETAIL`] characters, saying it was cut, and never
/// empty, so a row always validates.
fn fit(detail: &str) -> String {
    const CUT: &str = " … (cut)";
    if detail.is_empty() {
        return "(no detail was given)".to_string();
    }
    if detail.chars().count() <= MAX_DETAIL {
        return detail.to_string();
    }
    let keep = MAX_DETAIL - CUT.chars().count();
    let mut cut: String = detail.chars().take(keep).collect();
    cut.push_str(CUT);
    cut
}

/// The detail of `agora/log-rotation`.
fn rotation_detail(why: &str) -> String {
    format!(
        "the book of record was due to rotate and could not ({why}); its lines still land in log.jsonl, which keeps growing"
    )
}

/// An `agora/` cause with no subject.
pub(crate) fn agora(slug: &str) -> Cause {
    Cause {
        source: Source::Agora,
        slug: slug.to_string(),
        subject: None,
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicI64, Ordering};

    use eph_fs::atomic::write_atomic;

    use super::*;
    use crate::book::tests::scratch;
    use crate::book::{Cursor, LOG_FILE};

    /// A channel on a clock the test moves, writing to a book in `dir`.
    fn channel(dir: &Path) -> (Degradations, Arc<AtomicI64>) {
        let now = Arc::new(AtomicI64::new(1_000));
        let clock = Arc::clone(&now);
        let channel = Degradations::with_clock(Arc::new(Book::open(dir)), move || {
            Timestamp::from_millis(clock.load(Ordering::SeqCst)).unwrap()
        });
        (channel, now)
    }

    fn cause(text: &str) -> Cause {
        text.parse().unwrap()
    }

    /// Every degradation row in the book, oldest first.
    fn rows(channel: &Degradations) -> Vec<Degradation> {
        channel
            .book
            .since(Cursor::START)
            .unwrap()
            .entries
            .iter()
            .map(|line| Degradation::from_line(line).unwrap())
            .collect()
    }

    #[test]
    fn a_cause_is_one_entry_its_repeats_counted_and_the_ladder_writes_its_lines() {
        let dir = scratch("ladder");
        let (channel, now) = channel(&dir);
        for n in 1..=12 {
            now.store(1_000 + n, Ordering::SeqCst);
            channel.report(cause("agora/commit"), &format!("try {n}"));
        }
        let list = channel.list();
        assert_eq!(list.len(), 1, "one entry per cause");
        assert_eq!(list[0].count, 12);
        assert_eq!(list[0].detail, "try 12", "the latest wording");
        assert_eq!(list[0].since.as_millis(), 1_001, "since its first report");
        assert_eq!(list[0].last_seen.as_millis(), 1_012);
        assert_eq!(list[0].freshness, Freshness::Live);
        let written = rows(&channel);
        assert_eq!(
            written.iter().map(|row| row.count).collect::<Vec<_>>(),
            [1, 10],
            "a line at the first occurrence and at every power of ten"
        );
        assert_eq!(written[1].detail, "try 10");
        assert!(channel.has(&cause("agora/commit")));
        assert!(!channel.has(&cause("agora/commit:seed")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_clear_says_how_long_it_held_and_a_clear_of_nothing_writes_nothing() {
        let dir = scratch("clear");
        let (channel, now) = channel(&dir);
        channel.clear(&cause("agora/commit"));
        assert!(rows(&channel).is_empty(), "not degraded is not news");
        channel.report(cause("agora/commit"), "late");
        now.store(91_000, Ordering::SeqCst);
        channel.clear(&cause("agora/commit"));
        assert!(!channel.has(&cause("agora/commit")));
        let written = rows(&channel);
        assert_eq!(written.len(), 2);
        assert_eq!(written[1].event(), DegradationEvent::Cleared);
        assert_eq!(written[1].cleared_after, Duration::from_millis(90_000));
        assert_eq!(written[1].detail, "late");
        // A clock that went backwards still clears, held for no time.
        channel.report(cause("agora/commit"), "again");
        now.store(5_000, Ordering::SeqCst);
        channel.clear(&cause("agora/commit"));
        let last = rows(&channel).pop().unwrap();
        assert_eq!(last.event(), DegradationEvent::Cleared);
        assert_eq!(last.cleared_after, Duration::from_millis(0));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clear_where_clears_every_match_oldest_first_and_keeps_the_rest() {
        let dir = scratch("clear-where");
        let (channel, _now) = channel(&dir);
        for text in [
            "agora/commit:reconcile",
            "agora/hidden-work",
            "agora/commit",
            "agora/commit:seed",
        ] {
            channel.report(cause(text), text);
        }
        channel.clear_where(|held| held.slug == "commit");
        assert_eq!(
            channel
                .list()
                .iter()
                .map(|entry| entry.cause.to_string())
                .collect::<Vec<_>>(),
            ["agora/hidden-work"]
        );
        let cleared: Vec<String> = rows(&channel)
            .iter()
            .filter(|row| row.event() == DegradationEvent::Cleared)
            .map(|row| row.cause.to_string())
            .collect();
        assert_eq!(
            cleared,
            [
                "agora/commit:reconcile",
                "agora/commit",
                "agora/commit:seed"
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_ring_keeps_fifty_causes_and_drops_the_oldest_however_noisy_it_was() {
        let dir = scratch("ring");
        let (channel, _now) = channel(&dir);
        for n in 0..LIMIT {
            channel.report(cause(&format!("agora/c{n}")), "x");
        }
        for _ in 0..3_000 {
            channel.report(cause("agora/c0"), "noisy");
        }
        assert_eq!(
            channel.list()[0].cause,
            cause("agora/c0"),
            "a repeat never moves"
        );
        channel.report(cause("agora/newest"), "x");
        let list = channel.list();
        assert_eq!(list.len(), LIMIT);
        assert!(
            !channel.has(&cause("agora/c0")),
            "the oldest cause went first"
        );
        assert!(channel.has(&cause("agora/c1")), "a quiet cause stayed");
        assert_eq!(list[LIMIT - 1].cause, cause("agora/newest"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_replay_keeps_each_causes_newest_row_and_skips_other_kinds() {
        let dir = scratch("replay-newest");
        let (before, now) = channel(&dir);
        for n in 1..=10 {
            now.store(1_000 + n, Ordering::SeqCst);
            before.report(cause("agora/a"), &format!("a {n}"));
        }
        let mut lines = before.book.tail(REPLAY_LINES).unwrap().entries;
        assert_eq!(lines.len(), 2, "the ladder wrote counts 1 and 10");
        let mut other = lines[0].clone();
        other.kind = LogKind::Gym;
        lines.insert(1, other);
        let (after, _now) = channel(&dir);
        after.replay(&lines);
        let list = after.list();
        assert_eq!(list.len(), 1, "a line of another kind is not a degradation");
        assert_eq!(
            list[0].count, 10,
            "the later row replaced the earlier carried one"
        );
        assert_eq!(list[0].detail, "a 10");
        assert_eq!(list[0].freshness, Freshness::Carried);
        assert_eq!(list[0].since.as_millis(), 1_001);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_replay_carries_what_held_drops_what_cleared_and_never_replaces_what_is_live() {
        let dir = scratch("replay");
        let (before, now) = channel(&dir);
        before.report(cause("agora/a"), "a held");
        before.report(cause("agora/b"), "b held");
        now.store(3_000, Ordering::SeqCst);
        before.clear(&cause("agora/b"));
        before.report(cause("agora/c"), "c held");
        before.report(cause("agora/a"), "a again");
        let lines = before.book.tail(REPLAY_LINES).unwrap().entries;
        let (after, _now) = channel(&dir);
        after.report(cause("agora/c"), "c seen live");
        let appended = after.book.appends();
        after.replay(&lines);
        assert_eq!(after.book.appends(), appended, "a replay writes nothing");
        let list = after.list();
        assert_eq!(
            list.iter()
                .map(|entry| (entry.cause.to_string(), entry.freshness))
                .collect::<Vec<_>>(),
            [
                ("agora/c".to_string(), Freshness::Live),
                ("agora/a".to_string(), Freshness::Carried)
            ],
            "b was cleared before the stop; c was already seen live"
        );
        assert_eq!(list[0].detail, "c seen live");
        assert_eq!(
            list[1].detail, "a held",
            "the last line the book holds for it"
        );
        assert_eq!(list[1].count, 1);
        after.report(cause("agora/a"), "a still");
        let a = after
            .list()
            .into_iter()
            .find(|entry| entry.cause == cause("agora/a"))
            .unwrap();
        assert_eq!(a.freshness, Freshness::Live, "a live report makes it live");
        assert_eq!(a.count, 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_replay_skips_other_kinds_and_rows_it_cannot_read_and_keeps_fifty() {
        let dir = scratch("replay-many");
        let (before, _now) = channel(&dir);
        for n in 0..=LIMIT {
            before.report(cause(&format!("agora/c{n}")), "x");
        }
        let mut lines = before.book.tail(REPLAY_LINES).unwrap().entries;
        let mut bad = lines[0].clone();
        bad.data.insert("count".into(), serde_json::Value::from(0));
        lines.insert(0, bad);
        let (after, _now) = channel(&dir);
        after.replay(&lines);
        assert_eq!(after.list().len(), LIMIT);
        assert!(!after.has(&cause("agora/c0")), "the oldest went first");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_line_the_book_cannot_take_is_noted_without_another_append() {
        let dir = scratch("unwritable");
        fs::create_dir_all(dir.join(LOG_FILE).join("inside")).unwrap();
        let (channel, _now) = channel(&dir);
        channel.report(cause("agora/commit"), "late");
        channel.report(cause("agora/commit:seed"), "late too");
        let list = channel.list();
        assert_eq!(
            list.iter()
                .map(|entry| entry.cause.to_string())
                .collect::<Vec<_>>(),
            ["agora/commit", "agora/log-unwritable", "agora/commit:seed"],
            "the channel still holds what the book could not take"
        );
        assert_eq!(list[1].count, 2, "each refused line is counted");
        assert!(
            list[1].detail.contains("could not be written"),
            "{}",
            list[1].detail
        );
        assert_eq!(channel.book.appends(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_skipped_rotation_is_reported_and_its_own_skip_only_counted() {
        let dir = scratch("rotation");
        // A live file past its threshold whose segment name is already taken,
        // so every rotation is skipped: sealing onto it would rewrite history.
        Book::open(&dir)
            .append(crate::book::tests::draft(1))
            .unwrap();
        let archive = dir.join(crate::book::ARCHIVE_DIR);
        fs::create_dir_all(&archive).unwrap();
        write_atomic(&archive.join(eph_fs::append::segment_name(1)), b"history\n").unwrap();
        let book = Arc::new(Book::open_rotating_at(&dir, 1));
        let channel = Degradations::new(Arc::clone(&book));
        channel.report(cause("agora/commit"), "late");
        let list = channel.list();
        assert_eq!(
            list.iter()
                .map(|entry| entry.cause.to_string())
                .collect::<Vec<_>>(),
            ["agora/commit", "agora/log-rotation"]
        );
        assert_eq!(
            list[1].count, 2,
            "its own line's skip was counted, not written"
        );
        assert!(
            list[1].detail.contains("would rewrite history"),
            "{}",
            list[1].detail
        );
        assert_eq!(
            fs::read(archive.join(eph_fs::append::segment_name(1))).unwrap(),
            b"history\n"
        );
        assert_eq!(
            channel.book.appends(),
            2,
            "the report and the rotation's first line"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_detail_is_never_empty_and_never_longer_than_a_row_allows() {
        assert_eq!(fit(""), "(no detail was given)");
        assert_eq!(fit("short"), "short");
        let exact = "x".repeat(MAX_DETAIL);
        assert_eq!(fit(&exact), exact);
        let long = fit(&"é".repeat(MAX_DETAIL + 1));
        assert_eq!(long.chars().count(), MAX_DETAIL);
        assert!(long.ends_with(" … (cut)"));
        let dir = scratch("fit");
        let (channel, _now) = channel(&dir);
        channel.report(cause("agora/commit"), &"y".repeat(5_000));
        channel.report(cause("agora/empty"), "");
        let written = rows(&channel);
        assert_eq!(written.len(), 2, "both rows validated and were written");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// `MAX_DETAIL` repeats `eph-schema`'s rule for a row; this holds them to
    /// one number.
    #[test]
    fn the_channels_detail_limit_is_the_rows_own() {
        let row = |chars: usize| Degradation {
            cause: cause("agora/limit"),
            detail: "d".repeat(chars),
            count: 1,
            since: Timestamp::from_millis(1).unwrap(),
            cleared_after: None,
        };
        assert!(row(MAX_DETAIL).validate().is_ok());
        assert!(row(MAX_DETAIL + 1).validate().is_err());
    }

    #[test]
    fn a_condition_is_reported_by_its_cause() {
        let dir = scratch("condition");
        let (channel, _now) = channel(&dir);
        channel.report_condition(&Condition {
            cause: cause("home/policy-behind:gate-policy.json"),
            detail: "seeded from an older default".into(),
        });
        assert!(channel.has(&cause("home/policy-behind:gate-policy.json")));
        assert!(format!("{channel:?}").contains("causes: 1"), "{channel:?}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
