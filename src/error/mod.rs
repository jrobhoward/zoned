use std::path::PathBuf;

use crate::types::{Sector, ZoneIndex};

/// Errors that can occur when interacting with zoned block devices.
#[derive(Debug, thiserror::Error)]
pub enum ZonedError {
    /// The device path does not exist.
    #[error("device not found: {path}")]
    DeviceNotFound {
        /// Path that was not found.
        path: PathBuf,
    },

    /// The device is not a zoned block device.
    #[error("not a zoned device: {path}")]
    NotZoned {
        /// Device path.
        path: PathBuf,
    },

    /// An I/O error occurred during a read or write operation.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// Device or file path.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// An ioctl call failed.
    #[error("ioctl failed on {path}: {source}")]
    Ioctl {
        /// Device path.
        path: PathBuf,
        /// Kernel errno.
        source: nix::errno::Errno,
    },

    /// Failed to read a sysfs attribute.
    #[error("failed to read sysfs attribute {attribute} for {path}: {source}")]
    Sysfs {
        /// Device path.
        path: PathBuf,
        /// Sysfs attribute name.
        attribute: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// Failed to parse a sysfs attribute value.
    #[error("failed to parse sysfs attribute {attribute} for {path}: {value:?}")]
    SysfsParse {
        /// Device path.
        path: PathBuf,
        /// Sysfs attribute name.
        attribute: String,
        /// Raw value that could not be parsed.
        value: String,
    },

    /// A sector range was invalid (zero length or overflow).
    #[error("invalid zone range: sector {sector}, count {nr_sectors}")]
    InvalidRange {
        /// Starting sector.
        sector: Sector,
        /// Number of sectors.
        nr_sectors: Sector,
    },

    /// Write attempted on a device opened read-only.
    #[error("device opened read-only, write access required: {path}")]
    ReadOnly {
        /// Device path.
        path: PathBuf,
    },

    /// The zone is already allocated by a `ZoneAllocator`.
    #[error("zone {zone_index} is already allocated")]
    ZoneAlreadyAllocated {
        /// Zone index.
        zone_index: ZoneIndex,
    },

    /// The zone is not currently allocated.
    #[error("zone {zone_index} is not allocated")]
    ZoneNotAllocated {
        /// Zone index.
        zone_index: ZoneIndex,
    },

    /// The zone's write pointer has reached its capacity.
    #[error("zone {zone_index} is full")]
    ZoneFull {
        /// Zone index.
        zone_index: ZoneIndex,
    },

    /// The path does not refer to a block (Linux) or character (FreeBSD) device.
    #[error("not a block device: {path} (mode {mode:#o})")]
    NotABlockDevice {
        /// Path checked.
        path: PathBuf,
        /// File mode bits.
        mode: u32,
    },

    /// The device is currently mounted.
    #[error("{path} is mounted at {mount_point}")]
    DeviceMounted {
        /// Device path.
        path: PathBuf,
        /// Mount point.
        mount_point: String,
    },

    /// The device has partitions.
    #[error("{path} has partitions: {}", partitions.join(", "))]
    DeviceHasPartitions {
        /// Device path.
        path: PathBuf,
        /// Partition device names.
        partitions: Vec<String>,
    },

    /// The current platform does not support zoned block device operations.
    #[error("platform not supported for zoned block device operations")]
    UnsupportedPlatform,
}

/// Alias for `std::result::Result<T, ZonedError>`.
pub type Result<T> = std::result::Result<T, ZonedError>;

#[cfg(test)]
mod error_tests;
