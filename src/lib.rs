//! Cardputer Zero HID — library modules (M0 scaffold only).

pub mod app;
pub mod gadget;
pub mod gadget_daemon;
pub mod hid;
pub mod input;
pub mod ui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
