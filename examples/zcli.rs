//! CLI tool for zoned block device management and testing.
//!
//! Exercises the full public API surface of the `zoned` crate. Each subcommand
//! demonstrates a different set of library capabilities.
//!
//! # Usage
//!
//! ```bash
//! # Show device and zone information (read-only)
//! sudo zcli info /dev/sda
//!
//! # List zones with filtering
//! sudo zcli zones /dev/sda --type seq-req --cond empty --count 10
//!
//! # Zone state transitions
//! sudo zcli open /dev/sda 378
//! sudo zcli close /dev/sda 378
//! sudo zcli finish /dev/sda 378
//!
//! # Reset zones
//! sudo zcli reset /dev/sda 378
//! sudo zcli reset /dev/sda --all --yes
//!
//! # Read data (hex dump)
//! sudo zcli read /dev/sda 0 --bytes 512
//! sudo zcli read /dev/sda 378 --bytes 1024 --scatter 512,512
//!
//! # Write pattern data
//! sudo zcli write /dev/sda 378 --bytes 4096 --yes
//! sudo zcli write /dev/sda 378 --bytes 4096 --pattern 0xBB --writev --yes
//!
//! # Validate device
//! sudo zcli validate /dev/sda
//!
//! # Concurrent write benchmark (DESTRUCTIVE)
//! sudo zcli bench /dev/sda -t 4 -z 2 --yes
//! sudo zcli bench /dev/sda --o-direct --writev -t 1 -z 4 -b 512 --yes
//! ```

use std::collections::HashMap;
use std::io::IoSlice;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use zoned::{
    Sector, ZoneAllocator, ZoneCondition, ZoneFilter, ZoneHandle, ZoneIndex, ZoneType, ZonedDevice,
    sysfs, validate,
};

