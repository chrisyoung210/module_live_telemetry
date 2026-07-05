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

use crate::compute::{
    context::ReferenceSource,
    items::{builtin_requires_reference, RealtimeComputeItem},
    ComputeRegistry,
};
use crate::item_key::{ItemKey, ItemType};
use crate::TelemetryFrame;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
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

/// Startup registration for a custom realtime calculated dashboard item.
#[derive(Clone)]
pub struct DashboardRealtimeItemRegistration {
    pub name: String,
    factory: Arc<dyn Fn() -> Box<dyn RealtimeComputeItem> + Send + Sync>,
}

impl std::fmt::Debug for DashboardRealtimeItemRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashboardRealtimeItemRegistration")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl DashboardRealtimeItemRegistration {
    pub fn new<F>(name: impl Into<String>, factory: F) -> Self
    where
        F: Fn() -> Box<dyn RealtimeComputeItem> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            factory: Arc::new(factory),
        }
    }

    pub(crate) fn create_item(&self) -> Box<dyn RealtimeComputeItem> {
        (self.factory)()
    }
}

/// Monotonically increasing identity of an applied dashboard subscription set.
pub type DashboardSubscriptionGeneration = u64;

/// Values produced by dashboard subscriptions for one telemetry frame.
#[derive(Debug, Clone)]
pub struct DashboardValuesFrame {
    /// Monotonically increasing subscription-set generation.
    /// Sparse frames from different generations must never be merged.
    pub subscription_generation: DashboardSubscriptionGeneration,
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: HashMap<String, f64>,
}

/// Stable numeric identity assigned to a dashboard field for compact transport.
pub type DashboardFieldId = u32;

/// Definition table shared before compact patches are decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardFieldDefinition {
    pub id: DashboardFieldId,
    pub name: String,
    pub kind: DashboardItemKind,
}

/// A sparse dashboard update using numeric field ids instead of repeated strings.
#[derive(Debug, Clone, PartialEq)]
pub struct DashboardCompactPatch {
    pub subscription_generation: DashboardSubscriptionGeneration,
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: Vec<(DashboardFieldId, f64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardCompactPatchError {
    InvalidHeader,
    Truncated,
    TrailingBytes,
    ValueCountOverflow,
}

impl std::fmt::Display for DashboardCompactPatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader => write!(f, "invalid dashboard compact patch header"),
            Self::Truncated => write!(f, "truncated dashboard compact patch"),
            Self::TrailingBytes => write!(f, "dashboard compact patch has trailing bytes"),
            Self::ValueCountOverflow => write!(f, "dashboard compact patch has too many values"),
        }
    }
}

impl std::error::Error for DashboardCompactPatchError {}

impl DashboardCompactPatch {
    const MAGIC: [u8; 4] = *b"DCP2";

    /// Encode to a stable little-endian binary payload.
    pub fn to_bytes(&self) -> Result<Vec<u8>, DashboardCompactPatchError> {
        let count = u32::try_from(self.values.len())
            .map_err(|_| DashboardCompactPatchError::ValueCountOverflow)?;
        let mut output = Vec::with_capacity(32 + self.values.len() * 12);
        output.extend_from_slice(&Self::MAGIC);
        output.extend_from_slice(&self.subscription_generation.to_le_bytes());
        output.extend_from_slice(&self.sample_tick.to_le_bytes());
        output.extend_from_slice(&self.timestamp_ns.to_le_bytes());
        output.extend_from_slice(&count.to_le_bytes());
        for (id, value) in &self.values {
            output.extend_from_slice(&id.to_le_bytes());
            output.extend_from_slice(&value.to_le_bytes());
        }
        Ok(output)
    }

