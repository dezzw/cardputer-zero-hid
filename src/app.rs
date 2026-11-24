//! Application orchestration (M1: UI/HID input still stub; gadget via systemd).

use crate::gadget;
use crate::{hid, VERSION};

/// Run the application. M1 does not start UI / report loops yet.
pub fn run() -> Result<(), String> {
    run_with_options(RunOptions::from_env())
}

#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    pub quiet: bool,
}

impl RunOptions {
    pub fn from_env() -> Self {
        Self {
            quiet: std::env::var("CARDPUTER_ZERO_HID_M0_QUIET").is_ok(),
        }
    }
}

/// Core run logic (used by tests).
pub fn run_with_options(opts: RunOptions) -> Result<(), String> {
    if opts.quiet {
        return Ok(());
    }

    eprintln!(
        "cardputer-zero-hid v{VERSION} (M1 gadget service)\n\
         USB composite gadget: {} (service: cardputer-zero-hid-gadget.service)\n\
         On-device UI / HID report loop: not implemented yet.\n\
         UDC: {} active={}\n\
         Planned HID modes: {:?}",
        gadget::GADGET_NAME,
        gadget::EXPECTED_UDC,
        gadget::is_active(),
        hid::Mode::ALL.iter().map(|m| m.label()).collect::<Vec<_>>(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_succeeds_in_test_harness() {
        assert!(run_with_options(RunOptions { quiet: true }).is_ok());
    }
}
