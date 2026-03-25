use std::path::{Path, PathBuf};

use crate::ZonedDevice;
use crate::error::Result;
use crate::validate;

/// Builder for opening a [`ZonedDevice`] with optional validation checks.
///
/// # Example
///
/// ```no_run
/// use zoned::ZonedDevice;
///
/// // Open with all safety checks
/// let dev = ZonedDevice::builder("/dev/sda")
///     .writable()
///     .validate_all()
///     .open()?;
///
/// // Open with specific checks only
/// let dev = ZonedDevice::builder("/dev/sda")
///     .direct_io()
///     .validate_block_device()
///     .validate_not_mounted()
///     .open()?;
/// # Ok::<(), zoned::ZonedError>(())
/// ```
pub struct DeviceBuilder {
    path: PathBuf,
    writable: bool,
    direct_io: bool,
    check_block_device: bool,
    check_not_mounted: bool,
    check_no_partitions: bool,
    check_is_zoned: bool,
}

impl DeviceBuilder {
    pub(crate) fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            writable: false,
            direct_io: false,
            check_block_device: false,
            check_not_mounted: false,
            check_no_partitions: false,
            check_is_zoned: false,
        }
    }

    /// Open the device with read-write access.
    pub fn writable(mut self) -> Self {
        self.writable = true;
        self
    }

    /// Open the device with `O_DIRECT` (implies writable).
    ///
    /// Bypasses the kernel page cache — write buffers must be aligned to
    /// the device's logical block size (typically 4096 bytes).
    pub fn direct_io(mut self) -> Self {
        self.direct_io = true;
        self.writable = true;
        self
    }

    /// Validate that the path is a block device before opening.
    pub fn validate_block_device(mut self) -> Self {
        self.check_block_device = true;
        self
    }

    /// Validate that the device is not mounted before opening.
    pub fn validate_not_mounted(mut self) -> Self {
        self.check_not_mounted = true;
        self
    }

    /// Validate that the device has no partitions before opening.
    pub fn validate_no_partitions(mut self) -> Self {
        self.check_no_partitions = true;
        self
    }

    /// Validate that the device is a zoned block device (via sysfs) before opening.
    pub fn validate_is_zoned(mut self) -> Self {
        self.check_is_zoned = true;
        self
    }

    /// Enable all validation checks (block device, not mounted, no partitions, is zoned).
    pub fn validate_all(mut self) -> Self {
        self.check_block_device = true;
        self.check_not_mounted = true;
        self.check_no_partitions = true;
        self.check_is_zoned = true;
        self
    }

    /// Open the device, running any configured validation checks first.
    ///
    /// Validation checks run in order: block device, not mounted, no partitions,
    /// is zoned. If any check fails, the device is not opened and the error
    /// is returned.
    pub fn open(self) -> Result<ZonedDevice> {
        if self.check_block_device {
            validate::is_block_device(&self.path)?;
        }
        if self.check_not_mounted {
            validate::is_not_mounted(&self.path)?;
        }
        if self.check_no_partitions {
            validate::has_no_partitions(&self.path)?;
        }
        if self.check_is_zoned {
            validate::is_zoned_device(&self.path)?;
        }

        if self.direct_io {
            ZonedDevice::open_direct(&self.path)
        } else if self.writable {
            ZonedDevice::open_writable(&self.path)
        } else {
            ZonedDevice::open(&self.path)
        }
    }
}
