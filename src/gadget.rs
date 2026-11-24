//! USB configfs / libcomposite composite HID gadget setup and UDC conflict handling.

use crate::hid::{
    BOOT_KEYBOARD_REPORT_DESC, BOOT_MOUSE_REPORT_DESC, BOOT_SUBCLASS, KEYBOARD_PROTOCOL,
    KEYBOARD_REPORT_LEN, MOUSE_PROTOCOL, MOUSE_REPORT_LEN,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// UDC name on Cardputer Zero (right-side Type-C device port).
pub const EXPECTED_UDC: &str = "3f980000.usb";

/// configfs gadget directory name (distinct from jiggler's `cp0hid`).
pub const GADGET_NAME: &str = "cardputer_zero_hid";

/// Known gadget names that map to a systemd unit to restart on restore.
pub const JIGGLER_GADGET_NAME: &str = "cp0hid";

/// Runtime file written after a takeover so `ExecStop` can restore the prior UDC owner.
pub const RESTORE_STATE_FILE: &str = "/run/cardputer-zero-hid-gadget.restore";

/// Exec paths for optional systemd restore (tests may point `run_root` elsewhere).
#[derive(Debug, Clone)]
pub struct SysfsLayout {
    pub configfs_gadgets: PathBuf,
    pub udc_class: PathBuf,
    pub run_systemd: PathBuf,
}

impl Default for SysfsLayout {
    fn default() -> Self {
        Self {
            configfs_gadgets: PathBuf::from("/sys/kernel/config/usb_gadget"),
            udc_class: PathBuf::from("/sys/class/udc"),
            run_systemd: PathBuf::from("/run/systemd"),
        }
    }
}

impl SysfsLayout {
    pub fn for_test(root: &Path) -> Self {
        Self {
            configfs_gadgets: root.join("sys/kernel/config/usb_gadget"),
            udc_class: root.join("sys/class/udc"),
            run_systemd: root.join("run/systemd"),
        }
    }

    pub fn gadget_dir(&self, name: &str) -> PathBuf {
        self.configfs_gadgets.join(name)
    }

    pub fn gadget_udc_file(&self, name: &str) -> PathBuf {
        self.gadget_dir(name).join("UDC")
    }
}

/// Another gadget currently bound to [`EXPECTED_UDC`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdcConflict {
    pub gadget_name: String,
    pub udc: String,
}

/// State saved when [`SetupOptions::take_over_udc`] is used, for teardown restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreState {
    pub previous_gadget: String,
    pub stopped_systemd_unit: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct SetupOptions {
    pub take_over_udc: bool,
}

/// Result of attempting setup (used by the daemon and unit tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupResult {
    Active,
    Conflict(UdcConflict),
    Error(String),
}

fn read_to_string(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("read {}: {e}", path.display()))
}

fn write_str(path: &Path, data: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut f = fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    f.write_all(data.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn mkdir_all(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("mkdir {}: {e}", path.display()))
}

/// Scan configfs for which gadget (if any) is bound to `udc`.
pub fn find_gadget_bound_to_udc(layout: &SysfsLayout, udc: &str) -> Result<Option<String>, String> {
    let entries = match fs::read_dir(&layout.configfs_gadgets) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("list {}: {e}", layout.configfs_gadgets.display())),
    };

    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let udc_path = layout.gadget_udc_file(&name);
        if !udc_path.is_file() {
            continue;
        }
        let bound = read_to_string(&udc_path)?;
        if !bound.is_empty() && bound == udc {
            return Ok(Some(name.into_owned()));
        }
    }
    Ok(None)
}

/// Detect whether `udc` is in use by a gadget other than [`GADGET_NAME`].
pub fn detect_udc_conflict(layout: &SysfsLayout, udc: &str) -> Result<Option<UdcConflict>, String> {
    match find_gadget_bound_to_udc(layout, udc)? {
        Some(name) if name == GADGET_NAME => Ok(None),
        Some(gadget_name) => Ok(Some(UdcConflict {
            gadget_name,
            udc: udc.to_string(),
        })),
        None => Ok(None),
    }
}

fn systemd_unit_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Map a well-known gadget directory to a boot service to restart after we release the UDC.
pub fn systemd_unit_for_gadget(gadget_name: &str) -> Option<&'static str> {
    match gadget_name {
        JIGGLER_GADGET_NAME => Some("jiggler-hid-gadget.service"),
        _ => None,
    }
}

