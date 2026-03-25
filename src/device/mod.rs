use std::path::Path;

use crate::error::{Result, ZonedError};
use crate::platform::PlatformDevice;
use crate::types::{DeviceInfo, SECTOR_SIZE, Zone};

/// Handle to an open zoned block device.
///
/// Provides a safe interface for querying and managing zones on SMR (Shingled
/// Magnetic Recording) and ZNS (Zoned Namespace) devices.
///
/// # Example
///
/// ```no_run
/// use zoned::ZonedDevice;
///
/// let dev = ZonedDevice::open("/dev/sdb")?;
/// let info = dev.device_info()?;
/// println!("Zone size: {} sectors, {} zones", info.zone_size, info.nr_zones);
///
/// let zones = dev.report_zones(0, 16)?;
/// for zone in &zones {
///     println!("{:?} at sector {}", zone.condition, zone.start);
/// }
/// # Ok::<(), zoned::ZonedError>(())
/// ```
pub struct ZonedDevice {
    inner: PlatformDevice,
}

impl ZonedDevice {
    /// Open a zoned block device by path.
    ///
    /// The path should be a block device node (e.g. `/dev/sdb` on Linux).
    /// Returns an error if the device does not exist or cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let inner = PlatformDevice::open(path.as_ref())?;
        Ok(Self { inner })
    }

    /// Get basic device information: zone size and zone count.
    ///
    /// Uses kernel ioctls (`BLKGETZONESZ` / `BLKGETNRZONES` on Linux).
    /// Returns `NotZoned` if the device reports a zone size of zero.
    pub fn device_info(&self) -> Result<DeviceInfo> {
        self.inner.device_info()
    }

    /// Report zones starting from the given sector.
    ///
    /// Returns up to `max_zones` zone descriptors starting at or after `sector`.
    /// If `max_zones` is 0, it is treated as 1.
    ///
    /// The returned zones are ordered by start sector.
    pub fn report_zones(&self, sector: u64, max_zones: u32) -> Result<Vec<Zone>> {
        self.inner.report_zones(sector, max_zones)
    }

    /// Report all zones on the device.
    ///
    /// Iteratively queries zones starting from sector 0 until all zones have
    /// been reported. Uses batches of `batch_size` zones per ioctl call.
    pub fn report_all_zones(&self, batch_size: u32) -> Result<Vec<Zone>> {
        let info = self.device_info()?;
        let batch = if batch_size == 0 { 512 } else { batch_size };
        let mut all_zones = Vec::with_capacity(info.nr_zones as usize);
        let mut sector = 0u64;

        loop {
            let zones = self.report_zones(sector, batch)?;
            if zones.is_empty() {
                break;
            }

            let last_zone = &zones[zones.len() - 1];
            let next_sector = last_zone.start + last_zone.len;
            all_zones.extend(zones);

            if all_zones.len() >= info.nr_zones as usize || next_sector <= sector {
                break;
            }
            sector = next_sector;
        }

        Ok(all_zones)
    }

    /// Reset write pointers for zones in the given sector range.
    ///
    /// All sequential zones overlapping the range `[sector, sector + nr_sectors)`
    /// will have their write pointers reset to the zone start. Data in those
    /// zones becomes inaccessible.
    ///
    /// To reset all zones, pass `sector = 0` and `nr_sectors` covering the
    /// entire device.
    pub fn reset_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        self.validate_range(sector, nr_sectors)?;
        self.inner.reset_zones(sector, nr_sectors)
    }

    /// Explicitly open zones in the given sector range.
    ///
    /// Opening a zone transitions it to the explicitly-open state. The device
    /// may have a limit on the number of simultaneously open zones
    /// (see `DeviceProperties::max_open_zones`).
    pub fn open_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        self.validate_range(sector, nr_sectors)?;
        self.inner.open_zones(sector, nr_sectors)
    }

    /// Close zones in the given sector range.
    ///
    /// Transitions open zones to the closed state, freeing open-zone resources
    /// on the device without resetting the write pointer.
    pub fn close_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        self.validate_range(sector, nr_sectors)?;
        self.inner.close_zones(sector, nr_sectors)
    }

    /// Finish (mark as full) zones in the given sector range.
    ///
    /// Transitions zones to the full state, advancing the write pointer to the
    /// end. No more writes are possible until the zone is reset.
    pub fn finish_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        self.validate_range(sector, nr_sectors)?;
        self.inner.finish_zones(sector, nr_sectors)
    }

    /// Open a zoned block device with read-write access.
    ///
    /// Required for zone management operations (reset, open, close, finish)
    /// and data I/O when not running as root. Read-only `open()` is sufficient
    /// for reporting operations.
    pub fn open_writable(path: impl AsRef<Path>) -> Result<Self> {
        let inner = PlatformDevice::open_writable(path.as_ref())?;
        Ok(Self { inner })
    }

    /// Open a zoned block device with read-write access and `O_DIRECT`.
    ///
    /// Bypasses the kernel page cache — writes go directly to the device.
    /// Required for accurate I/O benchmarking. Write buffers must be
    /// aligned to the device's logical block size (typically 4096 bytes).
    pub fn open_direct(path: impl AsRef<Path>) -> Result<Self> {
        let inner = PlatformDevice::open_direct(path.as_ref())?;
        Ok(Self { inner })
    }

    /// Returns true if the device was opened with write access.
    pub fn is_writable(&self) -> bool {
        self.inner.is_writable()
    }

    /// Write data at a sector offset.
    ///
    /// Uses `pwrite()` internally — does not depend on file position, safe for
    /// concurrent use from multiple threads (on different sector ranges).
    ///
    /// Requires the device to be opened with `open_writable()`.
    /// Returns `ReadOnly` error if opened read-only.
    pub fn write_at(&self, sector_offset: u64, buf: &[u8]) -> Result<usize> {
        let byte_offset =
            sector_offset
                .checked_mul(SECTOR_SIZE)
                .ok_or(ZonedError::InvalidRange {
                    sector: sector_offset,
                    nr_sectors: 0,
                })?;
        self.inner.write_at(buf, byte_offset)
    }

    /// Read data at a sector offset.
    ///
    /// Uses `pread()` internally — does not depend on file position, safe for
    /// concurrent use from multiple threads.
    ///
    /// Works with both read-only and writable device handles.
    pub fn read_at(&self, sector_offset: u64, buf: &mut [u8]) -> Result<usize> {
        let byte_offset =
            sector_offset
                .checked_mul(SECTOR_SIZE)
                .ok_or(ZonedError::InvalidRange {
                    sector: sector_offset,
                    nr_sectors: 0,
                })?;
        self.inner.read_at(buf, byte_offset)
    }

    /// Flush all pending writes to the device.
    ///
    /// Ensures all data written via `write_at` or `write_sequential` (through
    /// `ZoneHandle`) has been committed to the physical device. Blocks until
    /// the flush completes.
    pub fn fsync(&self) -> Result<()> {
        self.inner.fsync()
    }

    /// Return the path this device was opened with.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    fn validate_range(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        if nr_sectors == 0 {
            return Err(ZonedError::InvalidRange { sector, nr_sectors });
        }
        // Check for overflow
        if sector.checked_add(nr_sectors).is_none() {
            return Err(ZonedError::InvalidRange { sector, nr_sectors });
        }
        Ok(())
    }
}

#[cfg(test)]
mod device_tests;

impl std::fmt::Debug for ZonedDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZonedDevice")
            .field("path", &self.inner.path())
            .finish()
    }
}
