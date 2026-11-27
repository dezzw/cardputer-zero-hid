//! Privileged oneshot: create or remove the USB composite HID gadget (M1).

use cardputer_zero_hid::gadget_daemon;
use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    process::exit(gadget_daemon::run_with_args(&arg_refs));
}
