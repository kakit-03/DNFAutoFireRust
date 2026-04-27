use crate::autofire::{AutoFireService, RunnerEvent, RunnerHandle};
use crate::config::{
    ComboConfig, ComboStepConfig, ConfigStore, DEFAULT_COMBO_STEP_INTERVAL_MS,
    DEFAULT_COMBO_STEP_PRESS_DURATION_MS, DEFAULT_PRESS_DURATION_MS, DEFAULT_REPEAT_INTERVAL_MS,
    LinkedTriggerMode, Profile, SpecialKeyConfig,
};
use crate::gui_model::{ProfileDraft, target_windows_from_text, target_windows_to_text};
use crate::input::is_vk_down;
use crate::input_backend::{
    InputBackendKind, input_backend_descriptor, input_backend_descriptors, input_backend_label,
};
use crate::keymap::{
    HotkeyRegistration, display_hotkey_names, display_hotkey_text, display_key_name,
    hotkey_registration, is_modifier_key, normalize_hotkey_text, parse_hotkey, parse_key_specs,
    parse_single_key, sort_hotkey_names, supported_key_names,
};
use crate::single_instance::SingleInstanceGuard;
use crate::timing::SleepTimingMonitor;
use crate::win::{foreground_window_info, foreground_window_is};
use anyhow::{Context, Result, bail};
use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Key, Layout, Pos2,
    Rect, RichText, ScrollArea, Sense, Stroke, TopBottomPanel, Vec2, ViewportCommand,
};
use eframe::{App, CreationContext, Frame, NativeOptions};
use image::{ImageReader, imageops::FilterType};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle, sleep};
use std::time::Duration;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MB_ICONERROR, MB_OK, MSG, MessageBoxW, PM_REMOVE, PeekMessageW, SW_HIDE,
    SW_RESTORE, SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow,
    TranslateMessage, WM_HOTKEY,
};
use windows::core::{HSTRING, w};

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

const KEYBOARD_FRAME_WIDTH: f32 = 980.0;
const KEYBOARD_FRAME_HEIGHT: f32 = 240.0;
const KEYBOARD_KEY_WIDTH: f32 = 36.0;
const KEYBOARD_KEY_HEIGHT: f32 = 30.0;
const KEYBOARD_KEY_GAP: f32 = 4.0;
const KEYBOARD_BLOCK_GAP: f32 = 12.0;
const KEYBOARD_MARGIN: f32 = 12.0;
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

const KEYBOARD_BG_SELECTED: Color32 = Color32::from_rgb(191, 221, 255);
const KEYBOARD_BG_NORMAL: Color32 = Color32::from_rgb(239, 243, 248);
const KEYBOARD_BG_DISABLED: Color32 = Color32::from_rgb(228, 228, 228);
const KEYBOARD_BORDER: Color32 = Color32::from_rgb(120, 132, 148);
const KEYBOARD_TEXT_DISABLED: Color32 = Color32::from_rgb(125, 125, 125);
const KEYBOARD_TEXT_NORMAL: Color32 = Color32::from_rgb(40, 40, 40);

const TRAY_SHOW_ID: &str = "tray.show";
const TRAY_HIDE_ID: &str = "tray.hide";
const TRAY_START_ID: &str = "tray.start";
const TRAY_STOP_ID: &str = "tray.stop";
const TRAY_EXIT_ID: &str = "tray.exit";

const FONT_CANDIDATES: &[&str] = &["simhei.ttf", "msyh.ttf", "msyh.ttc", "simsun.ttc"];

#[derive(Clone, Copy)]
struct KeyboardLayoutKey {
    token: &'static str,
    label: &'static str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy)]
struct KeyboardCell {
    token: &'static str,
    label: &'static str,
    width_units: f32,
}

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

