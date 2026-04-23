#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod autofire;
mod config;
mod gui;
mod gui_model;
mod initial_config;
mod input;
mod keymap;
mod single_instance;
mod timing;
mod win;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use config::{ComboConfig, ComboStepConfig, ConfigStore, Profile, SpecialKeyConfig};
use keymap::{normalize_hotkey_text, parse_hotkey, parse_key_specs, parse_single_key};
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
        #[arg(long, default_value_t = 1)]
        repeat_interval_ms: u64,
        #[arg(long, default_value_t = 1)]
        press_duration_ms: u64,
        #[arg(long, default_value_t = 1, hide = true)]
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
        #[arg(long, default_value_t = 80)]
        step_interval_ms: u64,
        #[arg(long, default_value_t = 1)]
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

fn main() {
    if let Err(err) = run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
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
            autofire::run(&profile, &store.target_windows)
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
                poll_interval_ms: 1,
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
