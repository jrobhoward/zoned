# HGST HMH7210A0AL — Zoned/SMR Disk Characterization

## Device Identity

| Property | Value |
|----------|-------|
| Model | HGST HMH7210A0AL (firmware T520) |
| Interface | SCSI ZBC (ATA ZAC over SAT) |
| SCSI address | `[1:0:0:0]` |
| Device path | `/dev/sda` |
| Raw capacity | ~10 TB (9,314 GiB usable) |
| Zone model | **Host-managed** |
| I/O scheduler | `mq-deadline` (mandatory for host-managed) |

## Zone Layout

| Property | Value |
|----------|-------|
| Zone size | 256 MiB (524,288 sectors of 512 bytes) |
| Total zones | 37,256 |
| Conventional zones | 378 (zones 0–377, ~96.8 GiB) |
| Sequential write required | 36,878 (zones 378–37,255) |
| Zone append max | 688,128 bytes |
| Max open zones | 16 |
| Max active zones | unlimited |
| Capacity overhead | 0 (capacity == length for all zones) |

The first 378 zones are conventional (random-writable, no write pointer). The
remaining 36,878 zones are sequential-write-required — data must be written at
the zone's write pointer, and zones must be explicitly reset before rewriting.

## Performance Characteristics

All benchmarks used `zone_info bench` with the `zoned` Rust library on
Linux 6.8 (`mq-deadline` scheduler). The disk is a single-actuator 7200 RPM
drive — one head, one platter surface active at a time.

### O_DIRECT (bypass page cache — true disk throughput)

| Threads | Zones | Buffer | Aggregate MiB/s | Per-thread MiB/s | Wall time | Notes |
|---------|-------|--------|-----------------|------------------|-----------|-------|
| 1 | 1 | 128K | **63.7** | 63.7 | 4.0s | Baseline |
| 1 | 4 | 128K | **59.1** | 59.1 | 17.3s | Slight drop on later zones |
| 1 | 2 | 512K | **72.6** | 72.6 | 7.1s | Larger buffer helps |
| 1 | 2 | 1024K | **72.4** | 72.4 | 7.1s | Diminishing returns past 512K |
| 4 | 1ea | 128K | **61.4** | 15.3 | 16.7s | Seeking kills per-thread rate |
| 8 | 1ea | 128K | **50.9** | 6.4 | 40.2s | Severe seek penalty |
| 16 | 1ea | 128K | **13.6** | 0.8 | 302s | Catastrophic — 78% loss |

### Buffered I/O + fsync (page cache + kernel scheduler)

| Threads | Zones | Buffer | Aggregate MiB/s | Write time | Fsync time | Total |
|---------|-------|--------|-----------------|------------|------------|-------|
| 1 | 4 | 512K | **68.4** | 0.46s | 14.52s | 14.98s |
| 4 | 1ea | 512K | **69.3** | 0.52s | 14.26s | 14.78s |
| 8 | 1ea | 512K | **70.0** | 0.87s | 28.41s | 29.28s |

### Buffered I/O without fsync (page cache only — not real throughput)

These numbers reflect memory bandwidth, not disk throughput. Included only to
show that without fsync, buffered benchmarks are meaningless for storage.

| Threads | Reported MiB/s | Actual disk MiB/s |
|---------|---------------|-------------------|
| 1 | 2,224 | ~68 (measured via fsync) |
| 4 | 1,967 | ~69 (measured via fsync) |
| 8 | 2,354 | ~70 (measured via fsync) |

## Key Findings

### 1. Single-writer is optimal for O_DIRECT

With O_DIRECT, concurrent writes to different zones cause head seeking between
zone positions on the platter. Aggregate throughput does not improve — it
degrades sharply:

- 1 thread: 64 MiB/s
- 4 threads: 61 MiB/s (same aggregate, 4x slower per thread)
- 8 threads: 51 MiB/s (20% aggregate loss)
- 16 threads: 14 MiB/s (78% aggregate loss — 5 minutes for 4 GiB)

