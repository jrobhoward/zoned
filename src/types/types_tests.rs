#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

#[test]
fn zone____default_fields____are_accessible() {
    let zone = Zone {
        start: Sector(0),
        len: Sector(524288),
        capacity: Sector(524288),
        write_pointer: Some(Sector(1024)),
        zone_type: ZoneType::SequentialWriteRequired,
        condition: ZoneCondition::ImplicitlyOpen,
        non_seq: false,
        reset_recommended: false,
    };

    assert_eq!(zone.start, Sector(0));
    assert_eq!(zone.len, Sector(524288));
    assert_eq!(zone.capacity, Sector(524288));
    assert_eq!(zone.write_pointer, Some(Sector(1024)));
    assert_eq!(zone.zone_type, ZoneType::SequentialWriteRequired);
    assert_eq!(zone.condition, ZoneCondition::ImplicitlyOpen);
    assert!(!zone.non_seq);
    assert!(!zone.reset_recommended);
}

#[test]
fn zone____clone____produces_equal_copy() {
    let zone = Zone {
        start: Sector(100),
        len: Sector(200),
        capacity: Sector(180),
        write_pointer: Some(Sector(150)),
        zone_type: ZoneType::Conventional,
        condition: ZoneCondition::NotWritePointer,
        non_seq: true,
        reset_recommended: true,
    };

    let cloned = zone.clone();
    assert_eq!(zone, cloned);
}

#[test]
fn zone_type____all_variants____are_distinct() {
    let variants = [
        ZoneType::Conventional,
        ZoneType::SequentialWriteRequired,
        ZoneType::SequentialWritePreferred,
    ];

    for (i, a) in variants.iter().enumerate() {
        for (j, b) in variants.iter().enumerate() {
            if i == j {
                assert_eq!(a, b);
            } else {
                assert_ne!(a, b);
            }
        }
    }
}

#[test]
fn zone_condition____all_variants____are_distinct() {
    let variants = [
        ZoneCondition::NotWritePointer,
        ZoneCondition::Empty,
        ZoneCondition::ImplicitlyOpen,
        ZoneCondition::ExplicitlyOpen,
        ZoneCondition::Closed,
        ZoneCondition::ReadOnly,
        ZoneCondition::Full,
        ZoneCondition::Offline,
    ];

    for (i, a) in variants.iter().enumerate() {
        for (j, b) in variants.iter().enumerate() {
            if i == j {
                assert_eq!(a, b);
            } else {
                assert_ne!(a, b);
            }
        }
    }
}

#[test]
fn device_info____fields____are_accessible() {
    let info = DeviceInfo {
        zone_size: Sector(524288),
        nr_zones: 55880,
    };

    assert_eq!(info.zone_size, Sector(524288));
    assert_eq!(info.nr_zones, 55880);
}

#[test]
fn device_model____all_variants____are_distinct() {
    assert_ne!(DeviceModel::None, DeviceModel::HostAware);
    assert_ne!(DeviceModel::None, DeviceModel::HostManaged);
    assert_ne!(DeviceModel::HostAware, DeviceModel::HostManaged);
}

#[test]
fn device_properties____fields____are_accessible() {
    let props = DeviceProperties {
        model: DeviceModel::HostManaged,
        identity: DeviceIdentity {
            vendor: Some("ATA".to_string()),
            model_name: Some("HGST HMH7210A0AL".to_string()),
        },
        geometry: DeviceGeometry {
            chunk_sectors: Sector(524288),
            nr_zones: 55880,
            capacity_sectors: Sector(19532873728),
        },
        limits: DeviceLimits {
            zone_append_max_bytes: 0,
            max_open_zones: Some(128),
            max_active_zones: None,
            max_hw_sectors_kb: 1024,
            max_sectors_kb: 512,
        },
        block_sizes: BlockSizes {
            logical_block_size: 512,
            physical_block_size: 4096,
        },
        scheduler: Some("mq-deadline".to_string()),
    };

    assert_eq!(props.model, DeviceModel::HostManaged);
    assert_eq!(props.geometry.chunk_sectors, Sector(524288));
    assert_eq!(props.geometry.nr_zones, 55880);
    assert_eq!(props.limits.zone_append_max_bytes, 0);
    assert_eq!(props.limits.max_open_zones, Some(128));
    assert_eq!(props.limits.max_active_zones, None);
    assert_eq!(props.block_sizes.logical_block_size, 512);
    assert_eq!(props.block_sizes.physical_block_size, 4096);
    assert_eq!(props.geometry.capacity_sectors, Sector(19532873728));
    assert_eq!(props.scheduler.as_deref(), Some("mq-deadline"));
    assert_eq!(props.identity.vendor.as_deref(), Some("ATA"));
    assert_eq!(
        props.identity.model_name.as_deref(),
        Some("HGST HMH7210A0AL")
    );
}

