use crate::error::{TelemetryError, TelemetryResult};
use crate::format::{
    crc32, read_i32, read_u16, read_u32, read_u64, validate_schema, x1000_to_hz, ChunkHeader,
    ColumnEntry, FileHeader, IndexEntry, LAP_INDEX_MAGIC, CHUNK_MAGIC, CLUSTER_ENVIRONMENT,
    CLUSTER_SESSION, CLUSTER_TIMING, CLUSTER_CAR_STATE, CLUSTER_MOTION,
    CLUSTER_OTHER_CARS, CLUSTER_POWERTRAIN, CLUSTER_TYRES,
    COL_AIR_DENSITY, COL_AIR_TEMP, COL_BRAKE, COL_CLUTCH, COL_COMPLETED_LAPS, COL_CLOCK,
    COL_CURRENT_SECTOR_INDEX, COL_CURRENT_TIME_STR, COL_DISTANCE_TRAVELED, COL_FUEL,
    COL_FUEL_ESTIMATED_LAPS, COL_FUEL_X_LAP, COL_GAS, COL_GEAR, COL_GAP_AHEAD_OR_TAIL,
    COL_GAP_BEHIND, COL_FLAG, COL_GLOBAL_CHEQUERED, COL_GLOBAL_GREEN, COL_GLOBAL_RED,
    COL_GLOBAL_WHITE, COL_GLOBAL_YELLOW, COL_GLOBAL_YELLOW1, COL_GLOBAL_YELLOW2,
    COL_GLOBAL_YELLOW3, COL_I_BEST_TIME, COL_I_CURRENT_TIME, COL_I_DELTA_LAP_TIME,
    COL_I_ESTIMATED_LAP_TIME, COL_I_LAST_TIME, COL_I_SPLIT, COL_IS_DELTA_POSITIVE,
    COL_IS_IN_PIT, COL_IS_IN_PIT_LANE, COL_IS_VALID_LAP, COL_LAST_SECTOR_TIME,
    COL_LAST_TIME_STR, COL_MANDATORY_PIT_DONE, COL_MISSING_MANDATORY_PITS,
    COL_NORMALIZED_CAR_POSITION, COL_NUMBER_OF_LAPS, COL_OBSERVED_SLOT_BEFORE_I_SPLIT,
    COL_PENALTY_TIME, COL_PENALTY_TYPE, COL_POSITION, COL_RAIN_INTENSITY,
    COL_RAIN_INTENSITY_IN_10MIN, COL_RAIN_INTENSITY_IN_30MIN, COL_RPMS,
    COL_REPLAY_TIME_MULTIPLIER, COL_ROAD_TEMP, COL_SESSION, COL_SESSION_INDEX,
    COL_SESSION_TIME_LEFT, COL_SPEED_KMH, COL_SPLIT_STR, COL_STEER_ANGLE, COL_STATUS,
    COL_SURFACE_GRIP, COL_TIMESTAMP_NS, COL_TRACK_STATUS, COL_SAMPLE_TICK, COL_USED_FUEL,
    COL_WIND_DIRECTION, COL_WIND_SPEED,
    FOOTER_MAGIC, HEADER_SIZE, INDEX_MAGIC, META_MAGIC, COL_ESTIMATED_LAP_TIME_STR,
    COL_DELTA_LAP_TIME_STR, COL_BEST_TIME_STR,
};

// Motion, Tyres, Powertrain, CarState, OtherCars column IDs
use crate::format::{
    COL_VELOCITY, COL_ACC_G, COL_LOCAL_VELOCITY, COL_LOCAL_ANGULAR_VEL,
    COL_HEADING, COL_PITCH, COL_ROLL,
    COL_WHEEL_SLIP, COL_WHEEL_LOAD, COL_WHEELS_PRESSURE, COL_WHEEL_ANGULAR_SPEED,
    COL_TYRE_WEAR, COL_TYRE_DIRTY_LEVEL, COL_TYRE_CORE_TEMPERATURE, COL_CAMBER_RAD,
    COL_SUSPENSION_TRAVEL, COL_SLIP_RATIO, COL_SLIP_ANGLE, COL_TYRE_TEMP_I,
    COL_TYRE_TEMP_M, COL_TYRE_TEMP_O, COL_TYRE_TEMP, COL_MZ, COL_FX, COL_FY,
    COL_SUSPENSION_DAMAGE, COL_BRAKE_TEMP, COL_BRAKE_PRESSURE, COL_PAD_LIFE, COL_DISC_LIFE,
    COL_TYRE_CONTACT_POINT, COL_TYRE_CONTACT_NORMAL, COL_TYRE_CONTACT_HEADING,
    COL_NUMBER_OF_TYRES_OUT, COL_FRONT_BRAKE_COMPOUND, COL_REAR_BRAKE_COMPOUND,
    COL_TURBO_BOOST, COL_BALLAST, COL_KERS_CHARGE, COL_KERS_INPUT, COL_KERS_CURRENT_KJ,
    COL_DRS, COL_TC_PHYSICS, COL_ABS_PHYSICS, COL_ENGINE_BRAKE, COL_ERS_RECOVERY_LEVEL,
    COL_ERS_POWER_LEVEL, COL_ERS_HEAT_CHARGING, COL_ERS_IS_CHARGING, COL_DRS_AVAILABLE,
    COL_DRS_ENABLED, COL_TC_IN_ACTION, COL_ABS_IN_ACTION, COL_AUTO_SHIFTER_ON,
    COL_CURRENT_MAX_RPM, COL_P2P_ACTIVATIONS, COL_P2P_STATUS, COL_WATER_TEMP,
    COL_CAR_DAMAGE, COL_PIT_LIMITER_ON, COL_RIDE_HEIGHT, COL_IGNITION_ON,
    COL_STARTER_ENGINE_ON, COL_IS_ENGINE_RUNNING, COL_IS_AI_CONTROLLED, COL_CG_HEIGHT,
    COL_BRAKE_BIAS, COL_RAIN_LIGHTS, COL_FLASHING_LIGHTS, COL_LIGHTS_STAGE, COL_WIPER_LV,
    COL_DRIVER_STINT_TOTAL_TIME_LEFT, COL_DRIVER_STINT_TIME_LEFT, COL_RAIN_TYRES,
    COL_CURRENT_TYRE_SET, COL_STRATEGY_TYRE_SET, COL_TRACK_GRIP_STATUS, COL_TYRE_COMPOUND_STR,
    COL_MFD_TYRE_SET, COL_MFD_FUEL_TO_ADD, COL_MFD_TYRE_PRESSURE, COL_IDEAL_LINE_ON,
    COL_IS_SETUP_MENU_VISIBLE, COL_MAIN_DISPLAY_INDEX, COL_SECONDARY_DISPLAY_INDEX,
    COL_DIRECTION_LIGHTS_LEFT, COL_DIRECTION_LIGHTS_RIGHT, COL_TC_LEVEL, COL_TC_CUT,
    COL_ENGINE_MAP, COL_ABS_LEVEL, COL_EXHAUST_TEMPERATURE, COL_FINAL_FF,
    COL_PERFORMANCE_METER, COL_KERB_VIBRATION, COL_SLIP_VIBRATIONS, COL_G_VIBRATIONS,
    COL_ABS_VIBRATIONS,
    COL_ACTIVE_CARS, COL_PLAYER_CAR_ID, COL_CAR_COORDINATES, COL_CAR_ID,
};
use crate::types::{
    CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
    PowertrainSample, RecordingSummary, SessionMetadata, SessionSample,
    TimingSample, TyreSample, LapIndexEntry, CLUSTER_CONTROLS,
};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

