# Changelog

All notable changes to the `zoned` crate are documented in this file.

## [Unreleased]

### Added

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

- **Extended sysfs properties**: `DeviceProperties` now includes
  `logical_block_size`, `physical_block_size`, `max_hw_sectors_kb`,
  `max_sectors_kb`, `capacity_sectors`, `scheduler`, `vendor`, `model_name`.

- **Display impls**: `ZoneType`, `ZoneCondition`, and `DeviceModel` implement
  `Display` with human-readable strings.

- **CLI tool** (`zcli`): 13 subcommands exercising the full library API:
  `info`, `zones`, `report`, `open`, `close`, `finish`, `reset`, `read`,
  `write`, `pwrite`, `validate`, `bench`. Replaces the former `zone_info`
  example.

### Changed

- **`Zone::write_pointer`**: `Sector` -> `Option<Sector>`. Conventional zones
  now return `None` instead of a meaningless sentinel value (`u64::MAX`).

- **`DeviceProperties::max_open_zones`**: `u32` -> `Option<u32>`. `None` means
  no device limit (was `0`).

- **`DeviceProperties::max_active_zones`**: `u32` -> `Option<u32>`. Same
  treatment.

- **`DeviceInfo::zone_size`**: Widened from `u32` to `Sector` (wrapping `u64`)
  for consistency with other sector-valued fields.

- **`DeviceProperties::chunk_sectors`**: `u32` -> `Sector`.

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
- FreeBSD stubs (returns `UnsupportedPlatform`).
- `zone_info` CLI example with `info`, `reset-all`, and `bench` subcommands.
- Integration tests against null_blk emulated device and real HGST
  HMH7210A0AL drive.
