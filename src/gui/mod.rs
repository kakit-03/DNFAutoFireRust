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

mod app;
mod assets;
#[path = "dialogs/combo.rs"]
mod combo_dialog;
mod keyboard;
pub(crate) mod model;
mod quick_switch;
#[path = "dialogs/special_key.rs"]
mod special_key_dialog;
mod state;
mod tray;

pub use app::run;
use assets::{
    configure_fonts, hwnd_from_creation_context, load_tray_icon_base, load_window_icon,
    project_asset_path, tray_icon_with_status_dot,
};

const STATUS_IDLE: &str = "状态: 未运行";
const STATUS_RUNNING: &str = "状态: 运行中";
const STATUS_IME_PAUSED: &str = "状态: 输入法暂停";
const STATUS_STOPPED: &str = "状态: 已停止";
const PROJECT_GITHUB_URL: &str = "https://github.com/kakit-03/DNFAutoFireRust";
const APP_ICON_SIZE: u32 = 256;
const TRAY_ICON_SIZE: u32 = 32;
const MAIN_WINDOW_WIDTH: i32 = 1300;
const MAIN_WINDOW_HEIGHT: i32 = 870;
const SWITCHER_WINDOW_WIDTH: i32 = 340;
const SWITCHER_WINDOW_HEIGHT: i32 = 430;
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

fn hotkey_modifiers(bits: u32) -> u32 {
    let mut modifiers = 0u32;
    if bits & 0x0001 != 0 {
        modifiers |= MOD_ALT.0;
    }
    if bits & 0x0002 != 0 {
        modifiers |= MOD_CONTROL.0;
    }
    if bits & 0x0004 != 0 {
        modifiers |= MOD_SHIFT.0;
    }
    if bits & 0x0008 != 0 {
        modifiers |= MOD_WIN.0;
    }
    modifiers
}

fn special_key_dialog_from_config(
    index: usize,
    config: &SpecialKeyConfig,
) -> SpecialKeyDialogState {
    match config {
        SpecialKeyConfig::CustomAutofire {
            name,
            key,
            repeat_interval_ms,
            press_duration_ms,
        } => SpecialKeyDialogState {
            edit_index: Some(index),
            config_type: SpecialKeyType::CustomAutofire,
            name: name.clone(),
            custom_key: key.clone(),
            auto_trigger_key: "未录入".to_string(),
            auto_trigger_hotkey: "未录入".to_string(),
            linked_trigger_key: "未录入".to_string(),
            linked_target_key: "未录入".to_string(),
            linked_trigger_mode: LinkedTriggerMode::Press,
            repeat_interval_ms: repeat_interval_ms.to_string(),
            press_duration_ms: press_duration_ms.to_string(),
            linked_interval_ms: DEFAULT_COMBO_STEP_INTERVAL_MS.to_string(),
            linked_press_duration_ms: DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string(),
            capture_target: None,
            capture_down_keys: HashSet::new(),
        },
        SpecialKeyConfig::AutoTrigger {
            name,
            key,
            trigger_hotkey,
            repeat_interval_ms,
            press_duration_ms,
        } => SpecialKeyDialogState {
            edit_index: Some(index),
            config_type: SpecialKeyType::AutoTrigger,
            name: name.clone(),
            custom_key: "未录入".to_string(),
            auto_trigger_key: key.clone(),
            auto_trigger_hotkey: display_hotkey_text(trigger_hotkey),
            linked_trigger_key: "未录入".to_string(),
            linked_target_key: "未录入".to_string(),
            linked_trigger_mode: LinkedTriggerMode::Press,
            repeat_interval_ms: repeat_interval_ms.to_string(),
            press_duration_ms: press_duration_ms.to_string(),
            linked_interval_ms: DEFAULT_COMBO_STEP_INTERVAL_MS.to_string(),
            linked_press_duration_ms: DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string(),
            capture_target: None,
            capture_down_keys: HashSet::new(),
        },
        SpecialKeyConfig::LinkedKey {
            name,
            trigger_key,
            linked_key,
            trigger_mode,
            interval_ms,
            press_duration_ms,
        } => SpecialKeyDialogState {
            edit_index: Some(index),
            config_type: SpecialKeyType::LinkedKey,
            name: name.clone(),
            custom_key: "未录入".to_string(),
            auto_trigger_key: "未录入".to_string(),
            auto_trigger_hotkey: "未录入".to_string(),
            linked_trigger_key: trigger_key.clone(),
            linked_target_key: linked_key.clone(),
            linked_trigger_mode: *trigger_mode,
            repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS.to_string(),
            press_duration_ms: DEFAULT_PRESS_DURATION_MS.to_string(),
            linked_interval_ms: interval_ms.to_string(),
            linked_press_duration_ms: press_duration_ms.to_string(),
            capture_target: None,
            capture_down_keys: HashSet::new(),
        },
    }
}

fn start_combo_capture(dialog: &mut ComboDialogState, target: ComboCaptureTarget) {
    dialog.capture_target = Some(target);
    dialog.capture_down_keys = currently_pressed_supported_keys();
}

