//! Recording module — public API for ACC telemetry recording.
//!
//! Provides `TelemetrySource` abstraction, shared recording engine,
//! file naming / collision safety, and the `RecordingController` API.

pub mod source;
pub mod engine;
pub mod file_naming;
pub mod request;
pub mod status;
pub mod outcome;
pub mod dashboard;
pub mod controller;

// Re-export key types for convenience
pub use request::RecordingRequest;
pub use status::{
    status_channel, RecordingErrorKind, RecordingStatus, StatusReceiver, StatusSender, StopReason,
};
pub use outcome::{aggregate_laps, append_lap_index, extract_lap_telemetry, parse_acctlm_file, session_type_label, AggregatedLap, LapSummary, RecordingOutcome};
pub use dashboard::{DashboardItemInfo, DashboardItemKind, DashboardItemSubscription};
pub use file_naming::{
    build_output_path, check_no_collision, default_recording_name, ensure_output_dir,
};
pub use engine::{run_recording_loop, LapCompletedCallback, LapCompletedEvent, RecordingEngineConfig, RecordingLoopResult};
pub use source::{AccTelemetrySource, ScriptedStep, ScriptedTelemetrySource, TelemetrySource};
