use crate::autofire::{AutoFireService, RunnerEvent, RunnerHandle};
use crate::config::{ComboConfig, ConfigStore, Profile};
use crate::gui_model::ProfileDraft;
use crate::keymap::{parse_key_sequence, parse_key_specs, parse_single_key, supported_key_names};
use crate::single_instance::SingleInstanceGuard;
use anyhow::{Context, Result, bail};
use native_windows_gui as nwg;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

const STATUS_IDLE: &str = "状态: 未运行";
const STATUS_RUNNING: &str = "状态: 运行中";
const STATUS_IME_PAUSED: &str = "状态: 输入法暂停";
const STATUS_STOPPED: &str = "状态: 已停止";
const KEYBOARD_FRAME_WIDTH: i32 = 944;
const KEYBOARD_FRAME_HEIGHT: i32 = 220;
const KEYBOARD_KEY_WIDTH: i32 = 36;
const KEYBOARD_KEY_HEIGHT: i32 = 30;
const KEYBOARD_KEY_GAP: i32 = 4;
const KEYBOARD_BLOCK_GAP: i32 = 12;
const KEYBOARD_MARGIN: i32 = 4;

struct KeyboardButton {
    token: &'static str,
    label: &'static str,
    supported: bool,
    button: nwg::Button,
}

#[derive(Clone, Copy)]
struct KeyboardLayoutKey {
    token: &'static str,
    label: &'static str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Clone, Copy)]
struct KeyboardCell {
    token: &'static str,
    label: &'static str,
    width_units: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayState {
    Disabled,
    Enabled,
    Paused,
}

pub fn run(config_path: PathBuf, store: ConfigStore) -> Result<()> {
    let _instance_guard = SingleInstanceGuard::acquire()?;
    nwg::init().context("failed to initialize native-windows-gui")?;
    let _ = nwg::Font::set_global_family("Segoe UI");

    let _app = GuiApp::build(config_path, store)?;
    nwg::dispatch_thread_events();
    Ok(())
}

struct GuiApp {
    config_path: PathBuf,
    store: RefCell<ConfigStore>,
    current_profile_key: RefCell<Option<String>>,
    current_combos: RefCell<Vec<ComboConfig>>,
    enabled_keys: RefCell<HashSet<String>>,
    tray_state: Cell<TrayState>,
    window_hidden: Cell<bool>,
    quitting: Cell<bool>,
    runner: RefCell<Option<RunnerHandle>>,
    runner_events: RefCell<Option<Receiver<RunnerEvent>>>,
    combo_edit_index: Cell<Option<usize>>,
    combo_sequence: RefCell<Vec<String>>,
    keyboard_buttons: RefCell<Vec<KeyboardButton>>,
    evt_handlers: RefCell<Vec<nwg::EventHandler>>,

    window: nwg::Window,
    notice: nwg::Notice,
    main_layout: nwg::GridLayout,
    left_frame: nwg::Frame,
    right_frame: nwg::Frame,
    left_layout: nwg::GridLayout,
    right_layout: nwg::GridLayout,

    profiles_label: nwg::Label,
    default_label: nwg::Label,
    profile_list: nwg::ListBox<String>,
    new_button: nwg::Button,
    save_button: nwg::Button,
    delete_button: nwg::Button,
    default_button: nwg::Button,

    editor_label: nwg::Label,
    name_label: nwg::Label,
    name_input: nwg::TextInput,
    keys_label: nwg::Label,
    keys_hint_label: nwg::Label,
    keyboard_frame: nwg::Frame,
    repeat_label: nwg::Label,
    repeat_input: nwg::TextInput,
    press_label: nwg::Label,
    press_input: nwg::TextInput,
    poll_label: nwg::Label,
    poll_input: nwg::TextInput,
    windows_label: nwg::Label,
    windows_text: nwg::TextBox,
    combos_label: nwg::Label,
    combo_list: nwg::ListView,
    combo_add_button: nwg::Button,
    combo_edit_button: nwg::Button,
    combo_remove_button: nwg::Button,
    start_button: nwg::Button,
    stop_button: nwg::Button,
    status_label: nwg::Label,
    tray_enabled_icon: nwg::Icon,
    tray_paused_icon: nwg::Icon,
    tray_disabled_icon: nwg::Icon,
    tray: nwg::TrayNotification,
    tray_menu: nwg::Menu,
    tray_status_enabled: nwg::MenuItem,
    tray_status_paused: nwg::MenuItem,
    tray_status_disabled: nwg::MenuItem,
    tray_status_separator: nwg::MenuSeparator,
    tray_show_item: nwg::MenuItem,
    tray_hide_item: nwg::MenuItem,
    tray_window_separator: nwg::MenuSeparator,
    tray_start_item: nwg::MenuItem,
    tray_stop_item: nwg::MenuItem,
    tray_action_separator: nwg::MenuSeparator,
    tray_exit_item: nwg::MenuItem,

    combo_window: nwg::Window,
    combo_layout: nwg::GridLayout,
    combo_header_label: nwg::Label,
    combo_name_label: nwg::Label,
    combo_name_input: nwg::TextInput,
    combo_trigger_label: nwg::Label,
    combo_trigger_combo: nwg::ComboBox<String>,
    combo_step_label: nwg::Label,
    combo_step_input: nwg::TextInput,
    combo_press_label: nwg::Label,
    combo_press_input: nwg::TextInput,
    combo_sequence_label: nwg::Label,
    combo_sequence_list: nwg::ListBox<String>,
    combo_available_key_label: nwg::Label,
    combo_available_key_combo: nwg::ComboBox<String>,
    combo_add_step_button: nwg::Button,
    combo_remove_step_button: nwg::Button,
    combo_up_button: nwg::Button,
    combo_down_button: nwg::Button,
    combo_save_button: nwg::Button,
    combo_cancel_button: nwg::Button,
}

impl GuiApp {
    fn build(config_path: PathBuf, store: ConfigStore) -> Result<Rc<Self>> {
        let mut app = Self {
            config_path,
            store: RefCell::new(store),
            current_profile_key: RefCell::new(None),
            current_combos: RefCell::new(Vec::new()),
            enabled_keys: RefCell::new(HashSet::new()),
            tray_state: Cell::new(TrayState::Disabled),
            window_hidden: Cell::new(false),
            quitting: Cell::new(false),
            runner: RefCell::new(None),
            runner_events: RefCell::new(None),
            combo_edit_index: Cell::new(None),
            combo_sequence: RefCell::new(Vec::new()),
            keyboard_buttons: RefCell::new(Vec::new()),
            evt_handlers: RefCell::new(Vec::new()),

            window: Default::default(),
            notice: Default::default(),
            main_layout: Default::default(),
            left_frame: Default::default(),
            right_frame: Default::default(),
            left_layout: Default::default(),
            right_layout: Default::default(),

            profiles_label: Default::default(),
            default_label: Default::default(),
            profile_list: Default::default(),
            new_button: Default::default(),
            save_button: Default::default(),
            delete_button: Default::default(),
            default_button: Default::default(),

            editor_label: Default::default(),
            name_label: Default::default(),
            name_input: Default::default(),
            keys_label: Default::default(),
            keys_hint_label: Default::default(),
            keyboard_frame: Default::default(),
            repeat_label: Default::default(),
            repeat_input: Default::default(),
            press_label: Default::default(),
            press_input: Default::default(),
            poll_label: Default::default(),
            poll_input: Default::default(),
            windows_label: Default::default(),
            windows_text: Default::default(),
            combos_label: Default::default(),
            combo_list: Default::default(),
            combo_add_button: Default::default(),
            combo_edit_button: Default::default(),
            combo_remove_button: Default::default(),
            start_button: Default::default(),
            stop_button: Default::default(),
            status_label: Default::default(),
            tray_enabled_icon: Default::default(),
            tray_paused_icon: Default::default(),
            tray_disabled_icon: Default::default(),
            tray: Default::default(),
            tray_menu: Default::default(),
            tray_status_enabled: Default::default(),
            tray_status_paused: Default::default(),
            tray_status_disabled: Default::default(),
            tray_status_separator: Default::default(),
            tray_show_item: Default::default(),
            tray_hide_item: Default::default(),
            tray_window_separator: Default::default(),
            tray_start_item: Default::default(),
            tray_stop_item: Default::default(),
            tray_action_separator: Default::default(),
            tray_exit_item: Default::default(),

            combo_window: Default::default(),
            combo_layout: Default::default(),
            combo_header_label: Default::default(),
            combo_name_label: Default::default(),
            combo_name_input: Default::default(),
            combo_trigger_label: Default::default(),
            combo_trigger_combo: Default::default(),
            combo_step_label: Default::default(),
            combo_step_input: Default::default(),
            combo_press_label: Default::default(),
            combo_press_input: Default::default(),
            combo_sequence_label: Default::default(),
            combo_sequence_list: Default::default(),
            combo_available_key_label: Default::default(),
            combo_available_key_combo: Default::default(),
            combo_add_step_button: Default::default(),
            combo_remove_step_button: Default::default(),
            combo_up_button: Default::default(),
            combo_down_button: Default::default(),
            combo_save_button: Default::default(),
            combo_cancel_button: Default::default(),
        };

        app.build_main_window()?;
        app.build_combo_dialog()?;
        app.setup_initial_state()?;

        let app = Rc::new(app);
        app.bind_events();
        Ok(app)
    }