fn start_special_key_capture(dialog: &mut SpecialKeyDialogState, target: SpecialKeyCaptureTarget) {
    dialog.capture_target = Some(target);
    dialog.capture_down_keys = currently_pressed_supported_keys();
}

fn combo_capture_message(target: Option<ComboCaptureTarget>) -> Option<&'static str> {
    match target {
        Some(ComboCaptureTarget::Trigger) => Some("正在录入触发键，请按下一个有效按键。"),
        Some(ComboCaptureTarget::NewStep) => Some("正在录入新步骤，请按下一个有效按键。"),
        Some(ComboCaptureTarget::ContinuousSteps) => {
            Some("正在连续录入步骤，请持续按键；点“停止连续录入”即可结束。")
        }
        Some(ComboCaptureTarget::SelectedStep) => {
            Some("正在重新录入步骤按键，请按下一个有效按键。")
        }
        None => None,
    }
}

fn special_key_capture_message(target: Option<SpecialKeyCaptureTarget>) -> Option<&'static str> {
    match target {
        Some(SpecialKeyCaptureTarget::CustomKey) => {
            Some("正在录入独立连发键位，请按下一个有效按键。")
        }
        Some(SpecialKeyCaptureTarget::AutoTriggerKey) => {
            Some("正在录入自动触发键位，请按下一个有效按键。")
        }
        Some(SpecialKeyCaptureTarget::AutoTriggerHotkey) => {
            Some("正在录入自动触发热键，请先按修饰键，再按一次主键即可完成录入。")
        }
        Some(SpecialKeyCaptureTarget::LinkedTriggerKey) => {
            Some("正在录入连携触发键，请按下一个有效按键。")
        }
        Some(SpecialKeyCaptureTarget::LinkedTargetKey) => {
            Some("正在录入连携键，请按下一个有效按键。")
        }
        None => None,
    }
}

fn special_key_kind_label(config: &SpecialKeyConfig) -> &'static str {
    match config {
        SpecialKeyConfig::CustomAutofire { .. } => "独立连发",
        SpecialKeyConfig::AutoTrigger { .. } => "自动触发",
        SpecialKeyConfig::LinkedKey { .. } => "连携键位",
    }
}

fn capture_next_supported_key(
    previously_down: &HashSet<String>,
) -> (HashSet<String>, Option<String>) {
    let mut current_down = HashSet::new();
    let mut seen_vk = HashSet::new();
    let mut captured = None;

    for name in supported_key_names() {
        let Ok(spec) = parse_single_key(name) else {
            continue;
        };
        if !seen_vk.insert((spec.vk, spec.extended)) {
            continue;
        }
        if is_vk_down(spec.vk) {
            current_down.insert(spec.name.to_string());
            if captured.is_none() && !previously_down.contains(spec.name) {
                captured = Some(spec.name.to_string());
            }
        }
    }

    (current_down, captured)
}

fn currently_pressed_supported_keys() -> HashSet<String> {
    capture_next_supported_key(&HashSet::new()).0
}

fn parse_dialog_ms(raw: &str, label: &str) -> Result<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("{label}不能为空");
    }
    let value = trimmed.parse::<u64>()?;
    if value == 0 {
        bail!("{label}必须 >= 1");
    }
    Ok(value)
}

fn strip_clone_suffix(name: &str) -> Option<&str> {
    let (prefix, suffix) = name.rsplit_once("-cloned-")?;
    if prefix.is_empty() || suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(prefix)
}

#[cfg(test)]
mod tests {
    use super::{
        AppState, ComboCaptureTarget, ComboDialogState, ComboStepDraft, SpecialKeyDialogState,
        SpecialKeyType,
    };
    use crate::config::{ConfigStore, LinkedTriggerMode, SpecialKeyConfig};
    use crate::input::backend::InputBackendKind;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn save_combo_dialog_persists_to_config_file() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");

        let dialog = ComboDialogState {
            edit_index: None,
            name: "burst".to_string(),
            trigger_key: "U".to_string(),
            steps: vec![
                ComboStepDraft {
                    key: "A".to_string(),
                    interval_ms: "80".to_string(),
                    press_duration_ms: "2".to_string(),
                },
                ComboStepDraft {
                    key: "S".to_string(),
                    interval_ms: "90".to_string(),
                    press_duration_ms: "3".to_string(),
                },
            ],
            selected_step: None,
            new_step_interval_ms: "80".to_string(),
            new_step_press_duration_ms: "1".to_string(),
            capture_target: Some(ComboCaptureTarget::ContinuousSteps),
            capture_down_keys: HashSet::new(),
        };

        state
            .save_combo_dialog(&dialog)
            .expect("save combo dialog should persist");

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        let combo = &saved.profiles["default"].combos[0];
        assert_eq!(combo.name, "burst");
        assert_eq!(combo.trigger_key, "U");
        assert_eq!(combo.steps.len(), 2);
        assert_eq!(combo.steps[0].key, "A");
        assert_eq!(combo.steps[0].interval_ms, 80);
        assert_eq!(combo.steps[0].press_duration_ms, 2);
        assert_eq!(combo.steps[1].key, "S");
        assert_eq!(combo.steps[1].interval_ms, 90);
        assert_eq!(combo.steps[1].press_duration_ms, 3);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_special_key_dialog_persists_to_config_file() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");

