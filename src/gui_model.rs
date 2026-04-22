use crate::config::{ComboConfig, Profile};
use anyhow::{Result, bail};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ProfileDraft {
    pub name: String,
    pub enabled_keys: Vec<String>,
    pub repeat_interval_ms: String,
    pub press_duration_ms: String,
    pub poll_interval_ms: String,
    pub target_windows_text: String,
    pub quick_switch_hotkey: String,
    pub combos: Vec<ComboConfig>,
}

impl ProfileDraft {
    pub fn from_named_profile(name: &str, profile: &Profile) -> Self {
        Self {
            name: name.to_string(),
            enabled_keys: profile.enabled_keys.clone(),
            repeat_interval_ms: profile.repeat_interval_ms.to_string(),
            press_duration_ms: profile.press_duration_ms.to_string(),
            poll_interval_ms: profile.poll_interval_ms.to_string(),
            target_windows_text: target_windows_to_text(&profile.target_windows),
            quick_switch_hotkey: profile.quick_switch_hotkey.clone(),
            combos: profile.combos.clone(),
        }
    }

    pub fn to_named_profile(&self) -> Result<(String, Profile)> {
        let name = self.name.trim();
        if name.is_empty() {
            bail!("配置名不能为空");
        }

        let repeat_interval_ms = parse_ms(&self.repeat_interval_ms, "连发间隔")?;
        let press_duration_ms = parse_ms(&self.press_duration_ms, "按下时长")?;
        let poll_interval_ms = parse_ms(&self.poll_interval_ms, "轮询间隔")?;
        let target_windows = target_windows_from_text(&self.target_windows_text);

        let profile = Profile {
            enabled_keys: self.enabled_keys.clone(),
            repeat_interval_ms,
            press_duration_ms,
            poll_interval_ms,
            target_windows,
            quick_switch_hotkey: self.quick_switch_hotkey.trim().to_string(),
            combos: self.combos.clone(),
        }
        .normalized();

        profile.validate()?;
        Ok((name.to_string(), profile))
    }
}

pub fn target_windows_to_text(tokens: &[String]) -> String {
    tokens.join("\r\n")
}

pub fn target_windows_from_text(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }

    out
}

fn parse_ms(raw: &str, label: &str) -> Result<u64> {
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

#[cfg(test)]
mod tests {
    use super::{ProfileDraft, target_windows_from_text, target_windows_to_text};
    use crate::config::{ComboConfig, ComboStepConfig, Profile};

    #[test]
    fn draft_roundtrip_keeps_all_profile_fields() {
        let profile = Profile {
            enabled_keys: vec!["J".to_string(), "P".to_string()],
            repeat_interval_ms: 5,
            press_duration_ms: 3,
            poll_interval_ms: 2,
            target_windows: vec!["地下城与勇士".to_string(), "DNF".to_string()],
            quick_switch_hotkey: "LCTRL+LSHIFT+Q".to_string(),
            combos: vec![ComboConfig {
                name: "combo1".to_string(),
                trigger_key: "U".to_string(),
                steps: vec![
                    ComboStepConfig {
                        key: "A".to_string(),
                        interval_ms: 80,
                        press_duration_ms: 1,
                    },
                    ComboStepConfig {
                        key: "A".to_string(),
                        interval_ms: 90,
                        press_duration_ms: 2,
                    },
                    ComboStepConfig {
                        key: "D".to_string(),
                        interval_ms: 100,
                        press_duration_ms: 3,
                    },
                ],
                sequence_keys: Vec::new(),
                step_interval_ms: 0,
                press_duration_ms: 1,
            }],
        };

        let draft = ProfileDraft::from_named_profile("raid", &profile);
        let (name, restored) = draft.to_named_profile().expect("draft -> profile");

        assert_eq!(name, "raid");
        assert_eq!(restored.enabled_keys, profile.enabled_keys);
        assert_eq!(restored.repeat_interval_ms, profile.repeat_interval_ms);
        assert_eq!(restored.press_duration_ms, profile.press_duration_ms);
        assert_eq!(restored.poll_interval_ms, profile.poll_interval_ms);
        assert_eq!(restored.target_windows, profile.target_windows);
        assert_eq!(restored.quick_switch_hotkey, profile.quick_switch_hotkey);
        assert_eq!(restored.combos.len(), 1);
        assert_eq!(restored.combos[0].steps.len(), 3);
        assert_eq!(restored.combos[0].steps[0].key, "A");
        assert_eq!(restored.combos[0].steps[1].interval_ms, 90);
        assert_eq!(restored.combos[0].steps[2].press_duration_ms, 3);
    }

    #[test]
    fn target_window_text_conversion_deduplicates_and_trims() {
        let text = " DNF \r\n\r\n地下城与勇士\r\nDNF\r\n";
        let parsed = target_windows_from_text(text);
        assert_eq!(parsed, vec!["DNF", "地下城与勇士"]);
        assert_eq!(target_windows_to_text(&parsed), "DNF\r\n地下城与勇士");
    }

    #[test]
    fn combo_sequence_order_and_duplicates_survive_draft_save() {
        let draft = ProfileDraft {
            name: "combo-only".to_string(),
            enabled_keys: Vec::new(),
            repeat_interval_ms: "1".to_string(),
            press_duration_ms: "1".to_string(),
            poll_interval_ms: "1".to_string(),
            target_windows_text: "DNF".to_string(),
            quick_switch_hotkey: "LCTRL+Q".to_string(),
            combos: vec![ComboConfig {
                name: "burst".to_string(),
                trigger_key: "U".to_string(),
                steps: vec![
                    ComboStepConfig {
                        key: "A".to_string(),
                        interval_ms: 10,
                        press_duration_ms: 1,
                    },
                    ComboStepConfig {
                        key: "A".to_string(),
                        interval_ms: 20,
                        press_duration_ms: 2,
                    },
                    ComboStepConfig {
                        key: "S".to_string(),
                        interval_ms: 30,
                        press_duration_ms: 3,
                    },
                ],
                sequence_keys: Vec::new(),
                step_interval_ms: 0,
                press_duration_ms: 1,
            }],
        };

        let (_, profile) = draft.to_named_profile().expect("draft -> profile");
        assert_eq!(profile.combos[0].steps.len(), 3);
        assert_eq!(profile.combos[0].steps[0].key, "A");
        assert_eq!(profile.combos[0].steps[1].key, "A");
        assert_eq!(profile.combos[0].steps[2].key, "S");
        assert_eq!(profile.combos[0].steps[2].interval_ms, 30);
        assert_eq!(profile.combos[0].steps[2].press_duration_ms, 3);
    }
}
