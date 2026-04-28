// Manages profile draft state and generated names.

use super::*;

impl AppState {
    pub(super) fn new(config_path: PathBuf, store: ConfigStore) -> Self {
        let default_draft = ProfileDraft::from_named_profile("default", &Profile::default());
        let global_quick_switch_hotkey = display_hotkey_text(&store.quick_switch_hotkey);
        let global_target_windows_text = target_windows_to_text(&store.target_windows);
        let mut state = Self {
            config_path,
            store,
            current_profile_key: None,
            draft: default_draft,
            global_target_windows_text,
            enabled_keys: HashSet::new(),
            tray_state: TrayState::Disabled,
            window_hidden: false,
            quitting: false,
            runner: None,
            selected_combo_index: None,
            combo_dialog: None,
            selected_special_key_index: None,
            special_key_dialog: None,
            status_text: STATUS_IDLE,
            last_minimized: false,
            window_mode: WindowMode::Main,
            switcher_selected_profile: None,
            global_quick_switch_hotkey,
            quick_switch_hotkey_capturing: false,
            quick_switch_hotkey_down_keys: HashSet::new(),
            pending_switch_start_profile: None,
        };
        state.sync_enabled_keys(&state.draft.enabled_keys.clone());
        state
    }

    pub(super) fn setup_initial_state(&mut self) -> Result<()> {
        let default_name = self.store.default_profile.clone();
        self.load_profile_from_store(&default_name)?;
        self.tray_state = TrayState::Disabled;
        self.status_text = STATUS_IDLE;
        Ok(())
    }

    pub(super) fn is_editing_enabled(&self) -> bool {
        !self.is_runner_active()
    }

    pub(super) fn is_runner_active(&self) -> bool {
        self.runner
            .as_ref()
            .map(RunnerHandle::is_running)
            .unwrap_or(false)
    }

    pub(super) fn load_profile_from_store(&mut self, name: &str) -> Result<()> {
        let profile = self
            .store
            .get_profile(Some(name))
            .with_context(|| format!("配置不存在: {name}"))?;
        let draft = ProfileDraft::from_named_profile(name, &profile);
        self.current_profile_key = Some(name.to_string());
        self.draft = draft.clone();
        self.sync_enabled_keys(&draft.enabled_keys);
        self.selected_combo_index = None;
        self.combo_dialog = None;
        self.selected_special_key_index = None;
        self.special_key_dialog = None;
        self.status_text = STATUS_IDLE;
        self.tray_state = TrayState::Disabled;
        self.switcher_selected_profile = Some(name.to_string());
        Ok(())
    }

    pub(super) fn sync_enabled_keys(&mut self, enabled_keys: &[String]) {
        self.enabled_keys.clear();
        for key in enabled_keys {
            if let Ok(spec) = parse_single_key(key) {
                self.enabled_keys.insert(spec.name.to_string());
            }
        }
    }

    pub(super) fn toggle_enabled_key(&mut self, token: &str) {
        if !self.enabled_keys.remove(token) {
            self.enabled_keys.insert(token.to_string());
        }
    }

    pub(super) fn build_draft_from_form(&self) -> ProfileDraft {
        let mut draft = self.draft.clone();
        draft.enabled_keys = self.selected_enabled_keys();
        draft
    }

    pub(super) fn current_target_windows(&self) -> Result<Vec<String>> {
        let target_windows = target_windows_from_text(&self.global_target_windows_text);
        if target_windows.is_empty() {
            bail!("请至少填写一个目标窗口关键字");
        }
        Ok(target_windows)
    }

    pub(super) fn selected_enabled_keys(&self) -> Vec<String> {
        supported_key_names()
            .iter()
            .filter(|name| self.enabled_keys.contains(**name))
            .map(|name| (*name).to_string())
            .collect()
    }

    pub(super) fn validate_draft(&self, draft: &ProfileDraft) -> Result<(String, Profile)> {
        let (name, profile) = draft.to_named_profile()?;
        let normalized_quick_switch_hotkey =
            normalize_hotkey_text(&self.global_quick_switch_hotkey)?;
        if !profile.enabled_keys.is_empty() {
            parse_key_specs(&profile.enabled_keys)?;
        }
        for combo in &profile.combos {
            parse_single_key(&combo.trigger_key)?;
            for step in &combo.steps {
                parse_single_key(&step.key)?;
            }
        }
        for special in &profile.special_keys {
            match special {
                SpecialKeyConfig::CustomAutofire { key, .. } => {
                    parse_single_key(key)?;
                }
                SpecialKeyConfig::AutoTrigger {
                    key,
                    trigger_hotkey,
                    ..
                } => {
                    parse_single_key(key)?;
                    let normalized_trigger_hotkey = normalize_hotkey_text(trigger_hotkey)?;
                    parse_hotkey(&normalized_trigger_hotkey)?;
                    if normalized_trigger_hotkey == normalized_quick_switch_hotkey {
                        bail!("自动触发热键不能与全局快速切换热键冲突");
                    }
                }
                SpecialKeyConfig::LinkedKey {
                    trigger_key,
                    linked_key,
                    ..
                } => {
                    parse_single_key(trigger_key)?;
                    parse_single_key(linked_key)?;
                }
            }
        }
        Ok((name, profile))
    }

