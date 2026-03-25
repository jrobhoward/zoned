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

use std::sync::Arc;
use std::thread;

use zoned::{
    DeviceModel, Sector, ZoneAllocator, ZoneCondition, ZoneHandle, ZoneIndex, ZoneType, ZonedDevice,
};

// --- Test device configuration ---

const DEVICE_SIZE_MB: u32 = 1024;
const ZONE_SIZE_MB: u32 = 64;
const ZONE_NR_CONV: u32 = 2;
const ZONE_MAX_OPEN: u32 = 4;
const ZONE_MAX_ACTIVE: u32 = 8;
const BLOCK_SIZE: u32 = 4096;

// 64 MB in 512-byte sectors
const ZONE_SIZE_SECTORS: u64 = ZONE_SIZE_MB as u64 * 1024 * 1024 / 512;

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
        info.zone_size,
        Sector(ZONE_SIZE_SECTORS),
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
    assert_eq!(props.chunk_sectors, Sector(ZONE_SIZE_SECTORS));
    assert_eq!(props.nr_zones, EXPECTED_NR_ZONES);
    assert_eq!(props.max_open_zones, Some(ZONE_MAX_OPEN));
    assert_eq!(props.max_active_zones, Some(ZONE_MAX_ACTIVE));
}

// ============================================================
// Report zones tests
// ============================================================

#[test]
fn report_zones____nullblk____first_zones_are_conventional() {
    let nullblk = require_nullblk!("nullb_rz_conv");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zones = dev
        .report_zones(Sector::ZERO, ZONE_NR_CONV)
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
            Sector(i as u64 * ZONE_SIZE_SECTORS),
            "zone {i} start sector mismatch"
        );
        assert_eq!(zone.len, Sector(ZONE_SIZE_SECTORS));
    }
}

#[test]
fn report_zones____nullblk____sequential_zones_start_empty() {
    let nullblk = require_nullblk!("nullb_rz_seq");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    // Skip past conventional zones, read some sequential ones
    let seq_start_sector = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
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
            zone.write_pointer,
            Some(zone.start),
            "seq zone {i} write pointer should equal start for empty zone"
        );
    }
}

#[test]
fn report_zones____nullblk____zone_start_sectors_are_contiguous() {
    let nullblk = require_nullblk!("nullb_rz_contig");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let zones = dev
        .report_zones(Sector::ZERO, EXPECTED_NR_ZONES)
        .expect("report_zones failed");
    assert_eq!(zones.len(), EXPECTED_NR_ZONES as usize);

    for (i, zone) in zones.iter().enumerate() {
        let expected_start = Sector(i as u64 * ZONE_SIZE_SECTORS);
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
    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

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

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

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

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

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

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

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
        zones[0].write_pointer,
        Some(zones[0].start),
        "write pointer should be at zone start after reset"
    );
}