#[derive(Parser)]
#[command(name = "zcli")]
#[command(about = "Zoned block device CLI — exercises the full zoned crate API")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Display device properties and zone summary (read-only)
    Info {
        /// Path to the block device (e.g. /dev/sda)
        device: PathBuf,

        /// Print every zone (not just the summary)
        #[arg(long)]
        all_zones: bool,

        /// Number of zones to report per batch
        #[arg(long, default_value = "512")]
        batch_size: u32,
    },

    /// List zones with optional filtering
    Zones {
        /// Path to the block device
        device: PathBuf,

        /// Filter by zone type
        #[arg(long = "type", value_enum)]
        zone_type: Option<ZoneTypeArg>,

        /// Filter by zone condition
        #[arg(long = "cond", value_enum)]
        condition: Option<ZoneCondArg>,

        /// Maximum number of zones to display
        #[arg(long)]
        count: Option<usize>,

        /// Show all fields including non_seq and reset_recommended
        #[arg(long, short = 'v')]
        verbose: bool,

        /// Number of zones to fetch per batch
        #[arg(long, default_value = "512")]
        batch_size: u32,
    },

    /// Reset zone write pointers (DESTRUCTIVE)
    Reset {
        /// Path to the block device
        device: PathBuf,

        /// Zone index or range (e.g. 378 or 378-382). Omit for --all.
        zone: Option<String>,

        /// Reset ALL sequential zones on the device
        #[arg(long)]
        all: bool,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Report zones starting from a sector or zone index (raw report_zones)
    Report {
        /// Path to the block device
        device: PathBuf,

        /// Zone index to start from
        #[arg(long, default_value = "0")]
        start: u32,

        /// Maximum number of zones to return
        #[arg(long, short = 'n', default_value = "8")]
        max: u32,
    },

    /// Explicitly open a zone (or zone range)
    Open {
        /// Path to the block device
        device: PathBuf,
        /// Zone index
        zone: u32,

        /// Operate on a range of zones (e.g. --range 3 opens zones zone..zone+2)
        #[arg(long)]
        range: Option<u32>,
    },

    /// Close a zone (or zone range)
    Close {
        /// Path to the block device
        device: PathBuf,
        /// Zone index
        zone: u32,

        /// Operate on a range of zones
        #[arg(long)]
        range: Option<u32>,
    },

    /// Finish (mark as full) a zone (or zone range)
    Finish {
        /// Path to the block device
        device: PathBuf,
        /// Zone index
        zone: u32,

        /// Operate on a range of zones
        #[arg(long)]
        range: Option<u32>,
    },

    /// Read data from the device and display as hex dump
    Read {
        /// Path to the block device
        device: PathBuf,
        /// Zone index to read from
        zone: u32,

        /// Number of bytes to read
        #[arg(long, default_value = "512")]
        bytes: usize,

        /// Sector offset within the zone (default: 0)
        #[arg(long, default_value = "0")]
        offset: u64,

        /// Use scatter read (readv_at) with comma-separated buffer sizes
        #[arg(long, value_delimiter = ',')]
        scatter: Option<Vec<usize>>,
    },

    /// Write pattern data to a zone (DESTRUCTIVE)
    Write {
        /// Path to the block device
        device: PathBuf,
        /// Zone index to write to
        zone: u32,

        /// Number of bytes to write
        #[arg(long, default_value = "4096")]
        bytes: usize,

        /// Fill byte pattern (hex, e.g. 0xAA)
        #[arg(long, default_value = "0xAA", value_parser = parse_hex_byte)]
        pattern: u8,

        /// Use writev_sequential (vectored I/O) — splits buffer into 2 halves
        #[arg(long)]
        writev: bool,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Raw positional write via write_at/writev_at (DESTRUCTIVE)
    ///
    /// Unlike `write` (which uses ZoneHandle for sequential writes), this
    /// writes directly at an arbitrary sector offset — useful for conventional
    /// zones where random writes are allowed.
    Pwrite {
        /// Path to the block device
        device: PathBuf,

        /// Sector offset to write at
        sector: u64,

        /// Number of bytes to write
        #[arg(long, default_value = "4096")]
        bytes: usize,

        /// Fill byte pattern (hex, e.g. 0xAA)
        #[arg(long, default_value = "0xAA", value_parser = parse_hex_byte)]
        pattern: u8,

        /// Use writev_at (vectored I/O) — splits buffer into 2 halves
        #[arg(long)]
        writev: bool,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Run device validation checks
    Validate {
        /// Path to the block device
        device: PathBuf,

        /// Run only specific checks (block-device, not-mounted, no-partitions, is-zoned)
        #[arg(long = "check", value_enum)]
        checks: Vec<ValidateCheck>,
    },

    /// Test boundary zones: first/last conventional and sequential (DESTRUCTIVE)
    ///
    /// Writes to and verifies the first and last conventional zones (random
    /// write, rewrite with different pattern) and the first and last
    /// sequential zones (sequential write, reset, verify).
    BoundaryTest {
        /// Path to the block device
        device: PathBuf,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Concurrent sequential write benchmark (DESTRUCTIVE)
    Bench {
        /// Path to the block device
        device: PathBuf,

        /// Number of writer threads
        #[arg(long, short = 't', default_value = "4")]
        threads: u32,

        /// Number of zones each thread writes to sequentially
        #[arg(long, short = 'z', default_value = "2")]
        zones_per_thread: u32,

        /// Write buffer size in KiB (must be a multiple of 4 KiB)
        #[arg(long, short = 'b', default_value = "128")]
        buf_size_kib: u32,

        /// Use O_DIRECT to bypass the page cache
        #[arg(long)]
        o_direct: bool,

        /// Use writev_sequential (vectored I/O) — splits buffer into 2 halves
        #[arg(long)]
        writev: bool,

        /// Call fsync after all writes complete
        #[arg(long)]
        fsync: bool,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, ValueEnum)]
enum ZoneTypeArg {
    Conventional,
    SeqReq,
    SeqPref,
}

impl ZoneTypeArg {
    fn to_zone_type(&self) -> ZoneType {
        match self {
            ZoneTypeArg::Conventional => ZoneType::Conventional,
            ZoneTypeArg::SeqReq => ZoneType::SequentialWriteRequired,
            ZoneTypeArg::SeqPref => ZoneType::SequentialWritePreferred,
        }
    }
}

#[derive(Clone, ValueEnum)]
enum ZoneCondArg {
    NotWp,
    Empty,
    ImpOpen,
    ExpOpen,
    Closed,
    ReadOnly,
    Full,
    Offline,
}

impl ZoneCondArg {
    fn to_zone_condition(&self) -> ZoneCondition {
        match self {
            ZoneCondArg::NotWp => ZoneCondition::NotWritePointer,
            ZoneCondArg::Empty => ZoneCondition::Empty,
            ZoneCondArg::ImpOpen => ZoneCondition::ImplicitlyOpen,
            ZoneCondArg::ExpOpen => ZoneCondition::ExplicitlyOpen,
            ZoneCondArg::Closed => ZoneCondition::Closed,
            ZoneCondArg::ReadOnly => ZoneCondition::ReadOnly,
            ZoneCondArg::Full => ZoneCondition::Full,
            ZoneCondArg::Offline => ZoneCondition::Offline,
        }
    }
}

#[derive(Clone, ValueEnum)]
enum ValidateCheck {
    BlockDevice,
    NotMounted,
    NoPartitions,
    IsZoned,
}

fn parse_hex_byte(s: &str) -> Result<u8, String> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16).map_err(|e| format!("invalid hex byte: {e}"))
    } else {
        s.parse::<u8>().map_err(|e| format!("invalid byte: {e}"))
    }
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Info {
            ref device,
            all_zones,
            batch_size,
        } => run_info(device, all_zones, batch_size),
        Commands::Zones {
            ref device,
            ref zone_type,
            ref condition,
            count,
            verbose,
            batch_size,
        } => run_zones(device, zone_type, condition, count, verbose, batch_size),
        Commands::Reset {
            ref device,
            ref zone,
            all,
            yes,
        } => run_reset(device, zone.as_deref(), all, yes),
        Commands::Report {
            ref device,
            start,
            max,
        } => run_report(device, start, max),
        Commands::Open {
            ref device,
            zone,
            range,
        } => run_zone_op(device, zone, range, "open"),
        Commands::Close {
            ref device,
            zone,
            range,
        } => run_zone_op(device, zone, range, "close"),
        Commands::Finish {
            ref device,
            zone,
            range,
        } => run_zone_op(device, zone, range, "finish"),
        Commands::Read {
            ref device,
            zone,
            bytes,
            offset,
            ref scatter,
        } => run_read(device, zone, bytes, offset, scatter.as_deref()),
        Commands::Write {
            ref device,
            zone,
            bytes,
            pattern,
            writev,
            yes,
        } => run_write(device, zone, bytes, pattern, writev, yes),
        Commands::Pwrite {
            ref device,
            sector,
            bytes,
            pattern,
            writev,
            yes,
        } => run_pwrite(device, sector, bytes, pattern, writev, yes),
        Commands::Validate {
            ref device,
            ref checks,
        } => run_validate(device, checks),
        Commands::BoundaryTest { ref device, yes } => run_boundary_test(device, yes),
        Commands::Bench {
            ref device,
            threads,
            zones_per_thread,
            buf_size_kib,
            o_direct,
            writev,
            fsync,
            yes,
        } => run_bench(
            device,
            threads,
            zones_per_thread,
            buf_size_kib,
            o_direct,
            writev,
            fsync,
            yes,
        ),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

// ============================================================
// Info subcommand
// ============================================================

fn run_info(
    path: &Path,
    all_zones: bool,
    batch_size: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let dev = ZonedDevice::builder(path).validate_all().open()?;
    let info = dev.device_info()?;

    // sysfs properties are Linux-only; gracefully skip on other platforms.
    let props = sysfs::device_properties(path).ok();

    println!("=== Device Identity ===");
    if let Some(ref p) = props {
        if let Some(ref v) = p.identity.vendor {
            println!("  Vendor:              {v}");
        }
        if let Some(ref m) = p.identity.model_name {
            println!("  Model:               {m}");
        }
        println!("  Zone model:          {}", p.model);
        if let Some(ref s) = p.scheduler {
            println!("  Scheduler:           {s}");
        }
    }
    println!();

    println!("=== Device Info (ioctl) ===");
    println!(
        "  Zone size:           {} sectors ({} MiB)",
        info.zone_size,
        info.zone_size.to_bytes() / (1024 * 1024)
    );
    println!("  Number of zones:     {}", info.nr_zones);

    if let Some(ref p) = props {
        if p.geometry.capacity_sectors.raw() > 0 {
            println!(
                "  Total capacity:      {} sectors ({:.1} GiB)",
                p.geometry.capacity_sectors,
                p.geometry.capacity_sectors.to_bytes() as f64 / (1024.0 * 1024.0 * 1024.0)
            );
        }
        println!(
            "  Zone append max:     {} bytes",
            p.limits.zone_append_max_bytes
        );
        println!(
            "  Max open zones:      {}",
            format_limit(p.limits.max_open_zones)
        );
        println!(
            "  Max active zones:    {}",
            format_limit(p.limits.max_active_zones)
        );
        if p.block_sizes.logical_block_size > 0 {
            println!(
                "  Logical block size:  {} bytes",
                p.block_sizes.logical_block_size
            );
        }
        if p.block_sizes.physical_block_size > 0 {
            println!(
                "  Physical block size: {} bytes",
                p.block_sizes.physical_block_size
            );
        }
        if p.limits.max_sectors_kb > 0 {
            println!("  Max I/O size:        {} KiB", p.limits.max_sectors_kb);
        }
        if p.limits.max_hw_sectors_kb > 0 {
            println!("  Max HW I/O size:     {} KiB", p.limits.max_hw_sectors_kb);
        }
    }
    println!();

    // Use zone_iter() for lazy zone census
    println!("=== Zone Census (via zone_iter) ===");

    let mut cond_counts: HashMap<ZoneCondition, u32> = HashMap::new();
    let mut total_capacity = Sector::ZERO;
    let mut total_len = Sector::ZERO;
    let mut conventional_count: u32 = 0;
    let mut seq_required_count: u32 = 0;
    let mut seq_preferred_count: u32 = 0;
    let mut non_seq_count: u32 = 0;
    let mut reset_recommended_count: u32 = 0;
    let mut zone_list = Vec::new();

    for zone_result in dev.zone_iter(batch_size) {
        let zone = zone_result?;
        *cond_counts.entry(zone.condition).or_insert(0) += 1;
        total_capacity += zone.capacity;
        total_len += zone.len;
        if zone.non_seq {
            non_seq_count += 1;
        }
        if zone.reset_recommended {
            reset_recommended_count += 1;
        }
        match zone.zone_type {
            ZoneType::Conventional => conventional_count += 1,
            ZoneType::SequentialWriteRequired => seq_required_count += 1,
            ZoneType::SequentialWritePreferred => seq_preferred_count += 1,
        }
        if all_zones {
            zone_list.push(zone);
        }
    }

    let total_zones = conventional_count + seq_required_count + seq_preferred_count;
    println!("  Conventional:             {:>6}", conventional_count);
    println!("  Sequential Write Required:{:>6}", seq_required_count);
    println!("  Sequential Write Preferred:{:>5}", seq_preferred_count);
    println!("  Total:                    {:>6}", total_zones);
    if non_seq_count > 0 {
        println!("  Non-sequential active:    {:>6}", non_seq_count);
    }
    if reset_recommended_count > 0 {
        println!("  Reset recommended:        {:>6}", reset_recommended_count);
    }
    println!();

    println!("=== Zone Condition Summary ===");
    let cond_order = [
        ZoneCondition::NotWritePointer,
        ZoneCondition::Empty,
        ZoneCondition::ImplicitlyOpen,
        ZoneCondition::ExplicitlyOpen,
        ZoneCondition::Closed,
        ZoneCondition::Full,
        ZoneCondition::ReadOnly,
        ZoneCondition::Offline,
    ];
    for cond in &cond_order {
        let count = cond_counts.get(cond).copied().unwrap_or(0);
        if count > 0 {
            println!("  {cond:<25} {count:>6}");
        }
    }
    println!();

    let total_capacity_gib = total_capacity.to_bytes() as f64 / (1024.0 * 1024.0 * 1024.0);
    let total_len_gib = total_len.to_bytes() as f64 / (1024.0 * 1024.0 * 1024.0);
    println!("=== Capacity ===");
    println!(
        "  Total zone length:     {} sectors ({:.1} GiB)",
        total_len, total_len_gib
    );
    println!(
        "  Total usable capacity: {} sectors ({:.1} GiB)",
        total_capacity, total_capacity_gib
    );
    if total_len > total_capacity {
        let overhead = total_len - total_capacity;
        let overhead_mib = overhead.to_bytes() / (1024 * 1024);
        println!(
            "  Capacity overhead:     {} sectors ({} MiB)",
            overhead, overhead_mib
        );
    }
    println!();

    if all_zones {
        println!("=== All Zones ===");
        println!(
            "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}",
            "Index", "Start", "Length", "Capacity", "WritePtr", "Type", "Condition"
        );
        println!("{}", "-".repeat(115));
        for (i, zone) in zone_list.iter().enumerate() {
            println!(
                "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}",
                i,
                zone.start,
                zone.len,
                zone.capacity,
                format_wp(zone.write_pointer),
                zone.zone_type,
                zone.condition,
            );
        }
        println!();
    }

    println!("Done.");
    Ok(())
}

// ============================================================
// Zones subcommand
// ============================================================

fn run_zones(
    path: &Path,
    zone_type: &Option<ZoneTypeArg>,
    condition: &Option<ZoneCondArg>,
    count: Option<usize>,
    verbose: bool,
    batch_size: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let dev = ZonedDevice::builder(path).validate_all().open()?;

    let mut filter = ZoneFilter::new();
    if let Some(zt) = zone_type {
        filter = filter.zone_type(zt.to_zone_type());
    }
    if let Some(cond) = condition {
        filter = filter.condition(cond.to_zone_condition());
    }

    let zones = dev.report_zones_filtered(&filter, batch_size)?;
    let display_count = count.unwrap_or(zones.len()).min(zones.len());

    println!("{} zones match (showing {})", zones.len(), display_count);
    println!();

    if verbose {
        println!(
            "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}  {:>7}  {:>5}",
            "Index",
            "Start",
            "Length",
            "Capacity",
            "WritePtr",
            "Type",
            "Condition",
            "NonSeq",
            "RstRc"
        );
        println!("{}", "-".repeat(135));
    } else {
        println!(
            "{:>6}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}",
            "Index", "Start", "Capacity", "WritePtr", "Type", "Condition"
        );
        println!("{}", "-".repeat(105));
    }

    let info = dev.device_info()?;
    for zone in zones.iter().take(display_count) {
        let idx = if info.zone_size.raw() > 0 {
            zone.start.raw() / info.zone_size.raw()
        } else {
            0
        };
        if verbose {
            println!(
                "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}  {:>7}  {:>5}",
                idx,
                zone.start,
                zone.len,
                zone.capacity,
                format_wp(zone.write_pointer),
                zone.zone_type,
                zone.condition,
                zone.non_seq,
                zone.reset_recommended,
            );
        } else {
            println!(
                "{:>6}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}",
                idx,
                zone.start,
                zone.capacity,
                format_wp(zone.write_pointer),
                zone.zone_type,
                zone.condition,
            );
        }
    }

    if display_count < zones.len() {
        println!("  ... {} more zones not shown", zones.len() - display_count);
    }

    Ok(())
}

// ============================================================
// Report subcommand
// ============================================================

fn run_report(
    path: &Path,
    start_zone: u32,
    max_zones: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let dev = ZonedDevice::builder(path).validate_all().open()?;
    let info = dev.device_info()?;
    let start_sector = info.zone_size * start_zone as u64;

    let zones = dev.report_zones(start_sector, max_zones)?;

    println!(
        "report_zones(sector={}, max={}): {} zones returned",
        start_sector,
        max_zones,
        zones.len()
    );
    println!();
    println!(
        "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}",
        "Index", "Start", "Length", "Capacity", "WritePtr", "Type", "Condition"
    );
    println!("{}", "-".repeat(115));

    for zone in &zones {
        let idx = if info.zone_size.raw() > 0 {
            zone.start.raw() / info.zone_size.raw()
        } else {
            0
        };
        println!(
            "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}",
            idx,
            zone.start,
            zone.len,
            zone.capacity,
            format_wp(zone.write_pointer),
            zone.zone_type,
            zone.condition,
        );
    }

    Ok(())
}

// ============================================================
// Open / Close / Finish subcommands
// ============================================================

fn run_zone_op(
    path: &Path,
    zone_idx: u32,
    range: Option<u32>,
    op: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let dev = Arc::new(
        ZonedDevice::builder(path)
            .writable()
            .validate_all()
            .open()?,
    );

    if let Some(count) = range {
        // Use direct device methods with sector ranges (open_zones/close_zones/finish_zones)
        let info = dev.device_info()?;
        let sector = info.zone_size * zone_idx as u64;
        let nr_sectors = info.zone_size * count as u64;

        println!(
            "{op} zones {zone_idx}..{} (sector {sector}, {nr_sectors} sectors)",
            zone_idx + count - 1
        );

        match op {
            "open" => dev.open_zones(sector, nr_sectors)?,
            "close" => dev.close_zones(sector, nr_sectors)?,
            "finish" => dev.finish_zones(sector, nr_sectors)?,
            _ => return Err(format!("unknown operation: {op}").into()),
        }

        // Show resulting state for each zone in the range
        let zones = dev.report_zones(sector, count)?;
        for zone in &zones {
            let idx = if info.zone_size.raw() > 0 {
                zone.start.raw() / info.zone_size.raw()
            } else {
                0
            };
            println!(
                "  zone {idx}: {} (wp: {})",
                zone.condition,
                format_wp(zone.write_pointer)
            );
        }
    } else {
        // Use ZoneHandle for single-zone operation
        let mut handle = ZoneHandle::new(dev, ZoneIndex::new(zone_idx))?;

        let before = handle.report()?;
        println!(
            "Zone {zone_idx} before: {} (wp: {})",
            before.condition,
            format_wp(before.write_pointer)
        );

        match op {
            "open" => handle.open()?,
            "close" => handle.close()?,
            "finish" => handle.finish()?,
            _ => return Err(format!("unknown operation: {op}").into()),
        }

        let after = handle.report()?;
        println!(
            "Zone {zone_idx} after:  {} (wp: {})",
            after.condition,
            format_wp(after.write_pointer)
        );
    }

    Ok(())
}

// ============================================================
// Reset subcommand
// ============================================================

fn run_reset(
    path: &Path,
    zone_arg: Option<&str>,
    all: bool,
    skip_confirm: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let dev = ZonedDevice::builder(path)
        .writable()
        .validate_all()
        .open()?;
    let info = dev.device_info()?;

    if all {
        let non_empty_count = dev
            .zone_iter(512)
            .filter_map(|r| r.ok())
            .filter(|z| {
                z.zone_type == ZoneType::SequentialWriteRequired
                    && z.condition != ZoneCondition::Empty
            })
            .count();

        if non_empty_count == 0 {
            println!("All sequential zones are already empty.");
            return Ok(());
        }

        if !skip_confirm {
            println!(
                "WARNING: This will RESET {} non-empty sequential zones on {}.",
                non_empty_count,
                path.display()
            );
            confirm()?;
        }

        let total_sectors = info.zone_size * info.nr_zones as u64;
        let start = Instant::now();
        dev.reset_zones(Sector::ZERO, total_sectors)?;
        println!(
            "Reset {} zones in {:.2}s",
            non_empty_count,
            start.elapsed().as_secs_f64()
        );
    } else if let Some(zone_str) = zone_arg {
        let (start_idx, end_idx) = parse_zone_range(zone_str)?;

        if !skip_confirm {
            if start_idx == end_idx {
                println!(
                    "WARNING: This will RESET zone {} on {}.",
                    start_idx,
                    path.display()
                );
            } else {
                println!(
                    "WARNING: This will RESET zones {}-{} on {}.",
                    start_idx,
                    end_idx,
                    path.display()
                );
            }
            confirm()?;
        }

        for idx in start_idx..=end_idx {
            let sector = info.zone_size * idx as u64;
            dev.reset_zones(sector, info.zone_size)?;
            println!("  Reset zone {idx}");
        }
    } else {
        return Err("specify a zone index/range or --all".into());
    }

    println!("Done.");
    Ok(())
}

// ============================================================
// Read subcommand
// ============================================================

fn run_read(
    path: &Path,
    zone_idx: u32,
    bytes: usize,
    offset_sectors: u64,
    scatter: Option<&[usize]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let dev = ZonedDevice::builder(path).validate_all().open()?;
    let info = dev.device_info()?;
    let zone_start = info.zone_size * zone_idx as u64;
    let read_sector = zone_start + Sector::new(offset_sectors);

    if let Some(sizes) = scatter {
        // Demonstrate readv_at with scatter buffers
        let mut buffers: Vec<Vec<u8>> = sizes.iter().map(|&s| vec![0u8; s]).collect();
        let mut slices: Vec<std::io::IoSliceMut<'_>> = buffers
            .iter_mut()
            .map(|b| std::io::IoSliceMut::new(b))
            .collect();

        let n = dev.readv_at(read_sector, &mut slices)?;
        println!(
            "readv_at: {} bytes from sector {} ({} scatter buffers)",
            n,
            read_sector,
            sizes.len()
        );
        println!();

        let mut consumed = 0usize;
        for (i, buf) in buffers.iter().enumerate() {
            let buf_read = buf.len().min(n.saturating_sub(consumed));
            println!("--- buffer {} ({} bytes) ---", i, buf_read);
            hex_dump(&buf[..buf_read]);
            consumed += buf.len();
            println!();
        }
    } else {
        let mut buf = vec![0u8; bytes];
        let n = dev.read_at(read_sector, &mut buf)?;
        println!(
            "read_at: {} bytes from sector {} (zone {} + offset {})",
            n, read_sector, zone_idx, offset_sectors
        );
        println!();
        hex_dump(&buf[..n]);
    }

    Ok(())
}

// ============================================================
// Write subcommand
// ============================================================

fn run_write(
    path: &Path,
    zone_idx: u32,
    bytes: usize,
    pattern: u8,
    use_writev: bool,
    skip_confirm: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !skip_confirm {
        println!(
            "WARNING: This will WRITE {} bytes to zone {} on {}.",
            bytes,
            zone_idx,
            path.display()
        );
        confirm()?;
    }

    let dev = Arc::new(
        ZonedDevice::builder(path)
            .writable()
            .validate_all()
            .open()?,
    );

    let mut handle = ZoneHandle::new(dev, ZoneIndex::new(zone_idx))?;

    let before = handle.report()?;
    println!(
        "Zone {} before: {} (wp: {})",
        zone_idx,
        before.condition,
        format_wp(before.write_pointer)
    );

    let data = vec![pattern; bytes];

    let written = if use_writev {
        let mid = data.len() / 2;
        let bufs = [IoSlice::new(&data[..mid]), IoSlice::new(&data[mid..])];
        let w = handle.writev_sequential(&bufs)?;
        println!(
            "  writev_sequential: {} bytes via 2 scatter buffers ({} + {})",
            w,
            mid,
            data.len() - mid
        );
        w
    } else {
        let w = handle.write_sequential(&data)?;
        println!("  write_sequential: {} bytes", w);
        w
    };

    let after = handle.report()?;
    println!(
        "Zone {} after:  {} (wp: {})",
        zone_idx,
        after.condition,
        format_wp(after.write_pointer)
    );
    println!(
        "  Wrote {} bytes ({} sectors), pattern 0x{:02X}",
        written,
        Sector::from_bytes(written as u64).map_or("?".to_string(), |s| s.to_string()),
        pattern,
    );

    Ok(())
}

// ============================================================
// Pwrite subcommand
// ============================================================

fn run_pwrite(
    path: &Path,
    sector: u64,
    bytes: usize,
    pattern: u8,
    use_writev: bool,
    skip_confirm: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !skip_confirm {
        println!(
            "WARNING: This will WRITE {} bytes at sector {} on {}.",
            bytes,
            sector,
            path.display()
        );
        confirm()?;
    }

    let dev = ZonedDevice::builder(path)
        .writable()
        .validate_all()
        .open()?;

    let offset = Sector::new(sector);
    let data = vec![pattern; bytes];

    let written = if use_writev {
        let mid = data.len() / 2;
        let bufs = [IoSlice::new(&data[..mid]), IoSlice::new(&data[mid..])];
        let w = dev.writev_at(offset, &bufs)?;
        println!(
            "  writev_at(sector={offset}): {} bytes via 2 gather buffers ({} + {})",
            w,
            mid,
            data.len() - mid
        );
        w
    } else {
        let w = dev.write_at(offset, &data)?;
        println!("  write_at(sector={offset}): {} bytes", w);
        w
    };

    println!(
        "  Wrote {} bytes ({} sectors) at sector {}, pattern 0x{:02X}",
        written,
        Sector::from_bytes(written as u64).map_or("?".to_string(), |s| s.to_string()),
        sector,
        pattern,
    );

    Ok(())
}

// ============================================================
// Validate subcommand
// ============================================================

fn run_validate(path: &Path, checks: &[ValidateCheck]) -> Result<(), Box<dyn std::error::Error>> {
    let run_all = checks.is_empty();
    let mut passed = 0u32;
    let mut failed = 0u32;

    let check_list: Vec<ValidateCheck> = if run_all {
        vec![
            ValidateCheck::BlockDevice,
            ValidateCheck::NotMounted,
            ValidateCheck::NoPartitions,
            ValidateCheck::IsZoned,
        ]
    } else {
        checks.to_vec()
    };

    for check in &check_list {
        let (name, result) = match check {
            ValidateCheck::BlockDevice => ("block-device", validate::is_block_device(path)),
            ValidateCheck::NotMounted => ("not-mounted", validate::is_not_mounted(path)),
            ValidateCheck::NoPartitions => ("no-partitions", validate::has_no_partitions(path)),
            ValidateCheck::IsZoned => ("is-zoned", validate::is_zoned_device(path)),
        };
        match result {
            Ok(()) => {
                println!("  [PASS] {name}");
                passed += 1;
            }
            Err(e) => {
                println!("  [FAIL] {name}: {e}");
                failed += 1;
            }
        }
    }

    println!();
    println!("{passed} passed, {failed} failed");
    if failed > 0 {
        process::exit(1);
    }
    Ok(())
}

// ============================================================
// Boundary Test subcommand
// ============================================================

fn run_boundary_test(path: &Path, skip_confirm: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !skip_confirm {
        println!(
            "WARNING: This will WRITE to boundary zones on {}.",
            path.display()
        );
        confirm()?;
    }

    let dev = Arc::new(
        ZonedDevice::builder(path)
            .writable()
            .validate_all()
            .open()?,
    );
    let info = dev.device_info()?;
    let mut passed = 0u32;
    let mut failed = 0u32;

    // Discover zone layout
    let first_zones = dev.report_zones(Sector::ZERO, 1)?;
    if first_zones.is_empty() {
        return Err("no zones found".into());
    }

    // Find first and last conventional zones
    let conv_filter = ZoneFilter::new().zone_type(ZoneType::Conventional);
    let conv_zones = dev.report_zones_filtered(&conv_filter, 512)?;

    // Find first and last sequential zones
    let seq_filter = ZoneFilter::new()
        .zone_type(ZoneType::SequentialWriteRequired)
        .condition(ZoneCondition::Empty);
    let seq_zones = dev.report_zones_filtered(&seq_filter, 512)?;

    println!("=== Device Layout ===");
    println!("  Zone size:         {} sectors", info.zone_size);
    println!("  Total zones:       {}", info.nr_zones);
    println!("  Conventional:      {}", conv_zones.len());
    println!("  Sequential (empty): {}", seq_zones.len());
    println!();

    // --- Conventional zone tests ---
    if conv_zones.len() >= 2 {
        let first_conv = &conv_zones[0];
        let last_conv = &conv_zones[conv_zones.len() - 1];
        let first_conv_idx = first_conv.start.raw() / info.zone_size.raw();
        let last_conv_idx = last_conv.start.raw() / info.zone_size.raw();

        println!("=== Conventional Zone Boundary Tests ===");

        // First conventional zone: write, verify, rewrite, verify
        print!("  First conv zone {first_conv_idx}: write 0xAA... ");
        dev.write_at(first_conv.start, &vec![0xAAu8; 4096])?;
        let mut buf = vec![0u8; 4096];
        dev.read_at(first_conv.start, &mut buf)?;
        if buf.iter().all(|&b| b == 0xAA) {
            print!("OK, rewrite 0x55... ");
            dev.write_at(first_conv.start, &vec![0x55u8; 4096])?;
            dev.read_at(first_conv.start, &mut buf)?;
            if buf.iter().all(|&b| b == 0x55) {
                println!("OK");
                passed += 1;
            } else {
                println!("FAIL (rewrite verify)");
                failed += 1;
            }
        } else {
            println!("FAIL (initial verify)");
            failed += 1;
        }

        // Last conventional zone: write, verify, rewrite, verify
        print!("  Last conv zone {last_conv_idx}: write 0xBB... ");
        dev.write_at(last_conv.start, &vec![0xBBu8; 4096])?;
        dev.read_at(last_conv.start, &mut buf)?;
        if buf.iter().all(|&b| b == 0xBB) {
            print!("OK, rewrite 0x66... ");
            dev.write_at(last_conv.start, &vec![0x66u8; 4096])?;
            dev.read_at(last_conv.start, &mut buf)?;
            if buf.iter().all(|&b| b == 0x66) {
                println!("OK");
                passed += 1;
            } else {
                println!("FAIL (rewrite verify)");
                failed += 1;
            }
        } else {
            println!("FAIL (initial verify)");
            failed += 1;
        }
    } else {
        println!("  Skipping conventional tests (need >= 2 conventional zones)");
    }

    // --- Sequential zone tests ---
    if seq_zones.len() >= 2 {
        let first_seq = &seq_zones[0];
        let last_seq = &seq_zones[seq_zones.len() - 1];
        let first_seq_idx = first_seq.start.raw() / info.zone_size.raw();
        let last_seq_idx = last_seq.start.raw() / info.zone_size.raw();

        println!();
        println!("=== Sequential Zone Boundary Tests ===");

        // First sequential zone: write, verify, reset
        print!("  First seq zone {first_seq_idx}: write 0xCC... ");
        let mut handle = ZoneHandle::new(dev.clone(), ZoneIndex::new(first_seq_idx as u32))?;
        handle.write_sequential(&vec![0xCCu8; 4096])?;
        let mut buf = vec![0u8; 4096];
        dev.read_at(first_seq.start, &mut buf)?;
        if buf.iter().all(|&b| b == 0xCC) {
            println!("OK, reset... ");
            handle.reset()?;
            let zone = handle.report()?;
            if zone.condition == ZoneCondition::Empty {
                print!("    Reset OK, rewrite 0x33... ");
                handle.write_sequential(&vec![0x33u8; 4096])?;
                dev.read_at(first_seq.start, &mut buf)?;
                if buf.iter().all(|&b| b == 0x33) {
                    println!("OK");
                    passed += 1;
                } else {
                    println!("FAIL (rewrite verify)");
                    failed += 1;
                }
                handle.reset()?;
            } else {
                println!("FAIL (not empty after reset)");
                failed += 1;
            }
        } else {
            println!("FAIL (initial verify)");
            failed += 1;
            handle.reset()?;
        }
        drop(handle);

        // Last sequential zone: write, verify, reset
        print!("  Last seq zone {last_seq_idx}: write 0xDD... ");
        let mut handle = ZoneHandle::new(dev.clone(), ZoneIndex::new(last_seq_idx as u32))?;
        handle.write_sequential(&vec![0xDDu8; 4096])?;
        dev.read_at(last_seq.start, &mut buf)?;
        if buf.iter().all(|&b| b == 0xDD) {
            println!("OK, reset... ");
            handle.reset()?;
            let zone = handle.report()?;
            if zone.condition == ZoneCondition::Empty {
                print!("    Reset OK, rewrite 0x44... ");
                handle.write_sequential(&vec![0x44u8; 4096])?;
                dev.read_at(last_seq.start, &mut buf)?;
                if buf.iter().all(|&b| b == 0x44) {
                    println!("OK");
                    passed += 1;
                } else {
                    println!("FAIL (rewrite verify)");
                    failed += 1;
                }
                handle.reset()?;
            } else {
                println!("FAIL (not empty after reset)");
                failed += 1;
            }
        } else {
            println!("FAIL (initial verify)");
            failed += 1;
            handle.reset()?;
        }
    } else {
        println!("  Skipping sequential tests (need >= 2 empty sequential zones)");
    }

    println!();
    println!("=== Results ===");
    println!("  {passed} passed, {failed} failed");
    if failed > 0 {
        process::exit(1);
    }
    Ok(())
}

// ============================================================
// Bench subcommand
// ============================================================

#[allow(clippy::too_many_arguments)]
fn run_bench(
    path: &Path,
    threads: u32,
    zones_per_thread: u32,
    buf_size_kib: u32,
    o_direct: bool,
    use_writev: bool,
    do_fsync: bool,
    skip_confirm: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let total_zones_needed = threads * zones_per_thread;
    let buf_size = (buf_size_kib as usize) * 1024;

    if o_direct && !buf_size.is_multiple_of(4096) {
        return Err(format!(
            "O_DIRECT requires buffer size to be a multiple of 4 KiB, got {buf_size_kib} KiB"
        )
        .into());
    }

    // sysfs is Linux-only; use device_info for zone size on all platforms.
    let props = sysfs::device_properties(path).ok();
    let max_open_zones = props.as_ref().and_then(|p| p.limits.max_open_zones);

    if let Some(max_open) = max_open_zones
        && threads > max_open
    {
        return Err(format!(
            "requested {} threads but device only supports {} open zones. Use --threads {}",
            threads, max_open, max_open
        )
        .into());
    }

    // Open device early so we can get zone size via ioctl
    let dev = if o_direct {
        Arc::new(
            ZonedDevice::builder(path)
                .direct_io()
                .validate_all()
                .open()?,
        )
    } else {
        Arc::new(
            ZonedDevice::builder(path)
                .writable()
                .validate_all()
                .open()?,
        )
    };
    let info = dev.device_info()?;
    let zone_size = info.zone_size;

    let zone_size_bytes = zone_size.to_bytes();
    let data_per_zone_mib = zone_size_bytes / (1024 * 1024);
    let total_data_mib = data_per_zone_mib * total_zones_needed as u64;

    println!("=== Benchmark Configuration ===");
    if let Some(ref p) = props {
        println!("  Model:               {}", p.model);
    }
    println!(
        "  Zone size:           {} MiB ({} sectors)",
        data_per_zone_mib, zone_size
    );
    println!("  Writer threads:      {threads}");
    println!("  Zones per thread:    {zones_per_thread}");
    println!("  Total zones:         {total_zones_needed}");
    println!("  Write buffer:        {buf_size_kib} KiB");
    println!(
        "  O_DIRECT:            {}",
        if o_direct { "yes" } else { "no" }
    );
    println!(
        "  Vectored I/O:        {}",
        if use_writev {
            "yes (writev_sequential)"
        } else {
            "no"
        }
    );
    println!(
        "  fsync:               {}",
        if do_fsync { "yes" } else { "no" }
    );
    println!("  Total data:          {total_data_mib} MiB");
    println!("  Max open zones:      {}", format_limit(max_open_zones));
    println!();

    if !skip_confirm {
        println!(
            "WARNING: This will RESET and WRITE to {} zones on {}.",
            total_zones_needed,
            path.display()
        );
        confirm()?;
    }

    let allocator = ZoneAllocator::new(dev.clone());

    println!("Allocating {total_zones_needed} zones...");
    let mut thread_zones: Vec<Vec<ZoneHandle>> = Vec::with_capacity(threads as usize);

    for t in 0..threads {
        let mut handles = Vec::with_capacity(zones_per_thread as usize);
        for _ in 0..zones_per_thread {
            let mut handle = allocator.allocate().map_err(|e| {
                format!("failed to allocate zone for thread {t}: {e} (not enough empty zones?)")
            })?;
            handle.reset().map_err(|e| {
                format!(
                    "failed to reset zone {} for thread {t}: {e}",
                    handle.zone_index()
                )
            })?;
            handles.push(handle);
        }
        thread_zones.push(handles);
    }

    let allocated = allocator.allocated_zones();
    println!(
        "  Allocated zones: {:?}",
        &allocated[..allocated.len().min(20)]
    );
    if allocated.len() > 20 {
        println!("  ... and {} more", allocated.len() - 20);
    }
    println!();

    let total_bytes_written = Arc::new(AtomicU64::new(0));
    let threads_done = Arc::new(AtomicU64::new(0));

    println!("=== Starting Benchmark ===");
    println!("  {threads} threads, each writing {zones_per_thread} full zones");
    println!();

    let start = Instant::now();
    let mut join_handles = Vec::with_capacity(threads as usize);

    for (thread_id, handles) in thread_zones.into_iter().enumerate() {
        let bytes_counter = total_bytes_written.clone();
        let done_counter = threads_done.clone();
        let buf = alloc_aligned_buf(buf_size, ((thread_id & 0xFF) as u8).wrapping_add(0xA0));
        let jh = thread::spawn(move || -> Result<ThreadResult, String> {
            let thread_start = Instant::now();
            let mut thread_bytes: u64 = 0;
            let mut zones_written = 0u32;

            for mut handle in handles {
                let zone_start_time = Instant::now();
                let zone_idx = handle.zone_index();
                let capacity_bytes = handle.capacity().to_bytes();
                let mut zone_bytes: u64 = 0;

                while zone_bytes < capacity_bytes {
                    let remaining = (capacity_bytes - zone_bytes) as usize;
                    let write_len = remaining.min(buf.len());

                    let written = if use_writev {
                        let mid = write_len / 2;
                        let bufs = [
                            IoSlice::new(&buf[..mid]),
                            IoSlice::new(&buf[mid..write_len]),
                        ];
                        handle
                            .writev_sequential(&bufs)
                            .map_err(|e| format!("thread {thread_id} zone {zone_idx}: {e}"))?
                    } else {
                        handle
                            .write_sequential(&buf[..write_len])
                            .map_err(|e| format!("thread {thread_id} zone {zone_idx}: {e}"))?
                    };

                    zone_bytes += written as u64;
                    thread_bytes += written as u64;
                    bytes_counter.fetch_add(written as u64, Ordering::Relaxed);
                }

                let zone_elapsed = zone_start_time.elapsed();
                let zone_mib = zone_bytes as f64 / (1024.0 * 1024.0);
                let zone_rate = zone_mib / zone_elapsed.as_secs_f64();
                zones_written += 1;

                eprintln!(
                    "  [thread {thread_id:>2}] zone {zone_idx:>5}: \
                     {zone_mib:>8.1} MiB in {:.2}s ({zone_rate:>8.1} MiB/s)",
                    zone_elapsed.as_secs_f64()
                );
            }

            done_counter.fetch_add(1, Ordering::Relaxed);

            Ok(ThreadResult {
                thread_id,
                bytes_written: thread_bytes,
                zones_written,
                elapsed: thread_start.elapsed(),
            })
        });

        join_handles.push(jh);
    }

    let mut results = Vec::new();
    for jh in join_handles {
        match jh.join() {
            Ok(Ok(result)) => results.push(result),
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err("a writer thread panicked".into()),
        }
    }

    let writes_elapsed = start.elapsed();

    let mut fsync_elapsed = Duration::ZERO;
    if do_fsync {
        eprint!("  Flushing to disk (fsync)...");
        let fsync_start = Instant::now();
        dev.fsync()?;
        fsync_elapsed = fsync_start.elapsed();
        eprintln!(" {:.2}s", fsync_elapsed.as_secs_f64());
    }

    let total_elapsed = writes_elapsed + fsync_elapsed;
    let total_bytes = total_bytes_written.load(Ordering::Relaxed);
    let total_mib = total_bytes as f64 / (1024.0 * 1024.0);
    let aggregate_rate = total_mib / total_elapsed.as_secs_f64();

    println!();
    println!("=== Per-Thread Results ===");
    println!(
        "  {:>6}  {:>8}  {:>12}  {:>10}  {:>12}",
        "Thread", "Zones", "Data (MiB)", "Time (s)", "Rate (MiB/s)"
    );
    println!("  {}", "-".repeat(56));
    for r in &results {
        let mib = r.bytes_written as f64 / (1024.0 * 1024.0);
        let rate = mib / r.elapsed.as_secs_f64();
        println!(
            "  {:>6}  {:>8}  {:>12.1}  {:>10.2}  {:>12.1}",
            r.thread_id,
            r.zones_written,
            mib,
            r.elapsed.as_secs_f64(),
            rate
        );
    }

    println!();
    println!("=== Summary ===");
    println!("  Total data written:  {total_mib:.1} MiB");
    if do_fsync {
        println!(
            "  Write time:          {:.2}s",
            writes_elapsed.as_secs_f64()
        );
        println!("  Fsync time:          {:.2}s", fsync_elapsed.as_secs_f64());
    }
    println!("  Wall clock time:     {:.2}s", total_elapsed.as_secs_f64());
    println!("  Aggregate throughput: {aggregate_rate:.1} MiB/s");
    println!(
        "  Per-thread average:  {:.1} MiB/s",
        aggregate_rate / threads as f64
    );

    // Reset zones after benchmark
    println!();
    println!("Resetting zones...");
    for zone_idx in &allocated {
        let zone_start = zone_size * zone_idx.raw() as u64;
        dev.reset_zones(zone_start, zone_size)
            .map_err(|e| format!("failed to reset zone {zone_idx}: {e}"))?;
    }
    println!("  Reset {} zones", allocated.len());
    println!();
    println!("Done.");
    Ok(())
}

// ============================================================
// Helpers
// ============================================================

struct ThreadResult {
    thread_id: usize,
    bytes_written: u64,
    zones_written: u32,
    elapsed: Duration,
}

fn confirm() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    print!("Type 'yes' to continue: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if input.trim() != "yes" {
        return Err("Aborted.".into());
    }
    println!();
    Ok(())
}

