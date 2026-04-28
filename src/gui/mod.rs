// Wires GUI modules, shared state types, and common constants.

use crate::autofire::{AutoFireService, RunnerEvent, RunnerHandle};
use crate::config::{
    ComboConfig, ComboStepConfig, ConfigStore, DEFAULT_COMBO_STEP_INTERVAL_MS,
    DEFAULT_COMBO_STEP_PRESS_DURATION_MS, DEFAULT_PRESS_DURATION_MS, DEFAULT_REPEAT_INTERVAL_MS,
    LinkedTriggerMode, Profile, SpecialKeyConfig,
};
use crate::gui::model::{ProfileDraft, target_windows_from_text, target_windows_to_text};
use crate::input::backend::{
    InputBackendKind, input_backend_descriptor, input_backend_descriptors, input_backend_label,
};
use crate::input::is_vk_down;
use crate::keymap::{
    HotkeyRegistration, display_hotkey_names, display_hotkey_text, display_key_name,
    hotkey_registration, is_modifier_key, normalize_hotkey_text, parse_hotkey, parse_key_specs,
    parse_single_key, sort_hotkey_names, supported_key_names,
};
use crate::platform::single_instance::SingleInstanceGuard;
use crate::timing::SleepTimingMonitor;
use anyhow::{Context, Result, bail};
use eframe::egui::{
    self, Align, Align2, Color32, Key, Layout, RichText, ScrollArea, TopBottomPanel, Vec2,
    ViewportCommand,
};
use eframe::{App, CreationContext, Frame, NativeOptions};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tray_icon::menu::MenuItem;
use tray_icon::{Icon, TrayIcon};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MessageBoxW, SW_HIDE, SW_RESTORE, SWP_NOMOVE, SWP_NOZORDER,
    SetForegroundWindow, SetWindowPos, ShowWindow,
};
use windows::core::{HSTRING, w};

#[path = "app/action_panel.rs"]
mod action_panel;
mod app;
mod assets;
#[path = "dialogs/combo.rs"]
mod combo_dialog;
mod helpers;
mod keyboard;
pub(crate) mod model;
#[path = "app/other_panel.rs"]
mod other_panel;
mod quick_switch;
#[path = "app/settings_panel.rs"]
mod settings_panel;
#[path = "dialogs/special_key.rs"]
mod special_key_dialog;
mod state;
#[path = "state/combo.rs"]
mod state_combo;
#[path = "state/runner.rs"]
mod state_runner;
#[path = "state/special_key.rs"]
mod state_special_key;
#[path = "app/switcher_panel.rs"]
mod switcher_panel;
mod tray;
#[path = "app/window.rs"]
mod window;

pub use app::run;
use assets::{
    configure_fonts, hwnd_from_creation_context, load_tray_icon_base, load_window_icon,
    project_asset_path, tray_icon_with_status_dot,
};
use helpers::*;

const STATUS_IDLE: &str = "状态: 未运行";
const STATUS_RUNNING: &str = "状态: 运行中";
const STATUS_IME_PAUSED: &str = "状态: 输入法暂停";
const STATUS_STOPPED: &str = "状态: 已停止";
const PROJECT_GITHUB_URL: &str = "https://github.com/kakit-03/DNFAutoFireRust";
const APP_ICON_SIZE: u32 = 256;
const TRAY_ICON_SIZE: u32 = 32;
const MAIN_WINDOW_WIDTH: i32 = 1300;
const MAIN_WINDOW_HEIGHT: i32 = 870;
const SWITCHER_WINDOW_WIDTH: i32 = MAIN_WINDOW_WIDTH;
const SWITCHER_WINDOW_HEIGHT: i32 = MAIN_WINDOW_HEIGHT;
const QUICK_SWITCH_POLL_INTERVAL: Duration = Duration::from_millis(30);

const PROFILE_LIST_ITEM_HEIGHT: f32 = 32.0;
const TARGET_WINDOWS_INPUT_HEIGHT: f32 = 110.0;
const TARGET_WINDOWS_SECTION_HEIGHT: f32 = 200.0;
const OTHER_CONFIG_VISIBLE_ROWS: f32 = 3.0;
const OTHER_CONFIG_ROW_GAP: f32 = 6.0;
const OTHER_CONFIG_LIST_HEIGHT: f32 =
    PROFILE_LIST_ITEM_HEIGHT * OTHER_CONFIG_VISIBLE_ROWS + OTHER_CONFIG_ROW_GAP * 2.0;
