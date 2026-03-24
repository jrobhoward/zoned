use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use crate::error::{Result, ZonedError};
use crate::types::{DeviceInfo, Zone, ZoneCondition, ZoneType};

// Linux ioctl magic number for block devices
const BLK_IOCTL_MAGIC: u8 = 0x12;

// ioctl command numbers
const BLKREPORTZONE_NR: u8 = 130;
const BLKRESETZONE_NR: u8 = 131;
const BLKGETZONESZ_NR: u8 = 132;
const BLKGETNRZONES_NR: u8 = 133;
const BLKOPENZONE_NR: u8 = 134;
const BLKCLOSEZONE_NR: u8 = 135;
const BLKFINISHZONE_NR: u8 = 136;

// Kernel struct matching `struct blk_zone` from linux/blkzoned.h (64 bytes)
#[repr(C)]
#[derive(Clone)]
struct BlkZone {
    start: u64,
    len: u64,
    wp: u64,
    zone_type: u8,
    cond: u8,
    non_seq: u8,
    reset: u8,
    resv: [u8; 4],
    capacity: u64,
    reserved: [u8; 24],
}

// Kernel struct matching `struct blk_zone_report` header
#[repr(C)]
struct BlkZoneReportHeader {
    sector: u64,
    nr_zones: u32,
    flags: u32,
}

// Kernel struct matching `struct blk_zone_range`
#[repr(C)]
struct BlkZoneRange {
    sector: u64,
    nr_sectors: u64,
}

// blk_zone_report_flags
const BLK_ZONE_REP_CAPACITY: u32 = 1 << 0;

// blk_zone_type values (from linux/blkzoned.h)
const BLK_ZONE_TYPE_SEQWRITE_REQ: u8 = 0x2;
const BLK_ZONE_TYPE_SEQWRITE_PREF: u8 = 0x3;

// blk_zone_cond values (from linux/blkzoned.h)
const BLK_ZONE_COND_EMPTY: u8 = 0x1;
const BLK_ZONE_COND_IMP_OPEN: u8 = 0x2;
const BLK_ZONE_COND_EXP_OPEN: u8 = 0x3;
const BLK_ZONE_COND_CLOSED: u8 = 0x4;
const BLK_ZONE_COND_READONLY: u8 = 0xD;
const BLK_ZONE_COND_FULL: u8 = 0xE;
const BLK_ZONE_COND_OFFLINE: u8 = 0xF;

// Generate ioctl request codes using nix macros
nix::ioctl_read!(blk_get_zone_sz, BLK_IOCTL_MAGIC, BLKGETZONESZ_NR, u32);
nix::ioctl_read!(blk_get_nr_zones, BLK_IOCTL_MAGIC, BLKGETNRZONES_NR, u32);
nix::ioctl_readwrite_buf!(blk_report_zones, BLK_IOCTL_MAGIC, BLKREPORTZONE_NR, u8);
nix::ioctl_write_ptr!(
    blk_reset_zones,
    BLK_IOCTL_MAGIC,
    BLKRESETZONE_NR,
    BlkZoneRange
);
nix::ioctl_write_ptr!(
    blk_open_zones,
    BLK_IOCTL_MAGIC,
    BLKOPENZONE_NR,
    BlkZoneRange
);
nix::ioctl_write_ptr!(
    blk_close_zones,
    BLK_IOCTL_MAGIC,
    BLKCLOSEZONE_NR,
    BlkZoneRange
);
nix::ioctl_write_ptr!(
    blk_finish_zones,
    BLK_IOCTL_MAGIC,
    BLKFINISHZONE_NR,
    BlkZoneRange
);

fn parse_zone_type(raw: u8) -> ZoneType {
    match raw {
        BLK_ZONE_TYPE_SEQWRITE_REQ => ZoneType::SequentialWriteRequired,
        BLK_ZONE_TYPE_SEQWRITE_PREF => ZoneType::SequentialWritePreferred,
        _ => ZoneType::Conventional,
    }
}

