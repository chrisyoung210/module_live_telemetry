//! Recording outcome — result delivered to m1 when recording ends.

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::error::{TelemetryError, TelemetryResult};
use crate::format::LAP_INDEX_MAGIC;
use crate::item_key::{ItemKey, ItemType};
use crate::reader::BinaryTelemetryReader;
use crate::types::{
    CarStateSample, ControlSample, EnvironmentSample, LapIndexEntry, MotionSample, OtherCarsSample,
    PowertrainSample, SessionSample, TimingSample, TyreSample,
};

/// Summary of a single lap within a recording.
#[derive(Debug, Clone)]
pub struct LapSummary {
    /// Lap number (1-based).
    pub lap_number: u32,
    /// Whether ACC considers this a valid lap.
    pub is_valid: bool,
    /// Total lap time, if available.
    pub lap_time: Option<Duration>,
    /// Sector times, if available.
    pub split_times: Vec<Duration>,
}

/// Final result delivered to m1 via the outcome channel when recording ends.
#[derive(Debug, Clone)]
pub struct RecordingOutcome {
    /// Track name (from ACC static info).
    pub track_name: String,
    /// Car model (from ACC static info).
    pub car_model: String,
    /// Session type label (e.g. "PRACTICE", "RACE").
    pub session_type: String,
    /// Raw session type value from physics page (0-8).
    pub session_type_raw: i32,
    /// Path to the recorded `.acctlm2` file.
    pub file_path: PathBuf,
    /// File size in bytes.
    pub file_size_bytes: u64,
    /// Total frames recorded.
    pub total_samples: u64,
    /// Total recording duration.
    pub duration: Duration,
    /// Recording start date in "YYYY/MM/DD" format (China timezone, UTC+8).
    pub recording_date: String,
    /// Recording start time in "HH:MM:SS" format (China timezone, UTC+8).
    pub recording_time: String,
    /// Per-lap summaries (from in-memory lap tracker or file index).
    pub laps: Vec<LapSummary>,
}

/// Convert a [`SystemTime`] into `(recording_date, recording_time)` strings
/// in China timezone (UTC+8).
///
/// - `recording_date`: `"YYYY/MM/DD"` format
/// - `recording_time`: `"HH:MM:SS"` format
///
/// Uses the Howard Hinnant civil-date algorithm
/// (<https://howardhinnant.github.io/date_algorithms.html>).
pub fn format_recording_datetime(start: SystemTime) -> (String, String) {
    // Seconds since epoch in China timezone (UTC+8)
    let since_epoch = start.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = since_epoch.as_secs().saturating_add(8 * 3600);

    let days = total_secs / 86400;
    let time_of_day = total_secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Howard Hinnant's `civil_from_days` algorithm
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let final_y = if m <= 2 { y + 1 } else { y };

    let date = format!("{}/{:02}/{:02}", final_y, m, d);
    let time = format!("{:02}:{:02}:{:02}", hours, minutes, seconds);

    (date, time)
}

/// Map a physics page session value (0-8) to a human-readable label.
///
/// This is the same mapping table used by the CLI. Note that this is
/// DIFFERENT from `AccSessionKind::from_raw()` — that enum uses a
/// different numbering system (a known bug, not addressed here).
pub fn session_type_label(value: i32) -> &'static str {
    match value {
        0 => "PRACTICE",
        1 => "QUALIFY",
        2 => "RACE",
        3 => "HOTLAP",
        4 => "TIME_ATTACK",
        5 => "DRIFT",
        6 => "DRAG",
        7 => "HOTSTINT",
        8 => "HOTLAP_SUPERPOLE",
        _ => "UNKNOWN",
    }
}

/// Aggregated lap data extracted from session and timing data.
///
/// Provides tick boundaries, validity, and ACC timing data (`i_last_time`,
/// `i_best_time`) for each detected lap.
#[derive(Debug, Clone)]
pub struct AggregatedLap {
    pub lap_number: u32,
    pub start_tick: u64,
    pub end_tick: u64,
    pub sample_count: usize,
    pub is_valid: bool,
    pub is_out_lap: bool,
    pub last_time_ms: Option<i32>,
    pub best_time_ms: Option<i32>,
}

