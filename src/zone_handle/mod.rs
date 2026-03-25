use std::sync::Arc;

use crate::ZonedDevice;
use crate::error::{Result, ZonedError};
use crate::types::{SECTOR_SIZE, Sector, Zone, ZoneIndex};
use crate::zone_allocator::AllocatorInner;

/// Exclusive handle to a single zone on a zoned block device.
///
/// A `ZoneHandle` provides zone-scoped operations with a locally-tracked write
/// pointer. It is **not `Clone`**, enforcing at compile time that only one owner
/// can write to a given zone.
///
/// Methods that advance the write pointer (`write_sequential`, `reset`, `finish`)
/// take `&mut self`, preventing concurrent writes to the same zone.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use zoned::{ZonedDevice, ZoneHandle, ZoneIndex};
///
/// let dev = Arc::new(ZonedDevice::open_writable("/dev/sdb")?);
/// let mut handle = ZoneHandle::new(dev, ZoneIndex::new(5))?;
///
/// handle.open()?;
/// let written = handle.write_sequential(&[0u8; 4096])?;
/// println!("Wrote {} bytes, wp now at sector {}", written, handle.write_pointer());
/// handle.reset()?;
/// # Ok::<(), zoned::ZonedError>(())
/// ```
pub struct ZoneHandle {
    device: Arc<ZonedDevice>,
    zone_index: ZoneIndex,
    start: Sector,
    len: Sector,
    capacity: Sector,
    write_pointer: Sector,
    allocator: Option<Arc<AllocatorInner>>,
}

impl ZoneHandle {
    /// Create a handle for a specific zone by index.
    ///
    /// Queries the device to populate zone metadata (start, length, capacity,
    /// current write pointer). Does not register with any allocator — the
    /// caller is responsible for ensuring exclusivity.
    pub fn new(device: Arc<ZonedDevice>, zone_index: ZoneIndex) -> Result<Self> {
        Self::new_inner(device, zone_index, None)
    }

    /// Create a handle registered with a zone allocator.
    ///
    /// The zone will be released back to the allocator when this handle is dropped.
    pub(crate) fn new_with_allocator(
        device: Arc<ZonedDevice>,
        zone_index: ZoneIndex,
        allocator: Arc<AllocatorInner>,
    ) -> Result<Self> {
        Self::new_inner(device, zone_index, Some(allocator))
    }

    fn new_inner(
        device: Arc<ZonedDevice>,
        zone_index: ZoneIndex,
        allocator: Option<Arc<AllocatorInner>>,
    ) -> Result<Self> {
        let info = device.device_info()?;
        let start = info.zone_size * zone_index.0 as u64;
        let zones = device.report_zones(start, 1)?;

        if zones.is_empty() {
            return Err(ZonedError::InvalidRange {
                sector: start,
                nr_sectors: Sector::ZERO,
            });
        }

        let zone = &zones[0];

        Ok(Self {
            device,
            zone_index,
            start: zone.start,
            len: zone.len,
            capacity: zone.capacity,
            write_pointer: zone.write_pointer.unwrap_or(zone.start),
            allocator,
        })
    }

    /// Write data sequentially at the current write pointer.
    ///
    /// The buffer should be aligned to the device's sector size. The write
    /// pointer advances by the number of bytes written (converted to sectors).
    ///
    /// This may perform a **partial write**, returning fewer bytes than
    /// `buf.len()`. Use [`write_all_sequential`](Self::write_all_sequential)
    /// to guarantee the entire buffer is written.
    ///
    /// Returns `ZoneFull` if the write pointer has reached the zone's capacity.
    /// Returns `ReadOnly` if the device was not opened with write access.
    pub fn write_sequential(&mut self, buf: &[u8]) -> Result<usize> {
        let capacity_end = self.start + self.capacity;
        if self.write_pointer >= capacity_end {
            return Err(ZonedError::ZoneFull {
                zone_index: self.zone_index,
            });
        }

        let written = self.device.write_at(self.write_pointer, buf)?;
        let sectors_written = Sector(written as u64 / SECTOR_SIZE);
        self.write_pointer += sectors_written;
        Ok(written)
    }

