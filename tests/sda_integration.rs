#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

//! Read-only integration tests against /dev/sda (a real zoned SMR disk).
//!
//! These tests NEVER modify device state — no reset, open, close, or finish
//! operations, and no writes. Safe to run on a disk with live data.
//!
//! Requires:
//! - /dev/sda exists and is a host-managed zoned device
//! - Current user has read access (e.g. member of the `disk` group)
//!
//! Tests are skipped automatically if these prerequisites are not met.
//!
//! Run with: `cargo test --test sda_integration`

use std::path::Path;

use zoned::{DeviceModel, Sector, ZoneCondition, ZoneType, ZonedDevice};

const DEV_PATH: &str = "/dev/sda";

// Known device parameters from sysfs:
//   chunk_sectors = 524288  (256 MB zones in 512-byte sectors)
//   nr_zones      = 37256
//   max_open_zones = 16
//   max_active_zones = 0
//   zoned = host-managed
const EXPECTED_ZONE_SIZE_SECTORS: u64 = 524288;
const EXPECTED_NR_ZONES: u32 = 37256;
const EXPECTED_MAX_OPEN: u32 = 16;
const EXPECTED_MAX_ACTIVE: u32 = 0;

fn open_sda() -> Option<ZonedDevice> {
    let path = Path::new(DEV_PATH);

    if !path.exists() {
        eprintln!("SKIPPED: {DEV_PATH} does not exist");
        return None;
    }

    match ZonedDevice::open(path) {
        Ok(dev) => Some(dev),
        Err(e) => {
            eprintln!("SKIPPED: cannot open {DEV_PATH}: {e}");
            None
        }
    }
}

macro_rules! require_sda {
    () => {
        match open_sda() {
            Some(dev) => dev,
            None => {
                eprintln!("  test skipped (requires read access to {DEV_PATH})");
                return;
            }
        }
    };
}

// ============================================================
// Device info (ioctl)
// ============================================================

#[test]
fn device_info____sda____returns_correct_zone_size() {
    let dev = require_sda!();
    let info = dev.device_info().expect("device_info failed");

    assert_eq!(
        info.zone_size,
        Sector(EXPECTED_ZONE_SIZE_SECTORS),
        "zone_size: expected {EXPECTED_ZONE_SIZE_SECTORS}, got {}",
        info.zone_size
    );
}

#[test]
fn device_info____sda____returns_correct_zone_count() {
    let dev = require_sda!();
    let info = dev.device_info().expect("device_info failed");

    assert_eq!(
        info.nr_zones, EXPECTED_NR_ZONES,
        "nr_zones: expected {EXPECTED_NR_ZONES}, got {}",
        info.nr_zones
    );
}

// ============================================================
// Sysfs
// ============================================================

#[test]
fn sysfs____sda____reports_host_managed() {
    let path = Path::new(DEV_PATH);
    if !path.exists() {
        eprintln!("SKIPPED: {DEV_PATH} does not exist");
        return;
    }

    let model = zoned::sysfs::device_model(path).expect("device_model failed");
    assert_eq!(model, DeviceModel::HostManaged);
}

#[test]
fn sysfs____sda____properties_match_known_values() {
    let path = Path::new(DEV_PATH);
    if !path.exists() {
        eprintln!("SKIPPED: {DEV_PATH} does not exist");
        return;
    }

    let props = zoned::sysfs::device_properties(path).expect("device_properties failed");

    assert_eq!(props.model, DeviceModel::HostManaged);
    assert_eq!(props.chunk_sectors, Sector(EXPECTED_ZONE_SIZE_SECTORS));
    assert_eq!(props.nr_zones, EXPECTED_NR_ZONES);
    assert_eq!(props.max_open_zones, EXPECTED_MAX_OPEN);
    assert_eq!(props.max_active_zones, EXPECTED_MAX_ACTIVE);
}

// ============================================================
// Report zones — structure validation
// ============================================================

#[test]
fn report_zones____sda____first_zones_are_conventional() {
    let dev = require_sda!();

    let zones = dev
        .report_zones(Sector::ZERO, 8)
        .expect("report_zones failed");
    assert!(!zones.is_empty(), "should return at least 1 zone");

    // From blkzone report, we know at least the first 5 zones are conventional.
    // Check however many we got back.
    let conv_count = zones
        .iter()
        .take_while(|z| z.zone_type == ZoneType::Conventional)
        .count();

    assert!(
        conv_count >= 5,
        "expected at least 5 conventional zones at start, found {conv_count}"
    );

    for (i, zone) in zones.iter().take(conv_count).enumerate() {
        assert_eq!(
            zone.condition,
            ZoneCondition::NotWritePointer,
            "conventional zone {i} should have NotWritePointer condition"
        );
        assert_eq!(zone.len, Sector(EXPECTED_ZONE_SIZE_SECTORS));
        assert_eq!(zone.start, Sector(i as u64 * EXPECTED_ZONE_SIZE_SECTORS));
    }
}

#[test]
fn report_zones____sda____has_sequential_zones_after_conventional() {
    let dev = require_sda!();

    // Fetch enough zones to get past the conventional region
    let zones = dev
        .report_zones(Sector::ZERO, 128)
        .expect("report_zones failed");

    let first_seq = zones
        .iter()
        .find(|z| z.zone_type == ZoneType::SequentialWriteRequired);

    assert!(
        first_seq.is_some(),
        "device should have at least one SequentialWriteRequired zone"
    );

    let seq = first_seq.unwrap();
    assert_eq!(seq.len, Sector(EXPECTED_ZONE_SIZE_SECTORS));
    // Write pointer must be within the zone
    assert!(
        seq.write_pointer >= seq.start && seq.write_pointer <= seq.start + seq.len,
        "write pointer {} should be within zone [{}, {}]",
        seq.write_pointer,
        seq.start,
        seq.start + seq.len
    );
}

