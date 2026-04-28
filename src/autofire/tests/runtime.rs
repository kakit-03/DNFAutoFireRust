// Covers runtime profile construction and worker lifecycle behavior.

use super::*;

#[test]
fn worker_can_start_and_stop_without_leaking_thread() {
    let profile = Profile::default();
    let (tx, rx) = channel();
    let mut handle =
        AutoFireService::start_with_events(profile, vec!["DNF".to_string()], move |event| {
            let _ = tx.send(event);
        })
        .expect("worker should start");

    std::thread::sleep(Duration::from_millis(10));
    handle.stop();
    handle.wait().expect("worker should stop cleanly");

    let events: Vec<_> = rx.try_iter().collect();
    assert!(events.iter().any(|event| *event == RunnerEvent::Started));
    assert!(
        events
            .iter()
            .any(|event| *event == RunnerEvent::Stopped(StopReason::StopRequested))
    );
    assert!(!handle.is_running());
}

#[test]
fn runtime_profile_accepts_combo_only_profile() {
    let profile = Profile {
        enabled_keys: Vec::new(),
        repeat_interval_ms: 1,
        press_duration_ms: 1,
        poll_interval_ms: 1,
        combos: vec![ComboConfig {
            name: "combo".to_string(),
            trigger_key: "A".to_string(),
            steps: vec![
                ComboStepConfig {
                    key: "A".to_string(),
                    interval_ms: 80,
                    press_duration_ms: 1,
                },
                ComboStepConfig {
                    key: "A".to_string(),
                    interval_ms: 90,
                    press_duration_ms: 2,
                },
                ComboStepConfig {
                    key: "D".to_string(),
                    interval_ms: 100,
                    press_duration_ms: 3,
                },
            ],
            sequence_keys: Vec::new(),
            step_interval_ms: 0,
            press_duration_ms: 1,
        }],
        special_keys: vec![
            SpecialKeyConfig::CustomAutofire {
                name: "custom".to_string(),
                key: "J".to_string(),
                repeat_interval_ms: 2,
                press_duration_ms: 3,
            },
            SpecialKeyConfig::AutoTrigger {
                name: "auto".to_string(),
                key: "K".to_string(),
                trigger_hotkey: "LALT+Q".to_string(),
                repeat_interval_ms: 4,
                press_duration_ms: 5,
            },
            SpecialKeyConfig::LinkedKey {
                name: "link".to_string(),
                trigger_key: "A".to_string(),
                linked_key: "B".to_string(),
                trigger_mode: LinkedTriggerMode::Release,
                interval_ms: 6,
                press_duration_ms: 7,
            },
        ],
    };

    let runtime = super::RuntimeProfile::from_profile(&profile, &["DNF".to_string()])
        .expect("runtime profile");
    assert!(runtime.keys.is_empty());
    assert_eq!(runtime.combos.len(), 1);
    assert_eq!(runtime.combos[0].steps.len(), 3);
    assert_eq!(runtime.custom_autofires.len(), 1);
    assert_eq!(runtime.auto_triggers.len(), 1);
    assert_eq!(runtime.linked_keys.len(), 1);
    assert_eq!(
        runtime.linked_keys[0].trigger_mode,
        LinkedTriggerMode::Release
    );
}

#[test]
fn runtime_profile_retains_selected_input_backend() {
    let profile = Profile::default();
    let runtime = super::RuntimeProfile::from_profile_with_backend(
        &profile,
        &["DNF".to_string()],
        InputBackendKind::MessageBackend,
    )
    .expect("runtime profile");

    assert_eq!(runtime.input_backend, InputBackendKind::MessageBackend);
}

#[test]
fn synthetic_hold_keeps_previous_physical_state() {
    assert!(resolve_effective_key_down(true, false, true));
    assert!(!resolve_effective_key_down(false, true, true));
}

#[test]
fn combo_next_step_waits_from_actual_send_completion() {
    let finished_at = Instant::now();
    let step = RuntimeComboStep {
        key: parse_single_key("A").expect("key"),
        interval_ms: 7,
        press_duration_ms: 20,
    };

    let next_at = next_combo_step_ready_at(finished_at, &step);
    assert_eq!(
        next_at.duration_since(finished_at),
        Duration::from_millis(7)
    );
}

#[test]
fn monitored_keys_include_all_trigger_sources_without_duplicates() {
    let key_a = parse_single_key("A").expect("A");
    let key_b = parse_single_key("B").expect("B");
    let key_c = parse_single_key("C").expect("C");
    let key_d = parse_single_key("D").expect("D");

    let monitored = collect_monitored_vks(
        &[key_a],
        &[super::RuntimeCombo {
            name: "combo".to_string(),
            trigger: key_b,
            steps: Vec::new(),
        }],
        &[super::RuntimeCustomAutofire {
            name: "custom".to_string(),
            key: key_c,
            repeat_interval_ms: 1,
            press_duration_ms: 1,
        }],
        &[super::RuntimeAutoTrigger {
            name: "auto".to_string(),
            key: key_d,
            trigger_hotkey: super::RuntimeHotkey {
                text: "LCTRL+A".to_string(),
                specs: vec![parse_single_key("LCTRL").expect("ctrl"), key_a],
            },
            repeat_interval_ms: 1,
            press_duration_ms: 1,
        }],
        &[super::RuntimeLinkedKey {
            name: "linked".to_string(),
            trigger_key: key_b,
            linked_key: key_d,
            trigger_mode: LinkedTriggerMode::Press,
            interval_ms: 1,
            press_duration_ms: 1,
        }],
    );

    assert_eq!(monitored.len(), 4);
    assert!(monitored.contains(&key_a.vk));
    assert!(monitored.contains(&key_b.vk));
    assert!(monitored.contains(&key_c.vk));
    assert!(monitored.contains(&parse_single_key("LCTRL").expect("ctrl").vk));
}
