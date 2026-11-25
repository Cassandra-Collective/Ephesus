use std::time::{Duration, Instant};

use eph_schema::lifecycle::LifecycleEvent;

/// What the declared hire asks the manager to do after a session ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExitPolicy {
    /// Keep the hire down until an operator takes the offer.
    #[default]
    Offer,
    /// Use the bounded crew ladder.
    Crew,
    /// Use the bounded orchestrator ladder when its caller is wired in R7.
    Orchestrator,
}

impl ExitPolicy {
    fn timings(self) -> (&'static [u64], Duration) {
        match self {
            Self::Offer => (&[], Duration::ZERO),
            Self::Crew => (&[5_000, 30_000, 120_000], Duration::from_secs(300)),
            Self::Orchestrator => (
                &[1_000, 2_000, 5_000, 15_000, 30_000],
                Duration::from_secs(60),
            ),
        }
    }
}

pub(crate) enum Due {
    Launch(usize),
    Held(LifecycleEvent),
}

#[derive(Clone, Default)]
pub(crate) struct Ladder {
    spent: usize,
    running_since: Option<Instant>,
    pending: Option<(Instant, bool)>,
    held: Option<String>,
    deferred: bool,
}

impl Ladder {
    pub(crate) fn running(&mut self, now: Instant) {
        self.running_since = Some(now);
    }

    pub(crate) fn exited(&mut self, now: Instant, policy: ExitPolicy) {
        let (_, stability) = policy.timings();
        if self
            .running_since
            .take()
            .is_some_and(|then| now.saturating_duration_since(then) >= stability)
        {
            self.spent = 0;
        }
    }

    pub(crate) fn schedule(&mut self, now: Instant, policy: ExitPolicy) -> Option<LifecycleEvent> {
        if policy == ExitPolicy::Offer || self.pending.is_some() {
            return None;
        }
        if let Some(reason) = &self.held {
            self.deferred = true;
            return Some(LifecycleEvent::Deferred {
                rung: self.spent,
                reason: reason.clone(),
            });
        }
        let (delays, _) = policy.timings();
        let Some(wait_ms) = delays.get(self.spent).copied() else {
            return Some(LifecycleEvent::Exhausted {
                rung: self.spent,
                reason: "the respawn ladder is spent".into(),
            });
        };
        self.spent += 1;
        self.pending = Some((now + Duration::from_millis(wait_ms), true));
        Some(LifecycleEvent::Scheduled {
            rung: self.spent,
            wait_ms,
        })
    }

    pub(crate) fn hold(&mut self, reason: String) -> Option<LifecycleEvent> {
        if self.held.is_some() {
            return None;
        }
        self.held = Some(reason.clone());
        Some(LifecycleEvent::Held {
            rung: self.spent,
            reason,
        })
    }

    pub(crate) fn capacity(&self) -> Option<&str> {
        self.held.as_deref()
    }

    pub(crate) fn release(&mut self, now: Instant) -> Option<LifecycleEvent> {
        self.held.take()?;
        if self.deferred && self.pending.is_none() {
            self.deferred = false;
            self.pending = Some((now, false));
            return Some(LifecycleEvent::Released { rung: self.spent });
        }
        None
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(at, _)| at)
    }

    pub(crate) fn due(&mut self, now: Instant) -> Option<Due> {
        let (at, charged) = self.pending?;
        if now < at {
            return None;
        }
        self.pending = None;
        if let Some(reason) = &self.held {
            if charged {
                self.spent -= 1;
            }
            self.deferred = true;
            return Some(Due::Held(LifecycleEvent::Deferred {
                rung: self.spent,
                reason: reason.clone(),
            }));
        }
        Some(Due::Launch(self.spent))
    }

    pub(crate) fn manual(&mut self) {
        if self.pending.take().is_some_and(|(_, charged)| charged) {
            self.spent -= 1;
        }
        self.deferred = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_restart_does_not_reset_the_crew_ladder() {
        let mut now = Instant::now();
        let mut ladder = Ladder::default();
        for (index, wait_ms) in [5_000, 30_000, 120_000].into_iter().enumerate() {
            assert_eq!(
                ladder.schedule(now, ExitPolicy::Crew),
                Some(LifecycleEvent::Scheduled {
                    rung: index + 1,
                    wait_ms
                })
            );
            assert!(ladder.due(now).is_none());
            now += Duration::from_millis(wait_ms);
            assert!(matches!(ladder.due(now), Some(Due::Launch(rung)) if rung == index + 1));
            ladder.running(now);
            now += Duration::from_secs(1);
            ladder.exited(now, ExitPolicy::Crew);
        }
        assert!(matches!(
            ladder.schedule(now, ExitPolicy::Crew),
            Some(LifecycleEvent::Exhausted { rung: 3, .. })
        ));
        ladder.running(now);
        now += Duration::from_secs(300);
        ladder.exited(now, ExitPolicy::Crew);
        assert!(matches!(
            ladder.schedule(now, ExitPolicy::Crew),
            Some(LifecycleEvent::Scheduled { rung: 1, .. })
        ));
    }

    #[test]
    fn a_hold_and_manual_restart_do_not_spend_pending_rungs() {
        let now = Instant::now();
        let mut ladder = Ladder::default();
        assert!(ladder.schedule(now, ExitPolicy::Crew).is_some());
        assert!(ladder.hold("capacity".into()).is_some());
        let at = now + Duration::from_secs(5);
        assert!(matches!(
            ladder.due(at),
            Some(Due::Held(LifecycleEvent::Deferred { rung: 0, .. }))
        ));
        assert_eq!(
            ladder.release(at),
            Some(LifecycleEvent::Released { rung: 0 })
        );
        assert!(matches!(ladder.due(at), Some(Due::Launch(0))));
        assert!(ladder.schedule(at, ExitPolicy::Crew).is_some());
        ladder.manual();
        assert_eq!(ladder.deadline(), None);
        assert!(matches!(
            ladder.schedule(at, ExitPolicy::Crew),
            Some(LifecycleEvent::Scheduled { rung: 1, .. })
        ));
    }
}