pub struct BinaryTelemetryReader {
    bytes: Vec<u8>,
    header: FileHeader,
    metadata: SessionMetadata,
    index_entries: Vec<IndexEntry>,
    lap_entries: Vec<LapIndexEntry>,
    summary: RecordingSummary,
}

impl BinaryTelemetryReader {
    pub fn open(path: impl AsRef<Path>) -> TelemetryResult<Self> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> TelemetryResult<Self> {
        let mut cursor = Cursor::new(bytes.as_slice());
        let header = FileHeader::read_from(&mut cursor)?;

        if header.schema_offset < HEADER_SIZE as u64
            || header.metadata_offset <= header.schema_offset
            || header.first_chunk_offset <= header.metadata_offset
        {
            return Err(TelemetryError::InvalidFormat(
                "invalid header offsets".to_string(),
            ));
        }
        if header.first_chunk_offset as usize > bytes.len() {
            return Err(TelemetryError::InvalidFormat(
                "first chunk offset is past eof".to_string(),
            ));
        }

        let schema = &bytes[header.schema_offset as usize..header.metadata_offset as usize];
        validate_schema(schema)?;
        let metadata = decode_metadata(
            &bytes[header.metadata_offset as usize..header.first_chunk_offset as usize],
        )?;

        let (index_entries, total_samples, chunk_count, footer_offset) =
            if header.footer_offset > 0 && (header.footer_offset as usize) < bytes.len() {
                read_index_from_footer(&bytes, header.footer_offset)?
            } else {
                scan_chunks(&bytes, header.first_chunk_offset)?
            };

        let summary = RecordingSummary {
            total_samples,
            chunk_count,
            total_bytes: bytes.len() as u64,
            footer_offset,
        };

        // Try to read lap index after footer (only when footer exists)
        let lap_entries = if footer_offset > 0 {
            let lap_offset = footer_offset + 12 + (index_entries.len() as u64) * IndexEntry::BYTE_LEN as u64 + 28;
            read_lap_index_if_present(&bytes, lap_offset)
        } else {
            Vec::new()
        };

        Ok(Self {
            bytes, header, metadata,
            index_entries, lap_entries, summary,
        })
    }

    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    pub fn summary(&self) -> &RecordingSummary {
        &self.summary
    }

    pub fn header(&self) -> FileHeader {
        self.header
    }

    pub fn chunk_index(&self) -> &[IndexEntry] {
        &self.index_entries
    }

    pub fn lap_index(&self) -> &[LapIndexEntry] {
        &self.lap_entries
    }

    pub fn read_all_controls(&self) -> TelemetryResult<Vec<ControlSample>> {
        let mut out = Vec::new();
        for entry in self
            .index_entries
            .iter()
            .filter(|entry| entry.cluster_id == CLUSTER_CONTROLS)
        {
            out.extend(self.read_controls_chunk(entry)?);
        }
        Ok(out)
    }

    pub fn read_all_session(&self) -> TelemetryResult<Vec<SessionSample>> {
        let mut out = Vec::new();
        for entry in self
            .index_entries
            .iter()
            .filter(|entry| entry.cluster_id == CLUSTER_SESSION)
        {
            out.extend(self.read_session_chunk(entry)?);
        }
        Ok(out)
    }

    pub fn read_all_timing(&self) -> TelemetryResult<Vec<TimingSample>> {
        let mut out = Vec::new();
        for entry in self
            .index_entries
            .iter()
            .filter(|entry| entry.cluster_id == CLUSTER_TIMING)
        {
            out.extend(self.read_timing_chunk(entry)?);
        }
        Ok(out)
    }

    pub fn read_all_environment(&self) -> TelemetryResult<Vec<EnvironmentSample>> {
        let mut out = Vec::new();
        for entry in self
            .index_entries
            .iter()
            .filter(|entry| entry.cluster_id == CLUSTER_ENVIRONMENT)
        {
            out.extend(self.read_environment_chunk(entry)?);
        }
        Ok(out)
    }

