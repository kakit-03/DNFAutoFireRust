// Synchronizes native window, tray, and switcher visibility state.

use super::*;

impl EguiApp {
    pub(super) fn sync_tray_ui(&mut self) -> Result<()> {
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

    pub(super) fn sync_quick_switch_monitor(&self) {
        self.quick_switch_monitor
            .set_config(self.state.quick_switch_watch_config());
    }

    pub(super) fn resize_window(&self, width: i32, height: i32) {
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

    pub(super) fn show_main_window(&mut self) -> Result<()> {
        self.state.stop_runner_for_editing()?;
        self.state.window_mode = WindowMode::Main;
        self.state.combo_dialog = None;
        self.state.special_key_dialog = None;
        self.state.stop_quick_switch_hotkey_capture();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
        }
        self.resize_window(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT);
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.state.window_hidden = false;
        self.window_hidden_flag.store(false, Ordering::SeqCst);
        self.state.last_minimized = false;
        self.sync_tray_ui()
    }

    pub(super) fn show_switcher_window(&mut self) -> Result<()> {
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
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
        }
        self.resize_window(SWITCHER_WINDOW_WIDTH, SWITCHER_WINDOW_HEIGHT);
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.state.window_hidden = false;
        self.window_hidden_flag.store(false, Ordering::SeqCst);
        self.state.last_minimized = false;
        self.sync_tray_ui()
    }

    pub(super) fn hide_main_window_to_tray(&mut self) -> Result<()> {
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

    pub(super) fn handle_switcher_keyboard(&mut self, ctx: &egui::Context) -> Result<()> {
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
