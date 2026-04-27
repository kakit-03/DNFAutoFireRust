use crate::platform::window::{foreground_window_info, foreground_window_is};
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, SW_RESTORE, SWP_NOMOVE, SWP_NOZORDER,
    SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage, WM_HOTKEY,
};

use super::{
    AppEvent, QUICK_SWITCH_POLL_INTERVAL, QuickSwitchMonitor, QuickSwitchWatchConfig,
    SWITCHER_WINDOW_HEIGHT, SWITCHER_WINDOW_WIDTH, hotkey_modifiers,
};
impl QuickSwitchMonitor {
    pub(super) fn spawn(
        ctx: &egui::Context,
        hwnd: HWND,
        event_tx: Sender<AppEvent>,
        window_hidden_flag: Arc<AtomicBool>,
    ) -> Self {
        let config = Arc::new(Mutex::new(QuickSwitchWatchConfig::default()));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let join_config = Arc::clone(&config);
        let join_stop = Arc::clone(&stop_flag);
        let join_ctx = ctx.clone();
        let join_hidden = Arc::clone(&window_hidden_flag);
        let hwnd_raw = hwnd.0 as isize;

        let join = thread::spawn(move || {
            let hotkey_id = 0xD1FA;
            let mut active_config = QuickSwitchWatchConfig::default();
            let mut registered = false;
            while !join_stop.load(Ordering::SeqCst) {
                let snapshot = join_config
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default();
                if snapshot != active_config {
                    if registered {
                        unsafe {
                            let _ = UnregisterHotKey(HWND::default(), hotkey_id);
                        }
                        registered = false;
                    }

                    if let Some(hotkey) = snapshot.hotkey {
                        let modifiers =
                            HOT_KEY_MODIFIERS(hotkey_modifiers(hotkey.modifiers) | MOD_NOREPEAT.0);
                        registered = unsafe {
                            RegisterHotKey(HWND::default(), hotkey_id, modifiers, hotkey.vk)
                        }
                        .is_ok();
                    }
                    active_config = snapshot.clone();
                }

                let mut msg = MSG::default();
                while unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) }.as_bool()
                {
                    if msg.message == WM_HOTKEY {
                        if foreground_window_is(HWND(hwnd_raw as _)) {
                            continue;
                        }

                        let is_target = foreground_window_info().as_ref().is_some_and(|info| {
                            info.matches_any_target(&active_config.target_windows)
                        });
                        if !is_target {
                            continue;
                        }

                        unsafe {
                            let hwnd = HWND(hwnd_raw as _);
                            let _ = SetWindowPos(
                                hwnd,
                                HWND::default(),
                                0,
                                0,
                                SWITCHER_WINDOW_WIDTH,
                                SWITCHER_WINDOW_HEIGHT,
                                SWP_NOMOVE | SWP_NOZORDER,
                            );
                            let _ = ShowWindow(hwnd, SW_RESTORE);
                            let _ = SetForegroundWindow(hwnd);
                        }
                        join_hidden.store(false, Ordering::SeqCst);
                        let _ = event_tx.send(AppEvent::HotkeyOpenSwitcher);
                        join_ctx.request_repaint();
                    } else {
                        unsafe {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }

                sleep(QUICK_SWITCH_POLL_INTERVAL);
            }

            if registered {
                unsafe {
                    let _ = UnregisterHotKey(HWND::default(), hotkey_id);
                }
            }
        });

        Self {
            config,
            stop_flag,
            join: Some(join),
        }
    }

    pub(super) fn set_config(&self, config: QuickSwitchWatchConfig) {
        if let Ok(mut guard) = self.config.lock() {
            *guard = config;
        }
    }

    pub(super) fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
