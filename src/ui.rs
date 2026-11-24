//! On-device UI (320×170) — stub until a later milestone.

use crate::hid::Mode;
use crate::input::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Minimal UI state for tests and future rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiState {
    pub mode: Mode,
    pub title: &'static str,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            mode: Mode::Combo,
            title: "Cardputer Zero HID",
        }
    }
}

pub fn viewport_pixels() -> (u16, u16) {
    (SCREEN_WIDTH, SCREEN_HEIGHT)
}

/// M0: no framebuffer / LVGL yet.
pub fn render(_state: &UiState) -> Result<(), &'static str> {
    Err("ui not implemented (M0 scaffold)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ui_uses_combo_mode() {
        let ui = UiState::default();
        assert_eq!(ui.mode, Mode::Combo);
    }

    #[test]
    fn render_is_stubbed() {
        assert!(render(&UiState::default()).is_err());
    }
}
