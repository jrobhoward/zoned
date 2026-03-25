# zoned

Pure Rust library for zoned block device management (SMR/ZNS).

Provides a safe, idiomatic interface for interacting with Shingled Magnetic
Recording (SMR) hard drives and Zoned Namespace (ZNS) NVMe SSDs on Linux.

## Features

- **Zone reporting** with lazy iteration and client-side filtering
- **Zone management** — open, close, finish, reset
- **Data I/O** — positional read/write and vectored (scatter-gather) I/O
- **Exclusive zone handles** — compile-time enforcement of single-owner writes via `ZoneHandle`
- **Thread-safe zone allocation** — `ZoneAllocator` for concurrent multi-zone workloads
- **Device validation** — block device, mount, partition, and zoned-model checks
- **Builder pattern** — composable device opening with opt-in validation
- **Newtype safety** — `Sector` and `ZoneIndex` prevent unit confusion at compile time
- **sysfs integration** — zone model, block sizes, scheduler, vendor/model, capacity

## Platform Support

- **Linux**: Full support via kernel ioctls and sysfs (kernel 5.9+)
- **FreeBSD**: Planned

## Quick Start

```rust
use zoned::{Sector, ZonedDevice, ZoneFilter, ZoneType, ZoneCondition};

// Open with validation
let dev = ZonedDevice::builder("/dev/sdb")
    .validate_all()
    .open()?;

// Query device info
let info = dev.device_info()?;
println!("{} zones, {} sectors each", info.nr_zones, info.zone_size);

// Iterate over empty sequential zones
let empty_seq = dev.report_zones_filtered(
    &ZoneFilter::new()
        .zone_type(ZoneType::SequentialWriteRequired)
        .condition(ZoneCondition::Empty),
    512,
)?;
println!("{} empty sequential zones available", empty_seq.len());
```

## Concurrent Writes with ZoneHandle

```rust
use std::sync::Arc;
use zoned::{ZonedDevice, ZoneAllocator};

let dev = Arc::new(ZonedDevice::open_writable("/dev/sdb")?);
let allocator = ZoneAllocator::new(dev.clone());

// Each handle has exclusive ownership of its zone
let mut zone_a = allocator.allocate()?;
let mut zone_b = allocator.allocate()?;

// Safe to send to different threads — ZoneHandle is Send but not Clone
std::thread::spawn(move || {
    zone_a.write_sequential(&[0u8; 4096]).unwrap();
});
std::thread::spawn(move || {
    zone_b.write_sequential(&[0u8; 4096]).unwrap();
});
```

## CLI Tool

The `zcli` example exercises the full library API and serves as a practical
tool for inspecting and managing zoned devices:

```bash
cargo build --release --example zcli

# Device info (read-only)
sudo ./target/release/examples/zcli info /dev/sda

# List empty sequential zones
sudo ./target/release/examples/zcli zones /dev/sda --type seq-req --cond empty --count 10

# Zone state transitions
sudo ./target/release/examples/zcli open /dev/sda 378
sudo ./target/release/examples/zcli finish /dev/sda 378
sudo ./target/release/examples/zcli reset /dev/sda 378 --yes

# Read/write with hex dump
sudo ./target/release/examples/zcli read /dev/sda 0 --bytes 512
sudo ./target/release/examples/zcli pwrite /dev/sda 0 --bytes 4096 --pattern 0xAA --yes

# Validation checks
sudo ./target/release/examples/zcli validate /dev/sda

# Concurrent write benchmark
sudo ./target/release/examples/zcli bench /dev/sda -t 4 -z 2 -b 512 --yes
```

Run `zcli --help` or `zcli <subcommand> --help` for full usage.

## Testing

```bash
# Unit tests (no hardware required)
cargo test

# Integration tests with emulated zoned device (requires root + null_blk module)
sudo cargo test --test nullblk_integration

# Read-only tests against a real device (requires /dev/sda to be a zoned device)
cargo test --test sda_integration
```

## Requirements

- Rust 1.88.0+ (edition 2024)
- Linux kernel 5.9+ (for full sysfs attribute support)
- Root or `disk` group membership for device access

## License

MIT OR Apache-2.0
