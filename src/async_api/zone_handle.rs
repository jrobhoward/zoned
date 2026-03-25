use std::io::IoSlice;
use std::sync::Arc;

use crate::ZoneHandle;
use crate::error::Result;
use crate::types::{Sector, Zone, ZoneIndex};

/// Async wrapper around [`ZoneHandle`] for zone-scoped operations.
///
/// Uses `tokio::sync::Mutex` internally to safely dispatch write operations
/// to the blocking thread pool while maintaining exclusive access semantics.
///
/// # Example
///
/// ```no_run
/// use zoned::async_api::{AsyncZonedDevice, AsyncZoneHandle};
/// use zoned::ZoneIndex;
///
/// # async fn example() -> zoned::Result<()> {
/// let dev = AsyncZonedDevice::open_writable("/dev/sdb").await?;
/// let mut handle = dev.zone_handle(ZoneIndex::new(5)).await?;
///
/// handle.open().await?;
/// let written = handle.write_sequential(vec![0u8; 4096]).await?;
/// handle.reset().await?;
/// # Ok(())
/// # }
/// ```
pub struct AsyncZoneHandle {
    inner: Arc<tokio::sync::Mutex<ZoneHandle>>,
}

impl AsyncZoneHandle {
    /// Wrap a synchronous `ZoneHandle` in an async handle.
    pub fn new(handle: ZoneHandle) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(handle)),
        }
    }

    /// Write data sequentially at the current write pointer.
    pub async fn write_sequential(&self, buf: Vec<u8>) -> Result<usize> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut handle = inner.blocking_lock();
            handle.write_sequential(&buf)
        })
        .await
        .map_err(join_error)?
    }

    /// Write the entire buffer sequentially, looping on partial writes.
    pub async fn write_all_sequential(&self, buf: Vec<u8>) -> Result<()> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut handle = inner.blocking_lock();
            handle.write_all_sequential(&buf)
        })
        .await
        .map_err(join_error)?
    }

    /// Write scattered buffers sequentially (vectored write).
    pub async fn writev_sequential(&self, bufs: Vec<Vec<u8>>) -> Result<usize> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut handle = inner.blocking_lock();
            let slices: Vec<IoSlice<'_>> = bufs.iter().map(|b| IoSlice::new(b)).collect();
            handle.writev_sequential(&slices)
        })
        .await
        .map_err(join_error)?
    }

    /// Reset this zone's write pointer to the start.
    pub async fn reset(&self) -> Result<()> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut handle = inner.blocking_lock();
            handle.reset()
        })
        .await
        .map_err(join_error)?
    }

    /// Explicitly open this zone.
    pub async fn open(&self) -> Result<()> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let handle = inner.blocking_lock();
            handle.open()
        })
        .await
        .map_err(join_error)?
    }

    /// Close this zone.
    pub async fn close(&self) -> Result<()> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let handle = inner.blocking_lock();
            handle.close()
        })
        .await
        .map_err(join_error)?
    }

    /// Finish (mark as full) this zone.
    pub async fn finish(&self) -> Result<()> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut handle = inner.blocking_lock();
            handle.finish()
        })
        .await
        .map_err(join_error)?
    }

    /// Report the current state of this zone from the device.
    pub async fn report(&self) -> Result<Zone> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let handle = inner.blocking_lock();
            handle.report()
        })
        .await
        .map_err(join_error)?
    }

    /// Start sector of this zone.
    pub async fn start(&self) -> Sector {
        self.inner.lock().await.start()
    }

    /// Length of this zone in sectors.
    pub async fn len(&self) -> Sector {
        self.inner.lock().await.len()
    }

    /// Returns true if the zone has zero length.
    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }

    /// Usable capacity of this zone in sectors.
    pub async fn capacity(&self) -> Sector {
        self.inner.lock().await.capacity()
    }

    /// Current locally-tracked write pointer position (in sectors).
    pub async fn write_pointer(&self) -> Sector {
        self.inner.lock().await.write_pointer()
    }

    /// Zone index on the device.
    pub async fn zone_index(&self) -> ZoneIndex {
        self.inner.lock().await.zone_index()
    }
}

impl std::fmt::Debug for AsyncZoneHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncZoneHandle").finish_non_exhaustive()
    }
}

fn join_error(e: tokio::task::JoinError) -> crate::error::ZonedError {
    crate::error::ZonedError::Io {
        path: std::path::PathBuf::from("<async task>"),
        source: std::io::Error::other(e.to_string()),
    }
}
