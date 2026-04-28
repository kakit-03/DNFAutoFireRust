// Holds shared GUI formatting, capture, and parsing helpers.

use super::*;

pub(super) fn hotkey_modifiers(bits: u32) -> u32 {
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

pub(super) fn special_key_dialog_from_config(
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

pub(super) fn start_combo_capture(dialog: &mut ComboDialogState, target: ComboCaptureTarget) {
    dialog.capture_target = Some(target);
    dialog.capture_down_keys = currently_pressed_supported_keys();
}

pub(super) fn start_special_key_capture(
    dialog: &mut SpecialKeyDialogState,
    target: SpecialKeyCaptureTarget,
) {
    dialog.capture_target = Some(target);
    dialog.capture_down_keys = currently_pressed_supported_keys();
}

pub(super) fn combo_capture_message(target: Option<ComboCaptureTarget>) -> Option<&'static str> {
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

pub(super) fn special_key_capture_message(
    target: Option<SpecialKeyCaptureTarget>,
) -> Option<&'static str> {
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

pub(super) fn special_key_kind_label(config: &SpecialKeyConfig) -> &'static str {
    match config {
        SpecialKeyConfig::CustomAutofire { .. } => "独立连发",
        SpecialKeyConfig::AutoTrigger { .. } => "自动触发",
        SpecialKeyConfig::LinkedKey { .. } => "连携键位",
    }
}

pub(super) fn capture_next_supported_key(
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

pub(super) fn currently_pressed_supported_keys() -> HashSet<String> {
    capture_next_supported_key(&HashSet::new()).0
}

pub(super) fn parse_dialog_ms(raw: &str, label: &str) -> Result<u64> {
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

pub(super) fn strip_clone_suffix(name: &str) -> Option<&str> {
    let (prefix, suffix) = name.rsplit_once("-cloned-")?;
    if prefix.is_empty() || suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(prefix)
}
