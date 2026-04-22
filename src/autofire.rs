use crate::config::{ComboStepConfig, Profile, SpecialKeyConfig};
use crate::input::{VK_ESCAPE, is_vk_down, send_key_once};
use crate::keymap::{KeySpec, parse_hotkey, parse_key_specs, parse_single_key};
use crate::win::{foreground_window_ime_open, foreground_window_title, is_target_window};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle, sleep};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EscapePressed,
    StopRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerEvent {
    Started,
    PausedByIme,
    ResumedFromIme,
    Stopped(StopReason),
}

pub struct AutoFireService;

impl AutoFireService {
    #[allow(dead_code)]
    pub fn start(profile: Profile, target_windows: Vec<String>) -> Result<RunnerHandle> {
        Self::start_with_events(profile, target_windows, |_| {})
    }

    pub fn start_with_events<F>(
        profile: Profile,
        target_windows: Vec<String>,
        on_event: F,
    ) -> Result<RunnerHandle>
    where
        F: Fn(RunnerEvent) + Send + 'static,
    {
        let runtime = RuntimeProfile::from_profile(&profile, &target_windows)?;
        let stop_requested = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));

        let stop_for_thread = Arc::clone(&stop_requested);
        let running_for_thread = Arc::clone(&running);
        let join = thread::spawn(move || {
            let result = run_loop(runtime, stop_for_thread, on_event);
            running_for_thread.store(false, Ordering::SeqCst);
            result
        });

        Ok(RunnerHandle {
            stop_requested,
            running,
            join: Some(join),
        })
    }
}

pub struct RunnerHandle {
    stop_requested: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<()>>>,
}

impl RunnerHandle {
    pub fn stop(&self) {
        self.stop_requested.store(true, Ordering::SeqCst);
    }

    pub fn wait(&mut self) -> Result<()> {
        let Some(join) = self.join.take() else {
            return Ok(());
        };

        match join.join() {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("autofire worker thread panicked")),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Drop for RunnerHandle {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn run(profile: &Profile, target_windows: &[String]) -> Result<()> {
    let runtime = RuntimeProfile::from_profile(profile, target_windows)?;
    print_start_summary(&runtime);

    let mut handle =
        AutoFireService::start_with_events(profile.clone(), target_windows.to_vec(), |event| {
            match event {
                RunnerEvent::Started => {}
                RunnerEvent::PausedByIme => println!("检测到输入法开启，暂停连发。"),
                RunnerEvent::ResumedFromIme => println!("输入法关闭，恢复连发。"),
                RunnerEvent::Stopped(StopReason::EscapePressed) => println!("检测到 ESC，退出。"),
                RunnerEvent::Stopped(StopReason::StopRequested) => println!("连发已停止。"),
            }
        })?;

    handle.wait()
}

#[derive(Clone)]
struct RuntimeProfile {
    keys: Vec<KeySpec>,
    combos: Vec<RuntimeCombo>,
    custom_autofires: Vec<RuntimeCustomAutofire>,
    auto_triggers: Vec<RuntimeAutoTrigger>,
    linked_keys: Vec<RuntimeLinkedKey>,
    repeat_interval: Duration,
    press_duration: Duration,
    poll_interval: Duration,
    combo_trigger_vks: HashSet<u16>,
    custom_autofire_vks: HashSet<u16>,
    target_windows: Vec<String>,
}

impl RuntimeProfile {
    fn from_profile(profile: &Profile, target_windows: &[String]) -> Result<Self> {
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
        let combo_trigger_vks = combos.iter().map(|combo| combo.trigger.vk).collect();
        let custom_autofire_vks = custom_autofires.iter().map(|entry| entry.key.vk).collect();

        Ok(Self {
            keys,
            combos,
            custom_autofires,
            auto_triggers,
            linked_keys,
            repeat_interval: Duration::from_millis(profile.repeat_interval_ms.max(1)),
            press_duration: Duration::from_millis(profile.press_duration_ms.max(1)),
            poll_interval: Duration::from_millis(profile.poll_interval_ms.max(1)),
            combo_trigger_vks,
            custom_autofire_vks,
            target_windows: target_windows.to_vec(),
        })
    }
}

#[derive(Clone)]
struct RuntimeCombo {
    name: String,
    trigger: KeySpec,
    steps: Vec<RuntimeComboStep>,
}

#[derive(Clone)]
struct RuntimeComboStep {
    key: KeySpec,
    interval: Duration,
    press_duration: Duration,
}

#[derive(Clone)]
struct RuntimeCustomAutofire {
    name: String,
    key: KeySpec,
    repeat_interval: Duration,
    press_duration: Duration,
}

#[derive(Clone)]
struct RuntimeHotkey {
    text: String,
    specs: Vec<KeySpec>,
}

#[derive(Clone)]
struct RuntimeAutoTrigger {
    name: String,
    key: KeySpec,
    trigger_hotkey: RuntimeHotkey,
    repeat_interval: Duration,
    press_duration: Duration,
}

#[derive(Clone)]
struct RuntimeLinkedKey {
    name: String,
    trigger_key: KeySpec,
    linked_key: KeySpec,
    interval: Duration,
    press_duration: Duration,
}

struct ActiveCombo {
    name: String,
    steps: Vec<RuntimeComboStep>,
    next_index: usize,
    next_at: Instant,
}

struct PendingLinkedKey {
    key: KeySpec,
    execute_at: Instant,
    press_duration: Duration,
}

fn print_start_summary(runtime: &RuntimeProfile) {
    println!(
        "连发启动: keys=[{}], repeat={}ms, press={}ms, poll={}ms",
        runtime
            .keys
            .iter()
            .map(|k| k.name)
            .collect::<Vec<_>>()
            .join(","),
        runtime.repeat_interval.as_millis(),
        runtime.press_duration.as_millis(),
        runtime.poll_interval.as_millis()
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
                                    step.key.name,
                                    step.interval.as_millis(),
                                    step.press_duration.as_millis()
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
                entry.name,
                entry.key.name,
                entry.repeat_interval.as_millis(),
                entry.press_duration.as_millis()
            )
        }));
        items.extend(runtime.auto_triggers.iter().map(|entry| {
            format!(
                "{}[自动触发:{} <= {} @{}ms/{}ms]",
                entry.name,
                entry.key.name,
                entry.trigger_hotkey.text,
                entry.repeat_interval.as_millis(),
                entry.press_duration.as_millis()
            )
        }));
        items.extend(runtime.linked_keys.iter().map(|entry| {
            format!(
                "{}[连携:{} -> {} @{}ms/{}ms]",
                entry.name,
                entry.trigger_key.name,
                entry.linked_key.name,
                entry.interval.as_millis(),
                entry.press_duration.as_millis()
            )
        }));
        println!("已加载特殊键位: {}", items.join("; "));
    }
    println!("按住配置中的按键触发连发，按 ESC 退出。");
}