fn parse_zone_range(s: &str) -> Result<(u32, u32), Box<dyn std::error::Error>> {
    if let Some((a, b)) = s.split_once('-') {
        let start: u32 = a.parse().map_err(|_| format!("invalid zone index: {a}"))?;
        let end: u32 = b.parse().map_err(|_| format!("invalid zone index: {b}"))?;
        if end < start {
            return Err(format!("invalid range: {start}-{end} (end < start)").into());
        }
        Ok((start, end))
    } else {
        let idx: u32 = s.parse().map_err(|_| format!("invalid zone index: {s}"))?;
        Ok((idx, idx))
    }
}

fn hex_dump(data: &[u8]) {
    for (i, chunk) in data.chunks(16).enumerate() {
        let offset = i * 16;
        print!("  {offset:08x}  ");
        for (j, byte) in chunk.iter().enumerate() {
            if j == 8 {
                print!(" ");
            }
            print!("{byte:02x} ");
        }
        for j in chunk.len()..16 {
            if j == 8 {
                print!(" ");
            }
            print!("   ");
        }
        print!(" |");
        for byte in chunk {
            if byte.is_ascii_graphic() || *byte == b' ' {
                print!("{}", *byte as char);
            } else {
                print!(".");
            }
        }
        println!("|");
    }
}

/// Allocate a page-aligned buffer filled with a pattern byte.
/// O_DIRECT requires memory alignment (typically 4096 bytes / page size).
fn alloc_aligned_buf(size: usize, fill: u8) -> Vec<u8> {
    const ALIGNMENT: usize = 4096;
    #[allow(clippy::expect_used)]
    let layout =
        std::alloc::Layout::from_size_align(size, ALIGNMENT).expect("invalid buffer layout");
    // SAFETY: layout has non-zero size and valid alignment. We initialize
    // all bytes immediately after allocation via write_bytes.
    let ptr = unsafe {
        let p = std::alloc::alloc(layout);
        if p.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        std::ptr::write_bytes(p, fill, size);
        p
    };
    // SAFETY: ptr was allocated with the global allocator, is non-null,
    // properly aligned, and all `size` bytes are initialized.
    unsafe { Vec::from_raw_parts(ptr, size, size) }
}

fn format_limit(value: Option<u32>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "unlimited".to_string(),
    }
}

/// Format a write pointer for display. `None` (conventional zones) shows as "-".
fn format_wp(wp: Option<Sector>) -> String {
    match wp {
        Some(s) => s.to_string(),
        None => "-".to_string(),
    }
}
