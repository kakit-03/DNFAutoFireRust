// Covers configuration normalization, migration, and validation behavior.

use super::{
    ComboConfig, ComboStepConfig, ConfigStore, DEFAULT_COMBO_STEP_INTERVAL_MS,
    DEFAULT_COMBO_STEP_PRESS_DURATION_MS, DEFAULT_POLL_INTERVAL_MS, DEFAULT_PRESS_DURATION_MS,
    DEFAULT_REPEAT_INTERVAL_MS, LinkedTriggerMode, Profile, SpecialKeyConfig,
};
use crate::input::backend::InputBackendKind;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn profile_normalize_adds_defaults_for_empty_fields() {
    let profile = Profile {
        enabled_keys: Vec::new(),
        repeat_interval_ms: 0,
        press_duration_ms: 0,
        poll_interval_ms: 0,
        combos: Vec::new(),
        special_keys: Vec::new(),
    }
    .normalized();

    assert!(!profile.enabled_keys.is_empty());
    assert_eq!(profile.repeat_interval_ms, DEFAULT_REPEAT_INTERVAL_MS);
    assert_eq!(profile.press_duration_ms, DEFAULT_PRESS_DURATION_MS);
    assert_eq!(profile.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);
}

#[test]
fn combo_normalize_upcases_keys_and_fixes_intervals() {
    let combo = ComboConfig {
        name: " test ".to_string(),
        trigger_key: "j".to_string(),
        steps: vec![
            ComboStepConfig {
                key: "a".to_string(),
                interval_ms: 0,
                press_duration_ms: 0,
            },
            ComboStepConfig {
                key: "a".to_string(),
                interval_ms: 5,
                press_duration_ms: 2,
            },
            ComboStepConfig {
                key: "b".to_string(),
                interval_ms: 9,
                press_duration_ms: 3,
            },
        ],
        sequence_keys: Vec::new(),
        step_interval_ms: 0,
        press_duration_ms: 0,
    }
    .normalized();

    assert_eq!(combo.name, "test");
    assert_eq!(combo.trigger_key, "J");
    assert_eq!(combo.steps.len(), 3);
    assert_eq!(combo.steps[0].key, "A");
    assert_eq!(combo.steps[0].interval_ms, DEFAULT_COMBO_STEP_INTERVAL_MS);
    assert_eq!(
        combo.steps[0].press_duration_ms,
        DEFAULT_COMBO_STEP_PRESS_DURATION_MS
    );
    assert_eq!(combo.steps[1].key, "A");
    assert_eq!(combo.steps[2].key, "B");
    assert_eq!(combo.steps[2].press_duration_ms, 3);
    assert_eq!(combo.press_duration_ms, 0);
}

#[test]
fn combo_normalize_migrates_legacy_sequence_keys() {
    let combo = ComboConfig {
        name: "legacy".to_string(),
        trigger_key: "u".to_string(),
        steps: Vec::new(),
        sequence_keys: vec!["a".to_string(), "s".to_string(), "d".to_string()],
        step_interval_ms: 80,
        press_duration_ms: 1,
    }
    .normalized();

    assert_eq!(combo.steps.len(), 3);
    assert_eq!(combo.steps[0].key, "A");
    assert_eq!(combo.steps[1].key, "S");
    assert_eq!(combo.steps[2].key, "D");
    assert_eq!(combo.steps[0].interval_ms, 80);
    assert_eq!(combo.steps[0].press_duration_ms, 1);
    assert!(combo.sequence_keys.is_empty());
    assert_eq!(combo.step_interval_ms, 0);
    assert_eq!(combo.press_duration_ms, 0);
}

#[test]
fn delete_last_profile_is_rejected() {
    let mut store = ConfigStore::default();
    let err = store
        .delete_profile("default")
        .expect_err("should reject deleting last profile");
    assert!(err.to_string().contains("last profile"));
}

#[test]
fn remember_started_profile_updates_default_profile() {
    let mut store = ConfigStore::default();
    store.upsert_profile(
        "raid".to_string(),
        Profile {
            enabled_keys: vec!["J".to_string()],
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            combos: Vec::new(),
            special_keys: Vec::new(),
        },
    );

    store
        .remember_started_profile("raid")
        .expect("remember started profile");

    assert_eq!(store.default_profile, "raid");
}

