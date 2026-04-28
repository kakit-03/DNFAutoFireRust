// Selects input backend implementations and exposes backend metadata.

use crate::input::{is_vk_down, send_key_once, synthetic_key_is_down};
use crate::keymap::KeySpec;
use crate::timing::HighPrecisionSleeper;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle, sleep};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_REMOVE,
    PeekMessageW, PostMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputBackendKind {
    SendInputPolling,
    HookSendInput,
    MessageBackend,
    SidecarMacro,
    HidSerial,
}

impl Default for InputBackendKind {
    fn default() -> Self {
        Self::SendInputPolling
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InputBackendCapabilities {
    pub reads_physical_state: bool,
    pub sends_keyboard_events: bool,
    pub supports_hold_repeat: bool,
    pub supports_combo_sequence: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct InputBackendDescriptor {
    pub kind: InputBackendKind,
    pub label: &'static str,
    pub description: &'static str,
    pub available: bool,
    pub unavailable_reason: Option<&'static str>,
}

pub trait InputBackend: Send {
    fn update_context(&mut self, _context: InputBackendContext) {}

    fn snapshot(&mut self, monitored_vks: &[u16]) -> InputSnapshot;
    fn send_key_once(
        &mut self,
        key: KeySpec,
        press_duration: Duration,
        sleeper: &HighPrecisionSleeper,
    );
    fn capabilities(&self) -> InputBackendCapabilities;
    fn health_check(&self) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputBackendContext {
    pub target_hwnd: Option<isize>,
}

#[derive(Debug, Clone, Default)]
pub struct InputSnapshot {
    down: HashMap<u16, bool>,
}

impl InputSnapshot {
    pub fn new(down: HashMap<u16, bool>) -> Self {
        Self { down }
    }

    pub fn is_down(&self, vk: u16) -> bool {
        self.down.get(&vk).copied().unwrap_or(false)
    }
}

#[derive(Default)]
struct SendInputPollingBackend {
    stable_down: HashMap<u16, bool>,
}

impl InputBackend for SendInputPollingBackend {
    fn snapshot(&mut self, monitored_vks: &[u16]) -> InputSnapshot {
        let mut down = HashMap::with_capacity(monitored_vks.len());
        for &vk in monitored_vks {
            let raw_down = is_vk_down(vk);
            let previous_down = self.stable_down.get(&vk).copied().unwrap_or(false);
            let effective_down =
                resolve_effective_key_down(previous_down, raw_down, synthetic_key_is_down(vk));
            self.stable_down.insert(vk, effective_down);
            down.insert(vk, effective_down);
        }

        InputSnapshot::new(down)
    }

    fn send_key_once(
        &mut self,
        key: KeySpec,
        press_duration: Duration,
        sleeper: &HighPrecisionSleeper,
    ) {
        send_key_once(key, press_duration, sleeper);
    }

    fn capabilities(&self) -> InputBackendCapabilities {
        InputBackendCapabilities {
            reads_physical_state: true,
            sends_keyboard_events: true,
            supports_hold_repeat: true,
            supports_combo_sequence: true,
        }
    }

    fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

struct HookSendInputBackend {
    shared: Arc<HookSharedState>,
    stop_flag: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    stable_down: HashMap<u16, bool>,
    config: HookSendInputConfig,
    mouse_suppressed_until: Option<Instant>,
}

struct HookSharedState {
    down: Mutex<HashMap<u16, bool>>,
}

impl HookSendInputBackend {
    fn new(settings: &BackendSettings) -> Result<Self> {
        let shared = Arc::new(HookSharedState {
            down: Mutex::new(HashMap::new()),
        });
        let stop_flag = Arc::new(AtomicBool::new(false));
        let join_shared = Arc::clone(&shared);
        let join_stop = Arc::clone(&stop_flag);

        let join = thread::Builder::new()
            .name("input-hook-backend".to_string())
            .spawn(move || run_keyboard_hook_thread(join_shared, join_stop))
            .context("failed to spawn keyboard hook thread")?;

        Ok(Self {
            shared,
            stop_flag,
            join: Some(join),
            stable_down: HashMap::new(),
            config: HookSendInputConfig::from_settings(settings),
            mouse_suppressed_until: None,
        })
    }

    fn mouse_click_pause_active(&mut self) -> bool {
        if !self.config.pause_on_mouse_button {
            return false;
        }

        let now = Instant::now();
        if mouse_button_is_down() {
            self.mouse_suppressed_until = Some(now + self.config.mouse_resume_delay);
            return true;
        }

        self.mouse_suppressed_until.is_some_and(|until| now < until)
    }
}

impl Drop for HookSendInputBackend {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl InputBackend for HookSendInputBackend {
    fn snapshot(&mut self, monitored_vks: &[u16]) -> InputSnapshot {
        if self.mouse_click_pause_active() {
            for &vk in monitored_vks {
                self.stable_down.insert(vk, false);
            }
            return all_keys_up_snapshot(monitored_vks);
        }

        let raw = self
            .shared
            .down
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let mut down = HashMap::with_capacity(monitored_vks.len());

        for &vk in monitored_vks {
            let raw_down = raw.get(&vk).copied().unwrap_or_else(|| is_vk_down(vk));
            let previous_down = self.stable_down.get(&vk).copied().unwrap_or(false);
            let effective_down =
                resolve_effective_key_down(previous_down, raw_down, synthetic_key_is_down(vk));
            self.stable_down.insert(vk, effective_down);
            down.insert(vk, effective_down);
        }

        InputSnapshot::new(down)
    }

    fn send_key_once(
        &mut self,
        key: KeySpec,
        press_duration: Duration,
        sleeper: &HighPrecisionSleeper,
    ) {
        send_key_once(key, press_duration, sleeper);
    }

    fn capabilities(&self) -> InputBackendCapabilities {
        standard_keyboard_capabilities()
    }

    fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct MessageBackend {
    target_hwnd: Option<isize>,
    input_state: SendInputPollingBackend,
}

impl InputBackend for MessageBackend {
    fn update_context(&mut self, context: InputBackendContext) {
        self.target_hwnd = context.target_hwnd;
    }

    fn snapshot(&mut self, monitored_vks: &[u16]) -> InputSnapshot {
        self.input_state.snapshot(monitored_vks)
    }

    fn send_key_once(
        &mut self,
        key: KeySpec,
        press_duration: Duration,
        sleeper: &HighPrecisionSleeper,
    ) {
        let Some(hwnd) = self.target_hwnd else {
            return;
        };
        post_key_message(hwnd, key, false);
        sleeper.sleep_for(press_duration);
        post_key_message(hwnd, key, true);
    }

    fn capabilities(&self) -> InputBackendCapabilities {
        standard_keyboard_capabilities()
    }

    fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

struct SidecarMacroBackend {
    config: SidecarConfig,
    fallback: SendInputPollingBackend,
}

impl SidecarMacroBackend {
    fn new(settings: &BackendSettings) -> Self {
        Self {
            config: SidecarConfig::from_settings(settings),
            fallback: SendInputPollingBackend::default(),
        }
    }
}

impl InputBackend for SidecarMacroBackend {
    fn snapshot(&mut self, monitored_vks: &[u16]) -> InputSnapshot {
        self.fallback.snapshot(monitored_vks)
    }

    fn send_key_once(
        &mut self,
        key: KeySpec,
        press_duration: Duration,
        sleeper: &HighPrecisionSleeper,
    ) {
        if self.config.executable.is_empty() {
            self.fallback.send_key_once(key, press_duration, sleeper);
            return;
        }

        if run_sidecar_command(&self.config, key, press_duration).is_err() {
            self.fallback.send_key_once(key, press_duration, sleeper);
        }
    }

    fn capabilities(&self) -> InputBackendCapabilities {
        standard_keyboard_capabilities()
    }

    fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

struct HidSerialBackend {
    port: Option<File>,
    fallback: SendInputPollingBackend,
}

impl HidSerialBackend {
    fn new(settings: &BackendSettings) -> Self {
        let port = string_setting(settings, "hid_serial", "port")
            .and_then(|port| open_hid_serial_port(&port).ok());
        Self {
            port,
            fallback: SendInputPollingBackend::default(),
        }
    }
}

impl InputBackend for HidSerialBackend {
    fn snapshot(&mut self, monitored_vks: &[u16]) -> InputSnapshot {
        self.fallback.snapshot(monitored_vks)
    }

    fn send_key_once(
        &mut self,
        key: KeySpec,
        press_duration: Duration,
        sleeper: &HighPrecisionSleeper,
    ) {
        let Some(port) = self.port.as_mut() else {
            self.fallback.send_key_once(key, press_duration, sleeper);
            return;
        };

        if write_hid_serial_event(port, key, true, press_duration).is_err() {
            self.fallback.send_key_once(key, press_duration, sleeper);
            return;
        }
        sleeper.sleep_for(press_duration);
        if write_hid_serial_event(port, key, false, press_duration).is_err() {
            self.fallback
                .send_key_once(key, Duration::from_millis(1), sleeper);
        }
    }

    fn capabilities(&self) -> InputBackendCapabilities {
        standard_keyboard_capabilities()
    }

    fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

pub fn create_input_backend(
    kind: InputBackendKind,
    settings: &BackendSettings,
) -> Result<Box<dyn InputBackend>> {
    match kind {
        InputBackendKind::SendInputPolling => Ok(Box::<SendInputPollingBackend>::default()),
        InputBackendKind::HookSendInput => Ok(Box::new(HookSendInputBackend::new(settings)?)),
        InputBackendKind::MessageBackend => Ok(Box::<MessageBackend>::default()),
        InputBackendKind::SidecarMacro => Ok(Box::new(SidecarMacroBackend::new(settings))),
        InputBackendKind::HidSerial => Ok(Box::new(HidSerialBackend::new(settings))),
    }
}

pub fn input_backend_descriptors() -> &'static [InputBackendDescriptor] {
    &[
        InputBackendDescriptor {
            kind: InputBackendKind::SendInputPolling,
            label: "SendInput 轮询",
            description: "当前默认实现：轮询物理按键并用 SendInput 扫描码发键。",
            available: true,
            unavailable_reason: None,
        },
        InputBackendDescriptor {
            kind: InputBackendKind::HookSendInput,
            label: "Hook + SendInput",
            description: "低级键盘钩子采集状态，SendInput 发键；鼠标点击时默认短暂停发，避免影响游戏内图标点击。",
            available: true,
            unavailable_reason: None,
        },
        InputBackendDescriptor {
            kind: InputBackendKind::MessageBackend,
            label: "窗口消息",
            description: "通过窗口消息发送键盘事件，仅适合作为兼容性实验。",
            available: true,
            unavailable_reason: None,
        },
        InputBackendDescriptor {
            kind: InputBackendKind::SidecarMacro,
            label: "外部脚本",
            description: "由 GUI 管配置，外部 AutoHotkey/Python 脚本执行输入；未配置脚本时使用 SendInput 兼容发送。",
            available: true,
            unavailable_reason: None,
        },
        InputBackendDescriptor {
            kind: InputBackendKind::HidSerial,
            label: "HID 串口",
            description: "通过串口控制用户自有 HID 设备或可编程键盘；未配置串口时使用 SendInput 兼容发送。",
            available: true,
            unavailable_reason: None,
        },
    ]
}

pub fn input_backend_descriptor(kind: InputBackendKind) -> InputBackendDescriptor {
    input_backend_descriptors()
        .iter()
        .copied()
        .find(|descriptor| descriptor.kind == kind)
        .unwrap_or(input_backend_descriptors()[0])
}

pub fn input_backend_label(kind: InputBackendKind) -> &'static str {
    input_backend_descriptor(kind).label
}

pub fn resolve_effective_key_down(
    previous_down: bool,
    raw_down: bool,
    synthetic_down: bool,
) -> bool {
    if synthetic_down {
        previous_down
    } else {
        raw_down
    }
}

pub type BackendSettings = BTreeMap<String, Value>;

const MOUSE_BUTTON_VKS: &[u16] = &[0x01, 0x02, 0x04, 0x05, 0x06];
const DEFAULT_HOOK_MOUSE_RESUME_DELAY: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy)]
struct HookSendInputConfig {
    pause_on_mouse_button: bool,
    mouse_resume_delay: Duration,
}

impl Default for HookSendInputConfig {
    fn default() -> Self {
        Self {
            pause_on_mouse_button: true,
            mouse_resume_delay: DEFAULT_HOOK_MOUSE_RESUME_DELAY,
        }
    }
}

impl HookSendInputConfig {
    fn from_settings(settings: &BackendSettings) -> Self {
        let mut config = Self::default();
        if let Some(value) = bool_setting(settings, "hook_send_input", "pause_on_mouse_button") {
            config.pause_on_mouse_button = value;
        }
        if let Some(ms) = u64_setting(settings, "hook_send_input", "mouse_resume_delay_ms") {
            config.mouse_resume_delay = Duration::from_millis(ms);
        }
        config
    }
}

fn standard_keyboard_capabilities() -> InputBackendCapabilities {
    InputBackendCapabilities {
        reads_physical_state: true,
        sends_keyboard_events: true,
        supports_hold_repeat: true,
        supports_combo_sequence: true,
    }
}

fn run_keyboard_hook_thread(shared: Arc<HookSharedState>, stop_flag: Arc<AtomicBool>) {
    set_hook_shared_state(Some(shared));
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0) };
    let Ok(hook) = hook else {
        set_hook_shared_state(None);
        return;
    };

    while !stop_flag.load(Ordering::SeqCst) {
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        sleep(Duration::from_millis(2));
    }

    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }
    set_hook_shared_state(None);
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let message = wparam.0 as u32;
        let is_down = match message {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };

        if let Some(is_down) = is_down
            && !keyboard_event_is_injected(event)
            && let Some(shared) = hook_shared_state()
            && let Ok(mut down) = shared.down.lock()
        {
            down.insert(event.vkCode as u16, is_down);
        }
    }

    unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) }
}

