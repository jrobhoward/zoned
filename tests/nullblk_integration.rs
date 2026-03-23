#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

//! Integration tests using the null_blk kernel module to create an emulated
//! zoned block device. These tests require:
//!
//! - Linux kernel >= 5.9 (for zone_capacity, zone_max_open, zone_max_active)
//! - Root privileges (for modprobe / configfs / device access)
//! - The null_blk kernel module available
//!
//! Tests are skipped automatically if these prerequisites are not met.
//!
//! Run with: `sudo cargo test --test nullblk_integration`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use zoned::{DeviceModel, ZoneCondition, ZoneType, ZonedDevice};

// --- Test device configuration ---

const DEVICE_SIZE_MB: u32 = 1024;
const ZONE_SIZE_MB: u32 = 64;
const ZONE_NR_CONV: u32 = 2;
const ZONE_MAX_OPEN: u32 = 4;
const ZONE_MAX_ACTIVE: u32 = 8;
const BLOCK_SIZE: u32 = 4096;

// 64 MB in 512-byte sectors
const ZONE_SIZE_SECTORS: u32 = ZONE_SIZE_MB * 1024 * 1024 / 512;

// Expected total zones: 1024 MB / 64 MB = 16
const EXPECTED_NR_ZONES: u32 = DEVICE_SIZE_MB / ZONE_SIZE_MB;

// --- null_blk configfs device manager ---

struct NullBlkDevice {
    name: String,
    configfs_path: PathBuf,
    dev_path: PathBuf,
}

impl NullBlkDevice {
    fn create(name: &str) -> Option<Self> {
        // Bail out if not root
        if !running_as_root() {
            eprintln!("SKIPPED: not running as root");
            return None;
        }

        // Ensure null_blk module is loaded with nr_devices=0
        if !ensure_null_blk_loaded() {
            eprintln!("SKIPPED: could not load null_blk module");
            return None;
        }

        let configfs_path = PathBuf::from(format!("/sys/kernel/config/nullb/{name}"));
        let dev_path = PathBuf::from(format!("/dev/{name}"));

        // Clean up any leftover device from a previous failed run
        if configfs_path.exists() {
            let _ = fs::write(configfs_path.join("power"), "0");
            let _ = fs::remove_dir(&configfs_path);
        }

        // Create the device directory in configfs
        if fs::create_dir(&configfs_path).is_err() {
            eprintln!("SKIPPED: could not create configfs directory");
            return None;
        }

        let device = Self {
            name: name.to_string(),
            configfs_path,
            dev_path,
        };

        // Configure the device
        device.write_attr("size", &DEVICE_SIZE_MB.to_string());
        device.write_attr("blocksize", &BLOCK_SIZE.to_string());
        device.write_attr("zoned", "1");
        device.write_attr("zone_size", &ZONE_SIZE_MB.to_string());
        device.write_attr("zone_nr_conv", &ZONE_NR_CONV.to_string());
        device.write_attr("zone_max_open", &ZONE_MAX_OPEN.to_string());
        device.write_attr("zone_max_active", &ZONE_MAX_ACTIVE.to_string());
        device.write_attr("memory_backed", "1");

        // Power on the device
        device.write_attr("power", "1");

        // Verify the device appeared
        if !device.dev_path.exists() {
            eprintln!(
                "SKIPPED: device {} did not appear after power-on",
                device.dev_path.display()
            );
            // Clean up
            let _ = fs::remove_dir(&device.configfs_path);
            return None;
        }

        Some(device)
    }

    fn write_attr(&self, attr: &str, value: &str) {
        let path = self.configfs_path.join(attr);
        fs::write(&path, value)
            .unwrap_or_else(|e| panic!("failed to write {value:?} to {}: {e}", path.display()));
    }

    fn path(&self) -> &Path {
        &self.dev_path
    }
}

impl Drop for NullBlkDevice {
    fn drop(&mut self) {
        // Power off and remove the configfs entry
        let _ = fs::write(self.configfs_path.join("power"), "0");
        let _ = fs::remove_dir(&self.configfs_path);
    }
}

fn running_as_root() -> bool {
    // SAFETY: getuid is a simple syscall with no safety concerns
    unsafe { libc::getuid() == 0 }
}

