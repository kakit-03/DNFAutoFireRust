#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    if let Err(err) = dnf_auto_fire::run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