    fn read_controls_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<ControlSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_CONTROLS)?;
        decode_controls_payload(payload, &columns, header.sample_count as usize)
    }

    fn read_session_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<SessionSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_SESSION)?;
        decode_session_payload(payload, &columns, header.sample_count as usize)
    }

    fn read_timing_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<TimingSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_TIMING)?;
        decode_timing_payload(payload, &columns, header.sample_count as usize)
    }

    fn read_environment_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<EnvironmentSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_ENVIRONMENT)?;
        decode_environment_payload(payload, &columns, header.sample_count as usize)
    }

    fn read_chunk_raw(
        &self,
        entry: &IndexEntry,
        expected_cluster: u16,
    ) -> TelemetryResult<(ChunkHeader, Vec<ColumnEntry>, &[u8])> {
        let start = entry.file_offset as usize;
        let end = start.saturating_add(entry.byte_len as usize);
        if end > self.bytes.len() {
            return Err(TelemetryError::InvalidFormat(format!(
                "chunk {} points past eof",
                entry.chunk_seq
            )));
        }

        let mut cursor = Cursor::new(&self.bytes[start..end]);
        let header = ChunkHeader::read_from(&mut cursor)?;
        if header.cluster_id != expected_cluster {
            return Err(TelemetryError::InvalidFormat(format!(
                "attempted to decode chunk with cluster 0x{:04x} as 0x{:04x}",
                header.cluster_id, expected_cluster
            )));
        }

        let mut columns = Vec::with_capacity(header.column_count as usize);
        for _ in 0..header.column_count {
            columns.push(ColumnEntry::read_from(&mut cursor)?);
        }

        let payload_start = start + ChunkHeader::byte_len(columns.len());
        let payload_end = payload_start + header.payload_len as usize;
        if payload_end > end {
            return Err(TelemetryError::InvalidFormat(
                "payload points past chunk".to_string(),
            ));
        }
        let payload = &self.bytes[payload_start..payload_end];
        if crc32(payload) != header.payload_crc32 {
            return Err(TelemetryError::InvalidFormat(format!(
                "payload crc mismatch in chunk {}",
                header.chunk_seq
            )));
        }

        Ok((header, columns, payload))
    }

    // ---- Motion ----
    pub fn read_all_motion(&self) -> TelemetryResult<Vec<MotionSample>> {
        let mut out = Vec::new();
        for entry in self.index_entries.iter().filter(|e| e.cluster_id == CLUSTER_MOTION) {
            out.extend(self.read_motion_chunk(entry)?);
        }
        Ok(out)
    }
    fn read_motion_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<MotionSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_MOTION)?;
        decode_motion_payload(payload, &columns, header.sample_count as usize)
    }

    // ---- Tyres ----
    pub fn read_all_tyres(&self) -> TelemetryResult<Vec<TyreSample>> {
        let mut out = Vec::new();
        for entry in self.index_entries.iter().filter(|e| e.cluster_id == CLUSTER_TYRES) {
            out.extend(self.read_tyres_chunk(entry)?);
        }
        Ok(out)
    }
    fn read_tyres_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<TyreSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_TYRES)?;
        decode_tyres_payload(payload, &columns, header.sample_count as usize)
    }

    // ---- Powertrain ----
    pub fn read_all_powertrain(&self) -> TelemetryResult<Vec<PowertrainSample>> {
        let mut out = Vec::new();
        for entry in self.index_entries.iter().filter(|e| e.cluster_id == CLUSTER_POWERTRAIN) {
            out.extend(self.read_powertrain_chunk(entry)?);
        }
        Ok(out)
    }
    fn read_powertrain_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<PowertrainSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_POWERTRAIN)?;
        decode_powertrain_payload(payload, &columns, header.sample_count as usize)
    }

    // ---- CarState ----
    pub fn read_all_car_state(&self) -> TelemetryResult<Vec<CarStateSample>> {
        let mut out = Vec::new();
        for entry in self.index_entries.iter().filter(|e| e.cluster_id == CLUSTER_CAR_STATE) {
            out.extend(self.read_car_state_chunk(entry)?);
        }
        Ok(out)
    }
    fn read_car_state_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<CarStateSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_CAR_STATE)?;
        decode_car_state_payload(payload, &columns, header.sample_count as usize)
    }

    // ---- OtherCars ----
    pub fn read_all_other_cars(&self) -> TelemetryResult<Vec<OtherCarsSample>> {
        let mut out = Vec::new();
        for entry in self.index_entries.iter().filter(|e| e.cluster_id == CLUSTER_OTHER_CARS) {
            out.extend(self.read_other_cars_chunk(entry)?);
        }
        Ok(out)
    }
    fn read_other_cars_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<OtherCarsSample>> {
        let (header, columns, payload) = self.read_chunk_raw(entry, CLUSTER_OTHER_CARS)?;
        decode_other_cars_payload(payload, &columns, header.sample_count as usize)
    }
}

fn decode_metadata(bytes: &[u8]) -> TelemetryResult<SessionMetadata> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != META_MAGIC {
        return Err(TelemetryError::InvalidFormat("bad metadata block".to_string()));
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

    // extended fields (appended after car for backward compat)
    let remaining = bytes.len().saturating_sub(cursor.position() as usize);
    let (sm_version, ac_version, number_of_sessions, num_cars) =
        if remaining >= 12 {
            // enough bytes for sm_len(2) + ac_len(2) + sessions(4) + num_cars(4)
            let sm_len = read_u16(&mut cursor).unwrap_or(0) as usize;
            let ac_len = read_u16(&mut cursor).unwrap_or(0) as usize;
            let ns = read_i32(&mut cursor).unwrap_or(0);
            let nc = read_i32(&mut cursor).unwrap_or(0);
            let mut sm = vec![0u8; sm_len];
            let mut ac = vec![0u8; ac_len];
            let _ = cursor.read_exact(&mut sm);
            let _ = cursor.read_exact(&mut ac);
            (String::from_utf8_lossy(&sm).into_owned(),
             String::from_utf8_lossy(&ac).into_owned(), ns, nc)
        } else {
(String::new(), String::new(), 0, 0)
        };

    // v3 extended static fields (scalar, backwards compatible)
    let (sector_count, max_rpm, max_torque, max_power, max_fuel, penalties_enabled) = {
        let remaining2 = bytes.len().saturating_sub(cursor.position() as usize);
        if remaining2 >= 24 {
            let sc = read_i32(&mut cursor).unwrap_or(0);
            let mr = read_i32(&mut cursor).unwrap_or(0);
            let mt = f32::from_le_bytes({
                let mut b = [0u8;4]; let _ = cursor.read_exact(&mut b); b
            });
            let mp = f32::from_le_bytes({
                let mut b = [0u8;4]; let _ = cursor.read_exact(&mut b); b
            });
            let mf = f32::from_le_bytes({
                let mut b = [0u8;4]; let _ = cursor.read_exact(&mut b); b
            });
            let pe = read_i32(&mut cursor).unwrap_or(0);
            (sc, mr, mt, mp, mf, pe)
        } else {
            (0, 0, 0.0, 0.0, 0.0, 0)
        }
    };

    Ok(SessionMetadata {
        track_name: String::from_utf8_lossy(&track).into_owned(),
        car_model: String::from_utf8_lossy(&car).into_owned(),
        created_unix_ns, poll_hz, chunk_rows,
        sm_version, ac_version, number_of_sessions, num_cars,
        sector_count, max_rpm, max_torque, max_power, max_fuel, penalties_enabled,
        // v4: raw static page bytes (backward compat)
        raw_static_bytes: {
            let remaining3 = bytes.len().saturating_sub(cursor.position() as usize);
            if remaining3 >= 4 {
                let raw_len = read_u32(&mut cursor).unwrap_or(0) as usize;
                if raw_len > 0 && raw_len <= remaining3.saturating_sub(4) {
                    let mut raw = vec![0u8; raw_len];
                    let _ = cursor.read_exact(&mut raw);
                    raw
                } else { Vec::new() }
            } else { Vec::new() }
        },
    })
}

fn read_index_from_footer(
    bytes: &[u8],
    footer_offset: u64,
) -> TelemetryResult<(Vec<IndexEntry>, u64, u32, u64)> {
    let mut cursor = Cursor::new(&bytes[footer_offset as usize..]);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != INDEX_MAGIC {
        return Err(TelemetryError::InvalidFormat("bad index magic".to_string()));
    }
    let entry_count = read_u64(&mut cursor)? as usize;

    // Guard against corrupt / malicious entry_count: must fit within remaining bytes
    let remaining = bytes.len().saturating_sub(footer_offset as usize);
    let min_overhead = 40; // INDEX_MAGIC(4) + entry_count_field(8) + FOOTER(28)
    let max_entries = remaining.saturating_sub(min_overhead) / IndexEntry::BYTE_LEN;
    if entry_count > max_entries {
        return Err(TelemetryError::InvalidFormat(format!(
            "index entry count {entry_count} exceeds max {max_entries}",
        )));
    }

    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        entries.push(IndexEntry::read_from(&mut cursor)?);
    }
    cursor.read_exact(&mut magic)?;
    if magic != FOOTER_MAGIC {
        return Err(TelemetryError::InvalidFormat(
            "bad footer magic".to_string(),
        ));
    }
    let _index_offset = read_u64(&mut cursor)?;
    let total_samples = read_u64(&mut cursor)?;
    let chunk_count = read_u32(&mut cursor)?;
    Ok((entries, total_samples, chunk_count, footer_offset))
}

