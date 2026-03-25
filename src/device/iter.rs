use crate::ZonedDevice;
use crate::error::Result;
use crate::types::{Sector, Zone};

/// Lazily iterates over all zones on a device, fetching in batches.
///
/// Created by [`ZonedDevice::zone_iter`]. Yields `Result<Zone>` items,
/// allowing callers to chain standard iterator adapters like `.filter()`.
///
/// # Example
///
/// ```no_run
/// use zoned::{ZonedDevice, ZoneType};
///
/// let dev = ZonedDevice::open("/dev/sdb")?;
/// let empty_seq: Vec<_> = dev.zone_iter(512)
///     .filter_map(|r| r.ok())
///     .filter(|z| z.zone_type == ZoneType::SequentialWriteRequired)
///     .filter(|z| z.condition == zoned::ZoneCondition::Empty)
///     .collect();
/// # Ok::<(), zoned::ZonedError>(())
/// ```
pub struct ZoneIterator<'a> {
    device: &'a ZonedDevice,
    batch_size: u32,
    /// Buffered zones from the last batch fetch.
    buffer: Vec<Zone>,
    /// Index into `buffer` for the next item to yield.
    buf_pos: usize,
    /// The sector to start the next batch fetch from.
    next_sector: Sector,
    /// True once we've exhausted all zones.
    done: bool,
}

impl<'a> ZoneIterator<'a> {
    pub(crate) fn new(device: &'a ZonedDevice, batch_size: u32) -> Self {
        Self {
            device,
            batch_size: if batch_size == 0 { 512 } else { batch_size },
            buffer: Vec::new(),
            buf_pos: 0,
            next_sector: Sector::ZERO,
            done: false,
        }
    }
}

impl<'a> Iterator for ZoneIterator<'a> {
    type Item = Result<Zone>;

    fn next(&mut self) -> Option<Self::Item> {
        // Yield from the buffer if we have items remaining.
        if self.buf_pos < self.buffer.len() {
            let zone = self.buffer[self.buf_pos].clone();
            self.buf_pos += 1;
            return Some(Ok(zone));
        }

        if self.done {
            return None;
        }

        // Fetch the next batch.
        let zones = match self.device.report_zones(self.next_sector, self.batch_size) {
            Ok(z) => z,
            Err(e) => {
                self.done = true;
                return Some(Err(e));
            }
        };

        if zones.is_empty() {
            self.done = true;
            return None;
        }

        let last = &zones[zones.len() - 1];
        let next = last.start + last.len;
        if next <= self.next_sector {
            // No progress — avoid infinite loop.
            self.done = true;
        }
        self.next_sector = next;

        self.buffer = zones;
        self.buf_pos = 1; // We'll return index 0 below.

        Some(Ok(self.buffer[0].clone()))
    }
}
