//! Recording status — state notifications sent from m2 to m1.

use std::fmt;
use std::time::Duration;

/// Status update sent from the recording thread to m1.
#[derive(Debug, Clone)]
pub enum RecordingStatus {
    /// Recording thread has started (background thread spawned).
    /// `thread_id` is the OS thread ID for monitoring.
    Started { thread_id: u64 },

    /// Recording service is waiting for ACC shared memory to appear.
    WaitingForSharedMemory { message: String },

    /// ACC shared memory connected — ready to record once ACC goes Live.
    Connected,

    /// Recording has begun (writer created, frames flowing).
    RecordingStarted,

    /// Periodic progress update during recording.
    Running {
        /// Total frames recorded.
        sample_count: u64,
        /// Approximate bytes written to disk.
        bytes_written: u64,
        /// Wall-clock time since recording started.
        elapsed: Duration,
        /// Effective recording rate in Hz.
        fps: f64,
    },

    /// ACC is paused; recording suspended but writer kept open.
    Paused,

    /// Replay of recorded telemetry has started.
    ReplayStarted,

    /// An error occurred. Recording may continue or stop depending on severity.
    Error {
        message: String,
        kind: RecordingErrorKind,
    },

    /// Recording is stopping (cleanup in progress).
    Stopping { reason: StopReason },
}

/// Categories of recording errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingErrorKind {
    DiskFull,
    ShmemDisconnected,
    Unknown,
}

impl fmt::Display for RecordingErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiskFull => write!(f, "disk full"),
            Self::ShmemDisconnected => write!(f, "shared memory disconnected"),
            Self::Unknown => write!(f, "unknown error"),
        }
    }
}

/// Reason the recording is stopping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Manually stopped via `stop_recording()`.
    Manual,
    /// ACC session ended (status changed from Live to Off).
    SessionEnd,
    /// ACC shared memory was lost.
    ShmemLost,
    /// All available frames have been replayed.
    FramesExhausted,
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => write!(f, "manual stop"),
            Self::SessionEnd => write!(f, "session ended"),
            Self::ShmemLost => write!(f, "shared memory lost"),
            Self::FramesExhausted => write!(f, "frames exhausted"),
        }
    }
}

// Channel type aliases for convenience
use crossbeam_channel::{Receiver, Sender};

/// Sending end of the status channel (m2 → m1).
pub type StatusSender = Sender<RecordingStatus>;
/// Receiving end of the status channel (m1 side).
pub type StatusReceiver = Receiver<RecordingStatus>;

/// Create a bounded status channel pair.
pub fn status_channel(capacity: usize) -> (StatusSender, StatusReceiver) {
    crossbeam_channel::bounded(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_variants_constructible() {
        let started = RecordingStatus::Started { thread_id: 42 };
        let _waiting = RecordingStatus::WaitingForSharedMemory {
            message: "ACC not available".into(),
        };
        let _connected = RecordingStatus::Connected;
        let _rec_started = RecordingStatus::RecordingStarted;
        let running = RecordingStatus::Running {
            sample_count: 1000,
            bytes_written: 50000,
            elapsed: Duration::from_secs(10),
            fps: 100.0,
        };
        let _paused = RecordingStatus::Paused;
        let _error = RecordingStatus::Error {
            message: "test error".into(),
            kind: RecordingErrorKind::ShmemDisconnected,
        };
        let _stopping = RecordingStatus::Stopping {
            reason: StopReason::SessionEnd,
        };

        // Verify Debug output contains key fields
        let s = format!("{:?}", started);
        assert!(s.contains("42"));

        let s2 = format!("{:?}", running);
        assert!(s2.contains("1000"));
    }

    #[test]
    fn test_status_channel_pair() {
        let (tx, rx) = status_channel(8);
        tx.send(RecordingStatus::Connected).unwrap();
        let msg = rx.recv().unwrap();
        assert!(matches!(msg, RecordingStatus::Connected));
    }

    #[test]
    fn test_stop_reason_display() {
        assert_eq!(format!("{}", StopReason::Manual), "manual stop");
        assert_eq!(format!("{}", StopReason::SessionEnd), "session ended");
        assert_eq!(format!("{}", StopReason::ShmemLost), "shared memory lost");
    }

    #[test]
    fn test_error_kind_display() {
        assert_eq!(format!("{}", RecordingErrorKind::DiskFull), "disk full");
        assert_eq!(
            format!("{}", RecordingErrorKind::ShmemDisconnected),
            "shared memory disconnected"
        );
    }

    #[test]
    fn test_replay_started_variant() {
        let replay_started = RecordingStatus::ReplayStarted;
        let debug = format!("{:?}", replay_started);
        assert_eq!(debug, "ReplayStarted");
    }

    #[test]
    fn test_frames_exhausted_display() {
        assert_eq!(
            format!("{}", StopReason::FramesExhausted),
            "frames exhausted"
        );
    }

    #[test]
    fn test_existing_stop_reasons_unaffected() {
        // Verify existing Display variants are unchanged
        assert_eq!(format!("{}", StopReason::Manual), "manual stop");
        assert_eq!(format!("{}", StopReason::SessionEnd), "session ended");
        assert_eq!(format!("{}", StopReason::ShmemLost), "shared memory lost");
    }
}