fn ensure_null_blk_loaded() -> bool {
    // Check if already loaded
    let loaded = fs::read_to_string("/proc/modules")
        .map(|m| m.contains("null_blk"))
        .unwrap_or(false);

    if loaded {
        return true;
    }

    // Try to load it
    Command::new("modprobe")
        .args(["null_blk", "nr_devices=0"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Helper macro: skip the test if the device cannot be created.
macro_rules! require_nullblk {
    ($name:expr) => {
        match NullBlkDevice::create($name) {
            Some(dev) => dev,
            None => {
                eprintln!("  test skipped (requires root + null_blk module)");
                return;
            }
        }
    };
}

// ============================================================
// Device info tests
// ============================================================

#[test]
fn device_info____nullblk____returns_correct_zone_size() {
    let nullblk = require_nullblk!("nullb_info_zs");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let info = dev.device_info().expect("device_info failed");
    assert_eq!(
        info.zone_size, ZONE_SIZE_SECTORS,
        "zone_size: expected {ZONE_SIZE_SECTORS}, got {}",
        info.zone_size
    );
}

#[test]
fn device_info____nullblk____returns_correct_zone_count() {
    let nullblk = require_nullblk!("nullb_info_nz");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let info = dev.device_info().expect("device_info failed");
    assert_eq!(
        info.nr_zones, EXPECTED_NR_ZONES,
        "nr_zones: expected {EXPECTED_NR_ZONES}, got {}",
        info.nr_zones
    );
}

// ============================================================
// Sysfs tests
// ============================================================

#[test]
fn sysfs____nullblk____reports_host_managed() {
    let nullblk = require_nullblk!("nullb_sysfs_model");

    let model = zoned::sysfs::device_model(nullblk.path()).expect("device_model failed");
    assert_eq!(model, DeviceModel::HostManaged);
}

#[test]
fn sysfs____nullblk____properties_match_config() {
    let nullblk = require_nullblk!("nullb_sysfs_props");

    let props = zoned::sysfs::device_properties(nullblk.path()).expect("device_properties failed");

    assert_eq!(props.model, DeviceModel::HostManaged);
    assert_eq!(props.chunk_sectors, ZONE_SIZE_SECTORS);
    assert_eq!(props.nr_zones, EXPECTED_NR_ZONES);
    assert_eq!(props.max_open_zones, ZONE_MAX_OPEN);
    assert_eq!(props.max_active_zones, ZONE_MAX_ACTIVE);
}

// ============================================================
// Report zones tests
// ============================================================

#[test]
fn report_zones____nullblk____first_zones_are_conventional() {
    let nullblk = require_nullblk!("nullb_rz_conv");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zones = dev
        .report_zones(0, ZONE_NR_CONV)
        .expect("report_zones failed");
    assert_eq!(zones.len(), ZONE_NR_CONV as usize);

    for (i, zone) in zones.iter().enumerate() {
        assert_eq!(
            zone.zone_type,
            ZoneType::Conventional,
            "zone {i} should be conventional, got {:?}",
            zone.zone_type
        );
        assert_eq!(
            zone.condition,
            ZoneCondition::NotWritePointer,
            "zone {i} condition should be NotWritePointer, got {:?}",
            zone.condition
        );
        assert_eq!(
            zone.start,
            i as u64 * ZONE_SIZE_SECTORS as u64,
            "zone {i} start sector mismatch"
        );
        assert_eq!(zone.len, ZONE_SIZE_SECTORS as u64);
    }
}

#[test]
fn report_zones____nullblk____sequential_zones_start_empty() {
    let nullblk = require_nullblk!("nullb_rz_seq");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    // Skip past conventional zones, read some sequential ones
    let seq_start_sector = ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS as u64;
    let zones = dev
        .report_zones(seq_start_sector, 4)
        .expect("report_zones failed");

    assert!(!zones.is_empty(), "expected at least 1 sequential zone");

    for (i, zone) in zones.iter().enumerate() {
        assert_eq!(
            zone.zone_type,
            ZoneType::SequentialWriteRequired,
            "seq zone {i} should be SequentialWriteRequired"
        );
        assert_eq!(
            zone.condition,
            ZoneCondition::Empty,
            "seq zone {i} should be Empty on a fresh device"
        );
        // Write pointer should be at zone start for empty zones
        assert_eq!(
            zone.write_pointer, zone.start,
            "seq zone {i} write pointer should equal start for empty zone"
        );
    }
}

#[test]
fn report_zones____nullblk____zone_start_sectors_are_contiguous() {
    let nullblk = require_nullblk!("nullb_rz_contig");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zones = dev
        .report_zones(0, EXPECTED_NR_ZONES)
        .expect("report_zones failed");
    assert_eq!(zones.len(), EXPECTED_NR_ZONES as usize);

    for (i, zone) in zones.iter().enumerate() {
        let expected_start = i as u64 * ZONE_SIZE_SECTORS as u64;
        assert_eq!(
            zone.start, expected_start,
            "zone {i} start: expected {expected_start}, got {}",
            zone.start
        );
    }
}

// ============================================================
// Report all zones tests
// ============================================================

#[test]
fn report_all_zones____nullblk____returns_all_zones() {
    let nullblk = require_nullblk!("nullb_raz_all");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zones = dev.report_all_zones(4).expect("report_all_zones failed");
    assert_eq!(
        zones.len(),
        EXPECTED_NR_ZONES as usize,
        "expected {EXPECTED_NR_ZONES} zones, got {}",
        zones.len()
    );
}

#[test]
fn report_all_zones____nullblk____batch_size_1_still_works() {
    let nullblk = require_nullblk!("nullb_raz_b1");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zones = dev
        .report_all_zones(1)
        .expect("report_all_zones with batch=1 failed");
    assert_eq!(zones.len(), EXPECTED_NR_ZONES as usize);
}

// ============================================================
// Zone lifecycle tests (open / close / finish / reset)
// ============================================================

#[test]
fn open_zones____nullblk____zone_becomes_explicitly_open() {
    let nullblk = require_nullblk!("nullb_open");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    // Target the first sequential zone
    let seq_start = ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS as u64;
    let zone_len = ZONE_SIZE_SECTORS as u64;

    dev.open_zones(seq_start, zone_len)
        .expect("open_zones failed");

    let zones = dev.report_zones(seq_start, 1).expect("report_zones failed");
    assert_eq!(zones.len(), 1);
    assert_eq!(
        zones[0].condition,
        ZoneCondition::ExplicitlyOpen,
        "zone should be ExplicitlyOpen after open_zones, got {:?}",
        zones[0].condition
    );
}

#[test]
fn close_zones____nullblk____open_zone_becomes_closed() {
    let nullblk = require_nullblk!("nullb_close");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let seq_start = ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS as u64;
    let zone_len = ZONE_SIZE_SECTORS as u64;

    // Open then close
    dev.open_zones(seq_start, zone_len)
        .expect("open_zones failed");
    dev.close_zones(seq_start, zone_len)
        .expect("close_zones failed");

    let zones = dev.report_zones(seq_start, 1).expect("report_zones failed");
    assert_eq!(zones.len(), 1);
    assert_eq!(
        zones[0].condition,
        ZoneCondition::Closed,
        "zone should be Closed after close_zones, got {:?}",
        zones[0].condition
    );
}

#[test]
fn finish_zones____nullblk____zone_becomes_full() {
    let nullblk = require_nullblk!("nullb_finish");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let seq_start = ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS as u64;
    let zone_len = ZONE_SIZE_SECTORS as u64;

    dev.finish_zones(seq_start, zone_len)
        .expect("finish_zones failed");

    let zones = dev.report_zones(seq_start, 1).expect("report_zones failed");
    assert_eq!(zones.len(), 1);
    assert_eq!(
        zones[0].condition,
        ZoneCondition::Full,
        "zone should be Full after finish_zones, got {:?}",
        zones[0].condition
    );
}

#[test]
fn reset_zones____nullblk____full_zone_becomes_empty() {
    let nullblk = require_nullblk!("nullb_reset");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let seq_start = ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS as u64;
    let zone_len = ZONE_SIZE_SECTORS as u64;

    // Finish then reset
    dev.finish_zones(seq_start, zone_len)
        .expect("finish_zones failed");
    dev.reset_zones(seq_start, zone_len)
        .expect("reset_zones failed");

    let zones = dev.report_zones(seq_start, 1).expect("report_zones failed");
    assert_eq!(zones.len(), 1);
    assert_eq!(
        zones[0].condition,
        ZoneCondition::Empty,
        "zone should be Empty after reset, got {:?}",
        zones[0].condition
    );
    assert_eq!(
        zones[0].write_pointer, zones[0].start,
        "write pointer should be at zone start after reset"
    );
}

#[test]
fn finish_zones____nullblk____write_pointer_advances_to_end() {
    let nullblk = require_nullblk!("nullb_finish_wp");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let seq_start = ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS as u64;
    let zone_len = ZONE_SIZE_SECTORS as u64;

    dev.finish_zones(seq_start, zone_len)
        .expect("finish_zones failed");

    let zones = dev.report_zones(seq_start, 1).expect("report_zones failed");
    assert_eq!(
        zones[0].write_pointer,
        zones[0].start + zones[0].len,
        "write pointer should be at zone end after finish"
    );
}

// ============================================================
// Full lifecycle: open -> close -> open -> finish -> reset
// ============================================================

#[test]
fn zone_lifecycle____nullblk____full_state_machine() {
    let nullblk = require_nullblk!("nullb_lifecycle");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let seq_start = ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS as u64;
    let zone_len = ZONE_SIZE_SECTORS as u64;

    // Start: Empty
    let zones = dev.report_zones(seq_start, 1).expect("report failed");
    assert_eq!(
        zones[0].condition,
        ZoneCondition::Empty,
        "initial state should be Empty"
    );

    // Open
    dev.open_zones(seq_start, zone_len).expect("open failed");
    let zones = dev.report_zones(seq_start, 1).expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::ExplicitlyOpen);

    // Close
    dev.close_zones(seq_start, zone_len).expect("close failed");
    let zones = dev.report_zones(seq_start, 1).expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::Closed);

    // Re-open
    dev.open_zones(seq_start, zone_len).expect("re-open failed");
    let zones = dev.report_zones(seq_start, 1).expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::ExplicitlyOpen);

    // Finish
    dev.finish_zones(seq_start, zone_len)
        .expect("finish failed");
    let zones = dev.report_zones(seq_start, 1).expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::Full);

    // Reset
    dev.reset_zones(seq_start, zone_len).expect("reset failed");
    let zones = dev.report_zones(seq_start, 1).expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::Empty);
}

