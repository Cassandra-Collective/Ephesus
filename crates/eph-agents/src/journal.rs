//! Off-owner durable lifecycle events and degradation writes.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use eph_agora::agora::Agora;
use eph_fs::home::Condition;
use eph_schema::{degradation::Cause, lifecycle::LifecycleRecord, names::AgentId};

use crate::Error;

#[derive(Clone)]
pub(crate) struct Journal {
    queue: Sender<Request>,
    pending: Arc<AtomicUsize>,
}

enum Request {
    Record(LifecycleRecord, Option<Sender<Result<u64, Error>>>),
    Report(Cause, String),
    Clear(Cause),
    Condition(Condition),
}

pub(crate) struct Notice {
    pub(crate) agent: AgentId,
    pub(crate) detail: String,
}

impl Journal {
    pub(crate) fn open(agora: Arc<Agora>) -> io::Result<(Self, Receiver<Notice>)> {
        let (queue, requests) = mpsc::channel();
        let (notices, reports) = mpsc::channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let outstanding = Arc::clone(&pending);
        let _worker = thread::Builder::new()
            .name("eph-agents-journal".into())
            .spawn(move || {
                for request in requests {
                    match request {
                        Request::Record(record, reply) => {
                            let written = agora
                                .append_lifecycle(&record)
                                .map(|appended| {
                                    agora.committer().commit_soon("agents: record lifecycle");
                                    appended.line.seq
                                })
                                .map_err(|error| {
                                    Error::Refused(format!(
                                        "lifecycle record was not acknowledged: {error}"
                                    ))
                                });
                            if let Some(reply) = reply {
                                let _delivered = reply.send(written);
                            } else if let Err(error) = written
                                && notices
                                    .send(Notice {
                                        agent: record.agent,
                                        detail: error.to_string(),
                                    })
                                    .is_ok()
                            {
                                // The request stays outstanding until the owner sees the failure.
                                continue;
                            }
                        }
                        Request::Report(cause, detail) => {
                            agora.degradations().report(cause, &detail)
                        }
                        Request::Clear(cause) => agora.degradations().clear(&cause),
                        Request::Condition(condition) => {
                            agora.degradations().report_condition(&condition)
                        }
                    }
                    outstanding.fetch_sub(1, Ordering::AcqRel);
                }
            })?;
        Ok((Self { queue, pending }, reports))
    }

    fn send(&self, request: Request) -> Result<(), Error> {
        self.pending.fetch_add(1, Ordering::AcqRel);
        if self.queue.send(request).is_err() {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return Err(Error::Refused("the lifecycle journal worker ended".into()));
        }
        Ok(())
    }

    pub(crate) fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire) != 0
    }

    pub(crate) fn notice_received(&self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }

    pub(crate) fn record(&self, record: LifecycleRecord) -> Result<(), Error> {
        self.send(Request::Record(record, None))
    }

    /// Wait for a record from a worker effect. The event owner only enqueues.
    pub(crate) fn acknowledge(&self, record: LifecycleRecord) -> Result<u64, Error> {
        let (reply, acknowledgement) = mpsc::channel();
        self.send(Request::Record(record, Some(reply)))?;
        acknowledgement.recv().map_err(|_| {
            Error::Refused("the lifecycle journal did not return its acknowledgement".into())
        })?
    }

    pub(crate) fn report(&self, cause: Cause, detail: String) -> Result<(), Error> {
        self.send(Request::Report(cause, detail))
    }

    pub(crate) fn clear(&self, cause: Cause) -> Result<(), Error> {
        self.send(Request::Clear(cause))
    }

    pub(crate) fn condition(&self, condition: Condition) -> Result<(), Error> {
        self.send(Request::Condition(condition))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use eph_git::committer::Hooks;
    use eph_schema::{degradation::Source, lifecycle::LifecycleEvent, registry::EngineId};

    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn acknowledgement_orders_degradation_reports_and_clears() {
        let scratch = Scratch::new();
        let prompts = scratch.0.join("prompts");
        fs::create_dir(&prompts).unwrap();
        let agora = Arc::new(
            Agora::open(&scratch.0.join("agora"), &prompts, Hooks::none())
                .unwrap()
                .agora,
        );
        let (journal, notices) = Journal::open(Arc::clone(&agora)).unwrap();
        let agent: AgentId = "agent.fixture".parse().unwrap();
        let report = Cause {
            source: Source::Agents,
            slug: "tools".into(),
            subject: Some(agent.to_string()),
        };
        let condition = Condition {
            cause: Cause {
                source: Source::Home,
                slug: "policy-behind".into(),
                subject: Some("fixture.md".into()),
            },
            detail: "fixture policy requires an update".into(),
        };
        let barrier = || {
            journal
                .acknowledge(LifecycleRecord {
                    agent: agent.clone(),
                    attempt: None,
                    event: LifecycleEvent::Requested {
                        engine: EngineId::Claude,
                        workspace: scratch.0.display().to_string(),
                        grants: Vec::new(),
                    },
                })
                .unwrap()
        };

        journal
            .report(report.clone(), "fixture tool is missing".into())
            .unwrap();
        journal.condition(condition.clone()).unwrap();
        let reported = barrier();
        let active = agora.degradations().list();
        assert!(
            active
                .iter()
                .any(|entry| entry.cause == report && entry.detail == "fixture tool is missing")
        );
        assert!(
            active
                .iter()
                .any(|entry| entry.cause == condition.cause && entry.detail == condition.detail)
        );

        journal.clear(report.clone()).unwrap();
        assert!(barrier() > reported);
        assert!(!agora.degradations().has(&report));
        assert!(agora.degradations().has(&condition.cause));
        drop(journal);
        assert!(matches!(
            notices.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        agora.committer().drained();
    }
}
