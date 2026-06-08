//! Recording request — public API input type and validation.

use crate::error::{TelemetryError, TelemetryResult};
use std::path::PathBuf;
use std::time::Duration;

/// Parameters for a recording session.
///
/// Channel endpoints (status, outcome, dashboard) are passed separately
/// to `RecordingController::start()` so this struct remains pure data.
#[derive(Debug, Clone)]
pub struct RecordingRequest {
    /// Polling frequency in Hz. Range: [30.0, 120.0].
    pub poll_hz: f64,
    /// Output directory. Must exist and be a directory.
    /// The actual file name is auto-generated.
    pub output_dir: PathBuf,
    /// Interval between status callbacks to m1.
    pub status_interval: Duration,
    /// Whether dashboard data collection is enabled.
    pub enable_dashboard: bool,
    /// Dashboard items to subscribe to (only meaningful when `enable_dashboard`).
    pub dashboard_items: Vec<super::DashboardItemSubscription>,
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
            enable_dashboard: false,
            dashboard_items: vec![],
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
}