    /// Decode a payload produced by [`Self::to_bytes`].
    pub fn from_bytes(input: &[u8]) -> Result<Self, DashboardCompactPatchError> {
        if input.len() < 32 {
            return Err(DashboardCompactPatchError::Truncated);
        }
        if input[..4] != Self::MAGIC {
            return Err(DashboardCompactPatchError::InvalidHeader);
        }
        let subscription_generation = u64::from_le_bytes(input[4..12].try_into().unwrap());
        let sample_tick = u64::from_le_bytes(input[12..20].try_into().unwrap());
        let timestamp_ns = u64::from_le_bytes(input[20..28].try_into().unwrap());
        let count = u32::from_le_bytes(input[28..32].try_into().unwrap()) as usize;
        let expected_len = 32usize
            .checked_add(
                count
                    .checked_mul(12)
                    .ok_or(DashboardCompactPatchError::ValueCountOverflow)?,
            )
            .ok_or(DashboardCompactPatchError::ValueCountOverflow)?;
        if input.len() < expected_len {
            return Err(DashboardCompactPatchError::Truncated);
        }
        if input.len() != expected_len {
            return Err(DashboardCompactPatchError::TrailingBytes);
        }
        let mut values = Vec::with_capacity(count);
        for chunk in input[32..].chunks_exact(12) {
            let id = u32::from_le_bytes(chunk[..4].try_into().unwrap());
            let value = f64::from_le_bytes(chunk[4..12].try_into().unwrap());
            values.push((id, value));
        }
        Ok(Self {
            subscription_generation,
            sample_tick,
            timestamp_ns,
            values,
        })
    }
}

/// Deterministic field-name registry used by compact dashboard transports.
///
/// IDs are assigned monotonically and are never reused for the lifetime of a
/// registry. Consumers must receive `definitions()` before decoding patches.
#[derive(Debug, Clone, Default)]
pub struct DashboardFieldRegistry {
    by_name: HashMap<String, DashboardFieldId>,
    definitions: BTreeMap<DashboardFieldId, DashboardFieldDefinition>,
    next_id: DashboardFieldId,
}

impl DashboardFieldRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        kind: DashboardItemKind,
    ) -> DashboardFieldId {
        let name = name.into();
        if let Some(id) = self.by_name.get(&name) {
            return *id;
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("dashboard field id exhausted");
        self.by_name.insert(name.clone(), id);
        self.definitions
            .insert(id, DashboardFieldDefinition { id, name, kind });
        id
    }

    pub fn id_for(&self, name: &str) -> Option<DashboardFieldId> {
        self.by_name.get(name).copied()
    }

    pub fn definitions(&self) -> Vec<DashboardFieldDefinition> {
        self.definitions.values().cloned().collect()
    }

    pub fn encode_frame(&mut self, frame: &DashboardValuesFrame) -> DashboardCompactPatch {
        let mut values: Vec<_> = frame
            .values
            .iter()
            .map(|(name, value)| {
                let kind = ItemKey::parse(name)
                    .map(|key| kind_for_item_type(key.item_type))
                    .unwrap_or(DashboardItemKind::SystemItem);
                (self.register(name.clone(), kind), *value)
            })
            .collect();
        values.sort_by_key(|(id, _)| *id);
        DashboardCompactPatch {
            subscription_generation: frame.subscription_generation,
            sample_tick: frame.sample_tick,
            timestamp_ns: frame.timestamp_ns,
            values,
        }
    }

    pub fn decode_patch(&self, patch: &DashboardCompactPatch) -> Option<DashboardValuesFrame> {
        let mut values = HashMap::with_capacity(patch.values.len());
        for (id, value) in &patch.values {
            let definition = self.definitions.get(id)?;
            values.insert(definition.name.clone(), *value);
        }
        Some(DashboardValuesFrame {
            subscription_generation: patch.subscription_generation,
            sample_tick: patch.sample_tick,
            timestamp_ns: patch.timestamp_ns,
            values,
        })
    }
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

/// Validation result for a dashboard subscription set.
#[derive(Debug, Clone)]
pub struct DashboardSubscriptionValidation {
    pub valid: bool,
    pub errors: Vec<DashboardSubscriptionError>,
}

/// Validation error for one dashboard subscription item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardSubscriptionError {
    pub item_name: String,
    pub message: String,
}

