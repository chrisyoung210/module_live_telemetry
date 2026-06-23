//! Recording module — public API for ACC telemetry recording.
//!
//! The primary entry point is [`RecordingController::start()`], which internally
//! manages ACC shared memory connection, session detection, and file output.
//! The controller returns immediately and runs recording in a background thread.
//!
//! # Usage
//!
//! ```no_run
//! use module_live_telemetry::recording::{
//!     RecordingController, RecordingRequest, outcome_channel,
//! };
//! use module_live_telemetry::recording::status::status_channel;
//! use std::time::Duration;
//!
//! let req = RecordingRequest {
//!     poll_hz: 60.0,
//!     output_dir: "./data".into(),
//!     status_interval: Duration::from_secs(1),
//!     dashboard_items: vec![],
//!     dashboard_realtime_items: vec![],
//! };
//! let (status_tx, _status_rx) = status_channel(8);
//! let (outcome_tx, outcome_rx) = outcome_channel();
//!
//! let ctrl = RecordingController::start(req, status_tx, outcome_tx, None, None)?;
//! // ... recording runs in background ...
//! let outcome = outcome_rx.recv()?;
//! # Ok::<(), module_live_telemetry::TelemetryError>(())
//! ```

pub mod controller;
pub mod dashboard;
pub mod engine;
pub mod file_naming;
pub mod outcome;
pub mod request;
pub mod source;
pub mod status;

// Re-export key types for convenience
pub use controller::{outcome_channel, RecordingController};
pub use dashboard::{
    validate_dashboard_subscriptions, DashboardCompactPatch, DashboardCompactPatchError,
    DashboardFieldDefinition, DashboardFieldId, DashboardFieldRegistry, DashboardItemInfo,
    DashboardItemKind, DashboardItemSubscription, DashboardRealtimeItemRegistration,
    DashboardSubscriptionError, DashboardSubscriptionGeneration, DashboardSubscriptionValidation,
    DashboardValuesFrame,
};
pub use engine::{LapCompletedCallback, LapCompletedEvent};
pub use file_naming::{
    build_output_path, build_unique_output_path, check_no_collision, default_recording_name,
    ensure_output_dir,
};
pub use outcome::{
    aggregate_laps, append_lap_index, extract_lap_telemetry, extract_laps_telemetry,
    parse_acctlm_file, session_type_label, AggregatedLap, LapSummary, RecordingOutcome,
};
pub use request::{RecordingRequest, ReplayRequest};
pub use source::ReplayTelemetrySource;
pub use status::{
    status_channel, RecordingErrorKind, RecordingStatus, StatusReceiver, StatusSender, StopReason,
};
