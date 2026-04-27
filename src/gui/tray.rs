use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SW_HIDE, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

use super::{
    AppEvent, TRAY_EXIT_ID, TRAY_HIDE_ID, TRAY_SHOW_ID, TRAY_START_ID, TRAY_STOP_ID, TrayResources,
    load_tray_icon_base, project_asset_path, tray_icon_with_status_dot,
};
use anyhow::{Context, Result};
impl TrayResources {
    pub(super) fn build(
        ctx: &egui::Context,
        hwnd: HWND,
        event_tx: Sender<AppEvent>,
        window_hidden_flag: Arc<AtomicBool>,
    ) -> Result<Self> {
        let hwnd_raw = hwnd.0 as isize;
        let tray_base = load_tray_icon_base(&project_asset_path("tp.png"))?;
        let enabled_icon = tray_icon_with_status_dot(&tray_base, [73, 173, 84, 255])?;
        let paused_icon = tray_icon_with_status_dot(&tray_base, [235, 184, 62, 255])?;
        let disabled_icon = tray_icon_with_status_dot(&tray_base, [219, 78, 78, 255])?;

        let status_item = MenuItem::new("状态: 连发已关闭", false, None);
        let show_item = MenuItem::with_id(TRAY_SHOW_ID, "显示窗口", true, None);
        let hide_item = MenuItem::with_id(TRAY_HIDE_ID, "隐藏窗口", false, None);
        let start_item = MenuItem::with_id(TRAY_START_ID, "启动连发", true, None);
        let stop_item = MenuItem::with_id(TRAY_STOP_ID, "停止连发", false, None);
        let exit_item = MenuItem::with_id(TRAY_EXIT_ID, "退出", true, None);

        let menu = Menu::new();
        menu.append(&status_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&show_item)?;
        menu.append(&hide_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&start_item)?;
        menu.append(&stop_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&exit_item)?;

        let tray = TrayIconBuilder::new()
            .with_icon(disabled_icon.clone())
            .with_tooltip("DNFAutoFire - 连发已关闭")
            .with_menu(Box::new(menu))
            .build()
            .context("failed to create tray icon")?;
        tray.set_show_menu_on_left_click(false);

        let menu_tx = event_tx.clone();
        let menu_ctx = ctx.clone();
        let menu_hidden_flag = Arc::clone(&window_hidden_flag);
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            match event.id.0.as_str() {
                TRAY_SHOW_ID => {
                    unsafe {
                        let hwnd = HWND(hwnd_raw as _);
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                        let _ = SetForegroundWindow(hwnd);
                    }
                    menu_hidden_flag.store(false, Ordering::SeqCst);
                    let _ = menu_tx.send(AppEvent::TrayShowWindow);
                }
                TRAY_HIDE_ID => {
                    unsafe {
                        let hwnd = HWND(hwnd_raw as _);
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                    menu_hidden_flag.store(true, Ordering::SeqCst);
                    let _ = menu_tx.send(AppEvent::TrayHideWindow);
                }
                TRAY_START_ID => {
                    let _ = menu_tx.send(AppEvent::TrayStartRunner);
                }
                TRAY_STOP_ID => {
                    let _ = menu_tx.send(AppEvent::TrayStopRunner);
                }
                TRAY_EXIT_ID => {
                    unsafe {
                        let hwnd = HWND(hwnd_raw as _);
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                        let _ = SetForegroundWindow(hwnd);
                    }
                    menu_hidden_flag.store(false, Ordering::SeqCst);
                    let _ = menu_tx.send(AppEvent::TrayExit);
                }
                _ => {}
            }
            menu_ctx.request_repaint();
        }));

        let tray_tx = event_tx;
        let tray_ctx = ctx.clone();
        let tray_hidden_flag = window_hidden_flag;
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if tray_hidden_flag.load(Ordering::SeqCst) {
                    unsafe {
                        let hwnd = HWND(hwnd_raw as _);
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                        let _ = SetForegroundWindow(hwnd);
                    }
                    tray_hidden_flag.store(false, Ordering::SeqCst);
                    let _ = tray_tx.send(AppEvent::TrayShowWindow);
                } else {
                    unsafe {
                        let hwnd = HWND(hwnd_raw as _);
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                    tray_hidden_flag.store(true, Ordering::SeqCst);
                    let _ = tray_tx.send(AppEvent::TrayHideWindow);
                }
                tray_ctx.request_repaint();
            }
        }));

        Ok(Self {
            tray,
            status_item,
            show_item,
            hide_item,
            start_item,
            stop_item,
            enabled_icon,
            paused_icon,
            disabled_icon,
        })
    }
}
