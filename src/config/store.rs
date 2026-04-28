// Loads, normalizes, and saves the application configuration store.

use super::{
    DEFAULT_PROFILE_NAME, DEFAULT_QUICK_SWITCH_HOTKEY, DEFAULT_TARGET_WINDOWS, Profile,
    initial::INITIAL_CONFIG_JSON, normalize_tokens,
};
use crate::input::backend::{BackendSettings, InputBackendKind};
use crate::keymap::normalize_hotkey_text;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigStore {
    pub default_profile: String,
    #[serde(default = "default_quick_switch_hotkey")]
    pub quick_switch_hotkey: String,
    #[serde(default = "default_target_windows")]
    pub target_windows: Vec<String>,
    #[serde(default)]
    pub hide_gui_on_startup: bool,
    #[serde(default)]
    pub input_backend: InputBackendKind,
    #[serde(default)]
    pub backend_settings: BackendSettings,
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
            input_backend: InputBackendKind::default(),
            backend_settings: BackendSettings::default(),
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

    pub(super) fn normalized(mut self) -> Self {
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
