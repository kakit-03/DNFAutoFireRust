// Bootstraps the egui application and top-level event loop.

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
