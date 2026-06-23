//! Track map rendering from telemetry data.
//!
//! Extracts car world-coordinates from an acctlm2 file for a given lap
//! and renders a top-down (XZ-plane) track map as a PNG image.
//!
//! # Architecture
//!
//! The module is split into two layers so the coordinate data can be
//! reused by a future HTTP API without duplicating extraction logic:
//!
//! - [`extract_track_coordinates`] — reads an acctlm2 file, detects laps,
//!   and returns the player car's XZ coordinates for a given lap.
//! - [`render_track_png`] — takes coordinates and renders a PNG.
//! - [`generate_track_map`] — convenience function that chains both steps.

use std::path::Path;

use crate::error::TelemetryResult;
use crate::reader::BinaryTelemetryReader;
use crate::types::OtherCarsSample;

// ---------------------------------------------------------------------------
// Public API — data extraction
// ---------------------------------------------------------------------------

/// Player car's world-coordinate trajectory for a single lap.
///
/// Each entry is `[x, z]` in world-space (top-down view, Y = height ignored).
#[derive(Debug, Clone)]
pub struct TrackCoordinates {
    pub lap_number: usize,
    /// XZ pairs in sample order (one per telemetry frame within the lap).
    pub points: Vec<[f32; 2]>,
}

/// Extract the player car's track coordinates for a specific lap from an
/// acctlm2 file.
///
/// # Arguments
///
/// * `path` — path to an `.acctlm2` file.
/// * `lap_number` — 0-based lap index (0 = out-lap, 1 = first flying lap, …).
///
/// # Errors
///
/// Returns an error if the file cannot be read, no session data exists, or
/// `lap_number` is out of range.
pub fn extract_track_coordinates(
    path: impl AsRef<Path>,
    lap_number: usize,
) -> TelemetryResult<TrackCoordinates> {
    let reader = BinaryTelemetryReader::open(path.as_ref())?;

    // Phase 1 — detect lap boundaries from session (normalized_car_position)
    let session = reader.read_all_session()?;
    if session.is_empty() {
        return Err(crate::error::TelemetryError::InvalidArgument(
            "file contains no session data".to_string(),
        ));
    }

    let crossings = detect_lap_crossings(&session);
    let lap_ranges = build_lap_ranges(&session, &crossings);

    if lap_number >= lap_ranges.len() {
        return Err(crate::error::TelemetryError::InvalidArgument(format!(
            "lap {} out of range (total laps: {})",
            lap_number,
            lap_ranges.len()
        )));
    }

    let (start_tick, end_tick) = lap_ranges[lap_number];

    // Phase 2 — read other-cars data for the lap's tick range
    let other_cars: Vec<OtherCarsSample> =
        reader.read_other_cars_range(start_tick, end_tick)?;

    // Phase 3 — extract player's XZ coordinates
    let mut points: Vec<[f32; 2]> = Vec::with_capacity(other_cars.len());

    for sample in &other_cars {
        let player_idx = sample.player_car_id as usize;
        // car_coordinates is flat: 60 cars × 3 axes = 180 elements
        let base = player_idx * 3;
        if base + 2 < sample.car_coordinates.len() {
            let x = sample.car_coordinates[base];
            let z = sample.car_coordinates[base + 2];
            points.push([x, z]);
        }
    }

    Ok(TrackCoordinates {
        lap_number,
        points,
    })
}

// ---------------------------------------------------------------------------
// Public API — rendering
// ---------------------------------------------------------------------------

