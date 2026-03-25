use std::fs;
use std::path::Path;

use crate::error::{Result, ZonedError};
use crate::types::{DeviceModel, DeviceProperties, Sector};

/// Read a sysfs queue attribute for a block device, returning the trimmed string value.
fn read_sysfs_queue_attr(device_name: &str, attribute: &str) -> Result<String> {
    let sysfs_path = format!("/sys/block/{device_name}/queue/{attribute}");
    fs::read_to_string(&sysfs_path)
        .map(|s| s.trim().to_string())
        .map_err(|e| ZonedError::Sysfs {
            device: device_name.to_string(),
            attribute: attribute.to_string(),
            source: e,
        })
}

/// Read a sysfs block-level attribute (directly under /sys/block/<device>/).
fn read_sysfs_block_attr(device_name: &str, attribute: &str) -> Result<String> {
    let sysfs_path = format!("/sys/block/{device_name}/{attribute}");
    fs::read_to_string(&sysfs_path)
        .map(|s| s.trim().to_string())
        .map_err(|e| ZonedError::Sysfs {
            device: device_name.to_string(),
            attribute: attribute.to_string(),
            source: e,
        })
}

/// Read a sysfs device attribute (/sys/block/<device>/device/<attribute>).
fn read_sysfs_device_attr(device_name: &str, attribute: &str) -> Result<String> {
    let sysfs_path = format!("/sys/block/{device_name}/device/{attribute}");
    fs::read_to_string(&sysfs_path)
        .map(|s| s.trim().to_string())
        .map_err(|e| ZonedError::Sysfs {
            device: device_name.to_string(),
            attribute: attribute.to_string(),
            source: e,
        })
}

/// Parse a sysfs attribute as a u32.
fn parse_u32_attr(device_name: &str, attribute: &str, value: &str) -> Result<u32> {
    value.parse::<u32>().map_err(|_| ZonedError::SysfsParse {
        device: device_name.to_string(),
        attribute: attribute.to_string(),
        value: value.to_string(),
    })
}

/// Parse a sysfs attribute as a u64.
fn parse_u64_attr(device_name: &str, attribute: &str, value: &str) -> Result<u64> {
    value.parse::<u64>().map_err(|_| ZonedError::SysfsParse {
        device: device_name.to_string(),
        attribute: attribute.to_string(),
        value: value.to_string(),
    })
}

/// Extract the device name (e.g. "sda") from a device path (e.g. "/dev/sda").
fn device_name_from_path(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ZonedError::DeviceNotFound {
            path: path.to_path_buf(),
        })
}

/// Parse the active scheduler from the sysfs `scheduler` attribute.
///
/// The format is: `noop [mq-deadline] none` — the active one is in brackets.
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
    let value = read_sysfs_queue_attr(&name, "zoned")?;
    Ok(match value.as_str() {
        "host-aware" => DeviceModel::HostAware,
        "host-managed" => DeviceModel::HostManaged,
        _ => DeviceModel::None,
    })
}

/// Read extended device properties from sysfs.
///
/// This reads all available zoned device attributes from
/// `/sys/block/<device>/queue/`. Available since various kernel versions
/// (4.10+ for basic attributes, 5.8+ for zone_append_max_bytes, 5.9+ for
/// max_open_zones and max_active_zones).
///
/// Optional attributes (block sizes, scheduler, vendor, model) default to
/// zero or `None` when the sysfs files are absent (e.g. on older kernels
/// or virtual devices).
pub fn device_properties(path: &Path) -> Result<DeviceProperties> {
    let name = device_name_from_path(path)?;

    let model = device_model(path)?;

    let chunk_str = read_sysfs_queue_attr(&name, "chunk_sectors")?;
    let chunk_sectors_raw = parse_u32_attr(&name, "chunk_sectors", &chunk_str)?;
    let chunk_sectors = Sector(chunk_sectors_raw as u64);

    let nr_str = read_sysfs_queue_attr(&name, "nr_zones")?;
    let nr_zones = parse_u32_attr(&name, "nr_zones", &nr_str)?;

    // These attributes may not exist on older kernels; default to 0.
    let zone_append_max_bytes = read_sysfs_queue_attr(&name, "zone_append_max_bytes")
        .and_then(|v| parse_u64_attr(&name, "zone_append_max_bytes", &v))
        .unwrap_or(0);

    let max_open_zones = read_sysfs_queue_attr(&name, "max_open_zones")
        .and_then(|v| parse_u32_attr(&name, "max_open_zones", &v))
        .unwrap_or(0);

    let max_active_zones = read_sysfs_queue_attr(&name, "max_active_zones")
        .and_then(|v| parse_u32_attr(&name, "max_active_zones", &v))
        .unwrap_or(0);

    // Block sizes — always present for any block device.
    let logical_block_size = read_sysfs_queue_attr(&name, "logical_block_size")
        .and_then(|v| parse_u32_attr(&name, "logical_block_size", &v))
        .unwrap_or(0);

    let physical_block_size = read_sysfs_queue_attr(&name, "physical_block_size")
        .and_then(|v| parse_u32_attr(&name, "physical_block_size", &v))
        .unwrap_or(0);

    // Max I/O sizes.
    let max_hw_sectors_kb = read_sysfs_queue_attr(&name, "max_hw_sectors_kb")
        .and_then(|v| parse_u32_attr(&name, "max_hw_sectors_kb", &v))
        .unwrap_or(0);

    let max_sectors_kb = read_sysfs_queue_attr(&name, "max_sectors_kb")
        .and_then(|v| parse_u32_attr(&name, "max_sectors_kb", &v))
        .unwrap_or(0);

    // Total capacity in 512-byte sectors (/sys/block/<dev>/size).
    let capacity_sectors = read_sysfs_block_attr(&name, "size")
        .and_then(|v| parse_u64_attr(&name, "size", &v))
        .map(Sector)
        .unwrap_or(Sector::ZERO);

    // I/O scheduler.
    let scheduler = read_sysfs_queue_attr(&name, "scheduler")
        .ok()
        .and_then(|v| parse_active_scheduler(&v));

    // Device identity (may not exist for virtual devices).
    let vendor = read_sysfs_device_attr(&name, "vendor").ok();
    let model_name = read_sysfs_device_attr(&name, "model").ok();

    Ok(DeviceProperties {
        model,
        chunk_sectors,
        nr_zones,
        zone_append_max_bytes,
        max_open_zones,
        max_active_zones,
        logical_block_size,
        physical_block_size,
        max_hw_sectors_kb,
        max_sectors_kb,
        capacity_sectors,
        scheduler,
        vendor,
        model_name,
    })
}