fn scan_chunks(
    bytes: &[u8],
    first_chunk_offset: u64,
) -> TelemetryResult<(Vec<IndexEntry>, u64, u32, u64)> {
    let mut offset = first_chunk_offset as usize;
    let mut entries = Vec::new();
    let mut total_samples = 0u64;
    while offset + 4 <= bytes.len() {
        if bytes[offset..offset + 4] == INDEX_MAGIC || bytes[offset..offset + 4] == FOOTER_MAGIC {
            break;
        }
        if bytes[offset..offset + 4] != CHUNK_MAGIC {
            break;
        }
        let mut cursor = Cursor::new(&bytes[offset..]);
        let header = ChunkHeader::read_from(&mut cursor)?;
        let chunk_len =
            ChunkHeader::byte_len(header.column_count as usize) + header.payload_len as usize;
        if offset + chunk_len > bytes.len() {
            break;
        }
        let end_tick = header.base_sample_tick.saturating_add(
            (header.sample_count as u64).saturating_sub(1) * header.sample_stride as u64,
        );
        entries.push(IndexEntry {
            cluster_id: header.cluster_id,
            chunk_seq: header.chunk_seq,
            file_offset: offset as u64,
            byte_len: chunk_len as u32,
            start_time_ns: header.start_time_ns,
            end_time_ns: header.end_time_ns,
            start_tick: header.base_sample_tick,
            end_tick,
        });
        total_samples = total_samples.saturating_add(header.sample_count as u64);
        offset += chunk_len;
    }
    let chunk_count = entries.len() as u32;
    Ok((entries, total_samples, chunk_count, 0))
}

fn decode_controls_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<ControlSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let speed = read_f32_column(payload, columns, COL_SPEED_KMH, count)?;
    let gas = read_f32_column(payload, columns, COL_GAS, count)?;
    let brake = read_f32_column(payload, columns, COL_BRAKE, count)?;
    let clutch = read_f32_column(payload, columns, COL_CLUTCH, count)?;
    let steer = read_f32_column(payload, columns, COL_STEER_ANGLE, count)?;
    let gear = read_i32_column(payload, columns, COL_GEAR, count)?;
    let rpms = read_i32_column(payload, columns, COL_RPMS, count)?;
    let fuel = read_f32_column(payload, columns, COL_FUEL, count)?;
    let phys_id = read_i32_column_opt(payload, columns, crate::format::COL_PHYSICS_PACKET_ID, count);
    let gfx_id = read_i32_column_opt(payload, columns, crate::format::COL_GRAPHICS_PACKET_ID, count);

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(ControlSample {
            sample_tick: ticks[i],
            timestamp_ns: timestamps[i],
            physics_packet_id: phys_id.as_ref().map(|v| v[i]).unwrap_or(0),
            graphics_packet_id: gfx_id.as_ref().map(|v| v[i]).unwrap_or(0),
            speed_kmh: speed[i],
            gas: gas[i],
            brake: brake[i],
            clutch: clutch[i],
            steer_angle: steer[i],
            gear: gear[i],
            rpms: rpms[i],
            fuel: fuel[i],
        });
    }
    Ok(out)
}

fn decode_session_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<SessionSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let status = read_i32_column(payload, columns, COL_STATUS, count)?;
    let session = read_i32_column(payload, columns, COL_SESSION, count)?;
    let session_index = read_i32_column(payload, columns, COL_SESSION_INDEX, count)?;
    let completed_laps = read_i32_column(payload, columns, COL_COMPLETED_LAPS, count)?;
    let position = read_i32_column(payload, columns, COL_POSITION, count)?;
    let session_time_left = read_f32_column(payload, columns, COL_SESSION_TIME_LEFT, count)?;
    let number_of_laps = read_i32_column(payload, columns, COL_NUMBER_OF_LAPS, count)?;
    let current_sector_index = read_i32_column(payload, columns, COL_CURRENT_SECTOR_INDEX, count)?;
    let normalized_car_position = read_f32_column(payload, columns, COL_NORMALIZED_CAR_POSITION, count)?;
    let is_in_pit = read_i32_column(payload, columns, COL_IS_IN_PIT, count)?;
    let is_in_pit_lane = read_i32_column(payload, columns, COL_IS_IN_PIT_LANE, count)?;
    let mandatory_pit_done = read_i32_column(payload, columns, COL_MANDATORY_PIT_DONE, count)?;
    let missing_mandatory_pits = read_i32_column(payload, columns, COL_MISSING_MANDATORY_PITS, count)?;
    let penalty_time = read_f32_column(payload, columns, COL_PENALTY_TIME, count)?;
    let penalty_type = read_i32_column(payload, columns, COL_PENALTY_TYPE, count)?;
    let track_status = read_u16_column_array(payload, columns, COL_TRACK_STATUS, count, 33)?;
    let clock = read_f32_column(payload, columns, COL_CLOCK, count)?;
    let replay_time_multiplier = read_f32_column(payload, columns, COL_REPLAY_TIME_MULTIPLIER, count)?;
    let is_valid_lap = read_i32_column(payload, columns, COL_IS_VALID_LAP, count)?;
    let global_yellow = read_i32_column(payload, columns, COL_GLOBAL_YELLOW, count)?;
    let global_yellow1 = read_i32_column(payload, columns, COL_GLOBAL_YELLOW1, count)?;
    let global_yellow2 = read_i32_column(payload, columns, COL_GLOBAL_YELLOW2, count)?;
    let global_yellow3 = read_i32_column(payload, columns, COL_GLOBAL_YELLOW3, count)?;
    let global_white = read_i32_column(payload, columns, COL_GLOBAL_WHITE, count)?;
    let global_green = read_i32_column(payload, columns, COL_GLOBAL_GREEN, count)?;
    let global_chequered = read_i32_column(payload, columns, COL_GLOBAL_CHEQUERED, count)?;
    let global_red = read_i32_column(payload, columns, COL_GLOBAL_RED, count)?;
    let gap_ahead_or_tail_value = read_i32_column(payload, columns, COL_GAP_AHEAD_OR_TAIL, count)?;
    let flag = read_i32_column_opt(payload, columns, COL_FLAG, count);
    let gap_behind = read_i32_column_opt(payload, columns, COL_GAP_BEHIND, count);

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(SessionSample {
            sample_tick: ticks[i],
            timestamp_ns: timestamps[i],
            status: status[i],
            session: session[i],
            session_index: session_index[i],
            completed_laps: completed_laps[i],
            position: position[i],
            session_time_left: session_time_left[i],
            number_of_laps: number_of_laps[i],
            current_sector_index: current_sector_index[i],
            normalized_car_position: normalized_car_position[i],
            is_in_pit: is_in_pit[i],
            is_in_pit_lane: is_in_pit_lane[i],
            mandatory_pit_done: mandatory_pit_done[i],
            missing_mandatory_pits: missing_mandatory_pits[i],
            penalty_time: penalty_time[i],
            penalty_type: penalty_type[i],
            track_status: track_status[i],
            clock: clock[i],
            replay_time_multiplier: replay_time_multiplier[i],
            is_valid_lap: is_valid_lap[i],
            global_yellow: global_yellow[i],
            global_yellow1: global_yellow1[i],
            global_yellow2: global_yellow2[i],
            global_yellow3: global_yellow3[i],
            global_white: global_white[i],
            global_green: global_green[i],
            global_chequered: global_chequered[i],
            global_red: global_red[i],
gap_ahead_or_tail_value: gap_ahead_or_tail_value[i],
                flag: flag.as_ref().map(|v| v[i]).unwrap_or(0),
                gap_behind: gap_behind.as_ref().map(|v| v[i]).unwrap_or(0),
            });
    }
    Ok(out)
}

