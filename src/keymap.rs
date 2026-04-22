use anyhow::{Result, bail};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec {
    pub name: &'static str,
    pub vk: u16,
    pub scan: u8,
    pub extended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyRegistration {
    pub modifiers: u32,
    pub vk: u32,
}

macro_rules! define_supported_keys {
    ($( $name:literal => ($vk:expr, $scan:expr, $extended:expr) ),+ $(,)?) => {
        const SUPPORTED_KEY_NAMES: &[&str] = &[
            $($name,)+
        ];

        fn key_spec(name: &str) -> Option<KeySpec> {
            match name {
                $(
                    $name => Some(KeySpec {
                        name: $name,
                        vk: $vk,
                        scan: $scan,
                        extended: $extended,
                    }),
                )+
                _ => None,
            }
        }
    };
}

define_supported_keys! {
    "ESC" => (0x1B, 0x01, false),
    "F1" => (0x70, 0x3B, false),
    "F2" => (0x71, 0x3C, false),
    "F3" => (0x72, 0x3D, false),
    "F4" => (0x73, 0x3E, false),
    "F5" => (0x74, 0x3F, false),
    "F6" => (0x75, 0x40, false),
    "F7" => (0x76, 0x41, false),
    "F8" => (0x77, 0x42, false),
    "F9" => (0x78, 0x43, false),
    "F10" => (0x79, 0x44, false),
    "F11" => (0x7A, 0x57, false),
    "F12" => (0x7B, 0x58, false),
    "BACKQUOTE" => (0xC0, 0x29, false),
    "1" => (0x31, 0x02, false),
    "2" => (0x32, 0x03, false),
    "3" => (0x33, 0x04, false),
    "4" => (0x34, 0x05, false),
    "5" => (0x35, 0x06, false),
    "6" => (0x36, 0x07, false),
    "7" => (0x37, 0x08, false),
    "8" => (0x38, 0x09, false),
    "9" => (0x39, 0x0A, false),
    "0" => (0x30, 0x0B, false),
    "MINUS" => (0xBD, 0x0C, false),
    "EQUAL" => (0xBB, 0x0D, false),
    "BACKSPACE" => (0x08, 0x0E, false),
    "TAB" => (0x09, 0x0F, false),
    "Q" => (0x51, 0x10, false),
    "W" => (0x57, 0x11, false),
    "E" => (0x45, 0x12, false),
    "R" => (0x52, 0x13, false),
    "T" => (0x54, 0x14, false),
    "Y" => (0x59, 0x15, false),
    "U" => (0x55, 0x16, false),
    "I" => (0x49, 0x17, false),
    "O" => (0x4F, 0x18, false),
    "P" => (0x50, 0x19, false),
    "LBRACKET" => (0xDB, 0x1A, false),
    "RBRACKET" => (0xDD, 0x1B, false),
    "BACKSLASH" => (0xDC, 0x2B, false),
    "CAPSLOCK" => (0x14, 0x3A, false),
    "A" => (0x41, 0x1E, false),
    "S" => (0x53, 0x1F, false),
    "D" => (0x44, 0x20, false),
    "F" => (0x46, 0x21, false),
    "G" => (0x47, 0x22, false),
    "H" => (0x48, 0x23, false),
    "J" => (0x4A, 0x24, false),
    "K" => (0x4B, 0x25, false),
    "L" => (0x4C, 0x26, false),
    "SEMICOLON" => (0xBA, 0x27, false),
    "APOSTROPHE" => (0xDE, 0x28, false),
    "ENTER" => (0x0D, 0x1C, false),
    "LSHIFT" => (0xA0, 0x2A, false),
    "Z" => (0x5A, 0x2C, false),
    "X" => (0x58, 0x2D, false),
    "C" => (0x43, 0x2E, false),
    "V" => (0x56, 0x2F, false),
    "B" => (0x42, 0x30, false),
    "N" => (0x4E, 0x31, false),
    "M" => (0x4D, 0x32, false),
    "COMMA" => (0xBC, 0x33, false),
    "PERIOD" => (0xBE, 0x34, false),
    "SLASH" => (0xBF, 0x35, false),
    "RSHIFT" => (0xA1, 0x36, false),
    "LCTRL" => (0xA2, 0x1D, false),
    "LWIN" => (0x5B, 0x5B, true),
    "LALT" => (0xA4, 0x38, false),
    "SPACE" => (0x20, 0x39, false),
    "RALT" => (0xA5, 0x38, true),
    "RWIN" => (0x5C, 0x5C, true),
    "MENU" => (0x5D, 0x5D, true),
    "RCTRL" => (0xA3, 0x1D, true),
    "INSERT" => (0x2D, 0x52, true),
    "HOME" => (0x24, 0x47, true),
    "PAGEUP" => (0x21, 0x49, true),
    "DELETE" => (0x2E, 0x53, true),
    "END" => (0x23, 0x4F, true),
    "PAGEDOWN" => (0x22, 0x51, true),
    "UP" => (0x26, 0x48, true),
    "LEFT" => (0x25, 0x4B, true),
    "DOWN" => (0x28, 0x50, true),
    "RIGHT" => (0x27, 0x4D, true),
    "NUMLOCK" => (0x90, 0x45, true),
    "NUMDIV" => (0x6F, 0x35, true),
    "NUMMUL" => (0x6A, 0x37, false),
    "NUMMINUS" => (0x6D, 0x4A, false),
    "NUM7" => (0x67, 0x47, false),
    "NUM8" => (0x68, 0x48, false),
    "NUM9" => (0x69, 0x49, false),
    "NUMPLUS" => (0x6B, 0x4E, false),
    "NUM4" => (0x64, 0x4B, false),
    "NUM5" => (0x65, 0x4C, false),
    "NUM6" => (0x66, 0x4D, false),
    "NUM1" => (0x61, 0x4F, false),
    "NUM2" => (0x62, 0x50, false),
    "NUM3" => (0x63, 0x51, false),
    "NUMENTER" => (0x0D, 0x1C, true),
    "NUM0" => (0x60, 0x52, false),
    "NUMDOT" => (0x6E, 0x53, false)
}

pub fn supported_key_names() -> &'static [&'static str] {
    SUPPORTED_KEY_NAMES
}

