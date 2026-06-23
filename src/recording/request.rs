//! Recording request — public API input type and validation.

use crate::error::{TelemetryError, TelemetryResult};
use crate::recording::dashboard::builtin_calculated_item_names;
use std::path::PathBuf;
use std::time::Duration;

/// Parameters for a recording session.
///
#[derive(Debug, Clone)]
pub struct RecordingRequest {
    /// Polling frequency in Hz. Range: [30.0, 120.0].
    pub poll_hz: f64,
    /// Output directory. Must exist and be a directory.
    /// The actual file name is auto-generated.
    pub output_dir: PathBuf,
    /// Interval between status callbacks to m1.
    pub status_interval: Duration,
    /// Dashboard items to subscribe to.
    /// Dashboard infrastructure is always initialized; items drive what data is computed.
    pub dashboard_items: Vec<super::DashboardItemSubscription>,
    /// Custom realtime calculated dashboard items registered at controller startup.
    pub dashboard_realtime_items: Vec<super::DashboardRealtimeItemRegistration>,
}

/// Parameters for replaying a previously recorded telemetry session.
///
#[derive(Debug, Clone)]
pub struct ReplayRequest {
    /// Path to the `.acctlm` recording file. Must exist.
    pub file_path: PathBuf,
    /// Replay speed multiplier. 1.0 = original speed, 2.0 = double speed, etc.
    /// Must be > 0.0 and finite.
    pub speed_multiplier: f64,
    /// Interval between status callbacks to m1.
    pub status_interval: Duration,
    /// Dashboard items to subscribe to.
    pub dashboard_items: Vec<super::DashboardItemSubscription>,
    /// Custom realtime calculated dashboard items registered at controller startup.
    pub dashboard_realtime_items: Vec<super::DashboardRealtimeItemRegistration>,
}

impl RecordingRequest {
    /// Validate all request parameters.
    ///
    /// Returns `Ok(())` if the request is valid, or `Err(InvalidArgument)`
    /// describing the first violation.
    pub fn validate(&self) -> TelemetryResult<()> {
        // poll_hz must be finite and in range
        if !self.poll_hz.is_finite() {
            return Err(TelemetryError::InvalidArgument(format!(
                "poll_hz must be finite, got {}",
                self.poll_hz
            )));
        }
        if self.poll_hz < 30.0 || self.poll_hz > 120.0 {
            return Err(TelemetryError::InvalidArgument(format!(
                "poll_hz must be between 30.0 and 120.0, got {}",
                self.poll_hz
            )));
        }

        // output_dir must exist and be a directory
        if !self.output_dir.is_dir() {
            return Err(TelemetryError::InvalidArgument(format!(
                "output directory does not exist or is not a directory: {}",
                self.output_dir.display()
            )));
        }

        // status_interval must be non-zero
        if self.status_interval.is_zero() {
            return Err(TelemetryError::InvalidArgument(
                "status_interval must be greater than zero".to_string(),
            ));
        }

        let mut registered = builtin_calculated_item_names();
        for item in &self.dashboard_realtime_items {
            if item.name.is_empty() {
                return Err(TelemetryError::InvalidArgument(
                    "dashboard realtime item name must not be empty".to_string(),
                ));
            }
            if !registered.insert(item.name.clone()) {
                return Err(TelemetryError::InvalidArgument(format!(
                    "dashboard realtime item '{}' is already registered",
                    item.name
                )));
            }
            let created = item.create_item();
            if created.name() != item.name {
                return Err(TelemetryError::InvalidArgument(format!(
                    "dashboard realtime item registration name '{}' does not match item name '{}'",
                    item.name,
                    created.name()
                )));
            }
        }

        Ok(())
    }
}

