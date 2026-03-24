//! Zoned block device information tool.
//!
//! Validates that a device is a zoned block device, then prints detailed
//! zone information including device properties and a zone summary.
//!
//! # Usage
//!
//! ```bash
//! sudo cargo run --example zone_info -- /dev/sda
//! sudo cargo run --example zone_info -- /dev/sda --all-zones
//! ```

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;
use zoned::{DeviceModel, ZoneCondition, ZoneType, ZonedDevice, sysfs};

#[derive(Parser)]
#[command(name = "zone_info")]
#[command(about = "Display information about a zoned block device")]
struct Cli {
    /// Path to the block device (e.g. /dev/sda)
    device: PathBuf,

    /// Print every zone (not just the summary)
    #[arg(long)]
    all_zones: bool,

    /// Number of zones to report per batch (default: 512)
    #[arg(long, default_value = "512")]
    batch_size: u32,
}

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(&cli) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let path = &cli.device;

    // --- Step 1: Basic path validation ---
    println!("Device: {}", path.display());
    println!();

    validate_block_device(path)?;
    check_not_mounted(path)?;
    check_no_partitions(path)?;

    // --- Step 2: Sysfs validation (is it actually zoned?) ---
    let model = sysfs::device_model(path)?;
    if model == DeviceModel::None {
        return Err(format!(
            "{} is not a zoned device (sysfs reports 'none')",
            path.display()
        )
        .into());
    }

    // --- Step 3: Print device properties from sysfs ---
    let props = sysfs::device_properties(path)?;
    println!("=== Device Properties (sysfs) ===");
    println!("  Model:               {}", format_model(props.model));
    println!(
        "  Zone size:           {} sectors ({} MiB)",
        props.chunk_sectors,
        props.chunk_sectors as u64 * 512 / (1024 * 1024)
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

    // --- Step 4: Open device and get ioctl info ---
    let dev = ZonedDevice::open(path)?;
    let info = dev.device_info()?;

    println!("=== Device Info (ioctl) ===");
    println!("  Zone size:           {} sectors", info.zone_size);
    println!("  Number of zones:     {}", info.nr_zones);
    println!();

    // --- Step 5: Report all zones and summarize ---
    println!("=== Zone Report ===");
    println!(
        "  Fetching all {} zones (batch size {})...",
        info.nr_zones, cli.batch_size
    );

    let zones = dev.report_all_zones(cli.batch_size)?;
    println!("  Retrieved {} zones", zones.len());
    println!();

    // Count zones by type
    let mut type_counts: HashMap<&str, u32> = HashMap::new();
    let mut cond_counts: HashMap<&str, u32> = HashMap::new();
    let mut total_capacity_sectors: u64 = 0;
    let mut total_len_sectors: u64 = 0;
    let mut conventional_count: u32 = 0;
    let mut seq_required_count: u32 = 0;
    let mut seq_preferred_count: u32 = 0;

    for zone in &zones {
        let type_name = format_zone_type(zone.zone_type);
        *type_counts.entry(type_name).or_insert(0) += 1;

        let cond_name = format_zone_condition(zone.condition);
        *cond_counts.entry(cond_name).or_insert(0) += 1;

        total_capacity_sectors += zone.capacity;
        total_len_sectors += zone.len;

        match zone.zone_type {
            ZoneType::Conventional => conventional_count += 1,
            ZoneType::SequentialWriteRequired => seq_required_count += 1,
            ZoneType::SequentialWritePreferred => seq_preferred_count += 1,
        }
    }

    let total_capacity_gib = total_capacity_sectors * 512 / (1024 * 1024 * 1024);
    let total_len_gib = total_len_sectors * 512 / (1024 * 1024 * 1024);

    println!("=== Zone Type Summary ===");
    println!("  Conventional:             {:>6}", conventional_count);
    println!("  Sequential Write Required:{:>6}", seq_required_count);
    println!("  Sequential Write Preferred:{:>5}", seq_preferred_count);
    println!("  Total:                    {:>6}", zones.len());
    println!();

    println!("=== Zone Condition Summary ===");
    let cond_order = [
        "Not Write Pointer",
        "Empty",
        "Implicitly Open",
        "Explicitly Open",
        "Closed",
        "Full",
        "Read Only",
        "Offline",
    ];
    for name in &cond_order {
        let count = cond_counts.get(name).copied().unwrap_or(0);
        if count > 0 {
            println!("  {name:<25} {count:>6}");
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
        let overhead_mib = overhead * 512 / (1024 * 1024);
        println!(
            "  Capacity overhead:   {} sectors ({} MiB)",
            overhead, overhead_mib
        );
    }
    println!();

    // --- Step 6: Optionally print all zones ---
    if cli.all_zones {
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
                format_zone_type(zone.zone_type),
                format_zone_condition(zone.condition),
            );
        }
        println!();
    }

    println!("Done.");
    Ok(())
}

// --- Validation helpers ---

fn validate_block_device(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let metadata = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;

    // On Linux, block devices have file type "block device"
    // Check using the mode bits: S_IFBLK = 0o60000
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
        // Field index 2 is "major:minor"
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
        // Not a whole-disk device, might be a partition itself
        return Err(format!(
            "{} does not appear in /sys/block/ -- is it a partition? \
             Use the whole disk device (e.g. /dev/sda, not /dev/sda1)",
            path.display()
        )
        .into());
    }

    // Look for partition subdirectories (e.g. /sys/block/sda/sda1/)
    let entries = fs::read_dir(&sysfs_dir)?;
    let mut partitions = Vec::new();

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Partition dirs start with the device name and have a "partition" file
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

// --- Formatting helpers ---

fn format_model(model: DeviceModel) -> &'static str {
    match model {
        DeviceModel::None => "none (not zoned)",
        DeviceModel::HostAware => "host-aware",
        DeviceModel::HostManaged => "host-managed",
    }
}

fn format_limit(value: u32) -> String {
    if value == 0 {
        "unlimited".to_string()
    } else {
        value.to_string()
    }
}

fn format_zone_type(zt: ZoneType) -> &'static str {
    match zt {
        ZoneType::Conventional => "Conventional",
        ZoneType::SequentialWriteRequired => "Sequential Write Required",
        ZoneType::SequentialWritePreferred => "Sequential Write Preferred",
    }
}

fn format_zone_condition(zc: ZoneCondition) -> &'static str {
    match zc {
        ZoneCondition::NotWritePointer => "Not Write Pointer",
        ZoneCondition::Empty => "Empty",
        ZoneCondition::ImplicitlyOpen => "Implicitly Open",
        ZoneCondition::ExplicitlyOpen => "Explicitly Open",
        ZoneCondition::Closed => "Closed",
        ZoneCondition::ReadOnly => "Read Only",
        ZoneCondition::Full => "Full",
        ZoneCondition::Offline => "Offline",
    }
}