pub fn parse_key_specs(keys: &[String]) -> Result<Vec<KeySpec>> {
    if keys.is_empty() {
        bail!("enabled_keys cannot be empty");
    }

    let mut parsed = Vec::new();
    let mut seen = HashSet::new();

    for input in keys {
        let normalized = normalize_key_name(input);
        let spec = key_spec(&normalized).ok_or_else(|| {
            anyhow::anyhow!(
                "unsupported key '{}'; supported keys: {}",
                input,
                SUPPORTED_KEY_NAMES.join(", ")
            )
        })?;

        if seen.insert(spec.name) {
            parsed.push(spec);
        }
    }

    if parsed.is_empty() {
        bail!("enabled_keys cannot be empty");
    }

    Ok(parsed)
}

#[allow(dead_code)]
pub fn parse_key_sequence(keys: &[String]) -> Result<Vec<KeySpec>> {
    if keys.is_empty() {
        bail!("sequence_keys cannot be empty");
    }

    keys.iter().map(|input| parse_single_key(input)).collect()
}

pub fn parse_single_key(input: &str) -> Result<KeySpec> {
    let normalized = normalize_key_name(input);
    key_spec(&normalized).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported key '{}'; supported keys: {}",
            input,
            SUPPORTED_KEY_NAMES.join(", ")
        )
    })
}

