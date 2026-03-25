//! Query zoned block device properties via sysfs.
//!
//! Linux exposes device attributes under `/sys/block/<device>/queue/`
//! (zone model, zone size, zone count, max open/active zones) and
//! `/sys/block/<device>/device/` (vendor, model name). These functions
//! read those attributes without opening the device.
//!
//! **Linux only.** On non-Linux platforms, these functions return
//! `UnsupportedPlatform` errors.

/// Query the device model from sysfs.
///
/// Returns `DeviceModel::None` if the device is not zoned.
///
/// **Linux only.** Returns `UnsupportedPlatform` on other platforms.
#[cfg(not(target_os = "linux"))]
pub fn device_model(_path: &std::path::Path) -> crate::error::Result<crate::types::DeviceModel> {
    Err(crate::error::ZonedError::UnsupportedPlatform)
}

/// Read extended device properties from sysfs.
///
/// **Linux only.** Returns `UnsupportedPlatform` on other platforms.
#[cfg(not(target_os = "linux"))]
pub fn device_properties(
    _path: &std::path::Path,
) -> crate::error::Result<crate::types::DeviceProperties> {
    Err(crate::error::ZonedError::UnsupportedPlatform)
}

// ============================================================
// Linux implementation
// ============================================================

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::path::Path;

    use crate::error::{Result, ZonedError};
    use crate::types::{
        BlockSizes, DeviceGeometry, DeviceIdentity, DeviceLimits, DeviceModel, DeviceProperties,
        Sector,
    };

    fn read_sysfs_queue_attr(path: &Path, device_name: &str, attribute: &str) -> Result<String> {
        let sysfs_path = format!("/sys/block/{device_name}/queue/{attribute}");
        fs::read_to_string(&sysfs_path)
            .map(|s| s.trim().to_string())
            .map_err(|e| ZonedError::Sysfs {
                path: path.to_path_buf(),
                attribute: attribute.to_string(),
                source: e,
            })
    }

    fn read_sysfs_block_attr(path: &Path, device_name: &str, attribute: &str) -> Result<String> {
        let sysfs_path = format!("/sys/block/{device_name}/{attribute}");
        fs::read_to_string(&sysfs_path)
            .map(|s| s.trim().to_string())
            .map_err(|e| ZonedError::Sysfs {
                path: path.to_path_buf(),
                attribute: attribute.to_string(),
                source: e,
            })
    }

    fn read_sysfs_device_attr(path: &Path, device_name: &str, attribute: &str) -> Result<String> {
        let sysfs_path = format!("/sys/block/{device_name}/device/{attribute}");
        fs::read_to_string(&sysfs_path)
            .map(|s| s.trim().to_string())
            .map_err(|e| ZonedError::Sysfs {
                path: path.to_path_buf(),
                attribute: attribute.to_string(),
                source: e,
            })
    }

    fn parse_u32_attr(path: &Path, attribute: &str, value: &str) -> Result<u32> {
        value.parse::<u32>().map_err(|_| ZonedError::SysfsParse {
            path: path.to_path_buf(),
            attribute: attribute.to_string(),
            value: value.to_string(),
        })
    }

    fn parse_u64_attr(path: &Path, attribute: &str, value: &str) -> Result<u64> {
        value.parse::<u64>().map_err(|_| ZonedError::SysfsParse {
            path: path.to_path_buf(),
            attribute: attribute.to_string(),
            value: value.to_string(),
        })
    }

    fn device_name_from_path(path: &Path) -> Result<String> {
        path.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ZonedError::DeviceNotFound {
                path: path.to_path_buf(),
            })
    }

    fn parse_active_scheduler(value: &str) -> Option<String> {
        let start = value.find('[')?;
        let end = value.find(']')?;
        if end > start + 1 {
            Some(value[start + 1..end].to_string())
        } else {
            None
        }
    }

    /// Query the device model from sysfs.
    ///
    /// Returns `DeviceModel::None` if the device is not zoned.
    pub fn device_model(path: &Path) -> Result<DeviceModel> {
        let name = device_name_from_path(path)?;
        let value = read_sysfs_queue_attr(path, &name, "zoned")?;
        Ok(match value.as_str() {
            "host-aware" => DeviceModel::HostAware,
            "host-managed" => DeviceModel::HostManaged,
            _ => DeviceModel::None,
        })
    }

    /// Read extended device properties from sysfs.
    pub fn device_properties(path: &Path) -> Result<DeviceProperties> {
        let name = device_name_from_path(path)?;

        let model = device_model(path)?;

        let chunk_str = read_sysfs_queue_attr(path, &name, "chunk_sectors")?;
        let chunk_sectors_raw = parse_u32_attr(path, "chunk_sectors", &chunk_str)?;
        let chunk_sectors = Sector(chunk_sectors_raw as u64);

        let nr_str = read_sysfs_queue_attr(path, &name, "nr_zones")?;
        let nr_zones = parse_u32_attr(path, "nr_zones", &nr_str)?;

        let zone_append_max_bytes = read_sysfs_queue_attr(path, &name, "zone_append_max_bytes")
            .and_then(|v| parse_u64_attr(path, "zone_append_max_bytes", &v))
            .unwrap_or(0);

        let max_open_zones = read_sysfs_queue_attr(path, &name, "max_open_zones")
            .and_then(|v| parse_u32_attr(path, "max_open_zones", &v))
            .ok()
            .filter(|&v| v > 0);

        let max_active_zones = read_sysfs_queue_attr(path, &name, "max_active_zones")
            .and_then(|v| parse_u32_attr(path, "max_active_zones", &v))
            .ok()
            .filter(|&v| v > 0);

        let logical_block_size = read_sysfs_queue_attr(path, &name, "logical_block_size")
            .and_then(|v| parse_u32_attr(path, "logical_block_size", &v))
            .unwrap_or(0);

        let physical_block_size = read_sysfs_queue_attr(path, &name, "physical_block_size")
            .and_then(|v| parse_u32_attr(path, "physical_block_size", &v))
            .unwrap_or(0);

        let max_hw_sectors_kb = read_sysfs_queue_attr(path, &name, "max_hw_sectors_kb")
            .and_then(|v| parse_u32_attr(path, "max_hw_sectors_kb", &v))
            .unwrap_or(0);

        let max_sectors_kb = read_sysfs_queue_attr(path, &name, "max_sectors_kb")
            .and_then(|v| parse_u32_attr(path, "max_sectors_kb", &v))
            .unwrap_or(0);

        let capacity_sectors = read_sysfs_block_attr(path, &name, "size")
            .and_then(|v| parse_u64_attr(path, "size", &v))
            .map(Sector)
            .unwrap_or(Sector::ZERO);

        let scheduler = read_sysfs_queue_attr(path, &name, "scheduler")
            .ok()
            .and_then(|v| parse_active_scheduler(&v));

        let vendor = read_sysfs_device_attr(path, &name, "vendor").ok();
        let model_name = read_sysfs_device_attr(path, &name, "model").ok();

        Ok(DeviceProperties {
            model,
            identity: DeviceIdentity { vendor, model_name },
            geometry: DeviceGeometry {
                chunk_sectors,
                nr_zones,
                capacity_sectors,
            },
            limits: DeviceLimits {
                zone_append_max_bytes,
                max_open_zones,
                max_active_zones,
                max_hw_sectors_kb,
                max_sectors_kb,
            },
            block_sizes: BlockSizes {
                logical_block_size,
                physical_block_size,
            },
            scheduler,
        })
    }
}

#[cfg(target_os = "linux")]
pub use linux::{device_model, device_properties};
