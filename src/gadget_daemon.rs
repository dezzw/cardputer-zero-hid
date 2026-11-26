//! CLI entry for `cardputer-zero-hid-gadget` (testable without hardware).

use crate::gadget::{
    clear_restore_state, exit_code_for_setup, load_restore_state, parse_daemon_action,
    persist_restore_state, restore_state_is_applicable, setup, take_over_confirmed, teardown,
    DaemonAction, SetupOptions, SetupResult, SysfsLayout, RESTORE_STATE_FILE,
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
                 binding unless --take-over-udc (or TAKE_OVER_UDC=1 / env) is set.\n\
                 Exit 0 only when this gadget is bound; exit 3 when the UDC was busy\n\
                 or yielded (systemd RemainAfterExit must not look \"active\")."
            );
            2
        }
        DaemonAction::Remove => {
            let restore =
                load_restore_state(state_path).filter(|s| restore_state_is_applicable(&layout, s));
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
            let result = setup(
                &layout,
                SetupOptions {
                    take_over_udc: take_over,
                },
                &mut restore,
            );
            let code = finish_setup(&layout, state_path, &result, restore);
            log_setup_result(&result);
            code
        }
    }
}

fn finish_setup(
    layout: &SysfsLayout,
    state_path: &Path,
    result: &SetupResult,
    restore: Option<crate::gadget::RestoreState>,
) -> i32 {
    match result {
        SetupResult::Active => {
            if let Some(ref state) = restore {
                if let Err(e) = persist_restore_state(state_path, state) {
                    let _ = teardown(layout, Some(state));
                    eprintln!("cardputer-zero-hid-gadget: failed to persist restore state: {e}");
                    return 1;
                }
            } else {
                clear_restore_state(state_path);
            }
            exit_code_for_setup(result)
        }
        SetupResult::Conflict(_) | SetupResult::YieldPeers(_) => {
            clear_restore_state(state_path);
            exit_code_for_setup(result)
        }
        SetupResult::Error(_) => 1,
    }
}

fn log_setup_result(result: &SetupResult) {
    match result {
        SetupResult::Active => {
            eprintln!(
                "cardputer-zero-hid-gadget: composite HID active on {}",
                crate::gadget::EXPECTED_UDC
            );
        }
        SetupResult::Conflict(c) => {
            eprintln!(
                "cardputer-zero-hid-gadget: UDC {} busy ({:?}: `{}`); no gadget created",
                c.udc, c.source, c.occupant
            );
        }
        SetupResult::YieldPeers(units) => {
            eprintln!(
                "cardputer-zero-hid-gadget: yielding to enabled peer service(s) {units:?}; \
                 no gadget created"
            );
        }
        SetupResult::Error(e) => eprintln!("cardputer-zero-hid-gadget: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gadget::{
        ConflictSource, DaemonAction, SetupResult, UdcConflict, EXIT_SETUP_BOUND,
        EXIT_SETUP_NOT_BOUND, EXPECTED_UDC,
    };

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

    #[test]
    fn setup_exit_codes_for_systemd() {
        assert_eq!(exit_code_for_setup(&SetupResult::Active), EXIT_SETUP_BOUND);
        let conflict = UdcConflict {
            udc: EXPECTED_UDC.to_string(),
            occupant: "cp0hid".into(),
            source: ConflictSource::Configfs,
        };
        assert_eq!(
            exit_code_for_setup(&SetupResult::Conflict(conflict)),
            EXIT_SETUP_NOT_BOUND
        );
    }
}