pub fn parse_hotkey(input: &str) -> Result<Vec<KeySpec>> {
    let parts = input
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(parse_single_key)
        .collect::<Result<Vec<_>>>()?;

    if parts.is_empty() {
        bail!("quick_switch_hotkey cannot be empty");
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for spec in parts {
        if seen.insert(spec.name) {
            out.push(spec);
        }
    }

    if !out.iter().any(|spec| !is_modifier_key(spec.name)) {
        bail!("quick_switch_hotkey must include at least one non-modifier key");
    }

    Ok(out)
}

pub fn normalize_hotkey_text(input: &str) -> Result<String> {
    let specs = parse_hotkey(input)?;
    Ok(sort_hotkey_names(
        specs
            .iter()
            .map(|spec| spec.name.to_string())
            .collect::<Vec<_>>(),
    )
    .join("+"))
}

pub fn display_key_name(name: &str) -> String {
    let normalized = normalize_key_name(name);
    match normalized.as_str() {
        "ESC" => "Esc".to_string(),
        "TAB" => "Tab".to_string(),
        "BACKSPACE" => "Bksp".to_string(),
        "CAPSLOCK" => "Caps".to_string(),
        "ENTER" => "Enter".to_string(),
        "SPACE" => "Space".to_string(),
        "LCTRL" => "LCtrl".to_string(),
        "RCTRL" => "RCtrl".to_string(),
        "LSHIFT" => "LShift".to_string(),
        "RSHIFT" => "RShift".to_string(),
        "LALT" => "LAlt".to_string(),
        "RALT" => "RAlt".to_string(),
        "LWIN" => "LWin".to_string(),
        "RWIN" => "RWin".to_string(),
        "INSERT" => "Ins".to_string(),
        "DELETE" => "Del".to_string(),
        "PAGEUP" => "PgUp".to_string(),
        "PAGEDOWN" => "PgDn".to_string(),
        "BACKQUOTE" => "~".to_string(),
        "MINUS" => "-".to_string(),
        "EQUAL" => "=".to_string(),
        "LBRACKET" => "[".to_string(),
        "RBRACKET" => "]".to_string(),
        "BACKSLASH" => "\\".to_string(),
        "SEMICOLON" => ";".to_string(),
        "APOSTROPHE" => "'".to_string(),
        "COMMA" => ",".to_string(),
        "PERIOD" => ".".to_string(),
        "SLASH" => "/".to_string(),
        "NUMLOCK" => "Num".to_string(),
        "NUMDIV" => "Num/".to_string(),
        "NUMMUL" => "Num*".to_string(),
        "NUMMINUS" => "Num-".to_string(),
        "NUMPLUS" => "Num+".to_string(),
        "NUMENTER" => "NumEnter".to_string(),
        "NUMDOT" => "Num.".to_string(),
        _ => normalized,
    }
}

pub fn display_hotkey_text(input: &str) -> String {
    input
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(display_key_name)
        .collect::<Vec<_>>()
        .join("+")
}

pub fn display_hotkey_names<I>(names: I) -> String
where
    I: IntoIterator<Item = String>,
{
    names
        .into_iter()
        .map(|name| display_key_name(&name))
        .collect::<Vec<_>>()
        .join("+")
}

pub fn hotkey_registration(input: &str) -> Result<HotkeyRegistration> {
    let specs = parse_hotkey(input)?;
    hotkey_registration_from_specs(&specs)
}

pub fn hotkey_registration_from_specs(specs: &[KeySpec]) -> Result<HotkeyRegistration> {
    let mut modifiers = 0u32;
    let mut main_key = None;

    for spec in specs {
        match spec.name {
            "LALT" | "RALT" => modifiers |= 0x0001,
            "LCTRL" | "RCTRL" => modifiers |= 0x0002,
            "LSHIFT" | "RSHIFT" => modifiers |= 0x0004,
            "LWIN" | "RWIN" => modifiers |= 0x0008,
            "MENU" => bail!("quick_switch_hotkey does not support MENU as a modifier"),
            _ => {
                if main_key.replace(spec.vk as u32).is_some() {
                    bail!("quick_switch_hotkey must contain exactly one non-modifier key");
                }
            }
        }
    }

    let Some(vk) = main_key else {
        bail!("quick_switch_hotkey must include one non-modifier key");
    };

    Ok(HotkeyRegistration { modifiers, vk })
}

pub fn sort_hotkey_names<I>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for name in names {
        if seen.insert(name.clone()) {
            unique.push(name);
        }
    }

    unique.sort_by_key(|name| hotkey_order(name));
    unique
}

pub fn is_modifier_key(name: &str) -> bool {
    matches!(
        name,
        "LCTRL" | "RCTRL" | "LSHIFT" | "RSHIFT" | "LALT" | "RALT" | "LWIN" | "RWIN" | "MENU"
    )
}

