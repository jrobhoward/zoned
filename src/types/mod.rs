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
    /// Start sector of the zone (512-byte sectors).
    pub start: u64,
    /// Length of the zone in sectors.
    pub len: u64,
    /// Usable capacity in sectors (may be less than len for ZNS devices).
    pub capacity: u64,
    /// Current write pointer position in sectors.
    pub write_pointer: u64,
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
    pub zone_size: u32,
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

/// Extended device information read from sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProperties {
    /// Device model (none, host-aware, host-managed).
    pub model: DeviceModel,
    /// Zone size in 512-byte sectors.
    pub chunk_sectors: u32,
    /// Total number of zones.
    pub nr_zones: u32,
    /// Maximum bytes for a zone append command (0 if unsupported).
    pub zone_append_max_bytes: u64,
    /// Maximum simultaneously open zones (0 = no limit).
    pub max_open_zones: u32,
    /// Maximum active zones (0 = no limit).
    pub max_active_zones: u32,
}

/// All sector values in this crate use 512-byte sectors, regardless of the
/// device's physical or logical block size. This matches the Linux kernel's
/// zoned block device interface.
pub const SECTOR_SIZE: u64 = 512;

#[cfg(test)]
mod types_tests;