    fn build_main_window(&mut self) -> Result<()> {
        nwg::Window::builder()
            .size((1340, 900))
            .position((200, 120))
            .title("DNFAutoFire 配置")
            .flags(nwg::WindowFlags::MAIN_WINDOW | nwg::WindowFlags::VISIBLE)
            .build(&mut self.window)?;

        nwg::Notice::builder()
            .parent(&self.window)
            .build(&mut self.notice)?;

        nwg::Frame::builder()
            .parent(&self.window)
            .flags(nwg::FrameFlags::VISIBLE | nwg::FrameFlags::BORDER)
            .build(&mut self.left_frame)?;

        nwg::Frame::builder()
            .parent(&self.window)
            .flags(nwg::FrameFlags::VISIBLE | nwg::FrameFlags::BORDER)
            .build(&mut self.right_frame)?;

        nwg::GridLayout::builder()
            .parent(&self.window)
            .margin([8, 8, 8, 8])
            .spacing(8)
            .max_column(Some(4))
            .child_item(nwg::GridLayoutItem::new(&self.left_frame, 0, 0, 1, 1))
            .child_item(nwg::GridLayoutItem::new(&self.right_frame, 1, 0, 3, 1))
            .build(&self.main_layout)?;

        self.build_left_panel()?;
        self.build_right_panel()?;
        self.build_tray()?;
        self.stop_button.set_enabled(false);
        self.status_label.set_text(STATUS_IDLE);
        self.sync_tray_ui();
        Ok(())
    }

    fn build_left_panel(&mut self) -> Result<()> {
        nwg::Label::builder()
            .text("配置列表")
            .parent(&self.left_frame)
            .build(&mut self.profiles_label)?;

        nwg::Label::builder()
            .text("默认配置: -")
            .parent(&self.left_frame)
            .build(&mut self.default_label)?;

        nwg::ListBox::builder()
            .parent(&self.left_frame)
            .build(&mut self.profile_list)?;

        nwg::Button::builder()
            .text("新建")
            .parent(&self.left_frame)
            .build(&mut self.new_button)?;

        nwg::Button::builder()
            .text("保存")
            .parent(&self.left_frame)
            .build(&mut self.save_button)?;

        nwg::Button::builder()
            .text("删除")
            .parent(&self.left_frame)
            .build(&mut self.delete_button)?;

        nwg::Button::builder()
            .text("设为默认")
            .parent(&self.left_frame)
            .build(&mut self.default_button)?;

        nwg::GridLayout::builder()
            .parent(&self.left_frame)
            .margin([8, 8, 8, 8])
            .spacing(6)
            .max_column(Some(2))
            .child_item(nwg::GridLayoutItem::new(&self.profiles_label, 0, 0, 2, 1))
            .child_item(nwg::GridLayoutItem::new(&self.default_label, 0, 1, 2, 1))
            .child_item(nwg::GridLayoutItem::new(&self.profile_list, 0, 2, 2, 6))
            .child(0, 8, &self.new_button)
            .child(1, 8, &self.save_button)
            .child(0, 9, &self.delete_button)
            .child(1, 9, &self.default_button)
            .build(&self.left_layout)?;

        Ok(())
    }