fn normalize_key_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed == " " {
        return "SPACE".to_string();
    }
    if trimmed.is_empty() {
        return String::new();
    }

    let upper = trimmed.to_ascii_uppercase();
    match upper.as_str() {
        "RETURN" => "ENTER".to_string(),
        "ESCAPE" => "ESC".to_string(),
        "CTRL" | "CONTROL" => "LCTRL".to_string(),
        "ALT" => "LALT".to_string(),
        "SHIFT" => "LSHIFT".to_string(),
        "WIN" | "WINDOWS" => "LWIN".to_string(),
        "APP" | "APPS" | "CONTEXT" => "MENU".to_string(),
        "CAPS" => "CAPSLOCK".to_string(),
        "INS" => "INSERT".to_string(),
        "DEL" => "DELETE".to_string(),
        "PGUP" => "PAGEUP".to_string(),
        "PGDN" => "PAGEDOWN".to_string(),
        "BKSP" | "BS" | "BACK" => "BACKSPACE".to_string(),
        "BACKTICK" | "GRAVE" | "GRAVEACCENT" | "OEM3" | "TILDE" => "BACKQUOTE".to_string(),
        "OEM_MINUS" => "MINUS".to_string(),
        "OEM_PLUS" => "EQUAL".to_string(),
        "OEM_4" => "LBRACKET".to_string(),
        "OEM_6" => "RBRACKET".to_string(),
        "OEM_5" => "BACKSLASH".to_string(),
        "OEM_1" => "SEMICOLON".to_string(),
        "OEM_7" => "APOSTROPHE".to_string(),
        "OEM_COMMA" => "COMMA".to_string(),
        "OEM_PERIOD" => "PERIOD".to_string(),
        "OEM_2" => "SLASH".to_string(),
        "NUM/" => "NUMDIV".to_string(),
        "NUM*" => "NUMMUL".to_string(),
        "NUM-" => "NUMMINUS".to_string(),
        "NUM+" => "NUMPLUS".to_string(),
        "NUM." => "NUMDOT".to_string(),
        "`" | "~" => "BACKQUOTE".to_string(),
        "-" => "MINUS".to_string(),
        "=" => "EQUAL".to_string(),
        "[" => "LBRACKET".to_string(),
        "]" => "RBRACKET".to_string(),
        "\\" => "BACKSLASH".to_string(),
        ";" => "SEMICOLON".to_string(),
        "'" => "APOSTROPHE".to_string(),
        "," => "COMMA".to_string(),
        "." => "PERIOD".to_string(),
        "/" => "SLASH".to_string(),
        _ => upper,
    }
}

fn hotkey_order(name: &str) -> (u8, usize) {
    let modifier_rank = match name {
        "LCTRL" => Some(0),
        "RCTRL" => Some(1),
        "LSHIFT" => Some(2),
        "RSHIFT" => Some(3),
        "LALT" => Some(4),
        "RALT" => Some(5),
        "LWIN" => Some(6),
        "RWIN" => Some(7),
        "MENU" => Some(8),
        _ => None,
    };
    if let Some(rank) = modifier_rank {
        return (0, rank);
    }

    let supported_rank = SUPPORTED_KEY_NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(usize::MAX);
    (1, supported_rank)
}

#[cfg(test)]
mod tests {
    use super::{
        display_hotkey_text, display_key_name, hotkey_registration, normalize_hotkey_text,
        parse_hotkey, parse_key_sequence, parse_key_specs, parse_single_key, sort_hotkey_names,
    };

