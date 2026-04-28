// Renders combo editing and capture dialogs.

use super::*;

impl EguiApp {
    fn poll_combo_capture(&self, dialog: &mut ComboDialogState, ctx: &egui::Context) {
        let Some(target) = dialog.capture_target else {
            return;
        };

        ctx.request_repaint_after(Duration::from_millis(16));
        let (current_down, captured_key) = capture_next_supported_key(&dialog.capture_down_keys);
        dialog.capture_down_keys = current_down;

        let Some(key) = captured_key else {
            return;
        };

        match target {
            ComboCaptureTarget::Trigger => {
                dialog.trigger_key = key;
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
            ComboCaptureTarget::NewStep => {
                dialog.steps.push(ComboStepDraft {
                    key,
                    interval_ms: dialog.new_step_interval_ms.clone(),
                    press_duration_ms: dialog.new_step_press_duration_ms.clone(),
                });
                dialog.selected_step = Some(dialog.steps.len().saturating_sub(1));
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
            ComboCaptureTarget::ContinuousSteps => {
                dialog.steps.push(ComboStepDraft {
                    key,
                    interval_ms: dialog.new_step_interval_ms.clone(),
                    press_duration_ms: dialog.new_step_press_duration_ms.clone(),
                });
                dialog.selected_step = Some(dialog.steps.len().saturating_sub(1));
            }
            ComboCaptureTarget::SelectedStep => {
                if let Some(index) = dialog.selected_step {
                    dialog.steps[index].key = key;
                }
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
        }
    }

    pub(super) fn render_combo_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.state.combo_dialog.take() else {
            return;
        };
        self.poll_combo_capture(&mut dialog, ctx);

        let mut keep_open = true;
        let mut close_requested = false;
        let title = if dialog.edit_index.is_some() {
            "编辑连招"
        } else {
            "新增连招"
        };

        egui::Window::new(title)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .open(&mut keep_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(640.0);
                ui.label("名称");
                ui.text_edit_singleline(&mut dialog.name);
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label("触发键");
                        ui.horizontal(|ui| {
                            ui.monospace(display_key_name(dialog.trigger_key.as_str()));
                            let label =
                                if dialog.capture_target == Some(ComboCaptureTarget::Trigger) {
                                    "等待按键..."
                                } else {
                                    "录入触发键"
                                };
                            if ui.button(label).clicked() {
                                start_combo_capture(&mut dialog, ComboCaptureTarget::Trigger);
                            }
                        });
                    });
                });

