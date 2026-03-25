use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::ZonedDevice;
use crate::types::{SECTOR_SIZE, Sector};

/// A cursor wrapping a [`ZonedDevice`] that implements [`Read`], [`Write`],
/// and [`Seek`].
///
/// The cursor tracks a byte position internally and translates standard I/O
/// trait calls into positional reads/writes on the underlying device. Multiple
/// cursors can exist for the same device simultaneously since the underlying
/// I/O uses `pread`/`pwrite`.
///
/// # Example
///
/// ```no_run
/// use std::io::{Read, Write, Seek, SeekFrom};
/// use zoned::{Sector, ZonedDevice};
///
/// let dev = ZonedDevice::open_writable("/dev/sdb")?;
/// let mut cursor = dev.cursor();
///
/// cursor.seek(SeekFrom::Start(4096))?;
/// cursor.write_all(&[0xAA; 512])?;
///
/// cursor.seek(SeekFrom::Start(4096))?;
/// let mut buf = [0u8; 512];
/// cursor.read_exact(&mut buf)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct ZonedDeviceCursor<'a> {
    device: &'a ZonedDevice,
    position: u64,
}

impl<'a> ZonedDeviceCursor<'a> {
    /// Create a new cursor at byte position 0.
    pub fn new(device: &'a ZonedDevice) -> Self {
        Self {
            device,
            position: 0,
        }
    }

    /// Create a new cursor positioned at the given sector.
    pub fn at_sector(device: &'a ZonedDevice, sector: Sector) -> Self {
        Self {
            device,
            position: sector.to_bytes(),
        }
    }

    /// Returns the current byte position of the cursor.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Returns the current position as a sector offset.
    ///
    /// Returns `None` if the position is not sector-aligned.
    pub fn sector_position(&self) -> Option<Sector> {
        Sector::from_bytes(self.position)
    }
}

impl Read for ZonedDeviceCursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let sector = Sector::from_bytes(self.position).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cursor position {} is not sector-aligned", self.position),
            )
        })?;
        let n = self.device.read_at(sector, buf).map_err(io::Error::other)?;
        self.position += n as u64;
        Ok(n)
    }
}

impl Write for ZonedDeviceCursor<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let sector = Sector::from_bytes(self.position).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cursor position {} is not sector-aligned", self.position),
            )
        })?;
        let n = self
            .device
            .write_at(sector, buf)
            .map_err(io::Error::other)?;
        self.position += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.device.fsync().map_err(io::Error::other)
    }
}

impl Seek for ZonedDeviceCursor<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                if offset >= 0 {
                    self.position.checked_add(offset as u64).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "seek overflow")
                    })?
                } else {
                    let abs = (-offset) as u64;
                    self.position.checked_sub(abs).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "seek before start of device")
                    })?
                }
            }
            SeekFrom::End(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "SeekFrom::End is not supported for block devices (unknown size at this layer)",
                ));
            }
        };

        // Validate sector alignment
        if new_pos % SECTOR_SIZE != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "seek position {new_pos} is not sector-aligned (must be multiple of {SECTOR_SIZE})"
                ),
            ));
        }

        self.position = new_pos;
        Ok(self.position)
    }
}

impl std::fmt::Debug for ZonedDeviceCursor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZonedDeviceCursor")
            .field("device", &self.device.path())
            .field("position", &self.position)
            .finish()
    }
}
