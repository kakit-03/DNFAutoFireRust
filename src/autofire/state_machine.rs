// Defines shared autofire state-machine data and scheduling helpers.

use super::command_queue::{CommandQueue, CommandSource, QueuedCommand};
use super::runtime::{
    RuntimeCombo, RuntimeComboStep, RuntimeHotkey, RuntimeLinkedKey, RuntimeProfile,
};
use crate::config::LinkedTriggerMode;
use crate::input::backend::{InputBackend, InputSnapshot};
use crate::keymap::KeySpec;
use crate::timing::{HighPrecisionSleeper, SleepTimingSnapshot};
use std::collections::HashSet;
use std::time::{Duration, Instant};

mod combo;
mod linked;
mod repeat;

#[cfg(test)]
pub(super) use combo::next_combo_step_ready_at;
pub(super) use combo::{drive_combo_state_machines, on_command_executed, sync_combo_inputs};
pub(super) use linked::{drive_linked_bindings, sync_linked_inputs};
pub(super) use repeat::{
    build_repeat_bindings, drive_repeat_state_machines, sync_repeat_binding_inputs,
};

#[derive(Clone)]
pub(super) struct RuntimeRepeatBinding {
    pub(super) key: KeySpec,
    pub(super) trigger: RepeatTrigger,
    pub(super) repeat_interval: Duration,
    pub(super) press_duration: Duration,
}

#[derive(Clone)]
pub(super) enum RepeatTrigger {
    HoldKey(u16),
    ToggleHotkey(RuntimeHotkey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RepeatPhase {
    Idle,
    Queued,
    Recovering { until: Instant },
}

pub(super) struct RepeatBindingState {
    pub(super) trigger_down: bool,
    pub(super) enabled: bool,
    pub(super) phase: RepeatPhase,
}

impl Default for RepeatBindingState {
    fn default() -> Self {
        Self {
            trigger_down: false,
            enabled: false,
            phase: RepeatPhase::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComboPhase {
    Idle,
    Ready {
        step_index: usize,
        ready_at: Instant,
    },
    Queued {
        step_index: usize,
    },
    Recovering {
        next_index: usize,
        until: Instant,
    },
}

pub(super) struct ComboState {
    pub(super) trigger_down: bool,
    pub(super) phase: ComboPhase,
}

impl Default for ComboState {
    fn default() -> Self {
        Self {
            trigger_down: false,
            phase: ComboPhase::Idle,
        }
    }
}

#[derive(Default)]
pub(super) struct LinkedBindingState {
    pub(super) trigger_down: bool,
}

pub(super) fn effective_poll_duration(timing: SleepTimingSnapshot) -> Duration {
    Duration::from_millis(timing.scheduler_interval_ms.max(1))
}

fn configured_interval_duration(base_ms: u64) -> Duration {
    Duration::from_millis(base_ms.max(1))
}

fn configured_press_duration(base_ms: u64) -> Duration {
    Duration::from_millis(base_ms.max(1))
}

pub(super) fn execute_ready_command(
    command: QueuedCommand,
    input_backend: &mut dyn InputBackend,
    sleeper: &HighPrecisionSleeper,
    repeat_bindings: &[RuntimeRepeatBinding],
    repeat_states: &mut [RepeatBindingState],
    combos: &[RuntimeCombo],
    combo_states: &mut [ComboState],
) {
    input_backend.send_key_once(command.key, command.press_duration, sleeper);
    on_command_executed(
        command,
        Instant::now(),
        repeat_bindings,
        repeat_states,
        combos,
        combo_states,
    );
}

pub(super) fn reset_runtime_states_for_pause(
    repeat_bindings: &[RuntimeRepeatBinding],
    repeat_states: &mut [RepeatBindingState],
    combo_states: &mut [ComboState],
    command_queue: &mut CommandQueue,
) {
    command_queue.clear();

    for (binding, state) in repeat_bindings.iter().zip(repeat_states.iter_mut()) {
        if matches!(binding.trigger, RepeatTrigger::HoldKey(_)) {
            state.enabled = false;
        }
        state.phase = RepeatPhase::Idle;
    }

    for state in combo_states {
        state.phase = ComboPhase::Idle;
    }
}

pub(super) fn next_runtime_deadline(
    repeat_states: &[RepeatBindingState],
    combo_states: &[ComboState],
    command_queue: &CommandQueue,
) -> Option<Instant> {
    let mut next_deadline = command_queue.next_ready_at();

    for state in repeat_states {
        if let RepeatPhase::Recovering { until } = state.phase {
            merge_earlier_deadline(&mut next_deadline, until);
        }
    }

    for state in combo_states {
        match state.phase {
            ComboPhase::Ready { ready_at, .. } => {
                merge_earlier_deadline(&mut next_deadline, ready_at)
            }
            ComboPhase::Recovering { until, .. } => {
                merge_earlier_deadline(&mut next_deadline, until)
            }
            ComboPhase::Idle | ComboPhase::Queued { .. } => {}
        }
    }

    next_deadline
}

fn merge_earlier_deadline(next_deadline: &mut Option<Instant>, candidate: Instant) {
    match next_deadline {
        Some(current) if candidate >= *current => {}
        _ => *next_deadline = Some(candidate),
    }
}

fn hotkey_is_down(hotkey: &RuntimeHotkey, input_snapshot: &InputSnapshot) -> bool {
    hotkey
        .specs
        .iter()
        .all(|spec| input_snapshot.is_down(spec.vk))
}