                if let Some(message) = combo_capture_message(dialog.capture_target) {
                    ui.add_space(6.0);
                    ui.label(RichText::new(message).color(Color32::from_rgb(55, 95, 165)));
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                ui.strong("新增步骤");
                ui.horizontal(|ui| {
                    ui.label("步骤间隔(ms)");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.new_step_interval_ms)
                            .desired_width(80.0),
                    );
                    ui.label("按下时长(ms)");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.new_step_press_duration_ms)
                            .desired_width(80.0),
                    );
                    let single_label = if dialog.capture_target == Some(ComboCaptureTarget::NewStep)
                    {
                        "等待按键..."
                    } else {
                        "按键录入并追加"
                    };
                    if ui.button(single_label).clicked() {
                        start_combo_capture(&mut dialog, ComboCaptureTarget::NewStep);
                    }
                    let continuous_active =
                        dialog.capture_target == Some(ComboCaptureTarget::ContinuousSteps);
                    let continuous_label = if continuous_active {
                        "停止连续录入"
                    } else {
                        "开始连续录入"
                    };
                    if ui.button(continuous_label).clicked() {
                        if continuous_active {
                            dialog.capture_target = None;
                            dialog.capture_down_keys.clear();
                        } else {
                            start_combo_capture(&mut dialog, ComboCaptureTarget::ContinuousSteps);
                        }
                    }
                });
                ui.label(
                    RichText::new(
                        "步骤间隔表示当前步骤执行后，到下一步开始前的等待时间。连续录入开启后，可直接连续按键追加步骤。",
                    )
                        .size(12.5)
                        .color(Color32::from_rgb(95, 100, 110)),
                );
                if let Some(last_step) = dialog.steps.last() {
                    ui.label(
                        RichText::new(format!(
                            "最近一步: {} / 间隔 {} ms / 按下 {} ms",
                            display_key_name(&last_step.key),
                            last_step.interval_ms,
                            last_step.press_duration_ms
                        ))
                        .size(12.5)
                        .color(Color32::from_rgb(55, 95, 165)),
                    );
                } else {
                    ui.label(
                        RichText::new("最近一步: 还没有录入步骤")
                            .size(12.5)
                            .color(Color32::from_rgb(120, 125, 135)),
                    );
                }

                ui.add_space(10.0);
                ui.strong("步骤列表");
                let stick_steps_to_bottom =
                    dialog.capture_target == Some(ComboCaptureTarget::ContinuousSteps);
                ScrollArea::vertical()
                    .id_salt("combo_sequence_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .stick_to_bottom(stick_steps_to_bottom)
                    .max_height(180.0)
                    .show(ui, |ui| {
                        let mut start_capture_step = None;
                        let mut move_up_step = None;
                        let mut move_down_step = None;
                        let mut delete_step = None;

                        egui::Grid::new("combo_steps_grid")
                            .num_columns(7)
                            .striped(true)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.strong("步骤");
                                ui.strong("按键");
                                ui.strong("间隔(ms)");
                                ui.strong("按下(ms)");
                                ui.strong("");
                                ui.strong("");
                                ui.strong("");
                                ui.end_row();

                                for index in 0..dialog.steps.len() {
                                    let key_text = display_key_name(&dialog.steps[index].key);
                                    let selected = dialog.selected_step == Some(index);
                                    if ui
                                        .add(
                                            egui::Button::new(format!("{}", index + 1))
                                                .selected(selected),
                                        )
                                        .clicked()
                                    {
                                        dialog.selected_step = Some(index);
                                    }
                                    let key_button_text = if dialog.capture_target
                                        == Some(ComboCaptureTarget::SelectedStep)
                                        && dialog.selected_step == Some(index)
                                    {
                                        "等待按键...".to_string()
                                    } else {
                                        key_text
                                    };
                                    if ui
                                        .add(egui::Button::new(key_button_text).selected(selected))
                                        .clicked()
                                    {
                                        dialog.selected_step = Some(index);
                                        start_capture_step = Some(index);
                                    }
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut dialog.steps[index].interval_ms,
                                        )
                                        .desired_width(80.0),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut dialog.steps[index].press_duration_ms,
                                        )
                                        .desired_width(80.0),
                                    );
                                    if ui.small_button("↑").clicked() && index > 0 {
                                        move_up_step = Some(index);
                                    }
                                    if ui.small_button("↓").clicked()
                                        && index + 1 < dialog.steps.len()
                                    {
                                        move_down_step = Some(index);
                                    }
                                    if ui.small_button("🗑").clicked() {
                                        delete_step = Some(index);
                                    }
                                    ui.end_row();
                                }
                            });

                        if let Some(index) = start_capture_step {
                            dialog.selected_step = Some(index);
                            start_combo_capture(&mut dialog, ComboCaptureTarget::SelectedStep);
                        }
                        if let Some(index) = move_up_step {
                            dialog.steps.swap(index, index - 1);
                            dialog.selected_step = Some(index - 1);
                        }
                        if let Some(index) = move_down_step {
                            dialog.steps.swap(index, index + 1);
                            dialog.selected_step = Some(index + 1);
                        }
                        if let Some(index) = delete_step {
                            dialog.steps.remove(index);
                            dialog.selected_step = if dialog.steps.is_empty() {
                                None
                            } else if index >= dialog.steps.len() {
                                Some(dialog.steps.len() - 1)
                            } else {
                                Some(index)
                            };
                        }
                    });

                ui.add_space(8.0);
                ui.label(
                    RichText::new("步骤按键可直接点列表重录；间隔和按下时长都可直接在列表内修改。")
                        .size(12.5)
                        .color(Color32::from_rgb(95, 100, 110)),
                );

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("保存连招").clicked() {
                        match self.state.save_combo_dialog(&dialog) {
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
            self.state.combo_dialog = Some(dialog);
        }
    }
}
