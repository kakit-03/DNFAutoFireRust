// Covers command queue ordering and command execution side effects.

use super::*;

#[test]
fn command_queue_pops_earliest_ready_command_first() {
    let key_a = parse_single_key("A").expect("A");
    let key_b = parse_single_key("B").expect("B");
    let now = Instant::now();
    let mut queue = CommandQueue::default();

    queue.enqueue(QueuedCommand {
        source: CommandSource::Linked(0),
        key: key_b,
        ready_at: now + Duration::from_millis(10),
        press_duration: Duration::from_millis(20),
    });
    queue.enqueue(QueuedCommand {
        source: CommandSource::Repeat(0),
        key: key_a,
        ready_at: now,
        press_duration: Duration::from_millis(15),
    });

    let first = queue.pop_next_ready(now + Duration::from_millis(1));
    assert!(matches!(
        first.map(|command| command.source),
        Some(CommandSource::Repeat(0))
    ));
}

#[test]
fn command_queue_cancel_source_only_removes_matching_commands() {
    let key_a = parse_single_key("A").expect("A");
    let key_b = parse_single_key("B").expect("B");
    let now = Instant::now();
    let mut queue = CommandQueue::default();

    queue.enqueue(QueuedCommand {
        source: CommandSource::Repeat(0),
        key: key_a,
        ready_at: now,
        press_duration: Duration::from_millis(1),
    });
    queue.enqueue(QueuedCommand {
        source: CommandSource::Combo(0),
        key: key_b,
        ready_at: now,
        press_duration: Duration::from_millis(1),
    });
    queue.enqueue(QueuedCommand {
        source: CommandSource::Repeat(0),
        key: key_a,
        ready_at: now + Duration::from_millis(1),
        press_duration: Duration::from_millis(1),
    });

    queue.cancel_source(CommandSource::Repeat(0));

    let remaining = queue
        .pop_next_ready(now + Duration::from_millis(2))
        .expect("combo command remains");
    assert!(matches!(remaining.source, CommandSource::Combo(0)));
    assert!(
        queue
            .pop_next_ready(now + Duration::from_millis(2))
            .is_none()
    );
}

#[test]
fn command_execution_advances_repeat_and_combo_states() {
    let key_a = parse_single_key("A").expect("A");
    let completed_at = Instant::now();
    let combo = super::RuntimeCombo {
        name: "combo".to_string(),
        trigger: key_a,
        steps: vec![
            RuntimeComboStep {
                key: key_a,
                interval_ms: 8,
                press_duration_ms: 20,
            },
            RuntimeComboStep {
                key: parse_single_key("B").expect("B"),
                interval_ms: 12,
                press_duration_ms: 20,
            },
        ],
    };
    let repeat_binding = RuntimeRepeatBinding {
        key: key_a,
        trigger: super::RepeatTrigger::HoldKey(key_a.vk),
        repeat_interval: Duration::from_millis(10),
        press_duration: Duration::from_millis(15),
    };
    let mut repeat_states = vec![RepeatBindingState {
        trigger_down: true,
        enabled: true,
        phase: RepeatPhase::Queued,
    }];
    let mut combo_states = vec![ComboState {
        trigger_down: false,
        phase: ComboPhase::Queued { step_index: 0 },
    }];

    on_command_executed(
        QueuedCommand {
            source: CommandSource::Repeat(0),
            key: key_a,
            ready_at: completed_at,
            press_duration: Duration::from_millis(15),
        },
        completed_at,
        &[repeat_binding.clone()],
        &mut repeat_states,
        &[combo.clone()],
        &mut combo_states,
    );
    assert!(matches!(
        repeat_states[0].phase,
        RepeatPhase::Recovering { until } if until == completed_at + Duration::from_millis(10)
    ));

    on_command_executed(
        QueuedCommand {
            source: CommandSource::Combo(0),
            key: key_a,
            ready_at: completed_at,
            press_duration: Duration::from_millis(20),
        },
        completed_at,
        &[repeat_binding],
        &mut repeat_states,
        &[combo],
        &mut combo_states,
    );
    assert!(matches!(
        combo_states[0].phase,
        ComboPhase::Recovering {
            next_index: 1,
            until
        } if until == completed_at + Duration::from_millis(8)
    ));
}

#[test]
fn ready_commands_are_sent_through_input_backend() {
    let key_a = parse_single_key("A").expect("A");
    let key_b = parse_single_key("B").expect("B");
    let sleeper = HighPrecisionSleeper::new();
    let mut backend = FakeInputBackend::default();
    let repeat_binding = RuntimeRepeatBinding {
        key: key_a,
        trigger: super::RepeatTrigger::HoldKey(key_a.vk),
        repeat_interval: Duration::from_millis(10),
        press_duration: Duration::from_millis(1),
    };
    let combo = super::RuntimeCombo {
        name: "combo".to_string(),
        trigger: key_a,
        steps: vec![
            RuntimeComboStep {
                key: key_a,
                interval_ms: 8,
                press_duration_ms: 1,
            },
            RuntimeComboStep {
                key: key_b,
                interval_ms: 8,
                press_duration_ms: 1,
            },
        ],
    };
    let mut repeat_states = vec![RepeatBindingState {
        trigger_down: true,
        enabled: true,
        phase: RepeatPhase::Queued,
    }];
    let mut combo_states = vec![ComboState {
        trigger_down: true,
        phase: ComboPhase::Queued { step_index: 0 },
    }];
    let now = Instant::now();

    for command in [
        QueuedCommand {
            source: CommandSource::Repeat(0),
            key: key_a,
            ready_at: now,
            press_duration: Duration::from_millis(1),
        },
        QueuedCommand {
            source: CommandSource::Combo(0),
            key: key_b,
            ready_at: now,
            press_duration: Duration::from_millis(2),
        },
        QueuedCommand {
            source: CommandSource::Linked(0),
            key: key_a,
            ready_at: now,
            press_duration: Duration::from_millis(3),
        },
    ] {
        execute_ready_command(
            command,
            &mut backend,
            &sleeper,
            &[repeat_binding.clone()],
            &mut repeat_states,
            &[combo.clone()],
            &mut combo_states,
        );
    }

    assert_eq!(backend.sent.len(), 3);
    assert_eq!(backend.sent[0], (key_a, Duration::from_millis(1)));
    assert_eq!(backend.sent[1], (key_b, Duration::from_millis(2)));
    assert_eq!(backend.sent[2], (key_a, Duration::from_millis(3)));
}
