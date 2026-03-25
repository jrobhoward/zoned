use std::path::PathBuf;

use crate::types::{Sector, ZoneIndex};

/// Errors that can occur when interacting with zoned block devices.
#[derive(Debug, thiserror::Error)]
pub enum ZonedError {
    #[error("device not found: {path}")]
    DeviceNotFound { path: PathBuf },

    #[error("not a zoned device: {path}")]
    NotZoned { path: PathBuf },

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("ioctl failed on {path}: {source}")]
    Ioctl {
        path: PathBuf,
        source: nix::errno::Errno,
    },

    #[error("failed to read sysfs attribute {attribute} for {path}: {source}")]
    Sysfs {
        path: PathBuf,
        attribute: String,
        source: std::io::Error,
    },

    #[error("failed to parse sysfs attribute {attribute} for {path}: {value:?}")]
    SysfsParse {
        path: PathBuf,
        attribute: String,
        value: String,
    },

    #[error("invalid zone range: sector {sector}, count {nr_sectors}")]
    InvalidRange { sector: Sector, nr_sectors: Sector },

    #[error("device opened read-only, write access required: {path}")]
    ReadOnly { path: PathBuf },

    #[error("zone {zone_index} is already allocated")]
    ZoneAlreadyAllocated { zone_index: ZoneIndex },

    #[error("zone {zone_index} is not allocated")]
    ZoneNotAllocated { zone_index: ZoneIndex },

    #[error("zone {zone_index} is full")]
    ZoneFull { zone_index: ZoneIndex },

    #[error("not a block device: {path} (mode {mode:#o})")]
    NotABlockDevice { path: PathBuf, mode: u32 },

    #[error("{path} is mounted at {mount_point}")]
    DeviceMounted { path: PathBuf, mount_point: String },

    #[error("{path} has partitions: {}", partitions.join(", "))]
    DeviceHasPartitions {
        path: PathBuf,
        partitions: Vec<String>,
    },

    #[error("platform not supported for zoned block device operations")]
    UnsupportedPlatform,
}

pub type Result<T> = std::result::Result<T, ZonedError>;

#[cfg(test)]
mod error_tests;
