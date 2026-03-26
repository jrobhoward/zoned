//! Device validation functions for zoned block devices.
//!
//! These checks are optional safety guards that can be composed via
//! [`DeviceBuilder`](crate::DeviceBuilder) or called individually.

use std::fs;
use std::path::Path;

use crate::error::{Result, ZonedError};

/// Check that the path refers to a device node.
///
/// On Linux, checks for a block device (`S_IFBLK`). On FreeBSD, disk devices
/// are character devices (`S_IFCHR`), so both are accepted.
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
    // S_IFBLK = 0o060000 (Linux block devices)
    // S_IFCHR = 0o020000 (FreeBSD disk devices are character devices)
    let is_device = file_type == 0o060000 || file_type == 0o020000;
    if !is_device {
        return Err(ZonedError::NotABlockDevice {
            path: path.to_path_buf(),
            mode,
        });
    }

    Ok(())
}

/// Check that the device is not currently mounted.
///
/// On Linux, reads `/proc/self/mountinfo`. On FreeBSD, uses `getfsstat()`.
#[cfg(target_os = "linux")]
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

#[cfg(target_os = "freebsd")]
pub fn is_not_mounted(path: &Path) -> Result<()> {
    use std::ffi::CStr;

    // Get the canonical path to compare against mount entries.
    let dev_path = fs::canonicalize(path)
        .map_err(|e| ZonedError::Io {
            path: path.to_path_buf(),
            source: e,
        })?
        .to_string_lossy()
        .to_string();

    // getfsstat(NULL, 0, MNT_NOWAIT) returns the number of mounted filesystems.
    // SAFETY: First call with null buffer to get count.
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if count < 0 {
        return Ok(()); // Can't check — assume not mounted.
    }

    let mut buf: Vec<libc::statfs> = Vec::with_capacity(count as usize);
    let buf_size = (count as usize) * std::mem::size_of::<libc::statfs>();

    // SAFETY: buf is properly sized, getfsstat fills it with statfs entries.
    let filled =
        unsafe { libc::getfsstat(buf.as_mut_ptr(), buf_size as libc::c_long, libc::MNT_NOWAIT) };
    if filled < 0 {
        return Ok(());
    }
    // SAFETY: getfsstat filled `filled` entries.
    unsafe { buf.set_len(filled as usize) };

    for entry in &buf {
        // SAFETY: f_mntfromname is a null-terminated C string.
        let from = unsafe { CStr::from_ptr(entry.f_mntfromname.as_ptr()) }.to_string_lossy();
        if from == dev_path {
            // SAFETY: f_mntonname is a null-terminated C string.
            let on = unsafe { CStr::from_ptr(entry.f_mntonname.as_ptr()) }.to_string_lossy();
            return Err(ZonedError::DeviceMounted {
                path: path.to_path_buf(),
                mount_point: on.to_string(),
            });
        }
    }

    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn is_not_mounted(_path: &Path) -> Result<()> {
    Ok(()) // No mount checking on unsupported platforms.
}

/// Check that the device has no partitions.
///
/// On Linux, checks `/sys/block/<device>/`. On FreeBSD, checks for
/// `/dev/<device>p*` and `/dev/<device>s*` partition device nodes.
#[cfg(target_os = "linux")]
pub fn has_no_partitions(path: &Path) -> Result<()> {
    let dev_name =
        path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| ZonedError::DeviceNotFound {
                path: path.to_path_buf(),
            })?;

    let sysfs_dir = format!("/sys/block/{dev_name}");
    if !Path::new(&sysfs_dir).exists() {
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

#[cfg(target_os = "freebsd")]
pub fn has_no_partitions(path: &Path) -> Result<()> {
    let dev_name =
        path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| ZonedError::DeviceNotFound {
                path: path.to_path_buf(),
            })?;

    // FreeBSD partitions appear as /dev/<dev>p1, /dev/<dev>s1, etc.
    let mut partitions = Vec::new();
    if let Ok(entries) = fs::read_dir("/dev") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Match <dev_name>p<N> or <dev_name>s<N>
            if let Some(suffix) = name_str.strip_prefix(dev_name)
                && (suffix.starts_with('p') || suffix.starts_with('s'))
                && suffix.len() > 1
                && suffix[1..].chars().all(|c| c.is_ascii_digit())
            {
                partitions.push(name_str.to_string());
            }
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

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn has_no_partitions(_path: &Path) -> Result<()> {
    Ok(())
}

/// Check that the device is a zoned block device.
///
/// On Linux, checks sysfs. On FreeBSD, issues `DIOCZONECMD GET_PARAMS`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn is_zoned_device(path: &Path) -> Result<()> {
    use crate::sysfs;
    use crate::types::DeviceModel;
    let model = sysfs::device_model(path)?;
    if model == DeviceModel::None {
        return Err(ZonedError::NotZoned {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn is_zoned_device(_path: &Path) -> Result<()> {
    Err(ZonedError::UnsupportedPlatform)
}

#[cfg(test)]
mod validate_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(non_snake_case)]

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
}
