use super::{Zone, ZoneCondition, ZoneType};

/// A composable filter for selecting zones by type and/or condition.
///
/// # Example
///
/// ```
/// use zoned::{ZoneFilter, ZoneType, ZoneCondition};
///
/// let filter = ZoneFilter::new()
///     .zone_type(ZoneType::SequentialWriteRequired)
///     .condition(ZoneCondition::Empty);
/// ```
#[derive(Debug, Clone, Default)]
pub struct ZoneFilter {
    zone_types: Vec<ZoneType>,
    conditions: Vec<ZoneCondition>,
}

impl ZoneFilter {
    /// Create an empty filter that matches all zones.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a zone type to match. Multiple calls are OR'd — a zone matches
    /// if its type is any of the specified types.
    pub fn zone_type(mut self, zt: ZoneType) -> Self {
        self.zone_types.push(zt);
        self
    }

    /// Add a zone condition to match. Multiple calls are OR'd — a zone matches
    /// if its condition is any of the specified conditions.
    pub fn condition(mut self, cond: ZoneCondition) -> Self {
        self.conditions.push(cond);
        self
    }

    /// Test whether a zone matches this filter.
    ///
    /// A zone matches if:
    /// - Its type matches any of the specified types (or no types specified), AND
    /// - Its condition matches any of the specified conditions (or no conditions specified).
    pub fn matches(&self, zone: &Zone) -> bool {
        let type_ok = self.zone_types.is_empty() || self.zone_types.contains(&zone.zone_type);
        let cond_ok = self.conditions.is_empty() || self.conditions.contains(&zone.condition);
        type_ok && cond_ok
    }
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod filter_tests;
