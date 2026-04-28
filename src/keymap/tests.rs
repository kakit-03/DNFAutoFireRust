// Covers key parsing, hotkey normalization, and display labels.

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