    #[test]
    fn parse_key_specs_supports_case_insensitive_and_alias() {
        let keys = vec!["j".to_string(), "return".to_string(), "escape".to_string()];
        let specs = parse_key_specs(&keys).expect("key parse");
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].name, "J");
        assert_eq!(specs[1].name, "ENTER");
        assert_eq!(specs[2].name, "ESC");
    }

    #[test]
    fn parse_key_specs_supports_punctuation_and_navigation_aliases() {
        assert_eq!(parse_single_key("`").expect("`").name, "BACKQUOTE");
        assert_eq!(parse_single_key("~").expect("~").name, "BACKQUOTE");
        assert_eq!(parse_single_key("[").expect("[").name, "LBRACKET");
        assert_eq!(parse_single_key("pgup").expect("pgup").name, "PAGEUP");
        assert_eq!(parse_single_key("del").expect("del").name, "DELETE");
    }

    #[test]
    fn parse_key_specs_supports_extended_keys() {
        let keys = vec![
            "left".to_string(),
            "rctrl".to_string(),
            "numenter".to_string(),
        ];
        let specs = parse_key_specs(&keys).expect("extended key parse");
        assert_eq!(specs.len(), 3);
        assert!(specs.iter().all(|spec| spec.extended));
    }

    #[test]
    fn parse_key_specs_rejects_unknown_key() {
        let keys = vec!["F13".to_string()];
        assert!(parse_key_specs(&keys).is_err());
    }

    #[test]
    fn parse_key_sequence_keeps_duplicates_and_order() {
        let keys = vec!["a".to_string(), "a".to_string(), "d".to_string()];
        let specs = parse_key_sequence(&keys).expect("sequence parse");
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].name, "A");
        assert_eq!(specs[1].name, "A");
        assert_eq!(specs[2].name, "D");
    }

    #[test]
    fn parse_key_specs_deduplicates_aliases_but_keeps_distinct_sides() {
        let keys = vec![
            "return".to_string(),
            "enter".to_string(),
            "lshift".to_string(),
            "rshift".to_string(),
        ];
        let specs = parse_key_specs(&keys).expect("dedupe parse");
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].name, "ENTER");
        assert_eq!(specs[1].name, "LSHIFT");
        assert_eq!(specs[2].name, "RSHIFT");
    }

    #[test]
    fn parse_hotkey_supports_modifier_combos() {
        let specs = parse_hotkey("ctrl + shift + q").expect("hotkey parse");
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].name, "LCTRL");
        assert_eq!(specs[1].name, "LSHIFT");
        assert_eq!(specs[2].name, "Q");
    }

    #[test]
    fn parse_hotkey_rejects_modifier_only_combo() {
        assert!(parse_hotkey("ctrl+shift").is_err());
    }

    #[test]
    fn normalize_hotkey_text_canonicalizes_tokens() {
        assert_eq!(
            normalize_hotkey_text(" shift + ctrl + q ").expect("normalize hotkey"),
            "LCTRL+LSHIFT+Q"
        );
    }

    #[test]
    fn sort_hotkey_names_places_modifiers_first() {
        let sorted = sort_hotkey_names(vec![
            "Q".to_string(),
            "LSHIFT".to_string(),
            "LCTRL".to_string(),
        ]);
        assert_eq!(sorted, vec!["LCTRL", "LSHIFT", "Q"]);
    }

    #[test]
    fn hotkey_registration_supports_alt_combo() {
        let hotkey = hotkey_registration("alt+q").expect("registerable hotkey");
        assert_eq!(hotkey.modifiers, 0x0001);
        assert_eq!(hotkey.vk, 0x51);
    }

    #[test]
    fn hotkey_registration_supports_alt_tilde_alias() {
        let hotkey = hotkey_registration("alt+~").expect("registerable alt tilde hotkey");
        assert_eq!(hotkey.modifiers, 0x0001);
        assert_eq!(hotkey.vk, 0xC0);
    }

    #[test]
    fn hotkey_registration_rejects_multiple_main_keys() {
        assert!(hotkey_registration("ctrl+a+s").is_err());
    }

    #[test]
    fn display_key_name_uses_symbolic_labels_for_punctuation() {
        assert_eq!(display_key_name("BACKQUOTE"), "~");
        assert_eq!(display_key_name("MINUS"), "-");
        assert_eq!(display_key_name("EQUAL"), "=");
        assert_eq!(display_key_name("LBRACKET"), "[");
        assert_eq!(display_key_name("RBRACKET"), "]");
        assert_eq!(display_key_name("BACKSLASH"), "\\");
        assert_eq!(display_key_name("SEMICOLON"), ";");
        assert_eq!(display_key_name("APOSTROPHE"), "'");
        assert_eq!(display_key_name("COMMA"), ",");
        assert_eq!(display_key_name("PERIOD"), ".");
        assert_eq!(display_key_name("SLASH"), "/");
    }

    #[test]
    fn display_hotkey_text_uses_symbolic_labels() {
        assert_eq!(display_hotkey_text("LCTRL+BACKQUOTE"), "LCtrl+~");
        assert_eq!(display_hotkey_text("LALT+SEMICOLON"), "LAlt+;");
        assert_eq!(display_hotkey_text("RCTRL+SLASH"), "RCtrl+/");
    }
}
