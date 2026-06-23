//! V2 binary telemetry reader 鈥?mmap-based selective column access.
//!
//! [`BinaryTelemetryReaderV2`] memory-maps the file and parses the header,
//! schema, and footer on open. Row groups are read on demand using the
//! skip index for O(1) navigation to column data.
//!
//! # Frame reconstruction
//!
//! Frames are reconstructed by reading all access groups and matching
//! rows by index. The `FrameMeta` group provides `sample_tick` and
//! `timestamp_ns`; every other group provides columns for one substructure.

use crate::encode_v2::decode_column;
use crate::error::{TelemetryError, TelemetryResult};
use crate::format::{read_f64, read_i32, read_u16, read_u32, read_u64, x1000_to_hz};
use crate::format_v2::{
    ColumnEntryV2, FileHeaderV2, FooterV2, GroupId, LapIndexEntryV2, RowGroupHeader, SchemaBlockV2,
    SkipIndexEntry,
};
use crate::mmap_win::MmapFile;
use crate::types::{
    CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
    PowertrainSample, RecordingSummary, SessionMetadata, SessionSample, TimingSample, TyreSample,
};
use crate::writer::TelemetryFrame;

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

// ---------------------------------------------------------------------------
// Decode metadata (same format as v1, shared with writer_v2)
// ---------------------------------------------------------------------------

