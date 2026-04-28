// Applies special-key dialog edits to GUI draft and persisted configuration.

use super::*;

impl AppState {
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
}