fn parse_zone_condition(raw: u8) -> ZoneCondition {
    match raw {
        BLK_ZONE_COND_EMPTY => ZoneCondition::Empty,
        BLK_ZONE_COND_IMP_OPEN => ZoneCondition::ImplicitlyOpen,
        BLK_ZONE_COND_EXP_OPEN => ZoneCondition::ExplicitlyOpen,
        BLK_ZONE_COND_CLOSED => ZoneCondition::Closed,
        BLK_ZONE_COND_READONLY => ZoneCondition::ReadOnly,
        BLK_ZONE_COND_FULL => ZoneCondition::Full,
        BLK_ZONE_COND_OFFLINE => ZoneCondition::Offline,
        _ => ZoneCondition::NotWritePointer,
    }
}

fn blk_zone_to_zone(bz: &BlkZone, has_capacity: bool) -> Zone {
    Zone {
        start: bz.start,
        len: bz.len,
        capacity: if has_capacity { bz.capacity } else { bz.len },
        write_pointer: bz.wp,
        zone_type: parse_zone_type(bz.zone_type),
        condition: parse_zone_condition(bz.cond),
        non_seq: bz.non_seq != 0,
        reset_recommended: bz.reset != 0,
    }
}

pub(crate) struct PlatformDevice {
    file: File,
    path: PathBuf,
    writable: bool,
}

