//! Store and retrieve files in individual zones using standard I/O traits.
//!
//! A minimal example showing how to use `ZoneHandle` with `std::io::Write`,
//! `BufWriter`, zone iteration, and filtering.
//!
//! # Usage
//!
//! ```bash
//! # List zones and their status
//! sudo zone_store list /dev/sdb
//!
//! # Write a file into zone 10 (resets the zone first)
//! sudo zone_store write /dev/sdb 10 myfile.bin
//!
//! # Read it back
//! sudo zone_store read /dev/sdb 10 restored.bin
//! ```
//!
//! # On-disk format
//!
//! The first 8 bytes of the zone store the file size as little-endian u64,
//! followed by the raw file content.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use zoned::{Sector, ZoneCondition, ZoneFilter, ZoneHandle, ZoneIndex, ZoneType, ZonedDevice};

const HEADER_SIZE: u64 = 8; // u64 file-size prefix
const WRITE_ALIGN: usize = 4096; // minimum I/O size for many zoned devices

#[derive(Parser)]
#[command(name = "zone_store", about = "Store and retrieve files in zones")]
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

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run(cli: Cli) -> zoned::Result<()> {
    match cli.command {
        Command::List { device } => cmd_list(&device),
        Command::Write { device, zone, file } => cmd_write(&device, zone, &file),
        Command::Read {
            device,
            zone,
            output,
        } => cmd_read(&device, zone, &output),
    }
}

/// List sequential zones showing index, condition, and used/total capacity.
fn cmd_list(device: &PathBuf) -> zoned::Result<()> {
    let dev = ZonedDevice::builder(device).validate_all().open()?;
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

    // Use the lazy zone iterator to avoid loading all zones at once
    for (idx, result) in (0u32..).zip(dev.zone_iter(256)) {
        let zone = result?;
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

    // Summarize empty sequential zones using a filter
    let empty_seq = dev.report_zones_filtered(
        &ZoneFilter::new()
            .zone_type(ZoneType::SequentialWriteRequired)
            .condition(ZoneCondition::Empty),
        512,
    )?;
    println!("\n{} empty sequential zones available", empty_seq.len());

    Ok(())
}

/// Write a file into a zone using BufWriter<ZoneHandle> (std::io::Write).
fn cmd_write(device: &PathBuf, zone_num: u32, file: &PathBuf) -> zoned::Result<()> {
    let data = fs::read(file).map_err(|e| zoned::ZonedError::Io {
        path: file.clone(),
        source: e,
    })?;
    let file_size = data.len() as u64;

    let dev = Arc::new(
        ZonedDevice::builder(device)
            .writable()
            .validate_all()
            .open()?,
    );

    let mut handle = ZoneHandle::new(dev, ZoneIndex::new(zone_num))?;
    let zone_cap = handle.capacity().to_bytes();

    // Check that file + header fits in one zone
    let total = HEADER_SIZE + file_size;
    if total > zone_cap {
        eprintln!(
            "error: file ({file_size} bytes) + header ({HEADER_SIZE} bytes) exceeds zone capacity ({zone_cap} bytes)"
        );
        process::exit(1);
    }

    // Reset the zone to start fresh
    handle.reset()?;

    // Write using BufWriter over the ZoneHandle's std::io::Write impl.
    // This batches small writes into larger I/O requests automatically.
    // Device writes must be sector-aligned (512 bytes), so we pad the
    // final write to a sector boundary.
    {
        let mut writer = BufWriter::with_capacity(131072, &mut handle);

        // 8-byte header: file size
        writer
            .write_all(&file_size.to_le_bytes())
            .map_err(io_to_zoned)?;

        // File content
        writer.write_all(&data).map_err(io_to_zoned)?;

        // Pad to WRITE_ALIGN boundary (many zoned devices require 4 KiB minimum I/O)
        let written = (HEADER_SIZE + file_size) as usize;
        let remainder = written % WRITE_ALIGN;
        if remainder != 0 {
            let padding = vec![0u8; WRITE_ALIGN - remainder];
            writer.write_all(&padding).map_err(io_to_zoned)?;
        }

        writer.flush().map_err(io_to_zoned)?;
    }

    println!(
        "Wrote {} bytes to zone {} ({})",
        file_size,
        zone_num,
        device.display()
    );
    Ok(())
}

/// Read a previously stored file from a zone using ZonedDevice::read_at.
fn cmd_read(device: &PathBuf, zone_num: u32, output: &PathBuf) -> zoned::Result<()> {
    let dev = ZonedDevice::builder(device).validate_all().open()?;
    let info = dev.device_info()?;

    let zone_start = info.zone_size * zone_num as u64;

    // Read the first block to get the 8-byte header
    let mut first_sector = vec![0u8; WRITE_ALIGN];
    dev.read_at(zone_start, &mut first_sector)?;
    let file_size = u64::from_le_bytes(first_sector[..8].try_into().unwrap_or([0; 8]));

    if file_size == 0 || file_size > info.zone_size.to_bytes() {
        eprintln!(
            "error: zone {zone_num} does not contain a valid stored file (header says {file_size} bytes)"
        );
        process::exit(1);
    }

    // The write path writes header + content contiguously via BufWriter,
    // so file data starts at byte 8. Read the full payload (header + file),
    // rounding up to the write alignment boundary.
    let total_bytes = HEADER_SIZE + file_size;
    let read_len = ((total_bytes as usize) + WRITE_ALIGN - 1) & !(WRITE_ALIGN - 1);
    let mut buf = vec![0u8; read_len];

    let mut read_so_far = 0usize;
    while read_so_far < buf.len() {
        let offset = zone_start + Sector::from_bytes(read_so_far as u64).unwrap_or(Sector::ZERO);
        let n = dev.read_at(offset, &mut buf[read_so_far..])?;
        if n == 0 {
            break;
        }
        read_so_far += n;
    }

    // Extract file content after the 8-byte header
    let content = &buf[HEADER_SIZE as usize..(HEADER_SIZE + file_size) as usize];

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

fn io_to_zoned(e: std::io::Error) -> zoned::ZonedError {
    zoned::ZonedError::Io {
        path: PathBuf::from("<zone_handle>"),
        source: e,
    }
}