#[test]
fn duplicate_combo_names_are_rejected() {
    let profile = Profile {
        enabled_keys: vec!["J".to_string()],
        repeat_interval_ms: 1,
        press_duration_ms: 1,
        poll_interval_ms: 1,
        combos: vec![
            ComboConfig {
                name: "combo".to_string(),
                trigger_key: "A".to_string(),
                steps: vec![ComboStepConfig {
                    key: "B".to_string(),
                    interval_ms: 1,
                    press_duration_ms: 1,
                }],
                sequence_keys: Vec::new(),
                step_interval_ms: 0,
                press_duration_ms: 1,
            },
            ComboConfig {
                name: "COMBO".to_string(),
                trigger_key: "C".to_string(),
                steps: vec![ComboStepConfig {
                    key: "D".to_string(),
                    interval_ms: 1,
                    press_duration_ms: 1,
                }],
                sequence_keys: Vec::new(),
                step_interval_ms: 0,
                press_duration_ms: 1,
            },
        ],
        special_keys: Vec::new(),
    };

    assert!(profile.validate().is_err());
}

#[test]
fn store_normalized_migrates_legacy_combo_json() {
    let raw = r#"{
          "default_profile": "default",
          "quick_switch_hotkey": "LALT+Q",
          "target_windows": ["DNF"],
          "profiles": {
            "default": {
              "enabled_keys": ["J"],
              "repeat_interval_ms": 1,
              "press_duration_ms": 1,
              "poll_interval_ms": 1,
              "combos": [
                {
                  "name": "legacy",
                  "trigger_key": "u",
                  "sequence_keys": ["a", "s", "d"],
                  "step_interval_ms": 80,
                  "press_duration_ms": 1
                }
              ],
              "special_keys": []
            }
          }
        }"#;

    let store: ConfigStore = serde_json::from_str(raw).expect("legacy config json");
    let normalized = store.normalized();
    let combo = &normalized.profiles["default"].combos[0];

    assert_eq!(combo.trigger_key, "U");
    assert_eq!(combo.steps.len(), 3);
    assert_eq!(combo.steps[0].key, "A");
    assert_eq!(combo.steps[1].key, "S");
    assert_eq!(combo.steps[2].key, "D");
    assert_eq!(combo.steps[0].interval_ms, 80);
    assert_eq!(combo.steps[0].press_duration_ms, 1);
    assert!(combo.sequence_keys.is_empty());
    assert_eq!(combo.step_interval_ms, 0);
    assert_eq!(normalized.quick_switch_hotkey, "LALT+Q");
    assert_eq!(normalized.target_windows, vec!["DNF"]);
    assert_eq!(normalized.input_backend, InputBackendKind::SendInputPolling);
    assert!(normalized.backend_settings.is_empty());
}

