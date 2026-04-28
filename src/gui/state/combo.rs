// Applies combo dialog edits to GUI draft and persisted configuration.

use super::*;

impl AppState {
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
}
