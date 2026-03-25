use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ZonedDevice;
use crate::device::DeviceBuilder;
use crate::error::{Result, ZonedError};
use crate::types::{DeviceInfo, Sector, Zone, ZoneFilter};

use super::AsyncZoneHandle;

/// Async wrapper around [`ZonedDevice`].
///
/// All I/O and ioctl operations are dispatched to a blocking thread pool via
/// `tokio::task::spawn_blocking`.
///
/// # Example
///
/// ```no_run
/// use zoned::{Sector, async_api::AsyncZonedDevice};
///
/// # async fn example() -> zoned::Result<()> {
/// let dev = AsyncZonedDevice::open("/dev/sdb").await?;
/// let info = dev.device_info()?;
/// let zones = dev.report_zones(Sector::ZERO, 16).await?;
/// for zone in &zones {
///     println!("{:?} at sector {}", zone.condition, zone.start);
/// }
/// # Ok(())
/// # }
/// ```
pub struct AsyncZonedDevice {
    inner: Arc<ZonedDevice>,
}

impl AsyncZonedDevice {
    /// Wrap an existing `ZonedDevice` in an async handle.
    pub fn from_sync(device: ZonedDevice) -> Self {
        Self {
            inner: Arc::new(device),
        }
    }

    /// Wrap an existing `Arc<ZonedDevice>` in an async handle.
    pub fn from_arc(device: Arc<ZonedDevice>) -> Self {
        Self { inner: device }
    }

    /// Get a reference to the underlying sync device.
    pub fn inner(&self) -> &ZonedDevice {
        &self.inner
    }

    /// Get the `Arc<ZonedDevice>` for sharing with other async handles.
    pub fn inner_arc(&self) -> Arc<ZonedDevice> {
        self.inner.clone()
    }

    /// Open a zoned block device by path (read-only).
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let dev = spawn_blocking(move || ZonedDevice::open(&path)).await?;
        Ok(Self::from_sync(dev))
    }

    /// Open a zoned block device with read-write access.
    pub async fn open_writable(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let dev = spawn_blocking(move || ZonedDevice::open_writable(&path)).await?;
        Ok(Self::from_sync(dev))
    }

    /// Open a zoned block device with read-write access and `O_DIRECT`.
    pub async fn open_direct(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let dev = spawn_blocking(move || ZonedDevice::open_direct(&path)).await?;
        Ok(Self::from_sync(dev))
    }

    /// Create a builder for opening a device with optional validation.
    pub fn builder(path: impl AsRef<Path>) -> DeviceBuilder {
        ZonedDevice::builder(path)
    }

    /// Get basic device information (zone size and zone count).
    ///
    /// This is a fast ioctl — runs synchronously without blocking dispatch.
    pub fn device_info(&self) -> Result<DeviceInfo> {
        self.inner.device_info()
    }

    /// Return the path this device was opened with.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    /// Returns true if the device was opened with write access.
    pub fn is_writable(&self) -> bool {
        self.inner.is_writable()
    }

    /// Report zones starting from the given sector.
    pub async fn report_zones(&self, sector: Sector, max_zones: u32) -> Result<Vec<Zone>> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.report_zones(sector, max_zones)).await
    }

    /// Report all zones on the device.
    pub async fn report_all_zones(&self, batch_size: u32) -> Result<Vec<Zone>> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.report_all_zones(batch_size)).await
    }

    /// Report all zones matching a filter.
    pub async fn report_zones_filtered(
        &self,
        filter: ZoneFilter,
        batch_size: u32,
    ) -> Result<Vec<Zone>> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.report_zones_filtered(&filter, batch_size)).await
    }

    /// Write data at a sector offset.
    pub async fn write_at(&self, sector_offset: Sector, buf: Vec<u8>) -> Result<usize> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.write_at(sector_offset, &buf)).await
    }

    /// Write the entire buffer at a sector offset.
    pub async fn write_all_at(&self, sector_offset: Sector, buf: Vec<u8>) -> Result<()> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.write_all_at(sector_offset, &buf)).await
    }

    /// Read data at a sector offset.
    pub async fn read_at(&self, sector_offset: Sector, len: usize) -> Result<Vec<u8>> {
        let dev = self.inner.clone();
        spawn_blocking(move || {
            let mut buf = vec![0u8; len];
            let n = dev.read_at(sector_offset, &mut buf)?;
            buf.truncate(n);
            Ok(buf)
        })
        .await
    }

    /// Reset write pointers for zones in the given sector range.
    pub async fn reset_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.reset_zones(sector, nr_sectors)).await
    }

    /// Explicitly open zones in the given sector range.
    pub async fn open_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.open_zones(sector, nr_sectors)).await
    }

    /// Close zones in the given sector range.
    pub async fn close_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.close_zones(sector, nr_sectors)).await
    }

    /// Finish (mark as full) zones in the given sector range.
    pub async fn finish_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.finish_zones(sector, nr_sectors)).await
    }

    /// Flush all pending writes to the device.
    pub async fn fsync(&self) -> Result<()> {
        let dev = self.inner.clone();
        spawn_blocking(move || dev.fsync()).await
    }

    /// Create an async zone handle for a specific zone.
    pub async fn zone_handle(
        &self,
        zone_index: crate::types::ZoneIndex,
    ) -> Result<AsyncZoneHandle> {
        let dev = self.inner.clone();
        let handle = spawn_blocking(move || crate::ZoneHandle::new(dev, zone_index)).await?;
        Ok(AsyncZoneHandle::new(handle))
    }
}

impl std::fmt::Debug for AsyncZonedDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncZonedDevice")
            .field("path", &self.inner.path())
            .finish()
    }
}

/// Run a blocking closure on the Tokio blocking thread pool.
///
/// Converts `JoinError` (task panic) into `ZonedError::Io`.
async fn spawn_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ZonedError::Io {
            path: PathBuf::from("<async task>"),
            source: std::io::Error::other(e.to_string()),
        })?
}
