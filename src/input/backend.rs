use crate::input::{is_vk_down, send_key_once, synthetic_key_is_down};
use crate::keymap::KeySpec;
use crate::timing::HighPrecisionSleeper;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

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

pub fn create_input_backend(
    kind: InputBackendKind,
    _settings: &BackendSettings,
) -> Result<Box<dyn InputBackend>> {
    match kind {
        InputBackendKind::SendInputPolling => Ok(Box::<SendInputPollingBackend>::default()),
        InputBackendKind::HookSendInput
        | InputBackendKind::MessageBackend
        | InputBackendKind::SidecarMacro
        | InputBackendKind::HidSerial => {
            let descriptor = input_backend_descriptor(kind);
            bail!(
                "输入后端 '{}' 暂不可用: {}",
                descriptor.label,
                descriptor
                    .unavailable_reason
                    .unwrap_or("当前版本尚未实现该后端")
            )
        }
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
            description: "低级键盘钩子或 Raw Input 采集状态，SendInput 发键。",
            available: false,
            unavailable_reason: Some("实验后端占位，尚未接入钩子事件循环"),
        },
        InputBackendDescriptor {
            kind: InputBackendKind::MessageBackend,
            label: "窗口消息",
            description: "通过窗口消息发送键盘事件，仅适合作为兼容性实验。",
            available: false,
            unavailable_reason: Some("实验后端占位，尚未实现目标窗口消息派发"),
        },
        InputBackendDescriptor {
            kind: InputBackendKind::SidecarMacro,
            label: "外部脚本",
            description: "由 GUI 管配置，外部 AutoHotkey/Python 脚本执行输入。",
            available: false,
            unavailable_reason: Some("实验后端占位，尚未实现脚本进程管理"),
        },
        InputBackendDescriptor {
            kind: InputBackendKind::HidSerial,
            label: "HID 串口",
            description: "通过串口控制用户自有 HID 设备或可编程键盘。",
            available: false,
            unavailable_reason: Some("实验后端占位，尚未实现串口协议"),
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

#[cfg(test)]
mod tests {
    use super::{InputBackendKind, create_input_backend, resolve_effective_key_down};
    use std::collections::BTreeMap;

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
    fn unavailable_backend_reports_clear_error() {
        let settings = BTreeMap::new();
        let err = match create_input_backend(InputBackendKind::HookSendInput, &settings) {
            Ok(_) => panic!("hook backend is only an unavailable placeholder"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("暂不可用"));
    }

    #[test]
    fn synthetic_hold_keeps_previous_physical_state() {
        assert!(resolve_effective_key_down(true, false, true));
        assert!(!resolve_effective_key_down(false, true, true));
    }
}