fn keyboard_event_is_injected(event: &KBDLLHOOKSTRUCT) -> bool {
    event.flags.0 & 0x10 != 0
}

fn hook_state_slot() -> &'static Mutex<Option<Arc<HookSharedState>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<HookSharedState>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn set_hook_shared_state(shared: Option<Arc<HookSharedState>>) {
    if let Ok(mut slot) = hook_state_slot().lock() {
        *slot = shared;
    }
}

fn hook_shared_state() -> Option<Arc<HookSharedState>> {
    hook_state_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(Arc::clone))
}

fn mouse_button_is_down() -> bool {
    MOUSE_BUTTON_VKS.iter().any(|&vk| is_vk_down(vk))
}

fn all_keys_up_snapshot(monitored_vks: &[u16]) -> InputSnapshot {
    InputSnapshot::new(monitored_vks.iter().map(|&vk| (vk, false)).collect())
}

fn post_key_message(hwnd: isize, key: KeySpec, key_up: bool) {
    let message = if key_up { WM_KEYUP } else { WM_KEYDOWN };
    let lparam = key_message_lparam(key, key_up);
    unsafe {
        let _ = PostMessageW(
            HWND(hwnd as _),
            message,
            WPARAM(key.vk as usize),
            LPARAM(lparam as isize),
        );
    }
}