/// Validate dashboard subscriptions against the default dashboard registry.
pub fn validate_dashboard_subscriptions(
    items: &[DashboardItemSubscription],
) -> DashboardSubscriptionValidation {
    let registry = ComputeRegistry::new();
    let builtin_names = builtin_calculated_item_names();
    validate_dashboard_subscriptions_with_item_check(items, |name| {
        registry.is_registered(name) || builtin_names.contains(name)
    })
}

pub(crate) fn validate_dashboard_subscriptions_with_registry(
    items: &[DashboardItemSubscription],
    registry: &ComputeRegistry,
) -> DashboardSubscriptionValidation {
    validate_dashboard_subscriptions_with_item_check(items, |name| registry.is_registered(name))
}

pub(crate) fn validate_dashboard_subscriptions_with_calculated_items(
    items: &[DashboardItemSubscription],
    calculated_items: &HashSet<String>,
) -> DashboardSubscriptionValidation {
    validate_dashboard_subscriptions_with_item_check(items, |name| calculated_items.contains(name))
}

fn validate_dashboard_subscriptions_with_item_check(
    items: &[DashboardItemSubscription],
    is_calculated_item_registered: impl Fn(&str) -> bool,
) -> DashboardSubscriptionValidation {
    let mut errors = Vec::new();

    for item in items {
        if item.interval.is_zero() {
            errors.push(DashboardSubscriptionError {
                item_name: item.item_name.clone(),
                message: "interval must be greater than zero".to_string(),
            });
        }

        let Some(key) = ItemKey::parse(&item.item_name) else {
            errors.push(DashboardSubscriptionError {
                item_name: item.item_name.clone(),
                message: "item name must use raw:, calc:, or system: prefix".to_string(),
            });
            continue;
        };

        let expected_kind = kind_for_item_type(key.item_type);
        if item.item_kind != expected_kind {
            errors.push(DashboardSubscriptionError {
                item_name: item.item_name.clone(),
                message: format!(
                    "item kind {:?} does not match item name prefix",
                    item.item_kind
                ),
            });
        }

        match key.item_type {
            ItemType::Raw => {
                if !TelemetryFrame::is_raw_field(&key.name) {
                    errors.push(DashboardSubscriptionError {
                        item_name: item.item_name.clone(),
                        message: "raw telemetry field is not available".to_string(),
                    });
                }
            }
            ItemType::Calculated => {
                if !is_calculated_item_registered(&key.name) {
                    errors.push(DashboardSubscriptionError {
                        item_name: item.item_name.clone(),
                        message: "calculated item is not registered".to_string(),
                    });
                }
                if builtin_requires_reference(&key.name) && item.reference_source.is_none() {
                    errors.push(DashboardSubscriptionError {
                        item_name: item.item_name.clone(),
                        message: "this calculated item requires a reference_source \
                                  (file path + lap number)".to_string(),
                    });
                }
            }
            ItemType::System => {
                errors.push(DashboardSubscriptionError {
                    item_name: item.item_name.clone(),
                    message: "system dashboard items are not supported".to_string(),
                });
            }
        }
    }

    DashboardSubscriptionValidation {
        valid: errors.is_empty(),
        errors,
    }
}

pub(crate) fn dashboard_item_info_for_subscription(
    item: &DashboardItemSubscription,
) -> DashboardItemInfo {
    let Some(key) = ItemKey::parse(&item.item_name) else {
        return DashboardItemInfo {
            name: item.item_name.clone(),
            kind: item.item_kind.clone(),
            description: "Invalid dashboard item".to_string(),
            unit: None,
        };
    };

    match key.item_type {
        ItemType::Raw => crate::raw_catalog::all_raw_items()
            .into_iter()
            .find(|entry| entry.key == key)
            .map(|entry| DashboardItemInfo {
                name: entry.key.to_string(),
                kind: DashboardItemKind::RawItem,
                description: entry.description,
                unit: entry.unit.map(|unit| unit.to_string()),
            })
            .unwrap_or_else(|| DashboardItemInfo {
                name: item.item_name.clone(),
                kind: DashboardItemKind::RawItem,
                description: format!("Raw telemetry item: {}", key.name),
                unit: None,
            }),
        ItemType::Calculated => DashboardItemInfo {
            name: item.item_name.clone(),
            kind: DashboardItemKind::CalculatedItem,
            description: builtin_calculated_item_info(&key.name)
                .map(|entry| entry.description.to_string())
                .unwrap_or_else(|| format!("Calculated item: {}", key.name)),
            unit: builtin_calculated_item_info(&key.name)
                .and_then(|entry| entry.unit.map(|unit| unit.to_string())),
        },
        ItemType::System => DashboardItemInfo {
            name: item.item_name.clone(),
            kind: DashboardItemKind::SystemItem,
            description: format!("System item: {}", key.name),
            unit: None,
        },
    }
}

