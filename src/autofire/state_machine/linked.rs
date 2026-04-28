// Advances linked-key trigger state machines.

use super::*;

pub(in crate::autofire) fn drive_linked_bindings(
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

pub(in crate::autofire) fn sync_linked_inputs(
    linked_keys: &[RuntimeLinkedKey],
    input_snapshot: &InputSnapshot,
    linked_states: &mut [LinkedBindingState],
) {
    for (linked, state) in linked_keys.iter().zip(linked_states.iter_mut()) {
        state.trigger_down = input_snapshot.is_down(linked.trigger_key.vk);
    }
}
