/// A sector offset or count in 512-byte sectors.
///
/// All sector values in this crate use 512-byte sectors, regardless of the
/// device's physical or logical block size. This matches the Linux kernel's
/// zoned block device interface.
///
/// Arithmetic operations that make physical sense are supported:
/// - `Sector + Sector`, `Sector - Sector` (offset arithmetic)
/// - `Sector * u64`, `Sector / u64` (scaling)
/// - `Sector * Sector` is intentionally **not** supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Sector(pub(crate) u64);

impl Sector {
    /// The zero sector.
    pub const ZERO: Sector = Sector(0);

    /// Create a new `Sector` from a raw 512-byte sector count.
    pub const fn new(value: u64) -> Self {
        Sector(value)
    }

    /// Convert to bytes (multiply by 512).
    pub fn to_bytes(self) -> u64 {
        self.0 * SECTOR_SIZE
    }

    /// Convert from a byte count. Returns `None` if not sector-aligned.
    pub fn from_bytes(bytes: u64) -> Option<Sector> {
        if bytes.is_multiple_of(SECTOR_SIZE) {
            Some(Sector(bytes / SECTOR_SIZE))
        } else {
            None
        }
    }

    /// Get the raw `u64` value.
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for Sector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::ops::Add for Sector {
    type Output = Sector;
    fn add(self, rhs: Sector) -> Sector {
        Sector(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for Sector {
    fn add_assign(&mut self, rhs: Sector) {
        self.0 += rhs.0;
    }
}

impl std::ops::Sub for Sector {
    type Output = Sector;
    fn sub(self, rhs: Sector) -> Sector {
        Sector(self.0 - rhs.0)
    }
}

impl std::ops::SubAssign for Sector {
    fn sub_assign(&mut self, rhs: Sector) {
        self.0 -= rhs.0;
    }
}

impl std::ops::Mul<u64> for Sector {
    type Output = Sector;
    fn mul(self, rhs: u64) -> Sector {
        Sector(self.0 * rhs)
    }
}

impl std::ops::Div<u64> for Sector {
    type Output = Sector;
    fn div(self, rhs: u64) -> Sector {
        Sector(self.0 / rhs)
    }
}

/// A zone index on a zoned block device.
///
/// Zone indices are identifiers, not quantities — no arithmetic operators
/// are provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneIndex(pub(crate) u32);

impl ZoneIndex {
    /// Create a new `ZoneIndex` from a raw zone number.
    pub const fn new(value: u32) -> Self {
        ZoneIndex(value)
    }

    /// Get the raw `u32` value.
    pub fn raw(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for ZoneIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Type of a zone on a zoned block device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoneType {
    /// Conventional zone — random writes allowed, no write pointer.
    Conventional,
    /// Sequential write required — must be written sequentially.
    SequentialWriteRequired,
    /// Sequential write preferred — sequential writes preferred but random allowed.
    SequentialWritePreferred,
}

/// Current condition (state) of a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoneCondition {
    /// Not a write-pointer zone (conventional).
    NotWritePointer,
    /// Empty — no data written since last reset.
    Empty,
    /// Implicitly opened by a write operation.
    ImplicitlyOpen,
    /// Explicitly opened by the host.
    ExplicitlyOpen,
    /// Closed — was open, now closed.
    Closed,
    /// Read-only — zone cannot be written.
    ReadOnly,
    /// Full — zone has been completely written.
    Full,
    /// Offline — zone is not usable.
    Offline,
}

/// Descriptor for a single zone on the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// Start sector of the zone.
    pub start: Sector,
    /// Length of the zone in sectors.
    pub len: Sector,
    /// Usable capacity in sectors (may be less than len for ZNS devices).
    pub capacity: Sector,
    /// Current write pointer position in sectors.
    /// `None` for conventional zones (which have no write pointer).
    pub write_pointer: Option<Sector>,
    /// Zone type.
    pub zone_type: ZoneType,
    /// Zone condition (state).
    pub condition: ZoneCondition,
    /// Non-sequential write resources are active.
    pub non_seq: bool,
    /// Reset write pointer recommended.
    pub reset_recommended: bool,
}

/// Device-level information about a zoned block device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Zone size in 512-byte sectors.
    pub zone_size: Sector,
    /// Total number of zones on the device.
    pub nr_zones: u32,
}

/// Model (type) of a zoned block device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceModel {
    /// Not a zoned device.
    None,
    /// Host-aware: device provides zone hints but allows random writes.
    HostAware,
    /// Host-managed: strict zone write rules enforced by device.
    HostManaged,
}

/// Device vendor and model identification from sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// Device vendor string from sysfs, if available.
    pub vendor: Option<String>,
    /// Device model name string from sysfs, if available.
    pub model_name: Option<String>,
}

/// Device geometry: zone layout and capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceGeometry {
    /// Zone size in 512-byte sectors.
    pub chunk_sectors: Sector,
    /// Total number of zones.
    pub nr_zones: u32,
    /// Total device capacity in 512-byte sectors.
    pub capacity_sectors: Sector,
}

