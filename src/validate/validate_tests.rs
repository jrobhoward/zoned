#![allow(clippy::unwrap_used)]
#![allow(non_snake_case)]

use std::path::Path;

use super::*;

#[test]
fn is_block_device____regular_file____returns_not_a_block_device() {
    let tmpfile = tempfile::NamedTempFile::new().unwrap();
    let result = is_block_device(tmpfile.path());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ZonedError::NotABlockDevice { .. }),
        "Expected NotABlockDevice, got: {err:?}"
    );
}

#[test]
fn is_block_device____nonexistent____returns_device_not_found() {
    let result = is_block_device(Path::new("/dev/this_does_not_exist_zzz"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ZonedError::DeviceNotFound { .. }),
        "Expected DeviceNotFound, got: {err:?}"
    );
}

#[test]
fn is_block_device____directory____returns_not_a_block_device() {
    let tmpdir = tempfile::tempdir().unwrap();
    let result = is_block_device(tmpdir.path());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ZonedError::NotABlockDevice { .. }),
        "Expected NotABlockDevice, got: {err:?}"
    );
}

#[test]
fn is_not_mounted____regular_file____succeeds() {
    // A regular file is never mounted, so this should pass.
    let tmpfile = tempfile::NamedTempFile::new().unwrap();
    let result = is_not_mounted(tmpfile.path());
    assert!(
        result.is_ok(),
        "regular file should not be mounted: {result:?}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn has_no_partitions____nonexistent_sysfs____returns_error() {
    // On Linux, a path not in /sys/block/ should fail.
    let result = has_no_partitions(Path::new("/dev/this_does_not_exist_zzz"));
    assert!(result.is_err());
}

#[test]
#[cfg(target_os = "linux")]
fn has_no_partitions____tmpfile____returns_error() {
    // On Linux, a tempfile won't have a /sys/block/ entry.
    let tmpfile = tempfile::NamedTempFile::new().unwrap();
    let result = has_no_partitions(tmpfile.path());
    assert!(result.is_err());
}

#[test]
#[cfg(not(target_os = "linux"))]
fn has_no_partitions____nonexistent____passes() {
    // On FreeBSD, a nonexistent device has no partition nodes — returns Ok.
    let result = has_no_partitions(Path::new("/dev/this_does_not_exist_zzz"));
    assert!(result.is_ok());
}

#[test]
fn is_zoned_device____nonexistent____returns_error() {
    let result = is_zoned_device(Path::new("/dev/this_does_not_exist_zzz"));
    assert!(result.is_err());
}