/// Aggregate lap data from session and timing samples in the reader.
///
/// Lap boundaries are detected via `normalized_car_position` wrapping
/// (0.8+ → 0.2−). Lap times come from ACC's `i_last_time` in timing
/// data, consistent with `laps_command`.
pub fn aggregate_laps(reader: &BinaryTelemetryReader) -> TelemetryResult<Vec<AggregatedLap>> {
    let session_samples = reader.read_all_session().unwrap_or_default();
    let timing_samples = reader.read_all_timing().unwrap_or_default();

    if session_samples.is_empty() {
        return Ok(Vec::new());
    }

    // --- Lap boundary detection ---
    // Uses `normalized_car_position` (0.0–1.0, wraps at S/F line).
    // When it drops from ~0.9+ to ~0.1- the car crossed start/finish.
    let mut crossings: Vec<usize> = Vec::new();
    for i in 1..session_samples.len() {
        let prev_pos = session_samples[i - 1].normalized_car_position;
        let cur_pos = session_samples[i].normalized_car_position;
        if prev_pos > 0.8 && cur_pos < 0.2 {
            crossings.push(i);
        }
    }

    let timing_ticks: Vec<u64> = timing_samples.iter().map(|t| t.sample_tick).collect();
    let mut laps: Vec<AggregatedLap> = Vec::new();
    let mut lap_start_idx: usize = 0;
    let mut lap_number: u32 = 0;

    for &cross_idx in &crossings {
        let start_tick = session_samples[lap_start_idx].sample_tick;
        let end_tick = session_samples[cross_idx.saturating_sub(1)].sample_tick;
        let sample_count = cross_idx - lap_start_idx;
        let is_out_lap = lap_number == 0;

        // Check last 3 samples of the lap for validity
        let end = cross_idx.saturating_sub(1);
        let start = end.saturating_sub(2).max(lap_start_idx);
        let is_valid = !is_out_lap
            && !session_samples[start..=end]
                .iter()
                .any(|s| s.is_valid_lap == 0);

        // Find timing data for this lap: look for i_last_time at/near the crossing
        let (last_time_ms, best_time_ms) = if !timing_samples.is_empty() {
            let timing_at_cross = timing_ticks
                .binary_search(&session_samples[cross_idx].sample_tick)
                .ok()
                .map(|idx| &timing_samples[idx])
                .or_else(|| {
                    timing_samples
                        .iter()
                        .filter(|t| {
                            t.sample_tick >= start_tick
                                && t.sample_tick <= session_samples[cross_idx].sample_tick
                        })
                        .max_by_key(|t| t.sample_tick)
                });
            timing_at_cross
                .map(|t| {
                    let last = if t.i_last_time > 0 && t.i_last_time < 2_000_000 {
                        Some(t.i_last_time)
                    } else {
                        None
                    };
                    let best = if t.i_best_time > 0 && t.i_best_time < 2_000_000 {
                        Some(t.i_best_time)
                    } else {
                        None
                    };
                    (last, best)
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        laps.push(AggregatedLap {
            lap_number,
            start_tick,
            end_tick,
            sample_count,
            is_valid,
            is_out_lap,
            last_time_ms,
            best_time_ms,
        });

        lap_start_idx = cross_idx;
        lap_number += 1;
    }

    // Handle remaining samples as the last (possibly incomplete) lap
    if lap_start_idx < session_samples.len() {
        let last_idx = session_samples.len() - 1;
        let start_tick = session_samples[lap_start_idx].sample_tick;
        let end_tick = session_samples[last_idx].sample_tick;
        let sample_count = last_idx - lap_start_idx + 1;
        let is_out_lap = lap_number == 0;
        let end_3 = last_idx.saturating_sub(2).max(lap_start_idx);
        let is_valid = !is_out_lap
            && !session_samples[end_3..=last_idx]
                .iter()
                .any(|s| s.is_valid_lap == 0);

        laps.push(AggregatedLap {
            lap_number,
            start_tick,
            end_tick,
            sample_count,
            is_valid,
            is_out_lap,
            last_time_ms: None,
            best_time_ms: None,
        });
    }

    Ok(laps)
}

/// Parse an existing `.acctlm` (v1) or `.acctlm2` (v2) file and extract its [`RecordingOutcome`].
///
/// This is the import API for telemetry files recorded by previous software
/// versions as well as the current `.acctlm2` format. It auto-detects the
/// format and validates the binary file structure (magic number, offsets,
/// schema hash, version) and returns a [`TelemetryError`](crate::error::TelemetryError)
/// if the file is malformed, truncated, or otherwise invalid.
///
/// File and session metadata management is handled by the parent module;
/// this function only requires a file path.
///
/// # Duration estimation
///
/// When reading a file, the exact wall-clock recording duration is not
/// stored. The duration is estimated from `total_samples / poll_hz`.
///
/// # Lap times
///
/// Lap times are derived from ACC's `i_last_time` in timing data
/// (consistent with the `laps` CLI command). Out laps and incomplete
/// laps have `lap_time=None`. Sector-level split times are not
/// available from the file index and will always be empty.
///
/// # Errors
///
/// | Condition | Error Variant |
/// |---|---|
/// | File not found / unreadable | [`TelemetryError::Io`](crate::error::TelemetryError::Io) |
/// | Bad magic / truncated header | [`TelemetryError::InvalidFormat`](crate::error::TelemetryError::InvalidFormat) |
/// | Unsupported version | [`TelemetryError::UnsupportedVersion`](crate::error::TelemetryError::UnsupportedVersion) |
/// | Invalid schema hash | [`TelemetryError::InvalidFormat`](crate::error::TelemetryError::InvalidFormat) |
///
/// # Example
///
/// ```no_run
/// use module_live_telemetry::recording::parse_acctlm_file;
///
/// let outcome = parse_acctlm_file("recording_20260606.acctlm2")?;
/// println!("Track: {}, Car: {}", outcome.track_name, outcome.car_model);
/// println!("Session: {}, Frames: {}", outcome.session_type, outcome.total_samples);
/// # Ok::<(), module_live_telemetry::TelemetryError>(())
/// ```
pub fn parse_acctlm_file(path: impl AsRef<Path>) -> TelemetryResult<RecordingOutcome> {
    let path = path.as_ref();
    let reader = BinaryTelemetryReader::open(path)?;
    let metadata = reader.metadata();
    let summary = reader.summary();

    let session_type_raw = metadata.session_type.unwrap_or(0);
    let session_type = session_type_label(session_type_raw).to_string();

    // Duration: estimate from poll_hz when not measured (always the case for imports)
    let duration = if summary.duration > Duration::ZERO {
        summary.duration
    } else {
        Duration::from_secs_f64(summary.total_samples as f64 / metadata.poll_hz.max(1.0))
    };

    let start = UNIX_EPOCH + Duration::from_nanos(metadata.created_unix_ns);
    let (recording_date, recording_time) = format_recording_datetime(start);

    let aggregated = aggregate_laps(&reader)?;
    let laps: Vec<LapSummary> = aggregated
        .iter()
        .map(|lap| {
            let lap_time = if lap.is_out_lap {
                None
            } else {
                lap.last_time_ms.map(|ms| Duration::from_millis(ms as u64))
            };
            LapSummary {
                lap_number: lap.lap_number,
                is_valid: lap.is_valid,
                lap_time,
                split_times: Vec::new(),
            }
        })
        .collect();

    Ok(RecordingOutcome {
        track_name: metadata.track_name.clone(),
        car_model: metadata.car_model.clone(),
        session_type,
        session_type_raw,
        file_path: path.to_path_buf(),
        file_size_bytes: summary.total_bytes,
        total_samples: summary.total_samples,
        duration,
        recording_date,
        recording_time,
        laps,
    })
}

/// Append a lap index block to an existing `.acctlm2` file.
///
/// Detects lap boundaries from session data via `normalized_car_position`
/// wrapping and writes a `LAPS` block after the footer. This makes the
/// lap data available to readers that use `lap_index()`.
///
/// Silently returns `Ok(())` if the file has insufficient session data
/// (fewer than 2 samples).
///
/// This is called automatically by [`RecordingController`] after recording
/// ends, and manually via the `build-lap-index` CLI command.
pub fn append_lap_index(path: &Path) -> TelemetryResult<usize> {
    // V2 files already have lap index embedded in the footer; skip V1-style append.
    {
        let mut file = std::fs::File::open(path)?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic).map_err(|e| {
            TelemetryError::InvalidFormat(format!(
                "cannot read file magic for lap index append: {e}"
            ))
        })?;
        if &magic == b"ACT2" {
            return Ok(0);
        }
    }

    let reader = BinaryTelemetryReader::open(path)?;
    // Use lightweight read — only (tick, norm_pos, is_valid_lap) instead of full SessionSample
    let lap_data = reader.read_lap_boundary_data().unwrap_or_default();
    if lap_data.len() < 2 {
        return Ok(0);
    }

    // Detect lap boundaries via normalized_car_position wrapping
    let mut crossings: Vec<usize> = Vec::new();
    for i in 1..lap_data.len() {
        let prev_pos = lap_data[i - 1].1;
        let cur_pos = lap_data[i].1;
        if prev_pos > 0.8 && cur_pos < 0.2 {
            crossings.push(i);
        }
    }

    let mut entries: Vec<LapIndexEntry> = Vec::new();
    let mut start_idx: usize = 0;
    let mut lap_number: i32 = 0;

    for &cross_idx in &crossings {
        entries.push(LapIndexEntry {
            lap_number,
            start_tick: lap_data[start_idx].0,
            end_tick: lap_data[cross_idx - 1].0,
            sample_count: (cross_idx - start_idx) as u32,
            is_valid: (lap_number != 0 && lap_data[cross_idx - 1].2 != 0) as i32,
            is_out_lap: (lap_number == 0) as i32,
        });
        start_idx = cross_idx;
        lap_number += 1;
    }

    // Last (possibly incomplete) lap
    let last_idx = lap_data.len() - 1;
    entries.push(LapIndexEntry {
        lap_number,
        start_tick: lap_data[start_idx].0,
        end_tick: lap_data[last_idx].0,
        sample_count: (last_idx - start_idx + 1) as u32,
        is_valid: (lap_number != 0) as i32,
        is_out_lap: (lap_number == 0) as i32,
    });

    let count = entries.len();
    // Append to file
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(&LAP_INDEX_MAGIC)?;
    file.write_all(&(count as u32).to_le_bytes())?;
    for e in &entries {
        file.write_all(&e.lap_number.to_le_bytes())?;
        file.write_all(&e.start_tick.to_le_bytes())?;
        file.write_all(&e.end_tick.to_le_bytes())?;
        file.write_all(&e.sample_count.to_le_bytes())?;
        file.write_all(&e.is_valid.to_le_bytes())?;
        file.write_all(&e.is_out_lap.to_le_bytes())?;
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// extract_lap_telemetry — per-lap raw field extraction
// ---------------------------------------------------------------------------

/// Validate that a field name exists in a given sample type's catalog.
fn validate_raw_field(substruct: &str, field: &str) -> bool {
    let names: &[&str] = match substruct {
        "car_state" => CarStateSample::raw_field_names(),
        "controls" => ControlSample::raw_field_names(),
        "environment" => EnvironmentSample::raw_field_names(),
        "motion" => MotionSample::raw_field_names(),
        "other_cars" => OtherCarsSample::raw_field_names(),
        "powertrain" => PowertrainSample::raw_field_names(),
        "session" => SessionSample::raw_field_names(),
        "timing" => TimingSample::raw_field_names(),
        "tyres" => TyreSample::raw_field_names(),
        _ => return false,
    };
    names.contains(&field)
}

/// Extract per-lap telemetry data for specified raw fields from an `.acctlm2` file.
///
/// Returns a `Vec` where each element (indexed by lap number, 0-based) is a
/// `HashMap<String, Vec<f64>>` mapping each requested field key to its per-frame
/// values within that lap.
///
/// # Arguments
///
/// * `path` - Path to the `.acctlm2` file.
/// * `keys` - Set of [`ItemKey`] in `raw:cluster.field` format (as returned by
///   [`crate::raw_catalog::all_raw_items`]).
///
/// # Returns
///
/// `Vec<HashMap<String, Vec<f64>>>` — outer `Vec` indexed by lap number
/// (lap 0 = out-lap, lap 1 = first flying lap, …), inner map keyed by the
/// original `ItemKey` string.
///
/// # Errors
///
/// Returns [`TelemetryError::InvalidArgument`] if any key is not a `raw` type
/// or references an unknown substruct / field.
///
/// # Example
///
/// ```ignore
/// use std::collections::HashSet;
/// use module_live_telemetry::item_key::ItemKey;
/// use module_live_telemetry::recording::extract_lap_telemetry;
///
/// let mut keys = HashSet::new();
/// keys.insert(ItemKey::parse("raw:controls.speed_kmh").unwrap());
/// keys.insert(ItemKey::parse("raw:controls.brake").unwrap());
/// keys.insert(ItemKey::parse("raw:session.normalized_car_position").unwrap());
///
/// let laps = extract_lap_telemetry("recording.acctlm2", &keys).unwrap();
/// // laps[0] → lap 0 (out-lap): { "raw:controls.speed_kmh": [...], ... }
/// // laps[1] → lap 1:            { "raw:controls.speed_kmh": [...], ... }
/// ```
pub fn extract_lap_telemetry(
    path: impl AsRef<Path>,
    keys: &HashSet<ItemKey>,
) -> TelemetryResult<Vec<HashMap<String, Vec<f64>>>> {
    extract_lap_telemetry_impl(path, keys, None)
}

/// Extract telemetry data for specified raw fields from specific laps in an `.acctlm2` file.
///
/// Returns only the laps specified by `lap_numbers`, in the same order.
/// An empty `lap_numbers` returns an empty result.
///
/// # Errors
///
/// Returns [`TelemetryError::InvalidArgument`] if any requested lap index is out of range.
pub fn extract_laps_telemetry(
    path: impl AsRef<std::path::Path>,
    keys: &HashSet<ItemKey>,
    lap_numbers: &[usize],
) -> TelemetryResult<Vec<HashMap<String, Vec<f64>>>> {
    if lap_numbers.is_empty() {
        return Ok(Vec::new());
    }
    extract_lap_telemetry_impl(path, keys, Some(lap_numbers))
}

// ── Internal implementation shared by both public APIs ──

fn extract_lap_telemetry_impl(
    path: impl AsRef<Path>,
    keys: &HashSet<ItemKey>,
    lap_filter: Option<&[usize]>,
) -> TelemetryResult<Vec<HashMap<String, Vec<f64>>>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    // ── Phase 0: validate & parse all keys ──

    struct Parsed<'a> {
        original: &'a ItemKey,
        substruct: &'a str, // e.g. "controls", "" for top-level
        field: &'a str,     // e.g. "speed_kmh"
    }

    let mut parsed: Vec<Parsed> = Vec::with_capacity(keys.len());
    let mut need_controls = false;
    let mut need_motion = false;
    let mut need_tyres = false;
    let mut need_powertrain = false;
    let mut need_timing = false;
    let mut need_car_state = false;
    let mut need_environment = false;
    let mut need_other_cars = false;

    for key in keys {
        if key.item_type != ItemType::Raw {
            return Err(TelemetryError::InvalidArgument(format!(
                "key '{}' is not a raw field (type is {:?})",
                key, key.item_type
            )));
        }

        let (substruct, field) = if let Some(pos) = key.name.find('.') {
            (&key.name[..pos], &key.name[pos + 1..])
        } else {
            // top-level field: sample_tick, timestamp_ns
            ("", key.name.as_str())
        };

        // Validate substruct
        if !substruct.is_empty() {
            if !validate_raw_field(substruct, field) {
                return Err(TelemetryError::InvalidArgument(format!(
                    "unknown field '{}' in substruct '{}' (key '{}')",
                    field, substruct, key
                )));
            }
            match substruct {
                "car_state" => need_car_state = true,
                "controls" => need_controls = true,
                "environment" => need_environment = true,
                "motion" => need_motion = true,
                "other_cars" => need_other_cars = true,
                "powertrain" => need_powertrain = true,
                "session" => {} // session is always read for lap boundaries
                "timing" => need_timing = true,
                "tyres" => need_tyres = true,
                _ => unreachable!(), // caught by validate_raw_field
            }
        } else {
            match field {
                "sample_tick" | "timestamp_ns" => {}
                _ => {
                    return Err(TelemetryError::InvalidArgument(format!(
                        "unknown top-level field '{}'",
                        field
                    )));
                }
            }
        }

        parsed.push(Parsed {
            original: key,
            substruct,
            field,
        });
    }

    // ── Phase 1: open file & read session data (needed for lap boundaries) ──

    let reader = BinaryTelemetryReader::open(path.as_ref())?;

    // Always read session — we need it for lap boundaries and the timeline.
    // We reuse the aggregate_laps helper, but aggregate_laps reads session
    // internally. To avoid double-read we read session here and detect
    // boundaries inline.
    let session_samples = reader.read_all_session()?;
    if session_samples.is_empty() {
        return Ok(Vec::new());
    }

    // ── Phase 2: detect lap boundaries from session data ──

    let mut crossings: Vec<usize> = Vec::new();
    for i in 1..session_samples.len() {
        if session_samples[i - 1].normalized_car_position > 0.8
            && session_samples[i].normalized_car_position < 0.2
        {
            crossings.push(i);
        }
    }

    struct LapRange {
        start_tick: u64,
        end_tick: u64,
    }

    let mut lap_ranges: Vec<LapRange> = Vec::new();
    let mut lap_start_idx: usize = 0;
    for &cross_idx in &crossings {
        lap_ranges.push(LapRange {
            start_tick: session_samples[lap_start_idx].sample_tick,
            end_tick: session_samples[cross_idx.saturating_sub(1)].sample_tick,
        });
        lap_start_idx = cross_idx;
    }
    // Last (possibly incomplete) lap
    if lap_start_idx < session_samples.len() {
        let last = session_samples.len() - 1;
        lap_ranges.push(LapRange {
            start_tick: session_samples[lap_start_idx].sample_tick,
            end_tick: session_samples[last].sample_tick,
        });
    }

    // ── Phase 3: determine which laps to extract ──

    let lap_indices: Vec<usize> = match lap_filter {
        Some(nums) => {
            for &idx in nums {
                if idx >= lap_ranges.len() {
                    return Err(TelemetryError::InvalidArgument(format!(
                        "lap index {} not found (total laps: {})",
                        idx,
                        lap_ranges.len()
                    )));
                }
            }
            nums.to_vec()
        }
        None => (0..lap_ranges.len()).collect(),
    };

    // ── Phase 4+5: read clusters & extract per-lap values ──
    //
    // Two code paths:
    //   - lap_filter = None (all laps): bulk-read all clusters once, then
    //     iterate laps with in-memory tick lookups. Optimal when reading
    //     most or all laps.
    //   - lap_filter = Some (specific laps): per-lap range reads via
    //     v2 skip-index (or v1 read-all + filter). Each lap reads only
    //     its own tick range, avoiding wasted I/O on unrelated laps.

    if lap_filter.is_some() {
        // ── Per-lap path: read only requested laps ──

        let mut result: Vec<HashMap<String, Vec<f64>>> = Vec::with_capacity(lap_indices.len());

        for &lap_idx in &lap_indices {
            let lap = &lap_ranges[lap_idx];
            let start = lap.start_tick;
            let end = lap.end_tick;

            // Read only this lap's data for each needed cluster
            let controls_data = if need_controls {
                reader.read_controls_range(start, end).unwrap_or_default()
            } else {
                Vec::new()
            };
            let controls_by_tick: HashMap<u64, &ControlSample> =
                controls_data.iter().map(|s| (s.sample_tick, s)).collect();

            let motion_data = if need_motion {
                reader.read_motion_range(start, end).unwrap_or_default()
            } else {
                Vec::new()
            };
            let motion_by_tick: HashMap<u64, &MotionSample> =
                motion_data.iter().map(|s| (s.sample_tick, s)).collect();

            let tyres_data = if need_tyres {
                reader.read_tyres_range(start, end).unwrap_or_default()
            } else {
                Vec::new()
            };
            let tyres_by_tick: HashMap<u64, &TyreSample> =
                tyres_data.iter().map(|s| (s.sample_tick, s)).collect();

            let powertrain_data = if need_powertrain {
                reader.read_powertrain_range(start, end).unwrap_or_default()
            } else {
                Vec::new()
            };
            let powertrain_by_tick: HashMap<u64, &PowertrainSample> =
                powertrain_data.iter().map(|s| (s.sample_tick, s)).collect();

            let timing_data = if need_timing {
                reader.read_timing_range(start, end).unwrap_or_default()
            } else {
                Vec::new()
            };
            let timing_by_tick: HashMap<u64, &TimingSample> =
                timing_data.iter().map(|s| (s.sample_tick, s)).collect();

            let car_state_data = if need_car_state {
                reader.read_car_state_range(start, end).unwrap_or_default()
            } else {
                Vec::new()
            };
            let car_state_by_tick: HashMap<u64, &CarStateSample> =
                car_state_data.iter().map(|s| (s.sample_tick, s)).collect();

            let env_data = if need_environment {
                reader
                    .read_environment_range(start, end)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let env_by_tick: HashMap<u64, &EnvironmentSample> =
                env_data.iter().map(|s| (s.sample_tick, s)).collect();

            let other_cars_data = if need_other_cars {
                reader.read_other_cars_range(start, end).unwrap_or_default()
            } else {
                Vec::new()
            };
            let other_cars_by_tick: HashMap<u64, &OtherCarsSample> =
                other_cars_data.iter().map(|s| (s.sample_tick, s)).collect();

            // Extract this lap's values
            let mut lap_map: HashMap<String, Vec<f64>> = HashMap::new();
            for p in &parsed {
                lap_map.insert(p.original.to_string(), Vec::new());
            }

            for s in &session_samples {
                if s.sample_tick < start || s.sample_tick > end {
                    if s.sample_tick > end {
                        break; // session samples are sorted by tick
                    }
                    continue;
                }

                for p in &parsed {
                    let value: Option<f64> = if p.substruct.is_empty() {
                        match p.field {
                            "sample_tick" => Some(s.sample_tick as f64),
                            "timestamp_ns" => Some(s.timestamp_ns as f64),
                            _ => None,
                        }
                    } else {
                        match p.substruct {
                            "car_state" => car_state_by_tick
                                .get(&s.sample_tick)
                                .and_then(|cs| cs.raw_field_value(p.field)),
                            "controls" => controls_by_tick
                                .get(&s.sample_tick)
                                .and_then(|c| c.raw_field_value(p.field)),
                            "environment" => env_by_tick
                                .get(&s.sample_tick)
                                .and_then(|e| e.raw_field_value(p.field)),
                            "motion" => motion_by_tick
                                .get(&s.sample_tick)
                                .and_then(|m| m.raw_field_value(p.field)),
                            "other_cars" => other_cars_by_tick
                                .get(&s.sample_tick)
                                .and_then(|oc| oc.raw_field_value(p.field)),
                            "powertrain" => powertrain_by_tick
                                .get(&s.sample_tick)
                                .and_then(|pt| pt.raw_field_value(p.field)),
                            "session" => s.raw_field_value(p.field),
                            "timing" => timing_by_tick
                                .get(&s.sample_tick)
                                .and_then(|t| t.raw_field_value(p.field)),
                            "tyres" => tyres_by_tick
                                .get(&s.sample_tick)
                                .and_then(|t| t.raw_field_value(p.field)),
                            _ => None,
                        }
                    };

                    if let Some(v) = value {
                        lap_map.get_mut(&p.original.to_string()).unwrap().push(v);
                    }
                }
            }

            result.push(lap_map);
        }

        return Ok(result);
    }

    // ── Bulk-read path (lap_filter = None): read all laps ──

    let controls_data = if need_controls {
        reader.read_all_controls().unwrap_or_default()
    } else {
        Vec::new()
    };
    let controls_by_tick: HashMap<u64, &ControlSample> =
        controls_data.iter().map(|s| (s.sample_tick, s)).collect();

    let motion_data = if need_motion {
        reader.read_all_motion().unwrap_or_default()
    } else {
        Vec::new()
    };
    let motion_by_tick: HashMap<u64, &MotionSample> =
        motion_data.iter().map(|s| (s.sample_tick, s)).collect();

    let tyres_data = if need_tyres {
        reader.read_all_tyres().unwrap_or_default()
    } else {
        Vec::new()
    };
    let tyres_by_tick: HashMap<u64, &TyreSample> =
        tyres_data.iter().map(|s| (s.sample_tick, s)).collect();

    let powertrain_data = if need_powertrain {
        reader.read_all_powertrain().unwrap_or_default()
    } else {
        Vec::new()
    };
    let powertrain_by_tick: HashMap<u64, &PowertrainSample> =
        powertrain_data.iter().map(|s| (s.sample_tick, s)).collect();

    let timing_data = if need_timing {
        reader.read_all_timing().unwrap_or_default()
    } else {
        Vec::new()
    };
    let timing_by_tick: HashMap<u64, &TimingSample> =
        timing_data.iter().map(|s| (s.sample_tick, s)).collect();

    let car_state_data = if need_car_state {
        reader.read_all_car_state().unwrap_or_default()
    } else {
        Vec::new()
    };
    let car_state_by_tick: HashMap<u64, &CarStateSample> =
        car_state_data.iter().map(|s| (s.sample_tick, s)).collect();

    let env_data = if need_environment {
        reader.read_all_environment().unwrap_or_default()
    } else {
        Vec::new()
    };
    let env_by_tick: HashMap<u64, &EnvironmentSample> =
        env_data.iter().map(|s| (s.sample_tick, s)).collect();

    let other_cars_data = if need_other_cars {
        reader.read_all_other_cars().unwrap_or_default()
    } else {
        Vec::new()
    };
    let other_cars_by_tick: HashMap<u64, &OtherCarsSample> =
        other_cars_data.iter().map(|s| (s.sample_tick, s)).collect();

    // ── Phase 5: extract per-lap values ──

    let mut result: Vec<HashMap<String, Vec<f64>>> = Vec::with_capacity(lap_indices.len());

    for &lap_idx in &lap_indices {
        let lap = &lap_ranges[lap_idx];
        let mut lap_map: HashMap<String, Vec<f64>> = HashMap::new();
        for p in &parsed {
            lap_map.insert(p.original.to_string(), Vec::new());
        }

        for s in &session_samples {
            if s.sample_tick < lap.start_tick || s.sample_tick > lap.end_tick {
                if s.sample_tick > lap.end_tick {
                    break; // session samples are sorted by tick
                }
                continue;
            }

            for p in &parsed {
                let value: Option<f64> = if p.substruct.is_empty() {
                    match p.field {
                        "sample_tick" => Some(s.sample_tick as f64),
                        "timestamp_ns" => Some(s.timestamp_ns as f64),
                        _ => None,
                    }
                } else {
                    match p.substruct {
                        "car_state" => car_state_by_tick
                            .get(&s.sample_tick)
                            .and_then(|cs| cs.raw_field_value(p.field)),
                        "controls" => controls_by_tick
                            .get(&s.sample_tick)
                            .and_then(|c| c.raw_field_value(p.field)),
                        "environment" => env_by_tick
                            .get(&s.sample_tick)
                            .and_then(|e| e.raw_field_value(p.field)),
                        "motion" => motion_by_tick
                            .get(&s.sample_tick)
                            .and_then(|m| m.raw_field_value(p.field)),
                        "other_cars" => other_cars_by_tick
                            .get(&s.sample_tick)
                            .and_then(|oc| oc.raw_field_value(p.field)),
                        "powertrain" => powertrain_by_tick
                            .get(&s.sample_tick)
                            .and_then(|pt| pt.raw_field_value(p.field)),
                        "session" => s.raw_field_value(p.field),
                        "timing" => timing_by_tick
                            .get(&s.sample_tick)
                            .and_then(|t| t.raw_field_value(p.field)),
                        "tyres" => tyres_by_tick
                            .get(&s.sample_tick)
                            .and_then(|t| t.raw_field_value(p.field)),
                        _ => None,
                    }
                };

                if let Some(v) = value {
                    // Safety: we inserted the key above
                    lap_map.get_mut(&p.original.to_string()).unwrap().push(v);
                }
            }
        }

        result.push(lap_map);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_type_label() {
        assert_eq!(session_type_label(0), "PRACTICE");
        assert_eq!(session_type_label(2), "RACE");
        assert_eq!(session_type_label(3), "HOTLAP");
        assert_eq!(session_type_label(8), "HOTLAP_SUPERPOLE");
        assert_eq!(session_type_label(-1), "UNKNOWN");
        assert_eq!(session_type_label(99), "UNKNOWN");
    }

    #[test]
    fn test_outcome_construction() {
        let outcome = RecordingOutcome {
            track_name: "monza".into(),
            car_model: "Ferrari 296 GT3".into(),
            session_type: "RACE".into(),
            session_type_raw: 2,
            file_path: PathBuf::from("/tmp/test.acctlm"),
            file_size_bytes: 1024,
            total_samples: 500,
            duration: Duration::from_secs(60),
            recording_date: "2026/06/06".into(),
            recording_time: "14:30:05".into(),
            laps: vec![
                LapSummary {
                    lap_number: 1,
                    is_valid: true,
                    lap_time: Some(Duration::from_secs(107)),
                    split_times: vec![
                        Duration::from_secs(35),
                        Duration::from_secs(35),
                        Duration::from_secs(37),
                    ],
                },
                LapSummary {
                    lap_number: 2,
                    is_valid: false,
                    lap_time: None,
                    split_times: vec![],
                },
            ],
        };

        assert_eq!(outcome.laps.len(), 2);
        assert_eq!(outcome.session_type, "RACE");
        assert_eq!(outcome.session_type_raw, 2);
        assert!(outcome.laps[0].is_valid);
        assert!(!outcome.laps[1].is_valid);
    }

    #[test]
    fn test_format_recording_datetime() {
        // 2026-06-06 14:30:05 UTC+8 = UNIX timestamp 1780727405 (UTC)
        let st = UNIX_EPOCH + Duration::from_secs(1780727405);
        let (date, time) = format_recording_datetime(st);
        assert_eq!(date, "2026/06/06");
        assert_eq!(time, "14:30:05");
    }
}