fn key_message_lparam(key: KeySpec, key_up: bool) -> u32 {
    let mut value = 1u32 | ((key.scan as u32) << 16);
    if key.extended {
        value |= 1 << 24;
    }
    if key_up {
        value |= 1 << 30;
        value |= 1 << 31;
    }
    value
}

#[derive(Default)]
struct SidecarConfig {
    executable: String,
    args: Vec<String>,
}

impl SidecarConfig {
    fn from_settings(settings: &BackendSettings) -> Self {
        Self {
            executable: string_setting(settings, "sidecar_macro", "executable").unwrap_or_default(),
            args: string_list_setting(settings, "sidecar_macro", "args")
                .unwrap_or_else(|| vec!["{key}".to_string(), "{duration_ms}".to_string()]),
        }
    }
}

fn run_sidecar_command(
    config: &SidecarConfig,
    key: KeySpec,
    press_duration: Duration,
) -> Result<()> {
    let mut command = Command::new(&config.executable);
    command.args(
        config
            .args
            .iter()
            .map(|arg| render_backend_template(arg, key, press_duration)),
    );
    let status = command
        .status()
        .with_context(|| format!("failed to run sidecar '{}'", config.executable))?;
    if !status.success() {
        anyhow::bail!("sidecar '{}' exited with {status}", config.executable);
    }
    Ok(())
}

