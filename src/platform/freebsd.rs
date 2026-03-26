use std::fs::{File, OpenOptions};
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::error::{Result, ZonedError};
use crate::types::{
    BlockSizes, DeviceGeometry, DeviceIdentity, DeviceInfo, DeviceLimits, DeviceModel,
    DeviceProperties, Sector, Zone, ZoneCondition, ZoneType,
};

// ============================================================
// FreeBSD disk_zone.h structs (repr(C), verified against FreeBSD 15.0)
// ============================================================

// DIOCZONECMD ioctl number: _IOWR('d', 143, struct disk_zone_args)
// sizeof(disk_zone_args) = 128 on amd64
nix::ioctl_readwrite!(dioczonecmd, b'd', 143, DiskZoneArgs);

// DIOCGSECTORSIZE: _IOR('d', 128, u_int)
nix::ioctl_read!(diocgsectorsize, b'd', 128, u32);

// DIOCGMEDIASIZE: _IOR('d', 129, off_t)
nix::ioctl_read!(diocgmediasize, b'd', 129, i64);

// Zone commands (disk_zone_args.zone_cmd)
const DISK_ZONE_OPEN: u8 = 0x00;
const DISK_ZONE_CLOSE: u8 = 0x01;
const DISK_ZONE_FINISH: u8 = 0x02;
const DISK_ZONE_REPORT_ZONES: u8 = 0x03;
const DISK_ZONE_RWP: u8 = 0x04;
const DISK_ZONE_GET_PARAMS: u8 = 0x05;

// Zone modes (disk_zone_disk_params.zone_mode)
const DISK_ZONE_MODE_NONE: u32 = 0x00;
#[allow(dead_code)]
const DISK_ZONE_MODE_HOST_AWARE: u32 = 0x01;
#[allow(dead_code)]
const DISK_ZONE_MODE_DRIVE_MANAGED: u32 = 0x02;
#[allow(dead_code)]
const DISK_ZONE_MODE_HOST_MANAGED: u32 = 0x04;

// Zone types (disk_zone_rep_entry.zone_type)
const DISK_ZONE_TYPE_SEQ_REQUIRED: u8 = 0x02;
const DISK_ZONE_TYPE_SEQ_PREFERRED: u8 = 0x03;

// Zone conditions (disk_zone_rep_entry.zone_condition)
#[allow(dead_code)]
const DISK_ZONE_COND_NOT_WP: u8 = 0x00;
const DISK_ZONE_COND_EMPTY: u8 = 0x01;
const DISK_ZONE_COND_IMPLICIT_OPEN: u8 = 0x02;
const DISK_ZONE_COND_EXPLICIT_OPEN: u8 = 0x03;
const DISK_ZONE_COND_CLOSED: u8 = 0x04;
const DISK_ZONE_COND_READONLY: u8 = 0x0D;
const DISK_ZONE_COND_FULL: u8 = 0x0E;
const DISK_ZONE_COND_OFFLINE: u8 = 0x0F;

// Zone flags (disk_zone_rep_entry.zone_flags)
const DISK_ZONE_FLAG_RESET: u8 = 0x01;
const DISK_ZONE_FLAG_NON_SEQ: u8 = 0x02;

// RWP flags
const DISK_ZONE_RWP_FLAG_ALL: u8 = 0x01;

// Report options
const DISK_ZONE_REP_ALL: u8 = 0x00;

// Minimum entries to allocate for REPORT ZONES. FreeBSD's da driver builds
// the SCSI REPORT ZONES CDB allocation length from entries_allocated. Some
// devices (or QEMU SCSI passthrough) reject small allocation lengths with
// EIO. Requesting a large buffer and truncating avoids this.
const MIN_REPORT_ENTRIES: usize = 16384;

// ============================================================
// Kernel structs — layout verified with offsetof() on FreeBSD 15.0
// ============================================================

