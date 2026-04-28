// Defines special-key configuration variants and validation.

use super::{
    DEFAULT_COMBO_STEP_INTERVAL_MS, DEFAULT_COMBO_STEP_PRESS_DURATION_MS,
    DEFAULT_PRESS_DURATION_MS, DEFAULT_REPEAT_INTERVAL_MS, normalize_ms_or_default,
};
use crate::keymap::normalize_hotkey_text;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecialKeyConfig {
    CustomAutofire {
        name: String,
        key: String,
        repeat_interval_ms: u64,
        press_duration_ms: u64,
    },
    AutoTrigger {
        name: String,
        key: String,
        trigger_hotkey: String,
        repeat_interval_ms: u64,
        press_duration_ms: u64,
    },
    LinkedKey {
        name: String,
        trigger_key: String,
        linked_key: String,
        #[serde(default)]
        trigger_mode: LinkedTriggerMode,
        interval_ms: u64,
        press_duration_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LinkedTriggerMode {
    #[default]
    Press,
    Release,
}

impl SpecialKeyConfig {
    pub fn normalized(self) -> Self {
        match self {
            Self::CustomAutofire {
                name,
                key,
                repeat_interval_ms,
                press_duration_ms,
            } => Self::CustomAutofire {
                name: name.trim().to_string(),
                key: key.trim().to_ascii_uppercase(),
                repeat_interval_ms: normalize_ms_or_default(
                    repeat_interval_ms,
                    DEFAULT_REPEAT_INTERVAL_MS,
                ),
                press_duration_ms: normalize_ms_or_default(
                    press_duration_ms,
                    DEFAULT_PRESS_DURATION_MS,
                ),
            },
            Self::AutoTrigger {
                name,
                key,
                trigger_hotkey,
                repeat_interval_ms,
                press_duration_ms,
            } => Self::AutoTrigger {
                name: name.trim().to_string(),
                key: key.trim().to_ascii_uppercase(),
                trigger_hotkey: normalize_special_hotkey_text(&trigger_hotkey),
                repeat_interval_ms: normalize_ms_or_default(
                    repeat_interval_ms,
                    DEFAULT_REPEAT_INTERVAL_MS,
                ),
                press_duration_ms: normalize_ms_or_default(
                    press_duration_ms,
                    DEFAULT_PRESS_DURATION_MS,
                ),
            },
            Self::LinkedKey {
                name,
                trigger_key,
                linked_key,
                trigger_mode,
                interval_ms,
                press_duration_ms,
            } => Self::LinkedKey {
                name: name.trim().to_string(),
                trigger_key: trigger_key.trim().to_ascii_uppercase(),
                linked_key: linked_key.trim().to_ascii_uppercase(),
                trigger_mode,
                interval_ms: normalize_ms_or_default(interval_ms, DEFAULT_COMBO_STEP_INTERVAL_MS),
                press_duration_ms: normalize_ms_or_default(
                    press_duration_ms,
                    DEFAULT_COMBO_STEP_PRESS_DURATION_MS,
                ),
            },
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::CustomAutofire {
                name,
                key,
                repeat_interval_ms,
                press_duration_ms,
            } => {
                ensure_special_name(name)?;
                if key.trim().is_empty() {
                    bail!("custom autofire key cannot be empty");
                }
                if *repeat_interval_ms == 0 {
                    bail!("custom autofire repeat_interval_ms must be >= 1");
                }
                if *press_duration_ms == 0 {
                    bail!("custom autofire press_duration_ms must be >= 1");
                }
            }
            Self::AutoTrigger {
                name,
                key,
                trigger_hotkey,
                repeat_interval_ms,
                press_duration_ms,
            } => {
                ensure_special_name(name)?;
                if key.trim().is_empty() {
                    bail!("auto trigger key cannot be empty");
                }
                if trigger_hotkey.trim().is_empty() {
                    bail!("auto trigger trigger_hotkey cannot be empty");
                }
                if *repeat_interval_ms == 0 {
                    bail!("auto trigger repeat_interval_ms must be >= 1");
                }
                if *press_duration_ms == 0 {
                    bail!("auto trigger press_duration_ms must be >= 1");
                }
            }
            Self::LinkedKey {
                name,
                trigger_key,
                linked_key,
                trigger_mode: _,
                interval_ms,
                press_duration_ms,
            } => {
                ensure_special_name(name)?;
                if trigger_key.trim().is_empty() {
                    bail!("linked key trigger_key cannot be empty");
                }
                if linked_key.trim().is_empty() {
                    bail!("linked key linked_key cannot be empty");
                }
                if *interval_ms == 0 {
                    bail!("linked key interval_ms must be >= 1");
                }
                if *press_duration_ms == 0 {
                    bail!("linked key press_duration_ms must be >= 1");
                }
            }
        }

        Ok(())
    }

    pub fn name(&self) -> &str {
        match self {
            Self::CustomAutofire { name, .. }
            | Self::AutoTrigger { name, .. }
            | Self::LinkedKey { name, .. } => name,
        }
    }
}

fn ensure_special_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("special key config name cannot be empty");
    }
    Ok(())
}

fn normalize_special_hotkey_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    normalize_hotkey_text(trimmed).unwrap_or_else(|_| trimmed.to_ascii_uppercase())
}
