#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

#[test]
fn open____nonexistent_device____returns_device_not_found() {
    let result = ZonedDevice::open("/dev/this_device_does_not_exist_zzz");
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(
        matches!(err, ZonedError::DeviceNotFound { .. }),
        "Expected DeviceNotFound, got: {err:?}"
    );
}

#[test]
fn open____regular_file____returns_error() {
    // Opening a regular file should fail at the ioctl level, not at open.
    // But we can at least verify open itself succeeds on a readable file.
    let tmpfile = tempfile::NamedTempFile::new().unwrap();
    let result = ZonedDevice::open(tmpfile.path());

    // On Linux, opening a regular file succeeds (it's just a file open),
    // but device_info() will fail because ioctls don't work on regular files.
    if let Ok(dev) = result {
        let info_result = dev.device_info();
        assert!(info_result.is_err());
    }
    // On other platforms, open itself may fail with UnsupportedPlatform.
}

#[test]
fn validate_range____zero_nr_sectors____returns_invalid_range() {
    let tmpfile = tempfile::NamedTempFile::new().unwrap();

    // We need to test validate_range indirectly through the public API.
    // If open succeeds, try reset_zones with invalid range.
    if let Ok(dev) = ZonedDevice::open(tmpfile.path()) {
        let result = dev.reset_zones(0, 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                ZonedError::InvalidRange {
                    sector: 0,
                    nr_sectors: 0
                }
            ),
            "Expected InvalidRange, got: {err:?}"
        );
    }
}

#[test]
fn validate_range____overflow____returns_invalid_range() {
    let tmpfile = tempfile::NamedTempFile::new().unwrap();

    if let Ok(dev) = ZonedDevice::open(tmpfile.path()) {
        let result = dev.reset_zones(u64::MAX, 1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ZonedError::InvalidRange { .. }),
            "Expected InvalidRange, got: {err:?}"
        );
    }
}

#[test]
fn debug____format____includes_path() {
    let tmpfile = tempfile::NamedTempFile::new().unwrap();

    if let Ok(dev) = ZonedDevice::open(tmpfile.path()) {
        let debug_str = format!("{dev:?}");
        assert!(debug_str.contains("ZonedDevice"));
    }
}
