// Renders the quick profile switcher window.

use super::*;

impl EguiApp {
    pub(super) fn render_switcher_window(&mut self, ctx: &egui::Context) {
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
}
