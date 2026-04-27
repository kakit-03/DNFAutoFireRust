use crate::config::{ComboStepConfig, LinkedTriggerMode, Profile, SpecialKeyConfig};
use crate::input::backend::{InputBackendKind, input_backend_label};
use crate::keymap::{KeySpec, parse_hotkey, parse_key_specs, parse_single_key};
use crate::timing::{SleepTimingMonitor, SleepTimingSnapshot};
use anyhow::{Context, Result};
use std::collections::HashSet;

#[derive(Clone)]
pub(super) struct RuntimeProfile {
    pub(super) keys: Vec<KeySpec>,
    pub(super) combos: Vec<RuntimeCombo>,
    pub(super) custom_autofires: Vec<RuntimeCustomAutofire>,
    pub(super) auto_triggers: Vec<RuntimeAutoTrigger>,
    pub(super) linked_keys: Vec<RuntimeLinkedKey>,
    pub(super) monitored_vks: Vec<u16>,
    pub(super) repeat_interval_ms: u64,
    pub(super) press_duration_ms: u64,
    pub(super) target_windows: Vec<String>,
    pub(super) input_backend: InputBackendKind,
    sleep_timing: SleepTimingMonitor,
}

impl RuntimeProfile {
    #[cfg(test)]
    pub(super) fn from_profile(profile: &Profile, target_windows: &[String]) -> Result<Self> {
        Self::from_profile_with_backend(profile, target_windows, InputBackendKind::default())
    }

    pub(super) fn from_profile_with_backend(
        profile: &Profile,
        target_windows: &[String],
        input_backend: InputBackendKind,
    ) -> Result<Self> {
        profile.validate()?;
        if target_windows.is_empty() {
            return Err(anyhow::anyhow!("target_windows cannot be empty"));
        }

        let keys = if profile.enabled_keys.is_empty() {
            Vec::new()
        } else {
            parse_key_specs(&profile.enabled_keys).context("invalid enabled_keys in profile")?
        };
        let combos = build_runtime_combos(profile).context("invalid combos in profile")?;
        let custom_autofires = build_runtime_custom_autofires(profile)
            .context("invalid custom autofire config in profile")?;
        let auto_triggers = build_runtime_auto_triggers(profile)
            .context("invalid auto trigger config in profile")?;
        let linked_keys =
            build_runtime_linked_keys(profile).context("invalid linked key config in profile")?;
        let monitored_vks = collect_monitored_vks(
            &keys,
            &combos,
            &custom_autofires,
            &auto_triggers,
            &linked_keys,
        );

        Ok(Self {
            keys,
            combos,
            custom_autofires,
            auto_triggers,
            linked_keys,
            monitored_vks,
            repeat_interval_ms: profile.repeat_interval_ms.max(1),
            press_duration_ms: profile.press_duration_ms.max(1),
            target_windows: target_windows.to_vec(),
            input_backend,
            sleep_timing: SleepTimingMonitor::shared(),
        })
    }

    pub(super) fn sleep_timing_snapshot(&self) -> SleepTimingSnapshot {
        self.sleep_timing.snapshot()
    }
}

#[derive(Clone)]
pub(super) struct RuntimeCombo {
    pub(super) name: String,
    pub(super) trigger: KeySpec,
    pub(super) steps: Vec<RuntimeComboStep>,
}

#[derive(Clone)]
pub(super) struct RuntimeComboStep {
    pub(super) key: KeySpec,
    pub(super) interval_ms: u64,
    pub(super) press_duration_ms: u64,
}

#[derive(Clone)]
pub(super) struct RuntimeCustomAutofire {
    pub(super) name: String,
    pub(super) key: KeySpec,
    pub(super) repeat_interval_ms: u64,
    pub(super) press_duration_ms: u64,
}

#[derive(Clone)]
pub(super) struct RuntimeHotkey {
    pub(super) text: String,
    pub(super) specs: Vec<KeySpec>,
}

#[derive(Clone)]
pub(super) struct RuntimeAutoTrigger {
    pub(super) name: String,
    pub(super) key: KeySpec,
    pub(super) trigger_hotkey: RuntimeHotkey,
    pub(super) repeat_interval_ms: u64,
    pub(super) press_duration_ms: u64,
}

