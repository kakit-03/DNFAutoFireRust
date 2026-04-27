use super::*;

pub fn run(config_path: PathBuf, store: ConfigStore) -> Result<()> {
    let instance_guard = SingleInstanceGuard::acquire()?;
    let window_icon = load_window_icon(&project_asset_path("tp.png"))?;

    let native_options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DNFAutoFire 配置")
            .with_inner_size([MAIN_WINDOW_WIDTH as f32, MAIN_WINDOW_HEIGHT as f32])
            .with_resizable(false)
            .with_position([200.0, 120.0])
            .with_icon(window_icon),
        ..Default::default()
    };

    eframe::run_native(
        "DNFAutoFire 配置",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(EguiApp::new(
                cc,
                config_path,
                store,
                instance_guard,
            )?))
        }),
    )
    .map_err(|err| anyhow::anyhow!("failed to start egui application: {err}"))?;

    Ok(())
}

impl EguiApp {
    fn new(
        cc: &CreationContext<'_>,
        config_path: PathBuf,
        store: ConfigStore,
        instance_guard: SingleInstanceGuard,
    ) -> Result<Self> {
        configure_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let mut style = (*cc.egui_ctx.style()).clone();
        let mut scroll_style = egui::style::ScrollStyle::solid();
        scroll_style.bar_width = 14.0;
        scroll_style.handle_min_length = 24.0;
        scroll_style.bar_inner_margin = 1.0;
        scroll_style.foreground_color = true;
        style.spacing.scroll = scroll_style;
        cc.egui_ctx.set_style(style);

        let hwnd = hwnd_from_creation_context(cc).context("failed to get native window handle")?;
        let (event_tx, event_rx) = channel();
        let window_hidden_flag = Arc::new(AtomicBool::new(false));
        let tray = TrayResources::build(
            &cc.egui_ctx,
            hwnd,
            event_tx.clone(),
            Arc::clone(&window_hidden_flag),
        )?;
        let quick_switch_monitor = QuickSwitchMonitor::spawn(
            &cc.egui_ctx,
            hwnd,
            event_tx.clone(),
            Arc::clone(&window_hidden_flag),
        );

        let mut state = AppState::new(config_path, store);
        state.setup_initial_state()?;

        let mut app = Self {
            _instance_guard: instance_guard,
            hwnd,
            window_hidden_flag,
            state,
            tray,
            quick_switch_monitor,
            event_tx,
            event_rx,
        };
        app.sync_quick_switch_monitor();
        app.sync_tray_ui()?;
        if app.state.store.hide_gui_on_startup {
            app.hide_main_window_to_tray()?;
        }
        Ok(app)
    }

    fn process_logic(&mut self, ctx: &egui::Context) -> Result<()> {
        self.state.window_hidden = self.window_hidden_flag.load(Ordering::SeqCst);

        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::Runner(event) => self.state.handle_runner_event(event)?,
                AppEvent::TrayShowWindow => self.show_main_window()?,
                AppEvent::TrayHideWindow => self.hide_main_window_to_tray()?,
                AppEvent::TrayStartRunner => {
                    self.state.start_runner_from_form(&self.event_tx, ctx)?;
                    self.hide_main_window_to_tray()?;
                }
                AppEvent::TrayStopRunner => self.state.cancel_pending_start_and_request_stop(),
                AppEvent::HotkeyOpenSwitcher => self.show_switcher_window()?,
                AppEvent::TrayExit => {
                    self.state.quitting = true;
                    self.state.shutdown_runner();
                    self.show_main_window()?;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            if self.state.quitting {
                self.state.shutdown_runner();
            } else {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.hide_main_window_to_tray()?;
            }
        }

        let minimized = ctx.input(|input| input.viewport().minimized.unwrap_or(false));
        if minimized && !self.state.last_minimized && !self.state.window_hidden {
            self.hide_main_window_to_tray()?;
        }
        self.state.last_minimized = minimized;

        self.state.poll_quick_switch_hotkey_capture(ctx)?;
        self.handle_switcher_keyboard(ctx)?;
        self.state
            .try_start_pending_switch_profile(&self.event_tx, ctx)?;
        self.sync_quick_switch_monitor();
        self.sync_tray_ui()?;
        Ok(())
    }