fn decode_timing_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<TimingSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let i_current_time = read_i32_column(payload, columns, COL_I_CURRENT_TIME, count)?;
    let i_last_time = read_i32_column(payload, columns, COL_I_LAST_TIME, count)?;
    let i_best_time = read_i32_column(payload, columns, COL_I_BEST_TIME, count)?;
    let i_split = read_i32_column(payload, columns, COL_I_SPLIT, count)?;
    let last_sector_time = read_i32_column(payload, columns, COL_LAST_SECTOR_TIME, count)?;
    let i_delta_lap_time = read_i32_column(payload, columns, COL_I_DELTA_LAP_TIME, count)?;
    let is_delta_positive = read_i32_column(payload, columns, COL_IS_DELTA_POSITIVE, count)?;
    let i_estimated_lap_time = read_i32_column(payload, columns, COL_I_ESTIMATED_LAP_TIME, count)?;
    let fuel_estimated_laps = read_f32_column(payload, columns, COL_FUEL_ESTIMATED_LAPS, count)?;
    let fuel_x_lap = read_f32_column(payload, columns, COL_FUEL_X_LAP, count)?;
    let used_fuel = read_f32_column(payload, columns, COL_USED_FUEL, count)?;
    let distance_traveled = read_f32_column(payload, columns, COL_DISTANCE_TRAVELED, count)?;
    let current_time_str = read_u16_column_array(payload, columns, COL_CURRENT_TIME_STR, count, 15)?;
    let last_time_str = read_u16_column_array(payload, columns, COL_LAST_TIME_STR, count, 15)?;
    let best_time_str = read_u16_column_array(payload, columns, COL_BEST_TIME_STR, count, 15)?;
    let split_str = read_u16_column_array(payload, columns, COL_SPLIT_STR, count, 15)?;
    let delta_lap_time_str = read_u16_column_array(payload, columns, COL_DELTA_LAP_TIME_STR, count, 15)?;
    let estimated_lap_time_str = read_u16_column_array(payload, columns, COL_ESTIMATED_LAP_TIME_STR, count, 15)?;
    let observed_slot_before_i_split = read_i32_column(payload, columns, COL_OBSERVED_SLOT_BEFORE_I_SPLIT, count)?;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(TimingSample {
            sample_tick: ticks[i],
            timestamp_ns: timestamps[i],
            i_current_time: i_current_time[i],
            i_last_time: i_last_time[i],
            i_best_time: i_best_time[i],
            i_split: i_split[i],
            last_sector_time: last_sector_time[i],
            i_delta_lap_time: i_delta_lap_time[i],
            is_delta_positive: is_delta_positive[i],
            i_estimated_lap_time: i_estimated_lap_time[i],
            fuel_estimated_laps: fuel_estimated_laps[i],
            fuel_x_lap: fuel_x_lap[i],
            used_fuel: used_fuel[i],
            distance_traveled: distance_traveled[i],
            current_time_str: current_time_str[i],
            last_time_str: last_time_str[i],
            best_time_str: best_time_str[i],
            split_str: split_str[i],
            delta_lap_time_str: delta_lap_time_str[i],
            estimated_lap_time_str: estimated_lap_time_str[i],
            observed_slot_before_i_split: observed_slot_before_i_split[i],
        });
    }
    Ok(out)
}

fn decode_environment_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<EnvironmentSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let air_density = read_f32_column(payload, columns, COL_AIR_DENSITY, count)?;
    let air_temp = read_f32_column(payload, columns, COL_AIR_TEMP, count)?;
    let road_temp = read_f32_column(payload, columns, COL_ROAD_TEMP, count)?;
    let wind_speed = read_f32_column(payload, columns, COL_WIND_SPEED, count)?;
    let wind_direction = read_f32_column(payload, columns, COL_WIND_DIRECTION, count)?;
    let surface_grip = read_f32_column(payload, columns, COL_SURFACE_GRIP, count)?;
    let rain_intensity = read_i32_column(payload, columns, COL_RAIN_INTENSITY, count)?;
    let rain_intensity_in_10min = read_i32_column(payload, columns, COL_RAIN_INTENSITY_IN_10MIN, count)?;
    let rain_intensity_in_30min = read_i32_column(payload, columns, COL_RAIN_INTENSITY_IN_30MIN, count)?;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(EnvironmentSample {
            sample_tick: ticks[i],
            timestamp_ns: timestamps[i],
            air_density: air_density[i],
            air_temp: air_temp[i],
            road_temp: road_temp[i],
            wind_speed: wind_speed[i],
            wind_direction: wind_direction[i],
            surface_grip: surface_grip[i],
            rain_intensity: rain_intensity[i],
            rain_intensity_in_10min: rain_intensity_in_10min[i],
            rain_intensity_in_30min: rain_intensity_in_30min[i],
        });
    }
    Ok(out)
}

fn find_column(columns: &[ColumnEntry], id: u16) -> TelemetryResult<&ColumnEntry> {
    columns
        .iter()
        .find(|column| column.column_id == id)
        .ok_or_else(|| {
            TelemetryError::InvalidFormat(format!("missing column id {id} in controls chunk"))
        })
}

