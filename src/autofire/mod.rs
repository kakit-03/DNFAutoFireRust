use crate::config::Profile;
use crate::input::backend::{
    BackendSettings, InputBackend, InputBackendKind, create_input_backend, input_backend_label,
};
use crate::platform::window::{foreground_window_info, window_ime_open};
use crate::timing::HighPrecisionSleeper;
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

mod command_queue;
mod runtime;
mod state_machine;

use command_queue::CommandQueue;
#[cfg(test)]
use command_queue::{CommandSource, QueuedCommand};
#[cfg(test)]
use runtime::{
    RuntimeAutoTrigger, RuntimeCombo, RuntimeComboStep, RuntimeCustomAutofire, RuntimeHotkey,
    RuntimeLinkedKey, collect_monitored_vks,
};
use runtime::{RuntimeProfile, print_start_summary};
#[cfg(test)]
use state_machine::{
    ComboPhase, RepeatPhase, RepeatTrigger, RuntimeRepeatBinding, next_combo_step_ready_at,
    on_command_executed,
};
use state_machine::{
    ComboState, LinkedBindingState, RepeatBindingState, build_repeat_bindings,
    drive_combo_state_machines, drive_linked_bindings, drive_repeat_state_machines,
    effective_poll_duration, execute_ready_command, next_runtime_deadline,
    reset_runtime_states_for_pause, sync_combo_inputs, sync_linked_inputs,
    sync_repeat_binding_inputs,
};

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
        InputBackend, InputBackendCapabilities, InputBackendKind, InputSnapshot,
        resolve_effective_key_down,
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
}
