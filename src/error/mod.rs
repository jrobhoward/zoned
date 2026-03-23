use std::path::PathBuf;

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

    #[error("failed to read sysfs attribute {attribute} for {device}: {source}")]
    Sysfs {
        device: String,
        attribute: String,
        source: std::io::Error,
    },

    #[error("failed to parse sysfs attribute {attribute} for {device}: {value:?}")]
    SysfsParse {
        device: String,
        attribute: String,
        value: String,
    },

    #[error("invalid zone range: sector {sector}, count {nr_sectors}")]
    InvalidRange { sector: u64, nr_sectors: u64 },

    #[error("platform not supported for zoned block device operations")]
    UnsupportedPlatform,
}

pub type Result<T> = std::result::Result<T, ZonedError>;

#[cfg(test)]
mod error_tests;
