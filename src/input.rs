//! Built-in matrix keyboard and related input (stub until a later milestone).

/// Display width in pixels (ST7789 panel).
pub const SCREEN_WIDTH: u16 = 320;

/// Display height in pixels.
pub const SCREEN_HEIGHT: u16 = 170;

/// Matrix key count on Cardputer Zero.
pub const KEY_COUNT: usize = 46;

/// M0: no hardware input polling.
pub fn poll_key() -> Option<char> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_matches_cardputer_zero_spec() {
        assert_eq!((SCREEN_WIDTH, SCREEN_HEIGHT), (320, 170));
    }

    #[test]
    fn m0_poll_returns_none() {
        assert_eq!(poll_key(), None);
    }
}
