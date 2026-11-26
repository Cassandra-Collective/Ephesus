use std::time::{Duration, Instant};

const FIRST_KEY: Duration = Duration::from_millis(150);
const CONFIRM: Duration = Duration::from_secs(2);
const MAX_KEYS: u8 = 4;

/// A current engine observation used to decide when operator input is ready.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CommandPhase {
    /// A newly started process has yet to report a phase.
    #[default]
    Unknown,
    /// The process is ready for text.
    Idle,
    /// The last turn has finished, before the idle observation.
    Success,
    /// The process cannot take a command yet.
    Busy(String),
}

/// Operator text still held for a ready observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldCommand {
    /// Text entered so far, joined with newlines.
    pub text: String,
    /// The current reason the command remains held.
    pub reason: String,
}

pub(crate) enum SubmitKey {
    Press,
    Unconfirmed(u8),
}

#[derive(Default)]
pub(crate) struct Commands {
    phase: CommandPhase,
    held: Option<HeldCommand>,
    pending: Option<(Instant, u8)>,
}

impl Commands {
    pub(crate) fn held(&self) -> Option<&HeldCommand> {
        self.held.as_ref()
    }

    pub(crate) fn text(&mut self, text: String, writable: bool) -> Option<String> {
        match &self.phase {
            CommandPhase::Idle | CommandPhase::Success if writable => {
                Some(match self.held.take() {
                    Some(held) => join(held.text, &text),
                    None => text,
                })
            }
            CommandPhase::Idle | CommandPhase::Success => {
                self.hold(text, "the terminal is still writing input".into())
            }
            CommandPhase::Unknown => self.hold(text, "the engine phase is not yet known".into()),
            CommandPhase::Busy(reason) => self.hold(text, reason.clone()),
        }
    }

    fn hold(&mut self, text: String, reason: String) -> Option<String> {
        let text = match self.held.take() {
            Some(held) => join(held.text, &text),
            None => text,
        };
        self.held = Some(HeldCommand { text, reason });
        None
    }

    pub(crate) fn observe(&mut self, phase: CommandPhase) {
        self.phase = phase;
    }

    pub(crate) fn take_ready(&mut self) -> Option<String> {
        if matches!(self.phase, CommandPhase::Idle | CommandPhase::Success) {
            return self.held.take().map(|held| held.text);
        }
        None
    }

    pub(crate) fn sent(&mut self, now: Instant) {
        self.pending = Some((now + FIRST_KEY, 0));
    }

    pub(crate) fn due(&mut self, now: Instant) -> Option<SubmitKey> {
        let (due, keys) = self.pending?;
        if now < due {
            return None;
        }
        if keys == MAX_KEYS {
            self.pending = None;
            return Some(SubmitKey::Unconfirmed(keys));
        }
        self.pending = Some((now + CONFIRM, keys + 1));
        Some(SubmitKey::Press)
    }

    pub(crate) fn accepted(&mut self) {
        self.pending = None;
    }

    pub(crate) fn key_written(&mut self, now: Instant) {
        if let Some((deadline, _)) = &mut self.pending {
            *deadline = now + CONFIRM;
        }
    }

    pub(crate) fn interrupt(&mut self) {
        self.held = None;
        self.pending = None;
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(due, _)| due)
    }
}

fn join(mut held: String, text: &str) -> String {
    held.push('\n');
    held.push_str(text);
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_multiple_inputs_then_waits_between_text_and_each_submit_key() {
        let now = Instant::now();
        let mut queue = Commands::default();
        assert_eq!(queue.text("one".into(), true), None);
        assert_eq!(queue.text("two".into(), true), None);
        assert_eq!(queue.held().unwrap().text, "one\ntwo");
        queue.observe(CommandPhase::Success);
        assert_eq!(queue.take_ready().unwrap(), "one\ntwo");
        queue.sent(now);
        let start = now + FIRST_KEY;
        assert!(queue.due(start - Duration::from_millis(1)).is_none());
        for key in 0..4 {
            let at = start + CONFIRM * key;
            assert!(matches!(queue.due(at), Some(SubmitKey::Press)));
            assert!(queue.due(at).is_none());
        }
        assert!(matches!(
            queue.due(start + CONFIRM * 4),
            Some(SubmitKey::Unconfirmed(4))
        ));
        assert_eq!(queue.deadline(), None);
    }

    #[test]
    fn escape_clears_held_input_and_every_pending_submit() {
        let now = Instant::now();
        let mut queue = Commands::default();
        queue.sent(now);
        assert!(queue.text("stop here".into(), true).is_none());
        queue.interrupt();
        assert!(queue.held().is_none());
        queue.observe(CommandPhase::Idle);
        assert!(queue.take_ready().is_none());
        assert!(queue.due(now + Duration::from_secs(10)).is_none());
    }

    #[test]
    fn an_acknowledgement_or_new_send_disarms_the_old_schedule() {
        let now = Instant::now();
        let mut queue = Commands::default();
        queue.sent(now);
        queue.sent(now + Duration::from_millis(1));
        assert!(queue.due(now + FIRST_KEY).is_none());
        queue.accepted();
        queue.accepted();
        assert!(queue.due(now + Duration::from_secs(10)).is_none());
    }

    #[test]
    fn ready_text_waits_for_the_current_write_then_joins_new_input() {
        let mut queue = Commands::default();
        queue.observe(CommandPhase::Idle);
        assert_eq!(queue.text("one".into(), false), None);
        assert_eq!(
            queue.held(),
            Some(&HeldCommand {
                text: "one".into(),
                reason: "the terminal is still writing input".into(),
            })
        );
        assert_eq!(queue.text("two".into(), true).as_deref(), Some("one\ntwo"));
        assert!(queue.held().is_none());
    }

    #[test]
    fn the_submit_deadline_moves_to_the_key_acknowledgement() {
        let now = Instant::now();
        let mut queue = Commands::default();
        queue.sent(now);
        let first = now + FIRST_KEY;
        assert_eq!(queue.deadline(), Some(first));
        assert!(matches!(queue.due(first), Some(SubmitKey::Press)));
        let provisional = first + CONFIRM;
        assert_eq!(queue.deadline(), Some(provisional));

        let acknowledgement = first + Duration::from_millis(500);
        queue.key_written(acknowledgement);
        assert_eq!(queue.deadline(), Some(acknowledgement + CONFIRM));
        assert!(queue.due(provisional).is_none());
        assert!(matches!(
            queue.due(acknowledgement + CONFIRM),
            Some(SubmitKey::Press)
        ));
    }
}
