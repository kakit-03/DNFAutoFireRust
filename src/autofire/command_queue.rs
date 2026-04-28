// Queues ready keyboard commands by runtime source and scheduled time.

use crate::keymap::KeySpec;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommandSource {
    Repeat(usize),
    Combo(usize),
    Linked(usize),
}

#[derive(Clone, Copy)]
pub(super) struct QueuedCommand {
    pub(super) source: CommandSource,
    pub(super) key: KeySpec,
    pub(super) ready_at: Instant,
    pub(super) press_duration: Duration,
}

#[derive(Default)]
pub(super) struct CommandQueue {
    pending: Vec<QueuedCommand>,
}

impl CommandQueue {
    pub(super) fn clear(&mut self) {
        self.pending.clear();
    }

    pub(super) fn enqueue(&mut self, command: QueuedCommand) {
        self.pending.push(command);
        self.pending.sort_by_key(|item| item.ready_at);
    }

    pub(super) fn cancel_source(&mut self, source: CommandSource) {
        self.pending.retain(|item| item.source != source);
    }

    pub(super) fn next_ready_at(&self) -> Option<Instant> {
        self.pending.iter().map(|item| item.ready_at).min()
    }

    pub(super) fn pop_next_ready(&mut self, now: Instant) -> Option<QueuedCommand> {
        let next_index = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, item)| item.ready_at <= now)
            .min_by_key(|(_, item)| item.ready_at)
            .map(|(index, _)| index)?;
        Some(self.pending.remove(next_index))
    }
}
