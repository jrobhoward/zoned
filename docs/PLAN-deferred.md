# Deferred: libzbc parity & future capabilities

Items identified during the libzbc API comparison that are not supported by
the current target hardware (HGST HMH7210A0AL, ZBC-1 host-managed) and were
deliberately deferred. Revisit when testing against ZBC-2 or NVMe ZNS devices.

## Zone domains & realms (ZBC-2 / ZAC-2)

The largest gap vs libzbc. Enables dynamic zone type conversion (e.g.
conventional ↔ sequential) on devices that support it.

- `report_domains` / `list_domains` — enumerate zone domains
- `report_realms` / `list_realms` — enumerate zone realms
- `zone_activate` / `zone_query` — activate zones in a domain
- `zone_activation_ctl` — control max activation, URSWRZ
- Supporting types: `ZoneDomain`, `ZoneRealm`, `ActivationResult`
- New zone types: `SequentialOrBeforeRequired` (0x04), `Gap` (0x05)
- New zone condition: `Inactive` (0x05)

**Blocked on**: no test hardware. The HGST drive does not report
`ZBC_ZONE_REALMS_SUPPORT` or `ZBC_ZONE_DOMAINS_SUPPORT`.

## SCSI sense data

libzbc exposes `zbc_errno_ext` with sense key, ASC/ASCQ, and extended error
info. The zoned crate uses the kernel block layer (ioctls), which maps errors
to `errno`. Sense data would require SCSI passthrough (`SG_IO`), which is a
different access model.

- Only useful if we add a SCSI/ATA passthrough backend
- Would need new error variants or a structured error detail field

## Device statistics

libzbc's `zbc_get_zbd_stats` returns 12 counters (max open zones reached,
zones emptied, suboptimal write commands, read/write rule failures, etc.).

- Likely exposed via sysfs or a device-specific ioctl
- Nice for monitoring/observability, not critical for operation

## Driver backend selection

libzbc can force SCSI (`ZBC_O_DRV_SCSI`) or ATA (`ZBC_O_DRV_ATA`) transport.
The zoned crate goes through the kernel block layer exclusively, which is the
correct approach for Linux — the kernel handles SAT (SCSI-ATA Translation)
transparently.

- Only relevant if adding direct SCSI/ATA passthrough
- No current use case

## Library logging

libzbc has `zbc_set_log_level` for debug output. The zoned crate has no
logging facility.

- Add optional `tracing` or `log` support behind a feature flag
- Low priority — users can instrument at the application level
