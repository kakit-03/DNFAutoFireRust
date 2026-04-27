use crate::config::{ComboStepConfig, LinkedTriggerMode, Profile, SpecialKeyConfig};
use crate::input::backend::{
    BackendSettings, InputBackend, InputBackendKind, InputSnapshot, create_input_backend,
    input_backend_label,
};
use crate::keymap::{KeySpec, parse_hotkey, parse_key_specs, parse_single_key};
use crate::platform::window::{foreground_window_info, window_ime_open};
use crate::timing::{HighPrecisionSleeper, SleepTimingMonitor, SleepTimingSnapshot};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
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
        Self::start_with_backend_events(
            profile,
            target_windows,
            InputBackendKind::default(),
            BackendSettings::default(),
            on_event,
        )
    }

    pub fn start_with_backend_events<F>(
        profile: Profile,
        target_windows: Vec<String>,
        input_backend: InputBackendKind,
        backend_settings: BackendSettings,
        on_event: F,
    ) -> Result<RunnerHandle>
    where
        F: Fn(RunnerEvent) + Send + 'static,
    {
        let runtime =
            RuntimeProfile::from_profile_with_backend(&profile, &target_windows, input_backend)?;
        let backend = create_input_backend(input_backend, &backend_settings)?;
        backend.health_check()?;
        let capabilities = backend.capabilities();
        if !capabilities.reads_physical_state
            || !capabilities.sends_keyboard_events
            || !capabilities.supports_hold_repeat
            || !capabilities.supports_combo_sequence
        {
            return Err(anyhow::anyhow!(
                "input backend '{}' does not support the required autofire capabilities",
                input_backend_label(input_backend)
            ));
        }
        let stop_requested = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));

        let stop_for_thread = Arc::clone(&stop_requested);
        let running_for_thread = Arc::clone(&running);
        let join = thread::spawn(move || {
            let result = run_loop(runtime, backend, stop_for_thread, on_event);
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

#[allow(dead_code)]
pub fn run(profile: &Profile, target_windows: &[String]) -> Result<()> {
    let settings = BackendSettings::default();
    run_with_backend(
        profile,
        target_windows,
        InputBackendKind::default(),
        &settings,
    )
}

pub fn run_with_backend(
    profile: &Profile,
    target_windows: &[String],
    input_backend: InputBackendKind,
    backend_settings: &BackendSettings,
) -> Result<()> {
    let runtime =
        RuntimeProfile::from_profile_with_backend(profile, target_windows, input_backend)?;
    print_start_summary(&runtime);

    let mut handle = AutoFireService::start_with_backend_events(
        profile.clone(),
        target_windows.to_vec(),
        input_backend,
        backend_settings.clone(),
        |event| match event {
            RunnerEvent::Started => {}
            RunnerEvent::PausedByIme => println!("检测到输入法开启，暂停连发。"),
            RunnerEvent::ResumedFromIme => println!("输入法关闭，恢复连发。"),
            RunnerEvent::Stopped(StopReason::StopRequested) => println!("连发已停止。"),
        },
    )?;

    handle.wait()
}

#[derive(Clone)]
struct RuntimeProfile {
    keys: Vec<KeySpec>,
    combos: Vec<RuntimeCombo>,
    custom_autofires: Vec<RuntimeCustomAutofire>,
    auto_triggers: Vec<RuntimeAutoTrigger>,
    linked_keys: Vec<RuntimeLinkedKey>,
    monitored_vks: Vec<u16>,
    repeat_interval_ms: u64,
    press_duration_ms: u64,
    target_windows: Vec<String>,
    input_backend: InputBackendKind,
    sleep_timing: SleepTimingMonitor,
}

impl RuntimeProfile {
    #[cfg(test)]
    fn from_profile(profile: &Profile, target_windows: &[String]) -> Result<Self> {
        Self::from_profile_with_backend(profile, target_windows, InputBackendKind::default())
    }

    fn from_profile_with_backend(
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
    interval_ms: u64,
    press_duration_ms: u64,
}

#[derive(Clone)]
struct RuntimeCustomAutofire {
    name: String,
    key: KeySpec,
    repeat_interval_ms: u64,
    press_duration_ms: u64,
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
    repeat_interval_ms: u64,
    press_duration_ms: u64,
}

#[derive(Clone)]
struct RuntimeLinkedKey {
    name: String,
    trigger_key: KeySpec,
    linked_key: KeySpec,
    trigger_mode: LinkedTriggerMode,
    interval_ms: u64,
    press_duration_ms: u64,
}

#[derive(Clone)]
struct RuntimeRepeatBinding {
    key: KeySpec,
    trigger: RepeatTrigger,
    repeat_interval: Duration,
    press_duration: Duration,
}

#[derive(Clone)]
enum RepeatTrigger {
    HoldKey(u16),
    ToggleHotkey(RuntimeHotkey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepeatPhase {
    Idle,
    Queued,
    Recovering { until: Instant },
}

struct RepeatBindingState {
    trigger_down: bool,
    enabled: bool,
    phase: RepeatPhase,
}

impl Default for RepeatBindingState {
    fn default() -> Self {
        Self {
            trigger_down: false,
            enabled: false,
            phase: RepeatPhase::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComboPhase {
    Idle,
    Ready {
        step_index: usize,
        ready_at: Instant,
    },
    Queued {
        step_index: usize,
    },
    Recovering {
        next_index: usize,
        until: Instant,
    },
}

struct ComboState {
    trigger_down: bool,
    phase: ComboPhase,
}

impl Default for ComboState {
    fn default() -> Self {
        Self {
            trigger_down: false,
            phase: ComboPhase::Idle,
        }
    }
}

#[derive(Default)]
struct LinkedBindingState {
    trigger_down: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandSource {
    Repeat(usize),
    Combo(usize),
    Linked(usize),
}

#[derive(Clone, Copy)]
struct QueuedCommand {
    source: CommandSource,
    key: KeySpec,
    ready_at: Instant,
    press_duration: Duration,
}

#[derive(Default)]
struct CommandQueue {
    pending: Vec<QueuedCommand>,
}

impl CommandQueue {
    fn clear(&mut self) {
        self.pending.clear();
    }

    fn enqueue(&mut self, command: QueuedCommand) {
        self.pending.push(command);
        self.pending.sort_by_key(|item| item.ready_at);
    }

    fn cancel_source(&mut self, source: CommandSource) {
        self.pending.retain(|item| item.source != source);
    }

    fn next_ready_at(&self) -> Option<Instant> {
        self.pending.iter().map(|item| item.ready_at).min()
    }

    fn pop_next_ready(&mut self, now: Instant) -> Option<QueuedCommand> {
        let next_index = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, item)| item.ready_at <= now)
            .min_by_key(|(_, item)| item.ready_at)
            .map(|(index, _)| index)?;
        Some(self.pending.remove(next_index))
    }
}

impl RuntimeProfile {
    fn sleep_timing_snapshot(&self) -> SleepTimingSnapshot {
        self.sleep_timing.snapshot()
    }
}

fn effective_poll_duration(timing: SleepTimingSnapshot) -> Duration {
    Duration::from_millis(timing.scheduler_interval_ms.max(1))
}

fn configured_interval_duration(base_ms: u64) -> Duration {
    Duration::from_millis(base_ms.max(1))
}

fn configured_press_duration(base_ms: u64) -> Duration {
    Duration::from_millis(base_ms.max(1))
}

fn next_combo_step_ready_at(sent_completed_at: Instant, step: &RuntimeComboStep) -> Instant {
    sent_completed_at + configured_interval_duration(step.interval_ms)
}

fn collect_monitored_vks(
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

fn print_start_summary(runtime: &RuntimeProfile) {
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

fn run_loop<F>(
    runtime: RuntimeProfile,
    mut input_backend: Box<dyn InputBackend>,
    stop_requested: Arc<AtomicBool>,
    on_event: F,
) -> Result<()>
where
    F: Fn(RunnerEvent),
{
    on_event(RunnerEvent::Started);
    let sleeper = HighPrecisionSleeper::new();
    let repeat_bindings = build_repeat_bindings(&runtime);
    let mut repeat_states = repeat_bindings
        .iter()
        .map(|_| RepeatBindingState::default())
        .collect::<Vec<_>>();
    let mut combo_states = runtime
        .combos
        .iter()
        .map(|_| ComboState::default())
        .collect::<Vec<_>>();
    let mut linked_states = runtime
        .linked_keys
        .iter()
        .map(|_| LinkedBindingState::default())
        .collect::<Vec<_>>();
    let mut command_queue = CommandQueue::default();
    let mut is_active = false;
    let mut ime_blocking = false;

    let stop_reason = loop {
        if stop_requested.load(Ordering::SeqCst) {
            break StopReason::StopRequested;
        }
        let sleep_timing = runtime.sleep_timing_snapshot();
        let input_snapshot = input_backend.snapshot(&runtime.monitored_vks);

        let foreground_window = foreground_window_info();
        let active_now = foreground_window
            .as_ref()
            .is_some_and(|info| info.matches_any_target(&runtime.target_windows));

        if active_now != is_active {
            is_active = active_now;
            if !is_active {
                reset_runtime_states_for_pause(
                    &repeat_bindings,
                    &mut repeat_states,
                    &mut combo_states,
                    &mut command_queue,
                );
                if ime_blocking {
                    ime_blocking = false;
                    on_event(RunnerEvent::ResumedFromIme);
                }
            }
        }

        if is_active {
            let ime_open = foreground_window
                .as_ref()
                .map(|info| window_ime_open(info.hwnd))
                .unwrap_or(false);
            if ime_open != ime_blocking {
                ime_blocking = ime_open;
                if ime_blocking {
                    reset_runtime_states_for_pause(
                        &repeat_bindings,
                        &mut repeat_states,
                        &mut combo_states,
                        &mut command_queue,
                    );
                    on_event(RunnerEvent::PausedByIme);
                } else {
                    on_event(RunnerEvent::ResumedFromIme);
                }
            }

            if ime_blocking {
                sync_repeat_binding_inputs(&repeat_bindings, &input_snapshot, &mut repeat_states);
                sync_combo_inputs(&runtime.combos, &input_snapshot, &mut combo_states);
                sync_linked_inputs(&runtime.linked_keys, &input_snapshot, &mut linked_states);
                sleeper.sleep_for(effective_poll_duration(sleep_timing));
                continue;
            }

            let now = Instant::now();
            drive_combo_state_machines(
                &runtime.combos,
                &input_snapshot,
                &mut combo_states,
                &mut command_queue,
                now,
            );
            drive_repeat_state_machines(
                &repeat_bindings,
                &input_snapshot,
                &mut repeat_states,
                &mut command_queue,
                now,
            );
            drive_linked_bindings(
                &runtime.linked_keys,
                &input_snapshot,
                &mut linked_states,
                &mut command_queue,
                now,
            );

            if let Some(command) = command_queue.pop_next_ready(now) {
                execute_ready_command(
                    command,
                    input_backend.as_mut(),
                    &sleeper,
                    &repeat_bindings,
                    &mut repeat_states,
                    &runtime.combos,
                    &mut combo_states,
                );
                continue;
            }
        } else {
            reset_runtime_states_for_pause(
                &repeat_bindings,
                &mut repeat_states,
                &mut combo_states,
                &mut command_queue,
            );
            sync_repeat_binding_inputs(&repeat_bindings, &input_snapshot, &mut repeat_states);
            sync_combo_inputs(&runtime.combos, &input_snapshot, &mut combo_states);
            sync_linked_inputs(&runtime.linked_keys, &input_snapshot, &mut linked_states);
        }

        let now = Instant::now();
        let next_poll_at = now + effective_poll_duration(sleep_timing);
        let next_deadline = next_runtime_deadline(&repeat_states, &combo_states, &command_queue)
            .map(|deadline| deadline.min(next_poll_at))
            .unwrap_or(next_poll_at);
        sleeper.sleep_until(next_deadline);
    };

    on_event(RunnerEvent::Stopped(stop_reason));
    Ok(())
}

fn execute_ready_command(
    command: QueuedCommand,
    input_backend: &mut dyn InputBackend,
    sleeper: &HighPrecisionSleeper,
    repeat_bindings: &[RuntimeRepeatBinding],
    repeat_states: &mut [RepeatBindingState],
    combos: &[RuntimeCombo],
    combo_states: &mut [ComboState],
) {
    input_backend.send_key_once(command.key, command.press_duration, sleeper);
    on_command_executed(
        command,
        Instant::now(),
        repeat_bindings,
        repeat_states,
        combos,
        combo_states,
    );
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

fn build_repeat_bindings(runtime: &RuntimeProfile) -> Vec<RuntimeRepeatBinding> {
    let combo_trigger_vks = runtime
        .combos
        .iter()
        .map(|combo| combo.trigger.vk)
        .collect::<HashSet<_>>();
    let custom_autofire_vks = runtime
        .custom_autofires
        .iter()
        .map(|entry| entry.key.vk)
        .collect::<HashSet<_>>();

    let mut bindings = Vec::new();
    bindings.extend(
        runtime
            .keys
            .iter()
            .filter(|key| {
                !combo_trigger_vks.contains(&key.vk) && !custom_autofire_vks.contains(&key.vk)
            })
            .map(|key| RuntimeRepeatBinding {
                key: *key,
                trigger: RepeatTrigger::HoldKey(key.vk),
                repeat_interval: configured_interval_duration(runtime.repeat_interval_ms),
                press_duration: configured_press_duration(runtime.press_duration_ms),
            }),
    );
    bindings.extend(
        runtime
            .custom_autofires
            .iter()
            .map(|entry| RuntimeRepeatBinding {
                key: entry.key,
                trigger: RepeatTrigger::HoldKey(entry.key.vk),
                repeat_interval: configured_interval_duration(entry.repeat_interval_ms),
                press_duration: configured_press_duration(entry.press_duration_ms),
            }),
    );
    bindings.extend(
        runtime
            .auto_triggers
            .iter()
            .map(|entry| RuntimeRepeatBinding {
                key: entry.key,
                trigger: RepeatTrigger::ToggleHotkey(entry.trigger_hotkey.clone()),
                repeat_interval: configured_interval_duration(entry.repeat_interval_ms),
                press_duration: configured_press_duration(entry.press_duration_ms),
            }),
    );

    bindings
}

fn drive_repeat_state_machines(
    repeat_bindings: &[RuntimeRepeatBinding],
    input_snapshot: &InputSnapshot,
    repeat_states: &mut [RepeatBindingState],
    command_queue: &mut CommandQueue,
    now: Instant,
) {
    for (index, binding) in repeat_bindings.iter().enumerate() {
        let state = &mut repeat_states[index];
        let source = CommandSource::Repeat(index);

        match &binding.trigger {
            RepeatTrigger::HoldKey(vk) => {
                let is_down = input_snapshot.is_down(*vk);
                state.trigger_down = is_down;
                state.enabled = is_down;
                if !state.enabled {
                    command_queue.cancel_source(source);
                    state.phase = RepeatPhase::Idle;
                    continue;
                }
            }
            RepeatTrigger::ToggleHotkey(hotkey) => {
                let is_down = hotkey_is_down(hotkey, input_snapshot);
                if is_down && !state.trigger_down {
                    state.enabled = !state.enabled;
                    if !state.enabled {
                        command_queue.cancel_source(source);
                        state.phase = RepeatPhase::Idle;
                    }
                }
                state.trigger_down = is_down;
                if !state.enabled {
                    continue;
                }
            }
        }

        match state.phase {
            RepeatPhase::Idle => queue_repeat_command(binding, state, command_queue, source, now),
            RepeatPhase::Recovering { until } if now >= until => {
                queue_repeat_command(binding, state, command_queue, source, now)
            }
            RepeatPhase::Queued | RepeatPhase::Recovering { .. } => {}
        }
    }
}

fn queue_repeat_command(
    binding: &RuntimeRepeatBinding,
    state: &mut RepeatBindingState,
    command_queue: &mut CommandQueue,
    source: CommandSource,
    ready_at: Instant,
) {
    command_queue.cancel_source(source);
    command_queue.enqueue(QueuedCommand {
        source,
        key: binding.key,
        ready_at,
        press_duration: binding.press_duration,
    });
    state.phase = RepeatPhase::Queued;
}

fn sync_repeat_binding_inputs(
    repeat_bindings: &[RuntimeRepeatBinding],
    input_snapshot: &InputSnapshot,
    repeat_states: &mut [RepeatBindingState],
) {
    for (binding, state) in repeat_bindings.iter().zip(repeat_states.iter_mut()) {
        match &binding.trigger {
            RepeatTrigger::HoldKey(vk) => {
                state.trigger_down = input_snapshot.is_down(*vk);
                state.enabled = false;
                state.phase = RepeatPhase::Idle;
            }
            RepeatTrigger::ToggleHotkey(hotkey) => {
                state.trigger_down = hotkey_is_down(hotkey, input_snapshot);
                state.phase = RepeatPhase::Idle;
            }
        }
    }
}

fn drive_combo_state_machines(
    combos: &[RuntimeCombo],
    input_snapshot: &InputSnapshot,
    combo_states: &mut [ComboState],
    command_queue: &mut CommandQueue,
    now: Instant,
) {
    for (index, combo) in combos.iter().enumerate() {
        let state = &mut combo_states[index];
        let is_down = input_snapshot.is_down(combo.trigger.vk);
        if is_down && !state.trigger_down {
            command_queue.cancel_source(CommandSource::Combo(index));
            state.phase = ComboPhase::Ready {
                step_index: 0,
                ready_at: now,
            };
        }
        state.trigger_down = is_down;
        advance_combo_state(
            combo,
            state,
            command_queue,
            CommandSource::Combo(index),
            now,
        );
    }
}

fn advance_combo_state(
    combo: &RuntimeCombo,
    state: &mut ComboState,
    command_queue: &mut CommandQueue,
    source: CommandSource,
    now: Instant,
) {
    loop {
        match state.phase {
            ComboPhase::Idle | ComboPhase::Queued { .. } => break,
            ComboPhase::Recovering { next_index, until } => {
                if now < until {
                    break;
                }
                if next_index >= combo.steps.len() {
                    state.phase = ComboPhase::Idle;
                    break;
                }
                state.phase = ComboPhase::Ready {
                    step_index: next_index,
                    ready_at: now,
                };
            }
            ComboPhase::Ready {
                step_index,
                ready_at,
            } => {
                if now < ready_at || step_index >= combo.steps.len() {
                    break;
                }
                let step = &combo.steps[step_index];
                command_queue.cancel_source(source);
                command_queue.enqueue(QueuedCommand {
                    source,
                    key: step.key,
                    ready_at,
                    press_duration: configured_press_duration(step.press_duration_ms),
                });
                state.phase = ComboPhase::Queued { step_index };
                break;
            }
        }
    }
}

fn sync_combo_inputs(
    combos: &[RuntimeCombo],
    input_snapshot: &InputSnapshot,
    combo_states: &mut [ComboState],
) {
    for (combo, state) in combos.iter().zip(combo_states.iter_mut()) {
        state.trigger_down = input_snapshot.is_down(combo.trigger.vk);
        state.phase = ComboPhase::Idle;
    }
}

fn drive_linked_bindings(
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

fn sync_linked_inputs(
    linked_keys: &[RuntimeLinkedKey],
    input_snapshot: &InputSnapshot,
    linked_states: &mut [LinkedBindingState],
) {
    for (linked, state) in linked_keys.iter().zip(linked_states.iter_mut()) {
        state.trigger_down = input_snapshot.is_down(linked.trigger_key.vk);
    }
}

fn on_command_executed(
    command: QueuedCommand,
    completed_at: Instant,
    repeat_bindings: &[RuntimeRepeatBinding],
    repeat_states: &mut [RepeatBindingState],
    combos: &[RuntimeCombo],
    combo_states: &mut [ComboState],
) {
    match command.source {
        CommandSource::Repeat(index) => {
            let Some(state) = repeat_states.get_mut(index) else {
                return;
            };
            let Some(binding) = repeat_bindings.get(index) else {
                return;
            };
            state.phase = RepeatPhase::Recovering {
                until: completed_at + binding.repeat_interval,
            };
        }
        CommandSource::Combo(index) => {
            let Some(state) = combo_states.get_mut(index) else {
                return;
            };
            let Some(combo) = combos.get(index) else {
                return;
            };
            let ComboPhase::Queued { step_index } = state.phase else {
                return;
            };
            if step_index + 1 >= combo.steps.len() {
                state.phase = ComboPhase::Idle;
            } else {
                let step = &combo.steps[step_index];
                state.phase = ComboPhase::Recovering {
                    next_index: step_index + 1,
                    until: next_combo_step_ready_at(completed_at, step),
                };
            }
        }
        CommandSource::Linked(_) => {}
    }
}

fn reset_runtime_states_for_pause(
    repeat_bindings: &[RuntimeRepeatBinding],
    repeat_states: &mut [RepeatBindingState],
    combo_states: &mut [ComboState],
    command_queue: &mut CommandQueue,
) {
    command_queue.clear();

    for (binding, state) in repeat_bindings.iter().zip(repeat_states.iter_mut()) {
        if matches!(binding.trigger, RepeatTrigger::HoldKey(_)) {
            state.enabled = false;
        }
        state.phase = RepeatPhase::Idle;
    }

    for state in combo_states {
        state.phase = ComboPhase::Idle;
    }
}

fn next_runtime_deadline(
    repeat_states: &[RepeatBindingState],
    combo_states: &[ComboState],
    command_queue: &CommandQueue,
) -> Option<Instant> {
    let mut next_deadline = command_queue.next_ready_at();

    for state in repeat_states {
        if let RepeatPhase::Recovering { until } = state.phase {
            merge_earlier_deadline(&mut next_deadline, until);
        }
    }

    for state in combo_states {
        match state.phase {
            ComboPhase::Ready { ready_at, .. } => {
                merge_earlier_deadline(&mut next_deadline, ready_at)
            }
            ComboPhase::Recovering { until, .. } => {
                merge_earlier_deadline(&mut next_deadline, until)
            }
            ComboPhase::Idle | ComboPhase::Queued { .. } => {}
        }
    }

    next_deadline
}

fn merge_earlier_deadline(next_deadline: &mut Option<Instant>, candidate: Instant) {
    match next_deadline {
        Some(current) if candidate >= *current => {}
        _ => *next_deadline = Some(candidate),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn hotkey_is_down(hotkey: &RuntimeHotkey, input_snapshot: &InputSnapshot) -> bool {
    hotkey
        .specs
        .iter()
        .all(|spec| input_snapshot.is_down(spec.vk))
}

fn linked_trigger_mode_label(mode: LinkedTriggerMode) -> &'static str {
    match mode {
        LinkedTriggerMode::Press => "[按下]",
        LinkedTriggerMode::Release => "[松开]",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AutoFireService, ComboPhase, ComboState, CommandQueue, CommandSource, QueuedCommand,
        RepeatBindingState, RepeatPhase, RunnerEvent, RuntimeComboStep, RuntimeRepeatBinding,
        StopReason, collect_monitored_vks, execute_ready_command, next_combo_step_ready_at,
        on_command_executed,
    };
    use crate::config::{
        ComboConfig, ComboStepConfig, LinkedTriggerMode, Profile, SpecialKeyConfig,
    };
    use crate::input::backend::{
        InputBackend, InputBackendCapabilities, InputSnapshot, resolve_effective_key_down,
    };
    use crate::keymap::{KeySpec, parse_single_key};
    use crate::timing::HighPrecisionSleeper;
    use anyhow::Result;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct FakeInputBackend {
        sent: Vec<(KeySpec, Duration)>,
    }

    impl InputBackend for FakeInputBackend {
        fn snapshot(&mut self, _monitored_vks: &[u16]) -> InputSnapshot {
            InputSnapshot::default()
        }

        fn send_key_once(
            &mut self,
            key: KeySpec,
            press_duration: Duration,
            _sleeper: &HighPrecisionSleeper,
        ) {
            self.sent.push((key, press_duration));
        }

        fn capabilities(&self) -> InputBackendCapabilities {
            InputBackendCapabilities {
                reads_physical_state: true,
                sends_keyboard_events: true,
                supports_hold_repeat: true,
                supports_combo_sequence: true,
            }
        }

        fn health_check(&self) -> Result<()> {
            Ok(())
        }
    }

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
}
