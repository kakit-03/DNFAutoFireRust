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

pub(super) fn next_combo_step_ready_at(
    sent_completed_at: Instant,
    step: &RuntimeComboStep,
) -> Instant {
    sent_completed_at + configured_interval_duration(step.interval_ms)
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

pub(super) fn build_repeat_bindings(runtime: &RuntimeProfile) -> Vec<RuntimeRepeatBinding> {
    let combo_trigger_vks = runtime
        .combos
        .iter()
        .map(|combo| combo.trigger.vk)
        .collect::<HashSet<_>>();
    let custom_autofire_vks = runtime
        .custom_autofires
        .iter()
        .map(|entry| entry.key.vk)
        .collect::<HashSet<_>>();

    let mut bindings = Vec::new();
    bindings.extend(
        runtime
            .keys
            .iter()
            .filter(|key| {
                !combo_trigger_vks.contains(&key.vk) && !custom_autofire_vks.contains(&key.vk)
            })
            .map(|key| RuntimeRepeatBinding {
                key: *key,
                trigger: RepeatTrigger::HoldKey(key.vk),
                repeat_interval: configured_interval_duration(runtime.repeat_interval_ms),
                press_duration: configured_press_duration(runtime.press_duration_ms),
            }),
    );
    bindings.extend(
        runtime
            .custom_autofires
            .iter()
            .map(|entry| RuntimeRepeatBinding {
                key: entry.key,
                trigger: RepeatTrigger::HoldKey(entry.key.vk),
                repeat_interval: configured_interval_duration(entry.repeat_interval_ms),
                press_duration: configured_press_duration(entry.press_duration_ms),
            }),
    );
    bindings.extend(
        runtime
            .auto_triggers
            .iter()
            .map(|entry| RuntimeRepeatBinding {
                key: entry.key,
                trigger: RepeatTrigger::ToggleHotkey(entry.trigger_hotkey.clone()),
                repeat_interval: configured_interval_duration(entry.repeat_interval_ms),
                press_duration: configured_press_duration(entry.press_duration_ms),
            }),
    );

    bindings
}

pub(super) fn drive_repeat_state_machines(
    repeat_bindings: &[RuntimeRepeatBinding],
    input_snapshot: &InputSnapshot,
    repeat_states: &mut [RepeatBindingState],
    command_queue: &mut CommandQueue,
    now: Instant,
) {
    for (index, binding) in repeat_bindings.iter().enumerate() {
        let state = &mut repeat_states[index];
        let source = CommandSource::Repeat(index);

        match &binding.trigger {
            RepeatTrigger::HoldKey(vk) => {
                let is_down = input_snapshot.is_down(*vk);
                state.trigger_down = is_down;
                state.enabled = is_down;
                if !state.enabled {
                    command_queue.cancel_source(source);
                    state.phase = RepeatPhase::Idle;
                    continue;
                }
            }
            RepeatTrigger::ToggleHotkey(hotkey) => {
                let is_down = hotkey_is_down(hotkey, input_snapshot);
                if is_down && !state.trigger_down {
                    state.enabled = !state.enabled;
                    if !state.enabled {
                        command_queue.cancel_source(source);
                        state.phase = RepeatPhase::Idle;
                    }
                }
                state.trigger_down = is_down;
                if !state.enabled {
                    continue;
                }
            }
        }

        match state.phase {
            RepeatPhase::Idle => queue_repeat_command(binding, state, command_queue, source, now),
            RepeatPhase::Recovering { until } if now >= until => {
                queue_repeat_command(binding, state, command_queue, source, now)
            }
            RepeatPhase::Queued | RepeatPhase::Recovering { .. } => {}
        }
    }
}

fn queue_repeat_command(
    binding: &RuntimeRepeatBinding,
    state: &mut RepeatBindingState,
    command_queue: &mut CommandQueue,
    source: CommandSource,
    ready_at: Instant,
) {
    command_queue.cancel_source(source);
    command_queue.enqueue(QueuedCommand {
        source,
        key: binding.key,
        ready_at,
        press_duration: binding.press_duration,
    });
    state.phase = RepeatPhase::Queued;
}

pub(super) fn sync_repeat_binding_inputs(
    repeat_bindings: &[RuntimeRepeatBinding],
    input_snapshot: &InputSnapshot,
    repeat_states: &mut [RepeatBindingState],
) {
    for (binding, state) in repeat_bindings.iter().zip(repeat_states.iter_mut()) {
        match &binding.trigger {
            RepeatTrigger::HoldKey(vk) => {
                state.trigger_down = input_snapshot.is_down(*vk);
                state.enabled = false;
                state.phase = RepeatPhase::Idle;
            }
            RepeatTrigger::ToggleHotkey(hotkey) => {
                state.trigger_down = hotkey_is_down(hotkey, input_snapshot);
                state.phase = RepeatPhase::Idle;
            }
        }
    }
}

pub(super) fn drive_combo_state_machines(
    combos: &[RuntimeCombo],
    input_snapshot: &InputSnapshot,
    combo_states: &mut [ComboState],
    command_queue: &mut CommandQueue,
    now: Instant,
) {
    for (index, combo) in combos.iter().enumerate() {
        let state = &mut combo_states[index];
        let is_down = input_snapshot.is_down(combo.trigger.vk);
        if is_down && !state.trigger_down {
            command_queue.cancel_source(CommandSource::Combo(index));
            state.phase = ComboPhase::Ready {
                step_index: 0,
                ready_at: now,
            };
        }
        state.trigger_down = is_down;
        advance_combo_state(
            combo,
            state,
            command_queue,
            CommandSource::Combo(index),
            now,
        );
    }
}

fn advance_combo_state(
    combo: &RuntimeCombo,
    state: &mut ComboState,
    command_queue: &mut CommandQueue,
    source: CommandSource,
    now: Instant,
) {
    loop {
        match state.phase {
            ComboPhase::Idle | ComboPhase::Queued { .. } => break,
            ComboPhase::Recovering { next_index, until } => {
                if now < until {
                    break;
                }
                if next_index >= combo.steps.len() {
                    state.phase = ComboPhase::Idle;
                    break;
                }
                state.phase = ComboPhase::Ready {
                    step_index: next_index,
                    ready_at: now,
                };
            }
            ComboPhase::Ready {
                step_index,
                ready_at,
            } => {
                if now < ready_at || step_index >= combo.steps.len() {
                    break;
                }
                let step = &combo.steps[step_index];
                command_queue.cancel_source(source);
                command_queue.enqueue(QueuedCommand {
                    source,
                    key: step.key,
                    ready_at,
                    press_duration: configured_press_duration(step.press_duration_ms),
                });
                state.phase = ComboPhase::Queued { step_index };
                break;
            }
        }
    }
}

pub(super) fn sync_combo_inputs(
    combos: &[RuntimeCombo],
    input_snapshot: &InputSnapshot,
    combo_states: &mut [ComboState],
) {
    for (combo, state) in combos.iter().zip(combo_states.iter_mut()) {
        state.trigger_down = input_snapshot.is_down(combo.trigger.vk);
        state.phase = ComboPhase::Idle;
    }
}

pub(super) fn drive_linked_bindings(
    linked_keys: &[RuntimeLinkedKey],
    input_snapshot: &InputSnapshot,
    linked_states: &mut [LinkedBindingState],
    command_queue: &mut CommandQueue,
    now: Instant,
) {
    for (index, linked) in linked_keys.iter().enumerate() {
        let state = &mut linked_states[index];
        let is_down = input_snapshot.is_down(linked.trigger_key.vk);
        let should_trigger = match linked.trigger_mode {
            LinkedTriggerMode::Press => is_down && !state.trigger_down,
            LinkedTriggerMode::Release => !is_down && state.trigger_down,
        };
        if should_trigger {
            command_queue.enqueue(QueuedCommand {
                source: CommandSource::Linked(index),
                key: linked.linked_key,
                ready_at: now + configured_interval_duration(linked.interval_ms),
                press_duration: configured_press_duration(linked.press_duration_ms),
            });
        }
        state.trigger_down = is_down;
    }
}

pub(super) fn sync_linked_inputs(
    linked_keys: &[RuntimeLinkedKey],
    input_snapshot: &InputSnapshot,
    linked_states: &mut [LinkedBindingState],
) {
    for (linked, state) in linked_keys.iter().zip(linked_states.iter_mut()) {
        state.trigger_down = input_snapshot.is_down(linked.trigger_key.vk);
    }
}

pub(super) fn on_command_executed(
    command: QueuedCommand,
    completed_at: Instant,
    repeat_bindings: &[RuntimeRepeatBinding],
    repeat_states: &mut [RepeatBindingState],
    combos: &[RuntimeCombo],
    combo_states: &mut [ComboState],
) {
    match command.source {
        CommandSource::Repeat(index) => {
            let Some(state) = repeat_states.get_mut(index) else {
                return;
            };
            let Some(binding) = repeat_bindings.get(index) else {
                return;
            };
            state.phase = RepeatPhase::Recovering {
                until: completed_at + binding.repeat_interval,
            };
        }
        CommandSource::Combo(index) => {
            let Some(state) = combo_states.get_mut(index) else {
                return;
            };
            let Some(combo) = combos.get(index) else {
                return;
            };
            let ComboPhase::Queued { step_index } = state.phase else {
                return;
            };
            if step_index + 1 >= combo.steps.len() {
                state.phase = ComboPhase::Idle;
            } else {
                let step = &combo.steps[step_index];
                state.phase = ComboPhase::Recovering {
                    next_index: step_index + 1,
                    until: next_combo_step_ready_at(completed_at, step),
                };
            }
        }
        CommandSource::Linked(_) => {}
    }
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