    fn render_root(&mut self, ctx: &egui::Context) {
        if self.state.window_mode == WindowMode::Switcher {
            self.render_switcher_window(ctx);
            return;
        }

        TopBottomPanel::top("keyboard_panel")
            .exact_height(360.0)
            .resizable(false)
            .show(ctx, |ui| {
                self.render_keyboard_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_size();
            let left_width = (available.x * 0.40).clamp(400.0, 560.0);
            let right_width = 180.0;
            let spacing = 12.0;
            let middle_width =
                (available.x - left_width - right_width - spacing * 2.0).clamp(320.0, 480.0);
            let panel_height = available.y;

            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(left_width, panel_height),
                    Layout::top_down(Align::Min),
                    |ui| self.render_settings_panel(ui),
                );
                ui.add_space(spacing);
                ui.allocate_ui_with_layout(
                    Vec2::new(middle_width, panel_height),
                    Layout::top_down(Align::Min),
                    |ui| self.render_other_panel(ui),
                );
                ui.add_space(spacing);
                ui.allocate_ui_with_layout(
                    Vec2::new(right_width, panel_height),
                    Layout::top_down(Align::Min),
                    |ui| self.render_action_panel(ui),
                );
            });
        });

        self.render_combo_dialog(ctx);
        self.render_special_key_dialog(ctx);
    }

    fn render_switcher_window(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt("switcher_profile_list")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .max_height(340.0)
                    .show(ui, |ui| {
                        let names = self
                            .state
                            .store
                            .list_profile_names()
                            .into_iter()
                            .map(ToOwned::to_owned)
                            .collect::<Vec<_>>();
                        self.state.ensure_switcher_selection(&names);

                        for name in names {
                            let selected = self.state.switcher_selected_profile.as_deref()
                                == Some(name.as_str());
                            let response = ui.add(
                                egui::Button::new(name.as_str())
                                    .selected(selected)
                                    .min_size(Vec2::new(ui.available_width(), 34.0)),
                            );
                            if response.clicked() {
                                self.state.switcher_selected_profile = Some(name);
                            }
                        }
                    });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let has_selection = self.state.switcher_selected_profile.is_some();
                    if ui
                        .add_enabled(has_selection, egui::Button::new("切换并启动连发"))
                        .clicked()
                    {
                        match self
                            .state
                            .switch_profile_and_start_selected(&self.event_tx, ctx)
                        {
                            Ok(()) => {
                                if let Err(err) = self.hide_main_window_to_tray() {
                                    self.show_error(&format!("{err:#}"));
                                }
                            }
                            Err(err) => self.show_error(&format!("{err:#}")),
                        }
                    }
                    if ui.button("停止连发").clicked() {
                        self.state.cancel_pending_start_and_request_stop();
                        if let Err(err) = self.hide_main_window_to_tray() {
                            self.show_error(&format!("{err:#}"));
                        }
                    }
                });
            });
        });
    }

    fn render_settings_panel(&mut self, ui: &mut egui::Ui) {
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

    fn render_input_backend_selector(&mut self, ui: &mut egui::Ui, editable: bool) {
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

    fn render_other_panel(&mut self, ui: &mut egui::Ui) {
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

    fn render_action_panel(&mut self, ui: &mut egui::Ui) {
        let running = self.state.is_runner_active();
        let panel_width = ui.available_width();
        let sleep_timing = SleepTimingMonitor::shared().snapshot();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(panel_width);
            ui.heading("关于我们");
            ui.add_space(12.0);
            if ui
                .button("GitHub 仓库")
                .on_hover_text(PROJECT_GITHUB_URL)
                .clicked()
            {
                ui.ctx()
                    .open_url(egui::OpenUrl::new_tab(PROJECT_GITHUB_URL));
            }
            ui.add_space(8.0);
            let startup_hide_label = if self.state.store.hide_gui_on_startup {
                "启动时隐藏 GUI：开"
            } else {
                "启动时隐藏 GUI：关"
            };
            if ui.button(startup_hide_label).clicked()
                && let Err(err) = self.state.toggle_hide_gui_on_startup()
            {
                self.show_error(&format!("{err:#}"));
            }
        });
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(panel_width);
            ui.heading("运行控制");
            ui.add_space(12.0);

            if ui
                .add_enabled(
                    !running,
                    egui::Button::new("启动").min_size(Vec2::new(120.0, 40.0)),
                )
                .clicked()
            {
                match self.state.start_runner_from_form(&self.event_tx, ui.ctx()) {
                    Ok(()) => {
                        if let Err(err) = self.hide_main_window_to_tray() {
                            self.show_error(&format!("{err:#}"));
                        }
                    }
                    Err(err) => self.show_error(&format!("{err:#}")),
                }
            }

            ui.add_space(8.0);
            if ui
                .add_enabled(
                    running,
                    egui::Button::new("停止").min_size(Vec2::new(120.0, 40.0)),
                )
                .clicked()
            {
                self.state.cancel_pending_start_and_request_stop();
            }

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(12.0);
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            ui.label(
                RichText::new(format!(
                    "实际睡眠粒度: {} ms",
                    sleep_timing.measured_granularity_ms
                ))
                .strong(),
            );
            ui.small(format!(
                "高精度轮询间隔: {} ms",
                sleep_timing.scheduler_interval_ms
            ));
            ui.add_space(10.0);
            let status_color = match self.state.tray_state {
                TrayState::Disabled => Color32::from_rgb(170, 55, 55),
                TrayState::Enabled => Color32::from_rgb(40, 120, 70),
                TrayState::Paused => Color32::from_rgb(185, 125, 25),
            };
            ui.label(
                RichText::new(self.state.status_text)
                    .strong()
                    .color(status_color),
            );
        });
    }

    fn sync_tray_ui(&mut self) -> Result<()> {
        let (tip, icon, status_text) = match self.state.tray_state {
            TrayState::Disabled => (
                "DNFAutoFire - 连发已关闭",
                &self.tray.disabled_icon,
                "状态: 连发已关闭",
            ),
            TrayState::Enabled => (
                "DNFAutoFire - 连发已开启",
                &self.tray.enabled_icon,
                "状态: 连发已开启",
            ),
            TrayState::Paused => (
                "DNFAutoFire - 输入法暂停",
                &self.tray.paused_icon,
                "状态: 输入法暂停",
            ),
        };

        self.tray.status_item.set_text(status_text);
        self.tray.show_item.set_enabled(self.state.window_hidden);
        self.tray.hide_item.set_enabled(!self.state.window_hidden);

        let running = self.state.tray_state != TrayState::Disabled;
        self.tray.start_item.set_enabled(!running);
        self.tray.stop_item.set_enabled(running);

        self.tray
            .tray
            .set_icon(Some(icon.clone()))
            .context("failed to update tray icon")?;
        self.tray
            .tray
            .set_tooltip(Some(tip))
            .context("failed to update tray tooltip")?;

        Ok(())
    }

    fn sync_quick_switch_monitor(&self) {
        self.quick_switch_monitor
            .set_config(self.state.quick_switch_watch_config());
    }

    fn resize_window(&self, width: i32, height: i32) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND::default(),
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER,
            );
        }
    }

    fn show_main_window(&mut self) -> Result<()> {
        self.state.cancel_pending_start_and_request_stop();
        self.state.window_mode = WindowMode::Main;
        self.state.combo_dialog = None;
        self.state.special_key_dialog = None;
        self.state.stop_quick_switch_hotkey_capture();
        self.resize_window(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT);
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.state.window_hidden = false;
        self.window_hidden_flag.store(false, Ordering::SeqCst);
        self.state.last_minimized = false;
        self.sync_tray_ui()
    }

    fn show_switcher_window(&mut self) -> Result<()> {
        self.state.window_mode = WindowMode::Switcher;
        self.state.combo_dialog = None;
        self.state.special_key_dialog = None;
        self.state.stop_quick_switch_hotkey_capture();
        let names = self
            .state
            .store
            .list_profile_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        self.state.ensure_switcher_selection(&names);
        self.resize_window(SWITCHER_WINDOW_WIDTH, SWITCHER_WINDOW_HEIGHT);
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.state.window_hidden = false;
        self.window_hidden_flag.store(false, Ordering::SeqCst);
        self.state.last_minimized = false;
        self.sync_tray_ui()
    }

    fn hide_main_window_to_tray(&mut self) -> Result<()> {
        self.state.window_hidden = true;
        self.window_hidden_flag.store(true, Ordering::SeqCst);
        self.state.last_minimized = false;
        self.state.combo_dialog = None;
        self.state.special_key_dialog = None;
        self.state.stop_quick_switch_hotkey_capture();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.sync_tray_ui()
    }

    fn handle_switcher_keyboard(&mut self, ctx: &egui::Context) -> Result<()> {
        if self.state.window_mode != WindowMode::Switcher || self.state.window_hidden {
            return Ok(());
        }

        if ctx.input(|input| input.key_pressed(Key::ArrowUp)) {
            self.state.move_switcher_selection(-1);
        }
        if ctx.input(|input| input.key_pressed(Key::ArrowDown)) {
            self.state.move_switcher_selection(1);
        }
        if ctx.input(|input| input.key_pressed(Key::Enter)) {
            self.state
                .switch_profile_and_start_selected(&self.event_tx, ctx)?;
            self.hide_main_window_to_tray()?;
        }
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.hide_main_window_to_tray()?;
        }

        Ok(())
    }

    pub(super) fn show_error(&self, message: &str) {
        unsafe {
            let _ = MessageBoxW(
                self.hwnd,
                &HSTRING::from(message),
                w!("操作失败"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

impl App for EguiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        if let Err(err) = self.process_logic(ctx) {
            self.show_error(&format!("{err:#}"));
        }
        self.render_root(ctx);
    }
}

impl Drop for EguiApp {
    fn drop(&mut self) {
        self.state.shutdown_runner();
        self.quick_switch_monitor.stop();
    }
}
