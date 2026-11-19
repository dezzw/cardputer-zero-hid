//! Cardputer Zero HID binary entry point (M0: scaffold, no USB HID yet).

use cardputer_zero_hid::app;

fn main() {
    if let Err(err) = app::run() {
        eprintln!("cardputer-zero-hid: {err}");
        std::process::exit(1);
    }
}