#[test]
fn finish_zones____nullblk____write_pointer_advances_to_end() {
    let nullblk = require_nullblk!("nullb_finish_wp");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

    dev.finish_zones(seq_start, zone_len)
        .expect("finish_zones failed");

    let zones = dev.report_zones(seq_start, 1).expect("report_zones failed");
    assert_eq!(
        zones[0].write_pointer,
        Some(zones[0].start + zones[0].len),
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

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

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

    let zone_len = Sector(ZONE_SIZE_SECTORS);

    // Open 3 sequential zones individually
    for i in 0..3u64 {
        let sector = Sector((ZONE_NR_CONV as u64 + i) * ZONE_SIZE_SECTORS);
        dev.open_zones(sector, zone_len)
            .unwrap_or_else(|e| panic!("open_zones failed for zone {i}: {e}"));
    }

    // Verify all three are open
    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
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

    let zone_len = Sector(ZONE_SIZE_SECTORS);
    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);

    // Finish 3 zones
    for i in 0..3u64 {
        let sector = seq_start + Sector(i * ZONE_SIZE_SECTORS);
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

// ============================================================
// Writable open + data I/O
// ============================================================

#[test]
fn open_writable____nullblk____zone_management_works() {
    let nullblk = require_nullblk!("nullb_writable");
    let dev = ZonedDevice::open_writable(nullblk.path()).expect("open_writable failed");
    assert!(dev.is_writable());

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

    dev.open_zones(seq_start, zone_len).expect("open failed");
    dev.close_zones(seq_start, zone_len).expect("close failed");
    dev.reset_zones(seq_start, zone_len).expect("reset failed");
}

#[test]
fn write_at____nullblk____sequential_write_advances_write_pointer() {
    let nullblk = require_nullblk!("nullb_write_wp");
    let dev = ZonedDevice::open_writable(nullblk.path()).expect("open_writable failed");

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

    // Write 4096 bytes (8 sectors of 512 bytes) at the write pointer
    let data = vec![0xAAu8; 4096];
    let written = dev.write_at(seq_start, &data).expect("write_at failed");
    assert_eq!(written, 4096);

    // Check that write pointer advanced
    let zones = dev.report_zones(seq_start, 1).expect("report failed");
    assert_eq!(
        zones[0].write_pointer,
        Some(seq_start + Sector(8)), // 4096 / 512 = 8 sectors
        "write pointer should have advanced by 8 sectors"
    );

    // Clean up
    dev.reset_zones(seq_start, zone_len).expect("reset failed");
}

#[test]
fn read_at____nullblk____reads_back_written_data() {
    let nullblk = require_nullblk!("nullb_read_back");
    let dev = ZonedDevice::open_writable(nullblk.path()).expect("open_writable failed");

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

    // Write a known pattern
    let mut data = vec![0u8; 4096];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i % 256) as u8;
    }
    dev.write_at(seq_start, &data).expect("write_at failed");

    // Read it back
    let mut buf = vec![0u8; 4096];
    let n = dev.read_at(seq_start, &mut buf).expect("read_at failed");
    assert_eq!(n, 4096);
    assert_eq!(buf, data, "read data should match written data");

    // Clean up
    dev.reset_zones(seq_start, zone_len).expect("reset failed");
}

#[test]
fn write_at____nullblk____conventional_zone_random_write() {
    let nullblk = require_nullblk!("nullb_conv_write");
    let dev = ZonedDevice::open_writable(nullblk.path()).expect("open_writable failed");

    // Write to the middle of the first conventional zone
    let offset = Sector(1024); // sector 1024 (within first conv zone)
    let data = vec![0xBBu8; 512];
    let written = dev.write_at(offset, &data).expect("write_at failed");
    assert_eq!(written, 512);

    let mut buf = vec![0u8; 512];
    let n = dev.read_at(offset, &mut buf).expect("read_at failed");
    assert_eq!(n, 512);
    assert_eq!(buf, data);
}

#[test]
fn write_at____nullblk____read_only_open_returns_error() {
    let nullblk = require_nullblk!("nullb_ro_write");
    let dev = ZonedDevice::open(nullblk.path()).expect("open failed");

    let result = dev.write_at(Sector::ZERO, &[0u8; 512]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, zoned::ZonedError::ReadOnly { .. }),
        "Expected ReadOnly, got: {err:?}"
    );
}

// ============================================================
// ZoneHandle tests
// ============================================================

#[test]
fn zone_handle____nullblk____write_sequential_advances_write_pointer() {
    let nullblk = require_nullblk!("nullb_zh_write");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));

    let first_seq_idx = ZoneIndex(ZONE_NR_CONV);
    let mut handle = ZoneHandle::new(dev.clone(), first_seq_idx).expect("ZoneHandle::new failed");

    let expected_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    assert_eq!(handle.start(), expected_start);
    assert_eq!(handle.write_pointer(), expected_start);

    // Write 4096 bytes
    let data = vec![0xCCu8; 4096];
    let written = handle
        .write_sequential(&data)
        .expect("write_sequential failed");
    assert_eq!(written, 4096);
    assert_eq!(handle.write_pointer(), expected_start + Sector(8));

    // Verify against the device
    let zone = handle.report().expect("report failed");
    assert_eq!(zone.write_pointer, Some(expected_start + Sector(8)));

    // Clean up
    handle.reset().expect("reset failed");
}

#[test]
fn zone_handle____nullblk____reset_resets_write_pointer() {
    let nullblk = require_nullblk!("nullb_zh_reset");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));

    let first_seq_idx = ZoneIndex(ZONE_NR_CONV);
    let mut handle = ZoneHandle::new(dev.clone(), first_seq_idx).expect("ZoneHandle::new failed");
    let start = handle.start();

    handle
        .write_sequential(&vec![0u8; 4096])
        .expect("write failed");
    assert_ne!(handle.write_pointer(), start);

    handle.reset().expect("reset failed");
    assert_eq!(handle.write_pointer(), start);

    let zone = handle.report().expect("report failed");
    assert_eq!(zone.condition, ZoneCondition::Empty);
}

