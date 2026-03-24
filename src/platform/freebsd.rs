use std::path::{Path, PathBuf};

use crate::error::{Result, ZonedError};
use crate::types::{DeviceInfo, Zone};

/// FreeBSD zoned block device backend.
///
/// FreeBSD supports zoned devices through the GEOM BIO layer using `BIO_ZONE`
/// commands and CAM (Common Access Method) passthrough for SCSI ZBC / ATA ZAC
/// devices. The interface uses `struct disk_zone_args` from `sys/disk_zone.h`.
///
/// This backend is not yet implemented. Contributions welcome.
pub(crate) struct PlatformDevice {
    path: PathBuf,
}

impl PlatformDevice {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        // TODO: Open via CAM passthrough or GEOM BIO_ZONE interface
        let _ = path;
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn device_info(&self) -> Result<DeviceInfo> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn report_zones(&self, _sector: u64, _max_zones: u32) -> Result<Vec<Zone>> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn reset_zones(&self, _sector: u64, _nr_sectors: u64) -> Result<()> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn open_zones(&self, _sector: u64, _nr_sectors: u64) -> Result<()> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn close_zones(&self, _sector: u64, _nr_sectors: u64) -> Result<()> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn finish_zones(&self, _sector: u64, _nr_sectors: u64) -> Result<()> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn open_writable(path: &Path) -> Result<Self> {
        let _ = path;
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn is_writable(&self) -> bool {
        false
    }

    pub(crate) fn write_at(&self, _buf: &[u8], _byte_offset: u64) -> Result<usize> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn read_at(&self, _buf: &mut [u8], _byte_offset: u64) -> Result<usize> {
        Err(ZonedError::UnsupportedPlatform)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