/// Device I/O and zone limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLimits {
    /// Maximum bytes for a zone append command (0 if unsupported).
    pub zone_append_max_bytes: u64,
    /// Maximum simultaneously open zones. `None` = no device limit.
    pub max_open_zones: Option<u32>,
    /// Maximum active zones. `None` = no device limit.
    pub max_active_zones: Option<u32>,
    /// Maximum hardware I/O size in KiB (0 if unavailable).
    pub max_hw_sectors_kb: u32,
    /// Maximum software I/O size in KiB (0 if unavailable).
    pub max_sectors_kb: u32,
}

/// Logical and physical block sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSizes {
    /// Logical block size in bytes (0 if unavailable).
    pub logical_block_size: u32,
    /// Physical block size in bytes (0 if unavailable).
    pub physical_block_size: u32,
}

/// Extended device information read from sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProperties {
    /// Device model (none, host-aware, host-managed).
    pub model: DeviceModel,
    /// Device vendor and model identification.
    pub identity: DeviceIdentity,
    /// Zone layout and capacity.
    pub geometry: DeviceGeometry,
    /// I/O and zone limits.
    pub limits: DeviceLimits,
    /// Logical and physical block sizes.
    pub block_sizes: BlockSizes,
    /// Active I/O scheduler (e.g. `"mq-deadline"`), if available.
    pub scheduler: Option<String>,
}

/// All sector values in this crate use 512-byte sectors, regardless of the
/// device's physical or logical block size. This matches the Linux kernel's
/// zoned block device interface.
pub const SECTOR_SIZE: u64 = 512;

impl Zone {
    /// Remaining writable capacity in sectors.
    ///
    /// For zones with a write pointer, returns the distance from the write
    /// pointer to the capacity limit. For conventional zones (no write pointer),
    /// returns the full capacity.
    pub fn remaining_capacity(&self) -> Sector {
        match self.write_pointer {
            Some(wp) => self.capacity - (wp - self.start),
            None => self.capacity,
        }
    }

    /// Returns `true` if this is a sequential-write zone (required or preferred).
    pub fn is_sequential(&self) -> bool {
        matches!(
            self.zone_type,
            ZoneType::SequentialWriteRequired | ZoneType::SequentialWritePreferred
        )
    }

    /// Returns `true` if this is a conventional (random-write) zone.
    pub fn is_conventional(&self) -> bool {
        self.zone_type == ZoneType::Conventional
    }

    /// Returns `true` if the zone can accept writes.
    ///
    /// A zone is writable unless it is read-only, full, or offline.
    pub fn is_writable(&self) -> bool {
        !matches!(
            self.condition,
            ZoneCondition::ReadOnly | ZoneCondition::Full | ZoneCondition::Offline
        )
    }

    /// Returns `true` if the zone is empty (no data written since last reset).
    pub fn is_empty(&self) -> bool {
        self.condition == ZoneCondition::Empty
    }

    /// Returns `true` if the zone is full.
    pub fn is_full(&self) -> bool {
        self.condition == ZoneCondition::Full
    }
}

impl std::fmt::Display for ZoneType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZoneType::Conventional => f.write_str("Conventional"),
            ZoneType::SequentialWriteRequired => f.write_str("Sequential Write Required"),
            ZoneType::SequentialWritePreferred => f.write_str("Sequential Write Preferred"),
        }
    }
}

impl std::fmt::Display for ZoneCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZoneCondition::NotWritePointer => f.write_str("Not Write Pointer"),
            ZoneCondition::Empty => f.write_str("Empty"),
            ZoneCondition::ImplicitlyOpen => f.write_str("Implicitly Open"),
            ZoneCondition::ExplicitlyOpen => f.write_str("Explicitly Open"),
            ZoneCondition::Closed => f.write_str("Closed"),
            ZoneCondition::ReadOnly => f.write_str("Read Only"),
            ZoneCondition::Full => f.write_str("Full"),
            ZoneCondition::Offline => f.write_str("Offline"),
        }
    }
}

impl std::fmt::Display for DeviceModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceModel::None => f.write_str("none"),
            DeviceModel::HostAware => f.write_str("host-aware"),
            DeviceModel::HostManaged => f.write_str("host-managed"),
        }
    }
}

mod filter;

pub use filter::ZoneFilter;

#[cfg(test)]
mod types_tests;
