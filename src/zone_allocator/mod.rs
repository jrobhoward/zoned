use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::ZonedDevice;
use crate::error::{Result, ZonedError};
use crate::types::{Sector, ZoneIndex, ZoneType};
use crate::zone_handle::ZoneHandle;

/// Shared internal state for tracking allocated zones.
///
/// Used by both `ZoneAllocator` and `ZoneHandle` (via `Drop`).
pub(crate) struct AllocatorInner {
    allocated: Mutex<HashSet<ZoneIndex>>,
}

impl AllocatorInner {
    fn new() -> Self {
        Self {
            allocated: Mutex::new(HashSet::new()),
        }
    }

    /// Release a zone index back to the pool.
    ///
    /// Called by `ZoneHandle::drop`. Handles poisoned mutex gracefully
    /// since `Drop` must not panic.
    pub(crate) fn release(&self, zone_index: ZoneIndex) {
        if let Ok(mut set) = self.allocated.lock() {
            set.remove(&zone_index);
        }
    }

    fn try_allocate(&self, zone_index: ZoneIndex) -> Result<()> {
        let mut set = self.allocated.lock().unwrap_or_else(|e| e.into_inner());
        if set.contains(&zone_index) {
            return Err(ZonedError::ZoneAlreadyAllocated { zone_index });
        }
        set.insert(zone_index);
        Ok(())
    }
}

/// Manages exclusive zone ownership for safe concurrent access.
///
/// The allocator tracks which zones are currently checked out as `ZoneHandle`s.
/// When a `ZoneHandle` is dropped, its zone is automatically returned to the pool.
///
/// All methods take `&self` and use internal locking — the allocator is safe to
/// share across threads via `Arc<ZoneAllocator>`.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use zoned::{ZonedDevice, ZoneAllocator};
///
/// let dev = Arc::new(ZonedDevice::open_writable("/dev/sdb")?);
/// let allocator = ZoneAllocator::new(dev);
///
/// let mut zone_a = allocator.allocate()?;
/// let mut zone_b = allocator.allocate()?;
///
/// // Each handle can be sent to a different thread
/// // zone_a.write_sequential(&data)?;
/// # Ok::<(), zoned::ZonedError>(())
/// ```
pub struct ZoneAllocator {
    device: Arc<ZonedDevice>,
    inner: Arc<AllocatorInner>,
}

impl ZoneAllocator {
    /// Create a new zone allocator for the given device.
    pub fn new(device: Arc<ZonedDevice>) -> Self {
        Self {
            device,
            inner: Arc::new(AllocatorInner::new()),
        }
    }

    /// Allocate the next available empty sequential zone.
    ///
    /// Scans all zones on the device and returns a `ZoneHandle` for the first
    /// empty sequential-write-required zone that is not already allocated.
    ///
    /// The handle is automatically returned to the pool when dropped.
    pub fn allocate(&self) -> Result<ZoneHandle> {
        let zones = self.device.report_all_zones(512)?;
        let allocated = self
            .inner
            .allocated
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        for (i, zone) in zones.iter().enumerate() {
            let idx = ZoneIndex(i as u32);
            if zone.zone_type == ZoneType::SequentialWriteRequired
                && zone.condition == crate::types::ZoneCondition::Empty
                && !allocated.contains(&idx)
            {
                drop(allocated);
                return self.allocate_zone(idx);
            }
        }

        // No suitable zone found — report as invalid range (no empty zones available)
        Err(ZonedError::InvalidRange {
            sector: Sector::ZERO,
            nr_sectors: Sector::ZERO,
        })
    }

    /// Allocate a specific zone by index.
    ///
    /// Returns `ZoneAlreadyAllocated` if the zone is already checked out.
    pub fn allocate_zone(&self, zone_index: ZoneIndex) -> Result<ZoneHandle> {
        self.inner.try_allocate(zone_index)?;

        match ZoneHandle::new_with_allocator(self.device.clone(), zone_index, self.inner.clone()) {
            Ok(handle) => Ok(handle),
            Err(e) => {
                // Roll back the allocation on failure
                self.inner.release(zone_index);
                Err(e)
            }
        }
    }

    /// Explicitly release a zone index back to the pool.
    ///
    /// This is not normally needed — `ZoneHandle::drop` does this automatically.
    /// Returns `ZoneNotAllocated` if the zone was not tracked.
    pub fn release(&self, zone_index: ZoneIndex) -> Result<()> {
        let mut set = self
            .inner
            .allocated
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if set.remove(&zone_index) {
            Ok(())
        } else {
            Err(ZonedError::ZoneNotAllocated { zone_index })
        }
    }

    /// Returns the list of currently allocated zone indices.
    pub fn allocated_zones(&self) -> Vec<ZoneIndex> {
        let set = self
            .inner
            .allocated
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<ZoneIndex> = set.iter().copied().collect();
        v.sort_unstable();
        v
    }
}

#[cfg(test)]
mod zone_allocator_tests;
