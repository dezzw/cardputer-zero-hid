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

/// ADB composite gadget on Cardputer Zero ([CardputerZero/launcher `cardputer-adb-gadget`](https://github.com/CardputerZero/launcher)).
pub const ADB_GADGET_NAME: &str = "g1";

/// Boot-time peers that may own the slave UDC (verify on device image).
///
/// - `jiggler-hid-gadget.service` — [intGus/jiggler](https://github.com/intGus/jiggler/blob/a59e756/tools/jiggler-hid-gadget.service)
/// - `adbd.service`, `cardputer-adb-hotplug.service` — [CardputerZero/launcher `cardputer-adb`](https://github.com/CardputerZero/launcher/blob/master/projects/APPLaunch/APPLaunch/adb/cardputer-adb)
pub const PEER_UDC_SYSTEMD_UNITS: &[&str] = &[
    "jiggler-hid-gadget.service",
    "adbd.service",
    "cardputer-adb-hotplug.service",
];

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

/// How the UDC is occupied (configfs composite vs legacy `g_*` / platform driver).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSource {
    Configfs,
    /// `/sys/class/udc/<udc>/function` — kernel ABI sysfs-class-udc.
    SysfsDriver,
}

/// Another consumer currently using [`EXPECTED_UDC`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdcConflict {
    pub udc: String,
    /// configfs gadget directory name, or sysfs `function` driver name.
    pub occupant: String,
    pub source: ConflictSource,
}

impl UdcConflict {
    pub fn gadget_name(&self) -> &str {
        &self.occupant
    }
}

/// Shell exit code when setup did not bind a gadget (systemd should not stay "active").
pub const EXIT_SETUP_NOT_BOUND: i32 = 3;

/// Shell exit code when the composite gadget is bound successfully.
pub const EXIT_SETUP_BOUND: i32 = 0;

/// State saved when [`SetupOptions::take_over_udc`] is used, for teardown restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreState {
    pub previous_gadget: String,
    /// systemd units stopped during takeover (all of them are started again on restore).
    pub stopped_systemd_units: Vec<String>,
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
    /// UDC is free, but a peer gadget unit is enabled; do not bind without takeover.
    YieldPeers(Vec<String>),
    Error(String),
}

/// Parsed intent from `cardputer-zero-hid-gadget` argv (`args[0]` = program name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonAction {
    Setup,
    Remove,
    Help,
}

/// Map argv to a daemon action. **No extra args** means setup (systemd `ExecStart`).
pub fn parse_daemon_action(args: &[&str]) -> DaemonAction {
    assert!(!args.is_empty(), "argv must include program name");
    if args.len() == 1 {
        return DaemonAction::Setup;
    }
    for a in &args[1..] {
        match *a {
            "--remove" | "--teardown" => return DaemonAction::Remove,
            "--help" | "-h" => return DaemonAction::Help,
            _ => {}
        }
    }
    DaemonAction::Setup
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

fn udc_sysfs_path(layout: &SysfsLayout, udc: &str, attr: &str) -> PathBuf {
    layout.udc_class.join(udc).join(attr)
}

/// Read `/sys/class/udc/<udc>/function` (name of the running USB gadget driver).
pub fn read_udc_sysfs_function(layout: &SysfsLayout, udc: &str) -> Result<Option<String>, String> {
    let path = udc_sysfs_path(layout, udc, "function");
    if !path.is_file() {
        return Ok(None);
    }
    let value = read_to_string(&path)?;
    if value.is_empty() || value.eq_ignore_ascii_case("(none)") {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

/// Read `/sys/class/udc/<udc>/state` (USB device state string; see sysfs-class-udc ABI).
pub fn read_udc_sysfs_state(layout: &SysfsLayout, udc: &str) -> Result<Option<String>, String> {
    let path = udc_sysfs_path(layout, udc, "state");
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(read_to_string(&path)?))
}

/// Returns `true` when the kernel reports a bound gadget driver via sysfs `function`
/// but configfs shows no composite bind (legacy `g_*` / platform drivers).
pub fn legacy_udc_driver_is_active(
    layout: &SysfsLayout,
    udc: &str,
    configfs_bound: &Option<String>,
) -> Result<Option<String>, String> {
    let driver = read_udc_sysfs_function(layout, udc)?;
    let Some(driver) = driver else {
        return Ok(None);
    };
    match configfs_bound {
        None => Ok(Some(driver)),
        Some(name) if name == GADGET_NAME => Ok(None),
        Some(_) => Ok(None),
    }
}

/// Detect whether `udc` is in use by something other than [`GADGET_NAME`].
pub fn detect_udc_conflict(layout: &SysfsLayout, udc: &str) -> Result<Option<UdcConflict>, String> {
    let bound = find_gadget_bound_to_udc(layout, udc)?;
    if let Some(name) = bound {
        if name == GADGET_NAME {
            return Ok(None);
        }
        return Ok(Some(UdcConflict {
            udc: udc.to_string(),
            occupant: name,
            source: ConflictSource::Configfs,
        }));
    }
    if let Some(driver) = legacy_udc_driver_is_active(layout, udc, &None)? {
        return Ok(Some(UdcConflict {
            udc: udc.to_string(),
            occupant: driver,
            source: ConflictSource::SysfsDriver,
        }));
    }
    Ok(None)
}

fn systemd_unit_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Primary systemd unit to restart when releasing a well-known configfs gadget.
pub fn systemd_unit_for_gadget(gadget_name: &str) -> Option<&'static str> {
    match gadget_name {
        JIGGLER_GADGET_NAME => Some("jiggler-hid-gadget.service"),
        ADB_GADGET_NAME => Some("adbd.service"),
        _ => None,
    }
}

/// Units associated with a gadget for stop/restore (primary unit first).
pub fn systemd_units_for_gadget(gadget_name: &str) -> &'static [&'static str] {
    match gadget_name {
        JIGGLER_GADGET_NAME => &["jiggler-hid-gadget.service"],
        ADB_GADGET_NAME => &["adbd.service", "cardputer-adb-hotplug.service"],
        _ => &[],
    }
}