/// Render a list of 2D points as a track-map PNG.
///
/// # Arguments
///
/// * `points` — XZ coordinate pairs.
/// * `image_width` / `image_height` — output dimensions in pixels.
/// * `padding` — fraction of the bounding-box added as margin (e.g. 0.05 = 5%).
/// * `output_path` — where to write the PNG file.
///
/// The background is black and the track is drawn in yellow.
pub fn render_track_png(
    points: &[[f32; 2]],
    image_width: u32,
    image_height: u32,
    padding: f32,
    output_path: impl AsRef<Path>,
) -> TelemetryResult<()> {
    if points.len() < 2 {
        return Err(crate::error::TelemetryError::InvalidArgument(
            "need at least 2 points to draw a track".to_string(),
        ));
    }

    // Compute bounding box
    let (min_x, max_x, min_z, max_z) = compute_bounding_box(points);

    if (max_x - min_x) < 1e-6 || (max_z - min_z) < 1e-6 {
        return Err(crate::error::TelemetryError::InvalidArgument(
            "track coordinates have zero extent".to_string(),
        ));
    }

    // Apply padding
    let range_x = max_x - min_x;
    let range_z = max_z - min_z;
    let pad_x = range_x * padding;
    let pad_z = range_z * padding;

    let world_min_x = min_x - pad_x;
    let world_max_x = max_x + pad_x;
    let world_min_z = min_z - pad_z;
    let world_max_z = max_z + pad_z;

    let world_w = world_max_x - world_min_x;
    let world_h = world_max_z - world_min_z;

    // Scale to fit within the image while preserving aspect ratio.
    // Use the larger axis to fill, centre the other.
    let scale = if world_w / world_h > image_width as f32 / image_height as f32 {
        // track is wider than canvas — fit by width
        image_width as f32 / world_w
    } else {
        // track is taller — fit by height
        image_height as f32 / world_h
    };

    let img_w = (world_w * scale).round() as u32;
    let img_h = (world_h * scale).round() as u32;

    let offset_x = (image_width.saturating_sub(img_w) / 2) as f32;
    let offset_y = (image_height.saturating_sub(img_h) / 2) as f32;

    // Helper: world → image pixel
    let to_pixel = |wx: f32, wz: f32| -> (u32, u32) {
        let px = ((wx - world_min_x) * scale + offset_x).round() as u32;
        // Flip Z because image Y grows downward
        let py = ((world_max_z - wz) * scale + offset_y).round() as u32;
        (px.min(image_width - 1), py.min(image_height - 1))
    };

    // Create image (RGB 8-bit, black background)
    let mut img = image::RgbImage::new(image_width, image_height);
    // Background is already black (zeroed pixels in RgbImage)

    let track_color = image::Rgb([255u8, 255, 0]); // yellow

    // Draw line segments between consecutive points (Bresenham)
    for pair in points.windows(2) {
        let (x0, y0) = to_pixel(pair[0][0], pair[0][1]);
        let (x1, y1) = to_pixel(pair[1][0], pair[1][1]);
        draw_line(&mut img, x0, y0, x1, y1, track_color);
    }

    img.save(output_path.as_ref())
        .map_err(|e| crate::error::TelemetryError::Io(std::io::Error::other(e.to_string())))?;

    Ok(())
}

/// Convenience: extract coordinates AND render in one call.
///
/// This is what the CLI command uses.  For the future HTTP API you would
/// call [`extract_track_coordinates`] and [`render_track_png`] separately.
pub fn generate_track_map(
    input_path: impl AsRef<Path>,
    lap_number: usize,
    image_width: u32,
    image_height: u32,
    output_path: impl AsRef<Path>,
) -> TelemetryResult<TrackCoordinates> {
    let coords = extract_track_coordinates(input_path.as_ref(), lap_number)?;
    render_track_png(&coords.points, image_width, image_height, 0.05, output_path)?;
    Ok(coords)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Detect start/finish line crossings from normalized track position.
///
/// A crossing is detected when `normalized_car_position` drops from >0.8
/// to <0.2 between consecutive samples (0.0 → 1.0 wraps at S/F line).
fn detect_lap_crossings(session: &[crate::types::SessionSample]) -> Vec<usize> {
    let mut crossings = Vec::new();
    for i in 1..session.len() {
        if session[i - 1].normalized_car_position > 0.8
            && session[i].normalized_car_position < 0.2
        {
            crossings.push(i);
        }
    }
    crossings
}

/// Build (start_tick, end_tick) pairs for every lap.
fn build_lap_ranges(
    session: &[crate::types::SessionSample],
    crossings: &[usize],
) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    let mut start_idx = 0usize;

    for &cross_idx in crossings {
        let end_sample_idx = cross_idx.saturating_sub(1);
        ranges.push((
            session[start_idx].sample_tick,
            session[end_sample_idx].sample_tick,
        ));
        start_idx = cross_idx;
    }

    // Last (possibly incomplete) lap
    if start_idx < session.len() {
        let last = session.len() - 1;
        ranges.push((session[start_idx].sample_tick, session[last].sample_tick));
    }

    ranges
}

