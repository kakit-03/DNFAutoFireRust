use crate::keymap::KeySpec;
use crate::timing::HighPrecisionSleeper;
use std::mem::size_of;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, SendInput, VIRTUAL_KEY,
};

pub fn is_vk_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

pub fn send_key_once(key: KeySpec, press_duration: Duration, sleeper: &HighPrecisionSleeper) {
    send_key_event(key, false);
    sleeper.sleep_for(press_duration);
    send_key_event(key, true);
}

fn send_key_event(key: KeySpec, key_up: bool) {
    let down_flags = if key.extended {
        KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY
    } else {
        KEYEVENTF_SCANCODE
    };
    let flags = if key_up {
        down_flags | KEYEVENTF_KEYUP
    } else {
        down_flags
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: key.scan as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    unsafe {
        let sent = SendInput(&[input], size_of::<INPUT>() as i32);
        debug_assert_eq!(
            sent, 1,
            "SendInput should submit exactly one keyboard event"
        );
    }
}
