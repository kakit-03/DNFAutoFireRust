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

impl AppState {
    fn new(config_path: PathBuf, store: ConfigStore) -> Self {
        let default_draft = ProfileDraft::from_named_profile("default", &Profile::default());
        let global_quick_switch_hotkey = display_hotkey_text(&store.quick_switch_hotkey);
        let global_target_windows_text = target_windows_to_text(&store.target_windows);
        let mut state = Self {
            config_path,
            store,
            current_profile_key: None,
            draft: default_draft,
            global_target_windows_text,
            enabled_keys: HashSet::new(),
            tray_state: TrayState::Disabled,
            window_hidden: false,
            quitting: false,
            runner: None,
            selected_combo_index: None,
            combo_dialog: None,
            selected_special_key_index: None,
            special_key_dialog: None,
            status_text: STATUS_IDLE,
            last_minimized: false,
            window_mode: WindowMode::Main,
            switcher_selected_profile: None,
            global_quick_switch_hotkey,
            quick_switch_hotkey_capturing: false,
            quick_switch_hotkey_down_keys: HashSet::new(),
            pending_switch_start_profile: None,
        };
        state.sync_enabled_keys(&state.draft.enabled_keys.clone());
        state
    }

    fn setup_initial_state(&mut self) -> Result<()> {
        let default_name = self.store.default_profile.clone();
        self.load_profile_from_store(&default_name)?;
        self.tray_state = TrayState::Disabled;
        self.status_text = STATUS_IDLE;
        Ok(())
    }

    fn is_editing_enabled(&self) -> bool {
        !self.is_runner_active()
    }

    fn is_runner_active(&self) -> bool {
        self.runner
            .as_ref()
            .map(RunnerHandle::is_running)
            .unwrap_or(false)
    }

    fn load_profile_from_store(&mut self, name: &str) -> Result<()> {
        let profile = self
            .store
            .get_profile(Some(name))
            .with_context(|| format!("配置不存在: {name}"))?;
        let draft = ProfileDraft::from_named_profile(name, &profile);
        self.current_profile_key = Some(name.to_string());
        self.draft = draft.clone();
        self.sync_enabled_keys(&draft.enabled_keys);
        self.selected_combo_index = None;
        self.combo_dialog = None;
        self.selected_special_key_index = None;
        self.special_key_dialog = None;
        self.status_text = STATUS_IDLE;
        self.tray_state = TrayState::Disabled;
        self.switcher_selected_profile = Some(name.to_string());
        Ok(())
    }

    fn sync_enabled_keys(&mut self, enabled_keys: &[String]) {
        self.enabled_keys.clear();
        for key in enabled_keys {
            if let Ok(spec) = parse_single_key(key) {
                self.enabled_keys.insert(spec.name.to_string());
            }
        }
    }

    fn toggle_enabled_key(&mut self, token: &str) {
        if !self.enabled_keys.remove(token) {
            self.enabled_keys.insert(token.to_string());
        }
    }

    fn build_draft_from_form(&self) -> ProfileDraft {
        let mut draft = self.draft.clone();
        draft.enabled_keys = self.selected_enabled_keys();
        draft
    }

    fn current_target_windows(&self) -> Result<Vec<String>> {
        let target_windows = target_windows_from_text(&self.global_target_windows_text);
        if target_windows.is_empty() {
            bail!("请至少填写一个目标窗口关键字");
        }
        Ok(target_windows)
    }

    fn selected_enabled_keys(&self) -> Vec<String> {
        supported_key_names()
            .iter()
            .filter(|name| self.enabled_keys.contains(**name))
            .map(|name| (*name).to_string())
            .collect()
    }

