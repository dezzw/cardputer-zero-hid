//! Application orchestration (M0: prints scaffold banner and exits).

use crate::gadget;
use crate::{hid, VERSION};

/// Run the application. M0 does not start HID or UI loops.
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
        "cardputer-zero-hid v{VERSION} (M0 scaffold)\n\
         USB gadget/HID/UI are not implemented yet.\n\
         Expected UDC: {}\n\
         Planned HID modes: {:?}",
        gadget::EXPECTED_UDC,
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