#[test]
fn zone_handle____nullblk____finish_sets_full() {
    let nullblk = require_nullblk!("nullb_zh_finish");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));

    let first_seq_idx = ZoneIndex(ZONE_NR_CONV);
    let mut handle = ZoneHandle::new(dev.clone(), first_seq_idx).expect("ZoneHandle::new failed");

    handle.finish().expect("finish failed");

    let zone = handle.report().expect("report failed");
    assert_eq!(zone.condition, ZoneCondition::Full);
    assert_eq!(handle.write_pointer(), handle.start() + handle.len());

    // Clean up
    handle.reset().expect("reset failed");
}

#[test]
fn zone_handle____nullblk____write_then_read_round_trip() {
    let nullblk = require_nullblk!("nullb_zh_rtrip");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));

    let first_seq_idx = ZoneIndex(ZONE_NR_CONV);
    let mut handle = ZoneHandle::new(dev.clone(), first_seq_idx).expect("ZoneHandle::new failed");

    let mut data = vec![0u8; 4096];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i % 251) as u8; // prime modulus for more interesting pattern
    }
    handle.write_sequential(&data).expect("write failed");

    // Read back via the device (ZoneHandle doesn't have read, by design)
    let mut buf = vec![0u8; 4096];
    let n = dev.read_at(handle.start(), &mut buf).expect("read failed");
    assert_eq!(n, 4096);
    assert_eq!(buf, data);

    handle.reset().expect("reset failed");
}

// ============================================================
// ZoneAllocator tests
// ============================================================

#[test]
fn zone_allocator____nullblk____allocate_returns_empty_sequential_zone() {
    let nullblk = require_nullblk!("nullb_za_alloc");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));
    let allocator = ZoneAllocator::new(dev);

    let handle = allocator.allocate().expect("allocate failed");
    assert!(
        handle.zone_index() >= ZoneIndex(ZONE_NR_CONV),
        "should skip conventional zones"
    );
    assert!(!handle.is_empty());
}

#[test]
fn zone_allocator____nullblk____allocate_zone_specific_index() {
    let nullblk = require_nullblk!("nullb_za_idx");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));
    let allocator = ZoneAllocator::new(dev);

    let zone_idx = ZoneIndex(ZONE_NR_CONV + 3);
    let handle = allocator
        .allocate_zone(zone_idx)
        .expect("allocate_zone failed");
    assert_eq!(handle.zone_index(), zone_idx);
}

#[test]
fn zone_allocator____nullblk____double_allocate_returns_error() {
    let nullblk = require_nullblk!("nullb_za_double");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));
    let allocator = ZoneAllocator::new(dev);

    let zone_idx = ZoneIndex(ZONE_NR_CONV);
    let _handle = allocator
        .allocate_zone(zone_idx)
        .expect("first allocate failed");

    let result = allocator.allocate_zone(zone_idx);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, zoned::ZonedError::ZoneAlreadyAllocated { .. }),
        "Expected ZoneAlreadyAllocated, got: {err:?}"
    );
}

#[test]
fn zone_allocator____nullblk____drop_handle_releases_zone() {
    let nullblk = require_nullblk!("nullb_za_drop");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));
    let allocator = ZoneAllocator::new(dev);

    let zone_idx = ZoneIndex(ZONE_NR_CONV);
    {
        let _handle = allocator.allocate_zone(zone_idx).expect("allocate failed");
        assert_eq!(allocator.allocated_zones(), vec![zone_idx]);
    }
    // Handle dropped — zone should be released
    assert!(allocator.allocated_zones().is_empty());

    // Should be allocatable again
    let _handle2 = allocator
        .allocate_zone(zone_idx)
        .expect("re-allocate failed");
}

