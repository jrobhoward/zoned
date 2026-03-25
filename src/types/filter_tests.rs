#![allow(clippy::unwrap_used)]
#![allow(non_snake_case)]

use super::*;
use crate::types::{Sector, Zone, ZoneCondition, ZoneType};

fn make_zone(zone_type: ZoneType, condition: ZoneCondition) -> Zone {
    Zone {
        start: Sector::ZERO,
        len: Sector(1024),
        capacity: Sector(1024),
        write_pointer: Sector::ZERO,
        zone_type,
        condition,
        non_seq: false,
        reset_recommended: false,
    }
}

#[test]
fn zone_filter____empty____matches_everything() {
    let filter = ZoneFilter::new();
    assert!(filter.matches(&make_zone(
        ZoneType::Conventional,
        ZoneCondition::NotWritePointer
    )));
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Empty
    )));
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Full
    )));
}

#[test]
fn zone_filter____single_type____filters_correctly() {
    let filter = ZoneFilter::new().zone_type(ZoneType::SequentialWriteRequired);
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Empty
    )));
    assert!(!filter.matches(&make_zone(
        ZoneType::Conventional,
        ZoneCondition::NotWritePointer
    )));
}

#[test]
fn zone_filter____single_condition____filters_correctly() {
    let filter = ZoneFilter::new().condition(ZoneCondition::Empty);
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Empty
    )));
    assert!(!filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Full
    )));
}

#[test]
fn zone_filter____type_and_condition____both_must_match() {
    let filter = ZoneFilter::new()
        .zone_type(ZoneType::SequentialWriteRequired)
        .condition(ZoneCondition::Empty);
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Empty
    )));
    assert!(!filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Full
    )));
    assert!(!filter.matches(&make_zone(ZoneType::Conventional, ZoneCondition::Empty)));
}

#[test]
fn zone_filter____multiple_types____or_semantics() {
    let filter = ZoneFilter::new()
        .zone_type(ZoneType::Conventional)
        .zone_type(ZoneType::SequentialWriteRequired);
    assert!(filter.matches(&make_zone(
        ZoneType::Conventional,
        ZoneCondition::NotWritePointer
    )));
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Empty
    )));
    assert!(!filter.matches(&make_zone(
        ZoneType::SequentialWritePreferred,
        ZoneCondition::Empty
    )));
}

#[test]
fn zone_filter____multiple_conditions____or_semantics() {
    let filter = ZoneFilter::new()
        .condition(ZoneCondition::Empty)
        .condition(ZoneCondition::Closed);
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Empty
    )));
    assert!(filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Closed
    )));
    assert!(!filter.matches(&make_zone(
        ZoneType::SequentialWriteRequired,
        ZoneCondition::Full
    )));
}
