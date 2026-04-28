// Exposes the application library entry points for the binary.

mod autofire;
mod cli;
mod config;
mod gui;
mod input;
mod keymap;
mod platform;
mod timing;

pub use cli::run;