fn run_loop<F>(runtime: RuntimeProfile, stop_requested: Arc<AtomicBool>, on_event: F) -> Result<()>
where
    F: Fn(RunnerEvent),
{
    on_event(RunnerEvent::Started);

    let mut last_sent_at: HashMap<u16, Instant> = HashMap::new();
    let mut trigger_states: HashMap<u16, bool> = runtime
        .combos
        .iter()
        .map(|combo| (combo.trigger.vk, false))
        .collect();
    let mut active_combos: Vec<ActiveCombo> = Vec::new();
    let mut auto_trigger_hotkey_states = vec![false; runtime.auto_triggers.len()];
    let mut auto_trigger_enabled = vec![false; runtime.auto_triggers.len()];
    let mut auto_trigger_last_sent_at = vec![None; runtime.auto_triggers.len()];
    let mut linked_key_trigger_states = vec![false; runtime.linked_keys.len()];
    let mut pending_linked_keys: Vec<PendingLinkedKey> = Vec::new();
    let mut is_active = false;
    let mut ime_blocking = false;

    let stop_reason = loop {
        if stop_requested.load(Ordering::SeqCst) {
            break StopReason::StopRequested;
        }

        if is_vk_down(VK_ESCAPE) {
            break StopReason::EscapePressed;
        }

        let title = foreground_window_title();
        let active_now = is_target_window(&title, &runtime.target_windows);

        if active_now != is_active {
            is_active = active_now;
            if !is_active {
                last_sent_at.clear();
                active_combos.clear();
                pending_linked_keys.clear();
                if ime_blocking {
                    ime_blocking = false;
                    on_event(RunnerEvent::ResumedFromIme);
                }
            }
        }

        if is_active {
            let ime_open = foreground_window_ime_open();
            if ime_open != ime_blocking {
                ime_blocking = ime_open;
                if ime_blocking {
                    last_sent_at.clear();
                    active_combos.clear();
                    pending_linked_keys.clear();
                    on_event(RunnerEvent::PausedByIme);
                } else {
                    on_event(RunnerEvent::ResumedFromIme);
                }
            }

            if ime_blocking {
                update_trigger_states(&runtime.combos, &mut trigger_states);
                update_auto_trigger_states(&runtime.auto_triggers, &mut auto_trigger_hotkey_states);
                update_linked_key_states(&runtime.linked_keys, &mut linked_key_trigger_states);
                sleep(runtime.poll_interval);
                continue;
            }

            trigger_combos(&runtime.combos, &mut trigger_states, &mut active_combos);
            toggle_auto_triggers(
                &runtime.auto_triggers,
                &mut auto_trigger_hotkey_states,
                &mut auto_trigger_enabled,
                &mut auto_trigger_last_sent_at,
            );
            trigger_linked_keys(
                &runtime.linked_keys,
                &mut linked_key_trigger_states,
                &mut pending_linked_keys,
            );
            run_due_combo_steps(&mut active_combos);
            run_due_linked_keys(&mut pending_linked_keys);
            run_custom_autofires(&runtime.custom_autofires, &mut last_sent_at);

            for key in &runtime.keys {
                if runtime.combo_trigger_vks.contains(&key.vk)
                    || runtime.custom_autofire_vks.contains(&key.vk)
                {
                    continue;
                }
                if !is_vk_down(key.vk) {
                    last_sent_at.remove(&key.vk);
                    continue;
                }

                let should_send = last_sent_at
                    .get(&key.vk)
                    .map(|ts| ts.elapsed() >= runtime.repeat_interval)
                    .unwrap_or(true);
                if !should_send {
                    continue;
                }

                send_key_once(*key, runtime.press_duration);
                last_sent_at.insert(key.vk, Instant::now());
            }
            run_enabled_auto_triggers(
                &runtime.auto_triggers,
                &auto_trigger_enabled,
                &mut auto_trigger_last_sent_at,
            );
        } else {
            update_trigger_states(&runtime.combos, &mut trigger_states);
            update_auto_trigger_states(&runtime.auto_triggers, &mut auto_trigger_hotkey_states);
            update_linked_key_states(&runtime.linked_keys, &mut linked_key_trigger_states);
        }

        sleep(runtime.poll_interval);
    };

    on_event(RunnerEvent::Stopped(stop_reason));
    Ok(())
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
                interval: Duration::from_millis(step.interval_ms.max(1)),
                press_duration: Duration::from_millis(step.press_duration_ms.max(1)),
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
                repeat_interval: Duration::from_millis((*repeat_interval_ms).max(1)),
                press_duration: Duration::from_millis((*press_duration_ms).max(1)),
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
                    repeat_interval: Duration::from_millis((*repeat_interval_ms).max(1)),
                    press_duration: Duration::from_millis((*press_duration_ms).max(1)),
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
                interval_ms,
                press_duration_ms,
            } => Some((
                name,
                trigger_key,
                linked_key,
                interval_ms,
                press_duration_ms,
            )),
            _ => None,
        })
        .map(
            |(name, trigger_key, linked_key, interval_ms, press_duration_ms)| {
                Ok(RuntimeLinkedKey {
                    name: name.clone(),
                    trigger_key: parse_single_key(trigger_key)?,
                    linked_key: parse_single_key(linked_key)?,
                    interval: Duration::from_millis((*interval_ms).max(1)),
                    press_duration: Duration::from_millis((*press_duration_ms).max(1)),
                })
            },
        )
        .collect()
}