    fn build_right_panel(&mut self) -> Result<()> {
        let numeric_flags = nwg::TextInputFlags::VISIBLE
            | nwg::TextInputFlags::TAB_STOP
            | nwg::TextInputFlags::NUMBER;

        nwg::Label::builder()
            .text("当前配置")
            .parent(&self.right_frame)
            .build(&mut self.editor_label)?;

        nwg::Label::builder()
            .text("配置名")
            .parent(&self.right_frame)
            .build(&mut self.name_label)?;

        nwg::TextInput::builder()
            .parent(&self.right_frame)
            .build(&mut self.name_input)?;

        nwg::Label::builder()
            .text("启用按键")
            .parent(&self.right_frame)
            .build(&mut self.keys_label)?;

        nwg::Label::builder()
            .text("点击键帽切换是否加入连发。带 '>' 的键表示当前已启用。右上角灰色键仅展示布局，不参与配置。")
            .parent(&self.right_frame)
            .build(&mut self.keys_hint_label)?;

        nwg::Frame::builder()
            .parent(&self.right_frame)
            .size((KEYBOARD_FRAME_WIDTH, KEYBOARD_FRAME_HEIGHT))
            .flags(nwg::FrameFlags::VISIBLE | nwg::FrameFlags::BORDER)
            .build(&mut self.keyboard_frame)?;
        self.build_keyboard_buttons()?;

        nwg::Label::builder()
            .text("连发间隔(ms)")
            .parent(&self.right_frame)
            .build(&mut self.repeat_label)?;

        nwg::TextInput::builder()
            .parent(&self.right_frame)
            .flags(numeric_flags)
            .build(&mut self.repeat_input)?;

        nwg::Label::builder()
            .text("按下时长(ms)")
            .parent(&self.right_frame)
            .build(&mut self.press_label)?;

        nwg::TextInput::builder()
            .parent(&self.right_frame)
            .flags(numeric_flags)
            .build(&mut self.press_input)?;

        nwg::Label::builder()
            .text("轮询间隔(ms)")
            .parent(&self.right_frame)
            .build(&mut self.poll_label)?;

        nwg::TextInput::builder()
            .parent(&self.right_frame)
            .flags(numeric_flags)
            .build(&mut self.poll_input)?;

        nwg::Label::builder()
            .text("目标窗口关键字（每行一个）")
            .parent(&self.right_frame)
            .build(&mut self.windows_label)?;

        nwg::TextBox::builder()
            .parent(&self.right_frame)
            .flags(
                nwg::TextBoxFlags::VISIBLE
                    | nwg::TextBoxFlags::TAB_STOP
                    | nwg::TextBoxFlags::VSCROLL
                    | nwg::TextBoxFlags::AUTOVSCROLL,
            )
            .build(&mut self.windows_text)?;

        nwg::Label::builder()
            .text("一键连招")
            .parent(&self.right_frame)
            .build(&mut self.combos_label)?;

        nwg::ListView::builder()
            .parent(&self.right_frame)
            .list_style(nwg::ListViewStyle::Detailed)
            .build(&mut self.combo_list)?;
        self.combo_list.insert_column(nwg::InsertListViewColumn {
            index: Some(0),
            width: Some(140),
            text: Some("名称".to_string()),
            fmt: None,
        });
        self.combo_list.insert_column(nwg::InsertListViewColumn {
            index: Some(1),
            width: Some(90),
            text: Some("触发键".to_string()),
            fmt: None,
        });
        self.combo_list.insert_column(nwg::InsertListViewColumn {
            index: Some(2),
            width: Some(360),
            text: Some("步骤".to_string()),
            fmt: None,
        });
        self.combo_list.insert_column(nwg::InsertListViewColumn {
            index: Some(3),
            width: Some(90),
            text: Some("步进间隔".to_string()),
            fmt: None,
        });

        nwg::Button::builder()
            .text("新增连招")
            .parent(&self.right_frame)
            .build(&mut self.combo_add_button)?;

        nwg::Button::builder()
            .text("编辑连招")
            .parent(&self.right_frame)
            .build(&mut self.combo_edit_button)?;

        nwg::Button::builder()
            .text("删除连招")
            .parent(&self.right_frame)
            .build(&mut self.combo_remove_button)?;

        nwg::Button::builder()
            .text("启动")
            .parent(&self.right_frame)
            .build(&mut self.start_button)?;

        nwg::Button::builder()
            .text("停止")
            .parent(&self.right_frame)
            .build(&mut self.stop_button)?;

        nwg::Label::builder()
            .text(STATUS_IDLE)
            .parent(&self.right_frame)
            .build(&mut self.status_label)?;

        nwg::GridLayout::builder()
            .parent(&self.right_frame)
            .margin([8, 8, 8, 8])
            .spacing(6)
            .max_column(Some(6))
            .child_item(nwg::GridLayoutItem::new(&self.editor_label, 0, 0, 6, 1))
            .child(0, 1, &self.name_label)
            .child_item(nwg::GridLayoutItem::new(&self.name_input, 1, 1, 5, 1))
            .child_item(nwg::GridLayoutItem::new(&self.keys_label, 0, 2, 6, 1))
            .child_item(nwg::GridLayoutItem::new(&self.keys_hint_label, 0, 3, 6, 1))
            .child_item(nwg::GridLayoutItem::new(&self.keyboard_frame, 0, 4, 6, 4))
            .child(0, 8, &self.repeat_label)
            .child_item(nwg::GridLayoutItem::new(&self.repeat_input, 1, 8, 1, 1))
            .child(2, 8, &self.press_label)
            .child_item(nwg::GridLayoutItem::new(&self.press_input, 3, 8, 1, 1))
            .child(4, 8, &self.poll_label)
            .child_item(nwg::GridLayoutItem::new(&self.poll_input, 5, 8, 1, 1))
            .child_item(nwg::GridLayoutItem::new(&self.windows_label, 0, 9, 6, 1))
            .child_item(nwg::GridLayoutItem::new(&self.windows_text, 0, 10, 6, 2))
            .child_item(nwg::GridLayoutItem::new(&self.combos_label, 0, 12, 6, 1))
            .child_item(nwg::GridLayoutItem::new(&self.combo_list, 0, 13, 6, 4))
            .child(0, 17, &self.combo_add_button)
            .child(1, 17, &self.combo_edit_button)
            .child(2, 17, &self.combo_remove_button)
            .child(3, 17, &self.start_button)
            .child(4, 17, &self.stop_button)
            .child_item(nwg::GridLayoutItem::new(&self.status_label, 0, 18, 6, 1))
            .build(&self.right_layout)?;

        Ok(())
    }

    fn build_tray(&mut self) -> Result<()> {
        self.tray_enabled_icon = nwg::Icon::from_system(nwg::OemIcon::Information);
        self.tray_paused_icon = nwg::Icon::from_system(nwg::OemIcon::Warning);
        self.tray_disabled_icon = nwg::Icon::from_system(nwg::OemIcon::Error);

        nwg::TrayNotification::builder()
            .parent(&self.window)
            .icon(Some(&self.tray_disabled_icon))
            .tip(Some("DNFAutoFire - 连发已关闭"))
            .build(&mut self.tray)?;

        nwg::Menu::builder()
            .popup(true)
            .parent(&self.window)
            .build(&mut self.tray_menu)?;

        nwg::MenuItem::builder()
            .text("状态: 连发已开启")
            .disabled(true)
            .parent(&self.tray_menu)
            .build(&mut self.tray_status_enabled)?;

        nwg::MenuItem::builder()
            .text("状态: 输入法暂停")
            .disabled(true)
            .parent(&self.tray_menu)
            .build(&mut self.tray_status_paused)?;

        nwg::MenuItem::builder()
            .text("状态: 连发已关闭")
            .disabled(true)
            .parent(&self.tray_menu)
            .build(&mut self.tray_status_disabled)?;

        nwg::MenuSeparator::builder()
            .parent(&self.tray_menu)
            .build(&mut self.tray_status_separator)?;

        nwg::MenuItem::builder()
            .text("显示窗口")
            .parent(&self.tray_menu)
            .build(&mut self.tray_show_item)?;

        nwg::MenuItem::builder()
            .text("隐藏窗口")
            .parent(&self.tray_menu)
            .build(&mut self.tray_hide_item)?;

        nwg::MenuSeparator::builder()
            .parent(&self.tray_menu)
            .build(&mut self.tray_window_separator)?;

        nwg::MenuItem::builder()
            .text("启动连发")
            .parent(&self.tray_menu)
            .build(&mut self.tray_start_item)?;

        nwg::MenuItem::builder()
            .text("停止连发")
            .parent(&self.tray_menu)
            .build(&mut self.tray_stop_item)?;

        nwg::MenuSeparator::builder()
            .parent(&self.tray_menu)
            .build(&mut self.tray_action_separator)?;

        nwg::MenuItem::builder()
            .text("退出")
            .parent(&self.tray_menu)
            .build(&mut self.tray_exit_item)?;

        Ok(())
    }

    fn build_keyboard_buttons(&mut self) -> Result<()> {
        let mut buttons = Vec::new();

        for key in keyboard_layout_keys() {
            let supported = parse_single_key(key.token).is_ok();
            let mut button = nwg::Button::default();
            nwg::Button::builder()
                .text(key.label)
                .parent(&self.keyboard_frame)
                .position((key.x, key.y))
                .size((key.width, key.height))
                .build(&mut button)?;
            button.set_enabled(supported);

            buttons.push(KeyboardButton {
                token: key.token,
                label: key.label,
                supported,
                button,
            });
        }

        *self.keyboard_buttons.get_mut() = buttons;
        Ok(())
    }

