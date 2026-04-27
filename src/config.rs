use crate::initial_config::INITIAL_CONFIG_JSON;
use crate::keymap::normalize_hotkey_text;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

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
        let fallback_press_duration = normalize_ms_or_default(
            self.press_duration_ms,
            DEFAULT_COMBO_STEP_PRESS_DURATION_MS,
        );

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
                interval_ms: normalize_ms_or_default(
                    interval_ms,
                    DEFAULT_COMBO_STEP_INTERVAL_MS,
                ),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigStore {
    pub default_profile: String,
    #[serde(default = "default_quick_switch_hotkey")]
    pub quick_switch_hotkey: String,
    #[serde(default = "default_target_windows")]
    pub target_windows: Vec<String>,
    #[serde(default)]
    pub hide_gui_on_startup: bool,
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert(DEFAULT_PROFILE_NAME.to_string(), Profile::default());
        Self {
            default_profile: DEFAULT_PROFILE_NAME.to_string(),
            quick_switch_hotkey: DEFAULT_QUICK_SWITCH_HOTKEY.to_string(),
            target_windows: default_target_windows(),
            hide_gui_on_startup: false,
            profiles,
        }
    }
}

impl ConfigStore {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let store = Self::initial_template()?;
            store.save(path)?;
            return Ok(store);
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read '{}'", path.display()))?;
        let parsed: Self = serde_json::from_str(&content)
            .with_context(|| format!("invalid json in '{}'", path.display()))?;
        Ok(parsed.normalized())
    }

    fn initial_template() -> Result<Self> {
        let store: Self = serde_json::from_str(INITIAL_CONFIG_JSON)
            .context("invalid built-in initial config template")?;
        Ok(store.normalized())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create '{}'", parent.display()))?;
        }

        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data).with_context(|| format!("failed to write '{}'", path.display()))?;
        Ok(())
    }

    pub fn profile_name_or_default<'a>(&'a self, profile: Option<&'a str>) -> &'a str {
        profile.unwrap_or(self.default_profile.as_str())
    }

    pub fn list_profile_names(&self) -> Vec<&str> {
        self.profiles.keys().map(String::as_str).collect()
    }

    pub fn get_profile(&self, profile: Option<&str>) -> Result<Profile> {
        let name = self.profile_name_or_default(profile);
        self.profiles
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("profile not found: {name}"))
    }

    pub fn upsert_profile(&mut self, name: String, profile: Profile) {
        self.profiles.insert(name, profile.normalized());
    }

    pub fn delete_profile(&mut self, name: &str) -> Result<()> {
        if !self.profiles.contains_key(name) {
            bail!("profile not found: {name}");
        }
        if self.profiles.len() == 1 {
            bail!("cannot delete the last profile");
        }

        self.profiles.remove(name);
        if self.default_profile == name {
            self.default_profile = self
                .profiles
                .keys()
                .next()
                .expect("profiles should not be empty")
                .to_string();
        }
        Ok(())
    }

    pub fn remember_started_profile(&mut self, name: &str) -> Result<()> {
        if !self.profiles.contains_key(name) {
            bail!("profile not found: {name}");
        }
        self.default_profile = name.to_string();
        Ok(())
    }

    fn normalized(mut self) -> Self {
        if self.profiles.is_empty() {
            self.profiles
                .insert(DEFAULT_PROFILE_NAME.to_string(), Profile::default());
        }

        for profile in self.profiles.values_mut() {
            *profile = profile.clone().normalized();
        }
        self.quick_switch_hotkey = normalize_hotkey_text_if_valid(&self.quick_switch_hotkey)
            .unwrap_or_else(default_quick_switch_hotkey);
        self.target_windows = normalize_tokens(&self.target_windows, false);
        if self.target_windows.is_empty() {
            self.target_windows = default_target_windows();
        }

        if !self.profiles.contains_key(&self.default_profile) {
            self.default_profile = self
                .profiles
                .keys()
                .next()
                .expect("profiles should not be empty")
                .to_string();
        }

        self
    }
}