    /// Write scattered buffers sequentially at the current write pointer.
    ///
    /// Uses `pwritev()` internally for a single gather-write I/O operation.
    /// The write pointer advances by the total number of bytes written.
    ///
    /// Returns `ZoneFull` if the write pointer has reached the zone's capacity.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::io::IoSlice;
    /// use std::sync::Arc;
    /// use zoned::{ZonedDevice, ZoneHandle, ZoneIndex};
    ///
    /// let dev = Arc::new(ZonedDevice::open_writable("/dev/sdb")?);
    /// let mut handle = ZoneHandle::new(dev, ZoneIndex::new(5))?;
    /// let header = [0xAAu8; 512];
    /// let payload = [0xBBu8; 4096];
    /// let bufs = [IoSlice::new(&header), IoSlice::new(&payload)];
    /// let written = handle.writev_sequential(&bufs)?;
    /// # Ok::<(), zoned::ZonedError>(())
    /// ```
    pub fn writev_sequential(&mut self, bufs: &[std::io::IoSlice<'_>]) -> Result<usize> {
        let capacity_end = self.start + self.capacity;
        if self.write_pointer >= capacity_end {
            return Err(ZonedError::ZoneFull {
                zone_index: self.zone_index,
            });
        }

        let written = self.device.writev_at(self.write_pointer, bufs)?;
        let sectors_written = Sector(written as u64 / SECTOR_SIZE);
        self.write_pointer += sectors_written;
        Ok(written)
    }

    /// Write the entire buffer sequentially, looping on partial writes.
    ///
    /// Unlike [`write_sequential`](Self::write_sequential), this method
    /// guarantees that all bytes in `buf` are written before returning.
    ///
    /// Returns `ZoneFull` if the zone cannot accommodate the full buffer.
    /// Returns `ReadOnly` if the device was not opened with write access.
    pub fn write_all_sequential(&mut self, buf: &[u8]) -> Result<()> {
        let mut remaining = buf;
        while !remaining.is_empty() {
            let written = self.write_sequential(remaining)?;
            if written == 0 {
                return Err(ZonedError::ZoneFull {
                    zone_index: self.zone_index,
                });
            }
            remaining = &remaining[written..];
        }
        Ok(())
    }

    /// Reset this zone's write pointer to the start.
    ///
    /// Data in the zone becomes inaccessible.
    pub fn reset(&mut self) -> Result<()> {
        self.device.reset_zones(self.start, self.len)?;
        self.write_pointer = self.start;
        Ok(())
    }

    /// Explicitly open this zone.
    ///
    /// Transitions the zone to the explicitly-open state.
    pub fn open(&self) -> Result<()> {
        self.device.open_zones(self.start, self.len)
    }

    /// Close this zone.
    ///
    /// Transitions to the closed state without resetting the write pointer.
    pub fn close(&self) -> Result<()> {
        self.device.close_zones(self.start, self.len)
    }

    /// Finish (mark as full) this zone.
    ///
    /// Advances the write pointer to the end. No more writes are possible
    /// until the zone is reset.
    pub fn finish(&mut self) -> Result<()> {
        self.device.finish_zones(self.start, self.len)?;
        self.write_pointer = self.start + self.len;
        Ok(())
    }

    /// Report the current state of this zone from the device.
    ///
    /// This queries the device directly, not the locally-tracked state.
    pub fn report(&self) -> Result<Zone> {
        let zones = self.device.report_zones(self.start, 1)?;
        if zones.is_empty() {
            return Err(ZonedError::InvalidRange {
                sector: self.start,
                nr_sectors: Sector::ZERO,
            });
        }
        Ok(zones[0].clone())
    }

    /// Start sector of this zone.
    pub fn start(&self) -> Sector {
        self.start
    }

    /// Length of this zone in sectors.
    pub fn len(&self) -> Sector {
        self.len
    }

    /// Returns true if the zone has zero length. Always false for valid zones.
    pub fn is_empty(&self) -> bool {
        self.len.0 == 0
    }

    /// Usable capacity of this zone in sectors.
    pub fn capacity(&self) -> Sector {
        self.capacity
    }

    /// Current locally-tracked write pointer position (in sectors).
    pub fn write_pointer(&self) -> Sector {
        self.write_pointer
    }

    /// Zone index on the device.
    pub fn zone_index(&self) -> ZoneIndex {
        self.zone_index
    }
}

impl std::io::Write for ZoneHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write_sequential(buf).map_err(std::io::Error::other)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.device.fsync().map_err(std::io::Error::other)
    }
}

impl Drop for ZoneHandle {
    fn drop(&mut self) {
        if let Some(ref allocator) = self.allocator {
            allocator.release(self.zone_index);
        }
    }
}

impl std::fmt::Debug for ZoneHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneHandle")
            .field("zone_index", &self.zone_index)
            .field("start", &self.start)
            .field("len", &self.len)
            .field("write_pointer", &self.write_pointer)
            .finish()
    }
}

// Compile-time assertion: ZoneHandle is Send (can be moved to another thread)
// but intentionally NOT Sync (mutable write pointer without interior mutability).
const _: () = {
    fn _assert_send<T: Send>() {}
    fn _assert() {
        _assert_send::<ZoneHandle>();
    }
};

#[cfg(test)]
mod zone_handle_tests;