fn open_hid_serial_port(port: &str) -> Result<File> {
    let path = if port.starts_with(r"\\.\") {
        port.to_string()
    } else {
        format!(r"\\.\{port}")
    };
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open HID serial port '{path}'"))
}

fn write_hid_serial_event(
    port: &mut File,
    key: KeySpec,
    is_down: bool,
    press_duration: Duration,
) -> Result<()> {
    let action = if is_down { "down" } else { "up" };
    writeln!(
        port,
        r#"{{"action":"{action}","key":"{}","vk":{},"scan":{},"extended":{},"duration_ms":{}}}"#,
        key.name,
        key.vk,
        key.scan,
        key.extended,
        press_duration.as_millis()
    )?;
    port.flush()?;
    Ok(())
}

fn render_backend_template(template: &str, key: KeySpec, press_duration: Duration) -> String {
    template
        .replace("{key}", key.name)
        .replace("{vk}", &key.vk.to_string())
        .replace("{scan}", &key.scan.to_string())
        .replace("{extended}", if key.extended { "true" } else { "false" })
        .replace("{duration_ms}", &press_duration.as_millis().to_string())
}

fn string_setting(settings: &BackendSettings, backend_key: &str, name: &str) -> Option<String> {
    settings
        .get(backend_key)
        .and_then(Value::as_object)
        .and_then(|backend| backend.get(name))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn string_list_setting(
    settings: &BackendSettings,
    backend_key: &str,
    name: &str,
) -> Option<Vec<String>> {
    settings
        .get(backend_key)
        .and_then(Value::as_object)
        .and_then(|backend| backend.get(name))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
}

fn bool_setting(settings: &BackendSettings, backend_key: &str, name: &str) -> Option<bool> {
    settings
        .get(backend_key)
        .and_then(Value::as_object)
        .and_then(|backend| backend.get(name))
        .and_then(Value::as_bool)
}

fn u64_setting(settings: &BackendSettings, backend_key: &str, name: &str) -> Option<u64> {
    settings
        .get(backend_key)
        .and_then(Value::as_object)
        .and_then(|backend| backend.get(name))
        .and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::{
        HookSendInputConfig, InputBackendContext, InputBackendKind, all_keys_up_snapshot,
        create_input_backend, input_backend_descriptor, input_backend_descriptors,
        key_message_lparam, render_backend_template, resolve_effective_key_down,
    };
    use crate::keymap::parse_single_key;
    use std::collections::BTreeMap;
    use std::time::Duration;

    #[test]
    fn send_input_polling_backend_is_available() {
        let settings = BTreeMap::new();
        let backend = create_input_backend(InputBackendKind::SendInputPolling, &settings)
            .expect("default backend should be available");
        backend
            .health_check()
            .expect("default backend health check");
    }

    #[test]
    fn all_declared_backends_are_constructible() {
        let settings = BTreeMap::new();
        for descriptor in input_backend_descriptors() {
            let mut backend =
                create_input_backend(descriptor.kind, &settings).unwrap_or_else(|err| {
                    panic!("{} should be constructible: {err}", descriptor.label)
                });
            backend.update_context(InputBackendContext::default());
            backend
                .health_check()
                .unwrap_or_else(|err| panic!("{} health check failed: {err}", descriptor.label));
        }
    }

    #[test]
    fn backend_descriptors_mark_default_available() {
        let descriptor = input_backend_descriptor(InputBackendKind::SendInputPolling);
        assert!(descriptor.available);
        assert_eq!(descriptor.unavailable_reason, None);
        assert!(
            input_backend_descriptors()
                .iter()
                .any(|item| item.kind == InputBackendKind::SendInputPolling && item.available)
        );
    }

    #[test]
    fn synthetic_hold_keeps_previous_physical_state() {
        assert!(resolve_effective_key_down(true, false, true));
        assert!(!resolve_effective_key_down(false, true, true));
    }

    #[test]
    fn key_message_lparam_sets_scan_extended_and_keyup_bits() {
        let key = parse_single_key("RIGHT").expect("RIGHT");
        let down = key_message_lparam(key, false);
        let up = key_message_lparam(key, true);

        assert_eq!((down >> 16) & 0xff, key.scan as u32);
        assert_ne!(down & (1 << 24), 0);
        assert_eq!(down & (1 << 31), 0);
        assert_ne!(up & (1 << 30), 0);
        assert_ne!(up & (1 << 31), 0);
    }

    #[test]
    fn sidecar_templates_render_key_fields() {
        let key = parse_single_key("A").expect("A");
        let rendered = render_backend_template(
            "{key}:{vk}:{scan}:{extended}:{duration_ms}",
            key,
            Duration::from_millis(25),
        );

        assert_eq!(rendered, "A:65:30:false:25");
    }

    #[test]
    fn hook_mouse_pause_settings_are_configurable() {
        let settings = serde_json::json!({
            "hook_send_input": {
                "pause_on_mouse_button": false,
                "mouse_resume_delay_ms": 45
            }
        });
        let settings = serde_json::from_value(settings).expect("settings map");
        let config = HookSendInputConfig::from_settings(&settings);

        assert!(!config.pause_on_mouse_button);
        assert_eq!(config.mouse_resume_delay, Duration::from_millis(45));
    }

    #[test]
    fn all_keys_up_snapshot_marks_monitored_keys_as_released() {
        let snapshot = all_keys_up_snapshot(&[0x41, 0x42]);

        assert!(!snapshot.is_down(0x41));
        assert!(!snapshot.is_down(0x42));
        assert!(!snapshot.is_down(0x43));
    }
}