fn decode_metadata_v2(bytes: &[u8]) -> TelemetryResult<SessionMetadata> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != *b"META" {
        return Err(TelemetryError::InvalidFormat(
            "bad metadata block".to_string(),
        ));
    }
    let created_unix_ns = read_u64(&mut cursor)?;
    let poll_hz = x1000_to_hz(read_u32(&mut cursor)?);
    let chunk_rows = read_u32(&mut cursor)? as usize;
    let track_len = read_u16(&mut cursor)? as usize;
    let car_len = read_u16(&mut cursor)? as usize;
    let mut track = vec![0u8; track_len];
    let mut car = vec![0u8; car_len];
    cursor.read_exact(&mut track)?;
    cursor.read_exact(&mut car)?;

    let remaining = bytes.len().saturating_sub(cursor.position() as usize);
    let (sm_version, ac_version, number_of_sessions, num_cars) = if remaining >= 12 {
        let sm_len = read_u16(&mut cursor).unwrap_or(0) as usize;
        let ac_len = read_u16(&mut cursor).unwrap_or(0) as usize;
        let ns = read_i32(&mut cursor).unwrap_or(0);
        let nc = read_i32(&mut cursor).unwrap_or(0);
        let mut sm = vec![0u8; sm_len];
        let mut ac = vec![0u8; ac_len];
        let _ = cursor.read_exact(&mut sm);
        let _ = cursor.read_exact(&mut ac);
        (
            String::from_utf8_lossy(&sm).into_owned(),
            String::from_utf8_lossy(&ac).into_owned(),
            ns,
            nc,
        )
    } else {
        (String::new(), String::new(), 0, 0)
    };

    let (sector_count, max_rpm, max_torque, max_power, max_fuel, penalties_enabled) = {
        let remaining2 = bytes.len().saturating_sub(cursor.position() as usize);
        if remaining2 >= 24 {
            let sc = read_i32(&mut cursor).unwrap_or(0);
            let mr = read_i32(&mut cursor).unwrap_or(0);
            let mt = read_f64(&mut cursor).unwrap_or(0.0) as f32;
            let mp = read_f64(&mut cursor).unwrap_or(0.0) as f32;
            let mf = read_f64(&mut cursor).unwrap_or(0.0) as f32;
            let pe = read_i32(&mut cursor).unwrap_or(0);
            (sc, mr, mt, mp, mf, pe)
        } else {
            (0, 0, 0.0, 0.0, 0.0, 0)
        }
    };

    Ok(SessionMetadata {
        track_name: String::from_utf8_lossy(&track).into_owned(),
        car_model: String::from_utf8_lossy(&car).into_owned(),
        created_unix_ns,
        poll_hz,
        chunk_rows,
        sm_version,
        ac_version,
        number_of_sessions,
        num_cars,
        sector_count,
        max_rpm,
        max_torque,
        max_power,
        max_fuel,
        penalties_enabled,
        raw_static_bytes: {
            let remaining3 = bytes.len().saturating_sub(cursor.position() as usize);
            if remaining3 >= 4 {
                let raw_len = read_u32(&mut cursor).unwrap_or(0) as usize;
                if raw_len > 0 && raw_len <= remaining3.saturating_sub(4) {
                    let mut raw = vec![0u8; raw_len];
                    let _ = cursor.read_exact(&mut raw);
                    raw
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        },
        session_type: {
            let remaining4 = bytes.len().saturating_sub(cursor.position() as usize);
            if remaining4 >= 8 {
                let has = read_i32(&mut cursor).unwrap_or(0);
                if has != 0 {
                    Some(read_i32(&mut cursor).unwrap_or(0))
                } else {
                    None
                }
            } else {
                None
            }
        },
    })
}

// ---------------------------------------------------------------------------
// BinaryTelemetryReaderV2
// ---------------------------------------------------------------------------

/// Intermediate parsed data from a byte slice.
struct ParsedFile {
    metadata: SessionMetadata,
    schema: SchemaBlockV2,
    skip_entries: Vec<SkipIndexEntry>,
    lap_entries: Vec<LapIndexEntryV2>,
    summary: RecordingSummary,
    row_group_offsets: Vec<u64>,
    total_samples: u64,
}

/// V2 binary telemetry reader with mmap-based I/O.
///
/// On `open()`, the file is memory-mapped and the header, schema,
/// metadata, and footer are parsed into memory. Row group data is
/// read on demand from the mmap'd region using the skip index.
pub struct BinaryTelemetryReaderV2 {
    /// In-memory byte buffer (for `from_bytes`). `mmap` is None.
    bytes: Option<Vec<u8>>,
    /// Memory-mapped file (for `open`). `bytes` is None.
    mmap: Option<MmapFile>,
    /// Parsed metadata.
    metadata: SessionMetadata,
    /// Parsed schema.
    schema: SchemaBlockV2,
    /// Skip index from footer.
    skip_entries: Vec<SkipIndexEntry>,
    /// Lap index from footer.
    lap_entries: Vec<LapIndexEntryV2>,
    /// Recording summary.
    summary: RecordingSummary,
    /// Row group metadata (parsed lazily or eagerly).
    row_group_offsets: Vec<u64>,
    /// total row count across all groups
    #[allow(dead_code)]
    total_samples: u64,
}

impl BinaryTelemetryReaderV2 {
    // -----------------------------------------------------------------------
    // Constructor 锟?file
    // -----------------------------------------------------------------------

    /// Open an acctlm2 file and memory-map it.
    pub fn open(path: impl AsRef<Path>) -> TelemetryResult<Self> {
        let mmap = MmapFile::open(path.as_ref())?;

        // Parse from mmap'd data (borrow must end before moving mmap)
        let parsed = {
            let data = mmap.as_slice();
            if data.is_empty() {
                return Err(TelemetryError::InvalidFormat("empty file".to_string()));
            }
            Self::parse_file_content(data)?
        };

        Ok(Self {
            bytes: None,
            mmap: Some(mmap),
            metadata: parsed.metadata,
            schema: parsed.schema,
            skip_entries: parsed.skip_entries,
            lap_entries: parsed.lap_entries,
            summary: parsed.summary,
            row_group_offsets: parsed.row_group_offsets,
            total_samples: parsed.total_samples,
        })
    }

    /// Create a reader from an in-memory byte buffer (for tests).
    pub fn from_bytes(bytes: Vec<u8>) -> TelemetryResult<Self> {
        let parsed = Self::parse_file_content(&bytes)?;
        Ok(Self {
            bytes: Some(bytes),
            mmap: None,
            metadata: parsed.metadata,
            schema: parsed.schema,
            skip_entries: parsed.skip_entries,
            lap_entries: parsed.lap_entries,
            summary: parsed.summary,
            row_group_offsets: parsed.row_group_offsets,
            total_samples: parsed.total_samples,
        })
    }

    /// Intermediate parsed data from a byte slice.
    fn parse_file_content(data: &[u8]) -> TelemetryResult<ParsedFile> {
        if data.len() < 64 {
            return Err(TelemetryError::InvalidFormat(
                "file too short for v2 header".to_string(),
            ));
        }

        // 1. Parse file header
        let mut cursor = Cursor::new(data);
        let header = FileHeaderV2::read_from(&mut cursor)?;

        // 2. Parse schema
        if header.schema_offset as usize >= data.len() {
            return Err(TelemetryError::InvalidFormat(
                "schema_offset past eof".to_string(),
            ));
        }
        let mut schema_cursor = Cursor::new(&data[header.schema_offset as usize..]);
        let schema = SchemaBlockV2::read_from(&mut schema_cursor)?;

        // 3. Parse metadata
        if header.metadata_offset as usize >= data.len() {
            return Err(TelemetryError::InvalidFormat(
                "metadata_offset past eof".to_string(),
            ));
        }
        if header.metadata_offset >= header.first_row_group_offset {
            return Err(TelemetryError::InvalidFormat(
                "metadata_offset >= first_row_group_offset".to_string(),
            ));
        }
        let meta_end = header.first_row_group_offset as usize;
        if meta_end > data.len() {
            return Err(TelemetryError::InvalidFormat(
                "first_row_group_offset past eof".to_string(),
            ));
        }
        let metadata = decode_metadata_v2(&data[header.metadata_offset as usize..meta_end])?;

        // 4. Parse footer (if present)
        let (skip_entries, lap_entries, footer_offset) =
            if header.footer_offset > 0 && (header.footer_offset as usize + 20) <= data.len() {
                let mut footer_cursor = Cursor::new(&data[header.footer_offset as usize..]);
                let footer = FooterV2::read_from(&mut footer_cursor)?;
                let skip = footer.read_skip_entries(&mut footer_cursor)?;
                let laps = footer.read_lap_entries(&mut footer_cursor)?;
                (skip, laps, header.footer_offset)
            } else {
                (Vec::new(), Vec::new(), 0u64)
            };

        // 5. Scan row group headers to collect offsets and total sample count
        let (row_group_offsets, total_samples) =
            Self::scan_row_groups(data, header.first_row_group_offset as usize)?;

        let summary = RecordingSummary {
            total_samples,
            chunk_count: row_group_offsets.len() as u32,
            total_bytes: data.len() as u64,
            footer_offset,
            duration: std::time::Duration::ZERO,
        };

        Ok(ParsedFile {
            metadata,
            schema,
            skip_entries,
            lap_entries,
            summary,
            row_group_offsets,
            total_samples,
        })
    }

    /// Get the underlying data slice.
    fn data(&self) -> &[u8] {
        if let Some(ref mmap) = self.mmap {
            mmap.as_slice()
        } else if let Some(ref bytes) = self.bytes {
            bytes.as_slice()
        } else {
            &[]
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    pub fn summary(&self) -> RecordingSummary {
        self.summary.clone()
    }

    pub fn lap_index(&self) -> &[LapIndexEntryV2] {
        &self.lap_entries
    }

    // -----------------------------------------------------------------------
    // Full read 锟?reconstruct all frames
    // -----------------------------------------------------------------------

    /// Read all frames by decoding every row group and reconstructing
    /// `TelemetryFrame` via row-index alignment.
    pub fn read_all_frames(&self) -> TelemetryResult<Vec<TelemetryFrame>> {
        // Read all groups
        let all_groups = [
            GroupId::FrameMeta,
            GroupId::DriverInputs,
            GroupId::VehicleDynamics,
            GroupId::Tyres,
            GroupId::Timing,
            GroupId::Environment,
            GroupId::ColdStorage,
        ];

        let group_data = self.read_group_frames(&all_groups, None, None)?;

        // Get tick and timestamp from FrameMeta
        let frame_meta = group_data
            .get(&GroupId::FrameMeta)
            .ok_or_else(|| TelemetryError::InvalidFormat("FrameMeta group missing".to_string()))?;

        if frame_meta.is_empty() {
            return Ok(Vec::new());
        }

        // frame_meta columns: [sample_tick: Vec<f64>, timestamp_ns: Vec<f64>,
        //   physics_packet_id: Vec<f64>, graphics_packet_id: Vec<f64>]
        // Where frame_meta[0] = sample_tick values, etc.
        let ticks = &frame_meta[0]; // ColumnId::SampleTick = 1 (first)
        let frame_count = ticks.len();

        let mut frames = Vec::with_capacity(frame_count);
        for (i, &tick_value) in ticks.iter().enumerate().take(frame_count) {
            let tick = tick_value as u64;
            let ts_ns = if frame_meta.len() > 1 {
                frame_meta[1].get(i).copied().unwrap_or(0.0)
            } else {
                0.0
            };

            let frame = self.reconstruct_frame(i, tick, ts_ns as u64, &group_data);
            frames.push(frame);
        }

        Ok(frames)
    }

    /// Reconstruct a single TelemetryFrame by row index from decoded group data.
    fn reconstruct_frame(
        &self,
        row_idx: usize,
        tick: u64,
        timestamp_ns: u64,
        group_data: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: tick,
            timestamp_ns,
            controls: self.build_controls(row_idx, tick, timestamp_ns, group_data),
            motion: self.build_motion(row_idx, tick, timestamp_ns, group_data),
            tyres: self.build_tyres(row_idx, tick, timestamp_ns, group_data),
            powertrain: self.build_powertrain(row_idx, tick, timestamp_ns, group_data),
            session: self.build_session(row_idx, tick, timestamp_ns, group_data),
            timing: self.build_timing(row_idx, tick, timestamp_ns, group_data),
            car_state: self.build_car_state(row_idx, tick, timestamp_ns, group_data),
            environment: self.build_environment(row_idx, tick, timestamp_ns, group_data),
            other_cars: self.build_other_cars(row_idx, tick, timestamp_ns, group_data),
        }
    }

    // =======================================================================
    // Substructure builders
    // =======================================================================

    /// Get column value at row_idx from group, column_id.
    /// Returns 0.0 if column not found or index out of bounds.
    fn col_val(
        &self,
        group_data: &HashMap<GroupId, Vec<Vec<f64>>>,
        group: GroupId,
        col_id: u16,
        row_idx: usize,
    ) -> f64 {
        if let Some(cols) = group_data.get(&group) {
            // Find column by scanning for the column_id in the schema
            let col_index = self.find_column_index_in_group(group, col_id);
            if let Some(ci) = col_index {
                if ci < cols.len() {
                    if let Some(v) = cols[ci].get(row_idx) {
                        return *v;
                    }
                }
            }
        }
        0.0
    }

    /// Get column values at row_idx for bytes-type array column.
    fn col_bytes(
        &self,
        group_data: &HashMap<GroupId, Vec<Vec<f64>>>,
        group: GroupId,
        col_id: u16,
        row_idx: usize,
    ) -> Vec<f64> {
        if let Some(cols) = group_data.get(&group) {
            let col_index = self.find_column_index_in_group(group, col_id);
            if let Some(ci) = col_index {
                if ci < cols.len() {
                    let col = &cols[ci];
                    // Compute total rows from FrameMeta sample_tick column
                    let total_rows = group_data
                        .get(&GroupId::FrameMeta)
                        .and_then(|fm| fm.first())
                        .map(|ticks| ticks.len())
                        .unwrap_or(0);
                    return self.extract_bytes_row(col, row_idx, total_rows);
                }
            }
        }
        Vec::new()
    }

    /// Find column index within a group by column_id.
    /// Column order follows schema definition order.
    fn find_column_index_in_group(&self, group: GroupId, col_id: u16) -> Option<usize> {
        for g in &self.schema.groups {
            if g.group_id == group as u16 {
                return g.columns.iter().position(|c| c.column_id == col_id);
            }
        }
        None
    }

    /// Extract sub-values for a TYPE_BYTES row from the flattened column.
    /// TYPE_BYTES columns have sub_count values per row stored flat.
    /// Given the total number of rows, computes sub_count and extracts
    /// the slice for the requested row.
    fn extract_bytes_row(&self, col: &[f64], row_idx: usize, total_rows: usize) -> Vec<f64> {
        if col.is_empty() || total_rows == 0 {
            return Vec::new();
        }
        let sub_count = col.len() / total_rows;
        if sub_count == 0 {
            return Vec::new();
        }
        let start = row_idx * sub_count;
        let end = (row_idx + 1) * sub_count;
        if end > col.len() {
            return Vec::new();
        }
        col[start..end].to_vec()
    }

    fn build_controls(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> ControlSample {
        let g = GroupId::DriverInputs;
        ControlSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            physics_packet_id: self.col_val(gd, GroupId::FrameMeta, 3, row_idx) as i32,
            graphics_packet_id: self.col_val(gd, GroupId::FrameMeta, 4, row_idx) as i32,
            speed_kmh: self.col_val(gd, g, 10, row_idx) as f32,
            gas: self.col_val(gd, g, 11, row_idx) as f32,
            brake: self.col_val(gd, g, 12, row_idx) as f32,
            clutch: self.col_val(gd, g, 13, row_idx) as f32,
            steer_angle: self.col_val(gd, g, 14, row_idx) as f32,
            gear: self.col_val(gd, g, 15, row_idx) as i32,
            rpms: self.col_val(gd, g, 16, row_idx) as i32,
            fuel: self.col_val(gd, g, 17, row_idx) as f32,
        }
    }

    fn build_motion(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> MotionSample {
        let g = GroupId::VehicleDynamics;
        let vel = self.col_bytes(gd, g, 20, row_idx);
        let acc = self.col_bytes(gd, g, 21, row_idx);
        let lv = self.col_bytes(gd, g, 22, row_idx);
        let lav = self.col_bytes(gd, g, 23, row_idx);
        MotionSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            velocity: arr3(&vel),
            acc_g: arr3(&acc),
            local_velocity: arr3(&lv),
            local_angular_vel: arr3(&lav),
            heading: self.col_val(gd, g, 24, row_idx) as f32,
            pitch: self.col_val(gd, g, 25, row_idx) as f32,
            roll: self.col_val(gd, g, 26, row_idx) as f32,
        }
    }

    fn build_tyres(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> TyreSample {
        let g = GroupId::Tyres;
        TyreSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            wheel_slip: arr4f(&self.col_bytes(gd, g, 30, row_idx)),
            wheel_load: arr4f(&self.col_bytes(gd, g, 31, row_idx)),
            wheels_pressure: arr4f(&self.col_bytes(gd, g, 32, row_idx)),
            wheel_angular_speed: arr4f(&self.col_bytes(gd, g, 33, row_idx)),
            tyre_wear: arr4f(&self.col_bytes(gd, g, 34, row_idx)),
            tyre_dirty_level: arr4f(&self.col_bytes(gd, g, 35, row_idx)),
            tyre_core_temperature: arr4f(&self.col_bytes(gd, g, 36, row_idx)),
            camber_rad: arr4f(&self.col_bytes(gd, g, 37, row_idx)),
            suspension_travel: arr4f(&self.col_bytes(gd, g, 38, row_idx)),
            slip_ratio: arr4f(&self.col_bytes(gd, g, 39, row_idx)),
            slip_angle: arr4f(&self.col_bytes(gd, g, 40, row_idx)),
            tyre_temp_i: arr4f(&self.col_bytes(gd, g, 41, row_idx)),
            tyre_temp_m: arr4f(&self.col_bytes(gd, g, 42, row_idx)),
            tyre_temp_o: arr4f(&self.col_bytes(gd, g, 43, row_idx)),
            tyre_temp: arr4f(&self.col_bytes(gd, g, 44, row_idx)),
            mz: arr4f(&self.col_bytes(gd, g, 45, row_idx)),
            fx: arr4f(&self.col_bytes(gd, g, 46, row_idx)),
            fy: arr4f(&self.col_bytes(gd, g, 47, row_idx)),
            suspension_damage: arr4f(&self.col_bytes(gd, g, 48, row_idx)),
            brake_temp: arr4f(&self.col_bytes(gd, g, 49, row_idx)),
            brake_pressure: arr4f(&self.col_bytes(gd, g, 50, row_idx)),
            pad_life: arr4f(&self.col_bytes(gd, g, 51, row_idx)),
            disc_life: arr4f(&self.col_bytes(gd, g, 52, row_idx)),
            tyre_contact_point: arr12f(&self.col_bytes(gd, g, 53, row_idx)),
            tyre_contact_normal: arr12f(&self.col_bytes(gd, g, 54, row_idx)),
            tyre_contact_heading: arr12f(&self.col_bytes(gd, g, 55, row_idx)),
            number_of_tyres_out: self.col_val(gd, g, 56, row_idx) as i32,
            front_brake_compound: self.col_val(gd, g, 57, row_idx) as i32,
            rear_brake_compound: self.col_val(gd, g, 58, row_idx) as i32,
        }
    }

    fn build_powertrain(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> PowertrainSample {
        let g = GroupId::DriverInputs;
        PowertrainSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            turbo_boost: self.col_val(gd, g, 60, row_idx) as f32,
            ballast: self.col_val(gd, g, 61, row_idx) as f32,
            kers_charge: self.col_val(gd, g, 62, row_idx) as f32,
            kers_input: self.col_val(gd, g, 63, row_idx) as f32,
            kers_current_kj: self.col_val(gd, g, 64, row_idx) as f32,
            drs: self.col_val(gd, g, 65, row_idx) as f32,
            tc: self.col_val(gd, g, 66, row_idx) as f32,
            abs: self.col_val(gd, g, 67, row_idx) as f32,
            engine_brake: self.col_val(gd, g, 68, row_idx) as i32,
            ers_recovery_level: self.col_val(gd, g, 69, row_idx) as i32,
            ers_power_level: self.col_val(gd, g, 70, row_idx) as i32,
            ers_heat_charging: self.col_val(gd, g, 71, row_idx) as i32,
            ers_is_charging: self.col_val(gd, g, 72, row_idx) as i32,
            drs_available: self.col_val(gd, g, 73, row_idx) as i32,
            drs_enabled: self.col_val(gd, g, 74, row_idx) as i32,
            tc_in_action: self.col_val(gd, g, 75, row_idx) as i32,
            abs_in_action: self.col_val(gd, g, 76, row_idx) as i32,
            auto_shifter_on: self.col_val(gd, g, 77, row_idx) as i32,
            current_max_rpm: self.col_val(gd, g, 78, row_idx) as i32,
            p2p_activations: self.col_val(gd, g, 79, row_idx) as i32,
            p2p_status: self.col_val(gd, g, 80, row_idx) as i32,
            water_temp: self.col_val(gd, g, 81, row_idx) as f32,
        }
    }

    fn build_session(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> SessionSample {
        // Session fields are spread across Timing, Environment, and ColdStorage groups
        // Timing: 93-94, 96-99, 108 (completed_laps, position, number_of_laps, current_sector_index,
        //   normalized_car_position, is_in_pit, is_in_pit_lane, is_valid_lap)
        // Environment: 90-91, 95, 106-107 (status, session, session_time_left, clock, replay_time_multiplier)
        // ColdStorage: 92, 100-105, 109-119 (session_index, is_in_pit_lane, mandatory_pit_done,
        //   missing_mandatory_pits, penalty_time, penalty_type, track_status, global_yellows,
        //   global_white, global_green, global_chequered, global_red, gap_ahead_or_tail_value,
        //   flag, gap_behind)
        let t = GroupId::Timing;
        let e = GroupId::Environment;
        let c = GroupId::ColdStorage;
        SessionSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            status: self.col_val(gd, e, 90, row_idx) as i32,
            session: self.col_val(gd, e, 91, row_idx) as i32,
            session_index: self.col_val(gd, c, 92, row_idx) as i32,
            completed_laps: self.col_val(gd, t, 93, row_idx) as i32,
            position: self.col_val(gd, t, 94, row_idx) as i32,
            session_time_left: self.col_val(gd, e, 95, row_idx) as f32,
            number_of_laps: self.col_val(gd, t, 96, row_idx) as i32,
            current_sector_index: self.col_val(gd, t, 97, row_idx) as i32,
            normalized_car_position: self.col_val(gd, t, 98, row_idx) as f32,
            is_in_pit: self.col_val(gd, t, 99, row_idx) as i32,
            is_in_pit_lane: self.col_val(gd, c, 100, row_idx) as i32,
            mandatory_pit_done: self.col_val(gd, c, 101, row_idx) as i32,
            missing_mandatory_pits: self.col_val(gd, c, 102, row_idx) as i32,
            penalty_time: self.col_val(gd, c, 103, row_idx) as f32,
            penalty_type: self.col_val(gd, c, 104, row_idx) as i32,
            track_status: {
                let raw = self.col_bytes(gd, c, 105, row_idx);
                let mut arr = [0u16; 33];
                for (i, v) in arr.iter_mut().enumerate() {
                    *v = raw.get(i).copied().unwrap_or(0.0) as u16;
                }
                arr
            },
            clock: self.col_val(gd, e, 106, row_idx) as f32,
            replay_time_multiplier: self.col_val(gd, e, 107, row_idx) as f32,
            is_valid_lap: self.col_val(gd, t, 108, row_idx) as i32,
            global_yellow: self.col_val(gd, c, 109, row_idx) as i32,
            global_yellow1: self.col_val(gd, c, 110, row_idx) as i32,
            global_yellow2: self.col_val(gd, c, 111, row_idx) as i32,
            global_yellow3: self.col_val(gd, c, 112, row_idx) as i32,
            global_white: self.col_val(gd, c, 113, row_idx) as i32,
            global_green: self.col_val(gd, c, 114, row_idx) as i32,
            global_chequered: self.col_val(gd, c, 115, row_idx) as i32,
            global_red: self.col_val(gd, c, 116, row_idx) as i32,
            gap_ahead_or_tail_value: self.col_val(gd, c, 117, row_idx) as i32,
            flag: self.col_val(gd, c, 118, row_idx) as i32,
            gap_behind: self.col_val(gd, c, 119, row_idx) as i32,
        }
    }

    fn build_timing(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> TimingSample {
        let g = GroupId::Timing;
        TimingSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            i_current_time: self.col_val(gd, g, 120, row_idx) as i32,
            i_last_time: self.col_val(gd, g, 121, row_idx) as i32,
            i_best_time: self.col_val(gd, g, 122, row_idx) as i32,
            i_split: self.col_val(gd, g, 123, row_idx) as i32,
            last_sector_time: self.col_val(gd, g, 124, row_idx) as i32,
            i_delta_lap_time: self.col_val(gd, g, 125, row_idx) as i32,
            is_delta_positive: self.col_val(gd, g, 126, row_idx) as i32,
            i_estimated_lap_time: self.col_val(gd, g, 127, row_idx) as i32,
            fuel_estimated_laps: self.col_val(gd, g, 128, row_idx) as f32,
            fuel_x_lap: self.col_val(gd, g, 129, row_idx) as f32,
            used_fuel: self.col_val(gd, g, 130, row_idx) as f32,
            distance_traveled: self.col_val(gd, g, 131, row_idx) as f32,
            current_time_str: {
                let raw = self.col_bytes(gd, g, 132, row_idx);
                let mut arr = [0u16; 15];
                for (i, v) in arr.iter_mut().enumerate() {
                    *v = raw.get(i).copied().unwrap_or(0.0) as u16;
                }
                arr
            },
            last_time_str: [0u16; 15],
            best_time_str: [0u16; 15],
            split_str: [0u16; 15],
            delta_lap_time_str: [0u16; 15],
            estimated_lap_time_str: [0u16; 15],
            observed_slot_before_i_split: self.col_val(gd, g, 138, row_idx) as i32,
        }
    }

    fn build_car_state(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> CarStateSample {
        let g = GroupId::ColdStorage;
        CarStateSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            car_damage: arr5f(&self.col_bytes(gd, g, 150, row_idx)),
            pit_limiter_on: self.col_val(gd, g, 151, row_idx) as i32,
            ride_height: arr2f(&self.col_bytes(gd, g, 152, row_idx)),
            ignition_on: self.col_val(gd, g, 153, row_idx) as i32,
            starter_engine_on: self.col_val(gd, g, 154, row_idx) as i32,
            is_engine_running: self.col_val(gd, g, 155, row_idx) as i32,
            is_ai_controlled: self.col_val(gd, g, 156, row_idx) as i32,
            cg_height: self.col_val(gd, g, 157, row_idx) as f32,
            brake_bias: self.col_val(gd, g, 158, row_idx) as f32,
            rain_lights: self.col_val(gd, g, 159, row_idx) as i32,
            flashing_lights: self.col_val(gd, g, 160, row_idx) as i32,
            lights_stage: self.col_val(gd, g, 161, row_idx) as i32,
            wiper_lv: self.col_val(gd, g, 162, row_idx) as i32,
            driver_stint_total_time_left: self.col_val(gd, g, 163, row_idx) as i32,
            driver_stint_time_left: self.col_val(gd, g, 164, row_idx) as i32,
            rain_tyres: self.col_val(gd, g, 165, row_idx) as i32,
            current_tyre_set: self.col_val(gd, g, 166, row_idx) as i32,
            strategy_tyre_set: self.col_val(gd, g, 167, row_idx) as i32,
            track_grip_status: self.col_val(gd, g, 168, row_idx) as i32,
            tyre_compound_str: {
                let raw = self.col_bytes(gd, g, 169, row_idx);
                let mut arr = [0u16; 33];
                for (i, v) in arr.iter_mut().enumerate() {
                    *v = raw.get(i).copied().unwrap_or(0.0) as u16;
                }
                arr
            },
            mfd_tyre_set: self.col_val(gd, g, 170, row_idx) as i32,
            mfd_fuel_to_add: self.col_val(gd, g, 171, row_idx) as f32,
            mfd_tyre_pressure: arr4f(&self.col_bytes(gd, g, 172, row_idx)),
            ideal_line_on: self.col_val(gd, g, 173, row_idx) as i32,
            is_setup_menu_visible: self.col_val(gd, g, 174, row_idx) as i32,
            main_display_index: self.col_val(gd, g, 175, row_idx) as i32,
            secondary_display_index: self.col_val(gd, g, 176, row_idx) as i32,
            direction_lights_left: self.col_val(gd, g, 177, row_idx) as i32,
            direction_lights_right: self.col_val(gd, g, 178, row_idx) as i32,
            tc_level: self.col_val(gd, g, 179, row_idx) as i32,
            tc_cut: self.col_val(gd, g, 180, row_idx) as i32,
            engine_map: self.col_val(gd, g, 181, row_idx) as i32,
            abs_level: self.col_val(gd, g, 182, row_idx) as i32,
            exhaust_temperature: self.col_val(gd, g, 183, row_idx) as f32,
            final_ff: self.col_val(gd, g, 184, row_idx) as f32,
            performance_meter: self.col_val(gd, g, 185, row_idx) as f32,
            kerb_vibration: self.col_val(gd, g, 186, row_idx) as f32,
            slip_vibrations: self.col_val(gd, g, 187, row_idx) as f32,
            g_vibrations: self.col_val(gd, g, 188, row_idx) as f32,
            abs_vibrations: self.col_val(gd, g, 189, row_idx) as f32,
        }
    }

    fn build_environment(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> EnvironmentSample {
        let g = GroupId::Environment;
        EnvironmentSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            air_density: self.col_val(gd, g, 200, row_idx) as f32,
            air_temp: self.col_val(gd, g, 201, row_idx) as f32,
            road_temp: self.col_val(gd, g, 202, row_idx) as f32,
            wind_speed: self.col_val(gd, g, 203, row_idx) as f32,
            wind_direction: self.col_val(gd, g, 204, row_idx) as f32,
            surface_grip: self.col_val(gd, g, 205, row_idx) as f32,
            rain_intensity: self.col_val(gd, g, 206, row_idx) as i32,
            rain_intensity_in_10min: self.col_val(gd, g, 207, row_idx) as i32,
            rain_intensity_in_30min: self.col_val(gd, g, 208, row_idx) as i32,
        }
    }

    fn build_other_cars(
        &self,
        row_idx: usize,
        tick: u64,
        ts_ns: u64,
        gd: &HashMap<GroupId, Vec<Vec<f64>>>,
    ) -> OtherCarsSample {
        let g = GroupId::ColdStorage;
        OtherCarsSample {
            sample_tick: tick,
            timestamp_ns: ts_ns,
            active_cars: self.col_val(gd, g, 210, row_idx) as i32,
            player_car_id: self.col_val(gd, g, 211, row_idx) as i32,
            car_coordinates: {
                let raw = self.col_bytes(gd, g, 212, row_idx);
                raw.iter().map(|&v| v as f32).collect()
            },
            car_id: {
                let raw = self.col_bytes(gd, g, 213, row_idx);
                raw.iter().map(|&v| v as i32).collect()
            },
        }
    }

    // =======================================================================
    // Lap-bounded read
    // =======================================================================

    /// Read frames belonging to a specific lap (1-based).
    pub fn read_lap_frames(&self, lap_number: i32) -> TelemetryResult<Vec<TelemetryFrame>> {
        // Find the lap entry
        let lap_entry = self
            .lap_entries
            .iter()
            .find(|e| e.lap_number == lap_number)
            .ok_or_else(|| {
                TelemetryError::InvalidArgument(format!("lap {lap_number} not found in lap index"))
            })?;

        // Read all groups for the lap tick range
        let all_groups: [GroupId; 7] = [
            GroupId::FrameMeta,
            GroupId::DriverInputs,
            GroupId::VehicleDynamics,
            GroupId::Tyres,
            GroupId::Timing,
            GroupId::Environment,
            GroupId::ColdStorage,
        ];

        // Read frames in the lap tick range
        let start_frame = Some(lap_entry.start_tick);
        let end_frame = Some(lap_entry.end_tick);

        let group_data = self.read_group_frames(&all_groups, start_frame, end_frame)?;

        let frame_meta = group_data
            .get(&GroupId::FrameMeta)
            .ok_or_else(|| TelemetryError::InvalidFormat("FrameMeta group missing".to_string()))?;

        if frame_meta.is_empty() {
            return Ok(Vec::new());
        }

        let ticks = &frame_meta[0];
        let frame_count = ticks.len();

        let mut frames = Vec::with_capacity(frame_count);
        for (i, &tick_value) in ticks.iter().enumerate().take(frame_count) {
            let tick = tick_value as u64;
            let ts_ns = if frame_meta.len() > 1 {
                frame_meta[1].get(i).copied().unwrap_or(0.0)
            } else {
                0.0
            };

            let frame = self.reconstruct_frame(i, tick, ts_ns as u64, &group_data);
            frames.push(frame);
        }

        Ok(frames)
    }

    // =======================================================================
    // Typed access — read_all_*_v2 methods
    // =======================================================================
    //
    // Each method reads the relevant access groups via read_group_frames(),
    // then reconstructs the substructure for every frame row-by-row using
    // the same col_val / col_bytes helpers as rebuild_frame.

    // ── Helper: reconstruct samples from group data ──

    fn reconstruct_from_groups<T>(
        &self,
        groups: &[GroupId],
        start_tick: Option<u64>,
        end_tick: Option<u64>,
        build: impl Fn(&Self, usize, u64, u64, &HashMap<GroupId, Vec<Vec<f64>>>) -> T,
    ) -> TelemetryResult<Vec<T>> {
        let gd = self.read_group_frames(groups, start_tick, end_tick)?;
        let fm = gd
            .get(&GroupId::FrameMeta)
            .filter(|cols| !cols.is_empty())
            .ok_or_else(|| TelemetryError::InvalidFormat("FrameMeta group missing".to_string()))?;
        let ticks = &fm[0];
        let n = ticks.len();
        let mut out = Vec::with_capacity(n);
        for (i, &tick_value) in ticks.iter().enumerate().take(n) {
            let tick = tick_value as u64;
            let ts_ns = fm.get(1).and_then(|c| c.get(i).copied()).unwrap_or(0.0) as u64;
            out.push(build(self, i, tick, ts_ns, &gd));
        }
        Ok(out)
    }

    // ── read_all_*_v2 — full reads (delegate to helper) ──

    /// Read all ControlSample frames (FrameMeta + DriverInputs groups).
    pub fn read_all_controls_v2(&self) -> TelemetryResult<Vec<ControlSample>> {
        self.read_controls_range_v2(0, u64::MAX)
    }

    /// Read all MotionSample frames (FrameMeta + VehicleDynamics groups).
    pub fn read_all_motion_v2(&self) -> TelemetryResult<Vec<MotionSample>> {
        self.read_motion_range_v2(0, u64::MAX)
    }

    /// Read all TyreSample frames (FrameMeta + Tyres groups).
    pub fn read_all_tyres_v2(&self) -> TelemetryResult<Vec<TyreSample>> {
        self.read_tyres_range_v2(0, u64::MAX)
    }

    /// Read all PowertrainSample frames (FrameMeta + DriverInputs groups).
    pub fn read_all_powertrain_v2(&self) -> TelemetryResult<Vec<PowertrainSample>> {
        self.read_powertrain_range_v2(0, u64::MAX)
    }

    /// Read all SessionSample frames (FrameMeta + Timing + Environment + ColdStorage groups).
    pub fn read_all_session_v2(&self) -> TelemetryResult<Vec<SessionSample>> {
        self.read_session_range_v2(0, u64::MAX)
    }

    /// Lightweight V2 lap boundary read: only `(tick, norm_pos, is_valid)`.
    ///
    /// Falls back to full session read since V2 files are already excluded
    /// from `append_lap_index` (which returns early for ACT2 magic).
    pub fn read_lap_boundary_data_v2(&self) -> TelemetryResult<Vec<(u64, f32, i32)>> {
        let samples = self.read_all_session_v2()?;
        Ok(samples
            .into_iter()
            .map(|s| (s.sample_tick, s.normalized_car_position, s.is_valid_lap))
            .collect())
    }

    /// Read all TimingSample frames (FrameMeta + Timing group).
    pub fn read_all_timing_v2(&self) -> TelemetryResult<Vec<TimingSample>> {
        self.read_timing_range_v2(0, u64::MAX)
    }

    /// Read all CarStateSample frames (FrameMeta + ColdStorage groups).
    pub fn read_all_car_state_v2(&self) -> TelemetryResult<Vec<CarStateSample>> {
        self.read_car_state_range_v2(0, u64::MAX)
    }

    /// Read all EnvironmentSample frames (FrameMeta + Environment groups).
    pub fn read_all_environment_v2(&self) -> TelemetryResult<Vec<EnvironmentSample>> {
        self.read_environment_range_v2(0, u64::MAX)
    }

    /// Read all OtherCarsSample frames (FrameMeta + ColdStorage groups).
    pub fn read_all_other_cars_v2(&self) -> TelemetryResult<Vec<OtherCarsSample>> {
        self.read_other_cars_range_v2(0, u64::MAX)
    }

    // ── read_*_range_v2 — tick-range reads ──

    pub fn read_controls_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<ControlSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::DriverInputs],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_controls(i, tick, ts_ns, gd),
        )
    }

    pub fn read_motion_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<MotionSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::VehicleDynamics],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_motion(i, tick, ts_ns, gd),
        )
    }

    pub fn read_tyres_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<TyreSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::Tyres],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_tyres(i, tick, ts_ns, gd),
        )
    }

    pub fn read_powertrain_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<PowertrainSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::DriverInputs],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_powertrain(i, tick, ts_ns, gd),
        )
    }

    pub fn read_session_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<SessionSample>> {
        self.reconstruct_from_groups(
            &[
                GroupId::FrameMeta,
                GroupId::Timing,
                GroupId::Environment,
                GroupId::ColdStorage,
            ],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_session(i, tick, ts_ns, gd),
        )
    }

    pub fn read_timing_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<TimingSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::Timing],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_timing(i, tick, ts_ns, gd),
        )
    }

    pub fn read_car_state_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<CarStateSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::ColdStorage],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_car_state(i, tick, ts_ns, gd),
        )
    }

    pub fn read_environment_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<EnvironmentSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::Environment],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_environment(i, tick, ts_ns, gd),
        )
    }

    pub fn read_other_cars_range_v2(
        &self,
        start_tick: u64,
        end_tick: u64,
    ) -> TelemetryResult<Vec<OtherCarsSample>> {
        self.reconstruct_from_groups(
            &[GroupId::FrameMeta, GroupId::ColdStorage],
            Some(start_tick),
            Some(end_tick),
            |slf, i, tick, ts_ns, gd| slf.build_other_cars(i, tick, ts_ns, gd),
        )
    }

    // =======================================================================
    // ItemKey-based column reader
    // =======================================================================

    /// Read values for a single raw column identified by ItemKey, for a
    /// frame range.
    ///
    /// Parses keys like `raw:controls.speed_kmh` — resolves the path to a
    /// `GroupId` + `ColumnId`, reads the relevant group via
    /// `read_group_frames`, extracts the column, applies the frame range
    /// filter, and returns a flat `Vec<f64>` (one value per remaining frame).
    ///
    /// For `calc:*` items this returns an error explaining that calculated
    /// items are computed at read time and not stored in acctlm2.
    pub fn read_item_frames(
        &self,
        key: &crate::item_key::ItemKey,
        start_frame: u64,
        end_frame: u64,
    ) -> TelemetryResult<Vec<f64>> {
        use crate::item_key::ItemType;

        match key.item_type {
            ItemType::Raw => {
                let col_id = path_to_column_id(&key.name).ok_or_else(|| {
                    TelemetryError::InvalidArgument(format!(
                        "unknown raw column path: {}",
                        key.name
                    ))
                })?;

                let target_group = crate::format_v2::column_group(col_id);

                // Must include FrameMeta for frame-range filtering to work.
                // read_group_frames uses FrameMeta ticks as the filter source.
                let groups = [GroupId::FrameMeta, target_group];

                let s = Some(start_frame);
                let e = Some(end_frame);
                let gd = self.read_group_frames(&groups, s, e)?;

                let cols = gd.get(&target_group).ok_or_else(|| {
                    TelemetryError::InvalidFormat(format!("no data for group {:?}", target_group))
                })?;

                let col_index = self
                    .find_column_index_in_group(target_group, col_id as u16)
                    .ok_or_else(|| {
                        TelemetryError::InvalidFormat(format!(
                            "column {:?} not found in schema",
                            col_id
                        ))
                    })?;

                if col_index >= cols.len() {
                    return Ok(Vec::new());
                }

                let raw = &cols[col_index];

                // For TYPE_BYTES columns: each frame contributes sub_count
                // values. Compute sub_count from total values / frame count.
                // Flatten all sub-values into the output
                let mut out = Vec::with_capacity(raw.len());
                out.extend_from_slice(raw);
                Ok(out)
            }
            ItemType::Calculated => Err(TelemetryError::InvalidArgument(
                "calc:* items are computed at read time by ComputeRegistry, not stored in acctlm2"
                    .to_string(),
            )),
            ItemType::System => Err(TelemetryError::InvalidArgument(
                "system:* items are not stored in acctlm2".to_string(),
            )),
        }
    }

    // =======================================================================
    // Selective group read with skip index
    // =======================================================================

    /// Read specific access groups for a frame range.
    ///
    /// Returns a `HashMap<GroupId, Vec<Vec<f64>>>` where each `Vec<Vec<f64>>`
    /// contains columns (inner Vec = one column's values for all rows).
    ///
    /// When `start_frame` and `end_frame` are both `None`, all frames are read.
    pub fn read_group_frames(
        &self,
        groups: &[GroupId],
        start_frame: Option<u64>,
        end_frame: Option<u64>,
    ) -> TelemetryResult<HashMap<GroupId, Vec<Vec<f64>>>> {
        let data = self.data();
        let mut result: HashMap<GroupId, Vec<Vec<f64>>> = HashMap::new();

        // For each requested group, find relevant row groups via skip index
        // and decode columns.
        for &group in groups {
            let _group_columns: Vec<Vec<f64>> = Vec::new();

            // Find all row groups that contain columns for this group
            // and that overlap with [start_frame, end_frame].
            let relevant_entries: Vec<&SkipIndexEntry> = self
                .skip_entries
                .iter()
                .filter(|e| e.access_group == group as u16)
                .filter(|entry| match (start_frame, end_frame) {
                    (None, None) => true,
                    (Some(s), None) => entry.frame_end >= s,
                    (None, Some(ef)) => entry.frame_start <= ef,
                    (Some(s), Some(ef)) => entry.frame_start <= ef && entry.frame_end >= s,
                })
                .collect();

            if relevant_entries.is_empty() {
                // Try reading all row groups for this group
                if self.row_group_offsets.is_empty() {
                    continue;
                }
                // Read all row groups
                let col_data =
                    self.read_group_from_all_row_groups(data, group, start_frame, end_frame)?;
                result.insert(group, col_data);
                continue;
            }

            // Collect unique row group indices
            let mut rg_indices: Vec<u32> =
                relevant_entries.iter().map(|e| e.row_group_index).collect();
            rg_indices.sort();
            rg_indices.dedup();

            // Read each row group and extract columns
            let mut per_rg_columns: Vec<Vec<Vec<f64>>> = Vec::new();

            for rg_idx in &rg_indices {
                if (*rg_idx as usize) >= self.row_group_offsets.len() {
                    continue;
                }
                let offset = self.row_group_offsets[*rg_idx as usize];

                // Parse the row group header
                let mut cursor = Cursor::new(&data[offset as usize..]);
                let rg_header = RowGroupHeader::read_from(&mut cursor)?;

                // Find the group entry for this group
                let group_entry = rg_header
                    .groups
                    .iter()
                    .find(|ge| ge.group_id == group as u16);

                if let Some(ge) = group_entry {
                    let group_data_start = offset as usize + ge.offset as usize;
                    let group_data_end = group_data_start + ge.byte_len as usize;

                    if group_data_end > data.len() {
                        return Err(TelemetryError::InvalidFormat(
                            "group data past eof".to_string(),
                        ));
                    }

                    let group_bytes = &data[group_data_start..group_data_end];
                    let mut group_cursor = Cursor::new(group_bytes);

                    // Read: [group_id:u16][col_count:u16][ColumnEntryV2 脳 N][data...]
                    let _gid = read_u16(&mut group_cursor)?;
                    let col_count = read_u16(&mut group_cursor)? as usize;

                    let mut col_entries = Vec::with_capacity(col_count);
                    for _ in 0..col_count {
                        col_entries.push(ColumnEntryV2::read_from(&mut group_cursor)?);
                    }

                    // Data starts after all column entries
                    let data_offset = 4 + col_count * 40; // 40 = ColumnEntryV2 size
                    let mut col_values: Vec<Vec<f64>> = Vec::with_capacity(col_count);

                    for entry in &col_entries {
                        // Use the actual offset
                        let mut col_data_offset = data_offset;
                        for e in &col_entries {
                            if e.column_id == entry.column_id {
                                break;
                            }
                            col_data_offset = col_data_offset
                                .checked_add(e.byte_len as usize)
                                .ok_or_else(|| {
                                    TelemetryError::InvalidFormat(
                                        "column data offset overflow".to_string(),
                                    )
                                })?;
                        }
                        let col_end = col_data_offset
                            .checked_add(entry.byte_len as usize)
                            .ok_or_else(|| {
                                TelemetryError::InvalidFormat(
                                    "column byte length overflow".to_string(),
                                )
                            })?;
                        if col_end > group_bytes.len() {
                            return Err(TelemetryError::InvalidFormat(
                                "column data extends past group end".to_string(),
                            ));
                        }
                        let col_bytes = &group_bytes[col_data_offset..col_end];

                        let decoded = decode_column(col_bytes, entry.crc32)?;
                        col_values.push(decoded);
                    }

                    per_rg_columns.push(col_values);
                }
            }

            // Merge columns across row groups (concatenate, row by row)
            if per_rg_columns.is_empty() {
                result.insert(group, Vec::new());
                continue;
            }

            // We need to align columns across row groups.
            // Each row group has the same columns in the same order.
            let num_cols = per_rg_columns[0].len();
            let mut merged: Vec<Vec<f64>> = vec![Vec::new(); num_cols];

            for rg_cols in &per_rg_columns {
                for (ci, col) in rg_cols.iter().enumerate() {
                    merged[ci].extend(col);
                }
            }

            // Store merged data (defer frame range filtering to after all groups)
            result.insert(group, merged);
        }

        // Apply frame range filtering to all groups using FrameMeta ticks
        if start_frame.is_some() || end_frame.is_some() {
            if let Some(meta_cols) = result.get(&GroupId::FrameMeta) {
                if !meta_cols.is_empty() {
                    let ticks = &meta_cols[0];
                    let row_count = ticks.len();
                    // Build keep mask
                    let keep: Vec<bool> = ticks
                        .iter()
                        .map(|&t| {
                            let tu = t as u64;
                            match (start_frame, end_frame) {
                                (None, None) => true,
                                (Some(s), None) => tu >= s,
                                (None, Some(e)) => tu <= e,
                                (Some(s), Some(e)) => tu >= s && tu <= e,
                            }
                        })
                        .collect();
                    let keep_count = keep.iter().filter(|&&k| k).count();
                    // Apply to every group
                    for (_gid, cols) in result.iter_mut() {
                        for col in cols.iter_mut() {
                            let col_len = col.len();
                            let sub_count = col_len.checked_div(row_count).unwrap_or(1);
                            let mut new_col = Vec::with_capacity(keep_count * sub_count);
                            for (row_idx, &should_keep) in keep.iter().enumerate() {
                                if should_keep {
                                    let start = row_idx * sub_count;
                                    let end = (start + sub_count).min(col_len);
                                    new_col.extend_from_slice(&col[start..end]);
                                }
                            }
                            *col = new_col;
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    /// Read a group from all row groups (fallback when skip index is empty).
    fn read_group_from_all_row_groups(
        &self,
        data: &[u8],
        group: GroupId,
        start_frame: Option<u64>,
        end_frame: Option<u64>,
    ) -> TelemetryResult<Vec<Vec<f64>>> {
        let mut all_cols: Vec<Vec<f64>> = Vec::new();

        for offset in &self.row_group_offsets {
            let mut cursor = Cursor::new(&data[*offset as usize..]);
            let rg_header = RowGroupHeader::read_from(&mut cursor)?;

            // Check if this row group overlaps with the requested frame range
            if let (Some(s), Some(e)) = (start_frame, end_frame) {
                if rg_header.frame_end_tick < s || rg_header.frame_start_tick > e {
                    continue;
                }
            }

            let group_entry = rg_header
                .groups
                .iter()
                .find(|ge| ge.group_id == group as u16);

            if let Some(ge) = group_entry {
                let group_data_start = *offset as usize + ge.offset as usize;
                let group_data_end = group_data_start + ge.byte_len as usize;

                if group_data_end > data.len() {
                    continue;
                }

                let group_bytes = &data[group_data_start..group_data_end];
                let mut group_cursor = Cursor::new(group_bytes);

                let _gid = read_u16(&mut group_cursor)?;
                let col_count = read_u16(&mut group_cursor)? as usize;

                let mut col_entries = Vec::with_capacity(col_count);
                for _ in 0..col_count {
                    col_entries.push(ColumnEntryV2::read_from(&mut group_cursor)?);
                }

                if all_cols.is_empty() {
                    all_cols.resize(col_count, Vec::new());
                }

                let mut data_offset = 4 + col_count * 40;

                for entry in &col_entries {
                    let col_end = data_offset
                        .checked_add(entry.byte_len as usize)
                        .ok_or_else(|| {
                            TelemetryError::InvalidFormat("column byte length overflow".to_string())
                        })?;
                    if col_end > group_bytes.len() {
                        return Err(TelemetryError::InvalidFormat(
                            "column data extends past group end".to_string(),
                        ));
                    }
                    let col_bytes = &group_bytes[data_offset..col_end];
                    let decoded = decode_column(col_bytes, entry.crc32)?;
                    let col_idx = col_entries
                        .iter()
                        .position(|e| e.column_id == entry.column_id)
                        .unwrap_or(0);
                    if col_idx < all_cols.len() {
                        all_cols[col_idx].extend(decoded);
                    }
                    data_offset = col_end;
                }
            }
        }

        Ok(all_cols)
    }

    // =======================================================================
    // Row group scanning
    // =======================================================================

    /// Scan all row group headers to collect file offsets and total sample count.
    fn scan_row_groups(data: &[u8], first_offset: usize) -> TelemetryResult<(Vec<u64>, u64)> {
        let mut offsets = Vec::new();
        let mut total_samples = 0u64;
        let mut pos = first_offset;

        while pos < data.len() {
            if pos + 4 > data.len() {
                break;
            }
            let magic = &data[pos..pos + 4];
            if magic != *b"RGHD" {
                break;
            }

            offsets.push(pos as u64);

            let mut cursor = Cursor::new(&data[pos..]);
            let rg_header = RowGroupHeader::read_from(&mut cursor)?;

            total_samples += rg_header.row_count as u64;

            // Compute total size of this row group.
            // `ge.offset` is absolute from the start of the row group (already includes header size).
            let mut rg_end = pos;
            for ge in &rg_header.groups {
                let group_end = pos
                    .checked_add(ge.offset as usize)
                    .and_then(|v| v.checked_add(ge.byte_len as usize))
                    .ok_or_else(|| {
                        TelemetryError::InvalidFormat("row group offset overflow".to_string())
                    })?;
                if group_end > data.len() {
                    return Err(TelemetryError::InvalidFormat(
                        "row group extends past file end".to_string(),
                    ));
                }
                if group_end > rg_end {
                    rg_end = group_end;
                }
            }

            if rg_end <= pos {
                return Err(TelemetryError::InvalidFormat(
                    "row group has zero or negative size".to_string(),
                ));
            }

            pos = rg_end;
        }

        Ok((offsets, total_samples))
    }
}

// ---------------------------------------------------------------------------
// Array helpers
// ---------------------------------------------------------------------------

fn arr3(v: &[f64]) -> [f32; 3] {
    [
        v.first().copied().unwrap_or(0.0) as f32,
        v.get(1).copied().unwrap_or(0.0) as f32,
        v.get(2).copied().unwrap_or(0.0) as f32,
    ]
}

fn arr4f(v: &[f64]) -> [f32; 4] {
    [
        v.first().copied().unwrap_or(0.0) as f32,
        v.get(1).copied().unwrap_or(0.0) as f32,
        v.get(2).copied().unwrap_or(0.0) as f32,
        v.get(3).copied().unwrap_or(0.0) as f32,
    ]
}

fn arr12f(v: &[f64]) -> [f32; 12] {
    let mut arr = [0.0f32; 12];
    for (i, val) in v.iter().take(12).enumerate() {
        arr[i] = *val as f32;
    }
    arr
}

fn arr5f(v: &[f64]) -> [f32; 5] {
    let mut arr = [0.0f32; 5];
    for (i, val) in v.iter().take(5).enumerate() {
        arr[i] = *val as f32;
    }
    arr
}

fn arr2f(v: &[f64]) -> [f32; 2] {
    [
        v.first().copied().unwrap_or(0.0) as f32,
        v.get(1).copied().unwrap_or(0.0) as f32,
    ]
}

// ---------------------------------------------------------------------------
// path → ColumnId resolver
// ---------------------------------------------------------------------------

/// Resolve a v1-field-path string (e.g. `"controls.speed_kmh"`) to its
/// `ColumnId` in the v2 column catalog.
///
/// Returns `None` when the path is not recognised.
fn path_to_column_id(path: &str) -> Option<crate::format_v2::ColumnId> {
    use crate::format_v2::ColumnId;

    // Strip optional struct prefix; both "controls.speed_kmh" and
    // "speed_kmh" should work when unambiguous.
    let leaf = path.rsplit('.').next().unwrap_or(path);

    match leaf {
        // -- controls (DriverInputs group) --
        "speed_kmh" => Some(ColumnId::SpeedKmh),
        "gas" => Some(ColumnId::Gas),
        "brake" => Some(ColumnId::Brake),
        "clutch" => Some(ColumnId::Clutch),
        "steer_angle" => Some(ColumnId::SteerAngle),
        "gear" => Some(ColumnId::Gear),
        "rpms" => Some(ColumnId::Rpms),
        "fuel" => Some(ColumnId::Fuel),

        // -- motion (VehicleDynamics group) --
        "velocity" => Some(ColumnId::Velocity),
        "acc_g" => Some(ColumnId::AccG),
        "local_velocity" => Some(ColumnId::LocalVelocity),
        "local_angular_vel" => Some(ColumnId::LocalAngularVel),
        "heading" => Some(ColumnId::Heading),
        "pitch" => Some(ColumnId::Pitch),
        "roll" => Some(ColumnId::Roll),

        // -- tyres (Tyres group) --
        "wheel_slip" => Some(ColumnId::WheelSlip),
        "wheel_load" => Some(ColumnId::WheelLoad),
        "wheels_pressure" => Some(ColumnId::WheelsPressure),
        "wheel_angular_speed" => Some(ColumnId::WheelAngularSpeed),
        "tyre_wear" => Some(ColumnId::TyreWear),
        "tyre_dirty_level" => Some(ColumnId::TyreDirtyLevel),
        "tyre_core_temperature" => Some(ColumnId::TyreCoreTemperature),
        "camber_rad" => Some(ColumnId::CamberRad),
        "suspension_travel" => Some(ColumnId::SuspensionTravel),
        "slip_ratio" => Some(ColumnId::SlipRatio),
        "slip_angle" => Some(ColumnId::SlipAngle),
        "tyre_temp_i" => Some(ColumnId::TyreTempI),
        "tyre_temp_m" => Some(ColumnId::TyreTempM),
        "tyre_temp_o" => Some(ColumnId::TyreTempO),
        "tyre_temp" => Some(ColumnId::TyreTemp),
        "mz" => Some(ColumnId::Mz),
        "fx" => Some(ColumnId::Fx),
        "fy" => Some(ColumnId::Fy),
        "suspension_damage" => Some(ColumnId::SuspensionDamage),
        "brake_temp" => Some(ColumnId::BrakeTemp),
        "brake_pressure" => Some(ColumnId::BrakePressure),
        "pad_life" => Some(ColumnId::PadLife),
        "disc_life" => Some(ColumnId::DiscLife),
        "tyre_contact_point" => Some(ColumnId::TyreContactPoint),
        "tyre_contact_normal" => Some(ColumnId::TyreContactNormal),
        "tyre_contact_heading" => Some(ColumnId::TyreContactHeading),
        "number_of_tyres_out" => Some(ColumnId::NumberOfTyresOut),
        "front_brake_compound" => Some(ColumnId::FrontBrakeCompound),
        "rear_brake_compound" => Some(ColumnId::RearBrakeCompound),

        // -- powertrain (DriverInputs group) --
        "turbo_boost" => Some(ColumnId::TurboBoost),
        "ballast" => Some(ColumnId::Ballast),
        "kers_charge" => Some(ColumnId::KersCharge),
        "kers_input" => Some(ColumnId::KersInput),
        "kers_current_kj" => Some(ColumnId::KersCurrentKj),
        "drs" => Some(ColumnId::Drs),
        "tc" | "tc_physics" => Some(ColumnId::TcPhysics),
        "abs" | "abs_physics" => Some(ColumnId::AbsPhysics),
        "engine_brake" => Some(ColumnId::EngineBrake),
        "ers_recovery_level" => Some(ColumnId::ErsRecoveryLevel),
        "ers_power_level" => Some(ColumnId::ErsPowerLevel),
        "ers_heat_charging" => Some(ColumnId::ErsHeatCharging),
        "ers_is_charging" => Some(ColumnId::ErsIsCharging),
        "drs_available" => Some(ColumnId::DrsAvailable),
        "drs_enabled" => Some(ColumnId::DrsEnabled),
        "tc_in_action" => Some(ColumnId::TcInAction),
        "abs_in_action" => Some(ColumnId::AbsInAction),
        "auto_shifter_on" => Some(ColumnId::AutoShifterOn),
        "current_max_rpm" => Some(ColumnId::CurrentMaxRpm),
        "p2p_activations" => Some(ColumnId::P2pActivations),
        "p2p_status" => Some(ColumnId::P2pStatus),
        "water_temp" => Some(ColumnId::WaterTemp),

        // -- session (Timing + Environment groups) --
        "status" => Some(ColumnId::Status),
        "session" => Some(ColumnId::Session),
        "session_index" => Some(ColumnId::SessionIndex),
        "completed_laps" => Some(ColumnId::CompletedLaps),
        "position" => Some(ColumnId::Position),
        "session_time_left" => Some(ColumnId::SessionTimeLeft),
        "number_of_laps" => Some(ColumnId::NumberOfLaps),
        "current_sector_index" => Some(ColumnId::CurrentSectorIndex),
        "normalized_car_position" => Some(ColumnId::NormalizedCarPosition),
        "is_in_pit" => Some(ColumnId::IsInPit),
        "is_in_pit_lane" => Some(ColumnId::IsInPitLane),
        "mandatory_pit_done" => Some(ColumnId::MandatoryPitDone),
        "missing_mandatory_pits" => Some(ColumnId::MissingMandatoryPits),
        "penalty_time" => Some(ColumnId::PenaltyTime),
        "penalty_type" => Some(ColumnId::PenaltyType),
        "track_status" => Some(ColumnId::TrackStatus),
        "clock" => Some(ColumnId::Clock),
        "replay_time_multiplier" => Some(ColumnId::ReplayTimeMultiplier),
        "is_valid_lap" => Some(ColumnId::IsValidLap),
        "global_yellow" => Some(ColumnId::GlobalYellow),
        "global_yellow1" => Some(ColumnId::GlobalYellow1),
        "global_yellow2" => Some(ColumnId::GlobalYellow2),
        "global_yellow3" => Some(ColumnId::GlobalYellow3),
        "global_white" => Some(ColumnId::GlobalWhite),
        "global_green" => Some(ColumnId::GlobalGreen),
        "global_chequered" => Some(ColumnId::GlobalChequered),
        "global_red" => Some(ColumnId::GlobalRed),
        "gap_ahead_or_tail_value" => Some(ColumnId::GapAheadOrTailValue),
        "flag" => Some(ColumnId::Flag),
        "gap_behind" => Some(ColumnId::GapBehind),

        // -- timing (Timing group) --
        "i_current_time" => Some(ColumnId::ICurrentTime),
        "i_last_time" => Some(ColumnId::ILastTime),
        "i_best_time" => Some(ColumnId::IBestTime),
        "i_split" => Some(ColumnId::ISplit),
        "last_sector_time" => Some(ColumnId::LastSectorTime),
        "i_delta_lap_time" => Some(ColumnId::IDeltaLapTime),
        "is_delta_positive" => Some(ColumnId::IsDeltaPositive),
        "i_estimated_lap_time" => Some(ColumnId::IEstimatedLapTime),
        "fuel_estimated_laps" => Some(ColumnId::FuelEstimatedLaps),
        "fuel_x_lap" => Some(ColumnId::FuelXLap),
        "used_fuel" => Some(ColumnId::UsedFuel),
        "distance_traveled" => Some(ColumnId::DistanceTraveled),
        "current_time_str" => Some(ColumnId::CurrentTimeStr),
        "last_time_str" => Some(ColumnId::LastTimeStr),
        "best_time_str" => Some(ColumnId::BestTimeStr),
        "split_str" => Some(ColumnId::SplitStr),
        "delta_lap_time_str" => Some(ColumnId::DeltaLapTimeStr),
        "estimated_lap_time_str" => Some(ColumnId::EstimatedLapTimeStr),
        "observed_slot_before_i_split" => Some(ColumnId::ObservedSlotBeforeISplit),

        // -- car_state (ColdStorage group) --
        "car_damage" => Some(ColumnId::CarDamage),
        "pit_limiter_on" => Some(ColumnId::PitLimiterOn),
        "ride_height" => Some(ColumnId::RideHeight),
        "ignition_on" => Some(ColumnId::IgnitionOn),
        "starter_engine_on" => Some(ColumnId::StarterEngineOn),
        "is_engine_running" => Some(ColumnId::IsEngineRunning),
        "is_ai_controlled" => Some(ColumnId::IsAiControlled),
        "cg_height" => Some(ColumnId::CgHeight),
        "brake_bias" => Some(ColumnId::BrakeBias),
        "rain_lights" => Some(ColumnId::RainLights),
        "flashing_lights" => Some(ColumnId::FlashingLights),
        "lights_stage" => Some(ColumnId::LightsStage),
        "wiper_lv" => Some(ColumnId::WiperLv),
        "driver_stint_total_time_left" => Some(ColumnId::DriverStintTotalTimeLeft),
        "driver_stint_time_left" => Some(ColumnId::DriverStintTimeLeft),
        "rain_tyres" => Some(ColumnId::RainTyres),
        "current_tyre_set" => Some(ColumnId::CurrentTyreSet),
        "strategy_tyre_set" => Some(ColumnId::StrategyTyreSet),
        "track_grip_status" => Some(ColumnId::TrackGripStatus),
        "tyre_compound_str" => Some(ColumnId::TyreCompoundStr),
        "mfd_tyre_set" => Some(ColumnId::MfdTyreSet),
        "mfd_fuel_to_add" => Some(ColumnId::MfdFuelToAdd),
        "mfd_tyre_pressure" => Some(ColumnId::MfdTyrePressure),
        "ideal_line_on" => Some(ColumnId::IdealLineOn),
        "is_setup_menu_visible" => Some(ColumnId::IsSetupMenuVisible),
        "main_display_index" => Some(ColumnId::MainDisplayIndex),
        "secondary_display_index" => Some(ColumnId::SecondaryDisplayIndex),
        "direction_lights_left" => Some(ColumnId::DirectionLightsLeft),
        "direction_lights_right" => Some(ColumnId::DirectionLightsRight),
        "tc_level" => Some(ColumnId::TcLevel),
        "tc_cut" => Some(ColumnId::TcCut),
        "engine_map" => Some(ColumnId::EngineMap),
        "abs_level" => Some(ColumnId::AbsLevel),
        "exhaust_temperature" => Some(ColumnId::ExhaustTemperature),
        "final_ff" => Some(ColumnId::FinalFf),
        "performance_meter" => Some(ColumnId::PerformanceMeter),
        "kerb_vibration" => Some(ColumnId::KerbVibration),
        "slip_vibrations" => Some(ColumnId::SlipVibrations),
        "g_vibrations" => Some(ColumnId::GVibrations),
        "abs_vibrations" => Some(ColumnId::AbsVibrations),

        // -- environment (Environment group) --
        "air_density" => Some(ColumnId::AirDensity),
        "air_temp" => Some(ColumnId::AirTemp),
        "road_temp" => Some(ColumnId::RoadTemp),
        "wind_speed" => Some(ColumnId::WindSpeed),
        "wind_direction" => Some(ColumnId::WindDirection),
        "surface_grip" => Some(ColumnId::SurfaceGrip),
        "rain_intensity" => Some(ColumnId::RainIntensity),
        "rain_intensity_in_10min" => Some(ColumnId::RainIntensityIn10min),
        "rain_intensity_in_30min" => Some(ColumnId::RainIntensityIn30min),

        // -- other_cars (ColdStorage group) --
        "active_cars" => Some(ColumnId::ActiveCars),
        "player_car_id" => Some(ColumnId::PlayerCarId),
        "car_coordinates" => Some(ColumnId::CarCoordinates),
        "car_id" => Some(ColumnId::CarId),

        // -- frame_meta columns (universal) --
        "sample_tick" => Some(ColumnId::SampleTick),
        "timestamp_ns" => Some(ColumnId::TimestampNs),
        "physics_packet_id" => Some(ColumnId::PhysicsPacketId),
        "graphics_packet_id" => Some(ColumnId::GraphicsPacketId),

        _ => None,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reader_open_empty_file_error() {
        let tmp = std::env::temp_dir().join("v2_empty_test.acctlm2");
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(&tmp, &[]).ok();
        let result = BinaryTelemetryReaderV2::open(&tmp);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_reader_from_bytes_truncated() {
        let short = vec![0u8; 32];
        let result = BinaryTelemetryReaderV2::from_bytes(short);
        assert!(result.is_err());
    }

    #[test]
    fn test_arr_helpers() {
        assert_eq!(arr3(&[1.0, 2.0, 3.0]), [1.0f32, 2.0, 3.0]);
        assert_eq!(arr4f(&[4.0, 5.0, 6.0, 7.0]), [4.0f32, 5.0, 6.0, 7.0]);
        let a12 = arr12f(&[1.0, 2.0, 3.0]);
        assert_eq!(a12[0], 1.0f32);
        assert_eq!(a12[11], 0.0f32);
    }

    // --------------------------------------------------------------------
    // path_to_column_id unit tests
    // --------------------------------------------------------------------

    #[test]
    fn test_path_to_col_controls() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("speed_kmh"), Some(ColumnId::SpeedKmh));
        assert_eq!(
            path_to_column_id("controls.speed_kmh"),
            Some(ColumnId::SpeedKmh)
        );
        assert_eq!(path_to_column_id("gas"), Some(ColumnId::Gas));
        assert_eq!(path_to_column_id("brake"), Some(ColumnId::Brake));
        assert_eq!(path_to_column_id("clutch"), Some(ColumnId::Clutch));
        assert_eq!(path_to_column_id("steer_angle"), Some(ColumnId::SteerAngle));
        assert_eq!(path_to_column_id("gear"), Some(ColumnId::Gear));
        assert_eq!(path_to_column_id("rpms"), Some(ColumnId::Rpms));
        assert_eq!(path_to_column_id("fuel"), Some(ColumnId::Fuel));
    }

    #[test]
    fn test_path_to_col_motion() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("velocity"), Some(ColumnId::Velocity));
        assert_eq!(path_to_column_id("acc_g"), Some(ColumnId::AccG));
        assert_eq!(path_to_column_id("heading"), Some(ColumnId::Heading));
        assert_eq!(path_to_column_id("pitch"), Some(ColumnId::Pitch));
        assert_eq!(path_to_column_id("roll"), Some(ColumnId::Roll));
        assert_eq!(
            path_to_column_id("local_velocity"),
            Some(ColumnId::LocalVelocity)
        );
        assert_eq!(
            path_to_column_id("local_angular_vel"),
            Some(ColumnId::LocalAngularVel)
        );
    }

    #[test]
    fn test_path_to_col_tyres() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("wheel_slip"), Some(ColumnId::WheelSlip));
        assert_eq!(path_to_column_id("wheel_load"), Some(ColumnId::WheelLoad));
        assert_eq!(
            path_to_column_id("wheels_pressure"),
            Some(ColumnId::WheelsPressure)
        );
        assert_eq!(path_to_column_id("tyre_wear"), Some(ColumnId::TyreWear));
        assert_eq!(path_to_column_id("brake_temp"), Some(ColumnId::BrakeTemp));
        assert_eq!(
            path_to_column_id("number_of_tyres_out"),
            Some(ColumnId::NumberOfTyresOut)
        );
    }

    #[test]
    fn test_path_to_col_powertrain() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("turbo_boost"), Some(ColumnId::TurboBoost));
        assert_eq!(path_to_column_id("kers_charge"), Some(ColumnId::KersCharge));
        assert_eq!(
            path_to_column_id("drs_available"),
            Some(ColumnId::DrsAvailable)
        );
        assert_eq!(
            path_to_column_id("ers_recovery_level"),
            Some(ColumnId::ErsRecoveryLevel)
        );
        assert_eq!(path_to_column_id("water_temp"), Some(ColumnId::WaterTemp));
    }

    #[test]
    fn test_path_to_col_session() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("status"), Some(ColumnId::Status));
        assert_eq!(path_to_column_id("session"), Some(ColumnId::Session));
        assert_eq!(
            path_to_column_id("completed_laps"),
            Some(ColumnId::CompletedLaps)
        );
        assert_eq!(path_to_column_id("is_in_pit"), Some(ColumnId::IsInPit));
        assert_eq!(
            path_to_column_id("global_yellow"),
            Some(ColumnId::GlobalYellow)
        );
    }

    #[test]
    fn test_path_to_col_timing() {
        use crate::format_v2::ColumnId;
        assert_eq!(
            path_to_column_id("i_current_time"),
            Some(ColumnId::ICurrentTime)
        );
        assert_eq!(path_to_column_id("i_last_time"), Some(ColumnId::ILastTime));
        assert_eq!(path_to_column_id("i_best_time"), Some(ColumnId::IBestTime));
        assert_eq!(path_to_column_id("i_split"), Some(ColumnId::ISplit));
        assert_eq!(
            path_to_column_id("fuel_estimated_laps"),
            Some(ColumnId::FuelEstimatedLaps)
        );
    }

    #[test]
    fn test_path_to_col_car_state() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("car_damage"), Some(ColumnId::CarDamage));
        assert_eq!(path_to_column_id("cg_height"), Some(ColumnId::CgHeight));
        assert_eq!(path_to_column_id("brake_bias"), Some(ColumnId::BrakeBias));
        assert_eq!(path_to_column_id("engine_map"), Some(ColumnId::EngineMap));
        assert_eq!(
            path_to_column_id("exhaust_temperature"),
            Some(ColumnId::ExhaustTemperature)
        );
    }

    #[test]
    fn test_path_to_col_environment() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("air_density"), Some(ColumnId::AirDensity));
        assert_eq!(path_to_column_id("air_temp"), Some(ColumnId::AirTemp));
        assert_eq!(path_to_column_id("road_temp"), Some(ColumnId::RoadTemp));
        assert_eq!(path_to_column_id("wind_speed"), Some(ColumnId::WindSpeed));
        assert_eq!(
            path_to_column_id("rain_intensity"),
            Some(ColumnId::RainIntensity)
        );
    }

    #[test]
    fn test_path_to_col_frame_meta() {
        use crate::format_v2::ColumnId;
        assert_eq!(path_to_column_id("sample_tick"), Some(ColumnId::SampleTick));
        assert_eq!(
            path_to_column_id("timestamp_ns"),
            Some(ColumnId::TimestampNs)
        );
        assert_eq!(
            path_to_column_id("physics_packet_id"),
            Some(ColumnId::PhysicsPacketId)
        );
        assert_eq!(
            path_to_column_id("graphics_packet_id"),
            Some(ColumnId::GraphicsPacketId)
        );
    }

    #[test]
    fn test_path_to_col_unknown() {
        assert_eq!(path_to_column_id("nonexistent_field"), None);
        assert_eq!(path_to_column_id(""), None);
        assert_eq!(path_to_column_id("foo.bar.baz"), None);
    }

    #[test]
    fn test_path_to_col_alias() {
        use crate::format_v2::ColumnId;
        // tc → TcPhysics alias
        assert_eq!(path_to_column_id("tc"), Some(ColumnId::TcPhysics));
        assert_eq!(path_to_column_id("tc_physics"), Some(ColumnId::TcPhysics));
        assert_eq!(path_to_column_id("abs"), Some(ColumnId::AbsPhysics));
        assert_eq!(path_to_column_id("abs_physics"), Some(ColumnId::AbsPhysics));
    }
}
