// Controls runner startup, shutdown, quick switching, and global settings.

use super::*;

impl AppState {
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

    pub(super) fn stop_runner_for_editing(&mut self) -> Result<()> {
        self.pending_switch_start_profile = None;
        if let Some(runner) = self.runner.as_ref() {
            runner.stop();
        }
        if let Some(mut runner) = self.runner.take() {
            runner.wait()?;
        }
        self.status_text = STATUS_STOPPED;
        self.tray_state = TrayState::Disabled;
        Ok(())
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
}