fn read_u64_column(
    payload: &[u8],
    columns: &[ColumnEntry],
    id: u16,
    count: usize,
) -> TelemetryResult<Vec<u64>> {
    let column = find_column(columns, id)?;
    let bytes = column_bytes(payload, column, count, 8)?;
    Ok(bytes
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn read_f32_column(
    payload: &[u8],
    columns: &[ColumnEntry],
    id: u16,
    count: usize,
) -> TelemetryResult<Vec<f32>> {
    let column = find_column(columns, id)?;
    let bytes = column_bytes(payload, column, count, 4)?;
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn read_i32_column(
    payload: &[u8],
    columns: &[ColumnEntry],
    id: u16,
    count: usize,
) -> TelemetryResult<Vec<i32>> {
    let column = find_column(columns, id)?;
    let bytes = column_bytes(payload, column, count, 4)?;
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn read_i32_column_opt(
    payload: &[u8],
    columns: &[ColumnEntry],
    id: u16,
    count: usize,
) -> Option<Vec<i32>> {
    find_column(columns, id).ok().and_then(|col| {
        column_bytes(payload, col, count, 4).ok().map(|bytes| {
            bytes.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect()
        })
    })
}

fn read_u16_column_array<const N: usize>(
    payload: &[u8],
    columns: &[ColumnEntry],
    id: u16,
    count: usize,
    lane_count: usize,
) -> TelemetryResult<Vec<[u16; N]>> {
    let column = find_column(columns, id)?;
    let item_size = lane_count * 2; // each u16 is 2 bytes
    let bytes = column_bytes(payload, column, count, item_size)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = i * item_size;
        let mut arr = [0u16; N];
        for (j, slot) in arr.iter_mut().enumerate().take(N.min(lane_count)) {
            let off = start + j * 2;
            *slot = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
        }
        out.push(arr);
    }
    Ok(out)
}

fn column_bytes<'a>(
    payload: &'a [u8],
    column: &ColumnEntry,
    count: usize,
    item_size: usize,
) -> TelemetryResult<&'a [u8]> {
    let start = column.offset as usize;
    let expected_len = count * item_size;
    let end = start.saturating_add(expected_len);
    if column.byte_len as usize != expected_len || end > payload.len() {
        return Err(TelemetryError::InvalidFormat(format!(
            "invalid byte range for column {}",
            column.column_id
        )));
    }
    Ok(&payload[start..end])
}

fn read_lap_index_if_present(bytes: &[u8], start: u64) -> Vec<LapIndexEntry> {
    let pos = start as usize;
    if pos + 8 > bytes.len() { return Vec::new(); }
    let mut magic = [0u8; 4];
    magic.copy_from_slice(&bytes[pos..pos+4]);
    if magic != LAP_INDEX_MAGIC { return Vec::new(); }
    let count = u32::from_le_bytes(bytes[pos+4..pos+8].try_into().unwrap()) as usize;
    if count > 1000 { return Vec::new(); } // sanity check
    let entry_size = 4 + 8 + 8 + 4 + 4 + 4; // 32 bytes
    let end = pos + 8 + count * entry_size;
    if end > bytes.len() { return Vec::new(); }
    let mut entries = Vec::with_capacity(count);
    let mut off = pos + 8;
    for _ in 0..count {
        entries.push(LapIndexEntry {
            lap_number: i32::from_le_bytes(bytes[off..off+4].try_into().unwrap()),
            start_tick: u64::from_le_bytes(bytes[off+4..off+12].try_into().unwrap()),
            end_tick: u64::from_le_bytes(bytes[off+12..off+20].try_into().unwrap()),
            sample_count: u32::from_le_bytes(bytes[off+20..off+24].try_into().unwrap()),
            is_valid: i32::from_le_bytes(bytes[off+24..off+28].try_into().unwrap()),
            is_out_lap: i32::from_le_bytes(bytes[off+28..off+32].try_into().unwrap()),
        });
        off += entry_size;
    }
    entries
}
// ---- f32 array column helper ----
fn read_f32_column_array<const N: usize>(
    payload: &[u8],
    columns: &[ColumnEntry],
    id: u16,
    count: usize,
) -> TelemetryResult<Vec<[f32; N]>> {
    let column = find_column(columns, id)?;
    let item_size = N * 4;
    let bytes = column_bytes(payload, column, count, item_size)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = i * item_size;
        let mut arr = [0.0f32; N];
        for (j, slot) in arr.iter_mut().enumerate().take(N) {
            let off = start + j * 4;
            *slot = f32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        }
        out.push(arr);
    }
    Ok(out)
}

// ---- Motion decode (9 columns) ----
fn decode_motion_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<MotionSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let velocity = read_f32_column_array::<3>(payload, columns, COL_VELOCITY, count)?;
    let acc_g = read_f32_column_array::<3>(payload, columns, COL_ACC_G, count)?;
    let local_velocity = read_f32_column_array::<3>(payload, columns, COL_LOCAL_VELOCITY, count)?;
    let local_angular_vel = read_f32_column_array::<3>(payload, columns, COL_LOCAL_ANGULAR_VEL, count)?;
    let heading = read_f32_column(payload, columns, COL_HEADING, count)?;
    let pitch = read_f32_column(payload, columns, COL_PITCH, count)?;
    let roll = read_f32_column(payload, columns, COL_ROLL, count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(MotionSample {
            sample_tick: ticks[i], timestamp_ns: timestamps[i],
            velocity: velocity[i], acc_g: acc_g[i],
            local_velocity: local_velocity[i], local_angular_vel: local_angular_vel[i],
            heading: heading[i], pitch: pitch[i], roll: roll[i],
        });
    }
    Ok(out)
}

