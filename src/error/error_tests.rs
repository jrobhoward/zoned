#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use std::path::PathBuf;

use crate::types::{Sector, ZoneIndex};

use super::*;

#[test]
fn zoned_error____device_not_found____display_message() {
    let err = ZonedError::DeviceNotFound {
        path: PathBuf::from("/dev/sdb"),
    };
    let msg = format!("{err}");
    assert!(msg.contains("/dev/sdb"), "Message was: {msg}");
    assert!(msg.contains("not found"), "Message was: {msg}");
}

#[test]
fn zoned_error____not_zoned____display_message() {
    let err = ZonedError::NotZoned {
        path: PathBuf::from("/dev/sda"),
    };
    let msg = format!("{err}");
    assert!(msg.contains("/dev/sda"), "Message was: {msg}");
    assert!(msg.contains("not a zoned"), "Message was: {msg}");
}

#[test]
fn zoned_error____io____display_message() {
    let err = ZonedError::Io {
        path: PathBuf::from("/dev/sdb"),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied"),
    };
    let msg = format!("{err}");
    assert!(msg.contains("/dev/sdb"), "Message was: {msg}");
    assert!(msg.contains("I/O error"), "Message was: {msg}");
}

#[test]
fn zoned_error____invalid_range____display_message() {
    let err = ZonedError::InvalidRange {
        sector: Sector(100),
        nr_sectors: Sector(0),
    };
    let msg = format!("{err}");
    assert!(msg.contains("100"), "Message was: {msg}");
    assert!(msg.contains("invalid zone range"), "Message was: {msg}");
}

#[test]
fn zoned_error____sysfs____display_message() {
    let err = ZonedError::Sysfs {
        path: PathBuf::from("/dev/sdb"),
        attribute: "zoned".to_string(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
    };
    let msg = format!("{err}");
    assert!(msg.contains("sdb"), "Message was: {msg}");
    assert!(msg.contains("zoned"), "Message was: {msg}");
}

#[test]
fn zoned_error____sysfs_parse____display_message() {
    let err = ZonedError::SysfsParse {
        path: PathBuf::from("/dev/sdb"),
        attribute: "nr_zones".to_string(),
        value: "not_a_number".to_string(),
    };
    let msg = format!("{err}");
    assert!(msg.contains("sdb"), "Message was: {msg}");
    assert!(msg.contains("nr_zones"), "Message was: {msg}");
    assert!(msg.contains("not_a_number"), "Message was: {msg}");
}

#[test]
fn zoned_error____read_only____display_message() {
    let err = ZonedError::ReadOnly {
        path: PathBuf::from("/dev/sdb"),
    };
    let msg = format!("{err}");
    assert!(msg.contains("/dev/sdb"), "Message was: {msg}");
    assert!(msg.contains("read-only"), "Message was: {msg}");
}

#[test]
fn zoned_error____zone_already_allocated____display_message() {
    let err = ZonedError::ZoneAlreadyAllocated {
        zone_index: ZoneIndex(5),
    };
    let msg = format!("{err}");
    assert!(msg.contains("5"), "Message was: {msg}");
    assert!(msg.contains("already allocated"), "Message was: {msg}");
}

#[test]
fn zoned_error____zone_not_allocated____display_message() {
    let err = ZonedError::ZoneNotAllocated {
        zone_index: ZoneIndex(7),
    };
    let msg = format!("{err}");
    assert!(msg.contains("7"), "Message was: {msg}");
    assert!(msg.contains("not allocated"), "Message was: {msg}");
}

#[test]
fn zoned_error____zone_full____display_message() {
    let err = ZonedError::ZoneFull {
        zone_index: ZoneIndex(3),
    };
    let msg = format!("{err}");
    assert!(msg.contains("3"), "Message was: {msg}");
    assert!(msg.contains("full"), "Message was: {msg}");
}

#[test]
fn zoned_error____not_a_block_device____display_message() {
    let err = ZonedError::NotABlockDevice {
        path: PathBuf::from("/tmp/foo"),
        mode: 0o100644,
    };
    let msg = format!("{err}");
    assert!(msg.contains("/tmp/foo"), "Message was: {msg}");
    assert!(msg.contains("not a block device"), "Message was: {msg}");
    assert!(msg.contains("0o100644"), "Message was: {msg}");
}

#[test]
fn zoned_error____device_mounted____display_message() {
    let err = ZonedError::DeviceMounted {
        path: PathBuf::from("/dev/sda"),
        mount_point: "/mnt/data".to_string(),
    };
    let msg = format!("{err}");
    assert!(msg.contains("/dev/sda"), "Message was: {msg}");
    assert!(msg.contains("/mnt/data"), "Message was: {msg}");
}

#[test]
fn zoned_error____device_has_partitions____display_message() {
    let err = ZonedError::DeviceHasPartitions {
        path: PathBuf::from("/dev/sda"),
        partitions: vec!["sda1".to_string(), "sda2".to_string()],
    };
    let msg = format!("{err}");
    assert!(msg.contains("/dev/sda"), "Message was: {msg}");
    assert!(msg.contains("sda1"), "Message was: {msg}");
    assert!(msg.contains("sda2"), "Message was: {msg}");
}

#[test]
fn zoned_error____unsupported_platform____display_message() {
    let err = ZonedError::UnsupportedPlatform;
    let msg = format!("{err}");
    assert!(msg.contains("not supported"), "Message was: {msg}");
}

#[test]
fn result_type____ok____is_accessible() {
    let result: Result<u32> = Ok(42);
    assert!(result.is_ok());
}

#[test]
fn result_type____err____is_accessible() {
    let result: Result<u32> = Err(ZonedError::UnsupportedPlatform);
    assert!(result.is_err());
}
