use crate::error::{TelemetryError, TelemetryResult};
use crate::format::{
    crc32, read_i32, read_u16, read_u32, read_u64, validate_schema, x1000_to_hz, ChunkHeader, ColumnEntry,
    FileHeader, IndexEntry, CHUNK_MAGIC, COL_BRAKE, COL_CLUTCH, COL_FUEL, COL_GAS, COL_GEAR,
    COL_RAW_GRAPHICS_PAGE, COL_RAW_PHYSICS_PAGE, COL_RAW_SAMPLE_TICK, COL_RAW_STATIC_PAGE,
    COL_RAW_TIMESTAMP_NS, COL_RPMS, COL_SAMPLE_TICK, COL_SPEED_KMH, COL_STEER_ANGLE,
    COL_TIMESTAMP_NS, FOOTER_MAGIC, HEADER_SIZE, INDEX_MAGIC, META_MAGIC,
    TYPE_BYTES,
};
use crate::laps::{segment_raw_session, RawSessionSegments};
use crate::types::{
    ControlSample, RawGraphicsPageSample, RawGraphicsSample, RawPageSample, RecordingSummary, SessionMetadata,
    CLUSTER_CONTROLS, CLUSTER_RAW_PAGES,
};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

pub struct BinaryTelemetryReader {
    bytes: Vec<u8>,
    header: FileHeader,
    metadata: SessionMetadata,
    index_entries: Vec<IndexEntry>,
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