        let dialog = SpecialKeyDialogState {
            edit_index: None,
            config_type: SpecialKeyType::AutoTrigger,
            name: "auto-j".to_string(),
            custom_key: "未录入".to_string(),
            auto_trigger_key: "J".to_string(),
            auto_trigger_hotkey: "LAlt+Q".to_string(),
            linked_trigger_key: "未录入".to_string(),
            linked_target_key: "未录入".to_string(),
            linked_trigger_mode: LinkedTriggerMode::Press,
            repeat_interval_ms: "30".to_string(),
            press_duration_ms: "2".to_string(),
            linked_interval_ms: "80".to_string(),
            linked_press_duration_ms: "1".to_string(),
            capture_target: None,
            capture_down_keys: HashSet::new(),
        };

        state
            .save_special_key_dialog(&dialog)
            .expect("save special key dialog should persist");

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        let special = &saved.profiles["default"].special_keys[0];
        match special {
            SpecialKeyConfig::AutoTrigger {
                name,
                key,
                trigger_hotkey,
                repeat_interval_ms,
                press_duration_ms,
            } => {
                assert_eq!(name, "auto-j");
                assert_eq!(key, "J");
                assert_eq!(trigger_hotkey, "LALT+Q");
                assert_eq!(*repeat_interval_ms, 30);
                assert_eq!(*press_duration_ms, 2);
            }
            _ => panic!("expected auto trigger special key"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn clone_profile_uses_incrementing_cloned_suffix() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");

        state.clone_profile().expect("first clone should save");
        assert_eq!(
            state.current_profile_key.as_deref(),
            Some("default-cloned-1")
        );

        state
            .load_profile_from_store("default")
            .expect("reload original profile");
        state.clone_profile().expect("second clone should save");
        assert_eq!(
            state.current_profile_key.as_deref(),
            Some("default-cloned-2")
        );

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        assert!(saved.profiles.contains_key("default-cloned-1"));
        assert!(saved.profiles.contains_key("default-cloned-2"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn remember_last_started_profile_updates_default_for_saved_selection() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");
        state
            .save_profile()
            .expect("initial profile should save cleanly");

        state.new_profile().expect("create new profile");
        state.draft.name = "raid".to_string();
        state
            .save_profile()
            .expect("second profile should save cleanly");
        state
            .load_profile_from_store("raid")
            .expect("load saved profile");

        state
            .remember_last_started_profile()
            .expect("remember last started profile");

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        assert_eq!(saved.default_profile, "raid");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn remember_last_started_profile_ignores_unsaved_rename() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");
        state
            .save_profile()
            .expect("initial profile should save cleanly");

        state.draft.name = "draft-renamed".to_string();
        state
            .remember_last_started_profile()
            .expect("unsaved rename should be ignored");

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        assert_eq!(saved.default_profile, "default");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn quick_switch_watch_config_uses_global_hotkey_and_current_target_windows() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");

        state.global_quick_switch_hotkey = "LAlt+Q".to_string();
        state.global_target_windows_text = "DNF\r\n地下城与勇士".to_string();

        let watch = state.quick_switch_watch_config();
        let hotkey = watch.hotkey.expect("hotkey should be registerable");

        assert_eq!(hotkey.modifiers, 0x0001);
        assert_eq!(hotkey.vk, 0x51);
        assert_eq!(watch.target_windows, vec!["DNF", "地下城与勇士"]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn persist_global_quick_switch_hotkey_saves_immediately() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");

        state.global_quick_switch_hotkey = "LAlt+;".to_string();
        state
            .persist_global_quick_switch_hotkey()
            .expect("global quick switch hotkey should persist");

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        assert_eq!(saved.quick_switch_hotkey, "LALT+SEMICOLON");
        assert_eq!(state.global_quick_switch_hotkey, "LAlt+;");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn persist_global_target_windows_saves_immediately() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");

        state.global_target_windows_text = "地下城与勇士\r\nDNF".to_string();
        state
            .persist_global_target_windows()
            .expect("global target windows should persist");

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        assert_eq!(saved.target_windows, vec!["地下城与勇士", "DNF"]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn input_backend_selection_persists_and_rejects_unavailable_backends() {
        let path = unique_test_config_path();
        let mut state = AppState::new(path.clone(), ConfigStore::default());
        state.setup_initial_state().expect("setup initial state");

        state
            .set_input_backend(InputBackendKind::SendInputPolling)
            .expect("default backend should be selectable");
        assert!(
            state
                .set_input_backend(InputBackendKind::MessageBackend)
                .is_err()
        );

        let saved = ConfigStore::load_or_create(&path).expect("load saved config");
        assert_eq!(saved.input_backend, InputBackendKind::SendInputPolling);

        let _ = fs::remove_file(path);
    }

    fn unique_test_config_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("dnf-gui-test-{nanos}.json"))
    }
}
