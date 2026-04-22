use crate::config::{ComboStepConfig, Profile};
use crate::input::{VK_ESCAPE, is_vk_down, send_key_once};
use crate::keymap::{KeySpec, parse_key_specs, parse_single_key};
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
    pub fn start(profile: Profile) -> Result<RunnerHandle> {
        Self::start_with_events(profile, |_| {})
    }

    pub fn start_with_events<F>(profile: Profile, on_event: F) -> Result<RunnerHandle>
    where
        F: Fn(RunnerEvent) + Send + 'static,
    {
        let runtime = RuntimeProfile::from_profile(&profile)?;
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

pub fn run(profile: &Profile) -> Result<()> {
    let runtime = RuntimeProfile::from_profile(profile)?;
    print_start_summary(&runtime);

    let mut handle = AutoFireService::start_with_events(profile.clone(), |event| match event {
        RunnerEvent::Started => {}
        RunnerEvent::PausedByIme => println!("检测到输入法开启，暂停连发。"),
        RunnerEvent::ResumedFromIme => println!("输入法关闭，恢复连发。"),
        RunnerEvent::Stopped(StopReason::EscapePressed) => println!("检测到 ESC，退出。"),
        RunnerEvent::Stopped(StopReason::StopRequested) => println!("连发已停止。"),
    })?;

    handle.wait()
}

#[derive(Clone)]
struct RuntimeProfile {
    keys: Vec<KeySpec>,
    combos: Vec<RuntimeCombo>,
    repeat_interval: Duration,
    press_duration: Duration,
    poll_interval: Duration,
    combo_trigger_vks: HashSet<u16>,
    target_windows: Vec<String>,
}

impl RuntimeProfile {
    fn from_profile(profile: &Profile) -> Result<Self> {
        profile.validate()?;

        let keys = if profile.enabled_keys.is_empty() {
            Vec::new()
        } else {
            parse_key_specs(&profile.enabled_keys).context("invalid enabled_keys in profile")?
        };
        let combos = build_runtime_combos(profile).context("invalid combos in profile")?;
        let combo_trigger_vks = combos.iter().map(|combo| combo.trigger.vk).collect();

        Ok(Self {
            keys,
            combos,
            repeat_interval: Duration::from_millis(profile.repeat_interval_ms.max(1)),
            press_duration: Duration::from_millis(profile.press_duration_ms.max(1)),
            poll_interval: Duration::from_millis(profile.poll_interval_ms.max(1)),
            combo_trigger_vks,
            target_windows: profile.target_windows.clone(),
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

struct ActiveCombo {
    name: String,
    steps: Vec<RuntimeComboStep>,
    next_index: usize,
    next_at: Instant,
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
                    on_event(RunnerEvent::PausedByIme);
                } else {
                    on_event(RunnerEvent::ResumedFromIme);
                }
            }

            if ime_blocking {
                update_trigger_states(&runtime.combos, &mut trigger_states);
                sleep(runtime.poll_interval);
                continue;
            }

            trigger_combos(&runtime.combos, &mut trigger_states, &mut active_combos);
            run_due_combo_steps(&mut active_combos);

            for key in &runtime.keys {
                if runtime.combo_trigger_vks.contains(&key.vk) {
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
        } else {
            update_trigger_states(&runtime.combos, &mut trigger_states);
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

fn update_trigger_states(combos: &[RuntimeCombo], trigger_states: &mut HashMap<u16, bool>) {
    for combo in combos {
        trigger_states.insert(combo.trigger.vk, is_vk_down(combo.trigger.vk));
    }
}

#[cfg(test)]
mod tests {
    use super::{AutoFireService, RunnerEvent, StopReason};
    use crate::config::{ComboConfig, ComboStepConfig, Profile};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn worker_can_start_and_stop_without_leaking_thread() {
        let profile = Profile::default();
        let (tx, rx) = channel();
        let mut handle = AutoFireService::start_with_events(profile, move |event| {
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
            target_windows: vec!["DNF".to_string()],
            quick_switch_hotkey: "LCTRL+Q".to_string(),
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
        };

        let runtime = super::RuntimeProfile::from_profile(&profile).expect("runtime profile");
        assert!(runtime.keys.is_empty());
        assert_eq!(runtime.combos.len(), 1);
        assert_eq!(runtime.combos[0].steps.len(), 3);
    }
}