#[test]
fn report_zones____sda____zone_starts_are_contiguous() {
    let dev = require_sda!();

    let zones = dev
        .report_zones(Sector::ZERO, 64)
        .expect("report_zones failed");
    assert!(
        zones.len() >= 2,
        "need at least 2 zones to check contiguity"
    );

    for pair in zones.windows(2) {
        let expected_next = pair[0].start + pair[0].len;
        assert_eq!(
            pair[1].start, expected_next,
            "zone at sector {} has length {}, so next should start at {}, but got {}",
            pair[0].start, pair[0].len, expected_next, pair[1].start
        );
    }
}

#[test]
fn report_zones____sda____all_zones_have_valid_zone_size() {
    let dev = require_sda!();

    let zones = dev
        .report_zones(Sector::ZERO, 256)
        .expect("report_zones failed");

    for (i, zone) in zones.iter().enumerate() {
        assert_eq!(
            zone.len,
            Sector(EXPECTED_ZONE_SIZE_SECTORS),
            "zone {i} at sector {} has unexpected length {}",
            zone.start,
            zone.len
        );
        assert!(
            zone.capacity <= zone.len,
            "zone {i} capacity {} exceeds length {}",
            zone.capacity,
            zone.len
        );
    }
}

// ============================================================
// Report zones — mid-device offset
// ============================================================

#[test]
fn report_zones____sda____offset_query_returns_correct_start() {
    let dev = require_sda!();

    // Query starting from zone 100
    let offset_sector = Sector(100 * EXPECTED_ZONE_SIZE_SECTORS);
    let zones = dev
        .report_zones(offset_sector, 4)
        .expect("report_zones at offset failed");

    assert!(!zones.is_empty());
    assert_eq!(
        zones[0].start, offset_sector,
        "first zone should start at requested sector {offset_sector}, got {}",
        zones[0].start
    );
}

// ============================================================
// Report all zones
// ============================================================

#[test]
fn report_all_zones____sda____returns_all_zones() {
    let dev = require_sda!();

    let zones = dev.report_all_zones(512).expect("report_all_zones failed");

    assert_eq!(
        zones.len(),
        EXPECTED_NR_ZONES as usize,
        "expected {EXPECTED_NR_ZONES} zones, got {}",
        zones.len()
    );

    // First zone starts at 0
    assert_eq!(zones[0].start, Sector::ZERO);

    // Last zone ends at the right place
    let last = &zones[zones.len() - 1];
    assert_eq!(
        last.start,
        Sector((EXPECTED_NR_ZONES as u64 - 1) * EXPECTED_ZONE_SIZE_SECTORS)
    );
}

#[test]
fn report_all_zones____sda____small_batch_matches_large_batch() {
    let dev = require_sda!();

    let small = dev.report_all_zones(8).expect("small batch failed");
    let large = dev.report_all_zones(4096).expect("large batch failed");

    assert_eq!(
        small.len(),
        large.len(),
        "different batch sizes should return same total zones"
    );

    // Spot-check a few zones match
    for i in [0, 1, 100, 1000, small.len() - 1] {
        assert_eq!(small[i], large[i], "zone {i} differs between batch sizes");
    }
}

// ============================================================
// Zone condition census
// ============================================================

#[test]
fn report_all_zones____sda____zone_conditions_are_valid() {
    let dev = require_sda!();

    let zones = dev.report_all_zones(512).expect("report_all_zones failed");

    let mut conventional = 0u32;
    let mut seq_required = 0u32;

    for (i, zone) in zones.iter().enumerate() {
        match zone.zone_type {
            ZoneType::Conventional => {
                conventional += 1;
                assert_eq!(
                    zone.condition,
                    ZoneCondition::NotWritePointer,
                    "conventional zone {i} should be NotWritePointer"
                );
            }
            ZoneType::SequentialWriteRequired => {
                seq_required += 1;
                // Write pointer must be within bounds for non-offline zones
                if zone.condition != ZoneCondition::Offline {
                    assert!(
                        zone.write_pointer >= zone.start
                            && zone.write_pointer <= zone.start + zone.len,
                        "zone {i} write pointer {} out of bounds [{}, {}]",
                        zone.write_pointer,
                        zone.start,
                        zone.start + zone.len
                    );
                }
            }
            ZoneType::SequentialWritePreferred => {}
        }
    }

    eprintln!("Zone census: {conventional} conventional, {seq_required} sequential-write-required");
    assert!(
        conventional > 0,
        "expected at least some conventional zones"
    );
    assert!(seq_required > 0, "expected at least some sequential zones");
    assert_eq!(
        conventional + seq_required,
        EXPECTED_NR_ZONES,
        "all zones should be either conventional or sequential"
    );
}

// ============================================================
// Debug formatting
// ============================================================

#[test]
fn debug____sda____includes_device_path() {
    let dev = require_sda!();
    let debug_str = format!("{dev:?}");
    assert!(
        debug_str.contains("sda"),
        "debug output should contain 'sda', got: {debug_str}"
    );
}
