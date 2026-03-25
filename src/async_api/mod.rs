//! Async wrappers for zoned block device operations.
//!
//! This module provides [`AsyncZonedDevice`] and [`AsyncZoneHandle`], which
//! wrap the synchronous API using `tokio::task::spawn_blocking`. This approach
//! is appropriate because zoned block device operations are ioctl-based
//! (microsecond latency) — the same strategy `tokio::fs` uses internally.
//!
//! Enable with the `tokio` feature flag:
//!
//! ```toml
//! [dependencies]
//! zoned = { version = "0.1", features = ["tokio"] }
//! ```

mod device;
mod zone_handle;

pub use device::AsyncZonedDevice;
pub use zone_handle::AsyncZoneHandle;