/// Stop peer systemd units for a configfs gadget name; returns every unit actually stopped.
pub fn stop_conflicting_units_with(
    gadget_name: &str,
    is_active: impl Fn(&str) -> bool,
) -> Vec<String> {
    let units = systemd_units_for_gadget(gadget_name);
    let mut stopped = Vec::new();
    for unit in units {
        if is_active(unit) {
            let _ = Command::new("systemctl").args(["stop", unit]).status();
            stopped.push(unit.to_string());
        }
    }
    stopped
}

fn stop_conflicting_unit(gadget_name: &str) -> Vec<String> {
    stop_conflicting_units_with(gadget_name, systemd_unit_active)
}

fn start_systemd_unit(unit: &str) {
    let _ = Command::new("systemctl").args(["start", unit]).status();
}

fn systemd_unit_is_enabled(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-enabled", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Peer gadget units currently enabled (may be mocked in tests).
pub fn enabled_peer_gadget_units_with(check: impl Fn(&str) -> bool) -> Vec<String> {
    PEER_UDC_SYSTEMD_UNITS
        .iter()
        .filter(|u| check(u))
        .map(|u| (*u).to_string())
        .collect()
}

pub fn enabled_peer_gadget_units() -> Vec<String> {
    enabled_peer_gadget_units_with(systemd_unit_is_enabled)
}

/// When the UDC is free, refuse to bind if a peer gadget unit stays enabled.
pub fn peers_to_yield_to(enabled_peers: &[String]) -> Option<Vec<String>> {
    if enabled_peers.is_empty() {
        None
    } else {
        Some(enabled_peers.to_vec())
    }
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
    let units = state.stopped_systemd_units.join(",");
    let body = format!(
        "took_over=1\nprevious_gadget={}\nstopped_units={units}\n",
        state.previous_gadget
    );
    write_str(path, &body)
}

/// Load persisted takeover state (if any). Ignores files without `took_over=1`.
pub fn load_restore_state(path: &Path) -> Option<RestoreState> {
    let content = fs::read_to_string(path).ok()?;
    let mut took_over = false;
    let mut gadget = None;
    let mut units: Vec<String> = Vec::new();
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("took_over=") {
            took_over = take_over_value_is_yes(v.trim());
        } else if let Some(v) = line.strip_prefix("previous_gadget=") {
            gadget = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("stopped_units=") {
            units = v
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        } else if let Some(v) = line.strip_prefix("stopped_unit=") {
            let v = v.trim();
            if !v.is_empty() {
                units.push(v.to_string());
            }
        }
    }
    if !took_over {
        return None;
    }
    let previous_gadget = gadget?;
    Some(RestoreState {
        previous_gadget,
        stopped_systemd_units: units,
    })
}

/// Whether a persisted takeover record still matches sysfs and is safe to apply on `--remove`.
pub fn restore_state_is_applicable(layout: &SysfsLayout, state: &RestoreState) -> bool {
    if legacy_udc_driver_is_active(layout, EXPECTED_UDC, &None)
        .ok()
        .flatten()
        .is_some()
    {
        return false;
    }
    match find_gadget_bound_to_udc(layout, EXPECTED_UDC)
        .ok()
        .flatten()
        .as_deref()
    {
        Some(GADGET_NAME) => return false,
        Some(other) => {
            if other != state.previous_gadget {
                return false;
            }
            if read_to_string(&layout.gadget_udc_file(other))
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                return false;
            }
        }
        None => {}
    }
    layout.gadget_dir(&state.previous_gadget).is_dir() || !state.stopped_systemd_units.is_empty()
}

