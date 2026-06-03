use crate::error::{TelemetryError, TelemetryResult};
use crate::format::{
    crc32, read_i32, read_u16, read_u32, read_u64, validate_schema, x1000_to_hz, ChunkHeader,
    ColumnEntry, FileHeader, IndexEntry, LAP_INDEX_MAGIC, CHUNK_MAGIC, CLUSTER_SESSION, CLUSTER_TIMING,
    COL_BRAKE, COL_CLUTCH, COL_COMPLETED_LAPS, COL_CLOCK, COL_CURRENT_SECTOR_INDEX,
    COL_CURRENT_TIME_STR, COL_DISTANCE_TRAVELED, COL_FUEL, COL_FUEL_ESTIMATED_LAPS, COL_FUEL_X_LAP,
    COL_GAS, COL_GEAR, COL_GAP_AHEAD_OR_TAIL, COL_GAP_BEHIND, COL_FLAG, COL_GLOBAL_CHEQUERED, COL_GLOBAL_GREEN,
    COL_GLOBAL_RED, COL_GLOBAL_WHITE, COL_GLOBAL_YELLOW, COL_GLOBAL_YELLOW1, COL_GLOBAL_YELLOW2,
    COL_GLOBAL_YELLOW3, COL_I_BEST_TIME, COL_I_CURRENT_TIME, COL_I_DELTA_LAP_TIME,
    COL_I_ESTIMATED_LAP_TIME, COL_I_LAST_TIME, COL_I_SPLIT, COL_IS_DELTA_POSITIVE,
    COL_IS_IN_PIT, COL_IS_IN_PIT_LANE, COL_IS_VALID_LAP, COL_LAST_SECTOR_TIME,
    COL_LAST_TIME_STR, COL_MANDATORY_PIT_DONE, COL_MISSING_MANDATORY_PITS,
    COL_NORMALIZED_CAR_POSITION, COL_NUMBER_OF_LAPS, COL_OBSERVED_SLOT_BEFORE_I_SPLIT,
    COL_PENALTY_TIME, COL_PENALTY_TYPE, COL_POSITION, COL_RPMS,
    COL_REPLAY_TIME_MULTIPLIER, COL_SESSION, COL_SESSION_INDEX, COL_SESSION_TIME_LEFT,
    COL_SPEED_KMH, COL_SPLIT_STR, COL_STEER_ANGLE, COL_STATUS, COL_TIMESTAMP_NS,
    COL_TRACK_STATUS, COL_SAMPLE_TICK, COL_USED_FUEL,
    FOOTER_MAGIC, HEADER_SIZE, INDEX_MAGIC, META_MAGIC, COL_ESTIMATED_LAP_TIME_STR,
    COL_DELTA_LAP_TIME_STR, COL_BEST_TIME_STR,
};
use crate::types::{
    ControlSample, RecordingSummary, SessionMetadata, SessionSample, TimingSample, LapIndexEntry,
    CLUSTER_CONTROLS,
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

        // Try to read lap index after footer
        let lap_entries = read_lap_index_if_present(&bytes, footer_offset + 24);

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

fn find_column<'a>(columns: &'a [ColumnEntry], id: u16) -> TelemetryResult<&'a ColumnEntry> {
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
        for j in 0..N.min(lane_count) {
            let off = start + j * 2;
            arr[j] = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
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