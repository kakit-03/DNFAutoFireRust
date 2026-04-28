// Defines core configuration models and shared validation helpers.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

mod initial;
mod special_key;
mod store;

pub use special_key::{LinkedTriggerMode, SpecialKeyConfig};
pub use store::ConfigStore;

const DEFAULT_PROFILE_NAME: &str = "default";
const DEFAULT_KEYS: [&str; 1] = ["J"];
const DEFAULT_TARGET_WINDOWS: [&str; 2] = ["地下城与勇士", "DNF"];
const DEFAULT_QUICK_SWITCH_HOTKEY: &str = "LCTRL+BACKQUOTE";
pub const DEFAULT_REPEAT_INTERVAL_MS: u64 = 10;
pub const DEFAULT_PRESS_DURATION_MS: u64 = 15;
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 1;
pub const DEFAULT_COMBO_STEP_INTERVAL_MS: u64 = 8;
pub const DEFAULT_COMBO_STEP_PRESS_DURATION_MS: u64 = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboStepConfig {
    pub key: String,
    pub interval_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub press_duration_ms: u64,
}

impl ComboStepConfig {
    pub fn normalized_with_fallback(mut self, fallback_press_duration_ms: u64) -> Self {
        self.key = self.key.trim().to_ascii_uppercase();
        if self.interval_ms == 0 {
            self.interval_ms = DEFAULT_COMBO_STEP_INTERVAL_MS;
        }
        if self.press_duration_ms == 0 {
            self.press_duration_ms = normalize_ms_or_default(
                fallback_press_duration_ms,
                DEFAULT_COMBO_STEP_PRESS_DURATION_MS,
            );
        }
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.key.is_empty() {
            bail!("combo step key cannot be empty");
        }
        if self.interval_ms == 0 {
            bail!("combo step interval_ms must be >= 1");
        }
        if self.press_duration_ms == 0 {
            bail!("combo step press_duration_ms must be >= 1");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboConfig {
    pub name: String,
    pub trigger_key: String,
    #[serde(default)]
    pub steps: Vec<ComboStepConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub step_interval_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub press_duration_ms: u64,
}

impl ComboConfig {
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.trigger_key = self.trigger_key.trim().to_ascii_uppercase();
        let fallback_interval =
            normalize_ms_or_default(self.step_interval_ms, DEFAULT_COMBO_STEP_INTERVAL_MS);
        let fallback_press_duration =
            normalize_ms_or_default(self.press_duration_ms, DEFAULT_COMBO_STEP_PRESS_DURATION_MS);

        if self.steps.is_empty() {
            self.steps = normalize_key_sequence(&self.sequence_keys)
                .into_iter()
                .map(|key| ComboStepConfig {
                    key,
                    interval_ms: fallback_interval,
                    press_duration_ms: fallback_press_duration,
                })
                .collect();
        } else {
            self.steps = self
                .steps
                .into_iter()
                .map(|step| step.normalized_with_fallback(fallback_press_duration))
                .collect();
        }

        self.sequence_keys.clear();
        self.step_interval_ms = 0;
        self.press_duration_ms = 0;

        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            bail!("combo name cannot be empty");
        }
        if self.trigger_key.is_empty() {
            bail!("combo trigger_key cannot be empty");
        }
        if self.steps.is_empty() {
            bail!("combo steps cannot be empty");
        }
        for step in &self.steps {
            step.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub enabled_keys: Vec<String>,
    pub repeat_interval_ms: u64,
    pub press_duration_ms: u64,
    pub poll_interval_ms: u64,
    #[serde(default)]
    pub combos: Vec<ComboConfig>,
    #[serde(default)]
    pub special_keys: Vec<SpecialKeyConfig>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            enabled_keys: DEFAULT_KEYS.iter().map(|s| (*s).to_string()).collect(),
            repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS,
            press_duration_ms: DEFAULT_PRESS_DURATION_MS,
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            combos: Vec::new(),
            special_keys: Vec::new(),
        }
    }
}

impl Profile {
    pub fn normalized(mut self) -> Self {
        self.enabled_keys = normalize_tokens(&self.enabled_keys, true);
        self.combos = self
            .combos
            .into_iter()
            .map(ComboConfig::normalized)
            .collect();
        self.special_keys = self
            .special_keys
            .into_iter()
            .map(SpecialKeyConfig::normalized)
            .collect();

        if self.enabled_keys.is_empty() && self.combos.is_empty() && self.special_keys.is_empty() {
            self.enabled_keys = DEFAULT_KEYS.iter().map(|s| (*s).to_string()).collect();
        }

        if self.repeat_interval_ms == 0 {
            self.repeat_interval_ms = DEFAULT_REPEAT_INTERVAL_MS;
        }
        if self.press_duration_ms == 0 {
            self.press_duration_ms = DEFAULT_PRESS_DURATION_MS;
        }
        if self.poll_interval_ms == 0 {
            self.poll_interval_ms = DEFAULT_POLL_INTERVAL_MS;
        }

        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.enabled_keys.is_empty() && self.combos.is_empty() && self.special_keys.is_empty() {
            bail!("enabled_keys, combos, and special_keys cannot all be empty");
        }
        if self.repeat_interval_ms == 0 {
            bail!("repeat_interval_ms must be >= 1");
        }
        if self.press_duration_ms == 0 {
            bail!("press_duration_ms must be >= 1");
        }
        if self.poll_interval_ms == 0 {
            bail!("poll_interval_ms must be >= 1");
        }
        ensure_unique_combo_names(&self.combos)?;
        ensure_unique_special_key_names(&self.special_keys)?;
        for combo in &self.combos {
            combo.validate()?;
        }
        for special in &self.special_keys {
            special.validate()?;
        }

        Ok(())
    }
}

fn normalize_tokens(tokens: &[String], uppercase: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for token in tokens {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }

        let normalized = if uppercase {
            trimmed.to_ascii_uppercase()
        } else {
            trimmed.to_string()
        };

        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }

    out
}

fn ensure_unique_combo_names(combos: &[ComboConfig]) -> Result<()> {
    let mut seen = HashSet::new();
    for combo in combos {
        if !seen.insert(combo.name.to_ascii_lowercase()) {
            bail!("duplicate combo name: {}", combo.name);
        }
    }
    Ok(())
}

fn ensure_unique_special_key_names(special_keys: &[SpecialKeyConfig]) -> Result<()> {
    let mut seen = HashSet::new();
    for special in special_keys {
        if !seen.insert(special.name().to_ascii_lowercase()) {
            bail!("duplicate special key config name: {}", special.name());
        }
    }
    Ok(())
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn normalize_ms_or_default(value: u64, default: u64) -> u64 {
    if value == 0 { default } else { value.max(1) }
}

fn normalize_key_sequence(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|token| {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_ascii_uppercase())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