### 2. mq-deadline scheduler recovers concurrent write performance

With buffered I/O + fsync, the `mq-deadline` scheduler reorders writes to
minimize seeking. Result: 4 concurrent threads achieve **69 MiB/s aggregate**
(vs 61 MiB/s with O_DIRECT) — the scheduler batches writes by zone, effectively
serializing disk access while allowing application-level concurrency.

The 8-thread case (70 MiB/s) confirms the scheduler maintains throughput even
with more concurrent writers. The fsync time scales linearly with data volume
(28s for 2 GiB vs 14s for 1 GiB), indicating the disk writes at a steady rate
regardless of how many threads produced the data.

### 3. Buffer size sweet spot is 512 KiB

| Buffer | O_DIRECT MiB/s |
|--------|---------------|
| 128 KiB | 62 |
| 512 KiB | 73 (+18%) |
| 1024 KiB | 72 (no gain) |

Larger buffers reduce syscall overhead and allow the disk to write longer
contiguous runs. Returns diminish past 512 KiB.

### 4. Outer zones are faster

Zone 378 (first sequential zone) consistently writes at 69–81 MiB/s while
subsequent zones drop to 55–66 MiB/s. The outer tracks of the disk have higher
linear velocity, yielding more data per rotation.

### 5. Buffered I/O has a stale page cache hazard

Resetting a zone via ioctl while the kernel page cache holds dirty pages for
that zone causes EIO on the next fsync — the kernel tries to flush pages
targeting write pointer positions that no longer exist. Mitigations:
- Always `fsync` before resetting a zone when using buffered I/O
- Or use O_DIRECT to avoid the page cache entirely

## Optimal Usage Patterns

### For maximum throughput (e.g., bulk ingest, backup)
- Open with buffered I/O (no O_DIRECT)
- Write one zone at a time, fill completely, then move to next
- Use 512 KiB write buffers
- `fsync` before resetting any zone
- Let `mq-deadline` handle scheduling if multiple logical writers exist

### For deterministic latency (e.g., database, log-structured storage)
- Open with O_DIRECT
- Single writer thread to a single zone at a time
- 512 KiB aligned write buffers
- Accept ~70 MiB/s sustained throughput

### For metadata / indexes
- Use conventional zones 0–377 (~97 GiB of random-writable space)
- These behave like a normal disk — no write pointer constraints

### What NOT to do
- Don't write to many zones concurrently with O_DIRECT on a single-actuator HDD
- Don't benchmark with buffered I/O without fsync — the numbers are fiction
- Don't reset zones without fsyncing first when using buffered I/O
- Don't exceed `max_open_zones` (16) — the device will reject the operation

## Hardware Context

This is a single-actuator 7200 RPM SMR (Shingled Magnetic Recording) hard
drive. The concurrent-write penalties are inherent to spinning media with one
head — seek time dominates when interleaving writes across zones.

NVMe ZNS SSDs have no seek penalty and would scale linearly with thread count
using the same `ZoneHandle`/`ZoneAllocator` API. The concurrency primitives in
the `zoned` library are designed for that use case as well.

## Benchmark Reproduction

```bash
# Build
cargo build --release --example zone_info

# Reset all zones first
sudo ./target/release/examples/zone_info reset-all /dev/sda --yes

# O_DIRECT single-thread baseline
sudo ./target/release/examples/zone_info bench /dev/sda --o-direct -t 1 -z 4 -b 512 --yes

# Buffered + fsync comparison
sudo ./target/release/examples/zone_info bench /dev/sda -t 1 -z 4 -b 512 --fsync --yes
sudo ./target/release/examples/zone_info bench /dev/sda -t 4 -z 1 -b 512 --fsync --yes
sudo ./target/release/examples/zone_info bench /dev/sda -t 8 -z 1 -b 512 --fsync --yes
```