    fn build_combo_dialog(&mut self) -> Result<()> {
        let numeric_flags = nwg::TextInputFlags::VISIBLE
            | nwg::TextInputFlags::TAB_STOP
            | nwg::TextInputFlags::NUMBER;

        nwg::Window::builder()
            .size((620, 470))
            .position((260, 180))
            .title("新增连招")
            .flags(nwg::WindowFlags::WINDOW)
            .parent(Some(&self.window))
            .build(&mut self.combo_window)?;

        nwg::Label::builder()
            .text("连招编辑")
            .parent(&self.combo_window)
            .build(&mut self.combo_header_label)?;

        nwg::Label::builder()
            .text("名称")
            .parent(&self.combo_window)
            .build(&mut self.combo_name_label)?;

        nwg::TextInput::builder()
            .parent(&self.combo_window)
            .build(&mut self.combo_name_input)?;

        nwg::Label::builder()
            .text("触发键")
            .parent(&self.combo_window)
            .build(&mut self.combo_trigger_label)?;

        nwg::ComboBox::builder()
            .parent(&self.combo_window)
            .collection(supported_keys_vec())
            .selected_index(Some(0))
            .build(&mut self.combo_trigger_combo)?;

        nwg::Label::builder()
            .text("步进间隔(ms)")
            .parent(&self.combo_window)
            .build(&mut self.combo_step_label)?;

        nwg::TextInput::builder()
            .parent(&self.combo_window)
            .flags(numeric_flags)
            .build(&mut self.combo_step_input)?;

        nwg::Label::builder()
            .text("按下时长(ms)")
            .parent(&self.combo_window)
            .build(&mut self.combo_press_label)?;

        nwg::TextInput::builder()
            .parent(&self.combo_window)
            .flags(numeric_flags)
            .build(&mut self.combo_press_input)?;

        nwg::Label::builder()
            .text("顺序按键")
            .parent(&self.combo_window)
            .build(&mut self.combo_sequence_label)?;

        nwg::ListBox::builder()
            .parent(&self.combo_window)
            .build(&mut self.combo_sequence_list)?;

        nwg::Label::builder()
            .text("添加按键")
            .parent(&self.combo_window)
            .build(&mut self.combo_available_key_label)?;

        nwg::ComboBox::builder()
            .parent(&self.combo_window)
            .collection(supported_keys_vec())
            .selected_index(Some(0))
            .build(&mut self.combo_available_key_combo)?;

        nwg::Button::builder()
            .text("添加步骤")
            .parent(&self.combo_window)
            .build(&mut self.combo_add_step_button)?;

        nwg::Button::builder()
            .text("删除步骤")
            .parent(&self.combo_window)
            .build(&mut self.combo_remove_step_button)?;

        nwg::Button::builder()
            .text("上移")
            .parent(&self.combo_window)
            .build(&mut self.combo_up_button)?;

        nwg::Button::builder()
            .text("下移")
            .parent(&self.combo_window)
            .build(&mut self.combo_down_button)?;

        nwg::Button::builder()
            .text("保存连招")
            .parent(&self.combo_window)
            .build(&mut self.combo_save_button)?;

        nwg::Button::builder()
            .text("取消")
            .parent(&self.combo_window)
            .build(&mut self.combo_cancel_button)?;

        nwg::GridLayout::builder()
            .parent(&self.combo_window)
            .margin([8, 8, 8, 8])
            .spacing(6)
            .max_column(Some(4))
            .child_item(nwg::GridLayoutItem::new(
                &self.combo_header_label,
                0,
                0,
                4,
                1,
            ))
            .child(0, 1, &self.combo_name_label)
            .child_item(nwg::GridLayoutItem::new(&self.combo_name_input, 1, 1, 3, 1))
            .child(0, 2, &self.combo_trigger_label)
            .child(1, 2, &self.combo_trigger_combo)
            .child(2, 2, &self.combo_step_label)
            .child(3, 2, &self.combo_step_input)
            .child(2, 3, &self.combo_press_label)
            .child(3, 3, &self.combo_press_input)
            .child_item(nwg::GridLayoutItem::new(
                &self.combo_sequence_label,
                0,
                4,
                4,
                1,
            ))
            .child_item(nwg::GridLayoutItem::new(
                &self.combo_sequence_list,
                0,
                5,
                2,
                4,
            ))
            .child_item(nwg::GridLayoutItem::new(
                &self.combo_available_key_label,
                2,
                5,
                2,
                1,
            ))
            .child_item(nwg::GridLayoutItem::new(
                &self.combo_available_key_combo,
                2,
                6,
                2,
                1,
            ))
            .child(2, 7, &self.combo_add_step_button)
            .child(3, 7, &self.combo_remove_step_button)
            .child(2, 8, &self.combo_up_button)
            .child(3, 8, &self.combo_down_button)
            .child(2, 9, &self.combo_save_button)
            .child(3, 9, &self.combo_cancel_button)
            .build(&self.combo_layout)?;

        self.combo_window.set_visible(false);
        Ok(())
    }

    fn setup_initial_state(&self) -> Result<()> {
        self.refresh_profile_list(None);
        self.update_default_label();

        let default_name = self.store.borrow().default_profile.clone();
        self.load_profile_from_store(&default_name)?;
        self.set_editing_enabled(true);
        self.set_tray_state(TrayState::Disabled);
        Ok(())
    }

    fn bind_events(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let main_handler =
            nwg::full_bind_event_handler(&self.window.handle, move |evt, evt_data, handle| {
                if let Some(app) = weak.upgrade() {
                    app.handle_event(evt, evt_data, handle);
                }
            });
        self.evt_handlers.borrow_mut().push(main_handler);

        let weak = Rc::downgrade(self);
        let combo_handler = nwg::full_bind_event_handler(
            &self.combo_window.handle,
            move |evt, evt_data, handle| {
                if let Some(app) = weak.upgrade() {
                    app.handle_event(evt, evt_data, handle);
                }
            },
        );
        self.evt_handlers.borrow_mut().push(combo_handler);
    }

    fn show_tray_menu(&self) {
        let (x, y) = current_cursor_position();
        self.tray_menu.popup(x, y);
    }

    fn toggle_main_window_visibility(&self) {
        if self.window_hidden.get() {
            self.show_main_window();
        } else {
            self.hide_main_window_to_tray();
        }
    }

    fn show_main_window(&self) {
        self.window.set_visible(true);
        self.window.restore();
        self.window.set_focus();
        self.window_hidden.set(false);
        self.sync_tray_ui();
    }

    fn hide_main_window_to_tray(&self) {
        if self.combo_window.visible() {
            self.close_combo_dialog();
        }
        self.window.set_visible(false);
        self.window_hidden.set(true);
        self.sync_tray_ui();
    }

    fn set_tray_state(&self, state: TrayState) {
        self.tray_state.set(state);
        self.sync_tray_ui();
    }

    fn sync_tray_ui(&self) {
        let (tip, icon) = match self.tray_state.get() {
            TrayState::Disabled => ("DNFAutoFire - 连发已关闭", &self.tray_disabled_icon),
            TrayState::Enabled => ("DNFAutoFire - 连发已开启", &self.tray_enabled_icon),
            TrayState::Paused => ("DNFAutoFire - 输入法暂停", &self.tray_paused_icon),
        };

        self.tray.set_icon(icon);
        self.tray.set_tip(tip);

        self.tray_status_enabled
            .set_checked(self.tray_state.get() == TrayState::Enabled);
        self.tray_status_paused
            .set_checked(self.tray_state.get() == TrayState::Paused);
        self.tray_status_disabled
            .set_checked(self.tray_state.get() == TrayState::Disabled);

        self.tray_show_item.set_enabled(self.window_hidden.get());
        self.tray_hide_item.set_enabled(!self.window_hidden.get());

        let running = self.tray_state.get() != TrayState::Disabled;
        self.tray_start_item.set_enabled(!running);
        self.tray_stop_item.set_enabled(running);
    }

    fn handle_tray_menu_action(&self, handle: nwg::ControlHandle) {
        let action = if handle == self.tray_show_item.handle {
            self.show_main_window();
            Ok(())
        } else if handle == self.tray_hide_item.handle {
            self.hide_main_window_to_tray();
            Ok(())
        } else if handle == self.tray_start_item.handle {
            self.start_runner_from_form()
        } else if handle == self.tray_stop_item.handle {
            self.request_stop_runner();
            Ok(())
        } else if handle == self.tray_exit_item.handle {
            self.exit_application();
            Ok(())
        } else {
            Ok(())
        };

        if let Err(err) = action {
            self.show_error(&format!("{err:#}"));
        }
    }

    fn exit_application(&self) {
        self.quitting.set(true);
        self.shutdown_runner();
        nwg::stop_thread_dispatch();
    }