#[derive(Clone)]
pub(super) struct RuntimeLinkedKey {
    pub(super) name: String,
    pub(super) trigger_key: KeySpec,
    pub(super) linked_key: KeySpec,
    pub(super) trigger_mode: LinkedTriggerMode,
    pub(super) interval_ms: u64,
    pub(super) press_duration_ms: u64,
}

pub(super) fn collect_monitored_vks(
    keys: &[KeySpec],
    combos: &[RuntimeCombo],
    custom_autofires: &[RuntimeCustomAutofire],
    auto_triggers: &[RuntimeAutoTrigger],
    linked_keys: &[RuntimeLinkedKey],
) -> Vec<u16> {
    let mut monitored = Vec::new();
    let mut seen = HashSet::new();

    for key in keys {
        if seen.insert(key.vk) {
            monitored.push(key.vk);
        }
    }
    for combo in combos {
        if seen.insert(combo.trigger.vk) {
            monitored.push(combo.trigger.vk);
        }
    }
    for custom in custom_autofires {
        if seen.insert(custom.key.vk) {
            monitored.push(custom.key.vk);
        }
    }
    for trigger in auto_triggers {
        for spec in &trigger.trigger_hotkey.specs {
            if seen.insert(spec.vk) {
                monitored.push(spec.vk);
            }
        }
    }
    for linked in linked_keys {
        if seen.insert(linked.trigger_key.vk) {
            monitored.push(linked.trigger_key.vk);
        }
    }

    monitored
}

