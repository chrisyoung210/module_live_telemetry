pub mod compute;
pub mod dashboard;
pub mod distributor;
pub(crate) mod encode_v2;
pub mod error;
pub mod format;
pub mod format_v2;
pub mod item_key;
pub(crate) mod mmap_win;
pub mod raw_catalog;
pub mod reader;
pub mod reader_v2;
pub mod recording;
pub mod shmem;
pub mod types;
pub mod writer;
pub mod writer_v2;

pub use error::{TelemetryError, TelemetryResult};
pub use reader::BinaryTelemetryReader;
pub use recording::{extract_lap_telemetry, extract_laps_telemetry};
pub use shmem::{parse_raw_frame, SPageFileStatic, RAW_GRAPHICS_SIZE, RAW_PHYSICS_SIZE, RAW_STATIC_SIZE};
pub use types::{
    AccSessionKind, CarStateSample, ControlSample, EnvironmentSample, LapIndexEntry, MotionSample,
    OtherCarsSample, PowertrainSample, RecordingSummary, SessionMetadata, SessionSample,
    TimingSample, TyreSample, CLUSTER_CONTROLS,
};
pub use writer::{LiveTelemetryConfig, TelemetryFrame};
