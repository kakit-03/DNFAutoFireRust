// Detects foreground windows and IME state for target matching.

use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::ProcessStatus::GetProcessImageFileNameW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmGetOpenStatus, ImmReleaseContext};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

#[derive(Debug, Clone)]
pub struct ForegroundWindowInfo {
    pub hwnd: HWND,
    pub title: String,
    pub class_name: String,
    pub process_name: String,
}

impl ForegroundWindowInfo {
    pub fn matches_any_target(&self, target_tokens: &[String]) -> bool {
        target_tokens.iter().any(|token| {
            target_token_matches(token, &self.title)
                || target_token_matches(token, &self.class_name)
                || target_token_matches(token, &self.process_name)
        })
    }

    #[cfg(test)]
    pub fn is_probable_dnf_window(&self) -> bool {
        self.class_name == "地下城与勇士"
            || self.title.contains("地下城与勇士")
            || self.process_name.eq_ignore_ascii_case("DNF.exe")
    }
}

pub fn foreground_window_info() -> Option<ForegroundWindowInfo> {
    let hwnd = foreground_window()?;
    Some(ForegroundWindowInfo {
        hwnd,
        title: window_text(hwnd),
        class_name: window_class_name(hwnd),
        process_name: window_process_name(hwnd),
    })
}

pub fn window_ime_open(hwnd: HWND) -> bool {
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

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(&buf[..len as usize])
}

fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(&buf[..len as usize])
}

fn window_process_name(hwnd: HWND) -> String {
    let mut process_id = 0u32;
    unsafe {
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == 0 {
        return String::new();
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) };
    let Ok(process) = process else {
        return String::new();
    };

    let mut buf = vec![0u16; 1024];
    let len = unsafe { GetProcessImageFileNameW(process, &mut buf) };
    let process_name = if len > 0 {
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        String::new()
    };

    unsafe {
        let _ = CloseHandle(process);
    }

    process_name
}

fn target_token_matches(token: &str, value: &str) -> bool {
    let token = token.trim();
    if token.is_empty() || value.is_empty() {
        return false;
    }

    value.to_lowercase().contains(&token.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::ForegroundWindowInfo;
    use windows::Win32::Foundation::HWND;

    #[test]
    fn target_window_match_works_for_title_class_and_process() {
        let targets = vec!["DNF".to_string(), "地下城与勇士".to_string()];
        let dnf_by_title = ForegroundWindowInfo {
            hwnd: HWND::default(),
            title: "地下城与勇士：创新世纪".to_string(),
            class_name: String::new(),
            process_name: String::new(),
        };
        let dnf_by_process = ForegroundWindowInfo {
            hwnd: HWND::default(),
            title: String::new(),
            class_name: String::new(),
            process_name: "dnf.exe".to_string(),
        };
        let notepad = ForegroundWindowInfo {
            hwnd: HWND::default(),
            title: "Notepad".to_string(),
            class_name: "Notepad".to_string(),
            process_name: "notepad.exe".to_string(),
        };

        assert!(dnf_by_title.matches_any_target(&targets));
        assert!(dnf_by_process.matches_any_target(&targets));
        assert!(!notepad.matches_any_target(&targets));
    }

    #[test]
    fn probable_dnf_window_detects_known_signatures() {
        let info = ForegroundWindowInfo {
            hwnd: HWND::default(),
            title: String::new(),
            class_name: "地下城与勇士".to_string(),
            process_name: String::new(),
        };

        assert!(info.is_probable_dnf_window());
    }
}
