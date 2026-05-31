use crate::types::{AccSessionKind, RawGraphicsSample, SessionMetadata};

const DUPLICATE_BOUNDARY_WINDOW_SAMPLES: usize = 1;
const MIN_RESET_PREVIOUS_LAP_TIME_MS: i32 = 10_000;
const MAX_RESET_CURRENT_LAP_TIME_MS: i32 = 2_000;

#[derive(Debug, Clone)]
pub struct RawSessionSegments {
    pub metadata: SessionMetadata,
    pub session_type: Option<i32>,
    pub session_kind: Option<AccSessionKind>,
    pub sample_count: usize,
    pub start_time_ns: Option<u64>,
    pub end_time_ns: Option<u64>,
    pub laps: Vec<RawLapSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LapBoundaryReason {
    CompletedLapsChanged,
    CurrentLapReset,
}

impl LapBoundaryReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::CompletedLapsChanged => "completed_laps_changed",
            Self::CurrentLapReset => "current_lap_reset",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RawLapSegment {
    pub lap_index: usize,
    pub acc_completed_laps_at_start: i32,
    pub acc_completed_laps_at_end: i32,
    pub start_sample_index: usize,
    pub end_sample_index: usize,
    pub start_tick: u64,
    pub end_tick: u64,
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub sample_count: usize,
    pub is_complete: bool,
    pub boundary_reason: Option<LapBoundaryReason>,
    pub lap_time_ms: Option<i32>,
    pub is_valid: Option<bool>,
    pub min_normalized_car_position: f32,
    pub max_normalized_car_position: f32,
    pub start_distance_traveled_m: f32,
    pub end_distance_traveled_m: f32,
}

impl RawLapSegment {
    pub fn duration_ns(&self) -> u64 {
        self.end_time_ns.saturating_sub(self.start_time_ns)
    }
}

pub fn segment_raw_session(
    metadata: &SessionMetadata,
    samples: &[RawGraphicsSample],
) -> RawSessionSegments {
    let laps = segment_raw_graphics_laps(samples);
    let session_type = samples.first().map(|sample| sample.session);
    RawSessionSegments {
        metadata: metadata.clone(),
        session_type,
        session_kind: session_type.map(AccSessionKind::from_raw),
        sample_count: samples.len(),
        start_time_ns: samples.first().map(|sample| sample.timestamp_ns),
        end_time_ns: samples.last().map(|sample| sample.timestamp_ns),
        laps,
    }
}

pub fn segment_raw_graphics_laps(samples: &[RawGraphicsSample]) -> Vec<RawLapSegment> {
    if samples.is_empty() {
        return Vec::new();
    }

    let boundaries = detect_lap_boundaries(samples);
    let mut laps = Vec::with_capacity(boundaries.len() + 1);
    let mut start = 0usize;

    for (boundary_index, reason) in boundaries {
        if boundary_index > start {
            laps.push(build_lap_segment(
                laps.len(),
                samples,
                start,
                boundary_index,
                true,
                Some(reason),
            ));
        }
        start = boundary_index;
    }

    if start < samples.len() {
        laps.push(build_lap_segment(
            laps.len(),
            samples,
            start,
            samples.len(),
            false,
            None,
        ));
    }

    laps
}

fn detect_lap_boundaries(samples: &[RawGraphicsSample]) -> Vec<(usize, LapBoundaryReason)> {
    let mut boundaries = Vec::new();
    let mut last_boundary_index: Option<usize> = None;

    for i in 1..samples.len() {
        let prev = samples[i - 1];
        let cur = samples[i];
        let reason = if is_current_lap_reset(prev, cur) {
            Some(LapBoundaryReason::CurrentLapReset)
        } else if prev.completed_laps != cur.completed_laps {
            Some(LapBoundaryReason::CompletedLapsChanged)
        } else {
            None
        };

        let Some(reason) = reason else {
            continue;
        };

        if let Some(last) = last_boundary_index {
            if i <= last + DUPLICATE_BOUNDARY_WINDOW_SAMPLES {
                continue;
            }
        }

        boundaries.push((i, reason));
        last_boundary_index = Some(i);
    }

    boundaries
}

fn is_current_lap_reset(prev: RawGraphicsSample, cur: RawGraphicsSample) -> bool {
    prev.current_lap_time_ms > MIN_RESET_PREVIOUS_LAP_TIME_MS
        && cur.current_lap_time_ms < MAX_RESET_CURRENT_LAP_TIME_MS
        && cur.current_lap_time_ms < prev.current_lap_time_ms
}

fn build_lap_segment(
    lap_index: usize,
    samples: &[RawGraphicsSample],
    start: usize,
    end: usize,
    is_complete: bool,
    boundary_reason: Option<LapBoundaryReason>,
) -> RawLapSegment {
    debug_assert!(start < end);
    let first = samples[start];
    let last = samples[end - 1];
    let (min_normalized_car_position, max_normalized_car_position) =
        normalized_position_range(&samples[start..end]);

    RawLapSegment {
        lap_index,
        acc_completed_laps_at_start: first.completed_laps,
        acc_completed_laps_at_end: last.completed_laps,
        start_sample_index: start,
        end_sample_index: end,
        start_tick: first.sample_tick,
        end_tick: last.sample_tick,
        start_time_ns: first.timestamp_ns,
        end_time_ns: last.timestamp_ns,
        sample_count: end - start,
        is_complete,
        boundary_reason,
        lap_time_ms: if is_complete {
            finished_lap_time_ms(last)
        } else {
            None
        },
        is_valid: if is_complete {
            Some(last.is_valid_lap != 0)
        } else {
            None
        },
        min_normalized_car_position,
        max_normalized_car_position,
        start_distance_traveled_m: first.distance_traveled_m,
        end_distance_traveled_m: last.distance_traveled_m,
    }
}

fn normalized_position_range(samples: &[RawGraphicsSample]) -> (f32, f32) {
    samples
        .iter()
        .map(|sample| sample.normalized_car_position)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        })
}

