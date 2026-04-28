// Advances combo sequence state machines and completion transitions.

use super::*;

pub(in crate::autofire) fn next_combo_step_ready_at(
    sent_completed_at: Instant,
    step: &RuntimeComboStep,
) -> Instant {
    sent_completed_at + configured_interval_duration(step.interval_ms)
}

pub(in crate::autofire) fn drive_combo_state_machines(
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

pub(in crate::autofire) fn sync_combo_inputs(
    combos: &[RuntimeCombo],
    input_snapshot: &InputSnapshot,
    combo_states: &mut [ComboState],
) {
    for (combo, state) in combos.iter().zip(combo_states.iter_mut()) {
        state.trigger_down = input_snapshot.is_down(combo.trigger.vk);
        state.phase = ComboPhase::Idle;
    }
}

pub(in crate::autofire) fn on_command_executed(
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
