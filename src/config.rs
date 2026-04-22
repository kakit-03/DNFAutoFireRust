use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

const DEFAULT_PROFILE_NAME: &str = "default";
const DEFAULT_KEYS: [&str; 4] = ["J", "P", "L", "H"];
const DEFAULT_TARGET_WINDOWS: [&str; 2] = ["地下城与勇士", "DNF"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboConfig {
    pub name: String,
    pub trigger_key: String,
    pub sequence_keys: Vec<String>,
    pub step_interval_ms: u64,
    pub press_duration_ms: u64,
}

impl ComboConfig {
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.trigger_key = self.trigger_key.trim().to_ascii_uppercase();
        self.sequence_keys = normalize_key_sequence(&self.sequence_keys);

        if self.step_interval_ms == 0 {
            self.step_interval_ms = 1;
        }
        if self.press_duration_ms == 0 {
            self.press_duration_ms = 1;
        }

        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            bail!("combo name cannot be empty");
        }
        if self.trigger_key.is_empty() {
            bail!("combo trigger_key cannot be empty");
        }
        if self.sequence_keys.is_empty() {
            bail!("combo sequence_keys cannot be empty");
        }
        if self.step_interval_ms == 0 {
            bail!("combo step_interval_ms must be >= 1");
        }
        if self.press_duration_ms == 0 {
            bail!("combo press_duration_ms must be >= 1");
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

    pub fn set_default_profile(&mut self, name: &str) -> Result<()> {
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
    use super::{ComboConfig, ConfigStore, Profile};

    #[test]
    fn profile_normalize_adds_defaults_for_empty_fields() {
        let profile = Profile {
            enabled_keys: Vec::new(),
            repeat_interval_ms: 0,
            press_duration_ms: 0,
            poll_interval_ms: 0,
            target_windows: Vec::new(),
            combos: Vec::new(),
        }
        .normalized();

        assert!(!profile.enabled_keys.is_empty());
        assert!(!profile.target_windows.is_empty());
        assert_eq!(profile.repeat_interval_ms, 1);
        assert_eq!(profile.press_duration_ms, 1);
        assert_eq!(profile.poll_interval_ms, 1);
    }

    #[test]
    fn combo_normalize_upcases_keys_and_fixes_intervals() {
        let combo = ComboConfig {
            name: " test ".to_string(),
            trigger_key: "j".to_string(),
            sequence_keys: vec!["a".to_string(), "a".to_string(), "b".to_string()],
            step_interval_ms: 0,
            press_duration_ms: 0,
        }
        .normalized();

        assert_eq!(combo.name, "test");
        assert_eq!(combo.trigger_key, "J");
        assert_eq!(combo.sequence_keys, vec!["A", "A", "B"]);
        assert_eq!(combo.step_interval_ms, 1);
        assert_eq!(combo.press_duration_ms, 1);
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
    fn duplicate_combo_names_are_rejected() {
        let profile = Profile {
            enabled_keys: vec!["J".to_string()],
            repeat_interval_ms: 1,
            press_duration_ms: 1,
            poll_interval_ms: 1,
            target_windows: vec!["DNF".to_string()],
            combos: vec![
                ComboConfig {
                    name: "combo".to_string(),
                    trigger_key: "A".to_string(),
                    sequence_keys: vec!["B".to_string()],
                    step_interval_ms: 1,
                    press_duration_ms: 1,
                },
                ComboConfig {
                    name: "COMBO".to_string(),
                    trigger_key: "C".to_string(),
                    sequence_keys: vec!["D".to_string()],
                    step_interval_ms: 1,
                    press_duration_ms: 1,
                },
            ],
        };

        assert!(profile.validate().is_err());
    }
}