    fn render_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("按键设置");
                ui.add_space(12.0);
                ui.label(
                    RichText::new(
                        "点击键帽切换是否加入连发。选中的键会保持高亮底色；右上角灰色键仅展示布局，不参与配置。",
                    )
                    .color(Color32::from_rgb(80, 88, 98))
                    .size(13.0),
                );
            });
            ui.add_space(8.0);

            let available = ui.available_size();
            let desired_height = available.y.max(220.0);
            let (response, painter) = ui.allocate_painter(
                Vec2::new(ui.available_width(), desired_height),
                Sense::click(),
            );
            let outer_rect = response.rect.shrink2(Vec2::splat(10.0));
            let scale = (outer_rect.width() / KEYBOARD_FRAME_WIDTH)
                .min(outer_rect.height() / KEYBOARD_FRAME_HEIGHT)
                .max(0.1);
            let scaled_size = Vec2::new(KEYBOARD_FRAME_WIDTH * scale, KEYBOARD_FRAME_HEIGHT * scale);
            let origin = Pos2::new(
                outer_rect.center().x - scaled_size.x / 2.0,
                outer_rect.center().y - scaled_size.y / 2.0,
            );

            let editable = self.state.is_editing_enabled();
            let layout_keys = keyboard_layout_keys();
            let mut hit_token = None;

            if editable && response.clicked() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    for key in &layout_keys {
                        let key_rect = scaled_key_rect(origin, scale, key);
                        if key_rect.contains(pointer_pos) && parse_single_key(key.token).is_ok() {
                            hit_token = Some(key.token.to_string());
                            break;
                        }
                    }
                }
            }

            for key in &layout_keys {
                let supported = parse_single_key(key.token).is_ok();
                let selected = supported && self.state.enabled_keys.contains(key.token);
                let key_rect = scaled_key_rect(origin, scale, key);
                let fill = if selected {
                    KEYBOARD_BG_SELECTED
                } else if supported {
                    KEYBOARD_BG_NORMAL
                } else {
                    KEYBOARD_BG_DISABLED
                };
                let text_color = if supported {
                    KEYBOARD_TEXT_NORMAL
                } else {
                    KEYBOARD_TEXT_DISABLED
                };
                let rounding = egui::CornerRadius::same((6.0 * scale).clamp(3.0, 8.0) as u8);

                painter.rect_filled(key_rect, rounding, fill);
                painter.rect_stroke(
                    key_rect,
                    rounding,
                    Stroke::new(if selected { 1.4 } else { 1.0 }, KEYBOARD_BORDER),
                    egui::StrokeKind::Outside,
                );
                painter.text(
                    key_rect.center(),
                    Align2::CENTER_CENTER,
                    key.label,
                    FontId::proportional((12.0 * scale).clamp(8.0, 18.0)),
                    text_color,
                );
            }

            if let Some(token) = hit_token {
                self.state.toggle_enabled_key(&token);
            }
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

    fn poll_combo_capture(&self, dialog: &mut ComboDialogState, ctx: &egui::Context) {
        let Some(target) = dialog.capture_target else {
            return;
        };

        ctx.request_repaint_after(Duration::from_millis(16));
        let (current_down, captured_key) = capture_next_supported_key(&dialog.capture_down_keys);
        dialog.capture_down_keys = current_down;

        let Some(key) = captured_key else {
            return;
        };

        match target {
            ComboCaptureTarget::Trigger => {
                dialog.trigger_key = key;
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
            ComboCaptureTarget::NewStep => {
                dialog.steps.push(ComboStepDraft {
                    key,
                    interval_ms: dialog.new_step_interval_ms.clone(),
                    press_duration_ms: dialog.new_step_press_duration_ms.clone(),
                });
                dialog.selected_step = Some(dialog.steps.len().saturating_sub(1));
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
            ComboCaptureTarget::ContinuousSteps => {
                dialog.steps.push(ComboStepDraft {
                    key,
                    interval_ms: dialog.new_step_interval_ms.clone(),
                    press_duration_ms: dialog.new_step_press_duration_ms.clone(),
                });
                dialog.selected_step = Some(dialog.steps.len().saturating_sub(1));
            }
            ComboCaptureTarget::SelectedStep => {
                if let Some(index) = dialog.selected_step {
                    dialog.steps[index].key = key;
                }
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
        }
    }

    fn poll_special_key_capture(&self, dialog: &mut SpecialKeyDialogState, ctx: &egui::Context) {
        let Some(target) = dialog.capture_target else {
            return;
        };

        ctx.request_repaint_after(Duration::from_millis(16));
        let (current_down, captured_key) = capture_next_supported_key(&dialog.capture_down_keys);

        match target {
            SpecialKeyCaptureTarget::AutoTriggerHotkey => {
                dialog.capture_down_keys = current_down.clone();
                let Some(captured_key) = captured_key else {
                    return;
                };
                if is_modifier_key(&captured_key) {
                    return;
                }
                let ordered = sort_hotkey_names(current_down.into_iter().collect::<Vec<_>>());
                dialog.auto_trigger_hotkey = display_hotkey_names(ordered);
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
            _ => {
                dialog.capture_down_keys = current_down;
                let Some(key) = captured_key else {
                    return;
                };
                match target {
                    SpecialKeyCaptureTarget::CustomKey => dialog.custom_key = key,
                    SpecialKeyCaptureTarget::AutoTriggerKey => dialog.auto_trigger_key = key,
                    SpecialKeyCaptureTarget::LinkedTriggerKey => dialog.linked_trigger_key = key,
                    SpecialKeyCaptureTarget::LinkedTargetKey => dialog.linked_target_key = key,
                    SpecialKeyCaptureTarget::AutoTriggerHotkey => unreachable!(),
                }
                dialog.capture_target = None;
                dialog.capture_down_keys.clear();
            }
        }
    }

    fn render_combo_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.state.combo_dialog.take() else {
            return;
        };
        self.poll_combo_capture(&mut dialog, ctx);

        let mut keep_open = true;
        let mut close_requested = false;
        let title = if dialog.edit_index.is_some() {
            "编辑连招"
        } else {
            "新增连招"
        };

        egui::Window::new(title)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .open(&mut keep_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(640.0);
                ui.label("名称");
                ui.text_edit_singleline(&mut dialog.name);
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label("触发键");
                        ui.horizontal(|ui| {
                            ui.monospace(display_key_name(dialog.trigger_key.as_str()));
                            let label =
                                if dialog.capture_target == Some(ComboCaptureTarget::Trigger) {
                                    "等待按键..."
                                } else {
                                    "录入触发键"
                                };
                            if ui.button(label).clicked() {
                                start_combo_capture(&mut dialog, ComboCaptureTarget::Trigger);
                            }
                        });
                    });
                });

                if let Some(message) = combo_capture_message(dialog.capture_target) {
                    ui.add_space(6.0);
                    ui.label(RichText::new(message).color(Color32::from_rgb(55, 95, 165)));
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                ui.strong("新增步骤");
                ui.horizontal(|ui| {
                    ui.label("步骤间隔(ms)");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.new_step_interval_ms)
                            .desired_width(80.0),
                    );
                    ui.label("按下时长(ms)");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.new_step_press_duration_ms)
                            .desired_width(80.0),
                    );
                    let single_label = if dialog.capture_target == Some(ComboCaptureTarget::NewStep)
                    {
                        "等待按键..."
                    } else {
                        "按键录入并追加"
                    };
                    if ui.button(single_label).clicked() {
                        start_combo_capture(&mut dialog, ComboCaptureTarget::NewStep);
                    }
                    let continuous_active =
                        dialog.capture_target == Some(ComboCaptureTarget::ContinuousSteps);
                    let continuous_label = if continuous_active {
                        "停止连续录入"
                    } else {
                        "开始连续录入"
                    };
                    if ui.button(continuous_label).clicked() {
                        if continuous_active {
                            dialog.capture_target = None;
                            dialog.capture_down_keys.clear();
                        } else {
                            start_combo_capture(&mut dialog, ComboCaptureTarget::ContinuousSteps);
                        }
                    }
                });
                ui.label(
                    RichText::new(
                        "步骤间隔表示当前步骤执行后，到下一步开始前的等待时间。连续录入开启后，可直接连续按键追加步骤。",
                    )
                        .size(12.5)
                        .color(Color32::from_rgb(95, 100, 110)),
                );
                if let Some(last_step) = dialog.steps.last() {
                    ui.label(
                        RichText::new(format!(
                            "最近一步: {} / 间隔 {} ms / 按下 {} ms",
                            display_key_name(&last_step.key),
                            last_step.interval_ms,
                            last_step.press_duration_ms
                        ))
                        .size(12.5)
                        .color(Color32::from_rgb(55, 95, 165)),
                    );
                } else {
                    ui.label(
                        RichText::new("最近一步: 还没有录入步骤")
                            .size(12.5)
                            .color(Color32::from_rgb(120, 125, 135)),
                    );
                }

                ui.add_space(10.0);
                ui.strong("步骤列表");
                let stick_steps_to_bottom =
                    dialog.capture_target == Some(ComboCaptureTarget::ContinuousSteps);
                ScrollArea::vertical()
                    .id_salt("combo_sequence_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .stick_to_bottom(stick_steps_to_bottom)
                    .max_height(180.0)
                    .show(ui, |ui| {
                        let mut start_capture_step = None;
                        let mut move_up_step = None;
                        let mut move_down_step = None;
                        let mut delete_step = None;

                        egui::Grid::new("combo_steps_grid")
                            .num_columns(7)
                            .striped(true)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.strong("步骤");
                                ui.strong("按键");
                                ui.strong("间隔(ms)");
                                ui.strong("按下(ms)");
                                ui.strong("");
                                ui.strong("");
                                ui.strong("");
                                ui.end_row();

                                for index in 0..dialog.steps.len() {
                                    let key_text = display_key_name(&dialog.steps[index].key);
                                    let selected = dialog.selected_step == Some(index);
                                    if ui
                                        .add(
                                            egui::Button::new(format!("{}", index + 1))
                                                .selected(selected),
                                        )
                                        .clicked()
                                    {
                                        dialog.selected_step = Some(index);
                                    }
                                    let key_button_text = if dialog.capture_target
                                        == Some(ComboCaptureTarget::SelectedStep)
                                        && dialog.selected_step == Some(index)
                                    {
                                        "等待按键...".to_string()
                                    } else {
                                        key_text
                                    };
                                    if ui
                                        .add(egui::Button::new(key_button_text).selected(selected))
                                        .clicked()
                                    {
                                        dialog.selected_step = Some(index);
                                        start_capture_step = Some(index);
                                    }
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut dialog.steps[index].interval_ms,
                                        )
                                        .desired_width(80.0),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut dialog.steps[index].press_duration_ms,
                                        )
                                        .desired_width(80.0),
                                    );
                                    if ui.small_button("↑").clicked() && index > 0 {
                                        move_up_step = Some(index);
                                    }
                                    if ui.small_button("↓").clicked()
                                        && index + 1 < dialog.steps.len()
                                    {
                                        move_down_step = Some(index);
                                    }
                                    if ui.small_button("🗑").clicked() {
                                        delete_step = Some(index);
                                    }
                                    ui.end_row();
                                }
                            });

                        if let Some(index) = start_capture_step {
                            dialog.selected_step = Some(index);
                            start_combo_capture(&mut dialog, ComboCaptureTarget::SelectedStep);
                        }
                        if let Some(index) = move_up_step {
                            dialog.steps.swap(index, index - 1);
                            dialog.selected_step = Some(index - 1);
                        }
                        if let Some(index) = move_down_step {
                            dialog.steps.swap(index, index + 1);
                            dialog.selected_step = Some(index + 1);
                        }
                        if let Some(index) = delete_step {
                            dialog.steps.remove(index);
                            dialog.selected_step = if dialog.steps.is_empty() {
                                None
                            } else if index >= dialog.steps.len() {
                                Some(dialog.steps.len() - 1)
                            } else {
                                Some(index)
                            };
                        }
                    });

                ui.add_space(8.0);
                ui.label(
                    RichText::new("步骤按键可直接点列表重录；间隔和按下时长都可直接在列表内修改。")
                        .size(12.5)
                        .color(Color32::from_rgb(95, 100, 110)),
                );

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("保存连招").clicked() {
                        match self.state.save_combo_dialog(&dialog) {
                            Ok(()) => close_requested = true,
                            Err(err) => self.show_error(&format!("{err:#}")),
                        }
                    }
                    if ui.button("取消").clicked() {
                        close_requested = true;
                    }
                });
            });

        if close_requested {
            keep_open = false;
        }
        if keep_open {
            self.state.combo_dialog = Some(dialog);
        }
    }

    fn render_special_key_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.state.special_key_dialog.take() else {
            return;
        };
        self.poll_special_key_capture(&mut dialog, ctx);

        let mut keep_open = true;
        let mut close_requested = false;
        let title = if dialog.edit_index.is_some() {
            "编辑特殊键位配置"
        } else {
            "新增特殊键位配置"
        };

        egui::Window::new(title)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .open(&mut keep_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(620.0);
                ui.label("名称");
                ui.text_edit_singleline(&mut dialog.name);
                ui.add_space(8.0);

                ui.label("类型");
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut dialog.config_type,
                        SpecialKeyType::CustomAutofire,
                        "独立连发",
                    );
                    ui.selectable_value(
                        &mut dialog.config_type,
                        SpecialKeyType::AutoTrigger,
                        "自动触发",
                    );
                    ui.selectable_value(
                        &mut dialog.config_type,
                        SpecialKeyType::LinkedKey,
                        "连携键位",
                    );
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                match dialog.config_type {
                    SpecialKeyType::CustomAutofire => {
                        ui.horizontal(|ui| {
                            ui.label("键位");
                            ui.monospace(display_key_name(dialog.custom_key.as_str()));
                            let label =
                                if dialog.capture_target == Some(SpecialKeyCaptureTarget::CustomKey)
                                {
                                    "等待按键..."
                                } else {
                                    "录入键位"
                                };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::CustomKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        egui::Grid::new("special_custom_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("连发间隔(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.repeat_interval_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();

                                ui.label("按下时长(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.press_duration_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();
                            });
                    }
                    SpecialKeyType::AutoTrigger => {
                        ui.horizontal(|ui| {
                            ui.label("自动触发键位");
                            ui.monospace(display_key_name(dialog.auto_trigger_key.as_str()));
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::AutoTriggerKey)
                            {
                                "等待按键..."
                            } else {
                                "录入键位"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::AutoTriggerKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("触发热键");
                            ui.monospace(dialog.auto_trigger_hotkey.as_str());
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::AutoTriggerHotkey)
                            {
                                "等待组合键..."
                            } else {
                                "录入热键"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::AutoTriggerHotkey,
                                );
                            }
                        });
                        ui.label(
                            RichText::new(
                                "触发热键会切换该键位的自动触发开关；热键允许组合，但不能与全局快速切换热键冲突。",
                            )
                            .size(12.5)
                            .color(Color32::from_rgb(95, 100, 110)),
                        );
                        ui.add_space(8.0);
                        egui::Grid::new("special_auto_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("触发间隔(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.repeat_interval_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();

                                ui.label("按下时长(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.press_duration_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();
                            });
                    }
                    SpecialKeyType::LinkedKey => {
                        ui.horizontal(|ui| {
                            ui.label("触发键");
                            ui.monospace(display_key_name(dialog.linked_trigger_key.as_str()));
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::LinkedTriggerKey)
                            {
                                "等待按键..."
                            } else {
                                "录入触发键"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::LinkedTriggerKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("连携键");
                            ui.monospace(display_key_name(dialog.linked_target_key.as_str()));
                            let label = if dialog.capture_target
                                == Some(SpecialKeyCaptureTarget::LinkedTargetKey)
                            {
                                "等待按键..."
                            } else {
                                "录入连携键"
                            };
                            if ui.button(label).clicked() {
                                start_special_key_capture(
                                    &mut dialog,
                                    SpecialKeyCaptureTarget::LinkedTargetKey,
                                );
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("触发方式");
                            ui.selectable_value(
                                &mut dialog.linked_trigger_mode,
                                LinkedTriggerMode::Press,
                                "按下触发",
                            );
                            ui.selectable_value(
                                &mut dialog.linked_trigger_mode,
                                LinkedTriggerMode::Release,
                                "松开触发",
                            );
                        });
                        ui.add_space(8.0);
                        egui::Grid::new("special_linked_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("触发延迟(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut dialog.linked_interval_ms)
                                        .desired_width(100.0),
                                );
                                ui.end_row();

                                ui.label("按下时长(ms)");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut dialog.linked_press_duration_ms,
                                    )
                                    .desired_width(100.0),
                                );
                                ui.end_row();
                            });
                    }
                }

                if let Some(message) = special_key_capture_message(dialog.capture_target) {
                    ui.add_space(8.0);
                    ui.label(RichText::new(message).color(Color32::from_rgb(55, 95, 165)));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("保存配置").clicked() {
                        match self.state.save_special_key_dialog(&dialog) {
                            Ok(()) => close_requested = true,
                            Err(err) => self.show_error(&format!("{err:#}")),
                        }
                    }
                    if ui.button("取消").clicked() {
                        close_requested = true;
                    }
                });
            });

        if close_requested {
            keep_open = false;
        }
        if keep_open {
            self.state.special_key_dialog = Some(dialog);
        }
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

    fn show_error(&self, message: &str) {
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

impl QuickSwitchMonitor {
    fn spawn(
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

    fn set_config(&self, config: QuickSwitchWatchConfig) {
        if let Ok(mut guard) = self.config.lock() {
            *guard = config;
        }
    }

    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl TrayResources {
    fn build(
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

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = load_cjk_font() {
        fonts
            .font_data
            .insert("cjk".to_string(), FontData::from_owned(bytes).into());
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "cjk".to_string());
        }
        if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
            family.push("cjk".to_string());
        }
    }
    ctx.set_fonts(fonts);
}

fn load_cjk_font() -> Option<Vec<u8>> {
    let font_dir = std::env::var("WINDIR")
        .ok()
        .map(PathBuf::from)?
        .join("Fonts");
    for candidate in FONT_CANDIDATES {
        let path = font_dir.join(candidate);
        if let Ok(bytes) = fs::read(path) {
            return Some(bytes);
        }
    }
    None
}

fn hwnd_from_creation_context(cc: &CreationContext<'_>) -> Result<HWND> {
    let handle = cc.window_handle()?.as_raw();
    match handle {
        RawWindowHandle::Win32(win32) => Ok(HWND(win32.hwnd.get() as _)),
        _ => bail!("unsupported platform window handle"),
    }
}

fn project_asset_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn load_window_icon(path: &Path) -> Result<egui::IconData> {
    let image = ImageReader::open(path)
        .with_context(|| format!("failed to open window icon '{}'", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode window icon '{}'", path.display()))?;
    let rgba = image
        .resize_exact(APP_ICON_SIZE, APP_ICON_SIZE, FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

fn load_tray_icon_base(path: &Path) -> Result<image::RgbaImage> {
    let image = ImageReader::open(&path)
        .with_context(|| format!("failed to open tray icon '{}'", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode tray icon '{}'", path.display()))?;
    Ok(image
        .resize_exact(TRAY_ICON_SIZE, TRAY_ICON_SIZE, FilterType::Lanczos3)
        .into_rgba8())
}

fn tray_icon_with_status_dot(base: &image::RgbaImage, dot_color: [u8; 4]) -> Result<Icon> {
    let mut rgba = base.clone();
    paint_status_dot(&mut rgba, dot_color);
    let (width, height) = rgba.dimensions();
    Icon::from_rgba(rgba.into_raw(), width, height)
        .context("failed to build tray icon with status dot")
}

fn paint_status_dot(image: &mut image::RgbaImage, dot_color: [u8; 4]) {
    let width = image.width() as i32;
    let height = image.height() as i32;
    let outer_radius = 8i32;
    let inner_radius = 7i32;
    let center_x = width - outer_radius;
    let center_y = height - outer_radius;

    for y in (center_y - outer_radius)..=(center_y + outer_radius) {
        if !(0..height).contains(&y) {
            continue;
        }
        for x in (center_x - outer_radius)..=(center_x + outer_radius) {
            if !(0..width).contains(&x) {
                continue;
            }

            let dx = x - center_x;
            let dy = y - center_y;
            let distance_squared = dx * dx + dy * dy;
            if distance_squared > outer_radius * outer_radius {
                continue;
            }

            let pixel = if distance_squared <= inner_radius * inner_radius {
                image::Rgba(dot_color)
            } else {
                image::Rgba([255, 255, 255, 255])
            };
            image.put_pixel(x as u32, y as u32, pixel);
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

fn scaled_key_rect(origin: Pos2, scale: f32, key: &KeyboardLayoutKey) -> Rect {
    Rect::from_min_size(
        Pos2::new(origin.x + key.x * scale, origin.y + key.y * scale),
        Vec2::new(key.width * scale, key.height * scale),
    )
}

fn keyboard_layout_keys() -> Vec<KeyboardLayoutKey> {
    let mut keys = Vec::new();
    let main_x = KEYBOARD_MARGIN;
    let nav_x = main_x + keyboard_units_to_px(16.0) + KEYBOARD_BLOCK_GAP;
    let num_x = nav_x + keyboard_units_to_px(3.0) + KEYBOARD_BLOCK_GAP;

    let row0 = keyboard_row_y(0.0);
    push_keyboard_row(
        &mut keys,
        main_x,
        row0,
        &[KeyboardCell {
            token: "ESC",
            label: "Esc",
            width_units: 1.0,
        }],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(2.0),
        row0,
        &[
            KeyboardCell {
                token: "F1",
                label: "F1",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F2",
                label: "F2",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F3",
                label: "F3",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F4",
                label: "F4",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(6.0) + KEYBOARD_BLOCK_GAP,
        row0,
        &[
            KeyboardCell {
                token: "F5",
                label: "F5",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F6",
                label: "F6",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F7",
                label: "F7",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F8",
                label: "F8",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(10.0) + KEYBOARD_BLOCK_GAP * 2.0,
        row0,
        &[
            KeyboardCell {
                token: "F9",
                label: "F9",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F10",
                label: "F10",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F11",
                label: "F11",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F12",
                label: "F12",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        row0,
        &[
            KeyboardCell {
                token: "PRINTSCREEN",
                label: "PrtSc",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SCROLLLOCK",
                label: "ScrLk",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PAUSE",
                label: "Pause",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(1.0),
        &[
            KeyboardCell {
                token: "BACKQUOTE",
                label: "~",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "1",
                label: "1",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "2",
                label: "2",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "3",
                label: "3",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "4",
                label: "4",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "5",
                label: "5",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "6",
                label: "6",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "7",
                label: "7",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "8",
                label: "8",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "9",
                label: "9",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "0",
                label: "0",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "MINUS",
                label: "-",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "EQUAL",
                label: "=",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "BACKSPACE",
                label: "Bksp",
                width_units: 2.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(1.0),
        &[
            KeyboardCell {
                token: "INSERT",
                label: "Ins",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "HOME",
                label: "Home",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PAGEUP",
                label: "PgUp",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(1.0),
        &[
            KeyboardCell {
                token: "NUMLOCK",
                label: "Num",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMDIV",
                label: "/",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMMUL",
                label: "*",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMMINUS",
                label: "-",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(2.0),
        &[
            KeyboardCell {
                token: "TAB",
                label: "Tab",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "Q",
                label: "Q",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "W",
                label: "W",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "E",
                label: "E",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "R",
                label: "R",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "T",
                label: "T",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "Y",
                label: "Y",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "U",
                label: "U",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "I",
                label: "I",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "O",
                label: "O",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "P",
                label: "P",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "LBRACKET",
                label: "[",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RBRACKET",
                label: "]",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "BACKSLASH",
                label: "\\",
                width_units: 2.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(2.0),
        &[
            KeyboardCell {
                token: "DELETE",
                label: "Del",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "END",
                label: "End",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PAGEDOWN",
                label: "PgDn",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(2.0),
        &[
            KeyboardCell {
                token: "NUM7",
                label: "7",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM8",
                label: "8",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM9",
                label: "9",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMPLUS",
                label: "+",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(3.0),
        &[
            KeyboardCell {
                token: "CAPSLOCK",
                label: "Caps",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "A",
                label: "A",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "S",
                label: "S",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "D",
                label: "D",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F",
                label: "F",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "G",
                label: "G",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "H",
                label: "H",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "J",
                label: "J",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "K",
                label: "K",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "L",
                label: "L",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SEMICOLON",
                label: ";",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "APOSTROPHE",
                label: "'",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "ENTER",
                label: "Enter",
                width_units: 3.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(3.0),
        &[
            KeyboardCell {
                token: "NUM4",
                label: "4",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM5",
                label: "5",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM6",
                label: "6",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMENTER",
                label: "Ent",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(4.0),
        &[
            KeyboardCell {
                token: "LSHIFT",
                label: "LShift",
                width_units: 3.0,
            },
            KeyboardCell {
                token: "Z",
                label: "Z",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "X",
                label: "X",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "C",
                label: "C",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "V",
                label: "V",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "B",
                label: "B",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "N",
                label: "N",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "M",
                label: "M",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "COMMA",
                label: ",",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PERIOD",
                label: ".",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SLASH",
                label: "/",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RSHIFT",
                label: "RShift",
                width_units: 3.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x + keyboard_units_to_px(1.0),
        keyboard_row_y(4.0),
        &[KeyboardCell {
            token: "UP",
            label: "Up",
            width_units: 1.0,
        }],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(4.0),
        &[
            KeyboardCell {
                token: "NUM1",
                label: "1",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM2",
                label: "2",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM3",
                label: "3",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(5.0),
        &[
            KeyboardCell {
                token: "LCTRL",
                label: "LCtrl",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "LWIN",
                label: "Win",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "LALT",
                label: "LAlt",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SPACE",
                label: "Space",
                width_units: 6.0,
            },
            KeyboardCell {
                token: "RALT",
                label: "RAlt",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RWIN",
                label: "Win",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "MENU",
                label: "Menu",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RCTRL",
                label: "RCtrl",
                width_units: 3.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(5.0),
        &[
            KeyboardCell {
                token: "LEFT",
                label: "Left",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "DOWN",
                label: "Down",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RIGHT",
                label: "Right",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(5.0),
        &[
            KeyboardCell {
                token: "NUM0",
                label: "0",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "NUMDOT",
                label: ".",
                width_units: 1.0,
            },
        ],
    );

    keys
}

fn push_keyboard_row(keys: &mut Vec<KeyboardLayoutKey>, base_x: f32, y: f32, row: &[KeyboardCell]) {
    let mut x = base_x;
    for cell in row {
        keys.push(KeyboardLayoutKey {
            token: cell.token,
            label: cell.label,
            x,
            y,
            width: keyboard_units_to_px(cell.width_units),
            height: KEYBOARD_KEY_HEIGHT,
        });
        x += keyboard_units_to_px(cell.width_units) + KEYBOARD_KEY_GAP;
    }
}

fn keyboard_row_y(row: f32) -> f32 {
    KEYBOARD_MARGIN + row * (KEYBOARD_KEY_HEIGHT + KEYBOARD_KEY_GAP)
}

fn keyboard_units_to_px(units: f32) -> f32 {
    units * KEYBOARD_KEY_WIDTH + (units - 1.0) * KEYBOARD_KEY_GAP
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
    use crate::input_backend::InputBackendKind;
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