fn default_quick_switch_hotkey() -> String {
    DEFAULT_QUICK_SWITCH_HOTKEY.to_string()
}

fn default_target_windows() -> Vec<String> {
    DEFAULT_TARGET_WINDOWS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

fn normalize_hotkey_text_if_valid(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(default_quick_switch_hotkey());
    }
    normalize_hotkey_text(trimmed).ok()
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

fn ensure_special_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("special key config name cannot be empty");
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

fn normalize_special_hotkey_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    normalize_hotkey_text(trimmed).unwrap_or_else(|_| trimmed.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::{
        ComboConfig, ComboStepConfig, ConfigStore, LinkedTriggerMode, Profile, SpecialKeyConfig,
        DEFAULT_COMBO_STEP_INTERVAL_MS, DEFAULT_COMBO_STEP_PRESS_DURATION_MS,
        DEFAULT_POLL_INTERVAL_MS, DEFAULT_PRESS_DURATION_MS, DEFAULT_REPEAT_INTERVAL_MS,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn profile_normalize_adds_defaults_for_empty_fields() {
        let profile = Profile {
            enabled_keys: Vec::new(),
            repeat_interval_ms: 0,
            press_duration_ms: 0,
            poll_interval_ms: 0,
            combos: Vec::new(),
            special_keys: Vec::new(),
        }
        .normalized();

        assert!(!profile.enabled_keys.is_empty());
        assert_eq!(profile.repeat_interval_ms, DEFAULT_REPEAT_INTERVAL_MS);
        assert_eq!(profile.press_duration_ms, DEFAULT_PRESS_DURATION_MS);
        assert_eq!(profile.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);
    }

    #[test]
    fn combo_normalize_upcases_keys_and_fixes_intervals() {
        let combo = ComboConfig {
            name: " test ".to_string(),
            trigger_key: "j".to_string(),
            steps: vec![
                ComboStepConfig {
                    key: "a".to_string(),
                    interval_ms: 0,
                    press_duration_ms: 0,
                },
                ComboStepConfig {
                    key: "a".to_string(),
                    interval_ms: 5,
                    press_duration_ms: 2,
                },
                ComboStepConfig {
                    key: "b".to_string(),
                    interval_ms: 9,
                    press_duration_ms: 3,
                },
            ],
            sequence_keys: Vec::new(),
            step_interval_ms: 0,
            press_duration_ms: 0,
        }
        .normalized();

        assert_eq!(combo.name, "test");
        assert_eq!(combo.trigger_key, "J");
        assert_eq!(combo.steps.len(), 3);
        assert_eq!(combo.steps[0].key, "A");
        assert_eq!(combo.steps[0].interval_ms, DEFAULT_COMBO_STEP_INTERVAL_MS);
        assert_eq!(
            combo.steps[0].press_duration_ms,
            DEFAULT_COMBO_STEP_PRESS_DURATION_MS
        );
        assert_eq!(combo.steps[1].key, "A");
        assert_eq!(combo.steps[2].key, "B");
        assert_eq!(combo.steps[2].press_duration_ms, 3);
        assert_eq!(combo.press_duration_ms, 0);
    }

    #[test]
    fn combo_normalize_migrates_legacy_sequence_keys() {
        let combo = ComboConfig {
            name: "legacy".to_string(),
            trigger_key: "u".to_string(),
            steps: Vec::new(),
            sequence_keys: vec!["a".to_string(), "s".to_string(), "d".to_string()],
            step_interval_ms: 80,
            press_duration_ms: 1,
        }
        .normalized();

        assert_eq!(combo.steps.len(), 3);
        assert_eq!(combo.steps[0].key, "A");
        assert_eq!(combo.steps[1].key, "S");
        assert_eq!(combo.steps[2].key, "D");
        assert_eq!(combo.steps[0].interval_ms, 80);
        assert_eq!(combo.steps[0].press_duration_ms, 1);
        assert!(combo.sequence_keys.is_empty());
        assert_eq!(combo.step_interval_ms, 0);
        assert_eq!(combo.press_duration_ms, 0);
    }

    #[test]
    fn delete_last_profile_is_rejected() {
        let mut store = ConfigStore::default();
        let err = store
            .delete_profile("default")
            .expect_err("should reject deleting last profile");
        assert!(err.to_string().contains("last profile"));
    }

    #[test]
    fn remember_started_profile_updates_default_profile() {
        let mut store = ConfigStore::default();
        store.upsert_profile(
            "raid".to_string(),
            Profile {
                enabled_keys: vec!["J".to_string()],
                repeat_interval_ms: 1,
                press_duration_ms: 1,
                poll_interval_ms: 1,
                combos: Vec::new(),
                special_keys: Vec::new(),
            },
        );

        store
            .remember_started_profile("raid")
            .expect("remember started profile");

        assert_eq!(store.default_profile, "raid");
    }

    #[test]
    fn duplicate_combo_names_are_rejected() {
        let profile = Profile {
            enabled_keys: vec!["J".to_string()],
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            combos: vec![
                ComboConfig {
                    name: "combo".to_string(),
                    trigger_key: "A".to_string(),
                    steps: vec![ComboStepConfig {
                        key: "B".to_string(),
                        interval_ms: 1,
                        press_duration_ms: 1,
                    }],
                    sequence_keys: Vec::new(),
                    step_interval_ms: 0,
                    press_duration_ms: 1,
                },
                ComboConfig {
                    name: "COMBO".to_string(),
                    trigger_key: "C".to_string(),
                    steps: vec![ComboStepConfig {
                        key: "D".to_string(),
                        interval_ms: 1,
                        press_duration_ms: 1,
                    }],
                    sequence_keys: Vec::new(),
                    step_interval_ms: 0,
                    press_duration_ms: 1,
                },
            ],
            special_keys: Vec::new(),
        };

        assert!(profile.validate().is_err());
    }

    #[test]
    fn store_normalized_migrates_legacy_combo_json() {
        let raw = r#"{
          "default_profile": "default",
          "quick_switch_hotkey": "LALT+Q",
          "target_windows": ["DNF"],
          "profiles": {
            "default": {
              "enabled_keys": ["J"],
              "repeat_interval_ms": 1,
              "press_duration_ms": 1,
              "poll_interval_ms": 1,
              "combos": [
                {
                  "name": "legacy",
                  "trigger_key": "u",
                  "sequence_keys": ["a", "s", "d"],
                  "step_interval_ms": 80,
                  "press_duration_ms": 1
                }
              ],
              "special_keys": []
            }
          }
        }"#;

        let store: ConfigStore = serde_json::from_str(raw).expect("legacy config json");
        let normalized = store.normalized();
        let combo = &normalized.profiles["default"].combos[0];

        assert_eq!(combo.trigger_key, "U");
        assert_eq!(combo.steps.len(), 3);
        assert_eq!(combo.steps[0].key, "A");
        assert_eq!(combo.steps[1].key, "S");
        assert_eq!(combo.steps[2].key, "D");
        assert_eq!(combo.steps[0].interval_ms, 80);
        assert_eq!(combo.steps[0].press_duration_ms, 1);
        assert!(combo.sequence_keys.is_empty());
        assert_eq!(combo.step_interval_ms, 0);
        assert_eq!(normalized.quick_switch_hotkey, "LALT+Q");
        assert_eq!(normalized.target_windows, vec!["DNF"]);
    }

    #[test]
    fn special_key_configs_are_normalized_and_validated() {
        let profile = Profile {
            enabled_keys: Vec::new(),
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            combos: Vec::new(),
            special_keys: vec![
                SpecialKeyConfig::CustomAutofire {
                    name: "  custom ".to_string(),
                    key: "j".to_string(),
                    repeat_interval_ms: 0,
                    press_duration_ms: 0,
                },
                SpecialKeyConfig::AutoTrigger {
                    name: "auto".to_string(),
                    key: "k".to_string(),
                    trigger_hotkey: "lalt+~".to_string(),
                    repeat_interval_ms: 0,
                    press_duration_ms: 0,
                },
                SpecialKeyConfig::LinkedKey {
                    name: "link".to_string(),
                    trigger_key: "a".to_string(),
                    linked_key: "b".to_string(),
                    trigger_mode: LinkedTriggerMode::Release,
                    interval_ms: 0,
                    press_duration_ms: 0,
                },
            ],
        }
        .normalized();

        assert_eq!(profile.special_keys.len(), 3);
        match &profile.special_keys[0] {
            SpecialKeyConfig::CustomAutofire {
                name,
                key,
                repeat_interval_ms,
                press_duration_ms,
            } => {
                assert_eq!(name, "custom");
                assert_eq!(key, "J");
                assert_eq!(*repeat_interval_ms, DEFAULT_REPEAT_INTERVAL_MS);
                assert_eq!(*press_duration_ms, DEFAULT_PRESS_DURATION_MS);
            }
            _ => panic!("expected custom autofire"),
        }
        match &profile.special_keys[1] {
            SpecialKeyConfig::AutoTrigger { trigger_hotkey, .. } => {
                assert_eq!(trigger_hotkey, "LALT+BACKQUOTE");
            }
            _ => panic!("expected auto trigger"),
        }
        match &profile.special_keys[2] {
            SpecialKeyConfig::LinkedKey {
                trigger_mode,
                interval_ms,
                press_duration_ms,
                ..
            } => {
                assert_eq!(*trigger_mode, LinkedTriggerMode::Release);
                assert_eq!(*interval_ms, DEFAULT_COMBO_STEP_INTERVAL_MS);
                assert_eq!(*press_duration_ms, DEFAULT_COMBO_STEP_PRESS_DURATION_MS);
            }
            _ => panic!("expected linked key"),
        }
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn duplicate_special_key_names_are_rejected() {
        let profile = Profile {
            enabled_keys: Vec::new(),
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            combos: Vec::new(),
            special_keys: vec![
                SpecialKeyConfig::LinkedKey {
                    name: "special".to_string(),
                    trigger_key: "A".to_string(),
                    linked_key: "B".to_string(),
                    trigger_mode: LinkedTriggerMode::Press,
                    interval_ms: 1,
                    press_duration_ms: 1,
                },
                SpecialKeyConfig::CustomAutofire {
                    name: "SPECIAL".to_string(),
                    key: "J".to_string(),
                    repeat_interval_ms: 1,
                    press_duration_ms: 1,
                },
            ],
        };

        assert!(profile.validate().is_err());
    }

    #[test]
    fn load_or_create_uses_embedded_initial_config_template() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("dnf-init-template-{unique}.json"));
        if path.exists() {
            std::fs::remove_file(&path).expect("remove stale temp config");
        }

        let store = ConfigStore::load_or_create(&path).expect("load initial template");

        assert_eq!(store.default_profile, "默认配置");
        assert_eq!(store.quick_switch_hotkey, "LALT+BACKQUOTE");
        assert!(!store.hide_gui_on_startup);
        assert!(
            store.profiles.contains_key("默认配置"),
            "expected embedded template profile to exist"
        );
        let profile = &store.profiles["默认配置"];
        assert_eq!(profile.repeat_interval_ms, DEFAULT_REPEAT_INTERVAL_MS);
        assert_eq!(profile.press_duration_ms, DEFAULT_PRESS_DURATION_MS);
        assert_eq!(profile.poll_interval_ms, DEFAULT_POLL_INTERVAL_MS);

        let saved = std::fs::read_to_string(&path).expect("read saved config");
        let saved_store: ConfigStore = serde_json::from_str(&saved).expect("parse saved config");
        assert_eq!(saved_store.default_profile, "默认配置");
        assert!(!saved_store.hide_gui_on_startup);

        std::fs::remove_file(PathBuf::from(&path)).expect("cleanup temp config");
    }
}