    fn handle_event(&self, evt: nwg::Event, evt_data: nwg::EventData, handle: nwg::ControlHandle) {
        match evt {
            nwg::Event::OnWindowClose => {
                if handle == self.combo_window.handle {
                    if let nwg::EventData::OnWindowClose(data) = evt_data {
                        data.close(false);
                    }
                    self.close_combo_dialog();
                    return;
                }

                if handle == self.window.handle {
                    if let nwg::EventData::OnWindowClose(data) = evt_data {
                        data.close(false);
                    }
                    if self.quitting.get() {
                        self.shutdown_runner();
                        nwg::stop_thread_dispatch();
                    } else {
                        self.hide_main_window_to_tray();
                    }
                }
            }
            nwg::Event::OnWindowMinimize => {
                if handle == self.window.handle {
                    self.hide_main_window_to_tray();
                }
            }
            nwg::Event::OnNotice => {
                if handle == self.notice.handle {
                    self.drain_runner_events();
                }
            }
            nwg::Event::OnContextMenu => {
                if handle == self.tray.handle {
                    self.show_tray_menu();
                }
            }
            nwg::Event::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) => {
                if handle == self.tray.handle {
                    self.toggle_main_window_visibility();
                }
            }
            nwg::Event::OnListBoxSelect => {
                if handle == self.profile_list.handle {
                    if let Some(name) = self.profile_list.selection_string()
                        && let Err(err) = self.load_profile_from_store(&name)
                    {
                        self.show_error(&format!("{err:#}"));
                    }
                }
            }
            nwg::Event::OnMenuItemSelected => {
                self.handle_tray_menu_action(handle);
            }
            nwg::Event::OnButtonClick => {
                self.handle_button_click(handle);
            }
            _ => {}
        }
    }

    fn handle_button_click(&self, handle: nwg::ControlHandle) {
        let action = if self.toggle_keyboard_button(handle) {
            Ok(())
        } else if handle == self.new_button.handle {
            self.new_profile()
        } else if handle == self.save_button.handle {
            self.save_profile()
        } else if handle == self.delete_button.handle {
            self.delete_profile()
        } else if handle == self.default_button.handle {
            self.set_default_profile()
        } else if handle == self.start_button.handle {
            self.start_runner_from_form()
        } else if handle == self.stop_button.handle {
            self.request_stop_runner();
            Ok(())
        } else if handle == self.combo_add_button.handle {
            self.open_combo_dialog(None);
            Ok(())
        } else if handle == self.combo_edit_button.handle {
            self.edit_selected_combo()
        } else if handle == self.combo_remove_button.handle {
            self.remove_selected_combo()
        } else if handle == self.combo_add_step_button.handle {
            self.add_combo_step()
        } else if handle == self.combo_remove_step_button.handle {
            self.remove_combo_step()
        } else if handle == self.combo_up_button.handle {
            self.move_combo_step_up()
        } else if handle == self.combo_down_button.handle {
            self.move_combo_step_down()
        } else if handle == self.combo_save_button.handle {
            self.save_combo_dialog()
        } else if handle == self.combo_cancel_button.handle {
            self.close_combo_dialog();
            Ok(())
        } else {
            Ok(())
        };

        if let Err(err) = action {
            self.show_error(&format!("{err:#}"));
        }
    }

    fn toggle_keyboard_button(&self, handle: nwg::ControlHandle) -> bool {
        let Some(token) = self
            .keyboard_buttons
            .borrow()
            .iter()
            .find(|entry| entry.button.handle == handle && entry.supported)
            .map(|entry| entry.token.to_string())
        else {
            return false;
        };

        let mut enabled = self.enabled_keys.borrow_mut();
        if !enabled.remove(&token) {
            enabled.insert(token);
        }
        drop(enabled);

        self.refresh_keyboard_buttons();
        true
    }

    fn load_profile_from_store(&self, name: &str) -> Result<()> {
        let profile = self
            .store
            .borrow()
            .get_profile(Some(name))
            .with_context(|| format!("配置不存在: {name}"))?;
        let draft = ProfileDraft::from_named_profile(name, &profile);

        *self.current_profile_key.borrow_mut() = Some(name.to_string());
        *self.current_combos.borrow_mut() = draft.combos.clone();
        self.populate_form(&draft);
        self.refresh_profile_list(Some(name));
        self.refresh_combo_list();
        self.update_default_label();
        self.status_label.set_text(STATUS_IDLE);
        Ok(())
    }

    fn populate_form(&self, draft: &ProfileDraft) {
        self.name_input.set_text(&draft.name);
        self.repeat_input.set_text(&draft.repeat_interval_ms);
        self.press_input.set_text(&draft.press_duration_ms);
        self.poll_input.set_text(&draft.poll_interval_ms);
        self.windows_text.set_text(&draft.target_windows_text);
        self.sync_enabled_keys(&draft.enabled_keys);
    }

    fn sync_enabled_keys(&self, enabled_keys: &[String]) {
        let mut current = self.enabled_keys.borrow_mut();
        current.clear();
        for key in enabled_keys {
            if let Ok(spec) = parse_single_key(key) {
                current.insert(spec.name.to_string());
            }
        }
        drop(current);
        self.refresh_keyboard_buttons();
    }

    fn refresh_keyboard_buttons(&self) {
        let enabled = self.enabled_keys.borrow();
        for entry in self.keyboard_buttons.borrow().iter() {
            let text = if entry.supported && enabled.contains(entry.token) {
                format!(">{}", entry.label)
            } else {
                entry.label.to_string()
            };
            entry.button.set_text(&text);
        }
    }

    fn refresh_profile_list(&self, selected: Option<&str>) {
        let names = self
            .store
            .borrow()
            .list_profile_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        self.profile_list.set_collection(names.clone());
        if let Some(selected_name) = selected {
            if let Some(index) = names.iter().position(|name| name == selected_name) {
                self.profile_list.set_selection(Some(index));
            } else {
                self.profile_list.set_selection(None);
            }
        } else {
            self.profile_list.set_selection(None);
        }
    }

    fn update_default_label(&self) {
        let default_name = self.store.borrow().default_profile.clone();
        self.default_label
            .set_text(&format!("默认配置: {default_name}"));
    }

    fn refresh_combo_list(&self) {
        self.combo_list.clear();
        for combo in self.current_combos.borrow().iter() {
            let steps = combo.sequence_keys.join(",");
            let interval = format!("{} ms", combo.step_interval_ms);
            let row = [
                combo.name.as_str(),
                combo.trigger_key.as_str(),
                steps.as_str(),
                interval.as_str(),
            ];
            self.combo_list.insert_items_row(None, &row);
        }
    }

    fn build_draft_from_form(&self) -> ProfileDraft {
        ProfileDraft {
            name: self.name_input.text(),
            enabled_keys: self.selected_enabled_keys(),
            repeat_interval_ms: self.repeat_input.text(),
            press_duration_ms: self.press_input.text(),
            poll_interval_ms: self.poll_input.text(),
            target_windows_text: self.windows_text.text(),
            combos: self.current_combos.borrow().clone(),
        }
    }

    fn selected_enabled_keys(&self) -> Vec<String> {
        let enabled = self.enabled_keys.borrow();
        supported_key_names()
            .iter()
            .filter(|name| enabled.contains(**name))
            .map(|name| (*name).to_string())
            .collect()
    }

    fn new_profile(&self) -> Result<()> {
        let name = self.generate_profile_name();
        let draft = ProfileDraft::from_named_profile(&name, &Profile::default());
        *self.current_profile_key.borrow_mut() = None;
        *self.current_combos.borrow_mut() = draft.combos.clone();
        self.populate_form(&draft);
        self.refresh_combo_list();
        self.refresh_profile_list(None);
        self.status_label.set_text(STATUS_IDLE);
        Ok(())
    }