pub(crate) fn builtin_calculated_item_names() -> HashSet<String> {
    crate::compute::items::all_builtin_calculated_items()
        .into_iter()
        .map(|entry| entry.key.name)
        .collect()
}

fn builtin_calculated_item_info(name: &str) -> Option<crate::compute::items::BuiltinCalcItemEntry> {
    crate::compute::items::all_builtin_calculated_items()
        .into_iter()
        .find(|entry| entry.key.name == name)
}

fn kind_for_item_type(item_type: ItemType) -> DashboardItemKind {
    match item_type {
        ItemType::Raw => DashboardItemKind::RawItem,
        ItemType::Calculated => DashboardItemKind::CalculatedItem,
        ItemType::System => DashboardItemKind::SystemItem,
    }
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

    #[test]
    fn compact_patch_round_trips_and_reuses_ids() {
        let mut registry = DashboardFieldRegistry::new();
        let frame = DashboardValuesFrame {
            subscription_generation: 3,
            sample_tick: 42,
            timestamp_ns: 1234,
            values: HashMap::from([
                ("raw:controls.speed_kmh".to_string(), 211.5),
                ("raw:controls.rpms".to_string(), 7200.0),
            ]),
        };

        let first = registry.encode_frame(&frame);
        let second = registry.encode_frame(&frame);
        assert_eq!(first.values, second.values);
        assert_eq!(registry.definitions().len(), 2);
        assert_eq!(registry.decode_patch(&first).unwrap().values, frame.values);
        let bytes = first.to_bytes().unwrap();
        assert_eq!(DashboardCompactPatch::from_bytes(&bytes).unwrap(), first);
        assert_eq!(
            DashboardCompactPatch::from_bytes(&bytes[..bytes.len() - 1]),
            Err(DashboardCompactPatchError::Truncated)
        );
    }

    // ---- P3-1: reference_source validation tests ----

    fn make_subscription(item_name: &str, reference_source: Option<ReferenceSource>) -> DashboardItemSubscription {
        DashboardItemSubscription {
            item_name: item_name.to_string(),
            item_kind: DashboardItemKind::CalculatedItem,
            interval: Duration::from_millis(50),
            reference_source,
        }
    }

    #[test]
    fn test_validation_session_best_without_reference_source_is_valid() {
        let items = vec![
            make_subscription("calc:delta_time_to_session_best_lap", None),
            make_subscription("calc:delta_time_to_session_best_lap_interpolated", None),
            make_subscription("calc:predict_lap_time_by_session_best_lap", None),
        ];
        let result = validate_dashboard_subscriptions(&items);
        assert!(result.valid, "session-best items without explicit reference_source should be valid: {:#?}", result.errors);
    }

    #[test]
    fn test_validation_life_best_without_reference_source_is_invalid() {
        let items = vec![make_subscription("calc:delta_time_to_life_best_lap", None)];
        let result = validate_dashboard_subscriptions(&items);
        assert!(!result.valid);
        assert!(
            result.errors[0].message.contains("reference_source"),
            "expected error about missing reference_source, got: {}",
            result.errors[0].message
        );
    }

    #[test]
    fn test_validation_life_best_with_reference_source_is_valid() {
        let items = vec![make_subscription(
            "calc:delta_time_to_life_best_lap",
            Some(ReferenceSource {
                file_path: std::path::PathBuf::from("test.acctlm2"),
                lap_number: 1,
            }),
        )];
        let result = validate_dashboard_subscriptions(&items);
        assert!(result.valid);
    }
}
