#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]
#![cfg(feature = "tokio")]

//! Async API integration tests using the null_blk emulated device.
//!
//! Run with: `sudo cargo test --test async_integration --features tokio`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use zoned::async_api::AsyncZonedDevice;
use zoned::{Sector, ZoneCondition, ZoneFilter, ZoneIndex, ZoneType};

// --- null_blk device setup (same as nullblk_integration.rs) ---

const DEVICE_SIZE_MB: u32 = 1024;
const ZONE_SIZE_MB: u32 = 64;
const ZONE_NR_CONV: u32 = 2;
const ZONE_MAX_OPEN: u32 = 4;
const ZONE_MAX_ACTIVE: u32 = 8;
const BLOCK_SIZE: u32 = 4096;
const ZONE_SIZE_SECTORS: u64 = ZONE_SIZE_MB as u64 * 1024 * 1024 / 512;
const EXPECTED_NR_ZONES: u32 = DEVICE_SIZE_MB / ZONE_SIZE_MB;

struct NullBlkDevice {
    #[allow(dead_code)]
    name: String,
    configfs_path: PathBuf,
    dev_path: PathBuf,
}

impl NullBlkDevice {
    fn create(name: &str) -> Option<Self> {
        if unsafe { libc::getuid() != 0 } {
            eprintln!("SKIPPED: not running as root");
            return None;
        }

        let loaded = fs::read_to_string("/proc/modules")
            .map(|m| m.contains("null_blk"))
            .unwrap_or(false);
        if !loaded {
            let ok = Command::new("modprobe")
                .args(["null_blk", "nr_devices=0"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                eprintln!("SKIPPED: could not load null_blk module");
                return None;
            }
        }

        let configfs_path = PathBuf::from(format!("/sys/kernel/config/nullb/{name}"));
        let dev_path = PathBuf::from(format!("/dev/{name}"));

        if configfs_path.exists() {
            let _ = fs::write(configfs_path.join("power"), "0");
            let _ = fs::remove_dir(&configfs_path);
        }

        if fs::create_dir(&configfs_path).is_err() {
            eprintln!("SKIPPED: could not create configfs directory");
            return None;
        }

        let device = Self {
            name: name.to_string(),
            configfs_path,
            dev_path,
        };

        for (attr, val) in [
            ("size", &DEVICE_SIZE_MB.to_string()),
            ("blocksize", &BLOCK_SIZE.to_string()),
            ("zoned", &"1".to_string()),
            ("zone_size", &ZONE_SIZE_MB.to_string()),
            ("zone_nr_conv", &ZONE_NR_CONV.to_string()),
            ("zone_max_open", &ZONE_MAX_OPEN.to_string()),
            ("zone_max_active", &ZONE_MAX_ACTIVE.to_string()),
            ("memory_backed", &"1".to_string()),
        ] {
            fs::write(device.configfs_path.join(attr), val)
                .unwrap_or_else(|e| panic!("failed to write {attr}={val}: {e}"));
        }
        fs::write(device.configfs_path.join("power"), "1").expect("failed to power on");

        if !device.dev_path.exists() {
            eprintln!("SKIPPED: device did not appear");
            let _ = fs::remove_dir(&device.configfs_path);
            return None;
        }

        Some(device)
    }

    fn path(&self) -> &Path {
        &self.dev_path
    }
}

impl Drop for NullBlkDevice {
    fn drop(&mut self) {
        let _ = fs::write(self.configfs_path.join("power"), "0");
        let _ = fs::remove_dir(&self.configfs_path);
    }
}

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
// AsyncZonedDevice tests
// ============================================================

#[tokio::test]
async fn async_device____open____succeeds() {
    let nullblk = require_nullblk!("nullb_async_open");
    let dev = AsyncZonedDevice::open(nullblk.path())
        .await
        .expect("async open failed");
    assert!(!dev.is_writable());
}

#[tokio::test]
async fn async_device____open_writable____succeeds() {
    let nullblk = require_nullblk!("nullb_async_ow");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("async open_writable failed");
    assert!(dev.is_writable());
}

#[tokio::test]
async fn async_device____device_info____returns_correct_values() {
    let nullblk = require_nullblk!("nullb_async_info");
    let dev = AsyncZonedDevice::open(nullblk.path())
        .await
        .expect("open failed");

    let info = dev.device_info().expect("device_info failed");
    assert_eq!(info.zone_size, Sector::new(ZONE_SIZE_SECTORS));
    assert_eq!(info.nr_zones, EXPECTED_NR_ZONES);
}

#[tokio::test]
async fn async_device____path____returns_device_path() {
    let nullblk = require_nullblk!("nullb_async_path");
    let dev = AsyncZonedDevice::open(nullblk.path())
        .await
        .expect("open failed");
    assert_eq!(dev.path(), nullblk.path());
}

#[tokio::test]
async fn async_device____report_zones____returns_zones() {
    let nullblk = require_nullblk!("nullb_async_rz");
    let dev = AsyncZonedDevice::open(nullblk.path())
        .await
        .expect("open failed");

    let zones = dev
        .report_zones(Sector::ZERO, 4)
        .await
        .expect("report_zones failed");
    assert_eq!(zones.len(), 4);
    assert_eq!(zones[0].zone_type, ZoneType::Conventional);
}

#[tokio::test]
async fn async_device____report_all_zones____returns_all() {
    let nullblk = require_nullblk!("nullb_async_raz");
    let dev = AsyncZonedDevice::open(nullblk.path())
        .await
        .expect("open failed");

    let zones = dev
        .report_all_zones(64)
        .await
        .expect("report_all_zones failed");
    assert_eq!(zones.len() as u32, EXPECTED_NR_ZONES);
}

#[tokio::test]
async fn async_device____report_zones_filtered____works() {
    let nullblk = require_nullblk!("nullb_async_rzf");
    let dev = AsyncZonedDevice::open(nullblk.path())
        .await
        .expect("open failed");

    let filter = ZoneFilter::new()
        .zone_type(ZoneType::SequentialWriteRequired)
        .condition(ZoneCondition::Empty);
    let zones = dev
        .report_zones_filtered(filter, 64)
        .await
        .expect("report_zones_filtered failed");
    assert_eq!(zones.len() as u32, EXPECTED_NR_ZONES - ZONE_NR_CONV);
}

#[tokio::test]
async fn async_device____from_sync____wraps_existing() {
    let nullblk = require_nullblk!("nullb_async_fs");
    let sync_dev = zoned::ZonedDevice::open(nullblk.path()).expect("sync open failed");
    let dev = AsyncZonedDevice::from_sync(sync_dev);
    let info = dev.device_info().expect("device_info failed");
    assert_eq!(info.nr_zones, EXPECTED_NR_ZONES);
}

#[tokio::test]
async fn async_device____from_arc____wraps_shared() {
    let nullblk = require_nullblk!("nullb_async_fa");
    let sync_dev =
        std::sync::Arc::new(zoned::ZonedDevice::open(nullblk.path()).expect("sync open failed"));
    let dev = AsyncZonedDevice::from_arc(sync_dev.clone());
    assert_eq!(dev.inner().path(), nullblk.path());
    assert_eq!(dev.inner_arc().path(), nullblk.path());
}

#[tokio::test]
async fn async_device____write_read_round_trip() {
    let nullblk = require_nullblk!("nullb_async_wrt");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");

    // Write to the first conventional zone
    let data = vec![0xBB_u8; 4096];
    dev.write_all_at(Sector::ZERO, data.clone())
        .await
        .expect("write_all_at failed");

    // Read back
    let result = dev
        .read_at(Sector::ZERO, 4096)
        .await
        .expect("read_at failed");
    assert_eq!(result, data);
}

#[tokio::test]
async fn async_device____fsync____succeeds() {
    let nullblk = require_nullblk!("nullb_async_fsync");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");
    dev.fsync().await.expect("fsync failed");
}

// ============================================================
// AsyncZoneHandle tests
// ============================================================

#[tokio::test]
async fn async_handle____write_sequential____advances_pointer() {
    let nullblk = require_nullblk!("nullb_async_hws");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");

    // Zone 2 is the first sequential zone (0, 1 are conventional)
    let handle = dev
        .zone_handle(ZoneIndex::new(2))
        .await
        .expect("zone_handle failed");

    assert_eq!(handle.zone_index().await, ZoneIndex::new(2));
    assert!(handle.is_empty().await);

    let start = handle.start().await;
    assert_eq!(handle.write_pointer().await, start);

    let written = handle
        .write_sequential(vec![0u8; 4096])
        .await
        .expect("write failed");
    assert_eq!(written, 4096);

    assert_eq!(
        handle.write_pointer().await,
        start + Sector::new(4096 / 512)
    );
}

#[tokio::test]
async fn async_handle____write_all_sequential____completes() {
    let nullblk = require_nullblk!("nullb_async_hwa");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");

    let handle = dev
        .zone_handle(ZoneIndex::new(3))
        .await
        .expect("zone_handle failed");
    handle
        .write_all_sequential(vec![0u8; 8192])
        .await
        .expect("write_all failed");
    let start = handle.start().await;
    assert_eq!(
        handle.write_pointer().await,
        start + Sector::new(8192 / 512)
    );
}

#[tokio::test]
async fn async_handle____reset____clears_pointer() {
    let nullblk = require_nullblk!("nullb_async_hrst");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");

    let handle = dev
        .zone_handle(ZoneIndex::new(4))
        .await
        .expect("zone_handle failed");
    handle
        .write_sequential(vec![0u8; 4096])
        .await
        .expect("write failed");
    handle.reset().await.expect("reset failed");

    let start = handle.start().await;
    assert_eq!(handle.write_pointer().await, start);
}

#[tokio::test]
async fn async_handle____open_close_finish____lifecycle() {
    let nullblk = require_nullblk!("nullb_async_hlc");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");

    let handle = dev
        .zone_handle(ZoneIndex::new(5))
        .await
        .expect("zone_handle failed");

    handle.open().await.expect("open failed");
    let report = handle.report().await.expect("report failed");
    assert_eq!(report.condition, ZoneCondition::ExplicitlyOpen);

    handle.close().await.expect("close failed");
    let report = handle.report().await.expect("report failed");
    assert_eq!(report.condition, ZoneCondition::Closed);

    handle.finish().await.expect("finish failed");
    let report = handle.report().await.expect("report failed");
    assert_eq!(report.condition, ZoneCondition::Full);
}

#[tokio::test]
async fn async_handle____properties____correct() {
    let nullblk = require_nullblk!("nullb_async_hprop");
    let dev = AsyncZonedDevice::open(nullblk.path())
        .await
        .expect("open failed");

    let handle = dev
        .zone_handle(ZoneIndex::new(2))
        .await
        .expect("zone_handle failed");
    assert_eq!(handle.len().await, Sector::new(ZONE_SIZE_SECTORS));
    assert!(handle.capacity().await.raw() > 0);
    assert_eq!(handle.zone_index().await, ZoneIndex::new(2));
}

#[tokio::test]
async fn async_handle____writev_sequential____works() {
    let nullblk = require_nullblk!("nullb_async_hwv");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");

    let handle = dev
        .zone_handle(ZoneIndex::new(6))
        .await
        .expect("zone_handle failed");

    let bufs = vec![vec![0xAA_u8; 2048], vec![0xBB_u8; 2048]];
    let written = handle.writev_sequential(bufs).await.expect("writev failed");
    assert_eq!(written, 4096);
}

// ============================================================
// AsyncZonedDevice zone management
// ============================================================

#[tokio::test]
async fn async_device____zone_management____open_close_reset() {
    let nullblk = require_nullblk!("nullb_async_zmgmt");
    let dev = AsyncZonedDevice::open_writable(nullblk.path())
        .await
        .expect("open_writable failed");

    let info = dev.device_info().expect("device_info failed");
    // First sequential zone
    let sector = info.zone_size * ZONE_NR_CONV as u64;

    dev.open_zones(sector, info.zone_size)
        .await
        .expect("open failed");
    let zones = dev.report_zones(sector, 1).await.expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::ExplicitlyOpen);

    dev.close_zones(sector, info.zone_size)
        .await
        .expect("close failed");
    let zones = dev.report_zones(sector, 1).await.expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::Closed);

    dev.finish_zones(sector, info.zone_size)
        .await
        .expect("finish failed");
    let zones = dev.report_zones(sector, 1).await.expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::Full);

    dev.reset_zones(sector, info.zone_size)
        .await
        .expect("reset failed");
    let zones = dev.report_zones(sector, 1).await.expect("report failed");
    assert_eq!(zones[0].condition, ZoneCondition::Empty);
}
