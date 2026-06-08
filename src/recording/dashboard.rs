//! Dashboard subscription types for the recording API.
//!
//! These types are used by `RecordingRequest` to specify which dashboard
//! items to track during recording. The recording adapter translates them
//! into `DashboardService::subscribe()` calls.
//!
//! # Naming convention
//!
//! Item names use the unified `{type}:{path}` format:
//! - `raw:controls.speed_kmh` — raw telemetry field
//! - `raw:motion.velocity[0]` — array element
//! - `calc:delta_time_to_life_best_lap` — calculated item (needs ReferenceSource)
//! - `system:cpu_temp` — system info (future)
//!
//! See [`crate::item_key::ItemKey`] for the canonical parser.

use crate::compute::context::ReferenceSource;
use std::time::Duration;

/// Kind of a dashboard item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardItemKind {
    /// Raw telemetry value directly from a `TelemetryFrame` field.
    ///
    /// Item name format: `raw:{substruct}.{field}` (e.g. `raw:controls.speed_kmh`)
    RawItem,
    /// Calculated item registered in `ComputeRegistry`.
    ///
    /// Item name format: `calc:{name}` (e.g. `calc:speed_mps`)
    CalculatedItem,
    /// System-level info (CPU temp, system time, etc.)
    ///
    /// Item name format: `system:{name}` (e.g. `system:cpu_temp`)
    SystemItem,
}

/// A subscription request for a single dashboard item.
#[derive(Debug, Clone)]
pub struct DashboardItemSubscription {
    /// Item name in `{type}:{path}` format
    /// (e.g. `"raw:controls.speed_kmh"`, `"calc:delta_time_to_life_best_lap"`).
    pub item_name: String,
    /// Whether this is a raw, calculated, or system item.
    pub item_kind: DashboardItemKind,
    /// How often to push updates back to m1.
    pub interval: Duration,
    /// Reference lap source for calculated items that need one (e.g. DeltaTimeToLifeBestLap).
    /// `None` for raw items and calc items that don't need a reference.
    pub reference_source: Option<ReferenceSource>,
}

impl DashboardItemSubscription {
    pub fn new(
        item_name: impl Into<String>,
        item_kind: DashboardItemKind,
        interval: Duration,
    ) -> Self {
        Self {
            item_name: item_name.into(),
            item_kind,
            interval,
            reference_source: None,
        }
    }

    /// Create a subscription with a reference lap source.
    pub fn with_reference(
        item_name: impl Into<String>,
        item_kind: DashboardItemKind,
        interval: Duration,
        reference_source: ReferenceSource,
    ) -> Self {
        Self {
            item_name: item_name.into(),
            item_kind,
            interval,
            reference_source: Some(reference_source),
        }
    }
}

/// Metadata about an available dashboard item (returned by `list_available_items()`).
#[derive(Debug, Clone)]
pub struct DashboardItemInfo {
    /// Item name in `{type}:{path}` format.
    pub name: String,
    /// Raw, calculated, or system.
    pub kind: DashboardItemKind,
    /// Human-readable description.
    pub description: String,
    /// Unit, if applicable (e.g. `"km/h"`, `"m/s"`, `"rpm"`).
    pub unit: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_construction() {
        let sub = DashboardItemSubscription::new(
            "raw:controls.speed_kmh",
            DashboardItemKind::RawItem,
            Duration::from_millis(50),
        );
        assert_eq!(sub.item_name, "raw:controls.speed_kmh");
        assert_eq!(sub.item_kind, DashboardItemKind::RawItem);
        assert_eq!(sub.interval, Duration::from_millis(50));
    }

    #[test]
    fn test_item_info() {
        let info = DashboardItemInfo {
            name: "calc:speed_mps".into(),
            kind: DashboardItemKind::CalculatedItem,
            description: "Speed in meters per second".into(),
            unit: Some("m/s".into()),
        };
        assert_eq!(info.name, "calc:speed_mps");
        assert_eq!(info.unit, Some("m/s".into()));
    }
}