const COMBO_NAME_COLUMN_WIDTH: f32 = 120.0;
const COMBO_TRIGGER_COLUMN_WIDTH: f32 = 70.0;
const COMBO_STEP_COUNT_COLUMN_WIDTH: f32 = 52.0;
const SPECIAL_KEY_NAME_COLUMN_WIDTH: f32 = 110.0;
const SPECIAL_KEY_TYPE_COLUMN_WIDTH: f32 = 110.0;

const TRAY_SHOW_ID: &str = "tray.show";
const TRAY_HIDE_ID: &str = "tray.hide";
const TRAY_START_ID: &str = "tray.start";
const TRAY_STOP_ID: &str = "tray.stop";
const TRAY_EXIT_ID: &str = "tray.exit";

const FONT_CANDIDATES: &[&str] = &["simhei.ttf", "msyh.ttf", "msyh.ttc", "simsun.ttc"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayState {
    Disabled,
    Enabled,
    Paused,
}

enum AppEvent {
    Runner(RunnerEvent),
    TrayShowWindow,
    TrayHideWindow,
    TrayStartRunner,
    TrayStopRunner,
    HotkeyOpenSwitcher,
    TrayExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowMode {
    Main,
    Switcher,
}

#[derive(Clone)]
struct ComboStepDraft {
    key: String,
    interval_ms: String,
    press_duration_ms: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComboCaptureTarget {
    Trigger,
    NewStep,
    ContinuousSteps,
    SelectedStep,
}

#[derive(Clone)]
struct ComboDialogState {
    edit_index: Option<usize>,
    name: String,
    trigger_key: String,
    steps: Vec<ComboStepDraft>,
    selected_step: Option<usize>,
    new_step_interval_ms: String,
    new_step_press_duration_ms: String,
    capture_target: Option<ComboCaptureTarget>,
    capture_down_keys: HashSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpecialKeyType {
    CustomAutofire,
    AutoTrigger,
    LinkedKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpecialKeyCaptureTarget {
    CustomKey,
    AutoTriggerKey,
    AutoTriggerHotkey,
    LinkedTriggerKey,
    LinkedTargetKey,
}

#[derive(Clone)]
struct SpecialKeyDialogState {
    edit_index: Option<usize>,
    config_type: SpecialKeyType,
    name: String,
    custom_key: String,
    auto_trigger_key: String,
    auto_trigger_hotkey: String,
    linked_trigger_key: String,
    linked_target_key: String,
    linked_trigger_mode: LinkedTriggerMode,
    repeat_interval_ms: String,
    press_duration_ms: String,
    linked_interval_ms: String,
    linked_press_duration_ms: String,
    capture_target: Option<SpecialKeyCaptureTarget>,
    capture_down_keys: HashSet<String>,
}

struct AppState {
    config_path: PathBuf,
    store: ConfigStore,
    current_profile_key: Option<String>,
    draft: ProfileDraft,
    global_target_windows_text: String,
    enabled_keys: HashSet<String>,
    tray_state: TrayState,
    window_hidden: bool,
    quitting: bool,
    runner: Option<RunnerHandle>,
    selected_combo_index: Option<usize>,
    combo_dialog: Option<ComboDialogState>,
    selected_special_key_index: Option<usize>,
    special_key_dialog: Option<SpecialKeyDialogState>,
    status_text: &'static str,
    last_minimized: bool,
    window_mode: WindowMode,
    switcher_selected_profile: Option<String>,
    global_quick_switch_hotkey: String,
    quick_switch_hotkey_capturing: bool,
    quick_switch_hotkey_down_keys: HashSet<String>,
    pending_switch_start_profile: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct QuickSwitchWatchConfig {
    hotkey: Option<HotkeyRegistration>,
    target_windows: Vec<String>,
}

struct QuickSwitchMonitor {
    config: Arc<Mutex<QuickSwitchWatchConfig>>,
    stop_flag: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

struct TrayResources {
    tray: TrayIcon,
    status_item: MenuItem,
    show_item: MenuItem,
    hide_item: MenuItem,
    start_item: MenuItem,
    stop_item: MenuItem,
    enabled_icon: Icon,
    paused_icon: Icon,
    disabled_icon: Icon,
}

struct EguiApp {
    _instance_guard: SingleInstanceGuard,
    hwnd: HWND,
    window_hidden_flag: Arc<AtomicBool>,
    state: AppState,
    tray: TrayResources,
    quick_switch_monitor: QuickSwitchMonitor,
    event_tx: Sender<AppEvent>,
    event_rx: Receiver<AppEvent>,
}

#[cfg(test)]
mod tests;