    fn validate_draft(&self, draft: &ProfileDraft) -> Result<(String, Profile)> {
        let (name, profile) = draft.to_named_profile()?;
        let normalized_quick_switch_hotkey =
            normalize_hotkey_text(&self.global_quick_switch_hotkey)?;
        if !profile.enabled_keys.is_empty() {
            parse_key_specs(&profile.enabled_keys)?;
        }
        for combo in &profile.combos {
            parse_single_key(&combo.trigger_key)?;
            for step in &combo.steps {
                parse_single_key(&step.key)?;
            }
        }
        for special in &profile.special_keys {
            match special {
                SpecialKeyConfig::CustomAutofire { key, .. } => {
                    parse_single_key(key)?;
                }
                SpecialKeyConfig::AutoTrigger {
                    key,
                    trigger_hotkey,
                    ..
                } => {
                    parse_single_key(key)?;
                    let normalized_trigger_hotkey = normalize_hotkey_text(trigger_hotkey)?;
                    parse_hotkey(&normalized_trigger_hotkey)?;
                    if normalized_trigger_hotkey == normalized_quick_switch_hotkey {
                        bail!("自动触发热键不能与全局快速切换热键冲突");
                    }
                }
                SpecialKeyConfig::LinkedKey {
                    trigger_key,
                    linked_key,
                    ..
                } => {
                    parse_single_key(trigger_key)?;
                    parse_single_key(linked_key)?;
                }
            }
        }
        Ok((name, profile))
    }

    fn new_profile(&mut self) -> Result<()> {
        let name = self.generate_profile_name();
        let draft = ProfileDraft::from_named_profile(&name, &Profile::default());
        self.current_profile_key = None;
        self.draft = draft.clone();
        self.sync_enabled_keys(&draft.enabled_keys);
        self.selected_combo_index = None;
        self.combo_dialog = None;
        self.selected_special_key_index = None;
        self.special_key_dialog = None;
        self.status_text = STATUS_IDLE;
        self.quick_switch_hotkey_capturing = false;
        self.quick_switch_hotkey_down_keys.clear();
        Ok(())
    }

    fn save_profile(&mut self) -> Result<()> {
        let draft = self.build_draft_from_form();
        let (new_name, profile) = self.validate_draft(&draft)?;
        let original_name = self.current_profile_key.clone();

        if let Some(original_name) = &original_name {
            if original_name != &new_name && self.store.profiles.contains_key(&new_name) {
                bail!("已存在同名配置: {new_name}");
            }
        } else if self.store.profiles.contains_key(&new_name) {
            bail!("已存在同名配置: {new_name}");
        }

        let renamed_default = original_name
            .as_ref()
            .map(|name| name == &self.store.default_profile && name != &new_name)
            .unwrap_or(false);

        if let Some(original_name) = original_name {
            if original_name != new_name {
                self.store.profiles.remove(&original_name);
            }
        }

        self.store.upsert_profile(new_name.clone(), profile.clone());
        if renamed_default {
            self.store.default_profile = new_name.clone();
        }
        self.store.save(&self.config_path)?;

        self.current_profile_key = Some(new_name.clone());
        self.draft = ProfileDraft::from_named_profile(&new_name, &profile);
        self.status_text = STATUS_STOPPED;
        self.switcher_selected_profile = Some(new_name);
        Ok(())
    }

    fn clone_profile(&mut self) -> Result<()> {
        let mut draft = self.build_draft_from_form();
        let clone_name = self.generate_cloned_profile_name();
        draft.name = clone_name.clone();
        let (name, profile) = self.validate_draft(&draft)?;

        self.store.upsert_profile(name.clone(), profile.clone());
        self.store.save(&self.config_path)?;

        self.current_profile_key = Some(name.clone());
        self.draft = ProfileDraft::from_named_profile(&name, &profile);
        self.sync_enabled_keys(&self.draft.enabled_keys.clone());
        self.selected_combo_index = None;
        self.combo_dialog = None;
        self.selected_special_key_index = None;
        self.special_key_dialog = None;
        self.status_text = STATUS_STOPPED;
        self.tray_state = TrayState::Disabled;
        self.switcher_selected_profile = Some(name);
        Ok(())
    }

    fn delete_profile(&mut self) -> Result<()> {
        let current_key = self
            .current_profile_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("当前是未保存的新配置，不能直接删除"))?;
        if self.draft.name.trim() != current_key {
            bail!("当前配置名已修改，请先保存后再删除");
        }

        self.store.delete_profile(&current_key)?;
        self.store.save(&self.config_path)?;

