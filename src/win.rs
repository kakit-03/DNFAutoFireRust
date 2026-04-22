use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmGetOpenStatus, ImmReleaseContext};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};

pub fn foreground_window_title() -> String {
    let Some(hwnd) = foreground_window() else {
        return String::new();
    };

    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(&buf[..len as usize])
}

pub fn is_target_window(title: &str, target_tokens: &[String]) -> bool {
    !title.is_empty()
        && target_tokens
            .iter()
            .any(|token| !token.is_empty() && title.contains(token))
}

pub fn foreground_window_ime_open() -> bool {
    let Some(hwnd) = foreground_window() else {
        return false;
    };

    unsafe {
        let imc = ImmGetContext(hwnd);
        if imc.0.is_null() {
            return false;
        }

        let is_open = ImmGetOpenStatus(imc).as_bool();
        let _ = ImmReleaseContext(hwnd, imc);
        is_open
    }
}

pub fn foreground_window_is(target: HWND) -> bool {
    foreground_window().is_some_and(|hwnd| hwnd == target)
}

fn foreground_window() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() { None } else { Some(hwnd) }
}

#[cfg(test)]
mod tests {
    use super::is_target_window;

    #[test]
    fn target_window_match_works() {
        let targets = vec!["DNF".to_string(), "地下城与勇士".to_string()];
        assert!(is_target_window("地下城与勇士：创新世纪", &targets));
        assert!(is_target_window("DNF", &targets));
        assert!(!is_target_window("Notepad", &targets));
    }
}