fn stop_conflicting_unit(gadget_name: &str) -> Option<String> {
    let unit = systemd_unit_for_gadget(gadget_name)?;
    if systemd_unit_active(unit) {
        let _ = Command::new("systemctl").args(["stop", unit]).status();
        Some(unit.to_string())
    } else {
        None
    }
}

fn start_systemd_unit(unit: &str) {
    let _ = Command::new("systemctl").args(["start", unit]).status();
}

fn unbind_gadget(layout: &SysfsLayout, gadget_name: &str) -> Result<(), String> {
    let udc_file = layout.gadget_udc_file(gadget_name);
    if udc_file.is_file() {
        write_str(&udc_file, "")?;
    }
    Ok(())
}

fn bind_gadget(layout: &SysfsLayout, gadget_name: &str, udc: &str) -> Result<(), String> {
    write_str(&layout.gadget_udc_file(gadget_name), udc)
}

/// Whether our composite gadget is bound to the expected UDC.
pub fn is_active() -> bool {
    let layout = SysfsLayout::default();
    find_gadget_bound_to_udc(&layout, EXPECTED_UDC)
        .ok()
        .flatten()
        .as_deref()
        == Some(GADGET_NAME)
}

fn modprobe_libcomposite() {
    let _ = Command::new("modprobe").arg("libcomposite").status();
}

/// Persist takeover state for a later `teardown` (systemd oneshot `ExecStop`).
pub fn persist_restore_state(path: &Path, state: &RestoreState) -> Result<(), String> {
    let unit = state.stopped_systemd_unit.as_deref().unwrap_or("");
    let body = format!(
        "previous_gadget={}\nstopped_unit={unit}\n",
        state.previous_gadget
    );
    write_str(path, &body)
}

/// Load persisted takeover state (if any).
pub fn load_restore_state(path: &Path) -> Option<RestoreState> {
    let content = fs::read_to_string(path).ok()?;
    let mut gadget = None;
    let mut unit = None;
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("previous_gadget=") {
            gadget = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("stopped_unit=") {
            let v = v.trim();
            if !v.is_empty() {
                unit = Some(v.to_string());
            }
        }
    }
    let previous_gadget = gadget?;
    Some(RestoreState {
        previous_gadget,
        stopped_systemd_unit: unit,
    })
}

pub fn clear_restore_state(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Remove our configfs gadget tree (idempotent).
pub fn teardown(layout: &SysfsLayout, restore: Option<&RestoreState>) -> Result<(), String> {
    let g = layout.gadget_dir(GADGET_NAME);
    if !g.is_dir() {
        if let Some(state) = restore {
            restore_previous(layout, state)?;
        }
        return Ok(());
    }

    let _ = unbind_gadget(layout, GADGET_NAME);

    let links = [
        g.join("configs/c.1/hid.usb0"),
        g.join("configs/c.1/hid.usb1"),
    ];
    for link in &links {
        if link.exists() {
            fs::remove_file(link).ok();
        }
    }

    for path in [
        g.join("configs/c.1/strings/0x409"),
        g.join("configs/c.1"),
        g.join("functions/hid.usb0"),
        g.join("functions/hid.usb1"),
        g.join("strings/0x409"),
    ] {
        if path.is_dir() {
            let _ = fs::remove_dir(&path);
        }
    }
    if g.is_dir() {
        let _ = fs::remove_dir_all(&g);
    }

    if let Some(state) = restore {
        restore_previous(layout, state)?;
    }
    Ok(())
}

fn restore_previous(layout: &SysfsLayout, state: &RestoreState) -> Result<(), String> {
    let prev_gadget_dir = layout.gadget_dir(&state.previous_gadget);
    if prev_gadget_dir.is_dir() {
        bind_gadget(layout, &state.previous_gadget, EXPECTED_UDC)?;
    } else if let Some(ref unit) = state.stopped_systemd_unit {
        start_systemd_unit(unit);
    }
    Ok(())
}

fn create_hid_function(
    layout: &SysfsLayout,
    func: &str,
    protocol: u8,
    report_len: u8,
    report_desc: &[u8],
) -> Result<(), String> {
    let base = layout
        .gadget_dir(GADGET_NAME)
        .join(format!("functions/{func}"));
    mkdir_all(&base)?;
    write_str(&base.join("protocol"), &protocol.to_string())?;
    write_str(&base.join("subclass"), &BOOT_SUBCLASS.to_string())?;
    write_str(&base.join("report_length"), &report_len.to_string())?;
    fs::write(base.join("report_desc"), report_desc)
        .map_err(|e| format!("write {}: {e}", base.join("report_desc").display()))?;
    Ok(())
}

fn symlink_function(layout: &SysfsLayout, func: &str) -> Result<(), String> {
    let from = layout
        .gadget_dir(GADGET_NAME)
        .join(format!("functions/{func}"));
    let to = layout
        .gadget_dir(GADGET_NAME)
        .join(format!("configs/c.1/{func}"));
    if to.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&from, &to).map_err(|e| format!("symlink {}: {e}", to.display()))
    }
    #[cfg(not(unix))]
    {
        Err("symlink not supported on this platform".into())
    }
}