fn finished_lap_time_ms(finish_sample: RawGraphicsSample) -> Option<i32> {
    if finish_sample.current_lap_time_ms > 0 {
        Some(finish_sample.current_lap_time_ms)
    } else if finish_sample.last_lap_time_ms > 0 && finish_sample.last_lap_time_ms != i32::MAX {
        Some(finish_sample.last_lap_time_ms)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_laps_and_skips_duplicate_boundary_samples() {
        let samples = vec![
            sample(0, 0, 0, 0, 0.0),
            sample(1, 0, 20_000, 0, 0.95),
            sample(2, 0, 0, 0, 1.0),
            sample(3, 1, 10, 20_100, 0.0),
            sample(4, 1, 30_000, 20_100, 0.5),
            sample(5, 1, 0, 20_100, 1.0),
            sample(6, 2, 12, 30_050, 0.0),
            sample(7, 2, 1_000, 30_050, 0.1),
        ];

        let laps = segment_raw_graphics_laps(&samples);
        assert_eq!(laps.len(), 3);
        assert_eq!(
            (laps[0].start_sample_index, laps[0].end_sample_index),
            (0, 2)
        );
        assert_eq!(
            (laps[1].start_sample_index, laps[1].end_sample_index),
            (2, 5)
        );
        assert_eq!(
            (laps[2].start_sample_index, laps[2].end_sample_index),
            (5, 8)
        );
        assert_eq!(
            laps[0].boundary_reason,
            Some(LapBoundaryReason::CurrentLapReset)
        );
        assert_eq!(
            laps[1].boundary_reason,
            Some(LapBoundaryReason::CurrentLapReset)
        );
        assert!(!laps[2].is_complete);
    }

    fn sample(
        sample_tick: u64,
        completed_laps: i32,
        current_lap_time_ms: i32,
        last_lap_time_ms: i32,
        normalized_car_position: f32,
    ) -> RawGraphicsSample {
        RawGraphicsSample {
            sample_tick,
            timestamp_ns: sample_tick * 1_000_000,
            status: 2,
            session: 3,
            completed_laps,
            current_lap_time_ms,
            last_lap_time_ms,
            best_lap_time_ms: i32::MAX,
            distance_traveled_m: sample_tick as f32 * 10.0,
            normalized_car_position,
            is_valid_lap: 1,
            current_sector: 0,
            last_sector_time_ms: 0,
            in_pit: 0,
            in_pit_lane: 0,
            delta_lap_time_ms: 0,
        }
    }
}