#[test]
fn zone_allocator____nullblk____allocated_zones_tracking() {
    let nullblk = require_nullblk!("nullb_za_track");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));
    let allocator = ZoneAllocator::new(dev);

    let _h1 = allocator
        .allocate_zone(ZoneIndex(ZONE_NR_CONV))
        .expect("alloc 1 failed");
    let _h2 = allocator
        .allocate_zone(ZoneIndex(ZONE_NR_CONV + 1))
        .expect("alloc 2 failed");
    let _h3 = allocator
        .allocate_zone(ZoneIndex(ZONE_NR_CONV + 3))
        .expect("alloc 3 failed");

    let allocated = allocator.allocated_zones();
    assert_eq!(
        allocated,
        vec![
            ZoneIndex(ZONE_NR_CONV),
            ZoneIndex(ZONE_NR_CONV + 1),
            ZoneIndex(ZONE_NR_CONV + 3)
        ]
    );
}

// ============================================================
// Concurrent access
// ============================================================

#[test]
fn concurrent____nullblk____parallel_writes_to_different_zones() {
    let nullblk = require_nullblk!("nullb_concurrent");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));
    let allocator = ZoneAllocator::new(dev.clone());

    // Allocate 3 separate zones
    let mut handle_a = allocator.allocate().expect("allocate A failed");
    let mut handle_b = allocator.allocate().expect("allocate B failed");
    let mut handle_c = allocator.allocate().expect("allocate C failed");

    // Each handle goes to a different thread
    let ta = thread::spawn(move || {
        let data = vec![0xAAu8; 4096];
        handle_a.write_sequential(&data).expect("write A failed");
        handle_a
    });

    let tb = thread::spawn(move || {
        let data = vec![0xBBu8; 4096];
        handle_b.write_sequential(&data).expect("write B failed");
        handle_b
    });

    let tc = thread::spawn(move || {
        let data = vec![0xCCu8; 4096];
        handle_c.write_sequential(&data).expect("write C failed");
        handle_c
    });

    let mut ha = ta.join().expect("thread A panicked");
    let mut hb = tb.join().expect("thread B panicked");
    let mut hc = tc.join().expect("thread C panicked");

    // Verify each zone was written correctly
    let mut buf = vec![0u8; 4096];

    dev.read_at(ha.start(), &mut buf).expect("read A failed");
    assert!(buf.iter().all(|&b| b == 0xAA), "zone A data mismatch");

    dev.read_at(hb.start(), &mut buf).expect("read B failed");
    assert!(buf.iter().all(|&b| b == 0xBB), "zone B data mismatch");

    dev.read_at(hc.start(), &mut buf).expect("read C failed");
    assert!(buf.iter().all(|&b| b == 0xCC), "zone C data mismatch");

    // All three zones should still be allocated
    assert_eq!(allocator.allocated_zones().len(), 3);

    // Clean up
    ha.reset().expect("reset A failed");
    hb.reset().expect("reset B failed");
    hc.reset().expect("reset C failed");
}

// ============================================================
// ZoneIterator tests
// ============================================================

#[test]
fn zone_iter____nullblk____yields_all_zones() {
    let nullblk = require_nullblk!("nullb_iter_all");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let count = dev.zone_iter(4).filter_map(|r| r.ok()).count();
    assert_eq!(
        count, EXPECTED_NR_ZONES as usize,
        "zone_iter should yield all {EXPECTED_NR_ZONES} zones, got {count}"
    );
}

#[test]
fn zone_iter____nullblk____matches_report_all_zones() {
    let nullblk = require_nullblk!("nullb_iter_match");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let from_iter: Vec<_> = dev
        .zone_iter(4)
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("iter failed");
    let from_report = dev.report_all_zones(4).expect("report_all_zones failed");

    assert_eq!(from_iter.len(), from_report.len());
    for (i, (a, b)) in from_iter.iter().zip(from_report.iter()).enumerate() {
        assert_eq!(
            a, b,
            "zone {i} differs between iterator and report_all_zones"
        );
    }
}

#[test]
fn zone_iter____nullblk____batch_size_1_works() {
    let nullblk = require_nullblk!("nullb_iter_b1");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let count = dev.zone_iter(1).filter_map(|r| r.ok()).count();
    assert_eq!(count, EXPECTED_NR_ZONES as usize);
}

// ============================================================
// report_zones_filtered tests
// ============================================================

#[test]
fn report_zones_filtered____nullblk____conventional_only() {
    let nullblk = require_nullblk!("nullb_filt_conv");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let filter = zoned::ZoneFilter::new().zone_type(ZoneType::Conventional);
    let zones = dev
        .report_zones_filtered(&filter, 4)
        .expect("filtered report failed");

    assert_eq!(
        zones.len(),
        ZONE_NR_CONV as usize,
        "expected {ZONE_NR_CONV} conventional zones, got {}",
        zones.len()
    );
    for zone in &zones {
        assert_eq!(zone.zone_type, ZoneType::Conventional);
    }
}