/// Create and bind the composite HID gadget.
pub fn setup(
    layout: &SysfsLayout,
    options: SetupOptions,
    restore_out: &mut Option<RestoreState>,
) -> SetupResult {
    modprobe_libcomposite();

    let conflict = match detect_udc_conflict(layout, EXPECTED_UDC) {
        Ok(c) => c,
        Err(e) => return SetupResult::Error(e),
    };

    if let Some(c) = conflict {
        if !options.take_over_udc {
            return SetupResult::Conflict(c);
        }
        let stopped = stop_conflicting_unit(&c.gadget_name);
        if let Err(e) = unbind_gadget(layout, &c.gadget_name) {
            return SetupResult::Error(e);
        }
        *restore_out = Some(RestoreState {
            previous_gadget: c.gadget_name,
            stopped_systemd_unit: stopped,
        });
    }

    if layout.gadget_dir(GADGET_NAME).is_dir() {
        if let Err(e) = teardown(layout, None) {
            return SetupResult::Error(e);
        }
    }

    let g = layout.gadget_dir(GADGET_NAME);

    if let Err(e) = (|| -> Result<(), String> {
        mkdir_all(&g)?;
        write_str(&g.join("idVendor"), "0x1d6b")?;
        write_str(&g.join("idProduct"), "0x0104")?;
        write_str(&g.join("bcdDevice"), "0x0100")?;
        write_str(&g.join("bcdUSB"), "0x0200")?;
        mkdir_all(&g.join("strings/0x409"))?;
        write_str(&g.join("strings/0x409/serialnumber"), "CZHID0001")?;
        write_str(&g.join("strings/0x409/manufacturer"), "CardputerZero")?;
        write_str(&g.join("strings/0x409/product"), "USB Keyboard and Mouse")?;

        create_hid_function(
            layout,
            "hid.usb0",
            KEYBOARD_PROTOCOL,
            KEYBOARD_REPORT_LEN,
            &BOOT_KEYBOARD_REPORT_DESC,
        )?;
        create_hid_function(
            layout,
            "hid.usb1",
            MOUSE_PROTOCOL,
            MOUSE_REPORT_LEN,
            &BOOT_MOUSE_REPORT_DESC,
        )?;

        mkdir_all(&g.join("configs/c.1/strings/0x409"))?;
        write_str(
            &g.join("configs/c.1/strings/0x409/configuration"),
            "Keyboard + Mouse",
        )?;
        write_str(&g.join("configs/c.1/MaxPower"), "250")?;
        symlink_function(layout, "hid.usb0")?;
        symlink_function(layout, "hid.usb1")?;

        bind_gadget(layout, GADGET_NAME, EXPECTED_UDC)
    })() {
        let _ = teardown(layout, restore_out.as_ref());
        return SetupResult::Error(e);
    }

    SetupResult::Active
}

/// Parse `TAKE_OVER_UDC=1` from the app conffile (simple key=value lines).
pub fn take_over_from_config(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    content.lines().any(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        line.eq_ignore_ascii_case("TAKE_OVER_UDC=1")
            || line.eq_ignore_ascii_case("TAKE_OVER_UDC=yes")
    })
}

