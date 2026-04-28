// Renders runner action controls and status text.

use super::*;

impl EguiApp {
    pub(super) fn render_action_panel(&mut self, ui: &mut egui::Ui) {
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
}