impl ReplayRequest {
    /// Validate all replay request parameters.
    ///
    /// Returns `Ok(())` if the request is valid, or `Err(InvalidArgument)`
    /// describing the first violation.
    pub fn validate(&self) -> TelemetryResult<()> {
        // file_path must exist and be a file
        if !self.file_path.is_file() {
            return Err(TelemetryError::InvalidArgument(format!(
                "file does not exist or is not a file: {}",
                self.file_path.display()
            )));
        }

        // speed_multiplier must be finite and positive
        if !self.speed_multiplier.is_finite() {
            return Err(TelemetryError::InvalidArgument(format!(
                "speed_multiplier must be finite, got {}",
                self.speed_multiplier
            )));
        }
        if self.speed_multiplier <= 0.0 {
            return Err(TelemetryError::InvalidArgument(format!(
                "speed_multiplier must be > 0.0, got {}",
                self.speed_multiplier
            )));
        }

        // status_interval must be non-zero
        if self.status_interval.is_zero() {
            return Err(TelemetryError::InvalidArgument(
                "status_interval must be greater than zero".to_string(),
            ));
        }

        let mut registered = builtin_calculated_item_names();
        for item in &self.dashboard_realtime_items {
            if item.name.is_empty() {
                return Err(TelemetryError::InvalidArgument(
                    "dashboard realtime item name must not be empty".to_string(),
                ));
            }
            if !registered.insert(item.name.clone()) {
                return Err(TelemetryError::InvalidArgument(format!(
                    "dashboard realtime item '{}' is already registered",
                    item.name
                )));
            }
            let created = item.create_item();
            if created.name() != item.name {
                return Err(TelemetryError::InvalidArgument(format!(
                    "dashboard realtime item registration name '{}' does not match item name '{}'",
                    item.name,
                    created.name()
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> RecordingRequest {
        RecordingRequest {
            poll_hz: 60.0,
            output_dir: std::env::temp_dir(),
            status_interval: Duration::from_secs(1),
            dashboard_items: vec![],
            dashboard_realtime_items: vec![],
        }
    }

    #[test]
    fn test_validation_valid() {
        assert!(valid_request().validate().is_ok());
    }

    #[test]
    fn test_validation_poll_hz_too_low() {
        let mut r = valid_request();
        r.poll_hz = 10.0;
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_validation_poll_hz_too_high() {
        let mut r = valid_request();
        r.poll_hz = 200.0;
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_validation_poll_hz_nan() {
        let mut r = valid_request();
        r.poll_hz = f64::NAN;
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_validation_poll_hz_inf() {
        let mut r = valid_request();
        r.poll_hz = f64::INFINITY;
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_validation_poll_hz_boundaries() {
        let mut r = valid_request();
        r.poll_hz = 30.0;
        assert!(r.validate().is_ok());
        r.poll_hz = 120.0;
        assert!(r.validate().is_ok());
    }

    #[test]
    fn test_validation_output_dir_nonexistent() {
        let mut r = valid_request();
        r.output_dir = PathBuf::from("__nonexistent_dir_xyz__");
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_validation_status_interval_zero() {
        let mut r = valid_request();
        r.status_interval = Duration::ZERO;
        assert!(r.validate().is_err());
    }

    // ------------------------------------------------------------------
    // ReplayRequest tests
    // ------------------------------------------------------------------

    fn valid_replay_request() -> ReplayRequest {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("__replay_test_{}.acctlm", ts));
        // Best-effort write — if it fails (race), validation will fail at is_file()
        let _ = std::fs::write(&path, b"dummy content");
        ReplayRequest {
            file_path: path,
            speed_multiplier: 1.0,
            status_interval: Duration::from_secs(1),
            dashboard_items: vec![],
            dashboard_realtime_items: vec![],
        }
    }

    #[test]
    fn test_replay_validation_valid() {
        let r = valid_replay_request();
        assert!(r.validate().is_ok());
        // Clean up the temp file
        let _ = std::fs::remove_file(&r.file_path);
    }

    #[test]
    fn test_replay_validation_speed_zero() {
        let mut r = valid_replay_request();
        r.speed_multiplier = 0.0;
        assert!(r.validate().is_err());
        let _ = std::fs::remove_file(&r.file_path);
    }

    #[test]
    fn test_replay_validation_speed_negative() {
        let mut r = valid_replay_request();
        r.speed_multiplier = -1.0;
        assert!(r.validate().is_err());
        let _ = std::fs::remove_file(&r.file_path);
    }

    #[test]
    fn test_replay_validation_speed_nan() {
        let mut r = valid_replay_request();
        r.speed_multiplier = f64::NAN;
        assert!(r.validate().is_err());
        let _ = std::fs::remove_file(&r.file_path);
    }

    #[test]
    fn test_replay_validation_speed_inf() {
        let mut r = valid_replay_request();
        r.speed_multiplier = f64::INFINITY;
        assert!(r.validate().is_err());
        let _ = std::fs::remove_file(&r.file_path);
    }

    #[test]
    fn test_replay_validation_nonexistent_file() {
        let mut r = valid_replay_request();
        // Save original path before overwriting — we must clean up the temp file
        let original = r.file_path.clone();
        r.file_path = PathBuf::from("__nonexistent_replay_file_xyz__.acctlm");
        assert!(r.validate().is_err());
        let _ = std::fs::remove_file(&original);
    }

    #[test]
    fn test_replay_validation_status_interval_zero() {
        let mut r = valid_replay_request();
        r.status_interval = Duration::ZERO;
        assert!(r.validate().is_err());
        let _ = std::fs::remove_file(&r.file_path);
    }
}