/// Compute axis-aligned bounding box of 2D points.
fn compute_bounding_box(points: &[[f32; 2]]) -> (f32, f32, f32, f32) {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_z = f32::MAX;
    let mut max_z = f32::MIN;

    for p in points {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_z = min_z.min(p[1]);
        max_z = max_z.max(p[1]);
    }

    (min_x, max_x, min_z, max_z)
}

/// Bresenham line drawing on an `RgbImage`.
fn draw_line(
    img: &mut image::RgbImage,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    color: image::Rgb<u8>,
) {
    let mut x = x0 as i32;
    let mut y = y0 as i32;
    let dx = (x1 as i32 - x0 as i32).abs();
    let dy = -(y1 as i32 - y0 as i32).abs();
    let sx = if x0 < x1 { 1i32 } else { -1i32 };
    let sy = if y0 < y1 { 1i32 } else { -1i32 };
    let mut err = dx + dy;

    loop {
        if (x as u32) < img.width() && (y as u32) < img.height() {
            img.put_pixel(x as u32, y as u32, color);
        }

        if x == x1 as i32 && y == y1 as i32 {
            break;
        }

        let e2 = 2 * err;
        if e2 >= dy {
            if x == x1 as i32 {
                break;
            }
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            if y == y1 as i32 {
                break;
            }
            err += dx;
            y += sy;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_bounding_box_single_point() {
        let points = [[10.0f32, 20.0]];
        let (min_x, max_x, min_z, max_z) = compute_bounding_box(&points);
        assert!((min_x - 10.0).abs() < 1e-6);
        assert!((max_x - 10.0).abs() < 1e-6);
        assert!((min_z - 20.0).abs() < 1e-6);
        assert!((max_z - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_compute_bounding_box_multiple() {
        let points = [[5.0, 10.0], [20.0, 3.0], [15.0, 25.0]];
        let (min_x, max_x, min_z, max_z) = compute_bounding_box(&points);
        assert!((min_x - 5.0).abs() < 1e-6);
        assert!((max_x - 20.0).abs() < 1e-6);
        assert!((min_z - 3.0).abs() < 1e-6);
        assert!((max_z - 25.0).abs() < 1e-6);
    }

    #[test]
    fn test_detect_lap_crossings_empty() {
        let session: Vec<crate::types::SessionSample> = Vec::new();
        let crossings = detect_lap_crossings(&session);
        assert!(crossings.is_empty());
    }

    #[test]
    fn test_build_lap_ranges_no_crossings() {
        let samples: Vec<crate::types::SessionSample> = (0..10)
            .map(|i| crate::types::SessionSample {
                sample_tick: i,
                timestamp_ns: 0,
                status: 2,
                session: 1,
                session_index: 0,
                completed_laps: 0,
                position: 1,
                session_time_left: 0.0,
                number_of_laps: 0,
                current_sector_index: 0,
                normalized_car_position: 0.5,
                is_in_pit: 0,
                is_in_pit_lane: 0,
                mandatory_pit_done: 0,
                missing_mandatory_pits: 0,
                penalty_time: 0.0,
                penalty_type: 0,
                track_status: [0u16; 33],
                clock: 0.0,
                replay_time_multiplier: 1.0,
                is_valid_lap: 1,
                global_yellow: 0,
                global_yellow1: 0,
                global_yellow2: 0,
                global_yellow3: 0,
                global_white: 0,
                global_green: 0,
                global_chequered: 0,
                global_red: 0,
                gap_ahead_or_tail_value: 0,
                flag: 0,
                gap_behind: 0,
            })
            .collect();

        let ranges = build_lap_ranges(&samples, &[]);
        // Should produce one lap covering all samples
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (0, 9));
    }
}
