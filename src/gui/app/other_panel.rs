// Renders combo and special-key management panels.

use super::*;

impl EguiApp {
    pub(super) fn render_other_panel(&mut self, ui: &mut egui::Ui) {
        let editable = self.state.is_editing_enabled();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading("其他配置");
            ui.add_space(6.0);

            ScrollArea::vertical()
                .id_salt("other_panel_scroll")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.strong("一键连招");
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(editable, egui::Button::new("新增连招"))
                            .clicked()
                        {
                            self.state.open_combo_dialog(None);
                        }
                        if ui
                            .add_enabled(editable, egui::Button::new("编辑连招"))
                            .clicked()
                        {
                            if let Err(err) = self.state.open_selected_combo_dialog() {
                                self.show_error(&format!("{err:#}"));
                            }
                        }
                        if ui
                            .add_enabled(editable, egui::Button::new("删除连招"))
                            .clicked()
                        {
                            if let Err(err) = self.state.remove_selected_combo() {
                                self.show_error(&format!("{err:#}"));
                            }
                        }
                    });

                    ui.add_space(6.0);
                    egui::Grid::new("combo_header_grid")
                        .num_columns(3)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.add_sized(
                                [COMBO_NAME_COLUMN_WIDTH, PROFILE_LIST_ITEM_HEIGHT],
                                egui::Label::new(RichText::new("名称").strong()),
                            );
                            ui.add_sized(
                                [COMBO_TRIGGER_COLUMN_WIDTH, PROFILE_LIST_ITEM_HEIGHT],
                                egui::Label::new(RichText::new("触发键").strong()),
                            );
                            ui.add_sized(
                                [COMBO_STEP_COUNT_COLUMN_WIDTH, PROFILE_LIST_ITEM_HEIGHT],
                                egui::Label::new(RichText::new("步骤数").strong()),
                            );
                            ui.end_row();
                        });

                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), OTHER_CONFIG_LIST_HEIGHT),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_height(OTHER_CONFIG_LIST_HEIGHT);
                            ScrollArea::vertical()
                                .id_salt("combo_list_scroll")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                )
                                .max_height(OTHER_CONFIG_LIST_HEIGHT)
                                .show(ui, |ui| {
                                    egui::Grid::new("combo_body_grid")
                                        .num_columns(3)
                                        .striped(true)
                                        .spacing([12.0, 6.0])
                                        .show(ui, |ui| {
                                            for (index, combo) in
                                                self.state.draft.combos.iter().enumerate()
                                            {
                                                let selected =
                                                    self.state.selected_combo_index == Some(index);
                                                if ui
                                                    .add(
                                                        egui::Button::new(&combo.name)
                                                            .selected(selected)
                                                            .min_size(Vec2::new(
                                                                COMBO_NAME_COLUMN_WIDTH,
                                                                PROFILE_LIST_ITEM_HEIGHT,
                                                            )),
                                                    )
                                                    .clicked()
                                                {
                                                    self.state.selected_combo_index = Some(index);
                                                }
                                                ui.add_sized(
                                                    [
                                                        COMBO_TRIGGER_COLUMN_WIDTH,
                                                        PROFILE_LIST_ITEM_HEIGHT,
                                                    ],
                                                    egui::Label::new(display_key_name(
                                                        &combo.trigger_key,
                                                    )),
                                                );
                                                ui.add_sized(
                                                    [
                                                        COMBO_STEP_COUNT_COLUMN_WIDTH,
                                                        PROFILE_LIST_ITEM_HEIGHT,
                                                    ],
                                                    egui::Label::new(format!(
                                                        "{} 步",
                                                        combo.steps.len()
                                                    )),
                                                );
                                                ui.end_row();
                                            }
                                        });
                                });
                        },
                    );

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(12.0);
                    ui.strong("特殊键位配置");
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(editable, egui::Button::new("新增配置"))
                            .clicked()
                        {
                            self.state.open_special_key_dialog(None);
                        }
                        if ui
                            .add_enabled(editable, egui::Button::new("编辑配置"))
                            .clicked()
                        {
                            if let Err(err) = self.state.open_selected_special_key_dialog() {
                                self.show_error(&format!("{err:#}"));
                            }
                        }
                        if ui
                            .add_enabled(editable, egui::Button::new("删除配置"))
                            .clicked()
                        {
                            if let Err(err) = self.state.remove_selected_special_key() {
                                self.show_error(&format!("{err:#}"));
                            }
                        }
                    });

                    ui.add_space(6.0);
                    egui::Grid::new("special_key_grid")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.add_sized(
                                [SPECIAL_KEY_NAME_COLUMN_WIDTH, PROFILE_LIST_ITEM_HEIGHT],
                                egui::Label::new(RichText::new("名称").strong()),
                            );
                            ui.add_sized(
                                [SPECIAL_KEY_TYPE_COLUMN_WIDTH, PROFILE_LIST_ITEM_HEIGHT],
                                egui::Label::new(RichText::new("类型").strong()),
                            );
                            ui.end_row();
                        });

                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), OTHER_CONFIG_LIST_HEIGHT),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_height(OTHER_CONFIG_LIST_HEIGHT);
                            ScrollArea::vertical()
                                .id_salt("special_key_list_scroll")
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                )
                                .max_height(OTHER_CONFIG_LIST_HEIGHT)
                                .show(ui, |ui| {
                                    egui::Grid::new("special_key_body_grid")
                                        .num_columns(2)
                                        .striped(true)
                                        .spacing([12.0, 6.0])
                                        .show(ui, |ui| {
                                            for (index, special) in
                                                self.state.draft.special_keys.iter().enumerate()
                                            {
                                                let selected =
                                                    self.state.selected_special_key_index
                                                        == Some(index);
                                                if ui
                                                    .add(
                                                        egui::Button::new(special.name())
                                                            .selected(selected)
                                                            .min_size(Vec2::new(
                                                                SPECIAL_KEY_NAME_COLUMN_WIDTH,
                                                                PROFILE_LIST_ITEM_HEIGHT,
                                                            )),
                                                    )
                                                    .clicked()
                                                {
                                                    self.state.selected_special_key_index =
                                                        Some(index);
                                                }
                                                ui.add_sized(
                                                    [
                                                        SPECIAL_KEY_TYPE_COLUMN_WIDTH,
                                                        PROFILE_LIST_ITEM_HEIGHT,
                                                    ],
                                                    egui::Label::new(special_key_kind_label(
                                                        special,
                                                    )),
                                                );
                                                ui.end_row();
                                            }
                                        });
                                });
                        },
                    );
                });
        });
    }
}
