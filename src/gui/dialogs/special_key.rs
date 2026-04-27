use super::*;

impl EguiApp {
    fn poll_special_key_capture(&self, dialog: &mut SpecialKeyDialogState, ctx: &egui::Context) {
        let Some(target) = dialog.capture_target else {
            return;
        };

        ctx.request_repaint_after(Duration::from_millis(16));
        let (current_down, captured_key) = capture_next_supported_key(&dialog.capture_down_keys);

        match target {
            SpecialKeyCaptureTarget::AutoTriggerHotkey => {
                dialog.capture_down_keys = current_down.clone();
                let Some(captured_key) = captured_key else {
                    return;
                };
                if is_modifier_key(&captured_key) {
                    return;
                }
                let ordered = sort_hotkey_names(current_down.into_iter().collect::<Vec<_>>());
                dialog.auto_trigger_hotkey = display_hotkey_names(ordered);
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
            _ => {
                dialog.capture_down_keys = current_down;
                let Some(key) = captured_key else {
                    return;
                };
                match target {
                    SpecialKeyCaptureTarget::CustomKey => dialog.custom_key = key,
                    SpecialKeyCaptureTarget::AutoTriggerKey => dialog.auto_trigger_key = key,
                    SpecialKeyCaptureTarget::LinkedTriggerKey => dialog.linked_trigger_key = key,
                    SpecialKeyCaptureTarget::LinkedTargetKey => dialog.linked_target_key = key,
                    SpecialKeyCaptureTarget::AutoTriggerHotkey => unreachable!(),
                }
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
        }
    }

    pub(super) fn render_special_key_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.state.special_key_dialog.take() else {
            return;
        };
        self.poll_special_key_capture(&mut dialog, ctx);

        let mut keep_open = true;
        let mut close_requested = false;
        let title = if dialog.edit_index.is_some() {
            "编辑特殊键位配置"
        } else {
            "新增特殊键位配置"
        };

        egui::Window::new(title)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .open(&mut keep_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(620.0);
                ui.label("名称");
                ui.text_edit_singleline(&mut dialog.name);
                ui.add_space(8.0);

                ui.label("类型");
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut dialog.config_type,
                        SpecialKeyType::CustomAutofire,
                        "独立连发",
                    );
                    ui.selectable_value(
                        &mut dialog.config_type,
                        SpecialKeyType::AutoTrigger,
                        "自动触发",
                    );
                    ui.selectable_value(
                        &mut dialog.config_type,
                        SpecialKeyType::LinkedKey,
                        "连携键位",
                    );
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                match dialog.config_type {
                    SpecialKeyType::CustomAutofire => {
                        ui.horizontal(|ui| {
                            ui.label("键位");
                            ui.monospace(display_key_name(dialog.custom_key.as_str()));
                            let label =
                                if dialog.capture_target == Some(SpecialKeyCaptureTarget::CustomKey)
                                {
                                    "等待按键..."
                                } else {
                                    "录入键位"
                                };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::CustomKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        egui::Grid::new("special_custom_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("连发间隔(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.repeat_interval_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();

                                ui.label("按下时长(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.press_duration_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();
                            });
                    }
                    SpecialKeyType::AutoTrigger => {
                        ui.horizontal(|ui| {
                            ui.label("自动触发键位");
                            ui.monospace(display_key_name(dialog.auto_trigger_key.as_str()));
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::AutoTriggerKey)
                            {
                                "等待按键..."
                            } else {
                                "录入键位"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::AutoTriggerKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("触发热键");
                            ui.monospace(dialog.auto_trigger_hotkey.as_str());
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::AutoTriggerHotkey)
                            {
                                "等待组合键..."
                            } else {
                                "录入热键"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::AutoTriggerHotkey,
                                );
                            }
                        });
                        ui.label(
                            RichText::new(
                                "触发热键会切换该键位的自动触发开关；热键允许组合，但不能与全局快速切换热键冲突。",
                            )
                            .size(12.5)
                            .color(Color32::from_rgb(95, 100, 110)),
                        );
                        ui.add_space(8.0);
                        egui::Grid::new("special_auto_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("触发间隔(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.repeat_interval_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();

                                ui.label("按下时长(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.press_duration_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();
                            });
                    }
                    SpecialKeyType::LinkedKey => {
                        ui.horizontal(|ui| {
                            ui.label("触发键");
                            ui.monospace(display_key_name(dialog.linked_trigger_key.as_str()));
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::LinkedTriggerKey)
                            {
                                "等待按键..."
                            } else {
                                "录入触发键"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::LinkedTriggerKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("连携键");
                            ui.monospace(display_key_name(dialog.linked_target_key.as_str()));
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::LinkedTargetKey)
                            {
                                "等待按键..."
                            } else {
                                "录入连携键"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::LinkedTargetKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("触发方式");
                            ui.selectable_value(
                                &mut dialog.linked_trigger_mode,
                                LinkedTriggerMode::Press,
                                "按下触发",
                            );
                            ui.selectable_value(
                                &mut dialog.linked_trigger_mode,
                                LinkedTriggerMode::Release,
                                "松开触发",
                            );
                        });
                        ui.add_space(8.0);
                        egui::Grid::new("special_linked_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("触发延迟(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.linked_interval_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();

                                ui.label("按下时长(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut dialog.linked_press_duration_ms,
                                    )
                                    .desired_width(100.0),
                                );
                                ui.end_row();
                            });
                    }
                }

                if let Some(message) = special_key_capture_message(dialog.capture_target) {
                    ui.add_space(8.0);
                    ui.label(RichText::new(message).color(Color32::from_rgb(55, 95, 165)));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("保存配置").clicked() {
                        match self.state.save_special_key_dialog(&dialog) {
                            Ok(()) => close_requested = true,
                            Err(err) => self.show_error(&format!("{err:#}")),
                        }
                    }
                    if ui.button("取消").clicked() {
                        close_requested = true;
                    }
                });
            });

        if close_requested {
            keep_open = false;
        }
        if keep_open {
            self.state.special_key_dialog = Some(dialog);
        }
    }
}