    fn save_profile(&self) -> Result<()> {
        let draft = self.build_draft_from_form();
        let (new_name, profile) = self.validate_draft(&draft)?;
        let original_name = self.current_profile_key.borrow().clone();
        let mut store = self.store.borrow_mut();

        if let Some(original_name) = &original_name {
            if original_name != &new_name && store.profiles.contains_key(&new_name) {
                bail!("已存在同名配置: {new_name}");
            }
        } else if store.profiles.contains_key(&new_name) {
            bail!("已存在同名配置: {new_name}");
        }

        let renamed_default = original_name
            .as_ref()
            .map(|original_name| {
                original_name == &store.default_profile && original_name != &new_name
            })
            .unwrap_or(false);

        if let Some(original_name) = original_name
            && original_name != new_name
        {
            store.profiles.remove(&original_name);
        }

        store.upsert_profile(new_name.clone(), profile);
        if renamed_default {
            store.default_profile = new_name.clone();
        }
        store.save(&self.config_path)?;

        drop(store);
        *self.current_profile_key.borrow_mut() = Some(new_name.clone());
        self.refresh_profile_list(Some(&new_name));
        self.update_default_label();
        self.status_label.set_text(STATUS_STOPPED);
        Ok(())
    }

    fn delete_profile(&self) -> Result<()> {
        let current_key = self
            .current_profile_key
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("当前是未保存的新配置，不能直接删除"))?;
        let current_name = self.name_input.text();
        if current_name.trim() != current_key {
            bail!("当前配置名已修改，请先保存后再删除");
        }

        {
            let mut store = self.store.borrow_mut();
            store.delete_profile(&current_key)?;
            store.save(&self.config_path)?;
        }

        let next_name = self.store.borrow().default_profile.clone();
        self.load_profile_from_store(&next_name)?;
        self.status_label.set_text(STATUS_STOPPED);
        Ok(())
    }

    fn set_default_profile(&self) -> Result<()> {
        let current_key = self
            .current_profile_key
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先保存配置，再设为默认"))?;
        let current_name = self.name_input.text();
        if current_name.trim() != current_key {
            bail!("当前配置名已修改，请先保存后再设为默认");
        }

        let mut store = self.store.borrow_mut();
        store.set_default_profile(&current_key)?;
        store.save(&self.config_path)?;
        drop(store);

        self.update_default_label();
        self.refresh_profile_list(Some(&current_key));
        Ok(())
    }

    fn start_runner_from_form(&self) -> Result<()> {
        if self
            .runner
            .borrow()
            .as_ref()
            .map(RunnerHandle::is_running)
            .unwrap_or(false)
        {
            bail!("连发已经在运行");
        }

        let draft = self.build_draft_from_form();
        let (_, profile) = self.validate_draft(&draft)?;
        let (tx, rx) = channel();
        let notice_sender = self.notice.sender();
        let handle = AutoFireService::start_with_events(profile, move |event| {
            let _ = tx.send(event);
            notice_sender.notice();
        })?;

        *self.runner_events.borrow_mut() = Some(rx);
        *self.runner.borrow_mut() = Some(handle);
        self.set_editing_enabled(false);
        self.stop_button.set_enabled(true);
        self.start_button.set_enabled(false);
        self.status_label.set_text(STATUS_RUNNING);
        self.set_tray_state(TrayState::Enabled);
        Ok(())
    }

    fn request_stop_runner(&self) {
        if let Some(runner) = self
            .runner
            .borrow()
            .as_ref()
            .filter(|runner| runner.is_running())
        {
            runner.stop();
        }
    }

    fn drain_runner_events(&self) {
        let mut events = Vec::new();
        if let Some(receiver) = self.runner_events.borrow().as_ref() {
            while let Ok(event) = receiver.try_recv() {
                events.push(event);
            }
        }

        for event in events {
            match event {
                RunnerEvent::Started | RunnerEvent::ResumedFromIme => {
                    self.status_label.set_text(STATUS_RUNNING);
                    self.set_tray_state(TrayState::Enabled);
                }
                RunnerEvent::PausedByIme => {
                    self.status_label.set_text(STATUS_IME_PAUSED);
                    self.set_tray_state(TrayState::Paused);
                }
                RunnerEvent::Stopped(_) => {
                    let mut wait_error = None;
                    if let Some(mut runner) = self.runner.borrow_mut().take() {
                        if let Err(err) = runner.wait() {
                            wait_error = Some(err);
                        }
                    }
                    *self.runner_events.borrow_mut() = None;
                    self.set_editing_enabled(true);
                    self.stop_button.set_enabled(false);
                    self.start_button.set_enabled(true);
                    self.status_label.set_text(STATUS_STOPPED);
                    self.set_tray_state(TrayState::Disabled);

                    if let Some(err) = wait_error {
                        self.show_error(&format!("{err:#}"));
                    }
                }
            }
        }
    }

    fn shutdown_runner(&self) {
        let mut runner = self.runner.borrow_mut().take();
        if let Some(handle) = runner.as_ref() {
            handle.stop();
        }
        if let Some(mut handle) = runner.take() {
            let _ = handle.wait();
        }
        *self.runner_events.borrow_mut() = None;
    }

    fn set_editing_enabled(&self, enabled: bool) {
        self.profile_list.set_enabled(enabled);
        self.new_button.set_enabled(enabled);
        self.save_button.set_enabled(enabled);
        self.delete_button.set_enabled(enabled);
        self.default_button.set_enabled(enabled);
        self.name_input.set_enabled(enabled);
        for entry in self.keyboard_buttons.borrow().iter() {
            entry.button.set_enabled(enabled && entry.supported);
        }
        self.repeat_input.set_enabled(enabled);
        self.press_input.set_enabled(enabled);
        self.poll_input.set_enabled(enabled);
        self.windows_text.set_enabled(enabled);
        self.combo_list.set_enabled(enabled);
        self.combo_add_button.set_enabled(enabled);
        self.combo_edit_button.set_enabled(enabled);
        self.combo_remove_button.set_enabled(enabled);
    }

    fn validate_draft(&self, draft: &ProfileDraft) -> Result<(String, Profile)> {
        let (name, profile) = draft.to_named_profile()?;
        if !profile.enabled_keys.is_empty() {
            parse_key_specs(&profile.enabled_keys)?;
        }
        for combo in &profile.combos {
            parse_single_key(&combo.trigger_key)?;
            parse_key_sequence(&combo.sequence_keys)?;
        }
        Ok((name, profile))
    }

    fn edit_selected_combo(&self) -> Result<()> {
        let Some(index) = self.combo_list.selected_item() else {
            bail!("请先选中一个连招");
        };
        self.open_combo_dialog(Some(index));
        Ok(())
    }

    fn remove_selected_combo(&self) -> Result<()> {
        let Some(index) = self.combo_list.selected_item() else {
            bail!("请先选中一个连招");
        };
        self.current_combos.borrow_mut().remove(index);
        self.refresh_combo_list();
        Ok(())
    }

    fn open_combo_dialog(&self, edit_index: Option<usize>) {
        self.combo_edit_index.set(edit_index);

        let combo = edit_index.and_then(|index| self.current_combos.borrow().get(index).cloned());
        if let Some(combo) = combo {
            self.combo_window.set_text("编辑连招");
            self.combo_header_label.set_text("编辑连招");
            self.combo_name_input.set_text(&combo.name);
            let _ = self
                .combo_trigger_combo
                .set_selection_string(&combo.trigger_key);
            self.combo_step_input
                .set_text(&combo.step_interval_ms.to_string());
            self.combo_press_input
                .set_text(&combo.press_duration_ms.to_string());
            *self.combo_sequence.borrow_mut() = combo.sequence_keys;
        } else {
            self.combo_window.set_text("新增连招");
            self.combo_header_label.set_text("新增连招");
            self.combo_name_input.set_text(&self.generate_combo_name());
            self.combo_trigger_combo.set_selection(Some(0));
            self.combo_step_input.set_text("80");
            self.combo_press_input.set_text("1");
            *self.combo_sequence.borrow_mut() = Vec::new();
        }

        self.refresh_combo_sequence_list(None);
        self.window.set_enabled(false);
        self.combo_window.set_visible(true);
    }