    pub(super) fn new_profile(&mut self) -> Result<()> {
        let name = self.generate_profile_name();
        let draft = ProfileDraft::from_named_profile(&name, &Profile::default());
        self.current_profile_key = None;
        self.draft = draft.clone();
        self.sync_enabled_keys(&draft.enabled_keys);
        self.selected_combo_index = None;
        self.combo_dialog = None;
        self.selected_special_key_index = None;
        self.special_key_dialog = None;
        self.status_text = STATUS_IDLE;
        self.quick_switch_hotkey_capturing = false;
        self.quick_switch_hotkey_down_keys.clear();
        Ok(())
    }

    pub(super) fn save_profile(&mut self) -> Result<()> {
        let draft = self.build_draft_from_form();
        let (new_name, profile) = self.validate_draft(&draft)?;
        let original_name = self.current_profile_key.clone();

        if let Some(original_name) = &original_name {
            if original_name != &new_name && self.store.profiles.contains_key(&new_name) {
                bail!("已存在同名配置: {new_name}");
            }
        } else if self.store.profiles.contains_key(&new_name) {
            bail!("已存在同名配置: {new_name}");
        }

        let renamed_default = original_name
            .as_ref()
            .map(|name| name == &self.store.default_profile && name != &new_name)
            .unwrap_or(false);

        if let Some(original_name) = original_name {
            if original_name != new_name {
                self.store.profiles.remove(&original_name);
            }
        }

        self.store.upsert_profile(new_name.clone(), profile.clone());
        if renamed_default {
            self.store.default_profile = new_name.clone();
        }
        self.store.save(&self.config_path)?;

        self.current_profile_key = Some(new_name.clone());
        self.draft = ProfileDraft::from_named_profile(&new_name, &profile);
        self.status_text = STATUS_STOPPED;
        self.switcher_selected_profile = Some(new_name);
        Ok(())
    }

    pub(super) fn clone_profile(&mut self) -> Result<()> {
        let mut draft = self.build_draft_from_form();
        let clone_name = self.generate_cloned_profile_name();
        draft.name = clone_name.clone();
        let (name, profile) = self.validate_draft(&draft)?;

        self.store.upsert_profile(name.clone(), profile.clone());
        self.store.save(&self.config_path)?;

        self.current_profile_key = Some(name.clone());
        self.draft = ProfileDraft::from_named_profile(&name, &profile);
        self.sync_enabled_keys(&self.draft.enabled_keys.clone());
        self.selected_combo_index = None;
        self.combo_dialog = None;
        self.selected_special_key_index = None;
        self.special_key_dialog = None;
        self.status_text = STATUS_STOPPED;
        self.tray_state = TrayState::Disabled;
        self.switcher_selected_profile = Some(name);
        Ok(())
    }

    pub(super) fn delete_profile(&mut self) -> Result<()> {
        let current_key = self
            .current_profile_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("当前是未保存的新配置，不能直接删除"))?;
        if self.draft.name.trim() != current_key {
            bail!("当前配置名已修改，请先保存后再删除");
        }

        self.store.delete_profile(&current_key)?;
        self.store.save(&self.config_path)?;

        let next_name = self.store.default_profile.clone();
        self.load_profile_from_store(&next_name)?;
        self.status_text = STATUS_STOPPED;
        Ok(())
    }

    pub(super) fn generate_profile_name(&self) -> String {
        let mut index = 1;
        loop {
            let candidate = format!("profile-{index}");
            if !self.store.profiles.contains_key(&candidate) {
                return candidate;
            }
            index += 1;
        }
    }

    pub(super) fn generate_cloned_profile_name(&self) -> String {
        let base_name = self.draft.name.trim().trim_end_matches('-').to_string();
        let mut base_name = if base_name.is_empty() {
            self.current_profile_key
                .clone()
                .unwrap_or_else(|| "profile".to_string())
        } else {
            base_name
        };
        while let Some(stripped) = strip_clone_suffix(&base_name) {
            base_name = stripped.to_string();
        }

        let mut index = 1;
        loop {
            let candidate = format!("{base_name}-cloned-{index}");
            if !self.store.profiles.contains_key(&candidate) {
                return candidate;
            }
            index += 1;
        }
    }

    pub(super) fn generate_combo_name(&self) -> String {
        let mut index = 1;
        loop {
            let candidate = format!("combo-{index}");
            if !self
                .draft
                .combos
                .iter()
                .any(|combo| combo.name.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            index += 1;
        }
    }

    pub(super) fn generate_special_key_name(&self) -> String {
        let mut index = 1;
        loop {
            let candidate = format!("special-{index}");
            if !self
                .draft
                .special_keys
                .iter()
                .any(|special| special.name().eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            index += 1;
        }
    }
}
