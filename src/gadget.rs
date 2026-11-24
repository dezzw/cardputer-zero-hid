//! USB configfs / libcomposite gadget setup (stub until a later milestone).

/// Whether the composite HID gadget is active on the device.
pub fn is_active() -> bool {
    false
}

/// UDC name used on Cardputer Zero (right-side Type-C device port).
pub const EXPECTED_UDC: &str = "3f980000.usb";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m0_gadget_is_not_active() {
        assert!(!is_active());
    }

    #[test]
    fn expected_udc_matches_device_docs() {
        assert_eq!(EXPECTED_UDC, "3f980000.usb");
    }
}
