// Provides shared autofire test fixtures and child test modules.

use super::{
    AutoFireService, ComboPhase, ComboState, CommandQueue, CommandSource, QueuedCommand,
    RepeatBindingState, RepeatPhase, RepeatTrigger, RunnerEvent, RuntimeAutoTrigger, RuntimeCombo,
    RuntimeComboStep, RuntimeCustomAutofire, RuntimeHotkey, RuntimeLinkedKey, RuntimeProfile,
    RuntimeRepeatBinding, StopReason, collect_monitored_vks, execute_ready_command,
    next_combo_step_ready_at, on_command_executed,
};
use crate::config::{ComboConfig, ComboStepConfig, LinkedTriggerMode, Profile, SpecialKeyConfig};
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

mod commands;
mod runtime;