    fn close_combo_dialog(&self) {
        self.combo_window.set_visible(false);
        self.window.set_enabled(true);
    }

    fn refresh_combo_sequence_list(&self, selected: Option<usize>) {
        let sequence = self.combo_sequence.borrow().clone();
        self.combo_sequence_list.set_collection(sequence.clone());
        if let Some(index) = selected {
            if index < sequence.len() {
                self.combo_sequence_list.set_selection(Some(index));
            } else {
                self.combo_sequence_list.set_selection(None);
            }
        } else {
            self.combo_sequence_list.set_selection(None);
        }
    }

    fn add_combo_step(&self) -> Result<()> {
        let key = self
            .combo_available_key_combo
            .selection_string()
            .ok_or_else(|| anyhow::anyhow!("请先选择要添加的按键"))?;
        self.combo_sequence.borrow_mut().push(key);
        let index = self.combo_sequence.borrow().len().saturating_sub(1);
        self.refresh_combo_sequence_list(Some(index));
        Ok(())
    }

    fn remove_combo_step(&self) -> Result<()> {
        let Some(index) = self.combo_sequence_list.selection() else {
            bail!("请先选中一个步骤");
        };
        self.combo_sequence.borrow_mut().remove(index);
        let next = index.saturating_sub(1);
        self.refresh_combo_sequence_list(Some(next));
        Ok(())
    }

    fn move_combo_step_up(&self) -> Result<()> {
        let Some(index) = self.combo_sequence_list.selection() else {
            bail!("请先选中一个步骤");
        };
        if index == 0 {
            return Ok(());
        }

        self.combo_sequence.borrow_mut().swap(index, index - 1);
        self.refresh_combo_sequence_list(Some(index - 1));
        Ok(())
    }

    fn move_combo_step_down(&self) -> Result<()> {
        let Some(index) = self.combo_sequence_list.selection() else {
            bail!("请先选中一个步骤");
        };

        let len = self.combo_sequence.borrow().len();
        if index + 1 >= len {
            return Ok(());
        }

        self.combo_sequence.borrow_mut().swap(index, index + 1);
        self.refresh_combo_sequence_list(Some(index + 1));
        Ok(())
    }

    fn save_combo_dialog(&self) -> Result<()> {
        let combo = self.read_combo_from_dialog()?;
        let edit_index = self.combo_edit_index.get();

        {
            let combos = self.current_combos.borrow();
            for (index, existing) in combos.iter().enumerate() {
                if Some(index) != edit_index && existing.name.eq_ignore_ascii_case(&combo.name) {
                    bail!("已存在同名连招: {}", combo.name);
                }
            }
        }

        let mut combos = self.current_combos.borrow_mut();
        if let Some(index) = edit_index {
            combos[index] = combo;
        } else {
            combos.push(combo);
        }

        drop(combos);
        self.refresh_combo_list();
        self.close_combo_dialog();
        Ok(())
    }

    fn read_combo_from_dialog(&self) -> Result<ComboConfig> {
        let name = self.combo_name_input.text();
        let trigger_key = self
            .combo_trigger_combo
            .selection_string()
            .ok_or_else(|| anyhow::anyhow!("请选择触发键"))?;
        let sequence_keys = self.combo_sequence.borrow().clone();
        let step_interval_ms = parse_dialog_ms(&self.combo_step_input.text(), "步进间隔")?;
        let press_duration_ms = parse_dialog_ms(&self.combo_press_input.text(), "按下时长")?;

        let combo = ComboConfig {
            name,
            trigger_key,
            sequence_keys,
            step_interval_ms,
            press_duration_ms,
        }
        .normalized();
        combo.validate()?;
        parse_single_key(&combo.trigger_key)?;
        parse_key_sequence(&combo.sequence_keys)?;
        Ok(combo)
    }

    fn generate_profile_name(&self) -> String {
        let store = self.store.borrow();
        let mut index = 1;
        loop {
            let candidate = format!("profile-{index}");
            if !store.profiles.contains_key(&candidate) {
                return candidate;
            }
            index += 1;
        }
    }

    fn generate_combo_name(&self) -> String {
        let combos = self.current_combos.borrow();
        let mut index = 1;
        loop {
            let candidate = format!("combo-{index}");
            if !combos
                .iter()
                .any(|combo| combo.name.eq_ignore_ascii_case(&candidate))
            {
                return candidate;
            }
            index += 1;
        }
    }

    fn show_error(&self, message: &str) {
        nwg::modal_error_message(&self.window, "操作失败", message);
    }
}

impl Drop for GuiApp {
    fn drop(&mut self) {
        self.shutdown_runner();
        for handler in self.evt_handlers.borrow_mut().drain(..) {
            nwg::unbind_event_handler(&handler);
        }
    }
}

