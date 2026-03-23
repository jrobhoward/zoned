#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

#[test]
fn zone____default_fields____are_accessible() {
    let zone = Zone {
        start: 0,
        len: 524288,
        capacity: 524288,
        write_pointer: 1024,
        zone_type: ZoneType::SequentialWriteRequired,
        condition: ZoneCondition::ImplicitlyOpen,
        non_seq: false,
        reset_recommended: false,
    };

    assert_eq!(zone.start, 0);
    assert_eq!(zone.len, 524288);
    assert_eq!(zone.capacity, 524288);
    assert_eq!(zone.write_pointer, 1024);
    assert_eq!(zone.zone_type, ZoneType::SequentialWriteRequired);
    assert_eq!(zone.condition, ZoneCondition::ImplicitlyOpen);
    assert!(!zone.non_seq);
    assert!(!zone.reset_recommended);
}

#[test]
fn zone____clone____produces_equal_copy() {
    let zone = Zone {
        start: 100,
        len: 200,
        capacity: 180,
        write_pointer: 150,
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
        zone_size: 524288,
        nr_zones: 55880,
    };

    assert_eq!(info.zone_size, 524288);
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
        chunk_sectors: 524288,
        nr_zones: 55880,
        zone_append_max_bytes: 0,
        max_open_zones: 128,
        max_active_zones: 0,
    };

    assert_eq!(props.model, DeviceModel::HostManaged);
    assert_eq!(props.chunk_sectors, 524288);
    assert_eq!(props.nr_zones, 55880);
    assert_eq!(props.zone_append_max_bytes, 0);
    assert_eq!(props.max_open_zones, 128);
    assert_eq!(props.max_active_zones, 0);
}

#[test]
fn sector_size____constant____is_512() {
    assert_eq!(SECTOR_SIZE, 512);
}
