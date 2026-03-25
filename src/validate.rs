//! Device validation functions for zoned block devices.
//!
//! These checks are optional safety guards that can be composed via
//! [`DeviceBuilder`](crate::DeviceBuilder) or called individually.

use std::fs;
use std::path::Path;

use crate::error::{Result, ZonedError};
use crate::sysfs;
use crate::types::DeviceModel;

/// Check that the path refers to a block device (mode bits `S_IFBLK`).
pub fn is_block_device(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ZonedError::DeviceNotFound {
                path: path.to_path_buf(),
            }
        } else {
            ZonedError::Io {
                path: path.to_path_buf(),
                source: e,
            }
        }
    })?;

    let mode = metadata.mode();
    let file_type = mode & 0o170000;
    if file_type != 0o060000 {
        return Err(ZonedError::NotABlockDevice {
            path: path.to_path_buf(),
            mode,
        });
    }

    Ok(())
}

/// Check that the device is not currently mounted.
///
/// Reads `/proc/self/mountinfo` and matches by device major:minor numbers.
pub fn is_not_mounted(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let dev_stat = fs::metadata(path).map_err(|e| ZonedError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let dev_rdev = dev_stat.rdev();
    let dev_major = (dev_rdev >> 8) as u32;
    let dev_minor = (dev_rdev & 0xFF) as u32;

    let mountinfo = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();

    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        if let Some((maj_str, min_str)) = fields[2].split_once(':')
            && let (Ok(maj), Ok(min)) = (maj_str.parse::<u32>(), min_str.parse::<u32>())
            && maj == dev_major
            && min == dev_minor
        {
            return Err(ZonedError::DeviceMounted {
                path: path.to_path_buf(),
                mount_point: fields[4].to_string(),
            });
        }
    }

    Ok(())
}

/// Check that the device has no partitions.
///
/// Looks for partition entries under `/sys/block/<device>/`.
pub fn has_no_partitions(path: &Path) -> Result<()> {
    let dev_name =
        path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| ZonedError::DeviceNotFound {
                path: path.to_path_buf(),
            })?;

    let sysfs_dir = format!("/sys/block/{dev_name}");
    if !Path::new(&sysfs_dir).exists() {
        // Not in /sys/block — likely a partition itself, not a whole disk.
        // We don't have partition info to report, so return DeviceNotFound.
        return Err(ZonedError::DeviceNotFound {
            path: path.to_path_buf(),
        });
    }

    let entries = fs::read_dir(&sysfs_dir).map_err(|e| ZonedError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let mut partitions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ZonedError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(dev_name) && entry.path().join("partition").exists() {
            partitions.push(name_str.to_string());
        }
    }

    if !partitions.is_empty() {
        partitions.sort();
        return Err(ZonedError::DeviceHasPartitions {
            path: path.to_path_buf(),
            partitions,
        });
    }

    Ok(())
}

/// Check that the device is a zoned block device (via sysfs).
///
/// Returns `NotZoned` if the device model is `None`.
pub fn is_zoned_device(path: &Path) -> Result<()> {
    let model = sysfs::device_model(path)?;
    if model == DeviceModel::None {
        return Err(ZonedError::NotZoned {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}
