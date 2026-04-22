use crate::keymap::{normalize_hotkey_text, parse_hotkey};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

const DEFAULT_PROFILE_NAME: &str = "default";
const DEFAULT_KEYS: [&str; 4] = ["J", "P", "L", "H"];
const DEFAULT_TARGET_WINDOWS: [&str; 2] = ["地下城与勇士", "DNF"];
const DEFAULT_QUICK_SWITCH_HOTKEY: &str = "LCTRL+BACKQUOTE";

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
            self.interval_ms = 1;
        }
        if self.press_duration_ms == 0 {
            self.press_duration_ms = fallback_press_duration_ms.max(1);
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
        let fallback_interval = self.step_interval_ms.max(1);
        let fallback_press_duration = self.press_duration_ms.max(1);

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
    pub target_windows: Vec<String>,
    #[serde(default = "default_quick_switch_hotkey")]
    pub quick_switch_hotkey: String,
    #[serde(default)]
    pub combos: Vec<ComboConfig>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            enabled_keys: DEFAULT_KEYS.iter().map(|s| (*s).to_string()).collect(),
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            target_windows: DEFAULT_TARGET_WINDOWS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            quick_switch_hotkey: DEFAULT_QUICK_SWITCH_HOTKEY.to_string(),
            combos: Vec::new(),
        }
    }
}

impl Profile {
    pub fn normalized(mut self) -> Self {
        self.enabled_keys = normalize_tokens(&self.enabled_keys, true);
        self.target_windows = normalize_tokens(&self.target_windows, false);
        self.combos = self
            .combos
            .into_iter()
            .map(ComboConfig::normalized)
            .collect();

        if self.enabled_keys.is_empty() && self.combos.is_empty() {
            self.enabled_keys = DEFAULT_KEYS.iter().map(|s| (*s).to_string()).collect();
        }
        if self.target_windows.is_empty() {
            self.target_windows = DEFAULT_TARGET_WINDOWS
                .iter()
                .map(|s| (*s).to_string())
                .collect();
        }
        self.quick_switch_hotkey = normalize_hotkey_text_if_valid(&self.quick_switch_hotkey)
            .unwrap_or_else(default_quick_switch_hotkey);

        if self.repeat_interval_ms == 0 {
            self.repeat_interval_ms = 1;
        }
        if self.press_duration_ms == 0 {
            self.press_duration_ms = 1;
        }
        if self.poll_interval_ms == 0 {
            self.poll_interval_ms = 1;
        }

        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.enabled_keys.is_empty() && self.combos.is_empty() {
            bail!("enabled_keys and combos cannot both be empty");
        }
        if self.target_windows.is_empty() {
            bail!("target_windows cannot be empty");
        }
        parse_hotkey(&self.quick_switch_hotkey)?;
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
        for combo in &self.combos {
            combo.validate()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigStore {
    pub default_profile: String,
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert(DEFAULT_PROFILE_NAME.to_string(), Profile::default());
        Self {
            default_profile: DEFAULT_PROFILE_NAME.to_string(),
            profiles,
        }
    }
}

impl ConfigStore {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let store = Self::default();
            store.save(path)?;
            return Ok(store);
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read '{}'", path.display()))?;
        let parsed: Self = serde_json::from_str(&content)
            .with_context(|| format!("invalid json in '{}'", path.display()))?;
        Ok(parsed.normalized())
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

fn is_zero(value: &u64) -> bool {
    *value == 0
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
mod tests {
    use super::{ComboConfig, ComboStepConfig, ConfigStore, Profile};

    #[test]
    fn profile_normalize_adds_defaults_for_empty_fields() {
        let profile = Profile {
            enabled_keys: Vec::new(),
            repeat_interval_ms: 0,
            press_duration_ms: 0,
            poll_interval_ms: 0,
            target_windows: Vec::new(),
            quick_switch_hotkey: String::new(),
            combos: Vec::new(),
        }
        .normalized();

        assert!(!profile.enabled_keys.is_empty());
        assert!(!profile.target_windows.is_empty());
        assert_eq!(profile.quick_switch_hotkey, "LCTRL+BACKQUOTE");
        assert_eq!(profile.repeat_interval_ms, 1);
        assert_eq!(profile.press_duration_ms, 1);
        assert_eq!(profile.poll_interval_ms, 1);
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
        assert_eq!(combo.steps[0].interval_ms, 1);
        assert_eq!(combo.steps[0].press_duration_ms, 1);
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
                target_windows: vec!["DNF".to_string()],
                quick_switch_hotkey: "LCTRL+Q".to_string(),
                combos: Vec::new(),
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
            target_windows: vec!["DNF".to_string()],
            quick_switch_hotkey: "LCTRL+Q".to_string(),
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
        };

        assert!(profile.validate().is_err());
    }

    #[test]
    fn store_normalized_migrates_legacy_combo_json() {
        let raw = r#"{
          "default_profile": "default",
          "profiles": {
            "default": {
              "enabled_keys": ["J"],
              "repeat_interval_ms": 1,
              "press_duration_ms": 1,
              "poll_interval_ms": 1,
              "target_windows": ["DNF"],
              "combos": [
                {
                  "name": "legacy",
                  "trigger_key": "u",
                  "sequence_keys": ["a", "s", "d"],
                  "step_interval_ms": 80,
                  "press_duration_ms": 1
                }
              ]
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
    }
}