fn keyboard_layout_keys() -> Vec<KeyboardLayoutKey> {
    let mut keys = Vec::new();
    let main_x = KEYBOARD_MARGIN;
    let nav_x = main_x + keyboard_units_to_px(16) + KEYBOARD_BLOCK_GAP;
    let num_x = nav_x + keyboard_units_to_px(3) + KEYBOARD_BLOCK_GAP;

    let row0 = keyboard_row_y(0);
    push_keyboard_row(
        &mut keys,
        main_x,
        row0,
        &[KeyboardCell {
            token: "ESC",
            label: "Esc",
            width_units: 1,
        }],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(2),
        row0,
        &[
            KeyboardCell {
                token: "F1",
                label: "F1",
                width_units: 1,
            },
            KeyboardCell {
                token: "F2",
                label: "F2",
                width_units: 1,
            },
            KeyboardCell {
                token: "F3",
                label: "F3",
                width_units: 1,
            },
            KeyboardCell {
                token: "F4",
                label: "F4",
                width_units: 1,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(6) + KEYBOARD_BLOCK_GAP,
        row0,
        &[
            KeyboardCell {
                token: "F5",
                label: "F5",
                width_units: 1,
            },
            KeyboardCell {
                token: "F6",
                label: "F6",
                width_units: 1,
            },
            KeyboardCell {
                token: "F7",
                label: "F7",
                width_units: 1,
            },
            KeyboardCell {
                token: "F8",
                label: "F8",
                width_units: 1,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(10) + KEYBOARD_BLOCK_GAP * 2,
        row0,
        &[
            KeyboardCell {
                token: "F9",
                label: "F9",
                width_units: 1,
            },
            KeyboardCell {
                token: "F10",
                label: "F10",
                width_units: 1,
            },
            KeyboardCell {
                token: "F11",
                label: "F11",
                width_units: 1,
            },
            KeyboardCell {
                token: "F12",
                label: "F12",
                width_units: 1,
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
                width_units: 1,
            },
            KeyboardCell {
                token: "SCROLLLOCK",
                label: "ScrLk",
                width_units: 1,
            },
            KeyboardCell {
                token: "PAUSE",
                label: "Pause",
                width_units: 1,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(1),
        &[
            KeyboardCell {
                token: "BACKQUOTE",
                label: "`",
                width_units: 1,
            },
            KeyboardCell {
                token: "1",
                label: "1",
                width_units: 1,
            },
            KeyboardCell {
                token: "2",
                label: "2",
                width_units: 1,
            },
            KeyboardCell {
                token: "3",
                label: "3",
                width_units: 1,
            },
            KeyboardCell {
                token: "4",
                label: "4",
                width_units: 1,
            },
            KeyboardCell {
                token: "5",
                label: "5",
                width_units: 1,
            },
            KeyboardCell {
                token: "6",
                label: "6",
                width_units: 1,
            },
            KeyboardCell {
                token: "7",
                label: "7",
                width_units: 1,
            },
            KeyboardCell {
                token: "8",
                label: "8",
                width_units: 1,
            },
            KeyboardCell {
                token: "9",
                label: "9",
                width_units: 1,
            },
            KeyboardCell {
                token: "0",
                label: "0",
                width_units: 1,
            },
            KeyboardCell {
                token: "MINUS",
                label: "-",
                width_units: 1,
            },
            KeyboardCell {
                token: "EQUAL",
                label: "=",
                width_units: 1,
            },
            KeyboardCell {
                token: "BACKSPACE",
                label: "Bksp",
                width_units: 2,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(1),
        &[
            KeyboardCell {
                token: "INSERT",
                label: "Ins",
                width_units: 1,
            },
            KeyboardCell {
                token: "HOME",
                label: "Home",
                width_units: 1,
            },
            KeyboardCell {
                token: "PAGEUP",
                label: "PgUp",
                width_units: 1,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(1),
        &[
            KeyboardCell {
                token: "NUMLOCK",
                label: "Num",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUMDIV",
                label: "/",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUMMUL",
                label: "*",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUMMINUS",
                label: "-",
                width_units: 1,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(2),
        &[
            KeyboardCell {
                token: "TAB",
                label: "Tab",
                width_units: 2,
            },
            KeyboardCell {
                token: "Q",
                label: "Q",
                width_units: 1,
            },
            KeyboardCell {
                token: "W",
                label: "W",
                width_units: 1,
            },
            KeyboardCell {
                token: "E",
                label: "E",
                width_units: 1,
            },
            KeyboardCell {
                token: "R",
                label: "R",
                width_units: 1,
            },
            KeyboardCell {
                token: "T",
                label: "T",
                width_units: 1,
            },
            KeyboardCell {
                token: "Y",
                label: "Y",
                width_units: 1,
            },
            KeyboardCell {
                token: "U",
                label: "U",
                width_units: 1,
            },
            KeyboardCell {
                token: "I",
                label: "I",
                width_units: 1,
            },
            KeyboardCell {
                token: "O",
                label: "O",
                width_units: 1,
            },
            KeyboardCell {
                token: "P",
                label: "P",
                width_units: 1,
            },
            KeyboardCell {
                token: "LBRACKET",
                label: "[",
                width_units: 1,
            },
            KeyboardCell {
                token: "RBRACKET",
                label: "]",
                width_units: 1,
            },
            KeyboardCell {
                token: "BACKSLASH",
                label: "\\",
                width_units: 2,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(2),
        &[
            KeyboardCell {
                token: "DELETE",
                label: "Del",
                width_units: 1,
            },
            KeyboardCell {
                token: "END",
                label: "End",
                width_units: 1,
            },
            KeyboardCell {
                token: "PAGEDOWN",
                label: "PgDn",
                width_units: 1,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(2),
        &[
            KeyboardCell {
                token: "NUM7",
                label: "7",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUM8",
                label: "8",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUM9",
                label: "9",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUMPLUS",
                label: "+",
                width_units: 1,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(3),
        &[
            KeyboardCell {
                token: "CAPSLOCK",
                label: "Caps",
                width_units: 2,
            },
            KeyboardCell {
                token: "A",
                label: "A",
                width_units: 1,
            },
            KeyboardCell {
                token: "S",
                label: "S",
                width_units: 1,
            },
            KeyboardCell {
                token: "D",
                label: "D",
                width_units: 1,
            },
            KeyboardCell {
                token: "F",
                label: "F",
                width_units: 1,
            },
            KeyboardCell {
                token: "G",
                label: "G",
                width_units: 1,
            },
            KeyboardCell {
                token: "H",
                label: "H",
                width_units: 1,
            },
            KeyboardCell {
                token: "J",
                label: "J",
                width_units: 1,
            },
            KeyboardCell {
                token: "K",
                label: "K",
                width_units: 1,
            },
            KeyboardCell {
                token: "L",
                label: "L",
                width_units: 1,
            },
            KeyboardCell {
                token: "SEMICOLON",
                label: ";",
                width_units: 1,
            },
            KeyboardCell {
                token: "APOSTROPHE",
                label: "'",
                width_units: 1,
            },
            KeyboardCell {
                token: "ENTER",
                label: "Enter",
                width_units: 3,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(3),
        &[
            KeyboardCell {
                token: "NUM4",
                label: "4",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUM5",
                label: "5",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUM6",
                label: "6",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUMENTER",
                label: "Ent",
                width_units: 1,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(4),
        &[
            KeyboardCell {
                token: "LSHIFT",
                label: "LShift",
                width_units: 3,
            },
            KeyboardCell {
                token: "Z",
                label: "Z",
                width_units: 1,
            },
            KeyboardCell {
                token: "X",
                label: "X",
                width_units: 1,
            },
            KeyboardCell {
                token: "C",
                label: "C",
                width_units: 1,
            },
            KeyboardCell {
                token: "V",
                label: "V",
                width_units: 1,
            },
            KeyboardCell {
                token: "B",
                label: "B",
                width_units: 1,
            },
            KeyboardCell {
                token: "N",
                label: "N",
                width_units: 1,
            },
            KeyboardCell {
                token: "M",
                label: "M",
                width_units: 1,
            },
            KeyboardCell {
                token: "COMMA",
                label: ",",
                width_units: 1,
            },
            KeyboardCell {
                token: "PERIOD",
                label: ".",
                width_units: 1,
            },
            KeyboardCell {
                token: "SLASH",
                label: "/",
                width_units: 1,
            },
            KeyboardCell {
                token: "RSHIFT",
                label: "RShift",
                width_units: 3,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x + keyboard_units_to_px(1),
        keyboard_row_y(4),
        &[KeyboardCell {
            token: "UP",
            label: "Up",
            width_units: 1,
        }],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(4),
        &[
            KeyboardCell {
                token: "NUM1",
                label: "1",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUM2",
                label: "2",
                width_units: 1,
            },
            KeyboardCell {
                token: "NUM3",
                label: "3",
                width_units: 1,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(5),
        &[
            KeyboardCell {
                token: "LCTRL",
                label: "LCtrl",
                width_units: 2,
            },
            KeyboardCell {
                token: "LWIN",
                label: "Win",
                width_units: 1,
            },
            KeyboardCell {
                token: "LALT",
                label: "LAlt",
                width_units: 1,
            },
            KeyboardCell {
                token: "SPACE",
                label: "Space",
                width_units: 6,
            },
            KeyboardCell {
                token: "RALT",
                label: "RAlt",
                width_units: 1,
            },
            KeyboardCell {
                token: "RWIN",
                label: "Win",
                width_units: 1,
            },
            KeyboardCell {
                token: "MENU",
                label: "Menu",
                width_units: 1,
            },
            KeyboardCell {
                token: "RCTRL",
                label: "RCtrl",
                width_units: 3,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(5),
        &[
            KeyboardCell {
                token: "LEFT",
                label: "Left",
                width_units: 1,
            },
            KeyboardCell {
                token: "DOWN",
                label: "Down",
                width_units: 1,
            },
            KeyboardCell {
                token: "RIGHT",
                label: "Right",
                width_units: 1,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(5),
        &[
            KeyboardCell {
                token: "NUM0",
                label: "0",
                width_units: 2,
            },
            KeyboardCell {
                token: "NUMDOT",
                label: ".",
                width_units: 1,
            },
        ],
    );

    keys
}

fn push_keyboard_row(keys: &mut Vec<KeyboardLayoutKey>, base_x: i32, y: i32, row: &[KeyboardCell]) {
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

fn keyboard_row_y(row: i32) -> i32 {
    KEYBOARD_MARGIN + row * (KEYBOARD_KEY_HEIGHT + KEYBOARD_KEY_GAP)
}

fn keyboard_units_to_px(units: i32) -> i32 {
    units * KEYBOARD_KEY_WIDTH + (units - 1) * KEYBOARD_KEY_GAP
}

fn supported_keys_vec() -> Vec<String> {
    supported_key_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

fn current_cursor_position() -> (i32, i32) {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    (point.x, point.y)
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
