use crate::keymap::KeySpec;
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, keybd_event,
};

pub const VK_ESCAPE: u16 = 0x1B;

pub fn is_vk_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

pub fn send_key_once(key: KeySpec, press_duration: Duration) {
    let down_flags = if key.extended {
        KEYEVENTF_EXTENDEDKEY
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    let up_flags = if key.extended {
        KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
    } else {
        KEYEVENTF_KEYUP
    };

    unsafe {
        // Keep behavior close to DNFAutoFire Python version:
        // set VK to 0xFF and rely on scan code for key dispatch.
        keybd_event(0xFF, key.scan, down_flags, 0);
    }
    sleep(press_duration);
    unsafe {
        keybd_event(0xFF, key.scan, up_flags, 0);
    }
}