pub fn clear_restore_state(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Remove a configfs gadget tree per kernel configfs rules: unbind, drop function
/// symlinks only, then bottom-up `rmdir` (never unlink attribute files).
pub fn remove_configfs_gadget_tree(layout: &SysfsLayout, gadget_name: &str) -> Result<(), String> {
    let g = layout.gadget_dir(gadget_name);
    if !g.is_dir() {
        return Ok(());
    }

    let _ = unbind_gadget(layout, gadget_name);

    let c1 = g.join("configs/c.1");
    if c1.is_dir() {
        for entry in fs::read_dir(&c1).map_err(|e| format!("read_dir {}: {e}", c1.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_symlink() {
                remove_configfs_symlink(&path)?;
            }
        }
        rmdir_configfs_only(&c1.join("strings/0x409"))?;
        rmdir_configfs_only(&c1)?;
        rmdir_configfs_only(&g.join("configs"))?;
    }

    rmdir_configfs_only(&g.join("functions/hid.usb0"))?;
    rmdir_configfs_only(&g.join("functions/hid.usb1"))?;
    rmdir_configfs_only(&g.join("functions"))?;
    rmdir_configfs_only(&g.join("strings/0x409"))?;
    rmdir_configfs_only(&g.join("strings"))?;
    rmdir_configfs_only(&g)?;
    Ok(())
}

/// Unlink only a config symlink (never attribute regular files).
fn remove_configfs_symlink(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
    if !meta.file_type().is_symlink() {
        return Err(format!(
            "refusing to unlink non-symlink configfs entry {}",
            path.display()
        ));
    }
    teardown_trace_unlink(path);
    fs::remove_file(path).map_err(|e| format!("remove symlink {}: {e}", path.display()))
}

/// `rmdir` a configfs directory; the kernel removes dangling attributes with the directory.
fn rmdir_configfs_only(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Ok(());
    }
    teardown_check_forced_rmdir_fail(path)?;
    fs::remove_dir(path).map_err(|e| format!("rmdir {}: {e}", path.display()))
}

#[cfg(test)]
thread_local! {
    static TEARDOWN_UNLINK_TRACE: std::cell::RefCell<Vec<PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static TEARDOWN_FAIL_RMDIR: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn teardown_test_clear_hooks() {
    TEARDOWN_UNLINK_TRACE.with(|t| t.borrow_mut().clear());
    TEARDOWN_FAIL_RMDIR.with(|f| *f.borrow_mut() = None);
}

#[cfg(test)]
pub(crate) fn teardown_test_set_fail_rmdir(path: PathBuf) {
    TEARDOWN_FAIL_RMDIR.with(|f| *f.borrow_mut() = Some(path));
}

#[cfg(test)]
pub(crate) fn teardown_test_unlink_trace() -> Vec<PathBuf> {
    TEARDOWN_UNLINK_TRACE.with(|t| t.borrow().clone())
}

fn teardown_trace_unlink(path: &Path) {
    #[cfg(test)]
    TEARDOWN_UNLINK_TRACE.with(|t| t.borrow_mut().push(path.to_path_buf()));
    let _ = path;
}

fn teardown_check_forced_rmdir_fail(path: &Path) -> Result<(), String> {
    #[cfg(test)]
    {
        if TEARDOWN_FAIL_RMDIR.with(|f| f.borrow().as_ref() == Some(&path.to_path_buf())) {
            return Err(format!("teardown_test_forced_rmdir: {}", path.display()));
        }
    }
    #[cfg(not(test))]
    let _ = path;
    Ok(())
}

/// Remove our configfs gadget tree (idempotent). Always attempts restore when applicable,
/// even if tree removal fails partway (e.g. on real configfs EPERM).
pub fn teardown(layout: &SysfsLayout, restore: Option<&RestoreState>) -> Result<(), String> {
    let g = layout.gadget_dir(GADGET_NAME);
    let tree_result = if g.is_dir() {
        remove_configfs_gadget_tree(layout, GADGET_NAME)
    } else {
        Ok(())
    };

    if let Some(state) = restore.filter(|s| restore_state_is_applicable(layout, s)) {
        let _ = restore_previous(layout, state);
    }

    tree_result
}

fn units_to_start_on_restore(state: &RestoreState) -> Vec<String> {
    if !state.stopped_systemd_units.is_empty() {
        return state.stopped_systemd_units.clone();
    }
    systemd_units_for_gadget(&state.previous_gadget)
        .iter()
        .map(|u| (*u).to_string())
        .collect()
}

fn restore_previous(layout: &SysfsLayout, state: &RestoreState) -> Result<(), String> {
    let prev_gadget_dir = layout.gadget_dir(&state.previous_gadget);
    if prev_gadget_dir.is_dir() {
        bind_gadget(layout, &state.previous_gadget, EXPECTED_UDC)?;
    }
    for unit in units_to_start_on_restore(state) {
        start_systemd_unit(&unit);
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
    if find_gadget_bound_to_udc(layout, EXPECTED_UDC)
        .ok()
        .flatten()
        .as_deref()
        == Some(GADGET_NAME)
    {
        return SetupResult::Active;
    }

    let conflict = match detect_udc_conflict(layout, EXPECTED_UDC) {
        Ok(c) => c,
        Err(e) => return SetupResult::Error(e),
    };

    if let Some(ref c) = conflict {
        if !options.take_over_udc {
            return SetupResult::Conflict(c.clone());
        }
        if c.source == ConflictSource::SysfsDriver {
            return SetupResult::Error(format!(
                "UDC {} busy with legacy gadget driver `{}` (sysfs function); \
                 will not bind configfs gadget",
                c.udc, c.occupant
            ));
        }
    } else if !options.take_over_udc {
        if let Some(peers) = peers_to_yield_to(&enabled_peer_gadget_units()) {
            return SetupResult::YieldPeers(peers);
        }
    }

    modprobe_libcomposite();

    if let Some(c) = conflict {
        let stopped = stop_conflicting_unit(&c.occupant);
        if let Err(e) = unbind_gadget(layout, &c.occupant) {
            return SetupResult::Error(e);
        }
        *restore_out = Some(RestoreState {
            previous_gadget: c.occupant,
            stopped_systemd_units: stopped,
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

/// Values treated as explicit user confirmation for UDC takeover (env + conffile).
pub fn take_over_value_is_yes(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "yes" | "true" | "on"
    )
}

/// Explicit denial for env override (`CARDPUTER_ZERO_HID_TAKE_OVER_UDC=0`, etc.).
pub fn take_over_value_is_no(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "no" | "false" | "off"
    )
}

/// Parse `TAKE_OVER_UDC=` from conffile content.
pub fn take_over_from_config_content(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, val)) = line.split_once('=') else {
            return false;
        };
        key.trim().eq_ignore_ascii_case("TAKE_OVER_UDC") && take_over_value_is_yes(val)
    })
}

/// Parse `TAKE_OVER_UDC=` from the app conffile path.
pub fn take_over_from_config(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    take_over_from_config_content(&content)
}

/// CLI / environment / conffile takeover confirmation.
pub fn take_over_confirmed(args: &[&str]) -> bool {
    take_over_confirmed_with(
        args,
        std::env::var("CARDPUTER_ZERO_HID_TAKE_OVER_UDC")
            .ok()
            .as_deref(),
        take_over_from_config(Path::new("/etc/cardputer-zero-hid-app.conf")),
    )
}

/// Testable takeover gate (`env` = `CARDPUTER_ZERO_HID_TAKE_OVER_UDC` value if set).
pub fn take_over_confirmed_with(args: &[&str], env: Option<&str>, config_yes: bool) -> bool {
    if args.contains(&"--take-over-udc") {
        return true;
    }
    if let Some(val) = env {
        // Env is set: only explicit yes values enable takeover; 0/empty/garbage deny
        // and do not fall through to the conffile.
        return take_over_value_is_yes(val);
    }
    config_yes
}

/// Map a setup result to an `ExecStart` exit status for systemd (`RemainAfterExit`).
pub fn exit_code_for_setup(result: &SetupResult) -> i32 {
    match result {
        SetupResult::Active => EXIT_SETUP_BOUND,
        SetupResult::Conflict(_) | SetupResult::YieldPeers(_) => EXIT_SETUP_NOT_BOUND,
        SetupResult::Error(_) => 1,
    }
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
        assert_eq!(c.occupant, JIGGLER_GADGET_NAME);
        assert_eq!(c.source, ConflictSource::Configfs);
    }

    #[test]
    fn detect_legacy_driver_via_sysfs_function() {
        let tmp = tempfile::tempdir().unwrap();
        let func = tmp
            .path()
            .join("sys/class/udc")
            .join(EXPECTED_UDC)
            .join("function");
        mkdir_all(func.parent().unwrap()).unwrap();
        write_str(&func, "g_ffs").unwrap();
        let layout = SysfsLayout::for_test(tmp.path());
        let c = detect_udc_conflict(&layout, EXPECTED_UDC).unwrap().unwrap();
        assert_eq!(c.source, ConflictSource::SysfsDriver);
        assert_eq!(c.occupant, "g_ffs");
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
            SetupResult::Conflict(c) => assert_eq!(c.occupant, JIGGLER_GADGET_NAME),
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
                stopped_systemd_units: vec![],
            })
        );
        assert_eq!(
            read_to_string(&layout.gadget_udc_file(GADGET_NAME)).unwrap(),
            EXPECTED_UDC
        );
        assert!(read_to_string(&layout.gadget_udc_file(JIGGLER_GADGET_NAME))
            .unwrap()
            .is_empty());
        teardown(&layout, restore.as_ref()).ok();
        assert_eq!(
            read_to_string(&layout.gadget_udc_file(JIGGLER_GADGET_NAME)).unwrap(),
            EXPECTED_UDC
        );
    }

    fn layout_gadget_path(root: &Path, name: &str) -> PathBuf {
        root.join("sys/kernel/config/usb_gadget").join(name)
    }

    #[test]
    fn systemd_unit_mapping_for_known_gadgets() {
        assert_eq!(
            systemd_unit_for_gadget(JIGGLER_GADGET_NAME),
            Some("jiggler-hid-gadget.service")
        );
        assert_eq!(
            systemd_unit_for_gadget(ADB_GADGET_NAME),
            Some("adbd.service")
        );
        assert_eq!(
            systemd_units_for_gadget(ADB_GADGET_NAME),
            &["adbd.service", "cardputer-adb-hotplug.service",][..]
        );
        assert_eq!(systemd_unit_for_gadget("unknown"), None);
    }

    #[test]
    fn take_over_from_config_parses_explicit_yes_only() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("app.conf");
        fs::write(&conf, "# comment\nTAKE_OVER_UDC=1\n").unwrap();
        assert!(take_over_from_config(&conf));
        fs::write(&conf, "TAKE_OVER_UDC=yes\n").unwrap();
        assert!(take_over_from_config(&conf));
        fs::write(&conf, "TAKE_OVER_UDC=0\n").unwrap();
        assert!(!take_over_from_config(&conf));
        assert!(!take_over_from_config_content("TAKE_OVER_UDC=0\n"));
        assert!(!take_over_from_config_content("TAKE_OVER_UDC=no\n"));
    }

    #[test]
    fn take_over_confirmed_respects_env_values() {
        assert!(!take_over_confirmed_with(&[], None, false));
        assert!(!take_over_confirmed_with(&[], Some("0"), false));
        assert!(!take_over_confirmed_with(&[], Some(""), false));
        assert!(!take_over_confirmed_with(&[], Some("maybe"), false));
        assert!(!take_over_confirmed_with(&[], Some("0"), true));
        assert!(!take_over_confirmed_with(&[], Some(""), true));
        assert!(!take_over_confirmed_with(&[], Some("garbage"), true));
        assert!(take_over_confirmed_with(&[], Some("1"), false));
        assert!(take_over_confirmed_with(&[], Some("yes"), false));
        assert!(take_over_confirmed_with(&[], Some("true"), false));
        assert!(take_over_confirmed_with(&[], Some("on"), false));
        assert!(take_over_confirmed_with(
            &["--take-over-udc"],
            Some("0"),
            false
        ));
    }

    #[test]
    fn parse_daemon_action_no_args_is_setup() {
        assert_eq!(
            parse_daemon_action(&["cardputer-zero-hid-gadget"]),
            DaemonAction::Setup
        );
    }

    #[test]
    fn peers_to_yield_to_matches_enabled_list() {
        assert!(peers_to_yield_to(&[]).is_none());
        let list = vec!["jiggler-hid-gadget.service".to_string()];
        assert_eq!(peers_to_yield_to(&list), Some(list));
    }

    #[test]
    fn enabled_peer_units_filter() {
        let all = enabled_peer_gadget_units_with(|_| true);
        assert_eq!(all.len(), PEER_UDC_SYSTEMD_UNITS.len());
        let none = enabled_peer_gadget_units_with(|_| false);
        assert!(none.is_empty());
    }

    #[test]
    fn restore_state_roundtrip_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("restore");
        let state = RestoreState {
            previous_gadget: JIGGLER_GADGET_NAME.to_string(),
            stopped_systemd_units: vec![
                "jiggler-hid-gadget.service".to_string(),
                "cardputer-adb-hotplug.service".to_string(),
            ],
        };
        persist_restore_state(&path, &state).unwrap();
        assert_eq!(load_restore_state(&path), Some(state));
        fs::write(&path, "previous_gadget=cp0hid\n").unwrap();
        assert!(load_restore_state(&path).is_none());
    }

    #[test]
    fn restore_not_applied_when_udc_owned_by_other_gadget() {
        let tmp = tempfile::tempdir().unwrap();
        seed_conflict_root(tmp.path(), "g1", EXPECTED_UDC);
        let layout = SysfsLayout::for_test(tmp.path());
        let state = RestoreState {
            previous_gadget: JIGGLER_GADGET_NAME.to_string(),
            stopped_systemd_units: vec![],
        };
        assert!(!restore_state_is_applicable(&layout, &state));
        teardown(&layout, Some(&state)).unwrap();
        assert_eq!(
            read_to_string(&layout.gadget_udc_file("g1")).unwrap(),
            EXPECTED_UDC
        );
    }

    #[test]
    fn exit_code_for_setup_not_bound_on_conflict_or_yield() {
        let conflict = UdcConflict {
            udc: EXPECTED_UDC.to_string(),
            occupant: "cp0hid".into(),
            source: ConflictSource::Configfs,
        };
        assert_eq!(
            exit_code_for_setup(&SetupResult::Conflict(conflict)),
            EXIT_SETUP_NOT_BOUND
        );
        assert_eq!(
            exit_code_for_setup(&SetupResult::YieldPeers(vec![])),
            EXIT_SETUP_NOT_BOUND
        );
        assert_eq!(exit_code_for_setup(&SetupResult::Active), EXIT_SETUP_BOUND);
    }

    fn seed_full_gadget_tree(root: &Path, gadget: &str, bind: bool) {
        let g = root.join("sys/kernel/config/usb_gadget").join(gadget);
        mkdir_all(&g.join("functions/hid.usb0")).unwrap();
        write_str(&g.join("functions/hid.usb0/protocol"), "1").unwrap();
        mkdir_all(&g.join("functions/hid.usb1")).unwrap();
        mkdir_all(&g.join("strings/0x409")).unwrap();
        write_str(&g.join("idVendor"), "0x1d6b").unwrap();
        mkdir_all(&g.join("configs/c.1/strings/0x409")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                g.join("functions/hid.usb0"),
                g.join("configs/c.1/hid.usb0"),
            )
            .unwrap();
        }
        if bind {
            write_str(&g.join("UDC"), EXPECTED_UDC).unwrap();
        }
    }

    #[test]
    fn remove_configfs_gadget_tree_never_unlinks_attributes() {
        let tmp = tempfile::tempdir().unwrap();
        seed_full_gadget_tree(tmp.path(), GADGET_NAME, true);
        let layout = SysfsLayout::for_test(tmp.path());
        teardown_test_clear_hooks();
        let _ = remove_configfs_gadget_tree(&layout, GADGET_NAME);
        let unlinks = teardown_test_unlink_trace();
        assert!(
            unlinks.iter().all(|p| {
                p.to_string_lossy().contains("configs/c.1")
                    && (p.ends_with("hid.usb0") || p.ends_with("hid.usb1"))
            }),
            "only config function symlinks should be unlinked: {unlinks:?}"
        );
        assert!(
            !unlinks
                .iter()
                .any(|p| p.to_string_lossy().contains("protocol")),
            "must not unlink attribute files"
        );
    }

    #[test]
    fn teardown_restores_previous_even_when_tree_removal_fails() {
        let tmp = tempfile::tempdir().unwrap();
        seed_full_gadget_tree(tmp.path(), GADGET_NAME, true);
        seed_conflict_root(tmp.path(), JIGGLER_GADGET_NAME, "");
        let layout = SysfsLayout::for_test(tmp.path());
        let state = RestoreState {
            previous_gadget: JIGGLER_GADGET_NAME.to_string(),
            stopped_systemd_units: vec![],
        };
        teardown_test_clear_hooks();
        teardown_test_set_fail_rmdir(layout.gadget_dir(GADGET_NAME));
        assert!(teardown(&layout, Some(&state)).is_err());
        assert_eq!(
            read_to_string(&layout.gadget_udc_file(JIGGLER_GADGET_NAME)).unwrap(),
            EXPECTED_UDC
        );
    }

    #[test]
    fn stop_conflicting_units_stops_every_mapped_service() {
        let stopped = stop_conflicting_units_with(ADB_GADGET_NAME, |_| true);
        assert_eq!(
            stopped,
            vec![
                "adbd.service".to_string(),
                "cardputer-adb-hotplug.service".to_string(),
            ]
        );
    }

    #[test]
    fn units_to_start_on_restore_uses_recorded_stops() {
        let state = RestoreState {
            previous_gadget: ADB_GADGET_NAME.to_string(),
            stopped_systemd_units: vec![
                "adbd.service".to_string(),
                "cardputer-adb-hotplug.service".to_string(),
            ],
        };
        assert_eq!(
            super::units_to_start_on_restore(&state),
            state.stopped_systemd_units
        );
    }

    #[test]
    fn setup_is_idempotent_when_own_gadget_holds_udc() {
        let tmp = tempfile::tempdir().unwrap();
        seed_full_gadget_tree(tmp.path(), GADGET_NAME, true);
        persist_restore_state(
            &tmp.path().join("restore"),
            &RestoreState {
                previous_gadget: JIGGLER_GADGET_NAME.to_string(),
                stopped_systemd_units: vec!["jiggler-hid-gadget.service".to_string()],
            },
        )
        .unwrap();
        let layout = SysfsLayout::for_test(tmp.path());
        let mut restore = None;
        let result = setup(
            &layout,
            SetupOptions {
                take_over_udc: false,
            },
            &mut restore,
        );
        assert_eq!(result, SetupResult::Active);
        assert!(restore.is_none());
        assert_eq!(
            read_to_string(&layout.gadget_udc_file(GADGET_NAME)).unwrap(),
            EXPECTED_UDC
        );
    }

    #[test]
    fn setup_returns_conflict_before_creating_gadget_on_legacy_driver() {
        let tmp = tempfile::tempdir().unwrap();
        let func = tmp
            .path()
            .join("sys/class/udc")
            .join(EXPECTED_UDC)
            .join("function");
        mkdir_all(func.parent().unwrap()).unwrap();
        write_str(&func, "android_usb").unwrap();
        let layout = SysfsLayout::for_test(tmp.path());
        let mut restore = None;
        let result = setup(
            &layout,
            SetupOptions {
                take_over_udc: false,
            },
            &mut restore,
        );
        assert!(matches!(result, SetupResult::Conflict(_)));
        assert!(!layout.gadget_dir(GADGET_NAME).exists());
    }
}