// ---- Tyres decode (31 columns) ----
fn decode_tyres_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<TyreSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let wheel_slip = read_f32_column_array::<4>(payload, columns, COL_WHEEL_SLIP, count)?;
    let wheel_load = read_f32_column_array::<4>(payload, columns, COL_WHEEL_LOAD, count)?;
    let wheels_pressure = read_f32_column_array::<4>(payload, columns, COL_WHEELS_PRESSURE, count)?;
    let wheel_angular_speed = read_f32_column_array::<4>(payload, columns, COL_WHEEL_ANGULAR_SPEED, count)?;
    let tyre_wear = read_f32_column_array::<4>(payload, columns, COL_TYRE_WEAR, count)?;
    let tyre_dirty_level = read_f32_column_array::<4>(payload, columns, COL_TYRE_DIRTY_LEVEL, count)?;
    let tyre_core_temperature = read_f32_column_array::<4>(payload, columns, COL_TYRE_CORE_TEMPERATURE, count)?;
    let camber_rad = read_f32_column_array::<4>(payload, columns, COL_CAMBER_RAD, count)?;
    let suspension_travel = read_f32_column_array::<4>(payload, columns, COL_SUSPENSION_TRAVEL, count)?;
    let slip_ratio = read_f32_column_array::<4>(payload, columns, COL_SLIP_RATIO, count)?;
    let slip_angle = read_f32_column_array::<4>(payload, columns, COL_SLIP_ANGLE, count)?;
    let tyre_temp_i = read_f32_column_array::<4>(payload, columns, COL_TYRE_TEMP_I, count)?;
    let tyre_temp_m = read_f32_column_array::<4>(payload, columns, COL_TYRE_TEMP_M, count)?;
    let tyre_temp_o = read_f32_column_array::<4>(payload, columns, COL_TYRE_TEMP_O, count)?;
    let tyre_temp = read_f32_column_array::<4>(payload, columns, COL_TYRE_TEMP, count)?;
    let mz = read_f32_column_array::<4>(payload, columns, COL_MZ, count)?;
    let fx = read_f32_column_array::<4>(payload, columns, COL_FX, count)?;
    let fy = read_f32_column_array::<4>(payload, columns, COL_FY, count)?;
    let suspension_damage = read_f32_column_array::<4>(payload, columns, COL_SUSPENSION_DAMAGE, count)?;
    let brake_temp = read_f32_column_array::<4>(payload, columns, COL_BRAKE_TEMP, count)?;
    let brake_pressure = read_f32_column_array::<4>(payload, columns, COL_BRAKE_PRESSURE, count)?;
    let pad_life = read_f32_column_array::<4>(payload, columns, COL_PAD_LIFE, count)?;
    let disc_life = read_f32_column_array::<4>(payload, columns, COL_DISC_LIFE, count)?;
    let tyre_contact_point = read_f32_column_array::<12>(payload, columns, COL_TYRE_CONTACT_POINT, count)?;
    let tyre_contact_normal = read_f32_column_array::<12>(payload, columns, COL_TYRE_CONTACT_NORMAL, count)?;
    let tyre_contact_heading = read_f32_column_array::<12>(payload, columns, COL_TYRE_CONTACT_HEADING, count)?;
    let number_of_tyres_out = read_i32_column(payload, columns, COL_NUMBER_OF_TYRES_OUT, count)?;
    let front_brake_compound = read_i32_column(payload, columns, COL_FRONT_BRAKE_COMPOUND, count)?;
    let rear_brake_compound = read_i32_column(payload, columns, COL_REAR_BRAKE_COMPOUND, count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(TyreSample {
            sample_tick: ticks[i], timestamp_ns: timestamps[i],
            wheel_slip: wheel_slip[i], wheel_load: wheel_load[i],
            wheels_pressure: wheels_pressure[i], wheel_angular_speed: wheel_angular_speed[i],
            tyre_wear: tyre_wear[i], tyre_dirty_level: tyre_dirty_level[i],
            tyre_core_temperature: tyre_core_temperature[i], camber_rad: camber_rad[i],
            suspension_travel: suspension_travel[i], slip_ratio: slip_ratio[i],
            slip_angle: slip_angle[i], tyre_temp_i: tyre_temp_i[i], tyre_temp_m: tyre_temp_m[i],
            tyre_temp_o: tyre_temp_o[i], tyre_temp: tyre_temp[i], mz: mz[i], fx: fx[i], fy: fy[i],
            suspension_damage: suspension_damage[i], brake_temp: brake_temp[i],
            brake_pressure: brake_pressure[i], pad_life: pad_life[i], disc_life: disc_life[i],
            tyre_contact_point: tyre_contact_point[i], tyre_contact_normal: tyre_contact_normal[i],
            tyre_contact_heading: tyre_contact_heading[i],
            number_of_tyres_out: number_of_tyres_out[i],
            front_brake_compound: front_brake_compound[i],
            rear_brake_compound: rear_brake_compound[i],
        });
    }
    Ok(out)
}

// ---- Powertrain decode (24 columns, all scalars) ----
fn decode_powertrain_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<PowertrainSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let turbo_boost = read_f32_column(payload, columns, COL_TURBO_BOOST, count)?;
    let ballast = read_f32_column(payload, columns, COL_BALLAST, count)?;
    let kers_charge = read_f32_column(payload, columns, COL_KERS_CHARGE, count)?;
    let kers_input = read_f32_column(payload, columns, COL_KERS_INPUT, count)?;
    let kers_current_kj = read_f32_column(payload, columns, COL_KERS_CURRENT_KJ, count)?;
    let drs = read_f32_column(payload, columns, COL_DRS, count)?;
    let tc = read_f32_column(payload, columns, COL_TC_PHYSICS, count)?;
    let abs = read_f32_column(payload, columns, COL_ABS_PHYSICS, count)?;
    let engine_brake = read_i32_column(payload, columns, COL_ENGINE_BRAKE, count)?;
    let ers_recovery_level = read_i32_column(payload, columns, COL_ERS_RECOVERY_LEVEL, count)?;
    let ers_power_level = read_i32_column(payload, columns, COL_ERS_POWER_LEVEL, count)?;
    let ers_heat_charging = read_i32_column(payload, columns, COL_ERS_HEAT_CHARGING, count)?;
    let ers_is_charging = read_i32_column(payload, columns, COL_ERS_IS_CHARGING, count)?;
    let drs_available = read_i32_column(payload, columns, COL_DRS_AVAILABLE, count)?;
    let drs_enabled = read_i32_column(payload, columns, COL_DRS_ENABLED, count)?;
    let tc_in_action = read_i32_column(payload, columns, COL_TC_IN_ACTION, count)?;
    let abs_in_action = read_i32_column(payload, columns, COL_ABS_IN_ACTION, count)?;
    let auto_shifter_on = read_i32_column(payload, columns, COL_AUTO_SHIFTER_ON, count)?;
    let current_max_rpm = read_i32_column(payload, columns, COL_CURRENT_MAX_RPM, count)?;
    let p2p_activations = read_i32_column(payload, columns, COL_P2P_ACTIVATIONS, count)?;
    let p2p_status = read_i32_column(payload, columns, COL_P2P_STATUS, count)?;
    let water_temp = read_f32_column(payload, columns, COL_WATER_TEMP, count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(PowertrainSample {
            sample_tick: ticks[i], timestamp_ns: timestamps[i],
            turbo_boost: turbo_boost[i], ballast: ballast[i],
            kers_charge: kers_charge[i], kers_input: kers_input[i],
            kers_current_kj: kers_current_kj[i], drs: drs[i], tc: tc[i], abs: abs[i],
            engine_brake: engine_brake[i], ers_recovery_level: ers_recovery_level[i],
            ers_power_level: ers_power_level[i], ers_heat_charging: ers_heat_charging[i],
            ers_is_charging: ers_is_charging[i], drs_available: drs_available[i],
            drs_enabled: drs_enabled[i], tc_in_action: tc_in_action[i],
            abs_in_action: abs_in_action[i], auto_shifter_on: auto_shifter_on[i],
            current_max_rpm: current_max_rpm[i], p2p_activations: p2p_activations[i],
            p2p_status: p2p_status[i], water_temp: water_temp[i],
        });
    }
    Ok(out)
}

