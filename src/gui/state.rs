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

    pub(super) fn start_runner_from_form(
        &mut self,
        event_tx: &Sender<AppEvent>,
        ctx: &egui::Context,
    ) -> Result<()> {
        if self.is_runner_active() {
            bail!("连发已经在运行");
        }

        let draft = self.build_draft_from_form();
        let (_, profile) = self.validate_draft(&draft)?;
        let target_windows = self.current_target_windows()?;
        let input_backend = self.store.input_backend;
        let backend_settings = self.store.backend_settings.clone();
        let tx = event_tx.clone();
        let repaint_ctx = ctx.clone();
        let mut handle = AutoFireService::start_with_backend_events(
            profile,
            target_windows,
            input_backend,
            backend_settings,
            move |event| {
                let _ = tx.send(AppEvent::Runner(event));
                repaint_ctx.request_repaint();
            },
        )?;
        if let Err(err) = self.remember_last_started_profile() {
            handle.stop();
            let _ = handle.wait();
            return Err(err);
        }

        self.runner = Some(handle);
        self.status_text = STATUS_RUNNING;
        self.tray_state = TrayState::Enabled;
        Ok(())
    }

    pub(super) fn request_stop_runner(&self) {
        if let Some(runner) = self.runner.as_ref().filter(|runner| runner.is_running()) {
            runner.stop();
        }
    }

    pub(super) fn cancel_pending_start_and_request_stop(&mut self) {
        self.pending_switch_start_profile = None;
        self.request_stop_runner();
        self.status_text = STATUS_STOPPED;
        self.tray_state = TrayState::Disabled;
    }

    pub(super) fn start_quick_switch_hotkey_capture(&mut self) {
        self.quick_switch_hotkey_capturing = true;
        self.quick_switch_hotkey_down_keys = currently_pressed_supported_keys();
    }

    pub(super) fn stop_quick_switch_hotkey_capture(&mut self) {
        self.quick_switch_hotkey_capturing = false;
        self.quick_switch_hotkey_down_keys.clear();
    }

    pub(super) fn poll_quick_switch_hotkey_capture(&mut self, ctx: &egui::Context) -> Result<()> {
        if !self.quick_switch_hotkey_capturing {
            return Ok(());
        }

        ctx.request_repaint_after(Duration::from_millis(16));
        let (current_down, captured_key) =
            capture_next_supported_key(&self.quick_switch_hotkey_down_keys);
        self.quick_switch_hotkey_down_keys = current_down.clone();

        let Some(captured_key) = captured_key else {
            return Ok(());
        };
        if is_modifier_key(&captured_key) {
            return Ok(());
        }

        let ordered = sort_hotkey_names(current_down.into_iter().collect::<Vec<_>>());
        self.global_quick_switch_hotkey = display_hotkey_names(ordered);
        self.stop_quick_switch_hotkey_capture();
        self.persist_global_quick_switch_hotkey()?;
        Ok(())
    }

    pub(super) fn persist_global_quick_switch_hotkey(&mut self) -> Result<()> {
        let normalized = normalize_hotkey_text(&self.global_quick_switch_hotkey)?;
        self.store.quick_switch_hotkey = normalized.clone();
        self.store.save(&self.config_path)?;
        self.global_quick_switch_hotkey = display_hotkey_text(&normalized);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn persist_global_target_windows(&mut self) -> Result<()> {
        let target_windows = self.current_target_windows()?;
        self.store.target_windows = target_windows;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    pub(super) fn set_input_backend(&mut self, input_backend: InputBackendKind) -> Result<()> {
        let descriptor = input_backend_descriptor(input_backend);
        if !descriptor.available {
            bail!(
                "输入后端 '{}' 暂不可用: {}",
                descriptor.label,
                descriptor
                    .unavailable_reason
                    .unwrap_or("当前版本尚未实现该后端")
            );
        }

        self.store.input_backend = input_backend;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    pub(super) fn persist_global_target_windows_if_valid(&mut self) -> Result<()> {
        let target_windows = target_windows_from_text(&self.global_target_windows_text);
        if target_windows.is_empty() {
            return Ok(());
        }
        self.store.target_windows = target_windows;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    pub(super) fn toggle_hide_gui_on_startup(&mut self) -> Result<()> {
        self.store.hide_gui_on_startup = !self.store.hide_gui_on_startup;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    pub(super) fn remember_last_started_profile(&mut self) -> Result<()> {
        let Some(current_key) = self.current_profile_key.clone() else {
            return Ok(());
        };
        if self.draft.name.trim() != current_key {
            return Ok(());
        }

        self.store.remember_started_profile(&current_key)?;
        self.store.save(&self.config_path)?;
        self.switcher_selected_profile = Some(current_key);
        Ok(())
    }

    pub(super) fn ensure_switcher_selection(&mut self, names: &[String]) {
        if names.is_empty() {
            self.switcher_selected_profile = None;
            return;
        }

        let current = self
            .switcher_selected_profile
            .clone()
            .filter(|name| names.iter().any(|candidate| candidate == name));
        self.switcher_selected_profile =
            current.or_else(|| Some(self.store.default_profile.clone()));
        if !names
            .iter()
            .any(|candidate| Some(candidate.as_str()) == self.switcher_selected_profile.as_deref())
        {
            self.switcher_selected_profile = names.first().cloned();
        }
    }

    pub(super) fn move_switcher_selection(&mut self, step: isize) {
        let names = self
            .store
            .list_profile_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if names.is_empty() {
            self.switcher_selected_profile = None;
            return;
        }
        self.ensure_switcher_selection(&names);

        let current_index = names
            .iter()
            .position(|name| Some(name.as_str()) == self.switcher_selected_profile.as_deref())
            .unwrap_or(0) as isize;
        let len = names.len() as isize;
        let next_index = (current_index + step).rem_euclid(len) as usize;
        self.switcher_selected_profile = Some(names[next_index].clone());
    }

    pub(super) fn switch_profile_and_start_selected(
        &mut self,
        event_tx: &Sender<AppEvent>,
        ctx: &egui::Context,
    ) -> Result<()> {
        let selected = self
            .switcher_selected_profile
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先选择一个配置"))?;
        if self.is_runner_active() {
            self.pending_switch_start_profile = Some(selected);
            self.request_stop_runner();
            return Ok(());
        }
        self.load_profile_from_store(&selected)?;
        self.start_runner_from_form(event_tx, ctx)
    }

    pub(super) fn quick_switch_watch_config(&self) -> QuickSwitchWatchConfig {
        let target_windows = target_windows_from_text(&self.global_target_windows_text);
        if target_windows.is_empty() {
            return QuickSwitchWatchConfig::default();
        }

        let Ok(hotkey) = hotkey_registration(&self.global_quick_switch_hotkey) else {
            return QuickSwitchWatchConfig::default();
        };

        QuickSwitchWatchConfig {
            hotkey: Some(hotkey),
            target_windows,
        }
    }

    pub(super) fn try_start_pending_switch_profile(
        &mut self,
        event_tx: &Sender<AppEvent>,
        ctx: &egui::Context,
    ) -> Result<()> {
        if self.runner.is_some() {
            return Ok(());
        }

        let Some(profile_name) = self.pending_switch_start_profile.take() else {
            return Ok(());
        };

        self.load_profile_from_store(&profile_name)?;
        self.start_runner_from_form(event_tx, ctx)
    }

    pub(super) fn handle_runner_event(&mut self, event: RunnerEvent) -> Result<()> {
        match event {
            RunnerEvent::Started | RunnerEvent::ResumedFromIme => {
                self.status_text = STATUS_RUNNING;
                self.tray_state = TrayState::Enabled;
            }
            RunnerEvent::PausedByIme => {
                self.status_text = STATUS_IME_PAUSED;
                self.tray_state = TrayState::Paused;
            }
            RunnerEvent::Stopped(_) => {
                if let Some(mut runner) = self.runner.take() {
                    runner.wait()?;
                }
                self.status_text = STATUS_STOPPED;
                self.tray_state = TrayState::Disabled;
            }
        }
        Ok(())
    }

    pub(super) fn shutdown_runner(&mut self) {
        if let Some(runner) = self.runner.as_ref() {
            runner.stop();
        }
        if let Some(mut runner) = self.runner.take() {
            let _ = runner.wait();
        }
    }

    pub(super) fn open_combo_dialog(&mut self, edit_index: Option<usize>) {
        self.combo_dialog = Some(if let Some(index) = edit_index {
            let combo = self.draft.combos[index].clone();
            let last_step_interval = combo
                .steps
                .last()
                .map(|step| step.interval_ms.to_string())
                .unwrap_or_else(|| DEFAULT_COMBO_STEP_INTERVAL_MS.to_string());
            let last_step_press = combo
                .steps
                .last()
                .map(|step| step.press_duration_ms.to_string())
                .unwrap_or_else(|| DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string());
            ComboDialogState {
                edit_index: Some(index),
                name: combo.name,
                trigger_key: combo.trigger_key,
                steps: combo
                    .steps
                    .into_iter()
                    .map(|step| ComboStepDraft {
                        key: step.key,
                        interval_ms: step.interval_ms.to_string(),
                        press_duration_ms: step.press_duration_ms.to_string(),
                    })
                    .collect(),
                selected_step: None,
                new_step_interval_ms: last_step_interval,
                new_step_press_duration_ms: last_step_press,
                capture_target: None,
                capture_down_keys: HashSet::new(),
            }
        } else {
            ComboDialogState {
                edit_index: None,
                name: self.generate_combo_name(),
                trigger_key: "未录入".to_string(),
                steps: Vec::new(),
                selected_step: None,
                new_step_interval_ms: DEFAULT_COMBO_STEP_INTERVAL_MS.to_string(),
                new_step_press_duration_ms: DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string(),
                capture_target: None,
                capture_down_keys: HashSet::new(),
            }
        });
    }

    pub(super) fn open_selected_combo_dialog(&mut self) -> Result<()> {
        let Some(index) = self.selected_combo_index else {
            bail!("请先选中一个连招");
        };
        self.open_combo_dialog(Some(index));
        Ok(())
    }

    pub(super) fn remove_selected_combo(&mut self) -> Result<()> {
        let Some(index) = self.selected_combo_index else {
            bail!("请先选中一个连招");
        };
        self.draft.combos.remove(index);
        self.selected_combo_index = None;
        self.persist_combo_changes()?;
        Ok(())
    }

    pub(super) fn save_combo_dialog(&mut self, dialog: &ComboDialogState) -> Result<()> {
        let combo = self.read_combo_from_dialog(dialog)?;
        for (index, existing) in self.draft.combos.iter().enumerate() {
            if Some(index) != dialog.edit_index && existing.name.eq_ignore_ascii_case(&combo.name) {
                bail!("已存在同名连招: {}", combo.name);
            }
        }

        if let Some(index) = dialog.edit_index {
            self.draft.combos[index] = combo;
            self.selected_combo_index = Some(index);
        } else {
            self.draft.combos.push(combo);
            self.selected_combo_index = Some(self.draft.combos.len().saturating_sub(1));
        }
        self.persist_combo_changes()?;
        Ok(())
    }

    pub(super) fn persist_combo_changes(&mut self) -> Result<()> {
        if self.current_profile_key.is_none()
            || self
                .current_profile_key
                .as_deref()
                .is_some_and(|name| self.draft.name.trim() != name)
        {
            return self.save_profile();
        }

        let current_key = self
            .current_profile_key
            .clone()
            .expect("current_profile_key checked above");
        let mut profile = self
            .store
            .get_profile(Some(&current_key))
            .with_context(|| format!("配置不存在: {current_key}"))?;
        profile.combos = self.draft.combos.clone();
        profile.special_keys = self.draft.special_keys.clone();
        profile = profile.normalized();
        profile.validate()?;
        self.validate_draft(&ProfileDraft::from_named_profile(&current_key, &profile))?;

        self.store.upsert_profile(current_key, profile);
        self.store.save(&self.config_path)?;
        self.status_text = STATUS_STOPPED;
        Ok(())
    }

    pub(super) fn read_combo_from_dialog(&self, dialog: &ComboDialogState) -> Result<ComboConfig> {
        if dialog.trigger_key.trim().is_empty() || dialog.trigger_key == "未录入" {
            bail!("请先录入触发键");
        }
        if dialog.steps.is_empty() {
            bail!("请至少录入一个步骤");
        }

        let steps = dialog
            .steps
            .iter()
            .map(|step| {
                Ok(ComboStepConfig {
                    key: step.key.clone(),
                    interval_ms: parse_dialog_ms(&step.interval_ms, "步骤间隔")?,
                    press_duration_ms: parse_dialog_ms(&step.press_duration_ms, "步骤按下时长")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let combo = ComboConfig {
            name: dialog.name.clone(),
            trigger_key: dialog.trigger_key.clone(),
            steps,
            sequence_keys: Vec::new(),
            step_interval_ms: 0,
            press_duration_ms: 0,
        }
        .normalized();
        combo.validate()?;
        parse_single_key(&combo.trigger_key)?;
        for step in &combo.steps {
            parse_single_key(&step.key)?;
        }
        Ok(combo)
    }

    pub(super) fn open_special_key_dialog(&mut self, edit_index: Option<usize>) {
        self.special_key_dialog = Some(if let Some(index) = edit_index {
            special_key_dialog_from_config(index, &self.draft.special_keys[index])
        } else {
            SpecialKeyDialogState {
                edit_index: None,
                config_type: SpecialKeyType::CustomAutofire,
                name: self.generate_special_key_name(),
                custom_key: "未录入".to_string(),
                auto_trigger_key: "未录入".to_string(),
                auto_trigger_hotkey: "未录入".to_string(),
                linked_trigger_key: "未录入".to_string(),
                linked_target_key: "未录入".to_string(),
                linked_trigger_mode: LinkedTriggerMode::Press,
                repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS.to_string(),
                press_duration_ms: DEFAULT_PRESS_DURATION_MS.to_string(),
                linked_interval_ms: DEFAULT_COMBO_STEP_INTERVAL_MS.to_string(),
                linked_press_duration_ms: DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string(),
                capture_target: None,
                capture_down_keys: HashSet::new(),
            }
        });
    }

    pub(super) fn open_selected_special_key_dialog(&mut self) -> Result<()> {
        let Some(index) = self.selected_special_key_index else {
            bail!("请先选中一个特殊键位配置");
        };
        self.open_special_key_dialog(Some(index));
        Ok(())
    }

    pub(super) fn remove_selected_special_key(&mut self) -> Result<()> {
        let Some(index) = self.selected_special_key_index else {
            bail!("请先选中一个特殊键位配置");
        };
        self.draft.special_keys.remove(index);
        self.selected_special_key_index = None;
        self.persist_special_key_changes()?;
        Ok(())
    }

    pub(super) fn save_special_key_dialog(&mut self, dialog: &SpecialKeyDialogState) -> Result<()> {
        let special_key = self.read_special_key_from_dialog(dialog)?;
        for (index, existing) in self.draft.special_keys.iter().enumerate() {
            if Some(index) != dialog.edit_index
                && existing.name().eq_ignore_ascii_case(special_key.name())
            {
                bail!("已存在同名特殊键位配置: {}", special_key.name());
            }
        }

        if let Some(index) = dialog.edit_index {
            self.draft.special_keys[index] = special_key;
            self.selected_special_key_index = Some(index);
        } else {
            self.draft.special_keys.push(special_key);
            self.selected_special_key_index = Some(self.draft.special_keys.len().saturating_sub(1));
        }
        self.persist_special_key_changes()?;
        Ok(())
    }

    pub(super) fn persist_special_key_changes(&mut self) -> Result<()> {
        if self.current_profile_key.is_none()
            || self
                .current_profile_key
                .as_deref()
                .is_some_and(|name| self.draft.name.trim() != name)
        {
            return self.save_profile();
        }

        let current_key = self
            .current_profile_key
            .clone()
            .expect("current_profile_key checked above");
        let mut profile = self
            .store
            .get_profile(Some(&current_key))
            .with_context(|| format!("配置不存在: {current_key}"))?;
        profile.combos = self.draft.combos.clone();
        profile.special_keys = self.draft.special_keys.clone();
        profile = profile.normalized();
        profile.validate()?;
        self.validate_draft(&ProfileDraft::from_named_profile(&current_key, &profile))?;

        self.store.upsert_profile(current_key, profile);
        self.store.save(&self.config_path)?;
        self.status_text = STATUS_STOPPED;
        Ok(())
    }

    pub(super) fn read_special_key_from_dialog(
        &self,
        dialog: &SpecialKeyDialogState,
    ) -> Result<SpecialKeyConfig> {
        let name = dialog.name.trim();
        if name.is_empty() {
            bail!("请填写配置名称");
        }

        let special = match dialog.config_type {
            SpecialKeyType::CustomAutofire => {
                if dialog.custom_key.trim().is_empty() || dialog.custom_key == "未录入" {
                    bail!("请先录入独立连发键位");
                }
                SpecialKeyConfig::CustomAutofire {
                    name: name.to_string(),
                    key: dialog.custom_key.clone(),
                    repeat_interval_ms: parse_dialog_ms(&dialog.repeat_interval_ms, "连发间隔")?,
                    press_duration_ms: parse_dialog_ms(&dialog.press_duration_ms, "按下时长")?,
                }
            }
            SpecialKeyType::AutoTrigger => {
                if dialog.auto_trigger_key.trim().is_empty() || dialog.auto_trigger_key == "未录入"
                {
                    bail!("请先录入自动触发键位");
                }
                if dialog.auto_trigger_hotkey.trim().is_empty()
                    || dialog.auto_trigger_hotkey == "未录入"
                {
                    bail!("请先录入自动触发热键");
                }
                SpecialKeyConfig::AutoTrigger {
                    name: name.to_string(),
                    key: dialog.auto_trigger_key.clone(),
                    trigger_hotkey: dialog.auto_trigger_hotkey.clone(),
                    repeat_interval_ms: parse_dialog_ms(&dialog.repeat_interval_ms, "触发间隔")?,
                    press_duration_ms: parse_dialog_ms(&dialog.press_duration_ms, "按下时长")?,
                }
            }
            SpecialKeyType::LinkedKey => {
                if dialog.linked_trigger_key.trim().is_empty()
                    || dialog.linked_trigger_key == "未录入"
                {
                    bail!("请先录入触发键");
                }
                if dialog.linked_target_key.trim().is_empty()
                    || dialog.linked_target_key == "未录入"
                {
                    bail!("请先录入连携键");
                }
                SpecialKeyConfig::LinkedKey {
                    name: name.to_string(),
                    trigger_key: dialog.linked_trigger_key.clone(),
                    linked_key: dialog.linked_target_key.clone(),
                    trigger_mode: dialog.linked_trigger_mode,
                    interval_ms: parse_dialog_ms(&dialog.linked_interval_ms, "触发延迟")?,
                    press_duration_ms: parse_dialog_ms(
                        &dialog.linked_press_duration_ms,
                        "按下时长",
                    )?,
                }
            }
        }
        .normalized();

        let mut draft = self.draft.clone();
        if let Some(index) = dialog.edit_index {
            draft.special_keys[index] = special.clone();
        } else {
            draft.special_keys.push(special.clone());
        }
        self.validate_draft(&draft)?;
        Ok(special)
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