pub(super) fn print_start_summary(runtime: &RuntimeProfile) {
    let timing = runtime.sleep_timing_snapshot();
    println!(
        "连发启动: keys=[{}], repeat={}ms, press={}ms, scheduler={}ms, sleep={}ms",
        runtime
            .keys
            .iter()
            .map(|k| k.name)
            .collect::<Vec<_>>()
            .join(","),
        runtime.repeat_interval_ms,
        runtime.press_duration_ms,
        timing.scheduler_interval_ms,
        timing.measured_granularity_ms
    );
    println!("输入后端: {}", input_backend_label(runtime.input_backend));
    println!(
        "高精度定时: timer_resolution={}, hidden_window_fix={}, high_res_waitable_timer={}",
        yes_no(timing.timer_resolution_requested),
        yes_no(timing.occlusion_workaround_enabled),
        yes_no(timing.high_resolution_waitable_timer)
    );
    println!("目标窗口关键字: {}", runtime.target_windows.join(", "));
    if runtime.combos.is_empty() {
        println!("当前未配置一键连招。");
    } else {
        println!(
            "已加载连招: {}",
            runtime
                .combos
                .iter()
                .map(|combo| {
                    format!(
                        "{}({} -> {})",
                        combo.name,
                        combo.trigger.name,
                        combo
                            .steps
                            .iter()
                            .map(|step| {
                                format!(
                                    "{}@{}ms/{}ms",
                                    step.key.name, step.interval_ms, step.press_duration_ms
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    if runtime.custom_autofires.is_empty()
        && runtime.auto_triggers.is_empty()
        && runtime.linked_keys.is_empty()
    {
        println!("当前未配置特殊键位。");
    } else {
        let mut items = Vec::new();
        items.extend(runtime.custom_autofires.iter().map(|entry| {
            format!(
                "{}[独立连发:{}@{}ms/{}ms]",
                entry.name, entry.key.name, entry.repeat_interval_ms, entry.press_duration_ms
            )
        }));
        items.extend(runtime.auto_triggers.iter().map(|entry| {
            format!(
                "{}[自动触发:{} <= {} @{}ms/{}ms]",
                entry.name,
                entry.key.name,
                entry.trigger_hotkey.text,
                entry.repeat_interval_ms,
                entry.press_duration_ms
            )
        }));
        items.extend(runtime.linked_keys.iter().map(|entry| {
            format!(
                "{}[连携:{}{} -> {} @{}ms/{}ms]",
                entry.name,
                entry.trigger_key.name,
                linked_trigger_mode_label(entry.trigger_mode),
                entry.linked_key.name,
                entry.interval_ms,
                entry.press_duration_ms
            )
        }));
        println!("已加载特殊键位: {}", items.join("; "));
    }
    println!("按住配置中的按键触发连发。");
}

fn build_runtime_combos(profile: &Profile) -> Result<Vec<RuntimeCombo>> {
    profile
        .combos
        .iter()
        .map(|combo| {
            let trigger = parse_single_key(&combo.trigger_key)
                .with_context(|| format!("invalid trigger_key in combo '{}'", combo.name))?;
            let steps = build_runtime_combo_steps(&combo.steps)
                .with_context(|| format!("invalid steps in combo '{}'", combo.name))?;

            Ok(RuntimeCombo {
                name: combo.name.clone(),
                trigger,
                steps,
            })
        })
        .collect()
}

fn build_runtime_combo_steps(steps: &[ComboStepConfig]) -> Result<Vec<RuntimeComboStep>> {
    steps
        .iter()
        .map(|step| {
            let key = parse_single_key(&step.key)?;
            Ok(RuntimeComboStep {
                key,
                interval_ms: step.interval_ms.max(1),
                press_duration_ms: step.press_duration_ms.max(1),
            })
        })
        .collect()
}

fn build_runtime_custom_autofires(profile: &Profile) -> Result<Vec<RuntimeCustomAutofire>> {
    profile
        .special_keys
        .iter()
        .filter_map(|special| match special {
            SpecialKeyConfig::CustomAutofire {
                name,
                key,
                repeat_interval_ms,
                press_duration_ms,
            } => Some((name, key, repeat_interval_ms, press_duration_ms)),
            _ => None,
        })
        .map(|(name, key, repeat_interval_ms, press_duration_ms)| {
            let key = parse_single_key(key)?;
            Ok(RuntimeCustomAutofire {
                name: name.clone(),
                key,
                repeat_interval_ms: (*repeat_interval_ms).max(1),
                press_duration_ms: (*press_duration_ms).max(1),
            })
        })
        .collect()
}

fn build_runtime_auto_triggers(profile: &Profile) -> Result<Vec<RuntimeAutoTrigger>> {
    profile
        .special_keys
        .iter()
        .filter_map(|special| match special {
            SpecialKeyConfig::AutoTrigger {
                name,
                key,
                trigger_hotkey,
                repeat_interval_ms,
                press_duration_ms,
            } => Some((
                name,
                key,
                trigger_hotkey,
                repeat_interval_ms,
                press_duration_ms,
            )),
            _ => None,
        })
        .map(
            |(name, key, trigger_hotkey, repeat_interval_ms, press_duration_ms)| {
                let key = parse_single_key(key)?;
                let specs = parse_hotkey(trigger_hotkey)?;
                Ok(RuntimeAutoTrigger {
                    name: name.clone(),
                    key,
                    trigger_hotkey: RuntimeHotkey {
                        text: trigger_hotkey.clone(),
                        specs,
                    },
                    repeat_interval_ms: (*repeat_interval_ms).max(1),
                    press_duration_ms: (*press_duration_ms).max(1),
                })
            },
        )
        .collect()
}

fn build_runtime_linked_keys(profile: &Profile) -> Result<Vec<RuntimeLinkedKey>> {
    profile
        .special_keys
        .iter()
        .filter_map(|special| match special {
            SpecialKeyConfig::LinkedKey {
                name,
                trigger_key,
                linked_key,
                trigger_mode,
                interval_ms,
                press_duration_ms,
            } => Some((
                name,
                trigger_key,
                linked_key,
                *trigger_mode,
                interval_ms,
                press_duration_ms,
            )),
            _ => None,
        })
        .map(
            |(name, trigger_key, linked_key, trigger_mode, interval_ms, press_duration_ms)| {
                Ok(RuntimeLinkedKey {
                    name: name.clone(),
                    trigger_key: parse_single_key(trigger_key)?,
                    linked_key: parse_single_key(linked_key)?,
                    trigger_mode,
                    interval_ms: (*interval_ms).max(1),
                    press_duration_ms: (*press_duration_ms).max(1),
                })
            },
        )
        .collect()
}

fn yes_no(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn linked_trigger_mode_label(mode: LinkedTriggerMode) -> &'static str {
    match mode {
        LinkedTriggerMode::Press => "[按下]",
        LinkedTriggerMode::Release => "[松开]",
    }
}
