// Parses CLI commands and applies configuration actions.

use crate::autofire;
use crate::config::{
    ComboConfig, ComboStepConfig, ConfigStore, DEFAULT_COMBO_STEP_INTERVAL_MS,
    DEFAULT_COMBO_STEP_PRESS_DURATION_MS, DEFAULT_POLL_INTERVAL_MS, DEFAULT_PRESS_DURATION_MS,
    DEFAULT_REPEAT_INTERVAL_MS, Profile, SpecialKeyConfig,
};
use crate::gui;
use crate::keymap::{normalize_hotkey_text, parse_hotkey, parse_key_specs, parse_single_key};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about = "DNFAutoFire Rust 重构版（仅在目标窗口前台时连发）")]
struct Cli {
    #[arg(long, default_value = "configs.json")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run {
        #[arg(long)]
        profile: Option<String>,
    },
    Gui,
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    List,
    Show {
        #[arg(long)]
        profile: Option<String>,
    },
    Save {
        #[arg(long)]
        name: String,
        #[arg(long, value_delimiter = ',')]
        keys: Vec<String>,
        #[arg(long, default_value_t = DEFAULT_REPEAT_INTERVAL_MS)]
        repeat_interval_ms: u64,
        #[arg(long, default_value_t = DEFAULT_PRESS_DURATION_MS)]
        press_duration_ms: u64,
        #[arg(long, default_value_t = DEFAULT_POLL_INTERVAL_MS, hide = true)]
        poll_interval_ms: u64,
        #[arg(long, value_delimiter = ',')]
        windows: Vec<String>,
    },
    AddCombo {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        name: String,
        #[arg(long)]
        trigger_key: String,
        #[arg(long, value_delimiter = ',')]
        sequence_keys: Vec<String>,
        #[arg(long, default_value_t = DEFAULT_COMBO_STEP_INTERVAL_MS)]
        step_interval_ms: u64,
        #[arg(long, default_value_t = DEFAULT_COMBO_STEP_PRESS_DURATION_MS)]
        press_duration_ms: u64,
    },
    RemoveCombo {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        name: String,
    },
    Delete {
        #[arg(long)]
        name: String,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut store = ConfigStore::load_or_create(&cli.config)
        .with_context(|| format!("failed to load config file '{}'", cli.config.display()))?;

    match cli.command.unwrap_or(Command::Gui) {
        Command::Run { profile } => {
            let profile_name = store
                .profile_name_or_default(profile.as_deref())
                .to_string();
            let profile = store
                .get_profile(Some(&profile_name))
                .with_context(|| format!("profile not found: {profile_name}"))?;
            store.remember_started_profile(&profile_name)?;
            store.save(&cli.config)?;

            println!("使用配置: {profile_name}");
            validate_profile_bindings(&profile, &store.quick_switch_hotkey)?;
            autofire::run_with_backend(
                &profile,
                &store.target_windows,
                store.input_backend,
                &store.backend_settings,
            )
        }
        Command::Gui => gui::run(cli.config.clone(), store),
        Command::Config { action } => handle_config_action(&mut store, &cli.config, action),
    }
}

fn handle_config_action(
    store: &mut ConfigStore,
    config_path: &PathBuf,
    action: ConfigAction,
) -> Result<()> {
    match action {
        ConfigAction::List => {
            println!("配置列表:");
            for name in store.list_profile_names() {
                let marker = if name == store.default_profile {
                    " (default)"
                } else {
                    ""
                };
                println!("- {name}{marker}");
            }
            Ok(())
        }
        ConfigAction::Show { profile } => {
            let profile_name = store
                .profile_name_or_default(profile.as_deref())
                .to_string();
            let profile = store
                .get_profile(Some(&profile_name))
                .with_context(|| format!("profile not found: {profile_name}"))?;
            println!("{}", serde_json::to_string_pretty(&profile)?);
            Ok(())
        }
        ConfigAction::Save {
            name,
            keys,
            repeat_interval_ms,
            press_duration_ms,
            poll_interval_ms: _poll_interval_ms,
            windows,
        } => {
            if !windows.is_empty() {
                store.target_windows = windows;
            }

            let existing_profile = store.get_profile(Some(&name)).ok();
            let existing_combos = existing_profile
                .as_ref()
                .map(|profile| profile.combos.clone())
                .unwrap_or_default();
            let existing_special_keys = existing_profile
                .as_ref()
                .map(|profile| profile.special_keys.clone())
                .unwrap_or_default();
            let profile = Profile {
                enabled_keys: keys,
                repeat_interval_ms,
                press_duration_ms,
                poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
                combos: existing_combos,
                special_keys: existing_special_keys,
            }
            .normalized();
            profile.validate()?;
            if !profile.enabled_keys.is_empty() {
                parse_key_specs(&profile.enabled_keys)?;
            }
            validate_profile_bindings(&profile, &store.quick_switch_hotkey)?;

            store.upsert_profile(name.clone(), profile);
            store.save(config_path)?;

            println!("配置已保存: {name}");
            Ok(())
        }
        ConfigAction::AddCombo {
            profile,
            name,
            trigger_key,
            sequence_keys,
            step_interval_ms,
            press_duration_ms,
        } => {
            if sequence_keys.is_empty() {
                bail!("--sequence-keys 不能为空，例如: --sequence-keys A,S,D,F");
            }

            let profile_name = store
                .profile_name_or_default(profile.as_deref())
                .to_string();
            let mut profile = store
                .get_profile(Some(&profile_name))
                .with_context(|| format!("profile not found: {profile_name}"))?;

            let combo = ComboConfig {
                name: name.clone(),
                trigger_key,
                steps: sequence_keys
                    .into_iter()
                    .map(|key| ComboStepConfig {
                        key,
                        interval_ms: step_interval_ms,
                        press_duration_ms,
                    })
                    .collect(),
                sequence_keys: Vec::new(),
                step_interval_ms: 0,
                press_duration_ms: 0,
            }
            .normalized();
            combo.validate()?;
            parse_single_key(&combo.trigger_key)?;
            for step in &combo.steps {
                parse_single_key(&step.key)?;
            }

            profile
                .combos
                .retain(|existing| !existing.name.eq_ignore_ascii_case(&combo.name));
            profile.combos.push(combo);
            profile = profile.normalized();
            profile.validate()?;
            validate_profile_bindings(&profile, &store.quick_switch_hotkey)?;

            store.upsert_profile(profile_name.clone(), profile);
            store.save(config_path)?;

            println!("连招已保存到配置 {profile_name}: {name}");
            Ok(())
        }
        ConfigAction::RemoveCombo { profile, name } => {
            let profile_name = store
                .profile_name_or_default(profile.as_deref())
                .to_string();
            let mut profile = store
                .get_profile(Some(&profile_name))
                .with_context(|| format!("profile not found: {profile_name}"))?;

            let before = profile.combos.len();
            profile
                .combos
                .retain(|combo| !combo.name.eq_ignore_ascii_case(&name));
            if profile.combos.len() == before {
                bail!("combo not found in profile {profile_name}: {name}");
            }

            store.upsert_profile(profile_name.clone(), profile);
            store.save(config_path)?;

            println!("已从配置 {profile_name} 删除连招: {name}");
            Ok(())
        }
        ConfigAction::Delete { name } => {
            store.delete_profile(&name)?;
            store.save(config_path)?;
            println!("配置已删除: {name}");
            Ok(())
        }
    }
}