#[test]
fn sector_size____constant____is_512() {
    assert_eq!(SECTOR_SIZE, 512);
}

#[test]
fn zone_type____display____matches_expected_strings() {
    assert_eq!(ZoneType::Conventional.to_string(), "Conventional");
    assert_eq!(
        ZoneType::SequentialWriteRequired.to_string(),
        "Sequential Write Required"
    );
    assert_eq!(
        ZoneType::SequentialWritePreferred.to_string(),
        "Sequential Write Preferred"
    );
}

#[test]
fn zone_condition____display____matches_expected_strings() {
    assert_eq!(
        ZoneCondition::NotWritePointer.to_string(),
        "Not Write Pointer"
    );
    assert_eq!(ZoneCondition::Empty.to_string(), "Empty");
    assert_eq!(ZoneCondition::ImplicitlyOpen.to_string(), "Implicitly Open");
    assert_eq!(ZoneCondition::ExplicitlyOpen.to_string(), "Explicitly Open");
    assert_eq!(ZoneCondition::Closed.to_string(), "Closed");
    assert_eq!(ZoneCondition::ReadOnly.to_string(), "Read Only");
    assert_eq!(ZoneCondition::Full.to_string(), "Full");
    assert_eq!(ZoneCondition::Offline.to_string(), "Offline");
}

#[test]
fn device_model____display____matches_expected_strings() {
    assert_eq!(DeviceModel::None.to_string(), "none");
    assert_eq!(DeviceModel::HostAware.to_string(), "host-aware");
    assert_eq!(DeviceModel::HostManaged.to_string(), "host-managed");
}

// Sector newtype tests

#[test]
fn sector____add____works() {
    assert_eq!(Sector(10) + Sector(20), Sector(30));
}

#[test]
fn sector____sub____works() {
    assert_eq!(Sector(30) - Sector(10), Sector(20));
}

#[test]
fn sector____mul_u64____works() {
    assert_eq!(Sector(100) * 4, Sector(400));
}

#[test]
fn sector____div_u64____works() {
    assert_eq!(Sector(400) / 4, Sector(100));
}

#[test]
fn sector____add_assign____works() {
    let mut s = Sector(10);
    s += Sector(5);
    assert_eq!(s, Sector(15));
}

#[test]
fn sector____to_bytes____multiplies_by_512() {
    assert_eq!(Sector(1).to_bytes(), 512);
    assert_eq!(Sector(0).to_bytes(), 0);
    assert_eq!(Sector(1024).to_bytes(), 524288);
}

#[test]
fn sector____from_bytes____aligned____returns_some() {
    assert_eq!(Sector::from_bytes(0), Some(Sector(0)));
    assert_eq!(Sector::from_bytes(512), Some(Sector(1)));
    assert_eq!(Sector::from_bytes(524288), Some(Sector(1024)));
}

#[test]
fn sector____from_bytes____unaligned____returns_none() {
    assert_eq!(Sector::from_bytes(1), None);
    assert_eq!(Sector::from_bytes(511), None);
    assert_eq!(Sector::from_bytes(513), None);
}

#[test]
fn sector____raw____returns_inner() {
    assert_eq!(Sector(42).raw(), 42);
}

#[test]
fn sector____display____shows_number() {
    assert_eq!(format!("{}", Sector(12345)), "12345");
}

#[test]
fn sector____ordering____works() {
    assert!(Sector(10) < Sector(20));
    assert!(Sector(20) > Sector(10));
    assert!(Sector(10) <= Sector(10));
}

#[test]
fn sector____zero_constant____is_zero() {
    assert_eq!(Sector::ZERO, Sector(0));
}

// ZoneIndex newtype tests

#[test]
fn zone_index____raw____returns_inner() {
    assert_eq!(ZoneIndex(5).raw(), 5);
}

#[test]
fn zone_index____display____shows_number() {
    assert_eq!(format!("{}", ZoneIndex(42)), "42");
}

#[test]
fn zone_index____ordering____works() {
    assert!(ZoneIndex(0) < ZoneIndex(1));
    assert_eq!(ZoneIndex(5), ZoneIndex(5));
}