fn trigger_combos(
    combos: &[RuntimeCombo],
    trigger_states: &mut HashMap<u16, bool>,
    active_combos: &mut Vec<ActiveCombo>,
) {
    for combo in combos {
        let is_down = is_vk_down(combo.trigger.vk);
        let was_down = trigger_states
            .get(&combo.trigger.vk)
            .copied()
            .unwrap_or(false);
        if is_down && !was_down {
            active_combos.retain(|active| active.name != combo.name);
            active_combos.push(ActiveCombo {
                name: combo.name.clone(),
                steps: combo.steps.clone(),
                next_index: 0,
                next_at: Instant::now(),
            });
        }
        trigger_states.insert(combo.trigger.vk, is_down);
    }
}

fn run_custom_autofires(
    custom_autofires: &[RuntimeCustomAutofire],
    last_sent_at: &mut HashMap<u16, Instant>,
) {
    for custom in custom_autofires {
        if !is_vk_down(custom.key.vk) {
            last_sent_at.remove(&custom.key.vk);
            continue;
        }

        let should_send = last_sent_at
            .get(&custom.key.vk)
            .map(|ts| ts.elapsed() >= custom.repeat_interval)
            .unwrap_or(true);
        if !should_send {
            continue;
        }

        send_key_once(custom.key, custom.press_duration);
        last_sent_at.insert(custom.key.vk, Instant::now());
    }
}

