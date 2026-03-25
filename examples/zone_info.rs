//! Zoned block device information and benchmark tool.
//!
//! # Usage
//!
//! ```bash
//! # Show zone information (read-only)
//! sudo ./target/release/examples/zone_info info /dev/sda
//! sudo ./target/release/examples/zone_info info /dev/sda --all-zones
//!
//! # Reset all sequential zones (DESTRUCTIVE)
//! sudo ./target/release/examples/zone_info reset-all /dev/sda
//!
//! # Concurrent write benchmark (DESTRUCTIVE - resets zones!)
//! sudo ./target/release/examples/zone_info bench /dev/sda
//! sudo ./target/release/examples/zone_info bench /dev/sda -t 8 -z 4 -b 256
//! sudo ./target/release/examples/zone_info bench /dev/sda --o-direct  # bypass page cache
//! ```

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use zoned::{
    DeviceModel, Sector, ZoneAllocator, ZoneCondition, ZoneHandle, ZoneType, ZonedDevice, sysfs,
};

#[derive(Parser)]
#[command(name = "zone_info")]
#[command(about = "Zoned block device information and benchmark tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Display device and zone information (read-only)
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

    /// Reset all sequential zones on the device (DESTRUCTIVE)
    ResetAll {
        /// Path to the block device (e.g. /dev/sda)
        device: PathBuf,

        /// Skip the "are you sure?" confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Concurrent sequential write benchmark (DESTRUCTIVE - resets zones!)
    Bench {
        /// Path to the block device (e.g. /dev/sda)
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

        /// Use O_DIRECT to bypass the page cache (real disk throughput)
        #[arg(long)]
        o_direct: bool,

        /// Call fsync after all writes complete (measures write + flush time)
        #[arg(long)]
        fsync: bool,

        /// Skip the "are you sure?" confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Info {
            device,
            all_zones,
            batch_size,
        } => run_info(device, *all_zones, *batch_size),
        Commands::ResetAll { device, yes } => run_reset_all(device, *yes),
        Commands::Bench {
            device,
            threads,
            zones_per_thread,
            buf_size_kib,
            o_direct,
            fsync,
            yes,
        } => run_bench(
            device,
            *threads,
            *zones_per_thread,
            *buf_size_kib,
            *o_direct,
            *fsync,
            *yes,
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
    println!("Device: {}", path.display());
    println!();

    validate_block_device(path)?;
    check_not_mounted(path)?;
    check_no_partitions(path)?;

    let model = sysfs::device_model(path)?;
    if model == DeviceModel::None {
        return Err(format!(
            "{} is not a zoned device (sysfs reports 'none')",
            path.display()
        )
        .into());
    }

    let props = sysfs::device_properties(path)?;
    println!("=== Device Properties (sysfs) ===");
    println!("  Model:               {}", props.model);
    println!(
        "  Zone size:           {} sectors ({} MiB)",
        props.chunk_sectors,
        props.chunk_sectors.to_bytes() / (1024 * 1024)
    );
    println!("  Number of zones:     {}", props.nr_zones);
    println!(
        "  Zone append max:     {} bytes",
        props.zone_append_max_bytes
    );
    println!(
        "  Max open zones:      {}",
        format_limit(props.max_open_zones)
    );
    println!(
        "  Max active zones:    {}",
        format_limit(props.max_active_zones)
    );
    println!();

    let dev = ZonedDevice::open(path)?;
    let info = dev.device_info()?;

    println!("=== Device Info (ioctl) ===");
    println!("  Zone size:           {} sectors", info.zone_size);
    println!("  Number of zones:     {}", info.nr_zones);
    println!();

    println!("=== Zone Report ===");
    println!(
        "  Fetching all {} zones (batch size {batch_size})...",
        info.nr_zones
    );

    let zones = dev.report_all_zones(batch_size)?;
    println!("  Retrieved {} zones", zones.len());
    println!();

    let mut cond_counts: HashMap<ZoneCondition, u32> = HashMap::new();
    let mut total_capacity_sectors = Sector::ZERO;
    let mut total_len_sectors = Sector::ZERO;
    let mut conventional_count: u32 = 0;
    let mut seq_required_count: u32 = 0;
    let mut seq_preferred_count: u32 = 0;

    for zone in &zones {
        *cond_counts.entry(zone.condition).or_insert(0) += 1;

        total_capacity_sectors += zone.capacity;
        total_len_sectors += zone.len;

        match zone.zone_type {
            ZoneType::Conventional => conventional_count += 1,
            ZoneType::SequentialWriteRequired => seq_required_count += 1,
            ZoneType::SequentialWritePreferred => seq_preferred_count += 1,
        }
    }

    let total_capacity_gib = total_capacity_sectors.to_bytes() / (1024 * 1024 * 1024);
    let total_len_gib = total_len_sectors.to_bytes() / (1024 * 1024 * 1024);

    println!("=== Zone Type Summary ===");
    println!("  Conventional:             {:>6}", conventional_count);
    println!("  Sequential Write Required:{:>6}", seq_required_count);
    println!("  Sequential Write Preferred:{:>5}", seq_preferred_count);
    println!("  Total:                    {:>6}", zones.len());
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

    println!("=== Capacity ===");
    println!(
        "  Total zone length:   {} sectors ({} GiB)",
        total_len_sectors, total_len_gib
    );
    println!(
        "  Total usable capacity: {} sectors ({} GiB)",
        total_capacity_sectors, total_capacity_gib
    );
    if total_len_sectors > total_capacity_sectors {
        let overhead = total_len_sectors - total_capacity_sectors;
        let overhead_mib = overhead.to_bytes() / (1024 * 1024);
        println!(
            "  Capacity overhead:   {} sectors ({} MiB)",
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

        for (i, zone) in zones.iter().enumerate() {
            println!(
                "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>25}  {:>20}",
                i,
                zone.start,
                zone.len,
                zone.capacity,
                zone.write_pointer,
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
// Reset-all subcommand
// ============================================================

fn run_reset_all(path: &Path, skip_confirm: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("Device: {}", path.display());
    println!();

    validate_block_device(path)?;
    check_not_mounted(path)?;
    check_no_partitions(path)?;

    let model = sysfs::device_model(path)?;
    if model == DeviceModel::None {
        return Err(format!("{} is not a zoned device", path.display()).into());
    }

    let dev = ZonedDevice::open_writable(path)?;
    let info = dev.device_info()?;
    let zones = dev.report_all_zones(512)?;

    let seq_zones: Vec<_> = zones
        .iter()
        .filter(|z| z.zone_type == ZoneType::SequentialWriteRequired)
        .collect();

    let non_empty: Vec<_> = seq_zones
        .iter()
        .filter(|z| z.condition != ZoneCondition::Empty)
        .collect();

    println!("  Total zones:       {}", zones.len());
    println!("  Sequential zones:  {}", seq_zones.len());
    println!("  Non-empty seq:     {}", non_empty.len());
    println!("  Already empty:     {}", seq_zones.len() - non_empty.len());
    println!();

    if non_empty.is_empty() {
        println!("All sequential zones are already empty. Nothing to reset.");
        return Ok(());
    }

    if !skip_confirm {
        println!(
            "WARNING: This will RESET {} sequential zones on {}.",
            non_empty.len(),
            path.display()
        );
        println!("         All data in those zones will be destroyed.");
        println!();
        print!("Type 'yes' to continue: ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim() != "yes" {
            println!("Aborted.");
            return Ok(());
        }
        println!();
    }

    // Reset all sequential zones in one operation covering the entire device
    let total_sectors = info.zone_size * info.nr_zones as u64;
    println!("Resetting all sequential zones...");
    let start = Instant::now();
    dev.reset_zones(Sector::ZERO, total_sectors)?;
    let elapsed = start.elapsed();

    println!(
        "  Reset {} sequential zones in {:.2}s",
        seq_zones.len(),
        elapsed.as_secs_f64()
    );
    println!();
    println!("Done.");
    Ok(())
}

// ============================================================
// Bench subcommand
// ============================================================

fn run_bench(
    path: &Path,
    threads: u32,
    zones_per_thread: u32,
    buf_size_kib: u32,
    o_direct: bool,
    do_fsync: bool,
    skip_confirm: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Device: {}", path.display());
    println!();

    validate_block_device(path)?;
    check_not_mounted(path)?;
    check_no_partitions(path)?;

    let model = sysfs::device_model(path)?;
    if model == DeviceModel::None {
        return Err(format!("{} is not a zoned device", path.display()).into());
    }

    let props = sysfs::device_properties(path)?;
    let total_zones_needed = threads * zones_per_thread;
    let buf_size = (buf_size_kib as usize) * 1024;

    if o_direct && !buf_size.is_multiple_of(4096) {
        return Err(format!(
            "O_DIRECT requires buffer size to be a multiple of 4 KiB, got {buf_size_kib} KiB"
        )
        .into());
    }

    // Validate against device limits
    if props.max_open_zones > 0 && threads > props.max_open_zones {
        return Err(format!(
            "requested {} threads but device only supports {} open zones simultaneously. \
             Use --threads {}",
            threads, props.max_open_zones, props.max_open_zones
        )
        .into());
    }

    let zone_size_bytes = props.chunk_sectors.to_bytes();
    let data_per_zone_mib = zone_size_bytes / (1024 * 1024);
    let total_data_mib = data_per_zone_mib * total_zones_needed as u64;

    println!("=== Benchmark Configuration ===");
    println!("  Model:               {}", props.model);
    println!(
        "  Zone size:           {} MiB ({} sectors)",
        data_per_zone_mib, props.chunk_sectors
    );
    println!("  Writer threads:      {threads}");
    println!("  Zones per thread:    {zones_per_thread}");
    println!("  Total zones:         {total_zones_needed}");
    println!("  Write buffer:        {buf_size_kib} KiB");
    println!(
        "  O_DIRECT:            {}",
        if o_direct {
            "yes (bypass page cache)"
        } else {
            "no (buffered)"
        }
    );
    println!(
        "  fsync:               {}",
        if do_fsync {
            "yes (wait for flush after writes)"
        } else {
            "no"
        }
    );
    println!("  Data per zone:       {data_per_zone_mib} MiB");
    println!("  Total data:          {total_data_mib} MiB");
    println!(
        "  Max open zones:      {}",
        format_limit(props.max_open_zones)
    );
    println!();

    if !skip_confirm {
        println!(
            "WARNING: This will RESET and WRITE to {} zones on {}.",
            total_zones_needed,
            path.display()
        );
        println!("         All data in those zones will be destroyed.");
        println!();
        print!("Type 'yes' to continue: ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim() != "yes" {
            println!("Aborted.");
            return Ok(());
        }
        println!();
    }

    // Open device writable (optionally with O_DIRECT)
    let dev = if o_direct {
        Arc::new(ZonedDevice::open_direct(path)?)
    } else {
        Arc::new(ZonedDevice::open_writable(path)?)
    };
    let allocator = ZoneAllocator::new(dev.clone());

    // Allocate zones for all threads upfront
    println!("Allocating {total_zones_needed} zones...");
    let mut thread_zones: Vec<Vec<ZoneHandle>> = Vec::with_capacity(threads as usize);

    for t in 0..threads {
        let mut handles = Vec::with_capacity(zones_per_thread as usize);
        for _ in 0..zones_per_thread {
            let mut handle = allocator.allocate().map_err(|e| {
                format!("failed to allocate zone for thread {t}: {e} (not enough empty zones?)")
            })?;
            // Reset each zone to ensure it's empty and write pointer is at start
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

    // Shared counters for live progress
    let total_bytes_written = Arc::new(AtomicU64::new(0));
    let threads_done = Arc::new(AtomicU64::new(0));

    println!("=== Starting Benchmark ===");
    println!("  {threads} threads, each writing {zones_per_thread} full zones sequentially");
    println!();

    let start = Instant::now();

    // Spawn writer threads
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

                // Write the zone to full capacity
                while zone_bytes < capacity_bytes {
                    let remaining = (capacity_bytes - zone_bytes) as usize;
                    let write_len = remaining.min(buf.len());
                    let written = handle
                        .write_sequential(&buf[..write_len])
                        .map_err(|e| format!("thread {thread_id} zone {zone_idx}: {e}"))?;
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

    // Collect results
    let mut results = Vec::new();
    for jh in join_handles {
        match jh.join() {
            Ok(Ok(result)) => results.push(result),
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => return Err("a writer thread panicked".into()),
        }
    }

    let writes_elapsed = start.elapsed();

    // Optionally fsync to flush page cache to disk (included in total timing)
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
        let zone_start = props.chunk_sectors * zone_idx.raw() as u64;
        dev.reset_zones(zone_start, props.chunk_sectors)
            .map_err(|e| format!("failed to reset zone {zone_idx}: {e}"))?;
    }
    println!("  Reset {} zones", allocated.len());

    println!();
    println!("Done.");
    Ok(())
}

struct ThreadResult {
    thread_id: usize,
    bytes_written: u64,
    zones_written: u32,
    elapsed: Duration,
}

/// Allocate a page-aligned buffer filled with a pattern byte.
/// O_DIRECT requires memory alignment (typically 4096 bytes / page size).
fn alloc_aligned_buf(size: usize, fill: u8) -> Vec<u8> {
    const ALIGNMENT: usize = 4096;
    // SAFETY: ALIGNMENT is a power of 2 and size > 0 for any reasonable buffer.
    // unwrap is acceptable here as this is example code, not library code.
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
    // properly aligned, and all `size` bytes are initialized. The Vec
    // takes ownership and will deallocate via the global allocator.
    unsafe { Vec::from_raw_parts(ptr, size, size) }
}

// ============================================================
// Validation helpers
// ============================================================

fn validate_block_device(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let metadata = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;

    let file_type = metadata.mode() & 0o170000;
    if file_type != 0o060000 {
        return Err(format!(
            "{} is not a block device (mode: {:#o})",
            path.display(),
            metadata.mode()
        )
        .into());
    }

    println!("  [OK] {} is a block device", path.display());
    Ok(())
}

fn check_not_mounted(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let dev_stat = fs::metadata(path)?;
    let dev_rdev = dev_stat.rdev();
    let dev_major = (dev_rdev >> 8) as u32;
    let dev_minor = (dev_rdev & 0xFF) as u32;

    let mountinfo = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();

    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        if let Some((maj_str, min_str)) = fields[2].split_once(':')
            && let (Ok(maj), Ok(min)) = (maj_str.parse::<u32>(), min_str.parse::<u32>())
            && maj == dev_major
            && min == dev_minor
        {
            let mount_point = if fields.len() > 4 {
                fields[4]
            } else {
                "unknown"
            };
            return Err(format!(
                "{} is mounted at {} -- refusing to operate on a mounted device",
                path.display(),
                mount_point
            )
            .into());
        }
    }

    println!("  [OK] {} is not mounted", path.display());
    Ok(())
}

fn check_no_partitions(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let dev_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("cannot determine device name from {}", path.display()))?;

    let sysfs_dir = format!("/sys/block/{dev_name}");
    if !Path::new(&sysfs_dir).exists() {
        return Err(format!(
            "{} does not appear in /sys/block/ -- is it a partition? \
             Use the whole disk device (e.g. /dev/sda, not /dev/sda1)",
            path.display()
        )
        .into());
    }

    let entries = fs::read_dir(&sysfs_dir)?;
    let mut partitions = Vec::new();

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(dev_name) {
            let partition_file = entry.path().join("partition");
            if partition_file.exists() {
                partitions.push(name_str.to_string());
            }
        }
    }

    if !partitions.is_empty() {
        partitions.sort();
        return Err(format!(
            "{} has partitions: {} -- zoned devices should not be partitioned",
            path.display(),
            partitions.join(", ")
        )
        .into());
    }

    println!("  [OK] {} has no partitions", path.display());
    Ok(())
}

// ============================================================
// Formatting helpers
// ============================================================

fn format_limit(value: u32) -> String {
    if value == 0 {
        "unlimited".to_string()
    } else {
        value.to_string()
    }
}
