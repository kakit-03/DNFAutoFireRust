// Renders profile and global settings panels.

use super::*;

impl EguiApp {
    pub(super) fn render_settings_panel(&mut self, ui: &mut egui::Ui) {
        let editable = self.state.is_editing_enabled();

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading("配置设置");
            ui.add_space(6.0);

            let full_width = ui.available_width();
            let top_height = 200.0;
            let list_width = 190.0;
            let split_spacing = 10.0;
            let right_width = (full_width - list_width - split_spacing).max(260.0);
            let top_panel_inner_height = top_height - 18.0;
            let profile_list_scroll_height =
                (top_panel_inner_height - PROFILE_LIST_ITEM_HEIGHT * 1.5).max(80.0);

            ui.allocate_ui_with_layout(
                Vec2::new(full_width, top_height),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(list_width, top_height),
                            Layout::top_down(Align::Min),
                            |ui| {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.set_min_height(top_panel_inner_height);
                                    ui.strong("配置列表");
                                    ui.add_space(6.0);
                                    ui.set_min_width(list_width);
                                    ScrollArea::vertical()
                                        .id_salt("profile_list_scroll")
                                        .scroll_bar_visibility(
                                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                        )
                                        .max_height(profile_list_scroll_height)
                                        .show(ui, |ui| {
                                            let names = self
                                                .state
                                                .store
                                                .list_profile_names()
                                                .into_iter()
                                                .map(ToOwned::to_owned)
                                                .collect::<Vec<_>>();

                                            for name in names {
                                                let selected =
                                                    self.state.current_profile_key.as_deref()
                                                        == Some(name.as_str());
                                                let response = ui.add_enabled(
                                                    editable,
                                                    egui::Button::new(name.as_str())
                                                        .selected(selected)
                                                        .min_size(Vec2::new(
                                                            list_width - 20.0,
                                                            32.0,
                                                        )),
                                                );
                                                if response.clicked() {
                                                    if let Err(err) =
                                                        self.state.load_profile_from_store(&name)
                                                    {
                                                        self.show_error(&format!("{err:#}"));
                                                    }
                                                }
                                            }
                                        });
                                });
                            },
                        );

                        ui.add_space(split_spacing);

                        ui.allocate_ui_with_layout(
                            Vec2::new(right_width, top_height),
                            Layout::top_down(Align::Min),
                            |ui| {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.set_min_height(top_panel_inner_height);
                                    ui.strong("配置内容");
                                    ui.add_space(6.0);

                                    ui.label("配置名");
                                    ui.add_enabled(
                                        editable,
                                        egui::TextEdit::singleline(&mut self.state.draft.name)
                                            .desired_width(f32::INFINITY),
                                    );

                                    ui.add_space(8.0);
                                    egui::Grid::new("timing_grid")
                                        .num_columns(2)
                                        .spacing([10.0, 8.0])
                                        .show(ui, |ui| {
                                            ui.label("连发间隔(ms)");
                                            ui.add_enabled(
                                                editable,
                                                egui::TextEdit::singleline(
                                                    &mut self.state.draft.repeat_interval_ms,
                                                ),
                                            );
                                            ui.end_row();

                                            ui.label("按下时长(ms)");
                                            ui.add_enabled(
                                                editable,
                                                egui::TextEdit::singleline(
                                                    &mut self.state.draft.press_duration_ms,
                                                ),
                                            );
                                            ui.end_row();
                                        });

                                    ui.add_space(6.0);
                                    ui.small("轮询间隔会根据实际睡眠粒度自动计算。");

                                    ui.add_space(12.0);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add_enabled(editable, egui::Button::new("保存"))
                                            .clicked()
                                        {
                                            if let Err(err) = self.state.save_profile() {
                                                self.show_error(&format!("{err:#}"));
                                            }
                                        }
                                        if ui
                                            .add_enabled(editable, egui::Button::new("克隆配置"))
                                            .clicked()
                                        {
                                            if let Err(err) = self.state.clone_profile() {
                                                self.show_error(&format!("{err:#}"));
                                            }
                                        }
                                        if ui
                                            .add_enabled(editable, egui::Button::new("删除"))
                                            .clicked()
                                        {
                                            if let Err(err) = self.state.delete_profile() {
                                                self.show_error(&format!("{err:#}"));
                                            }
                                        }
                                        if ui
                                            .add_enabled(editable, egui::Button::new("新建"))
                                            .clicked()
                                        {
                                            if let Err(err) = self.state.new_profile() {
                                                self.show_error(&format!("{err:#}"));
                                            }
                                        }
                                    });
                                });
                            },
                        );
                    });
                },
            );

            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                Vec2::new(full_width, TARGET_WINDOWS_SECTION_HEIGHT),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_height(TARGET_WINDOWS_SECTION_HEIGHT - 18.0);
                        ui.strong("目标窗口关键字");
                        ui.add_space(6.0);
                        let response = ui.add_enabled_ui(editable, |ui| {
                            ScrollArea::vertical()
                                .id_salt("global_target_windows_scroll")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                )
                                .max_height(TARGET_WINDOWS_INPUT_HEIGHT)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(
                                            &mut self.state.global_target_windows_text,
                                        )
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(6),
                                    )
                                })
                                .inner
                        });
                        if editable
                            && response.inner.changed()
                            && let Err(err) = self.state.persist_global_target_windows_if_valid()
                        {
                            self.show_error(&format!("{err:#}"));
                        }

                        ui.add_space(12.0);
                        ui.strong("快速切换热键");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_enabled(
                                false,
                                egui::TextEdit::singleline(
                                    &mut self.state.global_quick_switch_hotkey,
                                )
                                .desired_width(240.0),
                            );
                            let capture_label = if self.state.quick_switch_hotkey_capturing {
                                "等待组合键..."
                            } else {
                                "录入热键"
                            };
                            if ui
                                .add_enabled(editable, egui::Button::new(capture_label))
                                .clicked()
                            {
                                if self.state.quick_switch_hotkey_capturing {
                                    self.state.stop_quick_switch_hotkey_capture();
                                } else {
                                    self.state.start_quick_switch_hotkey_capture();
                                }
                            }
                            if ui
                                .add_enabled(editable, egui::Button::new("清空"))
                                .clicked()
                            {
                                self.state.stop_quick_switch_hotkey_capture();
                                self.state.global_quick_switch_hotkey.clear();
                            }
                        });
                        let hotkey_tip = if self.state.quick_switch_hotkey_capturing {
                            "正在录入组合键，请先按修饰键，再按一次主键即可完成录入，无需一直按住。"
                        } else {
                            "此处只显示当前全局热键；请点“录入热键”后直接按组合键。"
                        };
                        ui.label(
                            RichText::new(hotkey_tip)
                                .size(12.5)
                                .color(Color32::from_rgb(95, 100, 110)),
                        );

                        ui.add_space(12.0);
                        ui.strong("输入后端");
                        ui.add_space(6.0);
                        self.render_input_backend_selector(ui, editable);
                    });
                },
            );
        });
    }

    pub(super) fn render_input_backend_selector(&mut self, ui: &mut egui::Ui, editable: bool) {
        let current = self.state.store.input_backend;
        let mut selected = None;
        let current_descriptor = input_backend_descriptor(current);

        ui.add_enabled_ui(editable, |ui| {
            egui::ComboBox::from_id_salt("input_backend_selector")
                .selected_text(input_backend_label(current))
                .width(ui.available_width().min(280.0))
                .show_ui(ui, |ui| {
                    for descriptor in input_backend_descriptors() {
                        let response = ui.add_enabled(
                            descriptor.available,
                            egui::Button::new(descriptor.label)
                                .selected(current == descriptor.kind)
                                .min_size(Vec2::new(ui.available_width(), 26.0)),
                        );
                        let response = if let Some(reason) = descriptor.unavailable_reason {
                            response.on_hover_text(reason)
                        } else {
                            response.on_hover_text(descriptor.description)
                        };
                        if response.clicked() {
                            selected = Some(descriptor.kind);
                        }
                    }
                });
        });

        if let Some(kind) = selected
            && let Err(err) = self.state.set_input_backend(kind)
        {
            self.show_error(&format!("{err:#}"));
        }

        ui.label(
            RichText::new(current_descriptor.description)
                .size(12.5)
                .color(Color32::from_rgb(95, 100, 110)),
        );
        if let Some(reason) = current_descriptor.unavailable_reason {
            ui.colored_label(Color32::from_rgb(170, 55, 55), reason);
        }
        if !editable {
            ui.label(
                RichText::new("连发运行中，停止后可切换输入后端。")
                    .size(12.5)
                    .color(Color32::from_rgb(95, 100, 110)),
            );
        }
    }
}
