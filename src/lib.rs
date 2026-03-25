//! Pure Rust library for zoned block device management (SMR/ZNS).
//!
//! Provides a safe, idiomatic interface for interacting with zoned block
//! devices such as Shingled Magnetic Recording (SMR) hard drives and Zoned
//! Namespace (ZNS) NVMe SSDs.
//!
//! # Platform Support
//!
//! - **Linux**: Full support via kernel ioctls (`BLKREPORTZONE`, etc.) and sysfs.
//! - **FreeBSD**: Planned support via GEOM `BIO_ZONE` / CAM passthrough.
//!
//! # Example
//!
//! ```no_run
//! use zoned::{Sector, ZonedDevice, sysfs};
//!
//! // Check device model via sysfs
//! let props = sysfs::device_properties("/dev/sdb".as_ref())?;
//! println!("Model: {:?}, {} zones", props.model, props.nr_zones);
//!
//! // Open the device and query zones
//! let dev = ZonedDevice::open("/dev/sdb")?;
//! let zones = dev.report_zones(Sector::ZERO, 32)?;
//! for zone in &zones {
//!     println!("Zone at sector {}: {:?} ({:?})",
//!         zone.start, zone.zone_type, zone.condition);
//! }
//! # Ok::<(), zoned::ZonedError>(())
//! ```

mod device;
mod error;
mod platform;
pub mod sysfs;
mod types;
pub mod validate;
mod zone_allocator;
mod zone_handle;

pub use device::{DeviceBuilder, ZoneIterator, ZonedDevice};
pub use error::{Result, ZonedError};
pub use types::{
    DeviceInfo, DeviceModel, DeviceProperties, SECTOR_SIZE, Sector, Zone, ZoneCondition,
    ZoneFilter, ZoneIndex, ZoneType,
};
pub use zone_allocator::ZoneAllocator;
pub use zone_handle::ZoneHandle;