#[repr(C)]
#[derive(Clone)]
struct DiskZoneRepEntry {
    zone_type: u8,          // offset 0
    zone_condition: u8,     // offset 1
    zone_flags: u8,         // offset 2
    _pad0: [u8; 5],         // padding to offset 8
    zone_length: u64,       // offset 8
    zone_start_lba: u64,    // offset 16
    write_pointer_lba: u64, // offset 24
    _reserved: [u8; 32],    // offset 32..64
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DiskZoneRepHeader {
    same: u8,            // offset 0
    _pad0: [u8; 7],      // padding to offset 8
    maximum_lba: u64,    // offset 8
    _reserved: [u8; 64], // offset 16..80
}

#[repr(C)]
struct DiskZoneReport {
    starting_id: u64,               // offset 0
    rep_options: u8,                // offset 8
    _pad0: [u8; 7],                 // padding to offset 16
    header: DiskZoneRepHeader,      // offset 16 (80 bytes)
    entries_allocated: u32,         // offset 96
    entries_filled: u32,            // offset 100
    entries_available: u32,         // offset 104
    _pad1: [u8; 4],                 // padding to offset 112
    entries: *mut DiskZoneRepEntry, // offset 112 (pointer, 8 bytes)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DiskZoneRwp {
    id: u64,        // offset 0
    flags: u8,      // offset 8
    _pad0: [u8; 7], // padding to 16
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DiskZoneDiskParams {
    zone_mode: u32,            // offset 0
    _pad0: [u8; 4],            // padding to offset 8
    flags: u64,                // offset 8
    optimal_seq_zones: u64,    // offset 16
    optimal_nonseq_zones: u64, // offset 24
    max_seq_zones: u64,        // offset 32
}

// Union of all zone params — use the largest variant's size.
// sizeof(DiskZoneReport) = 120 (the largest), which is the union size.
#[repr(C)]
union DiskZoneParams {
    disk_params: DiskZoneDiskParams,
    rwp: DiskZoneRwp,
    report: std::mem::ManuallyDrop<DiskZoneReport>,
}

// sizeof = 128 (1 byte cmd + 7 padding + 120 byte union)
#[repr(C)]
struct DiskZoneArgs {
    zone_cmd: u8,                // offset 0
    _pad0: [u8; 7],              // padding to offset 8
    zone_params: DiskZoneParams, // offset 8
}

// ============================================================
// Parsing helpers
// ============================================================

fn parse_zone_type(raw: u8) -> ZoneType {
    match raw {
        DISK_ZONE_TYPE_SEQ_REQUIRED => ZoneType::SequentialWriteRequired,
        DISK_ZONE_TYPE_SEQ_PREFERRED => ZoneType::SequentialWritePreferred,
        _ => ZoneType::Conventional,
    }
}

fn parse_zone_condition(raw: u8) -> ZoneCondition {
    match raw {
        DISK_ZONE_COND_EMPTY => ZoneCondition::Empty,
        DISK_ZONE_COND_IMPLICIT_OPEN => ZoneCondition::ImplicitlyOpen,
        DISK_ZONE_COND_EXPLICIT_OPEN => ZoneCondition::ExplicitlyOpen,
        DISK_ZONE_COND_CLOSED => ZoneCondition::Closed,
        DISK_ZONE_COND_READONLY => ZoneCondition::ReadOnly,
        DISK_ZONE_COND_FULL => ZoneCondition::Full,
        DISK_ZONE_COND_OFFLINE => ZoneCondition::Offline,
        _ => ZoneCondition::NotWritePointer,
    }
}

/// Convert a zone report entry from device LBAs to 512-byte sectors.
fn entry_to_zone(entry: &DiskZoneRepEntry, lba_scale: u64) -> Zone {
    let condition = parse_zone_condition(entry.zone_condition);
    // FreeBSD returns 0xFFFFFFFFFFFFFFFF for zones without a meaningful
    // write pointer (conventional zones, and sometimes full zones).
    let write_pointer =
        if condition == ZoneCondition::NotWritePointer || entry.write_pointer_lba == u64::MAX {
            None
        } else {
            Some(Sector(entry.write_pointer_lba * lba_scale))
        };
    Zone {
        start: Sector(entry.zone_start_lba * lba_scale),
        len: Sector(entry.zone_length * lba_scale),
        // FreeBSD has no capacity field (predates NVMe ZNS) — capacity == length.
        capacity: Sector(entry.zone_length * lba_scale),
        write_pointer,
        zone_type: parse_zone_type(entry.zone_type),
        condition,
        non_seq: (entry.zone_flags & DISK_ZONE_FLAG_NON_SEQ) != 0,
        reset_recommended: (entry.zone_flags & DISK_ZONE_FLAG_RESET) != 0,
    }
}

// ============================================================
// PlatformDevice
// ============================================================

pub(crate) struct PlatformDevice {
    file: File,
    path: PathBuf,
    writable: bool,
    /// Device logical block size in bytes (typically 512 or 4096).
    #[allow(dead_code)]
    sector_size: u32,
    /// Scale factor: sector_size / 512. Used to convert device LBAs to
    /// the crate's 512-byte sector convention.
    lba_scale: u64,
}

impl PlatformDevice {
    /// Query the device sector size and compute the LBA scale factor.
    fn query_sector_size(file: &File, path: &Path) -> Result<(u32, u64)> {
        let mut sector_size: u32 = 0;
        // SAFETY: DIOCGSECTORSIZE reads a u32 into a valid pointer.
        unsafe { diocgsectorsize(file.as_raw_fd(), &mut sector_size) }.map_err(|e| {
            ZonedError::Ioctl {
                path: path.to_path_buf(),
                source: e,
            }
        })?;
        if sector_size == 0 {
            sector_size = 512;
        }
        let lba_scale = (sector_size / 512) as u64;
        Ok((sector_size, lba_scale))
    }

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
        let (sector_size, lba_scale) = Self::query_sector_size(&file, path)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            writable: false,
            sector_size,
            lba_scale,
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
        let (sector_size, lba_scale) = Self::query_sector_size(&file, path)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            writable: true,
            sector_size,
            lba_scale,
        })
    }

    pub(crate) fn open_direct(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_DIRECT)
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
        let (sector_size, lba_scale) = Self::query_sector_size(&file, path)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            writable: true,
            sector_size,
            lba_scale,
        })
    }

    pub(crate) fn is_writable(&self) -> bool {
        self.writable
    }

    pub(crate) fn fsync(&self) -> Result<()> {
        self.file.sync_all().map_err(|e| ZonedError::Io {
            path: self.path.clone(),
            source: e,
        })
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

    pub(crate) fn writev_at(&self, bufs: &[IoSlice<'_>], byte_offset: u64) -> Result<usize> {
        if !self.writable {
            return Err(ZonedError::ReadOnly {
                path: self.path.clone(),
            });
        }
        let fd = self.file.as_raw_fd();
        // SAFETY: pwritev is POSIX. fd is valid, iovec array is valid.
        let ret = unsafe {
            libc::pwritev(
                fd,
                bufs.as_ptr() as *const libc::iovec,
                bufs.len() as libc::c_int,
                byte_offset as libc::off_t,
            )
        };
        if ret < 0 {
            return Err(ZonedError::Io {
                path: self.path.clone(),
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(ret as usize)
    }

    pub(crate) fn readv_at(&self, bufs: &mut [IoSliceMut<'_>], byte_offset: u64) -> Result<usize> {
        let fd = self.file.as_raw_fd();
        // SAFETY: preadv is POSIX. fd is valid, iovec array is valid.
        let ret = unsafe {
            libc::preadv(
                fd,
                bufs.as_mut_ptr() as *mut libc::iovec,
                bufs.len() as libc::c_int,
                byte_offset as libc::off_t,
            )
        };
        if ret < 0 {
            return Err(ZonedError::Io {
                path: self.path.clone(),
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(ret as usize)
    }

    pub(crate) fn device_info(&self) -> Result<DeviceInfo> {
        // Get zone mode to verify this is a zoned device
        let params = self.get_zone_params()?;
        if params.zone_mode == DISK_ZONE_MODE_NONE {
            return Err(ZonedError::NotZoned {
                path: self.path.clone(),
            });
        }

        // Get total capacity via DIOCGMEDIASIZE
        let media_size = self.get_media_size()?;

        // Get zone size by reporting zones (needs large buffer — see report_zones_raw).
        let zones = self.report_zones_raw(0, MIN_REPORT_ENTRIES)?;
        if zones.is_empty() {
            return Err(ZonedError::NotZoned {
                path: self.path.clone(),
            });
        }
        let zone_len_lba = zones[0].zone_length;
        let zone_size = Sector(zone_len_lba * self.lba_scale);

        // Derive zone count from capacity
        let total_sectors = Sector(media_size as u64 / 512);
        let nr_zones = if zone_size.0 > 0 {
            total_sectors.0.div_ceil(zone_size.0) as u32
        } else {
            0
        };

        Ok(DeviceInfo {
            zone_size,
            nr_zones,
        })
    }

    pub(crate) fn report_zones(&self, sector: Sector, max_zones: u32) -> Result<Vec<Zone>> {
        let count = max_zones.max(1) as usize;

        let start_lba = if self.lba_scale > 0 {
            sector.0 / self.lba_scale
        } else {
            sector.0
        };

        // FreeBSD's da driver builds the SCSI REPORT ZONES command with an
        // allocation length derived from entries_allocated. Some devices
        // (or QEMU passthrough) reject small allocation lengths with EIO.
        // Always request at least MIN_REPORT_ENTRIES and truncate the result.
        let alloc_count = count.max(MIN_REPORT_ENTRIES);
        let raw_entries = self.report_zones_raw(start_lba, alloc_count)?;

        let mut zones = Vec::with_capacity(count.min(raw_entries.len()));
        for entry in raw_entries.iter().take(count) {
            zones.push(entry_to_zone(entry, self.lba_scale));
        }

        Ok(zones)
    }

    pub(crate) fn reset_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        // Check if this is a full-device reset (sector=0, covers everything)
        let info = self.device_info()?;
        let total = info.zone_size * info.nr_zones as u64;
        if sector == Sector::ZERO && nr_sectors >= total {
            // Use RWP_FLAG_ALL for a single ioctl
            return self.zone_rwp(0, DISK_ZONE_RWP_FLAG_ALL);
        }

        // Otherwise, iterate individual zones in the range
        self.for_each_zone_in_range(sector, nr_sectors, |lba| self.zone_rwp(lba, 0))
    }

    pub(crate) fn open_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        self.for_each_zone_in_range(sector, nr_sectors, |lba| {
            self.zone_cmd_single(DISK_ZONE_OPEN, lba)
        })
    }

    pub(crate) fn close_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        self.for_each_zone_in_range(sector, nr_sectors, |lba| {
            self.zone_cmd_single(DISK_ZONE_CLOSE, lba)
        })
    }

    pub(crate) fn finish_zones(&self, sector: Sector, nr_sectors: Sector) -> Result<()> {
        self.for_each_zone_in_range(sector, nr_sectors, |lba| {
            self.zone_cmd_single(DISK_ZONE_FINISH, lba)
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Query the zoned device model via GET_PARAMS.
    pub(crate) fn device_model(&self) -> Result<DeviceModel> {
        let params = self.get_zone_params()?;
        Ok(match params.zone_mode {
            DISK_ZONE_MODE_HOST_AWARE => DeviceModel::HostAware,
            DISK_ZONE_MODE_HOST_MANAGED => DeviceModel::HostManaged,
            _ => DeviceModel::None,
        })
    }

    /// Gather device properties from ioctls.
    ///
    /// FreeBSD has no sysfs, so this queries DIOCZONECMD (GET_PARAMS),
    /// DIOCGSECTORSIZE, and DIOCGMEDIASIZE directly. Fields that have
    /// no FreeBSD equivalent are returned as zero or None.
    pub(crate) fn device_properties(&self) -> Result<DeviceProperties> {
        let model = self.device_model()?;
        let info = self.device_info()?;
        let media_size = self.get_media_size()?;
        let capacity_sectors = Sector(media_size as u64 / 512);

        Ok(DeviceProperties {
            model,
            identity: DeviceIdentity {
                vendor: None,
                model_name: None,
            },
            geometry: DeviceGeometry {
                chunk_sectors: info.zone_size,
                nr_zones: info.nr_zones,
                capacity_sectors,
            },
            limits: DeviceLimits {
                zone_append_max_bytes: 0,
                max_open_zones: None,
                max_active_zones: None,
                max_hw_sectors_kb: 0,
                max_sectors_kb: 0,
            },
            block_sizes: BlockSizes {
                logical_block_size: self.sector_size,
                physical_block_size: self.sector_size,
            },
            scheduler: None,
        })
    }

    // ============================================================
    // Private helpers
    // ============================================================

    fn get_media_size(&self) -> Result<i64> {
        let mut media_size: i64 = 0;
        // SAFETY: DIOCGMEDIASIZE reads an off_t (i64) into a valid pointer.
        unsafe { diocgmediasize(self.file.as_raw_fd(), &mut media_size) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;
        Ok(media_size)
    }

    /// Low-level zone report returning raw kernel entries.
    fn report_zones_raw(
        &self,
        start_lba: u64,
        alloc_count: usize,
    ) -> Result<Vec<DiskZoneRepEntry>> {
        let empty_entry = DiskZoneRepEntry {
            zone_type: 0,
            zone_condition: 0,
            zone_flags: 0,
            _pad0: [0; 5],
            zone_length: 0,
            zone_start_lba: 0,
            write_pointer_lba: 0,
            _reserved: [0; 32],
        };
        let mut entries: Vec<DiskZoneRepEntry> = vec![empty_entry; alloc_count];

        let mut args = DiskZoneArgs {
            zone_cmd: DISK_ZONE_REPORT_ZONES,
            _pad0: [0; 7],
            zone_params: DiskZoneParams {
                report: std::mem::ManuallyDrop::new(DiskZoneReport {
                    starting_id: start_lba,
                    rep_options: DISK_ZONE_REP_ALL,
                    _pad0: [0; 7],
                    header: DiskZoneRepHeader {
                        same: 0,
                        _pad0: [0; 7],
                        maximum_lba: 0,
                        _reserved: [0; 64],
                    },
                    entries_allocated: alloc_count as u32,
                    entries_filled: 0,
                    entries_available: 0,
                    _pad1: [0; 4],
                    entries: entries.as_mut_ptr(),
                }),
            },
        };

        // SAFETY: dioczonecmd performs an IOWR ioctl. The entries array is
        // alive for the duration and properly sized. The fd is valid.
        let ret = unsafe { dioczonecmd(self.file.as_raw_fd(), &mut args) };
        if let Err(e) = ret {
            // EINVAL typically means the starting LBA is past the end of the
            // device — treat as "no zones to report" rather than a hard error.
            if e == nix::errno::Errno::EINVAL {
                return Ok(Vec::new());
            }
            return Err(ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            });
        }

        let filled = unsafe { args.zone_params.report.entries_filled } as usize;
        entries.truncate(filled);
        Ok(entries)
    }

    fn get_zone_params(&self) -> Result<DiskZoneDiskParams> {
        let mut args = DiskZoneArgs {
            zone_cmd: DISK_ZONE_GET_PARAMS,
            _pad0: [0; 7],
            zone_params: DiskZoneParams {
                disk_params: DiskZoneDiskParams {
                    zone_mode: 0,
                    _pad0: [0; 4],
                    flags: 0,
                    optimal_seq_zones: 0,
                    optimal_nonseq_zones: 0,
                    max_seq_zones: 0,
                },
            },
        };

        // SAFETY: dioczonecmd performs an IOWR ioctl with valid pointers.
        unsafe { dioczonecmd(self.file.as_raw_fd(), &mut args) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        // SAFETY: After successful ioctl, disk_params is filled by the kernel.
        Ok(unsafe { args.zone_params.disk_params })
    }

    /// Issue a RWP (Reset Write Pointer) command.
    fn zone_rwp(&self, lba: u64, flags: u8) -> Result<()> {
        let mut args = DiskZoneArgs {
            zone_cmd: DISK_ZONE_RWP,
            _pad0: [0; 7],
            zone_params: DiskZoneParams {
                rwp: DiskZoneRwp {
                    id: lba,
                    flags,
                    _pad0: [0; 7],
                },
            },
        };

        // SAFETY: dioczonecmd with valid DiskZoneArgs.
        unsafe { dioczonecmd(self.file.as_raw_fd(), &mut args) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        Ok(())
    }

    /// Issue an open/close/finish command for a single zone.
    fn zone_cmd_single(&self, cmd: u8, lba: u64) -> Result<()> {
        let mut args = DiskZoneArgs {
            zone_cmd: cmd,
            _pad0: [0; 7],
            zone_params: DiskZoneParams {
                rwp: DiskZoneRwp {
                    id: lba,
                    flags: 0,
                    _pad0: [0; 7],
                },
            },
        };

        // SAFETY: dioczonecmd with valid DiskZoneArgs.
        unsafe { dioczonecmd(self.file.as_raw_fd(), &mut args) }.map_err(|e| {
            ZonedError::Ioctl {
                path: self.path.clone(),
                source: e,
            }
        })?;

        Ok(())
    }

    /// Iterate over each zone whose start falls within [sector, sector + nr_sectors)
    /// and call `f` with the zone's LBA.
    fn for_each_zone_in_range(
        &self,
        sector: Sector,
        nr_sectors: Sector,
        f: impl Fn(u64) -> Result<()>,
    ) -> Result<()> {
        let end_sector = sector + nr_sectors;
        let mut cursor = sector;

        while cursor < end_sector {
            let zones = self.report_zones(cursor, 1)?;
            if zones.is_empty() {
                break;
            }
            let zone = &zones[0];
            let lba = zone.start.0 / self.lba_scale;
            f(lba)?;
            let next = zone.start + zone.len;
            if next <= cursor {
                break; // no progress
            }
            cursor = next;
        }

        Ok(())
    }
}
