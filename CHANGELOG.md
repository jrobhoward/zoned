# Changelog

All notable changes to the `zoned` crate are documented in this file.

## [Unreleased]

### Added

- **Zone convenience methods**: `Zone::remaining_capacity()`, `is_sequential()`,
  `is_conventional()`, `is_writable()`, `is_empty()`, `is_full()`.

- **`write_all` variants**: `ZonedDevice::write_all_at()` and
  `ZoneHandle::write_all_sequential()` loop on partial writes to guarantee
  complete buffer writes.

- **`std::io::Write` on `ZoneHandle`**: Enables `BufWriter<ZoneHandle>` and
  standard I/O adapters for sequential zone writes.

- **`ZonedDeviceCursor`**: Cursor type wrapping `ZonedDevice` that implements
  `std::io::Read`, `Write`, and `Seek`. Created via `dev.cursor()` or
  `dev.cursor_at(sector)`.

- **Async API** (feature-gated behind `tokio`): `AsyncZonedDevice` and
  `AsyncZoneHandle` wrap the sync API using `tokio::task::spawn_blocking`.

- **Send/Sync compile-time assertions**: `ZonedDevice` (Send + Sync),
  `ZoneHandle` (Send only), `ZoneAllocator` (Send + Sync).

- **FreeBSD support**: Full implementation via `DIOCZONECMD` ioctl.

### Changed

- **`Sector` and `ZoneIndex` fields are now private**. Use `Sector::new(val)`
  and `ZoneIndex::new(val)` constructors. `raw()` accessors remain unchanged.

- **`DeviceProperties` restructured** into sub-structs: `DeviceIdentity`
  (vendor, model_name), `DeviceGeometry` (chunk_sectors, nr_zones,
  capacity_sectors), `DeviceLimits` (zone_append_max_bytes, max_open_zones,
  max_active_zones, max_hw_sectors_kb, max_sectors_kb), `BlockSizes`
  (logical_block_size, physical_block_size).

- **`ZonedError::Sysfs` and `SysfsParse`**: `device: String` field renamed to
  `path: PathBuf` for consistency with other error variants.

- **Newtype safety**: `Sector(u64)` and `ZoneIndex(u32)` types replace raw
  integers throughout the public API. `Sector` supports arithmetic (`Add`,
  `Sub`, `Mul<u64>`, `Div<u64>`) and conversion (`to_bytes`, `from_bytes`).
  `ZoneIndex` is an identifier with no arithmetic.

- **Device builder**: `ZonedDevice::builder("/dev/sda").writable().validate_all().open()?`
  composes open mode and validation checks. Individual checks:
  `validate_block_device()`, `validate_not_mounted()`, `validate_no_partitions()`,
  `validate_is_zoned()`.

- **Validation module** (`validate`): Standalone functions `is_block_device()`,
  `is_not_mounted()`, `has_no_partitions()`, `is_zoned_device()` with dedicated
  error variants (`NotABlockDevice`, `DeviceMounted`, `DeviceHasPartitions`).

- **Zone filtering**: `ZoneFilter` with composable `.zone_type()` and
  `.condition()` methods. `ZonedDevice::report_zones_filtered()` applies
  filters during batch iteration for memory-efficient queries on large devices.

- **Lazy zone iterator**: `ZonedDevice::zone_iter(batch_size)` yields
  `Result<Zone>` lazily, fetching in batches. Compatible with standard
  iterator adapters.

- **Vectored I/O**: `writev_at()` / `readv_at()` on `ZonedDevice` and
  `writev_sequential()` on `ZoneHandle` via `pwritev` / `preadv`.

- **Extended sysfs properties**: `DeviceProperties` includes block sizes,
  I/O limits, capacity, scheduler, vendor, and model name (organized into
  `DeviceIdentity`, `DeviceGeometry`, `DeviceLimits`, and `BlockSizes`
  sub-structs).

- **Display impls**: `ZoneType`, `ZoneCondition`, and `DeviceModel` implement
  `Display` with human-readable strings.

- **CLI tool** (`zcli`): 13 subcommands exercising the full library API:
  `info`, `zones`, `report`, `open`, `close`, `finish`, `reset`, `read`,
  `write`, `pwrite`, `validate`, `bench`. Replaces the former `zone_info`
  example.

- **`Zone::write_pointer`**: `Sector` -> `Option<Sector>`. Conventional zones
  now return `None` instead of a meaningless sentinel value (`u64::MAX`).

- **`DeviceLimits::max_open_zones`**: `u32` -> `Option<u32>`. `None` means
  no device limit (was `0`).

- **`DeviceLimits::max_active_zones`**: `u32` -> `Option<u32>`. Same
  treatment.

- **`DeviceInfo::zone_size`**: Widened from `u32` to `Sector` (wrapping `u64`)
  for consistency with other sector-valued fields.

- **`DeviceGeometry::chunk_sectors`**: `u32` -> `Sector`.

- All `ZonedDevice` methods accepting sector offsets or counts now take `Sector`
  instead of `u64`. All zone-index parameters take `ZoneIndex` instead of `u32`.

- `ZonedError::InvalidRange` fields changed from `u64` to `Sector`.
  `ZoneAlreadyAllocated`, `ZoneNotAllocated`, `ZoneFull` fields changed from
  `u32` to `ZoneIndex`.

## [0.1.0] - 2026-03-22

Initial release.

### Added

- `ZonedDevice` with `open`, `open_writable`, `open_direct`, `device_info`,
  `report_zones`, `report_all_zones`, `reset_zones`, `open_zones`,
  `close_zones`, `finish_zones`, `write_at`, `read_at`, `fsync`.
- `ZoneHandle` for exclusive single-zone ownership with local write-pointer
  tracking. Not `Clone`, is `Send`.
- `ZoneAllocator` for thread-safe zone allocation with automatic release on
  drop.
- `sysfs` module for querying device model and properties without opening the
  device.
- Linux platform support via kernel ioctls (`BLKREPORTZONE`, `BLKGETZONESZ`,
  `BLKGETNRZONES`, `BLKRESETZONE`, `BLKOPENZONE`, `BLKCLOSEZONE`,
  `BLKFINISHZONE`).
- FreeBSD platform stubs.
- `zone_info` CLI example with `info`, `reset-all`, and `bench` subcommands.
- Integration tests against null_blk emulated device and real HGST
  HMH7210A0AL drive.