// ---- CarState decode (42 columns) ----
fn decode_car_state_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<CarStateSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let car_damage = read_f32_column_array::<5>(payload, columns, COL_CAR_DAMAGE, count)?;
    let pit_limiter_on = read_i32_column(payload, columns, COL_PIT_LIMITER_ON, count)?;
    let ride_height = read_f32_column_array::<2>(payload, columns, COL_RIDE_HEIGHT, count)?;
    let ignition_on = read_i32_column(payload, columns, COL_IGNITION_ON, count)?;
    let starter_engine_on = read_i32_column(payload, columns, COL_STARTER_ENGINE_ON, count)?;
    let is_engine_running = read_i32_column(payload, columns, COL_IS_ENGINE_RUNNING, count)?;
    let is_ai_controlled = read_i32_column(payload, columns, COL_IS_AI_CONTROLLED, count)?;
    let cg_height = read_f32_column(payload, columns, COL_CG_HEIGHT, count)?;
    let brake_bias = read_f32_column(payload, columns, COL_BRAKE_BIAS, count)?;
    let rain_lights = read_i32_column(payload, columns, COL_RAIN_LIGHTS, count)?;
    let flashing_lights = read_i32_column(payload, columns, COL_FLASHING_LIGHTS, count)?;
    let lights_stage = read_i32_column(payload, columns, COL_LIGHTS_STAGE, count)?;
    let wiper_lv = read_i32_column(payload, columns, COL_WIPER_LV, count)?;
    let driver_stint_total_time_left = read_i32_column(payload, columns, COL_DRIVER_STINT_TOTAL_TIME_LEFT, count)?;
    let driver_stint_time_left = read_i32_column(payload, columns, COL_DRIVER_STINT_TIME_LEFT, count)?;
    let rain_tyres = read_i32_column(payload, columns, COL_RAIN_TYRES, count)?;
    let current_tyre_set = read_i32_column(payload, columns, COL_CURRENT_TYRE_SET, count)?;
    let strategy_tyre_set = read_i32_column(payload, columns, COL_STRATEGY_TYRE_SET, count)?;
    let track_grip_status = read_i32_column(payload, columns, COL_TRACK_GRIP_STATUS, count)?;
    let tyre_compound_str = read_u16_column_array::<33>(payload, columns, COL_TYRE_COMPOUND_STR, count, 33)?;
    let mfd_tyre_set = read_i32_column(payload, columns, COL_MFD_TYRE_SET, count)?;
    let mfd_fuel_to_add = read_f32_column(payload, columns, COL_MFD_FUEL_TO_ADD, count)?;
    let mfd_tyre_pressure = read_f32_column_array::<4>(payload, columns, COL_MFD_TYRE_PRESSURE, count)?;
    let ideal_line_on = read_i32_column(payload, columns, COL_IDEAL_LINE_ON, count)?;
    let is_setup_menu_visible = read_i32_column(payload, columns, COL_IS_SETUP_MENU_VISIBLE, count)?;
    let main_display_index = read_i32_column(payload, columns, COL_MAIN_DISPLAY_INDEX, count)?;
    let secondary_display_index = read_i32_column(payload, columns, COL_SECONDARY_DISPLAY_INDEX, count)?;
    let direction_lights_left = read_i32_column(payload, columns, COL_DIRECTION_LIGHTS_LEFT, count)?;
    let direction_lights_right = read_i32_column(payload, columns, COL_DIRECTION_LIGHTS_RIGHT, count)?;
    let tc_level = read_i32_column(payload, columns, COL_TC_LEVEL, count)?;
    let tc_cut = read_i32_column(payload, columns, COL_TC_CUT, count)?;
    let engine_map = read_i32_column(payload, columns, COL_ENGINE_MAP, count)?;
    let abs_level = read_i32_column(payload, columns, COL_ABS_LEVEL, count)?;
    let exhaust_temperature = read_f32_column(payload, columns, COL_EXHAUST_TEMPERATURE, count)?;
    let final_ff = read_f32_column(payload, columns, COL_FINAL_FF, count)?;
    let performance_meter = read_f32_column(payload, columns, COL_PERFORMANCE_METER, count)?;
    let kerb_vibration = read_f32_column(payload, columns, COL_KERB_VIBRATION, count)?;
    let slip_vibrations = read_f32_column(payload, columns, COL_SLIP_VIBRATIONS, count)?;
    let g_vibrations = read_f32_column(payload, columns, COL_G_VIBRATIONS, count)?;
    let abs_vibrations = read_f32_column(payload, columns, COL_ABS_VIBRATIONS, count)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(CarStateSample {
            sample_tick: ticks[i], timestamp_ns: timestamps[i],
            car_damage: car_damage[i], pit_limiter_on: pit_limiter_on[i],
            ride_height: ride_height[i], ignition_on: ignition_on[i],
            starter_engine_on: starter_engine_on[i], is_engine_running: is_engine_running[i],
            is_ai_controlled: is_ai_controlled[i], cg_height: cg_height[i],
            brake_bias: brake_bias[i], rain_lights: rain_lights[i],
            flashing_lights: flashing_lights[i], lights_stage: lights_stage[i],
            wiper_lv: wiper_lv[i], driver_stint_total_time_left: driver_stint_total_time_left[i],
            driver_stint_time_left: driver_stint_time_left[i], rain_tyres: rain_tyres[i],
            current_tyre_set: current_tyre_set[i], strategy_tyre_set: strategy_tyre_set[i],
            track_grip_status: track_grip_status[i], tyre_compound_str: tyre_compound_str[i],
            mfd_tyre_set: mfd_tyre_set[i], mfd_fuel_to_add: mfd_fuel_to_add[i],
            mfd_tyre_pressure: mfd_tyre_pressure[i], ideal_line_on: ideal_line_on[i],
            is_setup_menu_visible: is_setup_menu_visible[i],
            main_display_index: main_display_index[i],
            secondary_display_index: secondary_display_index[i],
            direction_lights_left: direction_lights_left[i],
            direction_lights_right: direction_lights_right[i],
            tc_level: tc_level[i], tc_cut: tc_cut[i], engine_map: engine_map[i],
            abs_level: abs_level[i], exhaust_temperature: exhaust_temperature[i],
            final_ff: final_ff[i], performance_meter: performance_meter[i],
            kerb_vibration: kerb_vibration[i], slip_vibrations: slip_vibrations[i],
            g_vibrations: g_vibrations[i], abs_vibrations: abs_vibrations[i],
        });
    }
    Ok(out)
}

// ---- OtherCars decode (6 columns) ----
fn decode_other_cars_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<OtherCarsSample>> {
    let ticks = read_u64_column(payload, columns, COL_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_TIMESTAMP_NS, count)?;
    let active_cars = read_i32_column(payload, columns, COL_ACTIVE_CARS, count)?;
    let player_car_id = read_i32_column(payload, columns, COL_PLAYER_CAR_ID, count)?;
    let coord_col = find_column(columns, COL_CAR_COORDINATES)?;
    let coord_bytes = column_bytes(payload, coord_col, count, 720)?; // 180 f32
    let id_col = find_column(columns, COL_CAR_ID)?;
    let id_bytes = column_bytes(payload, id_col, count, 240)?; // 60 i32
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut coord_vec = Vec::with_capacity(180);
        let cs = i * 720;
        for j in 0..180 {
            let off = cs + j * 4;
            coord_vec.push(f32::from_le_bytes([coord_bytes[off], coord_bytes[off+1], coord_bytes[off+2], coord_bytes[off+3]]));
        }
        let mut id_vec = Vec::with_capacity(60);
        let is = i * 240;
        for j in 0..60 {
            let off = is + j * 4;
            id_vec.push(i32::from_le_bytes([id_bytes[off], id_bytes[off+1], id_bytes[off+2], id_bytes[off+3]]));
        }
        out.push(OtherCarsSample {
            sample_tick: ticks[i], timestamp_ns: timestamps[i],
            active_cars: active_cars[i], player_car_id: player_car_id[i],
            car_coordinates: coord_vec, car_id: id_vec,
        });
    }
    Ok(out)
}
