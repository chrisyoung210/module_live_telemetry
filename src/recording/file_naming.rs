//! Safe file naming and exclusive creation for recording output.
//!
//! Public API files are auto-named and must not overwrite existing files.
//! The CLI retains its existing `File::create` behaviour for backward compat.

use crate::error::{TelemetryError, TelemetryResult};
use std::path::{Path, PathBuf};

/// Sanitize a string for use in a filename.
///
/// Replaces characters invalid on Windows (`<`, `>`, `:`, `"`, `/`, `\\`,
/// `|`, `?`, `*`) with `_`.
fn sanitize_filename_component(s: &str) -> String {
    const INVALID: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let mut out = s.to_string();
    for ch in INVALID {
        out = out.replace(*ch, "_");
    }
    // Replace spaces with underscores for cleaner filenames
    out = out.replace(' ', "_");
    // Collapse multiple underscores and trim
    let mut result = String::new();
    let mut last_was_underscore = false;
    for ch in out.chars() {
        if ch == '_' {
            if !last_was_underscore {
                result.push('_');
            }
            last_was_underscore = true;
        } else {
            result.push(ch);
            last_was_underscore = false;
        }
    }
    result.trim_matches('_').to_string()
}

/// Generate a default recording file name.
///
/// Format: `{sanitized_car}_{sanitized_track}_{unix_timestamp_secs}.acctlm`
pub fn default_recording_name(track_name: &str, car_model: &str) -> PathBuf {
    let car = sanitize_filename_component(car_model);
    let track = sanitize_filename_component(track_name);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    PathBuf::from(format!("{}_{}_{}.acctlm", car, track, ts))
}

/// Verify that `dir` is an existing directory.
///
/// Returns `Err(InvalidArgument)` if the path does not exist or is not
/// a directory. Does NOT auto-create the directory.
pub fn ensure_output_dir(dir: &Path) -> TelemetryResult<()> {
    if !dir.is_dir() {
        return Err(TelemetryError::InvalidArgument(format!(
            "output directory does not exist: {}",
            dir.display()
        )));
    }
    Ok(())
}

/// Build the output file path for a recording.
///
/// 1. Verifies `output_dir` exists.
/// 2. Joins the auto-generated name.
pub fn build_output_path(
    output_dir: &Path,
    track_name: &str,
    car_model: &str,
) -> TelemetryResult<PathBuf> {
    ensure_output_dir(output_dir)?;
    let name = default_recording_name(track_name, car_model);
    Ok(output_dir.join(name))
}

/// Check if a file already exists at the auto-named path.
///
/// Returns `Err(InvalidArgument)` if the file exists.
pub fn check_no_collision(output_dir: &Path, track_name: &str, car_model: &str) -> TelemetryResult<()> {
    let path = build_output_path(output_dir, track_name, car_model)?;
    if path.exists() {
        return Err(TelemetryError::InvalidArgument(format!(
            "output file already exists: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename_component() {
        let s = sanitize_filename_component("BMW M4 GT3");
        assert!(!s.contains('<'));
        assert!(!s.contains('>'));
        assert!(!s.contains(':'));
        assert!(!s.contains('*'));
    }

    #[test]
    fn test_default_recording_name_format() {
        let name = default_recording_name("nurburgring", "BMW M4 GT3");
        let s = name.to_string_lossy().to_string();
        assert!(s.ends_with(".acctlm"));
        assert!(s.contains("BMW_M4_GT3"));
        assert!(s.contains("nurburgring"));
    }

    #[test]
    fn test_ensure_output_dir_nonexistent() {
        let result = ensure_output_dir(Path::new("__nonexistent_dir_12345__"));
        assert!(result.is_err());
        if let Err(TelemetryError::InvalidArgument(msg)) = result {
            assert!(msg.contains("does not exist"));
        } else {
            panic!("expected InvalidArgument");
        }
    }

    #[test]
    fn test_ensure_output_dir_valid() {
        // Use a temp dir that exists
        let dir = std::env::temp_dir();
        assert!(ensure_output_dir(&dir).is_ok());
    }

    #[test]
    fn test_build_output_path() {
        let dir = std::env::temp_dir();
        let path = build_output_path(&dir, "monza", "Ferrari 296 GT3").unwrap();
        assert!(path.starts_with(&dir));
        let s = path.to_string_lossy();
        assert!(s.contains("Ferrari_296_GT3"));
        assert!(s.contains("monza"));
        assert!(s.ends_with(".acctlm"));
    }

    #[test]
    fn test_existing_file_collision() -> TelemetryResult<()> {
        let dir = std::env::temp_dir();
        let path = build_output_path(&dir, "test_collision_track", "test_collision_car")?;

        // Create a sentinel file
        std::fs::write(&path, b"sentinel")?;

        // Check collision
        let result = check_no_collision(&dir, "test_collision_track", "test_collision_car");
        assert!(result.is_err());
        if let Err(TelemetryError::InvalidArgument(msg)) = &result {
            assert!(msg.contains("already exists"));
        }

        // Verify sentinel unchanged
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "sentinel");

        // Cleanup
        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