#[test]
fn store_serializes_input_backend_selection() {
    let raw = r#"{
          "default_profile": "default",
          "quick_switch_hotkey": "LALT+Q",
          "target_windows": ["DNF"],
          "hide_gui_on_startup": false,
          "input_backend": "message_backend",
          "backend_settings": {
            "message_backend": {
              "mode": "compat"
            }
          },
          "profiles": {
            "default": {
              "enabled_keys": ["J"],
              "repeat_interval_ms": 1,
              "press_duration_ms": 1,
              "poll_interval_ms": 1,
              "combos": [],
              "special_keys": []
            }
          }
        }"#;

    let store: ConfigStore = serde_json::from_str(raw).expect("config json");

    assert_eq!(store.input_backend, InputBackendKind::MessageBackend);
    assert!(store.backend_settings.contains_key("message_backend"));

    let saved = serde_json::to_string(&store).expect("serialize config");
    assert!(saved.contains(r#""input_backend":"message_backend""#));
}

#[test]
fn special_key_configs_are_normalized_and_validated() {
    let profile = Profile {
        enabled_keys: Vec::new(),
        repeat_interval_ms: 1,
        press_duration_ms: 1,
        poll_interval_ms: 1,
        combos: Vec::new(),
        special_keys: vec![
            SpecialKeyConfig::CustomAutofire {
                name: "  custom ".to_string(),
                key: "j".to_string(),
                repeat_interval_ms: 0,
                press_duration_ms: 0,
            },
            SpecialKeyConfig::AutoTrigger {
                name: "auto".to_string(),
                key: "k".to_string(),
                trigger_hotkey: "lalt+~".to_string(),
                repeat_interval_ms: 0,
                press_duration_ms: 0,
            },
            SpecialKeyConfig::LinkedKey {
                name: "link".to_string(),
                trigger_key: "a".to_string(),
                linked_key: "b".to_string(),
                trigger_mode: LinkedTriggerMode::Release,
                interval_ms: 0,
                press_duration_ms: 0,
            },
        ],
    }
    .normalized();

    assert_eq!(profile.special_keys.len(), 3);
    match &profile.special_keys[0] {
        SpecialKeyConfig::CustomAutofire {
            name,
            key,
            repeat_interval_ms,
            press_duration_ms,
        } => {
            assert_eq!(name, "custom");
            assert_eq!(key, "J");
            assert_eq!(*repeat_interval_ms, DEFAULT_REPEAT_INTERVAL_MS);
            assert_eq!(*press_duration_ms, DEFAULT_PRESS_DURATION_MS);
        }
        _ => panic!("expected custom autofire"),
    }
    match &profile.special_keys[1] {
        SpecialKeyConfig::AutoTrigger { trigger_hotkey, .. } => {
            assert_eq!(trigger_hotkey, "LALT+BACKQUOTE");
        }
        _ => panic!("expected auto trigger"),
    }
    match &profile.special_keys[2] {
        SpecialKeyConfig::LinkedKey {
            trigger_mode,
            interval_ms,
            press_duration_ms,
            ..
        } => {
            assert_eq!(*trigger_mode, LinkedTriggerMode::Release);
            assert_eq!(*interval_ms, DEFAULT_COMBO_STEP_INTERVAL_MS);
            assert_eq!(*press_duration_ms, DEFAULT_COMBO_STEP_PRESS_DURATION_MS);
        }
        _ => panic!("expected linked key"),
    }
    assert!(profile.validate().is_ok());
}

#[test]
fn duplicate_special_key_names_are_rejected() {
    let profile = Profile {
        enabled_keys: Vec::new(),
        repeat_interval_ms: 1,
        press_duration_ms: 1,
        poll_interval_ms: 1,
        combos: Vec::new(),
        special_keys: vec![
            SpecialKeyConfig::LinkedKey {
                name: "special".to_string(),
                trigger_key: "A".to_string(),
                linked_key: "B".to_string(),
                trigger_mode: LinkedTriggerMode::Press,
                interval_ms: 1,
                press_duration_ms: 1,
            },
            SpecialKeyConfig::CustomAutofire {
                name: "SPECIAL".to_string(),
                key: "J".to_string(),
                repeat_interval_ms: 1,
                press_duration_ms: 1,
            },
        ],
    };

    assert!(profile.validate().is_err());
}

#[test]
fn load_or_create_uses_embedded_initial_config_template() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("dnf-init-template-{unique}.json"));
    if path.exists() {
        std::fs::remove_file(&path).expect("remove stale temp config");
    }

    let store = ConfigStore::load_or_create(&path).expect("load initial template");

    assert_eq!(store.default_profile, "默认配置");
    assert_eq!(store.quick_switch_hotkey, "LALT+BACKQUOTE");
    assert!(!store.hide_gui_on_startup);
    assert_eq!(store.input_backend, InputBackendKind::SendInputPolling);
    assert!(store.backend_settings.is_empty());
    assert!(
        store.profiles.contains_key("默认配置"),
        "expected embedded template profile to exist"
    );
    let profile = &store.profiles["默认配置"];
    assert_eq!(profile.repeat_interval_ms, DEFAULT_REPEAT_INTERVAL_MS);
    assert_eq!(profile.press_duration_ms, DEFAULT_PRESS_DURATION_MS);
    assert_eq!(profile.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);

    let saved = std::fs::read_to_string(&path).expect("read saved config");
    let saved_store: ConfigStore = serde_json::from_str(&saved).expect("parse saved config");
    assert_eq!(saved_store.default_profile, "默认配置");
    assert!(!saved_store.hide_gui_on_startup);

    std::fs::remove_file(PathBuf::from(&path)).expect("cleanup temp config");
}
