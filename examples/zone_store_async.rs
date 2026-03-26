//! Async version of zone_store using tokio and the async API.
//!
//! Demonstrates `AsyncZonedDevice`, `AsyncZoneHandle`, and async zone
//! reporting with filters.
//!
//! # Usage
//!
//! ```bash
//! # List zones and their status
//! sudo zone_store_async list /dev/sdb
//!
//! # Write a file into zone 10 (resets the zone first)
//! sudo zone_store_async write /dev/sdb 10 myfile.bin
//!
//! # Read it back
//! sudo zone_store_async read /dev/sdb 10 restored.bin
//! ```
//!
//! # On-disk format
//!
//! Same as the sync `zone_store` example: 8-byte little-endian file size
//! followed by file content.

use std::fs;
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use zoned::async_api::AsyncZonedDevice;
use zoned::{Sector, ZoneCondition, ZoneFilter, ZoneIndex, ZoneType};

const HEADER_SIZE: u64 = 8;
const WRITE_ALIGN: usize = 4096; // minimum I/O size for many zoned devices

#[derive(Parser)]
#[command(name = "zone_store_async", about = "Async file storage in zones")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List sequential zones and their status
    List {
        /// Zoned block device path
        device: PathBuf,
    },
    /// Write a file into a zone (resets zone first)
    Write {
        /// Zoned block device path
        device: PathBuf,
        /// Zone number to write into
        zone: u32,
        /// File to store
        file: PathBuf,
    },
    /// Read a previously stored file from a zone
    Read {
        /// Zoned block device path
        device: PathBuf,
        /// Zone number to read from
        zone: u32,
        /// Output file path
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

async fn run(cli: Cli) -> zoned::Result<()> {
    match cli.command {
        Command::List { device } => cmd_list(&device).await,
        Command::Write { device, zone, file } => cmd_write(&device, zone, &file).await,
        Command::Read {
            device,
            zone,
            output,
        } => cmd_read(&device, zone, &output).await,
    }
}

/// List sequential zones using async zone reporting and filtering.
async fn cmd_list(device: &PathBuf) -> zoned::Result<()> {
    let dev = AsyncZonedDevice::open(device).await?;
    let info = dev.device_info()?;

    println!(
        "Device: {}  ({} zones, {} sectors/zone)\n",
        device.display(),
        info.nr_zones,
        info.zone_size
    );
    println!(
        "{:<6} {:<10} {:<14} {:>12} {:>12}",
        "Zone", "Type", "Condition", "Used", "Capacity"
    );
    println!("{}", "-".repeat(60));

    // Fetch all zones in one async call
    let zones = dev.report_all_zones(256).await?;
    for (idx, zone) in zones.iter().enumerate() {
        let type_str = match zone.zone_type {
            ZoneType::Conventional => "conv",
            ZoneType::SequentialWriteRequired => "seq-req",
            ZoneType::SequentialWritePreferred => "seq-pref",
        };

        let used = match zone.write_pointer {
            Some(wp) => wp - zone.start,
            None => Sector::ZERO,
        };

        println!(
            "{:<6} {:<10} {:<14} {:>12} {:>12}",
            idx, type_str, zone.condition, used, zone.capacity
        );
    }

    // Async filtered report for summary
    let empty_seq = dev
        .report_zones_filtered(
            ZoneFilter::new()
                .zone_type(ZoneType::SequentialWriteRequired)
                .condition(ZoneCondition::Empty),
            512,
        )
        .await?;
    println!("\n{} empty sequential zones available", empty_seq.len());

    Ok(())
}

/// Write a file into a zone using AsyncZoneHandle.
async fn cmd_write(device: &PathBuf, zone_num: u32, file: &PathBuf) -> zoned::Result<()> {
    let data = fs::read(file).map_err(|e| zoned::ZonedError::Io {
        path: file.clone(),
        source: e,
    })?;
    let file_size = data.len() as u64;

    let dev = AsyncZonedDevice::open_writable(device).await?;
    let handle = dev.zone_handle(ZoneIndex::new(zone_num)).await?;

    let zone_cap = handle.capacity().await.to_bytes();
    let total = HEADER_SIZE + file_size;
    if total > zone_cap {
        eprintln!(
            "error: file ({file_size} bytes) + header ({HEADER_SIZE} bytes) exceeds zone capacity ({zone_cap} bytes)"
        );
        process::exit(1);
    }

    // Reset and write: header then content
    handle.reset().await?;

    // Build the payload: 8-byte size header + file content.
    // Pad to WRITE_ALIGN boundary (many zoned devices require 4 KiB minimum I/O).
    let mut payload = Vec::with_capacity(total as usize);
    payload.extend_from_slice(&file_size.to_le_bytes());
    payload.extend_from_slice(&data);
    let remainder = payload.len() % WRITE_ALIGN;
    if remainder != 0 {
        payload.resize(payload.len() + (WRITE_ALIGN - remainder), 0);
    }

    handle.write_all_sequential(payload).await?;

    println!(
        "Wrote {} bytes to zone {} ({})",
        file_size,
        zone_num,
        device.display()
    );
    Ok(())
}

/// Read a previously stored file from a zone using async read_at.
async fn cmd_read(device: &PathBuf, zone_num: u32, output: &PathBuf) -> zoned::Result<()> {
    let dev = AsyncZonedDevice::open(device).await?;
    let info = dev.device_info()?;

    let zone_start = info.zone_size * zone_num as u64;

    // Read the first block to get the header
    let header_sector = dev.read_at(zone_start, WRITE_ALIGN).await?;
    if header_sector.len() < HEADER_SIZE as usize {
        eprintln!("error: could not read zone header");
        process::exit(1);
    }
    let file_size = u64::from_le_bytes(header_sector[..8].try_into().unwrap_or([0; 8]));

    if file_size == 0 || file_size > info.zone_size.to_bytes() {
        eprintln!(
            "error: zone {zone_num} does not contain a valid stored file (header says {file_size} bytes)"
        );
        process::exit(1);
    }

    // The write path writes header + content contiguously, so file data
    // starts at byte 8. Read the full payload, rounding up to alignment.
    let total_bytes = (HEADER_SIZE + file_size) as usize;
    let read_len = (total_bytes + WRITE_ALIGN - 1) & !(WRITE_ALIGN - 1);
    let raw = dev.read_at(zone_start, read_len).await?;
    let content = &raw[HEADER_SIZE as usize..HEADER_SIZE as usize + file_size as usize];

    fs::write(output, content).map_err(|e| zoned::ZonedError::Io {
        path: output.clone(),
        source: e,
    })?;

    println!(
        "Read {} bytes from zone {} -> {}",
        file_size,
        zone_num,
        output.display()
    );
    Ok(())
}
