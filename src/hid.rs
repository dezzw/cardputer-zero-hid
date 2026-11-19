//! HID report / endpoint layer (stub until a later milestone).

/// Planned operating mode for the composite gadget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Keyboard,
    Mouse,
    Combo,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Keyboard, Self::Mouse, Self::Combo];

    pub fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Combo => "keyboard+mouse",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Keyboard => Self::Mouse,
            Self::Mouse => Self::Combo,
            Self::Combo => Self::Keyboard,
        }
    }
}

/// M0: reports are not emitted yet.
pub fn can_emit_reports() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_cycles_through_all_variants() {
        let mut m = Mode::Keyboard;
        m = m.next();
        assert_eq!(m, Mode::Mouse);
        m = m.next();
        assert_eq!(m, Mode::Combo);
        m = m.next();
        assert_eq!(m, Mode::Keyboard);
    }

    #[test]
    fn m0_cannot_emit_reports() {
        assert!(!can_emit_reports());
    }
}