/// CLI / environment override for takeover confirmation.
pub fn take_over_confirmed(args: &[&str]) -> bool {
    if args.contains(&"--take-over-udc") {
        return true;
    }
    if std::env::var_os("CARDPUTER_ZERO_HID_TAKE_OVER_UDC").is_some() {
        return true;
    }
    take_over_from_config(Path::new("/etc/cardputer-zero-hid-app.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn seed_conflict_root(root: &Path, gadget: &str, udc: &str) {
        let udc_file = root
            .join("sys/kernel/config/usb_gadget")
            .join(gadget)
            .join("UDC");
        mkdir_all(udc_file.parent().unwrap()).unwrap();
        write_str(&udc_file, udc).unwrap();
    }

    #[test]
    fn expected_udc_matches_device_docs() {
        assert_eq!(EXPECTED_UDC, "3f980000.usb");
    }

    #[test]
    fn detect_conflict_when_jiggler_bound() {
        let tmp = tempfile::tempdir().unwrap();
        seed_conflict_root(tmp.path(), JIGGLER_GADGET_NAME, EXPECTED_UDC);
        let layout = SysfsLayout::for_test(tmp.path());
        let c = detect_udc_conflict(&layout, EXPECTED_UDC).unwrap().unwrap();
        assert_eq!(c.gadget_name, JIGGLER_GADGET_NAME);
    }

    #[test]
    fn no_conflict_when_only_our_gadget_bound() {
        let tmp = tempfile::tempdir().unwrap();
        seed_conflict_root(tmp.path(), GADGET_NAME, EXPECTED_UDC);
        let layout = SysfsLayout::for_test(tmp.path());
        assert!(detect_udc_conflict(&layout, EXPECTED_UDC)
            .unwrap()
            .is_none());
    }

    #[test]
    fn setup_refuses_takeover_without_flag() {
        let tmp = tempfile::tempdir().unwrap();
        seed_conflict_root(tmp.path(), JIGGLER_GADGET_NAME, EXPECTED_UDC);
        let layout = SysfsLayout::for_test(tmp.path());
        let mut restore = None;
        match setup(
            &layout,
            SetupOptions {
                take_over_udc: false,
            },
            &mut restore,
        ) {
            SetupResult::Conflict(c) => assert_eq!(c.gadget_name, JIGGLER_GADGET_NAME),
            other => panic!("expected conflict, got {other:?}"),
        }
        assert!(restore.is_none());
        assert!(!layout.gadget_dir(GADGET_NAME).exists());
    }

    #[test]
    fn restore_state_recorded_when_takeover_succeeds_on_fake_fs() {
        let tmp = tempfile::tempdir().unwrap();
        seed_conflict_root(tmp.path(), JIGGLER_GADGET_NAME, EXPECTED_UDC);
        // Pre-create minimal previous gadget tree so restore can rebind.
        let prev = layout_gadget_path(tmp.path(), JIGGLER_GADGET_NAME);
        mkdir_all(&prev).unwrap();
        let layout = SysfsLayout::for_test(tmp.path());
        let mut restore = None;
        let result = setup(
            &layout,
            SetupOptions {
                take_over_udc: true,
            },
            &mut restore,
        );
        assert_eq!(result, SetupResult::Active);
        assert_eq!(
            restore,
            Some(RestoreState {
                previous_gadget: JIGGLER_GADGET_NAME.to_string(),
                stopped_systemd_unit: None,
            })
        );
        assert_eq!(
            read_to_string(&layout.gadget_udc_file(GADGET_NAME)).unwrap(),
            EXPECTED_UDC
        );
        assert!(read_to_string(&layout.gadget_udc_file(JIGGLER_GADGET_NAME))
            .unwrap()
            .is_empty());
        teardown(&layout, restore.as_ref()).unwrap();
        assert!(!layout.gadget_dir(GADGET_NAME).exists());
        assert_eq!(
            read_to_string(&layout.gadget_udc_file(JIGGLER_GADGET_NAME)).unwrap(),
            EXPECTED_UDC
        );
    }

    fn layout_gadget_path(root: &Path, name: &str) -> PathBuf {
        root.join("sys/kernel/config/usb_gadget").join(name)
    }

    #[test]
    fn systemd_unit_mapping_for_jiggler() {
        assert_eq!(
            systemd_unit_for_gadget(JIGGLER_GADGET_NAME),
            Some("jiggler-hid-gadget.service")
        );
        assert_eq!(systemd_unit_for_gadget("adb"), None);
    }

    #[test]
    fn take_over_from_config_parses_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("app.conf");
        fs::write(&conf, "# comment\nTAKE_OVER_UDC=1\n").unwrap();
        assert!(take_over_from_config(&conf));
        fs::write(&conf, "TAKE_OVER_UDC=0\n").unwrap();
        assert!(!take_over_from_config(&conf));
    }

    #[test]
    fn restore_state_roundtrip_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("restore");
        let state = RestoreState {
            previous_gadget: JIGGLER_GADGET_NAME.to_string(),
            stopped_systemd_unit: Some("jiggler-hid-gadget.service".to_string()),
        };
        persist_restore_state(&path, &state).unwrap();
        assert_eq!(load_restore_state(&path), Some(state));
    }
}