// ============================================================
// Multiple zone operations
// ============================================================

#[test]
fn open_zones____nullblk____multiple_zones_simultaneously() {
    let nullblk = require_nullblk!("nullb_multi_open");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zone_len = ZONE_SIZE_SECTORS as u64;

    // Open 3 sequential zones individually
    for i in 0..3u64 {
        let sector = (ZONE_NR_CONV as u64 + i) * zone_len;
        dev.open_zones(sector, zone_len)
            .unwrap_or_else(|e| panic!("open_zones failed for zone {i}: {e}"));
    }

    // Verify all three are open
    let seq_start = ZONE_NR_CONV as u64 * zone_len;
    let zones = dev.report_zones(seq_start, 3).expect("report_zones failed");
    assert_eq!(zones.len(), 3);

    for (i, zone) in zones.iter().enumerate() {
        assert_eq!(
            zone.condition,
            ZoneCondition::ExplicitlyOpen,
            "zone {i} should be ExplicitlyOpen"
        );
    }
}

#[test]
fn reset_zones____nullblk____reset_multiple_finished_zones() {
    let nullblk = require_nullblk!("nullb_multi_reset");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zone_len = ZONE_SIZE_SECTORS as u64;
    let seq_start = ZONE_NR_CONV as u64 * zone_len;

    // Finish 3 zones
    for i in 0..3u64 {
        let sector = seq_start + i * zone_len;
        dev.finish_zones(sector, zone_len).expect("finish failed");
    }

    // Reset all 3 at once with a range covering all three
    dev.reset_zones(seq_start, zone_len * 3)
        .expect("reset failed");

    let zones = dev.report_zones(seq_start, 3).expect("report failed");
    for (i, zone) in zones.iter().enumerate() {
        assert_eq!(
            zone.condition,
            ZoneCondition::Empty,
            "zone {i} should be Empty after bulk reset"
        );
    }
}

// ============================================================
// Debug formatting
// ============================================================

#[test]
fn debug____nullblk____includes_device_path() {
    let nullblk = require_nullblk!("nullb_debug");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let debug_str = format!("{dev:?}");
    assert!(
        debug_str.contains(&nullblk.name),
        "debug output should contain device name, got: {debug_str}"
    );
}