fn run_due_combo_steps(active_combos: &mut Vec<ActiveCombo>) {
    let now = Instant::now();
    let mut completed = Vec::new();

    for (index, combo) in active_combos.iter_mut().enumerate() {
        if now < combo.next_at || combo.next_index >= combo.steps.len() {
            continue;
        }

        let step = combo.steps[combo.next_index].clone();
        send_key_once(step.key, step.press_duration);
        combo.next_index += 1;

        if combo.next_index >= combo.steps.len() {
            completed.push(index);
        } else {
            combo.next_at = Instant::now() + step.interval;
        }
    }

    for index in completed.into_iter().rev() {
        active_combos.remove(index);
    }
}

fn toggle_auto_triggers(
    auto_triggers: &[RuntimeAutoTrigger],
    trigger_states: &mut [bool],
    enabled_states: &mut [bool],
    last_sent_at: &mut [Option<Instant>],
) {
    for (index, trigger) in auto_triggers.iter().enumerate() {
        let is_down = hotkey_is_down(&trigger.trigger_hotkey);
        let was_down = trigger_states.get(index).copied().unwrap_or(false);
        if is_down && !was_down {
            enabled_states[index] = !enabled_states[index];
            if !enabled_states[index] {
                last_sent_at[index] = None;
            }
        }
        trigger_states[index] = is_down;
    }
}

fn update_auto_trigger_states(auto_triggers: &[RuntimeAutoTrigger], trigger_states: &mut [bool]) {
    for (index, trigger) in auto_triggers.iter().enumerate() {
        trigger_states[index] = hotkey_is_down(&trigger.trigger_hotkey);
    }
}

fn run_enabled_auto_triggers(
    auto_triggers: &[RuntimeAutoTrigger],
    enabled_states: &[bool],
    last_sent_at: &mut [Option<Instant>],
) {
    for (index, trigger) in auto_triggers.iter().enumerate() {
        if !enabled_states.get(index).copied().unwrap_or(false) {
            continue;
        }

        let should_send = last_sent_at[index]
            .map(|ts| ts.elapsed() >= trigger.repeat_interval)
            .unwrap_or(true);
        if !should_send {
            continue;
        }

        send_key_once(trigger.key, trigger.press_duration);
        last_sent_at[index] = Some(Instant::now());
    }
}

fn trigger_linked_keys(
    linked_keys: &[RuntimeLinkedKey],
    trigger_states: &mut [bool],
    pending_linked_keys: &mut Vec<PendingLinkedKey>,
) {
    for (index, linked) in linked_keys.iter().enumerate() {
        let is_down = is_vk_down(linked.trigger_key.vk);
        let was_down = trigger_states.get(index).copied().unwrap_or(false);
        if is_down && !was_down {
            pending_linked_keys.push(PendingLinkedKey {
                key: linked.linked_key,
                execute_at: Instant::now() + linked.interval,
                press_duration: linked.press_duration,
            });
        }
        trigger_states[index] = is_down;
    }
}

fn update_linked_key_states(linked_keys: &[RuntimeLinkedKey], trigger_states: &mut [bool]) {
    for (index, linked) in linked_keys.iter().enumerate() {
        trigger_states[index] = is_vk_down(linked.trigger_key.vk);
    }
}

fn run_due_linked_keys(pending_linked_keys: &mut Vec<PendingLinkedKey>) {
    let now = Instant::now();
    let mut completed = Vec::new();
    for (index, pending) in pending_linked_keys.iter().enumerate() {
        if now < pending.execute_at {
            continue;
        }
        send_key_once(pending.key, pending.press_duration);
        completed.push(index);
    }

    for index in completed.into_iter().rev() {
        pending_linked_keys.remove(index);
    }
}

fn update_trigger_states(combos: &[RuntimeCombo], trigger_states: &mut HashMap<u16, bool>) {
    for combo in combos {
        trigger_states.insert(combo.trigger.vk, is_vk_down(combo.trigger.vk));
    }
}

fn hotkey_is_down(hotkey: &RuntimeHotkey) -> bool {
    hotkey.specs.iter().all(|spec| is_vk_down(spec.vk))
}

#[cfg(test)]
mod tests {
    use super::{AutoFireService, RunnerEvent, StopReason};
    use crate::config::{ComboConfig, ComboStepConfig, Profile, SpecialKeyConfig};
    use std::sync::mpsc::channel;
    use std::time::Duration;

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
    }
}
