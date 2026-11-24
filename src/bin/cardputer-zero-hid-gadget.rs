//! Privileged oneshot: create or remove the USB composite HID gadget (M1).

use cardputer_zero_hid::gadget::{
    clear_restore_state, load_restore_state, persist_restore_state, teardown, take_over_confirmed,
    SetupOptions, SetupResult, SysfsLayout, RESTORE_STATE_FILE,
};
use std::env;
use std::path::Path;
use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "usage: cardputer-zero-hid-gadget [--take-over-udc]\n\
               cardputer-zero-hid-gadget --remove\n\
         \n\
         Creates a boot-protocol keyboard+mouse composite on UDC 3f980000.usb.\n\
         Without --take-over-udc (or TAKE_OVER_UDC=1 in conffile), exits without\n\
         changing anything when another gadget already owns the UDC."
    );
    exit(2);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    if arg_refs.len() < 2 {
        usage();
    }

    let layout = SysfsLayout::default();
    let state_path = Path::new(RESTORE_STATE_FILE);

    if arg_refs.iter().any(|a| *a == "--remove" || *a == "--teardown") {
        let restore = load_restore_state(state_path);
        if let Err(e) = teardown(&layout, restore.as_ref()) {
            eprintln!("cardputer-zero-hid-gadget: teardown failed: {e}");
            exit(1);
        }
        clear_restore_state(state_path);
        exit(0);
    }

    if arg_refs.iter().any(|a| *a == "--help" || *a == "-h") {
        usage();
    }

    let take_over = take_over_confirmed(&arg_refs[1..]);
    let mut restore = None;
    match cardputer_zero_hid::gadget::setup(&layout, SetupOptions { take_over_udc: take_over }, &mut restore)
    {
        SetupResult::Active => {
            if let Some(ref state) = restore {
                if let Err(e) = persist_restore_state(state_path, state) {
                    let _ = teardown(&layout, Some(state));
                    eprintln!("cardputer-zero-hid-gadget: failed to persist restore state: {e}");
                    exit(1);
                }
            } else {
                clear_restore_state(state_path);
            }
            eprintln!(
                "cardputer-zero-hid-gadget: composite HID active on {}",
                cardputer_zero_hid::gadget::EXPECTED_UDC
            );
            exit(0);
        }
        SetupResult::Conflict(c) => {
            eprintln!(
                "cardputer-zero-hid-gadget: UDC {} already bound to gadget `{}`\n\
                 No changes made. Use --take-over-udc (or TAKE_OVER_UDC=1 in\n\
                 /etc/cardputer-zero-hid-app.conf) to take over, or stop the\n\
                 other gadget (e.g. jiggler-hid-gadget.service) first.",
                c.udc, c.gadget_name
            );
            exit(0);
        }
        SetupResult::Error(e) => {
            eprintln!("cardputer-zero-hid-gadget: {e}");
            exit(1);
        }
    }
}