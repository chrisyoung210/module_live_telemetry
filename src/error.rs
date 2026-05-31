use std::fmt::{Display, Formatter};

pub type TelemetryResult<T> = Result<T, TelemetryError>;

#[derive(Debug)]
pub enum TelemetryError {
    Io(std::io::Error),
    InvalidFormat(String),
    UnsupportedVersion(u16),
    InvalidArgument(String),
}

impl Display for TelemetryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::InvalidFormat(message) => write!(f, "invalid telemetry format: {message}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported telemetry format version: {version}")
            }
            Self::InvalidArgument(message) => write!(f, "invalid argument: {message}"),
        }
    }
}

impl std::error::Error for TelemetryError {}

impl From<std::io::Error> for TelemetryError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
