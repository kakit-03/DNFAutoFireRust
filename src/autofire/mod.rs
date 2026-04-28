// Coordinates the autofire worker thread and public runner controls.

use crate::config::Profile;
use crate::input::backend::{
    BackendSettings, InputBackend, InputBackendContext, InputBackendKind, create_input_backend,
    input_backend_label,
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
        input_backend.update_context(InputBackendContext {
            target_hwnd: foreground_window
                .as_ref()
                .filter(|_| active_now)
                .map(|info| info.hwnd.0 as isize),
        });

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
mod tests;
