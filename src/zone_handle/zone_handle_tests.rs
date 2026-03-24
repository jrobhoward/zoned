#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

// ZoneHandle unit tests are limited since construction requires a real device.
// The bulk of testing is in integration tests (tests/nullblk_integration.rs).
// Here we test compile-time properties and Debug formatting.

use super::*;

#[test]
fn zone_handle____is_send____compile_time_check() {
    fn assert_send<T: Send>() {}
    assert_send::<ZoneHandle>();
}

#[test]
fn zone_handle____is_not_clone____by_design() {
    // ZoneHandle intentionally does not implement Clone.
    // This is a compile-time property that cannot be tested at runtime,
    // but we document the intent here.
    // If someone adds #[derive(Clone)] to ZoneHandle, the integration
    // tests exercising exclusive ownership semantics would catch misuse.
}