fn validate_profile_bindings(profile: &Profile, quick_switch_hotkey: &str) -> Result<()> {
    let normalized_quick_switch_hotkey = normalize_hotkey_text(quick_switch_hotkey)?;

    for combo in &profile.combos {
        parse_single_key(&combo.trigger_key)?;
        for step in &combo.steps {
            parse_single_key(&step.key)?;
        }
    }

    for special in &profile.special_keys {
        match special {
            SpecialKeyConfig::CustomAutofire { key, .. } => {
                parse_single_key(key)?;
            }
            SpecialKeyConfig::AutoTrigger {
                key,
                trigger_hotkey,
                ..
            } => {
                parse_single_key(key)?;
                let normalized_trigger_hotkey = normalize_hotkey_text(trigger_hotkey)?;
                parse_hotkey(&normalized_trigger_hotkey)?;
                if normalized_trigger_hotkey == normalized_quick_switch_hotkey {
                    bail!("特殊键位自动触发热键不能与全局快速切换热键冲突");
                }
            }
            SpecialKeyConfig::LinkedKey {
                trigger_key,
                linked_key,
                ..
            } => {
                parse_single_key(trigger_key)?;
                parse_single_key(linked_key)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ConfigAction, handle_config_action, validate_profile_bindings};
    use crate::config::{
        ComboConfig, ComboStepConfig, ConfigStore, DEFAULT_POLL_INTERVAL_MS, Profile,
        SpecialKeyConfig,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn profile_bindings_reject_auto_trigger_quick_switch_conflict() {
        let profile = Profile {
            enabled_keys: vec!["J".to_string()],
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            combos: Vec::new(),
            special_keys: vec![SpecialKeyConfig::AutoTrigger {
                name: "auto".to_string(),
                key: "SPACE".to_string(),
                trigger_hotkey: "LALT+Q".to_string(),
                repeat_interval_ms: 1,
                press_duration_ms: 1,
            }],
        };

        let err = validate_profile_bindings(&profile, "LALT+Q")
            .expect_err("matching quick switch hotkey should be rejected");
        assert!(err.to_string().contains("冲突"));
    }

    #[test]
    fn config_save_preserves_existing_combos_and_special_keys() {
        let path = unique_test_config_path();
        let mut store = ConfigStore::default();
        let profile = Profile {
            enabled_keys: vec!["J".to_string()],
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            combos: vec![ComboConfig {
                name: "burst".to_string(),
                trigger_key: "U".to_string(),
                steps: vec![ComboStepConfig {
                    key: "A".to_string(),
                    interval_ms: 8,
                    press_duration_ms: 1,
                }],
                sequence_keys: Vec::new(),
                step_interval_ms: 0,
                press_duration_ms: 0,
            }],
            special_keys: vec![SpecialKeyConfig::CustomAutofire {
                name: "custom".to_string(),
                key: "O".to_string(),
                repeat_interval_ms: 2,
                press_duration_ms: 1,
            }],
        };
        store.upsert_profile("raid".to_string(), profile);

        handle_config_action(
            &mut store,
            &path,
            ConfigAction::Save {
                name: "raid".to_string(),
                keys: vec!["K".to_string()],
                repeat_interval_ms: 12,
                press_duration_ms: 3,
                poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
                windows: Vec::new(),
            },
        )
        .expect("config save should succeed");

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        let profile = saved.get_profile(Some("raid")).expect("saved profile");
        assert_eq!(profile.enabled_keys, vec!["K"]);
        assert_eq!(profile.combos.len(), 1);
        assert_eq!(profile.combos[0].name, "burst");
        assert_eq!(profile.special_keys.len(), 1);
        assert_eq!(profile.special_keys[0].name(), "custom");

        let _ = std::fs::remove_file(path);
    }

    fn unique_test_config_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("dnf_cli_test_{nanos}.json"))
    }
}