#[test]
fn report_zones_filtered____nullblk____empty_sequential() {
    let nullblk = require_nullblk!("nullb_filt_seq");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    let filter = zoned::ZoneFilter::new()
        .zone_type(ZoneType::SequentialWriteRequired)
        .condition(ZoneCondition::Empty);
    let zones = dev
        .report_zones_filtered(&filter, 8)
        .expect("filtered report failed");

    let expected_seq = EXPECTED_NR_ZONES - ZONE_NR_CONV;
    assert_eq!(
        zones.len(),
        expected_seq as usize,
        "expected {expected_seq} empty sequential zones on fresh device, got {}",
        zones.len()
    );
}

#[test]
fn report_zones_filtered____nullblk____no_match_returns_empty() {
    let nullblk = require_nullblk!("nullb_filt_none");
    let dev = ZonedDevice::open(nullblk.path()).expect("failed to open device");

    // No zones should be both conventional and empty (conventional zones are NotWritePointer)
    let filter = zoned::ZoneFilter::new()
        .zone_type(ZoneType::Conventional)
        .condition(ZoneCondition::Empty);
    let zones = dev
        .report_zones_filtered(&filter, 8)
        .expect("filtered report failed");
    assert!(
        zones.is_empty(),
        "expected no zones matching impossible filter"
    );
}

// ============================================================
// Vectored I/O tests
// ============================================================

#[test]
fn writev_at____nullblk____writes_scattered_buffers() {
    let nullblk = require_nullblk!("nullb_writev");
    let dev = ZonedDevice::open_writable(nullblk.path()).expect("open_writable failed");

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

    let buf_a = vec![0xAAu8; 2048];
    let buf_b = vec![0xBBu8; 2048];
    let bufs = [std::io::IoSlice::new(&buf_a), std::io::IoSlice::new(&buf_b)];
    let written = dev.writev_at(seq_start, &bufs).expect("writev_at failed");
    assert_eq!(written, 4096);

    // Read back and verify
    let mut readback = vec![0u8; 4096];
    dev.read_at(seq_start, &mut readback)
        .expect("read_at failed");
    assert!(
        readback[..2048].iter().all(|&b| b == 0xAA),
        "first half mismatch"
    );
    assert!(
        readback[2048..].iter().all(|&b| b == 0xBB),
        "second half mismatch"
    );

    dev.reset_zones(seq_start, zone_len).expect("reset failed");
}

#[test]
fn readv_at____nullblk____reads_into_scattered_buffers() {
    let nullblk = require_nullblk!("nullb_readv");
    let dev = ZonedDevice::open_writable(nullblk.path()).expect("open_writable failed");

    let seq_start = Sector(ZONE_NR_CONV as u64 * ZONE_SIZE_SECTORS);
    let zone_len = Sector(ZONE_SIZE_SECTORS);

    // Write a known pattern: 2048 bytes of 0xCC then 2048 bytes of 0xDD
    let mut data = vec![0xCCu8; 2048];
    data.extend_from_slice(&[0xDDu8; 2048]);
    dev.write_at(seq_start, &data).expect("write_at failed");

    // Read back with scatter
    let mut buf_a = vec![0u8; 2048];
    let mut buf_b = vec![0u8; 2048];
    let mut bufs = [
        std::io::IoSliceMut::new(&mut buf_a),
        std::io::IoSliceMut::new(&mut buf_b),
    ];
    let n = dev.readv_at(seq_start, &mut bufs).expect("readv_at failed");
    assert_eq!(n, 4096);
    assert!(
        buf_a.iter().all(|&b| b == 0xCC),
        "first scatter buf mismatch"
    );
    assert!(
        buf_b.iter().all(|&b| b == 0xDD),
        "second scatter buf mismatch"
    );

    dev.reset_zones(seq_start, zone_len).expect("reset failed");
}