        Ok(Self {
            bytes,
            header,
            metadata,
            index_entries,
            summary,
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

    pub fn read_all_raw_graphics_samples(&self) -> TelemetryResult<Vec<RawGraphicsSample>> {
        let mut out = Vec::new();
        for entry in self
            .index_entries
            .iter()
            .filter(|entry| entry.cluster_id == CLUSTER_RAW_PAGES)
        {
            out.extend(self.read_raw_graphics_chunk(entry)?);
        }
        Ok(out)
    }

    pub fn read_all_raw_graphics_pages(&self) -> TelemetryResult<Vec<RawGraphicsPageSample>> {
        let mut out = Vec::new();
        for entry in self
            .index_entries
            .iter()
            .filter(|entry| entry.cluster_id == CLUSTER_RAW_PAGES)
        {
            out.extend(self.read_raw_graphics_page_chunk(entry)?);
        }
        Ok(out)
    }

    pub fn read_all_raw_page_samples(&self) -> TelemetryResult<Vec<RawPageSample>> {
        let mut out = Vec::new();
        for entry in self.index_entries.iter().filter(|e| e.cluster_id == CLUSTER_RAW_PAGES) {
            out.extend(self.read_raw_page_chunk(entry)?);
        }
        Ok(out)
    }

    pub fn segment_raw_session(&self) -> TelemetryResult<RawSessionSegments> {
        let samples = self.read_all_raw_graphics_samples()?;
        Ok(segment_raw_session(&self.metadata, &samples))
    }

    fn read_controls_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<ControlSample>> {
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
        if header.cluster_id != CLUSTER_CONTROLS {
            return Err(TelemetryError::InvalidFormat(
                "attempted to decode non-controls chunk as controls".to_string(),
            ));
        }

        let mut columns = Vec::with_capacity(header.column_count as usize);
        for _ in 0..header.column_count {
            columns.push(ColumnEntry::read_from(&mut cursor)?);
        }

        let payload_start = start + ChunkHeader::byte_len(columns.len());
        let payload_end = payload_start + header.payload_len as usize;
        if payload_end > end {
            return Err(TelemetryError::InvalidFormat(
                "controls payload points past chunk".to_string(),
            ));
        }
        let payload = &self.bytes[payload_start..payload_end];
        if crc32(payload) != header.payload_crc32 {
            return Err(TelemetryError::InvalidFormat(format!(
                "payload crc mismatch in chunk {}",
                header.chunk_seq
            )));
        }

        decode_controls_payload(payload, &columns, header.sample_count as usize)
    }

    fn read_raw_page_chunk(&self, entry: &IndexEntry) -> TelemetryResult<Vec<RawPageSample>> {
        let start = entry.file_offset as usize;
        let end = start.saturating_add(entry.byte_len as usize);
        if end > self.bytes.len() {
            return Err(TelemetryError::InvalidFormat(format!("raw chunk {} past eof", entry.chunk_seq)));
        }
        let mut cursor = Cursor::new(&self.bytes[start..end]);
        let header = ChunkHeader::read_from(&mut cursor)?;
        if header.cluster_id != CLUSTER_RAW_PAGES {
            return Err(TelemetryError::InvalidFormat("bad raw cluster id".into()));
        }
        let mut columns = Vec::with_capacity(header.column_count as usize);
        for _ in 0..header.column_count { columns.push(ColumnEntry::read_from(&mut cursor)?); }
        let payload_start = start + ChunkHeader::byte_len(columns.len());
        let payload_end = payload_start + header.payload_len as usize;
        if payload_end > end { return Err(TelemetryError::InvalidFormat("raw payload past chunk".into())); }
        let payload = &self.bytes[payload_start..payload_end];
        if crc32(payload) != header.payload_crc32 {
            return Err(TelemetryError::InvalidFormat(format!("crc mismatch in raw chunk {}", header.chunk_seq)));
        }
        decode_raw_pages_payload(payload, &columns, header.sample_count as usize)
    }

    fn read_raw_graphics_chunk(
        &self,
        entry: &IndexEntry,
    ) -> TelemetryResult<Vec<RawGraphicsSample>> {
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
        if header.cluster_id != CLUSTER_RAW_PAGES {
            return Err(TelemetryError::InvalidFormat(
                "attempted to decode non-raw chunk as raw graphics".to_string(),
            ));
        }

        let mut columns = Vec::with_capacity(header.column_count as usize);
        for _ in 0..header.column_count {
            columns.push(ColumnEntry::read_from(&mut cursor)?);
        }

        let payload_start = start + ChunkHeader::byte_len(columns.len());
        let payload_end = payload_start + header.payload_len as usize;
        if payload_end > end {
            return Err(TelemetryError::InvalidFormat(
                "raw payload points past chunk".to_string(),
            ));
        }
        let payload = &self.bytes[payload_start..payload_end];
        if crc32(payload) != header.payload_crc32 {
            return Err(TelemetryError::InvalidFormat(format!(
                "payload crc mismatch in raw chunk {}",
                header.chunk_seq
            )));
        }

        decode_raw_graphics_payload(payload, &columns, header.sample_count as usize)
    }

    fn read_raw_graphics_page_chunk(
        &self,
        entry: &IndexEntry,
    ) -> TelemetryResult<Vec<RawGraphicsPageSample>> {
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
        if header.cluster_id != CLUSTER_RAW_PAGES {
            return Err(TelemetryError::InvalidFormat(
                "attempted to decode non-raw chunk as raw graphics pages".to_string(),
            ));
        }
        let mut columns = Vec::with_capacity(header.column_count as usize);
        for _ in 0..header.column_count {
            columns.push(ColumnEntry::read_from(&mut cursor)?);
        }
        let payload_start = start + ChunkHeader::byte_len(columns.len());
        let payload_end = payload_start + header.payload_len as usize;
        if payload_end > end {
            return Err(TelemetryError::InvalidFormat(
                "raw payload points past chunk".to_string(),
            ));
        }
        let payload = &self.bytes[payload_start..payload_end];
        if crc32(payload) != header.payload_crc32 {
            return Err(TelemetryError::InvalidFormat(format!(
                "payload crc mismatch in raw chunk {}",
                header.chunk_seq
            )));
        }
        decode_raw_graphics_pages(payload, &columns, header.sample_count as usize)
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

    // v2 extended fields (appended after car for backward compat)
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

    Ok(SessionMetadata {
        track_name: String::from_utf8_lossy(&track).into_owned(),
        car_model: String::from_utf8_lossy(&car).into_owned(),
        created_unix_ns, poll_hz, chunk_rows,
        sm_version,
        ac_version,
        number_of_sessions,
        num_cars,
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
    // v2 columns (optional, default to 0 for v1 files)
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

fn decode_raw_pages_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<RawPageSample>> {
    let ticks = read_u64_column(payload, columns, COL_RAW_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_RAW_TIMESTAMP_NS, count)?;
    let physics = find_column(columns, COL_RAW_PHYSICS_PAGE)?;
    let graphics = find_column(columns, COL_RAW_GRAPHICS_PAGE)?;
    let stat = find_column(columns, COL_RAW_STATIC_PAGE)?;
    if count == 0 { return Ok(Vec::new()); }
    if physics.byte_len as usize % count != 0 {
        return Err(TelemetryError::InvalidFormat("physics column not divisible".into()));
    }
    if graphics.byte_len as usize % count != 0 {
        return Err(TelemetryError::InvalidFormat("graphics column not divisible".into()));
    }
    if stat.byte_len as usize % count != 0 {
        return Err(TelemetryError::InvalidFormat("static column not divisible".into()));
    }
    let phys_size = physics.byte_len as usize / count;
    let gfx_size = graphics.byte_len as usize / count;
    let stat_size = stat.byte_len as usize / count;
    let phys_bytes = column_bytes(payload, physics, count, phys_size)?;
    let gfx_bytes = column_bytes(payload, graphics, count, gfx_size)?;
    let stat_bytes = column_bytes(payload, stat, count, stat_size)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(RawPageSample {
            sample_tick: ticks[i],
            timestamp_ns: timestamps[i],
            physics_page: phys_bytes[i * phys_size..(i + 1) * phys_size].to_vec(),
            graphics_page: gfx_bytes[i * gfx_size..(i + 1) * gfx_size].to_vec(),
            static_page: stat_bytes[i * stat_size..(i + 1) * stat_size].to_vec(),
        });
    }
    Ok(out)
}

fn decode_raw_graphics_payload(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<RawGraphicsSample>> {
    let ticks = read_u64_column(payload, columns, COL_RAW_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_RAW_TIMESTAMP_NS, count)?;
    let graphics = find_column(columns, COL_RAW_GRAPHICS_PAGE)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if graphics.byte_len as usize % count != 0 {
        return Err(TelemetryError::InvalidFormat(
            "raw graphics column length is not divisible by sample count".to_string(),
        ));
    }
    let page_size = graphics.byte_len as usize / count;
    if page_size < GRAPHICS_GAP_BEHIND_OFFSET + 4 {
        return Err(TelemetryError::InvalidFormat(format!(
            "raw graphics page is too small: {page_size}"
        )));
    }
    let bytes = column_bytes(payload, graphics, count, page_size)?;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let page = &bytes[i * page_size..(i + 1) * page_size];
        out.push(RawGraphicsSample {
            sample_tick: ticks[i],
            timestamp_ns: timestamps[i],
            status: read_i32_at(page, GRAPHICS_STATUS_OFFSET),
            session: read_i32_at(page, GRAPHICS_SESSION_OFFSET),
            completed_laps: read_i32_at(page, GRAPHICS_COMPLETED_LAPS_OFFSET),
            current_lap_time_ms: read_i32_at(page, GRAPHICS_CURRENT_LAP_TIME_OFFSET),
            last_lap_time_ms: read_i32_at(page, GRAPHICS_LAST_LAP_TIME_OFFSET),
            best_lap_time_ms: read_i32_at(page, GRAPHICS_BEST_LAP_TIME_OFFSET),
            distance_traveled_m: read_f32_at(page, GRAPHICS_DISTANCE_TRAVELED_OFFSET),
            normalized_car_position: read_f32_at(page, GRAPHICS_NORMALIZED_CAR_POSITION_OFFSET),
            is_valid_lap: read_i32_at(page, GRAPHICS_IS_VALID_LAP_OFFSET),
            current_sector: read_i32_at(page, GRAPHICS_CURRENT_SECTOR_OFFSET),
            last_sector_time_ms: read_i32_at(page, GRAPHICS_LAST_SECTOR_TIME_OFFSET),
            in_pit: read_i32_at(page, GRAPHICS_IN_PIT_OFFSET),
            in_pit_lane: read_i32_at(page, GRAPHICS_IN_PIT_LANE_OFFSET),
            delta_lap_time_ms: read_i32_at(page, GRAPHICS_DELTA_LAP_TIME_OFFSET),
        });
    }
    Ok(out)
}

fn decode_raw_graphics_pages(
    payload: &[u8],
    columns: &[ColumnEntry],
    count: usize,
) -> TelemetryResult<Vec<RawGraphicsPageSample>> {
    let ticks = read_u64_column(payload, columns, COL_RAW_SAMPLE_TICK, count)?;
    let timestamps = read_u64_column(payload, columns, COL_RAW_TIMESTAMP_NS, count)?;
    let graphics = find_column(columns, COL_RAW_GRAPHICS_PAGE)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if graphics.byte_len as usize % count != 0 {
        return Err(TelemetryError::InvalidFormat(
            "raw graphics column length is not divisible by sample count".to_string(),
        ));
    }
    let page_size = graphics.byte_len as usize / count;
    let bytes = column_bytes(payload, graphics, count, page_size)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(RawGraphicsPageSample {
            sample_tick: ticks[i],
            timestamp_ns: timestamps[i],
            page: bytes[i * page_size..(i + 1) * page_size].to_vec(),
        });
    }
    Ok(out)
}

const GRAPHICS_STATUS_OFFSET: usize = 4;
const GRAPHICS_SESSION_OFFSET: usize = 8;
const GRAPHICS_COMPLETED_LAPS_OFFSET: usize = 132;
const GRAPHICS_CURRENT_LAP_TIME_OFFSET: usize = 140;
const GRAPHICS_LAST_LAP_TIME_OFFSET: usize = 144;
const GRAPHICS_BEST_LAP_TIME_OFFSET: usize = 148;
const GRAPHICS_DISTANCE_TRAVELED_OFFSET: usize = 156;
const GRAPHICS_IN_PIT_OFFSET: usize = 160;
const GRAPHICS_CURRENT_SECTOR_OFFSET: usize = 164;
const GRAPHICS_LAST_SECTOR_TIME_OFFSET: usize = 168;
const GRAPHICS_NORMALIZED_CAR_POSITION_OFFSET: usize = 248;
const GRAPHICS_IN_PIT_LANE_OFFSET: usize = 1232;
const GRAPHICS_DELTA_LAP_TIME_OFFSET: usize = 1356;
const GRAPHICS_IS_VALID_LAP_OFFSET: usize = 1408;
const GRAPHICS_GAP_BEHIND_OFFSET: usize = 1580;

fn read_i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
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
