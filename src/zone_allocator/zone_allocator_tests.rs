#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

// ZoneAllocator unit tests are limited since construction requires a real device.
// The bulk of testing is in integration tests (tests/nullblk_integration.rs).
// Here we test the AllocatorInner tracking logic directly.

use crate::types::ZoneIndex;

use super::*;

#[test]
fn allocator_inner____try_allocate____succeeds_for_new_index() {
    let inner = AllocatorInner::new();
    assert!(inner.try_allocate(ZoneIndex(5)).is_ok());
}

#[test]
fn allocator_inner____try_allocate____fails_for_duplicate() {
    let inner = AllocatorInner::new();
    inner.try_allocate(ZoneIndex(5)).unwrap();

    let result = inner.try_allocate(ZoneIndex(5));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            ZonedError::ZoneAlreadyAllocated {
                zone_index: ZoneIndex(5)
            }
        ),
        "Expected ZoneAlreadyAllocated, got: {err:?}"
    );
}

#[test]
fn allocator_inner____release____allows_reallocation() {
    let inner = AllocatorInner::new();
    inner.try_allocate(ZoneIndex(5)).unwrap();
    inner.release(ZoneIndex(5));
    assert!(inner.try_allocate(ZoneIndex(5)).is_ok());
}

#[test]
fn allocator_inner____multiple_indices____are_independent() {
    let inner = AllocatorInner::new();
    inner.try_allocate(ZoneIndex(1)).unwrap();
    inner.try_allocate(ZoneIndex(2)).unwrap();
    inner.try_allocate(ZoneIndex(3)).unwrap();

    // Each index is independent
    assert!(inner.try_allocate(ZoneIndex(1)).is_err());
    assert!(inner.try_allocate(ZoneIndex(4)).is_ok());

    inner.release(ZoneIndex(2));
    assert!(inner.try_allocate(ZoneIndex(2)).is_ok());
}
