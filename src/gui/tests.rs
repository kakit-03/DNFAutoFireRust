// Covers GUI state persistence and dialog behavior.

use super::{
    AppState, ComboCaptureTarget, ComboDialogState, ComboStepDraft, SpecialKeyDialogState,
    SpecialKeyType,
};
use crate::config::{ConfigStore, LinkedTriggerMode, SpecialKeyConfig};
use crate::input::backend::InputBackendKind;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn save_combo_dialog_persists_to_config_file() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");

    let dialog = ComboDialogState {
        edit_index: None,
        name: "burst".to_string(),
        trigger_key: "U".to_string(),
        steps: vec![
            ComboStepDraft {
                key: "A".to_string(),
                interval_ms: "80".to_string(),
                press_duration_ms: "2".to_string(),
            },
            ComboStepDraft {
                key: "S".to_string(),
                interval_ms: "90".to_string(),
                press_duration_ms: "3".to_string(),
            },
        ],
        selected_step: None,
        new_step_interval_ms: "80".to_string(),
        new_step_press_duration_ms: "1".to_string(),
        capture_target: Some(ComboCaptureTarget::ContinuousSteps),
        capture_down_keys: HashSet::new(),
    };

    state
        .save_combo_dialog(&dialog)
        .expect("save combo dialog should persist");

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    let combo = &saved.profiles["default"].combos[0];
    assert_eq!(combo.name, "burst");
    assert_eq!(combo.trigger_key, "U");
    assert_eq!(combo.steps.len(), 2);
    assert_eq!(combo.steps[0].key, "A");
    assert_eq!(combo.steps[0].interval_ms, 80);
    assert_eq!(combo.steps[0].press_duration_ms, 2);
    assert_eq!(combo.steps[1].key, "S");
    assert_eq!(combo.steps[1].interval_ms, 90);
    assert_eq!(combo.steps[1].press_duration_ms, 3);

    let _ = fs::remove_file(path);
}

#[test]
fn save_special_key_dialog_persists_to_config_file() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");

    let dialog = SpecialKeyDialogState {
        edit_index: None,
        config_type: SpecialKeyType::AutoTrigger,
        name: "auto-j".to_string(),
        custom_key: "未录入".to_string(),
        auto_trigger_key: "J".to_string(),
        auto_trigger_hotkey: "LAlt+Q".to_string(),
        linked_trigger_key: "未录入".to_string(),
        linked_target_key: "未录入".to_string(),
        linked_trigger_mode: LinkedTriggerMode::Press,
        repeat_interval_ms: "30".to_string(),
        press_duration_ms: "2".to_string(),
        linked_interval_ms: "80".to_string(),
        linked_press_duration_ms: "1".to_string(),
        capture_target: None,
        capture_down_keys: HashSet::new(),
    };

    state
        .save_special_key_dialog(&dialog)
        .expect("save special key dialog should persist");

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    let special = &saved.profiles["default"].special_keys[0];
    match special {
        SpecialKeyConfig::AutoTrigger {
            name,
            key,
            trigger_hotkey,
            repeat_interval_ms,
            press_duration_ms,
        } => {
            assert_eq!(name, "auto-j");
            assert_eq!(key, "J");
            assert_eq!(trigger_hotkey, "LALT+Q");
            assert_eq!(*repeat_interval_ms, 30);
            assert_eq!(*press_duration_ms, 2);
        }
        _ => panic!("expected auto trigger special key"),
    }

    let _ = fs::remove_file(path);
}

#[test]
fn clone_profile_uses_incrementing_cloned_suffix() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");

    state.clone_profile().expect("first clone should save");
    assert_eq!(
        state.current_profile_key.as_deref(),
        Some("default-cloned-1")
    );

    state
        .load_profile_from_store("default")
        .expect("reload original profile");
    state.clone_profile().expect("second clone should save");
    assert_eq!(
        state.current_profile_key.as_deref(),
        Some("default-cloned-2")
    );

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    assert!(saved.profiles.contains_key("default-cloned-1"));
    assert!(saved.profiles.contains_key("default-cloned-2"));

    let _ = fs::remove_file(path);
}

#[test]
fn remember_last_started_profile_updates_default_for_saved_selection() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");
    state
        .save_profile()
        .expect("initial profile should save cleanly");

    state.new_profile().expect("create new profile");
    state.draft.name = "raid".to_string();
    state
        .save_profile()
        .expect("second profile should save cleanly");
    state
        .load_profile_from_store("raid")
        .expect("load saved profile");

    state
        .remember_last_started_profile()
        .expect("remember last started profile");

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    assert_eq!(saved.default_profile, "raid");

    let _ = fs::remove_file(path);
}

#[test]
fn remember_last_started_profile_ignores_unsaved_rename() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");
    state
        .save_profile()
        .expect("initial profile should save cleanly");

    state.draft.name = "draft-renamed".to_string();
    state
        .remember_last_started_profile()
        .expect("unsaved rename should be ignored");

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    assert_eq!(saved.default_profile, "default");

    let _ = fs::remove_file(path);
}

#[test]
fn quick_switch_watch_config_uses_global_hotkey_and_current_target_windows() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");

    state.global_quick_switch_hotkey = "LAlt+Q".to_string();
    state.global_target_windows_text = "DNF\r\n地下城与勇士".to_string();

    let watch = state.quick_switch_watch_config();
    let hotkey = watch.hotkey.expect("hotkey should be registerable");

    assert_eq!(hotkey.modifiers, 0x0001);
    assert_eq!(hotkey.vk, 0x51);
    assert_eq!(watch.target_windows, vec!["DNF", "地下城与勇士"]);

    let _ = fs::remove_file(path);
}

#[test]
fn persist_global_quick_switch_hotkey_saves_immediately() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");

    state.global_quick_switch_hotkey = "LAlt+;".to_string();
    state
        .persist_global_quick_switch_hotkey()
        .expect("global quick switch hotkey should persist");

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    assert_eq!(saved.quick_switch_hotkey, "LALT+SEMICOLON");
    assert_eq!(state.global_quick_switch_hotkey, "LAlt+;");

    let _ = fs::remove_file(path);
}

#[test]
fn persist_global_target_windows_saves_immediately() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");

    state.global_target_windows_text = "地下城与勇士\r\nDNF".to_string();
    state
        .persist_global_target_windows()
        .expect("global target windows should persist");

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    assert_eq!(saved.target_windows, vec!["地下城与勇士", "DNF"]);

    let _ = fs::remove_file(path);
}

#[test]
fn input_backend_selection_persists_available_backends() {
    let path = unique_test_config_path();
    let mut state = AppState::new(path.clone(), ConfigStore::default());
    state.setup_initial_state().expect("setup initial state");

    state
        .set_input_backend(InputBackendKind::SendInputPolling)
        .expect("default backend should be selectable");
    state
        .set_input_backend(InputBackendKind::MessageBackend)
        .expect("message backend should be selectable");
    assert_eq!(state.store.input_backend, InputBackendKind::MessageBackend);

    let saved = ConfigStore::load_or_create(&path).expect("load saved config");
    assert_eq!(saved.input_backend, InputBackendKind::MessageBackend);

    let _ = fs::remove_file(path);
}

fn unique_test_config_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!("dnf-gui-test-{nanos}.json"))
}
