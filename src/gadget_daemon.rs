//! CLI entry for `cardputer-zero-hid-gadget` (testable without hardware).

use crate::gadget::{
    clear_restore_state, load_restore_state, parse_daemon_action, persist_restore_state, setup,
    take_over_confirmed, teardown, DaemonAction, SetupOptions, SetupResult, SysfsLayout,
    RESTORE_STATE_FILE,
};
use std::path::Path;

/// Process argv (`args[0]` = program name) and return a shell exit code.
pub fn run_with_args(args: &[&str]) -> i32 {
    let layout = SysfsLayout::default();
    let state_path = Path::new(RESTORE_STATE_FILE);

    match parse_daemon_action(args) {
        DaemonAction::Help => {
            eprintln!(
                "usage: cardputer-zero-hid-gadget [--take-over-udc]\n\
                       cardputer-zero-hid-gadget --remove\n\
                 \n\
                 With no extra arguments (as systemd ExecStart), creates the composite\n\
                 HID gadget when the UDC is free and no peer boot gadget service is\n\
                 enabled (jiggler-hid-gadget, adbd, …). Does not take over another\n\
                 binding unless --take-over-udc (or TAKE_OVER_UDC=1 / env) is set."
            );
            2
        }
        DaemonAction::Remove => {
            let restore = load_restore_state(state_path);
            if let Err(e) = teardown(&layout, restore.as_ref()) {
                eprintln!("cardputer-zero-hid-gadget: teardown failed: {e}");
                return 1;
            }
            clear_restore_state(state_path);
            0
        }
        DaemonAction::Setup => {
            let flag_args = if args.len() > 1 {
                &args[1..]
            } else {
                &[] as &[&str]
            };
            let take_over = take_over_confirmed(flag_args);
            let mut restore = None;
            match setup(
                &layout,
                SetupOptions {
                    take_over_udc: take_over,
                },
                &mut restore,
            ) {
                SetupResult::Active => {
                    if let Some(ref state) = restore {
                        if let Err(e) = persist_restore_state(state_path, state) {
                            let _ = teardown(&layout, Some(state));
                            eprintln!(
                                "cardputer-zero-hid-gadget: failed to persist restore state: {e}"
                            );
                            return 1;
                        }
                    } else {
                        clear_restore_state(state_path);
                    }
                    eprintln!(
                        "cardputer-zero-hid-gadget: composite HID active on {}",
                        crate::gadget::EXPECTED_UDC
                    );
                    0
                }
                SetupResult::Conflict(c) => {
                    eprintln!(
                        "cardputer-zero-hid-gadget: UDC {} already bound to gadget `{}`\n\
                         No changes made. Use --take-over-udc (or TAKE_OVER_UDC=1 in\n\
                         /etc/cardputer-zero-hid-app.conf) to take over, or stop the\n\
                         other gadget (e.g. jiggler-hid-gadget.service) first.",
                        c.udc, c.gadget_name
                    );
                    0
                }
                SetupResult::YieldPeers(units) => {
                    eprintln!(
                        "cardputer-zero-hid-gadget: UDC {} is free but peer gadget service(s)\n\
                         are enabled ({units:?}); not binding. Disable the peer service(s) or\n\
                         set TAKE_OVER_UDC=1 to take over explicitly.",
                        crate::gadget::EXPECTED_UDC
                    );
                    0
                }
                SetupResult::Error(e) => {
                    eprintln!("cardputer-zero-hid-gadget: {e}");
                    1
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gadget::DaemonAction;

    #[test]
    fn no_args_defaults_to_setup_path() {
        assert_eq!(
            parse_daemon_action(&["cardputer-zero-hid-gadget"]),
            DaemonAction::Setup
        );
        assert_eq!(
            parse_daemon_action(&["cardputer-zero-hid-gadget", "--take-over-udc"]),
            DaemonAction::Setup
        );
    }

    #[test]
    fn remove_flag_parsed() {
        assert_eq!(
            parse_daemon_action(&["prog", "--remove"]),
            DaemonAction::Remove
        );
    }
}