#[test]
fn writev_sequential____nullblk____advances_write_pointer() {
    let nullblk = require_nullblk!("nullb_writev_seq");
    let dev = Arc::new(ZonedDevice::open_writable(nullblk.path()).expect("open failed"));

    let first_seq_idx = ZoneIndex(ZONE_NR_CONV);
    let mut handle = ZoneHandle::new(dev.clone(), first_seq_idx).expect("ZoneHandle::new failed");
    let start = handle.start();

    let buf_a = vec![0xEEu8; 2048];
    let buf_b = vec![0xFFu8; 2048];
    let bufs = [std::io::IoSlice::new(&buf_a), std::io::IoSlice::new(&buf_b)];
    let written = handle
        .writev_sequential(&bufs)
        .expect("writev_sequential failed");
    assert_eq!(written, 4096);
    assert_eq!(handle.write_pointer(), start + Sector(8)); // 4096 / 512 = 8

    // Read back via device to verify
    let mut readback = vec![0u8; 4096];
    dev.read_at(start, &mut readback).expect("read failed");
    assert!(
        readback[..2048].iter().all(|&b| b == 0xEE),
        "first half mismatch"
    );
    assert!(
        readback[2048..].iter().all(|&b| b == 0xFF),
        "second half mismatch"
    );

    handle.reset().expect("reset failed");
}

// ============================================================
// DeviceBuilder tests
// ============================================================

#[test]
fn builder____nullblk____open_read_only() {
    let nullblk = require_nullblk!("nullb_bld_ro");
    let dev = ZonedDevice::builder(nullblk.path())
        .open()
        .expect("builder open failed");
    assert!(!dev.is_writable());
}

#[test]
fn builder____nullblk____open_writable() {
    let nullblk = require_nullblk!("nullb_bld_wr");
    let dev = ZonedDevice::builder(nullblk.path())
        .writable()
        .open()
        .expect("builder writable open failed");
    assert!(dev.is_writable());
}

#[test]
fn builder____nullblk____open_direct_io() {
    let nullblk = require_nullblk!("nullb_bld_dio");
    let dev = ZonedDevice::builder(nullblk.path())
        .direct_io()
        .open()
        .expect("builder direct_io open failed");
    assert!(dev.is_writable());
}

#[test]
fn builder____nullblk____validate_all_passes() {
    let nullblk = require_nullblk!("nullb_bld_val");
    let dev = ZonedDevice::builder(nullblk.path())
        .validate_all()
        .open()
        .expect("builder validate_all should pass on a zoned block device");
    let _ = dev.device_info().expect("device_info failed");
}

#[test]
fn builder____nullblk____validate_individual_checks_pass() {
    let nullblk = require_nullblk!("nullb_bld_indv");
    let dev = ZonedDevice::builder(nullblk.path())
        .validate_block_device()
        .validate_not_mounted()
        .validate_no_partitions()
        .validate_is_zoned()
        .open()
        .expect("individual validate checks should pass on a zoned block device");
    let _ = dev.device_info().expect("device_info failed");
}

#[test]
fn builder____nonexistent____validate_block_device_fails() {
    let result = ZonedDevice::builder("/dev/this_does_not_exist_zzz")
        .validate_block_device()
        .open();
    assert!(result.is_err());
}

#[test]
fn builder____tmpfile____validate_block_device_fails() {
    let tmpfile = tempfile::NamedTempFile::new().expect("tmpfile failed");
    let result = ZonedDevice::builder(tmpfile.path())
        .validate_block_device()
        .open();
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, zoned::ZonedError::NotABlockDevice { .. }),
        "Expected NotABlockDevice, got: {err:?}"
    );
}

// ============================================================
// Validate module tests (against real null_blk device)
// ============================================================

#[test]
fn validate____nullblk____is_block_device_passes() {
    let nullblk = require_nullblk!("nullb_val_blk");
    zoned::validate::is_block_device(nullblk.path()).expect("is_block_device should pass");
}

#[test]
fn validate____nullblk____is_not_mounted_passes() {
    let nullblk = require_nullblk!("nullb_val_mnt");
    zoned::validate::is_not_mounted(nullblk.path()).expect("is_not_mounted should pass");
}

#[test]
fn validate____nullblk____has_no_partitions_passes() {
    let nullblk = require_nullblk!("nullb_val_part");
    zoned::validate::has_no_partitions(nullblk.path()).expect("has_no_partitions should pass");
}

#[test]
fn validate____nullblk____is_zoned_device_passes() {
    let nullblk = require_nullblk!("nullb_val_zoned");
    zoned::validate::is_zoned_device(nullblk.path()).expect("is_zoned_device should pass");
}
