pub const INITIAL_CONFIG_JSON: &str = r#"{
  "default_profile": "默认配置",
  "quick_switch_hotkey": "LALT+BACKQUOTE",
  "target_windows": [
    "地下城与勇士",
    "DNF"
  ],
  "profiles": {
    "默认配置": {
      "enabled_keys": [
        "H",
        "J"
      ],
      "repeat_interval_ms": 1,
      "press_duration_ms": 1,
      "poll_interval_ms": 1,
      "combos": [
        {
          "name": "忍者连招",
          "trigger_key": "T",
          "steps": [
            {
              "key": "COMMA",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "O",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "I",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "U",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "Q",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "E",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "C",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "M",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "X",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "F",
              "interval_ms": 1,
              "press_duration_ms": 20
            },
            {
              "key": "SEMICOLON",
              "interval_ms": 1,
              "press_duration_ms": 20
            }
          ]
        },
        {
          "name": "一键buff",
          "trigger_key": "8",
          "steps": [
            {
              "key": "D",
              "interval_ms": 5,
              "press_duration_ms": 20
            },
            {
              "key": "D",
              "interval_ms": 5,
              "press_duration_ms": 20
            },
            {
              "key": "SPACE",
              "interval_ms": 10,
              "press_duration_ms": 20
            },
            {
              "key": "W",
              "interval_ms": 5,
              "press_duration_ms": 20
            },
            {
              "key": "W",
              "interval_ms": 5,
              "press_duration_ms": 20
            },
            {
              "key": "SPACE",
              "interval_ms": 10,
              "press_duration_ms": 20
            },
            {
              "key": "SPACE",
              "interval_ms": 5,
              "press_duration_ms": 20
            }
          ]
        }
      ],
      "special_keys": [
        {
          "kind": "custom_autofire",
          "name": "帝国剑术",
          "key": "O",
          "repeat_interval_ms": 150,
          "press_duration_ms": 5
        },
        {
          "kind": "linked_key",
          "name": "唱歌",
          "trigger_key": "M",
          "linked_key": "COMMA",
          "trigger_mode": "release",
          "interval_ms": 1,
          "press_duration_ms": 20
        },
        {
          "kind": "linked_key",
          "name": "唱歌2",
          "trigger_key": "M",
          "linked_key": "SEMICOLON",
          "trigger_mode": "release",
          "interval_ms": 20,
          "press_duration_ms": 20
        },
        {
          "kind": "auto_trigger",
          "name": "自动空格",
          "key": "SPACE",
          "trigger_hotkey": "LALT+8",
          "repeat_interval_ms": 4000,
          "press_duration_ms": 20
        },
        {
          "kind": "linked_key",
          "name": "唱歌3",
          "trigger_key": "M",
          "linked_key": "U",
          "trigger_mode": "press",
          "interval_ms": 60,
          "press_duration_ms": 20
        }
      ]
    }
  }
}"#;
