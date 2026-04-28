// Builds and advances repeat-key state machines.

use super::*;

pub(in crate::autofire) fn build_repeat_bindings(
    runtime: &RuntimeProfile,
) -> Vec<RuntimeRepeatBinding> {
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

pub(in crate::autofire) fn drive_repeat_state_machines(
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

pub(in crate::autofire) fn sync_repeat_binding_inputs(
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