impl PlatformDevice {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ZonedError::DeviceNotFound {
                    path: path.to_path_buf(),
                }
            } else {
                ZonedError::Io {
                    path: path.to_path_buf(),
                    source: e,
                }
            }
        })?;

        Ok(Self {
            file,
            path: path.to_path_buf(),
            writable: false,
        })
    }

    pub(crate) fn open_writable(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ZonedError::DeviceNotFound {
                        path: path.to_path_buf(),
                    }
                } else {
                    ZonedError::Io {
                        path: path.to_path_buf(),
                        source: e,
                    }
                }
            })?;

        Ok(Self {
            file,
            path: path.to_path_buf(),
            writable: true,
        })
    }

    pub(crate) fn is_writable(&self) -> bool {
        self.writable
    }

    pub(crate) fn write_at(&self, buf: &[u8], byte_offset: u64) -> Result<usize> {
        if !self.writable {
            return Err(ZonedError::ReadOnly {
                path: self.path.clone(),
            });
        }
        self.file
            .write_at(buf, byte_offset)
            .map_err(|e| ZonedError::Io {
                path: self.path.clone(),
                source: e,
            })
    }

    pub(crate) fn read_at(&self, buf: &mut [u8], byte_offset: u64) -> Result<usize> {
        self.file
            .read_at(buf, byte_offset)
            .map_err(|e| ZonedError::Io {
                path: self.path.clone(),
                source: e,
            })
    }

    pub(crate) fn device_info(&self) -> Result<DeviceInfo> {
        let mut zone_size: u32 = 0;
        let mut nr_zones: u32 = 0;

        // SAFETY: blk_get_zone_sz writes a u32 to the provided pointer.
        // The pointer is valid and properly aligned since it comes from a local variable.
        unsafe { blk_get_zone_sz(self.file.as_raw_fd(), &mut zone_size) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        // SAFETY: blk_get_nr_zones writes a u32 to the provided pointer.
        // The pointer is valid and properly aligned since it comes from a local variable.
        unsafe { blk_get_nr_zones(self.file.as_raw_fd(), &mut nr_zones) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        if zone_size == 0 {
            return Err(ZonedError::NotZoned {
                path: self.path.clone(),
            });
        }

        Ok(DeviceInfo {
            zone_size,
            nr_zones,
        })
    }

    pub(crate) fn report_zones(&self, sector: u64, max_zones: u32) -> Result<Vec<Zone>> {
        let zone_count = max_zones.max(1) as usize;

        // Allocate a buffer large enough for the header + zone_count BlkZone entries
        let header_size = size_of::<BlkZoneReportHeader>();
        let zone_entry_size = size_of::<BlkZone>();
        let buf_size = header_size + (zone_count * zone_entry_size);
        let mut buf = vec![0u8; buf_size];

        // Write the request header: sector to start from, max zones to return
        let header = BlkZoneReportHeader {
            sector,
            nr_zones: zone_count as u32,
            flags: 0,
        };

        // Copy header into the buffer
        // SAFETY: BlkZoneReportHeader is repr(C) with no padding concerns for these
        // fields. We copy exactly header_size bytes from a valid header into a buffer
        // that is at least header_size bytes long.
        unsafe {
            std::ptr::copy_nonoverlapping(
                &header as *const BlkZoneReportHeader as *const u8,
                buf.as_mut_ptr(),
                header_size,
            );
        }

        // SAFETY: blk_report_zones performs an IOWR ioctl that reads from and writes
        // to the provided buffer. The buffer is properly sized to hold the header plus
        // zone_count zone entries. The fd is a valid open file descriptor.
        unsafe { blk_report_zones(self.file.as_raw_fd(), &mut buf) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        // Parse the response header
        // SAFETY: We read back the header that the kernel wrote into our buffer.
        // The buffer is at least header_size bytes and properly aligned for u8 access.
        // We copy into a new BlkZoneReportHeader which is repr(C).
        let resp_header = unsafe {
            let mut h = std::mem::MaybeUninit::<BlkZoneReportHeader>::uninit();
            std::ptr::copy_nonoverlapping(buf.as_ptr(), h.as_mut_ptr() as *mut u8, header_size);
            h.assume_init()
        };

        let returned_zones = resp_header.nr_zones as usize;
        let has_capacity = (resp_header.flags & BLK_ZONE_REP_CAPACITY) != 0;

        let mut zones = Vec::with_capacity(returned_zones);
        for i in 0..returned_zones {
            let offset = header_size + (i * zone_entry_size);
            if offset + zone_entry_size > buf.len() {
                break;
            }

            // SAFETY: We read a BlkZone from the buffer at the computed offset.
            // The kernel wrote these entries contiguously after the header.
            // BlkZone is repr(C) and 64 bytes, and we verified the buffer bounds above.
            let bz = unsafe {
                let mut z = std::mem::MaybeUninit::<BlkZone>::uninit();
                std::ptr::copy_nonoverlapping(
                    buf.as_ptr().add(offset),
                    z.as_mut_ptr() as *mut u8,
                    zone_entry_size,
                );
                z.assume_init()
            };

            zones.push(blk_zone_to_zone(&bz, has_capacity));
        }

        Ok(zones)
    }

    pub(crate) fn reset_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        let range = BlkZoneRange { sector, nr_sectors };

        // SAFETY: blk_reset_zones performs an IOW ioctl that reads from the provided
        // BlkZoneRange pointer. The range is a valid local variable with proper alignment.
        unsafe { blk_reset_zones(self.file.as_raw_fd(), &range) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        Ok(())
    }

    pub(crate) fn open_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        let range = BlkZoneRange { sector, nr_sectors };

        // SAFETY: blk_open_zones performs an IOW ioctl that reads from the provided
        // BlkZoneRange pointer. The range is a valid local variable with proper alignment.
        unsafe { blk_open_zones(self.file.as_raw_fd(), &range) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        Ok(())
    }

    pub(crate) fn close_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        let range = BlkZoneRange { sector, nr_sectors };

        // SAFETY: blk_close_zones performs an IOW ioctl that reads from the provided
        // BlkZoneRange pointer. The range is a valid local variable with proper alignment.
        unsafe { blk_close_zones(self.file.as_raw_fd(), &range) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        Ok(())
    }

    pub(crate) fn finish_zones(&self, sector: u64, nr_sectors: u64) -> Result<()> {
        let range = BlkZoneRange { sector, nr_sectors };

        // SAFETY: blk_finish_zones performs an IOW ioctl that reads from the provided
        // BlkZoneRange pointer. The range is a valid local variable with proper alignment.
        unsafe { blk_finish_zones(self.file.as_raw_fd(), &range) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        Ok(())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