        let next_name = self.store.default_profile.clone();
        self.load_profile_from_store(&next_name)?;
        self.status_text = STATUS_STOPPED;
        Ok(())
    }

    fn start_runner_from_form(
        &mut self,
        event_tx: &Sender<AppEvent>,
        ctx: &egui::Context,
    ) -> Result<()> {
        if self.is_runner_active() {
            bail!("连发已经在运行");
        }

        let draft = self.build_draft_from_form();
        let (_, profile) = self.validate_draft(&draft)?;
        let target_windows = self.current_target_windows()?;
        let input_backend = self.store.input_backend;
        let backend_settings = self.store.backend_settings.clone();
        let tx = event_tx.clone();
        let repaint_ctx = ctx.clone();
        let mut handle = AutoFireService::start_with_backend_events(
            profile,
            target_windows,
            input_backend,
            backend_settings,
            move |event| {
                let _ = tx.send(AppEvent::Runner(event));
                repaint_ctx.request_repaint();
            },
        )?;
        if let Err(err) = self.remember_last_started_profile() {
            handle.stop();
            let _ = handle.wait();
            return Err(err);
        }

        self.runner = Some(handle);
        self.status_text = STATUS_RUNNING;
        self.tray_state = TrayState::Enabled;
        Ok(())
    }

    fn request_stop_runner(&self) {
        if let Some(runner) = self.runner.as_ref().filter(|runner| runner.is_running()) {
            runner.stop();
        }
    }

    fn cancel_pending_start_and_request_stop(&mut self) {
        self.pending_switch_start_profile = None;
        self.request_stop_runner();
        self.status_text = STATUS_STOPPED;
        self.tray_state = TrayState::Disabled;
    }

    fn start_quick_switch_hotkey_capture(&mut self) {
        self.quick_switch_hotkey_capturing = true;
        self.quick_switch_hotkey_down_keys = currently_pressed_supported_keys();
    }

    fn stop_quick_switch_hotkey_capture(&mut self) {
        self.quick_switch_hotkey_capturing = false;
        self.quick_switch_hotkey_down_keys.clear();
    }

    fn poll_quick_switch_hotkey_capture(&mut self, ctx: &egui::Context) -> Result<()> {
        if !self.quick_switch_hotkey_capturing {
            return Ok(());
        }

        ctx.request_repaint_after(Duration::from_millis(16));
        let (current_down, captured_key) =
            capture_next_supported_key(&self.quick_switch_hotkey_down_keys);
        self.quick_switch_hotkey_down_keys = current_down.clone();

        let Some(captured_key) = captured_key else {
            return Ok(());
        };
        if is_modifier_key(&captured_key) {
            return Ok(());
        }

        let ordered = sort_hotkey_names(current_down.into_iter().collect::<Vec<_>>());
        self.global_quick_switch_hotkey = display_hotkey_names(ordered);
        self.stop_quick_switch_hotkey_capture();
        self.persist_global_quick_switch_hotkey()?;
        Ok(())
    }

    fn persist_global_quick_switch_hotkey(&mut self) -> Result<()> {
        let normalized = normalize_hotkey_text(&self.global_quick_switch_hotkey)?;
        self.store.quick_switch_hotkey = normalized.clone();
        self.store.save(&self.config_path)?;
        self.global_quick_switch_hotkey = display_hotkey_text(&normalized);
        Ok(())
    }

    #[cfg(test)]
    fn persist_global_target_windows(&mut self) -> Result<()> {
        let target_windows = self.current_target_windows()?;
        self.store.target_windows = target_windows;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    fn set_input_backend(&mut self, input_backend: InputBackendKind) -> Result<()> {
        let descriptor = input_backend_descriptor(input_backend);
        if !descriptor.available {
            bail!(
                "输入后端 '{}' 暂不可用: {}",
                descriptor.label,
                descriptor
                    .unavailable_reason
                    .unwrap_or("当前版本尚未实现该后端")
            );
        }

        self.store.input_backend = input_backend;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    fn persist_global_target_windows_if_valid(&mut self) -> Result<()> {
        let target_windows = target_windows_from_text(&self.global_target_windows_text);
        if target_windows.is_empty() {
            return Ok(());
        }
        self.store.target_windows = target_windows;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    fn toggle_hide_gui_on_startup(&mut self) -> Result<()> {
        self.store.hide_gui_on_startup = !self.store.hide_gui_on_startup;
        self.store.save(&self.config_path)?;
        Ok(())
    }

    fn remember_last_started_profile(&mut self) -> Result<()> {
        let Some(current_key) = self.current_profile_key.clone() else {
            return Ok(());
        };
        if self.draft.name.trim() != current_key {
            return Ok(());
        }

        self.store.remember_started_profile(&current_key)?;
        self.store.save(&self.config_path)?;
        self.switcher_selected_profile = Some(current_key);
        Ok(())
    }

    fn ensure_switcher_selection(&mut self, names: &[String]) {
        if names.is_empty() {
            self.switcher_selected_profile = None;
            return;
        }

        let current = self
            .switcher_selected_profile
            .clone()
            .filter(|name| names.iter().any(|candidate| candidate == name));
        self.switcher_selected_profile =
            current.or_else(|| Some(self.store.default_profile.clone()));
        if !names
            .iter()
            .any(|candidate| Some(candidate.as_str()) == self.switcher_selected_profile.as_deref())
        {
            self.switcher_selected_profile = names.first().cloned();
        }
    }

    fn move_switcher_selection(&mut self, step: isize) {
        let names = self
            .store
            .list_profile_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if names.is_empty() {
            self.switcher_selected_profile = None;
            return;
        }
        self.ensure_switcher_selection(&names);

        let current_index = names
            .iter()
            .position(|name| Some(name.as_str()) == self.switcher_selected_profile.as_deref())
            .unwrap_or(0) as isize;
        let len = names.len() as isize;
        let next_index = (current_index + step).rem_euclid(len) as usize;
        self.switcher_selected_profile = Some(names[next_index].clone());
    }

    fn switch_profile_and_start_selected(
        &mut self,
        event_tx: &Sender<AppEvent>,
        ctx: &egui::Context,
    ) -> Result<()> {
        let selected = self
            .switcher_selected_profile
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先选择一个配置"))?;
        if self.is_runner_active() {
            self.pending_switch_start_profile = Some(selected);
            self.request_stop_runner();
            return Ok(());
        }
        self.load_profile_from_store(&selected)?;
        self.start_runner_from_form(event_tx, ctx)
    }

    fn quick_switch_watch_config(&self) -> QuickSwitchWatchConfig {
        let target_windows = target_windows_from_text(&self.global_target_windows_text);
        if target_windows.is_empty() {
            return QuickSwitchWatchConfig::default();
        }

        let Ok(hotkey) = hotkey_registration(&self.global_quick_switch_hotkey) else {
            return QuickSwitchWatchConfig::default();
        };

        QuickSwitchWatchConfig {
            hotkey: Some(hotkey),
            target_windows,
        }
    }

    fn try_start_pending_switch_profile(
        &mut self,
        event_tx: &Sender<AppEvent>,
        ctx: &egui::Context,
    ) -> Result<()> {
        if self.runner.is_some() {
            return Ok(());
        }

        let Some(profile_name) = self.pending_switch_start_profile.take() else {
            return Ok(());
        };

        self.load_profile_from_store(&profile_name)?;
        self.start_runner_from_form(event_tx, ctx)
    }

    fn handle_runner_event(&mut self, event: RunnerEvent) -> Result<()> {
        match event {
            RunnerEvent::Started | RunnerEvent::ResumedFromIme => {
                self.status_text = STATUS_RUNNING;
                self.tray_state = TrayState::Enabled;
            }
            RunnerEvent::PausedByIme => {
                self.status_text = STATUS_IME_PAUSED;
                self.tray_state = TrayState::Paused;
            }
            RunnerEvent::Stopped(_) => {
                if let Some(mut runner) = self.runner.take() {
                    runner.wait()?;
                }
                self.status_text = STATUS_STOPPED;
                self.tray_state = TrayState::Disabled;
            }
        }
        Ok(())
    }

    fn shutdown_runner(&mut self) {
        if let Some(runner) = self.runner.as_ref() {
            runner.stop();
        }
        if let Some(mut runner) = self.runner.take() {
            let _ = runner.wait();
        }
    }

    fn open_combo_dialog(&mut self, edit_index: Option<usize>) {
        self.combo_dialog = Some(if let Some(index) = edit_index {
            let combo = self.draft.combos[index].clone();
            let last_step_interval = combo
                .steps
                .last()
                .map(|step| step.interval_ms.to_string())
                .unwrap_or_else(|| DEFAULT_COMBO_STEP_INTERVAL_MS.to_string());
            let last_step_press = combo
                .steps
                .last()
                .map(|step| step.press_duration_ms.to_string())
                .unwrap_or_else(|| DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string());
            ComboDialogState {
                edit_index: Some(index),
                name: combo.name,
                trigger_key: combo.trigger_key,
                steps: combo
                    .steps
                    .into_iter()
                    .map(|step| ComboStepDraft {
                        key: step.key,
                        interval_ms: step.interval_ms.to_string(),
                        press_duration_ms: step.press_duration_ms.to_string(),
                    })
                    .collect(),
                selected_step: None,
                new_step_interval_ms: last_step_interval,
                new_step_press_duration_ms: last_step_press,
                capture_target: None,
                capture_down_keys: HashSet::new(),
            }
        } else {
            ComboDialogState {
                edit_index: None,
                name: self.generate_combo_name(),
                trigger_key: "未录入".to_string(),
                steps: Vec::new(),
                selected_step: None,
                new_step_interval_ms: DEFAULT_COMBO_STEP_INTERVAL_MS.to_string(),
                new_step_press_duration_ms: DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string(),
                capture_target: None,
                capture_down_keys: HashSet::new(),
            }
        });
    }

    fn open_selected_combo_dialog(&mut self) -> Result<()> {
        let Some(index) = self.selected_combo_index else {
            bail!("请先选中一个连招");
        };
        self.open_combo_dialog(Some(index));
        Ok(())
    }

    fn remove_selected_combo(&mut self) -> Result<()> {
        let Some(index) = self.selected_combo_index else {
            bail!("请先选中一个连招");
        };
        self.draft.combos.remove(index);
        self.selected_combo_index = None;
        self.persist_combo_changes()?;
        Ok(())
    }

    fn save_combo_dialog(&mut self, dialog: &ComboDialogState) -> Result<()> {
        let combo = self.read_combo_from_dialog(dialog)?;
        for (index, existing) in self.draft.combos.iter().enumerate() {
            if Some(index) != dialog.edit_index && existing.name.eq_ignore_ascii_case(&combo.name) {
                bail!("已存在同名连招: {}", combo.name);
            }
        }

        if let Some(index) = dialog.edit_index {
            self.draft.combos[index] = combo;
            self.selected_combo_index = Some(index);
        } else {
            self.draft.combos.push(combo);
            self.selected_combo_index = Some(self.draft.combos.len().saturating_sub(1));
        }
        self.persist_combo_changes()?;
        Ok(())
    }

    fn persist_combo_changes(&mut self) -> Result<()> {
        if self.current_profile_key.is_none()
            || self
                .current_profile_key
                .as_deref()
                .is_some_and(|name| self.draft.name.trim() != name)
        {
            return self.save_profile();
        }

        let current_key = self
            .current_profile_key
            .clone()
            .expect("current_profile_key checked above");
        let mut profile = self
            .store
            .get_profile(Some(&current_key))
            .with_context(|| format!("配置不存在: {current_key}"))?;
        profile.combos = self.draft.combos.clone();
        profile.special_keys = self.draft.special_keys.clone();
        profile = profile.normalized();
        profile.validate()?;
        self.validate_draft(&ProfileDraft::from_named_profile(&current_key, &profile))?;

        self.store.upsert_profile(current_key, profile);
        self.store.save(&self.config_path)?;
        self.status_text = STATUS_STOPPED;
        Ok(())
    }

    fn read_combo_from_dialog(&self, dialog: &ComboDialogState) -> Result<ComboConfig> {
        if dialog.trigger_key.trim().is_empty() || dialog.trigger_key == "未录入" {
            bail!("请先录入触发键");
        }
        if dialog.steps.is_empty() {
            bail!("请至少录入一个步骤");
        }

        let steps = dialog
            .steps
            .iter()
            .map(|step| {
                Ok(ComboStepConfig {
                    key: step.key.clone(),
                    interval_ms: parse_dialog_ms(&step.interval_ms, "步骤间隔")?,
                    press_duration_ms: parse_dialog_ms(&step.press_duration_ms, "步骤按下时长")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let combo = ComboConfig {
            name: dialog.name.clone(),
            trigger_key: dialog.trigger_key.clone(),
            steps,
            sequence_keys: Vec::new(),
            step_interval_ms: 0,
            press_duration_ms: 0,
        }
        .normalized();
        combo.validate()?;
        parse_single_key(&combo.trigger_key)?;
        for step in &combo.steps {
            parse_single_key(&step.key)?;
        }
        Ok(combo)
    }

    fn open_special_key_dialog(&mut self, edit_index: Option<usize>) {
        self.special_key_dialog = Some(if let Some(index) = edit_index {
            special_key_dialog_from_config(index, &self.draft.special_keys[index])
        } else {
            SpecialKeyDialogState {
                edit_index: None,
                config_type: SpecialKeyType::CustomAutofire,
                name: self.generate_special_key_name(),
                custom_key: "未录入".to_string(),
                auto_trigger_key: "未录入".to_string(),
                auto_trigger_hotkey: "未录入".to_string(),
                linked_trigger_key: "未录入".to_string(),
                linked_target_key: "未录入".to_string(),
                linked_trigger_mode: LinkedTriggerMode::Press,
                repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS.to_string(),
                press_duration_ms: DEFAULT_PRESS_DURATION_MS.to_string(),
                linked_interval_ms: DEFAULT_COMBO_STEP_INTERVAL_MS.to_string(),
                linked_press_duration_ms: DEFAULT_COMBO_STEP_PRESS_DURATION_MS.to_string(),
                capture_target: None,
                capture_down_keys: HashSet::new(),
            }
        });
    }

    fn open_selected_special_key_dialog(&mut self) -> Result<()> {
        let Some(index) = self.selected_special_key_index else {
            bail!("请先选中一个特殊键位配置");
        };
        self.open_special_key_dialog(Some(index));
        Ok(())
    }

    fn remove_selected_special_key(&mut self) -> Result<()> {
        let Some(index) = self.selected_special_key_index else {
            bail!("请先选中一个特殊键位配置");
        };
        self.draft.special_keys.remove(index);
        self.selected_special_key_index = None;
        self.persist_special_key_changes()?;
        Ok(())
    }

    fn save_special_key_dialog(&mut self, dialog: &SpecialKeyDialogState) -> Result<()> {
        let special_key = self.read_special_key_from_dialog(dialog)?;
        for (index, existing) in self.draft.special_keys.iter().enumerate() {
            if Some(index) != dialog.edit_index
                && existing.name().eq_ignore_ascii_case(special_key.name())
            {
                bail!("已存在同名特殊键位配置: {}", special_key.name());
            }
        }

        if let Some(index) = dialog.edit_index {
            self.draft.special_keys[index] = special_key;
            self.selected_special_key_index = Some(index);
        } else {
            self.draft.special_keys.push(special_key);
            self.selected_special_key_index = Some(self.draft.special_keys.len().saturating_sub(1));
        }
        self.persist_special_key_changes()?;
        Ok(())
    }

    fn persist_special_key_changes(&mut self) -> Result<()> {
        if self.current_profile_key.is_none()
            || self
                .current_profile_key
                .as_deref()
                .is_some_and(|name| self.draft.name.trim() != name)
        {
            return self.save_profile();
        }

        let current_key = self
            .current_profile_key
            .clone()
            .expect("current_profile_key checked above");
        let mut profile = self
            .store
            .get_profile(Some(&current_key))
            .with_context(|| format!("配置不存在: {current_key}"))?;
        profile.combos = self.draft.combos.clone();
        profile.special_keys = self.draft.special_keys.clone();
        profile = profile.normalized();
        profile.validate()?;
        self.validate_draft(&ProfileDraft::from_named_profile(&current_key, &profile))?;

        self.store.upsert_profile(current_key, profile);
        self.store.save(&self.config_path)?;
        self.status_text = STATUS_STOPPED;
        Ok(())
    }

    fn read_special_key_from_dialog(
        &self,
        dialog: &SpecialKeyDialogState,
    ) -> Result<SpecialKeyConfig> {
        let name = dialog.name.trim();
        if name.is_empty() {
            bail!("请填写配置名称");
        }

        let special = match dialog.config_type {
            SpecialKeyType::CustomAutofire => {
                if dialog.custom_key.trim().is_empty() || dialog.custom_key == "未录入" {
                    bail!("请先录入独立连发键位");
                }
                SpecialKeyConfig::CustomAutofire {
                    name: name.to_string(),
                    key: dialog.custom_key.clone(),
                    repeat_interval_ms: parse_dialog_ms(&dialog.repeat_interval_ms, "连发间隔")?,
                    press_duration_ms: parse_dialog_ms(&dialog.press_duration_ms, "按下时长")?,
                }
            }
            SpecialKeyType::AutoTrigger => {
                if dialog.auto_trigger_key.trim().is_empty() || dialog.auto_trigger_key == "未录入"
                {
                    bail!("请先录入自动触发键位");
                }
                if dialog.auto_trigger_hotkey.trim().is_empty()
                    || dialog.auto_trigger_hotkey == "未录入"
                {
                    bail!("请先录入自动触发热键");
                }
                SpecialKeyConfig::AutoTrigger {
                    name: name.to_string(),
                    key: dialog.auto_trigger_key.clone(),
                    trigger_hotkey: dialog.auto_trigger_hotkey.clone(),
                    repeat_interval_ms: parse_dialog_ms(&dialog.repeat_interval_ms, "触发间隔")?,
                    press_duration_ms: parse_dialog_ms(&dialog.press_duration_ms, "按下时长")?,
                }
            }
            SpecialKeyType::LinkedKey => {
                if dialog.linked_trigger_key.trim().is_empty()
                    || dialog.linked_trigger_key == "未录入"
                {
                    bail!("请先录入触发键");
                }
                if dialog.linked_target_key.trim().is_empty()
                    || dialog.linked_target_key == "未录入"
                {
                    bail!("请先录入连携键");
                }
                SpecialKeyConfig::LinkedKey {
                    name: name.to_string(),
                    trigger_key: dialog.linked_trigger_key.clone(),
                    linked_key: dialog.linked_target_key.clone(),
                    trigger_mode: dialog.linked_trigger_mode,
                    interval_ms: parse_dialog_ms(&dialog.linked_interval_ms, "触发延迟")?,
                    press_duration_ms: parse_dialog_ms(
                        &dialog.linked_press_duration_ms,
                        "按下时长",
                    )?,
                }
            }
        }
        .normalized();

        let mut draft = self.draft.clone();
        if let Some(index) = dialog.edit_index {
            draft.special_keys[index] = special.clone();
        } else {
            draft.special_keys.push(special.clone());
        }
        self.validate_draft(&draft)?;
        Ok(special)
    }

    fn generate_profile_name(&self) -> String {
        let mut index = 1;
        loop {
            let candidate = format!("profile-{index}");
            if !self.store.profiles.contains_key(&candidate) {
                return candidate;
            }
            index += 1;
        }
    }

    fn generate_cloned_profile_name(&self) -> String {
        let base_name = self.draft.name.trim().trim_end_matches('-').to_string();
        let mut base_name = if base_name.is_empty() {
            self.current_profile_key
                .clone()
                .unwrap_or_else(|| "profile".to_string())
        } else {
            base_name
        };
        while let Some(stripped) = strip_clone_suffix(&base_name) {
            base_name = stripped.to_string();
        }

        let mut index = 1;
        loop {
            let candidate = format!("{base_name}-cloned-{index}");
            if !self.store.profiles.contains_key(&candidate) {
                return candidate;
            }
            index += 1;
        }
    }

    fn generate_combo_name(&self) -> String {
        let mut index = 1;
        loop {
            let candidate = format!("combo-{index}");
            if !self
                .draft
                .combos
                .iter()
                .any(|combo| combo.name.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            index += 1;
        }
    }

    fn generate_special_key_name(&self) -> String {
        let mut index = 1;
        loop {
            let candidate = format!("special-{index}");
            if !self
                .draft
                .special_keys
                .iter()
                .any(|special| special.name().eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            index += 1;
        }
    }
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
