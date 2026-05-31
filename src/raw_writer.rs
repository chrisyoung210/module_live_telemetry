use crate::error::{TelemetryError, TelemetryResult};
use crate::format::{
    crc32, encode_schema, ChunkHeader, ColumnEntry, FileHeader, IndexEntry, CODEC_PLAIN_LE,
    COL_RAW_GRAPHICS_PAGE, COL_RAW_PHYSICS_PAGE, COL_RAW_STATIC_PAGE, FOOTER_MAGIC, INDEX_MAGIC,
    RAW_PAGE_COLUMNS, SCHEMA_HASH, TYPE_BYTES, TYPE_U64,
};
use crate::types::{
    CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
    PowertrainSample, RawPageSample, RecordingSummary, SessionMetadata, SessionSample,
    TimingSample, TyreSample, CLUSTER_RAW_PAGES,
};
use crate::writer::{encode_metadata, BinaryTelemetryWriter, LiveTelemetryConfig, TelemetryFrame};
use crate::reader::BinaryTelemetryReader;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RawPageTelemetryConfig {
    pub poll_hz: f64,
    pub chunk_rows: usize,
    pub physics_page_size: usize,
    pub graphics_page_size: usize,
    pub static_page_size: usize,
}

pub struct RawPageTelemetryWriter<W: Write + Seek> {
    writer: W,
    header: FileHeader,
    config: RawPageTelemetryConfig,
    pending_samples: Vec<RawPageSample>,
    index_entries: Vec<IndexEntry>,
    total_samples: u64,
    next_chunk_seq: u32,
    finished: bool,
}

impl RawPageTelemetryWriter<File> {
    pub fn create_file(
        path: impl AsRef<Path>,
        metadata: SessionMetadata,
        config: RawPageTelemetryConfig,
    ) -> TelemetryResult<Self> {
        let file = File::create(path)?;
        Self::create(file, metadata, config)
    }
}

impl<W: Write + Seek> RawPageTelemetryWriter<W> {
    pub fn create(
        mut writer: W,
        mut metadata: SessionMetadata,
        config: RawPageTelemetryConfig,
    ) -> TelemetryResult<Self> {
        if config.chunk_rows == 0 {
            return Err(TelemetryError::InvalidArgument(
                "chunk_rows must be greater than zero".to_string(),
            ));
        }
        if config.physics_page_size == 0
            || config.graphics_page_size == 0
            || config.static_page_size == 0
        {
            return Err(TelemetryError::InvalidArgument(
                "raw page sizes must be greater than zero".to_string(),
            ));
        }

        metadata.poll_hz = config.poll_hz;
        metadata.chunk_rows = config.chunk_rows;

        let schema = encode_schema();
        let metadata_bytes = encode_metadata(&metadata);
        let mut header = FileHeader::new(metadata.created_unix_ns, config.poll_hz);
        header.metadata_offset = header.schema_offset + schema.len() as u64;
        header.first_chunk_offset = header.metadata_offset + metadata_bytes.len() as u64;

        writer.seek(SeekFrom::Start(0))?;
        header.write_to(&mut writer)?;
        writer.write_all(&schema)?;
        writer.write_all(&metadata_bytes)?;
        writer.seek(SeekFrom::Start(0))?;
        header.write_to(&mut writer)?;
        writer.seek(SeekFrom::Start(header.first_chunk_offset))?;

        Ok(Self {
            writer,
            header,
            config,
            pending_samples: Vec::with_capacity(metadata.chunk_rows),
            index_entries: Vec::new(),
            total_samples: 0,
            next_chunk_seq: 0,
            finished: false,
        })
    }

    pub fn write_sample(&mut self, sample: RawPageSample) -> TelemetryResult<()> {
        if self.finished {
            return Err(TelemetryError::InvalidArgument(
                "cannot write after finish".to_string(),
            ));
        }
        if sample.physics_page.len() != self.config.physics_page_size {
            return Err(TelemetryError::InvalidArgument(format!(
                "physics page size mismatch: expected {}, got {}",
                self.config.physics_page_size,
                sample.physics_page.len()
            )));
        }
        if sample.graphics_page.len() != self.config.graphics_page_size {
            return Err(TelemetryError::InvalidArgument(format!(
                "graphics page size mismatch: expected {}, got {}",
                self.config.graphics_page_size,
                sample.graphics_page.len()
            )));
        }
        if sample.static_page.len() != self.config.static_page_size {
            return Err(TelemetryError::InvalidArgument(format!(
                "static page size mismatch: expected {}, got {}",
                self.config.static_page_size,
                sample.static_page.len()
            )));
        }

        self.pending_samples.push(sample);
        self.total_samples = self.total_samples.saturating_add(1);
        if self.pending_samples.len() >= self.config.chunk_rows {
            self.flush_chunk()?;
        }
        Ok(())
    }

    pub fn flush_ready_chunks(&mut self) -> TelemetryResult<()> {
        self.flush_chunk()
    }

    pub fn flush_to_disk(&mut self) -> TelemetryResult<()> {
        self.flush_ready_chunks()?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn finish(mut self) -> TelemetryResult<(W, RecordingSummary)> {
        if !self.pending_samples.is_empty() {
            self.flush_chunk()?;
        }

        let footer_offset = self.writer.stream_position()?;
        let index_offset = footer_offset;
        self.writer.write_all(&INDEX_MAGIC)?;
        self.writer
            .write_all(&(self.index_entries.len() as u64).to_le_bytes())?;
        for entry in &self.index_entries {
            entry.write_to(&mut self.writer)?;
        }
        self.writer.write_all(&FOOTER_MAGIC)?;
        self.writer.write_all(&index_offset.to_le_bytes())?;
        self.writer.write_all(&self.total_samples.to_le_bytes())?;
        self.writer.write_all(&self.next_chunk_seq.to_le_bytes())?;
        self.writer.write_all(&[0u8; 4])?;

        let total_bytes = self.writer.stream_position()?;
        self.header.footer_offset = footer_offset;
        self.writer.seek(SeekFrom::Start(0))?;
        self.header.write_to(&mut self.writer)?;
        self.writer.seek(SeekFrom::Start(total_bytes))?;
        self.writer.flush()?;
        self.finished = true;

        let summary = RecordingSummary {
            total_samples: self.total_samples,
            chunk_count: self.next_chunk_seq,
            total_bytes,
            footer_offset,
        };

        Ok((self.writer, summary))
    }

    fn flush_chunk(&mut self) -> TelemetryResult<()> {
        if self.pending_samples.is_empty() {
            return Ok(());
        }

        let file_offset = self.writer.stream_position()?;
        let samples = std::mem::take(&mut self.pending_samples);
        let payload = encode_raw_payload(
            &samples,
            self.config.physics_page_size,
            self.config.graphics_page_size,
            self.config.static_page_size,
        );
        let entries = build_raw_column_entries(
            &samples,
            self.config.physics_page_size,
            self.config.graphics_page_size,
            self.config.static_page_size,
        );
        let first = samples.first().expect("non-empty samples");
        let last = samples.last().expect("non-empty samples");
        let byte_len = (ChunkHeader::byte_len(entries.len()) + payload.len()) as u32;
        let header = ChunkHeader {
            cluster_id: CLUSTER_RAW_PAGES,
            chunk_seq: self.next_chunk_seq,
            schema_hash: SCHEMA_HASH,
            base_sample_tick: first.sample_tick,
            sample_stride: infer_sample_stride(&samples),
            sample_count: samples.len() as u32,
            start_time_ns: first.timestamp_ns,
            end_time_ns: last.timestamp_ns,
            start_lap: -1,
            end_lap: -1,
            column_count: entries.len() as u16,
            flags: 0,
            payload_len: payload.len() as u32,
            payload_crc32: crc32(&payload),
        };

        header.write_to(&mut self.writer)?;
        for entry in &entries {
            entry.write_to(&mut self.writer)?;
        }
        self.writer.write_all(&payload)?;

        self.index_entries.push(IndexEntry {
            cluster_id: CLUSTER_RAW_PAGES,
            chunk_seq: self.next_chunk_seq,
            file_offset,
            byte_len,
            start_time_ns: first.timestamp_ns,
            end_time_ns: last.timestamp_ns,
            start_tick: first.sample_tick,
            end_tick: last.sample_tick,
        });
        self.next_chunk_seq = self.next_chunk_seq.saturating_add(1);
        Ok(())
    }
}

fn encode_raw_payload(
    samples: &[RawPageSample],
    physics_page_size: usize,
    graphics_page_size: usize,
    static_page_size: usize,
) -> Vec<u8> {
    let row_bytes = 16 + physics_page_size + graphics_page_size + static_page_size;
    let mut payload = Vec::with_capacity(samples.len() * row_bytes);
    for sample in samples {
        payload.extend_from_slice(&sample.sample_tick.to_le_bytes());
    }
    for sample in samples {
        payload.extend_from_slice(&sample.timestamp_ns.to_le_bytes());
    }
    for sample in samples {
        payload.extend_from_slice(&sample.physics_page);
    }
    for sample in samples {
        payload.extend_from_slice(&sample.graphics_page);
    }
    for sample in samples {
        payload.extend_from_slice(&sample.static_page);
    }
    payload
}

fn build_raw_column_entries(
    samples: &[RawPageSample],
    physics_page_size: usize,
    graphics_page_size: usize,
    static_page_size: usize,
) -> Vec<ColumnEntry> {
    let count = samples.len();
    let tick_len = count * 8;
    let timestamp_len = count * 8;
    let physics_len = count * physics_page_size;
    let graphics_len = count * graphics_page_size;
    let static_len = count * static_page_size;

    let tick_min = samples.first().map(|s| s.sample_tick as f64).unwrap_or(0.0);
    let tick_max = samples.last().map(|s| s.sample_tick as f64).unwrap_or(0.0);
    let time_min = samples
        .first()
        .map(|s| s.timestamp_ns as f64)
        .unwrap_or(0.0);
    let time_max = samples.last().map(|s| s.timestamp_ns as f64).unwrap_or(0.0);

    let lengths = [
        tick_len,
        timestamp_len,
        physics_len,
        graphics_len,
        static_len,
    ];
    let ranges = [
        (tick_min, tick_max),
        (time_min, time_max),
        (0.0, 0.0),
        (0.0, 0.0),
        (0.0, 0.0),
    ];
    let mut offset = 0u32;
    let mut entries = Vec::with_capacity(RAW_PAGE_COLUMNS.len());
    for (index, column) in RAW_PAGE_COLUMNS.iter().enumerate() {
        let byte_len = lengths[index] as u32;
        let (min_value, max_value) = ranges[index];
        entries.push(ColumnEntry {
            column_id: column.id,
            codec: CODEC_PLAIN_LE,
            value_type: if matches!(
                column.id,
                COL_RAW_PHYSICS_PAGE | COL_RAW_GRAPHICS_PAGE | COL_RAW_STATIC_PAGE
            ) {
                TYPE_BYTES
            } else {
                TYPE_U64
            },
            lane_count: 1,
            flags: 0,
            offset,
            byte_len,
            null_offset: 0,
            min_value,
            max_value,
        });
        offset = offset.saturating_add(byte_len);
    }
    entries
}

fn infer_sample_stride(samples: &[RawPageSample]) -> u32 {
    if samples.len() < 2 {
        return 1;
    }
    let first = samples[0].sample_tick;
    let second = samples[1].sample_tick;
    second.saturating_sub(first).clamp(1, u32::MAX as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::BinaryTelemetryReader;
    use std::io::Cursor;

    #[test]
    fn flush_to_disk_emits_partial_raw_chunk_before_finish() {
        let metadata = SessionMetadata::new("monza", "mclaren_720s_gt3_evo", 120.0);
        let config = RawPageTelemetryConfig {
            poll_hz: 120.0,
            chunk_rows: 10,
            physics_page_size: 4,
            graphics_page_size: 4,
            static_page_size: 4,
        };
        let cursor = Cursor::new(Vec::new());
        let mut writer = RawPageTelemetryWriter::create(cursor, metadata, config).unwrap();

        for tick in 0..3 {
            writer.write_sample(raw_sample(tick)).unwrap();
        }
        writer.flush_to_disk().unwrap();

        let (cursor, summary) = writer.finish().unwrap();
        assert_eq!(summary.total_samples, 3);
        assert_eq!(summary.chunk_count, 1);

        let reader = BinaryTelemetryReader::from_bytes(cursor.into_inner()).unwrap();
        assert_eq!(reader.summary().total_samples, 3);
        assert_eq!(reader.chunk_index().len(), 1);
        assert_eq!(reader.chunk_index()[0].start_tick, 0);
        assert_eq!(reader.chunk_index()[0].end_tick, 2);
    }

    fn raw_sample(sample_tick: u64) -> RawPageSample {
        RawPageSample {
            sample_tick,
            timestamp_ns: sample_tick * 1_000_000,
            physics_page: vec![sample_tick as u8; 4],
            graphics_page: vec![sample_tick as u8 + 1; 4],
            static_page: vec![sample_tick as u8 + 2; 4],
        }
    }
}

// ---- Import: v1 raw-pages -> v2 flat clusters ----

pub fn import_raw_to_v2(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    chunk_rows: usize,
) -> TelemetryResult<crate::types::RecordingSummary> {
    let reader = BinaryTelemetryReader::open(&input)?;
    let raw_samples = reader.read_all_raw_page_samples()?;
    if raw_samples.is_empty() { return Err(crate::error::TelemetryError::InvalidArgument("empty input".into())); }

    // Parse static page from first sample
    let first_static = &raw_samples[0].static_page;
    let metadata = parse_static_metadata(&reader, first_static)?;

    let config = LiveTelemetryConfig { poll_hz: reader.metadata().poll_hz, chunk_rows };
    let mut writer = BinaryTelemetryWriter::create_file(&output, metadata, config)?;

    for raw in &raw_samples {
        let phys = &raw.physics_page;
        let gfx = &raw.graphics_page;
        if phys.len() < 800 || gfx.len() < 1584 { continue; }

        let controls = parse_controls_sample(raw.sample_tick, raw.timestamp_ns, phys);
        let motion = parse_motion_sample(raw.sample_tick, raw.timestamp_ns, phys);
        let tyres = parse_tyres_sample(raw.sample_tick, raw.timestamp_ns, phys);
        let powertrain = parse_powertrain_sample(raw.sample_tick, raw.timestamp_ns, phys);
        let session = parse_session_sample(raw.sample_tick, raw.timestamp_ns, gfx);
        let timing = parse_timing_sample(raw.sample_tick, raw.timestamp_ns, gfx);
        let car_state = parse_car_state_sample(raw.sample_tick, raw.timestamp_ns, phys, gfx);
        let environment = parse_environment_sample(raw.sample_tick, raw.timestamp_ns, phys, gfx);
        let other_cars = parse_other_cars_sample(raw.sample_tick, raw.timestamp_ns, gfx);

        writer.write_frame(TelemetryFrame {
            sample_tick: raw.sample_tick, timestamp_ns: raw.timestamp_ns,
            controls, motion, tyres, powertrain, session, timing, car_state, environment, other_cars,
        })?;
    }
    let (_, summary) = writer.finish()?;
    Ok(summary)
}

// ---- Page parsing helpers ----

fn read_f32(bytes: &[u8], offset: usize) -> f32 { f32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap()) }
fn read_i32(bytes: &[u8], offset: usize) -> i32 { i32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap()) }
fn read_f32x4(bytes: &[u8], offset: usize) -> [f32; 4] {
    [read_f32(bytes, offset), read_f32(bytes, offset+4), read_f32(bytes, offset+8), read_f32(bytes, offset+12)]
}
fn read_f32x3(bytes: &[u8], offset: usize) -> [f32; 3] {
    [read_f32(bytes, offset), read_f32(bytes, offset+4), read_f32(bytes, offset+8)]
}
fn read_f32x5(bytes: &[u8], offset: usize) -> [f32; 5] {
    [read_f32(bytes, offset), read_f32(bytes, offset+4), read_f32(bytes, offset+8), read_f32(bytes, offset+12), read_f32(bytes, offset+16)]
}
fn read_f32x2(bytes: &[u8], offset: usize) -> [f32; 2] {
    [read_f32(bytes, offset), read_f32(bytes, offset+4)]
}
fn read_f32x12(bytes: &[u8], offset: usize) -> [f32; 12] {
    let mut out = [0f32; 12];
    for i in 0..12 { out[i] = read_f32(bytes, offset + i * 4); }
    out
}
fn read_u16_array<const N: usize>(bytes: &[u8], offset: usize) -> [u16; N] {
    let mut out = [0u16; N];
    for i in 0..N { out[i] = u16::from_le_bytes(bytes[offset+i*2..offset+i*2+2].try_into().unwrap()); }
    out
}
fn read_f32_vec(bytes: &[u8], offset: usize, count: usize) -> Vec<f32> {
    (0..count).map(|i| read_f32(bytes, offset + i * 4)).collect()
}
fn read_i32_vec(bytes: &[u8], offset: usize, count: usize) -> Vec<i32> {
    (0..count).map(|i| read_i32(bytes, offset + i * 4)).collect()
}

// ---- String / static helpers ----

fn read_u16_string<const N: usize>(bytes: &[u8], offset: usize) -> String {
    let arr: [u16; N] = read_u16_array::<N>(bytes, offset);
    let chars: Vec<u8> = arr.iter().filter_map(|c| if *c == 0 { None } else { Some((*c & 0xFF) as u8) }).collect();
    String::from_utf8_lossy(&chars).to_string()
}

fn parse_static_metadata(reader: &BinaryTelemetryReader, static_bytes: &[u8]) -> TelemetryResult<SessionMetadata> {
    let meta = reader.metadata();
    Ok(SessionMetadata {
        track_name: meta.track_name.clone(), car_model: meta.car_model.clone(),
        created_unix_ns: meta.created_unix_ns, poll_hz: meta.poll_hz, chunk_rows: meta.chunk_rows,
        sm_version: read_u16_string::<15>(static_bytes, 0),
        ac_version: read_u16_string::<15>(static_bytes, 30),
        number_of_sessions: read_i32(static_bytes, 60), num_cars: read_i32(static_bytes, 64),
    })
}

fn parse_controls_sample(tick: u64, ts: u64, phys: &[u8]) -> ControlSample {
    ControlSample {
        sample_tick: tick, timestamp_ns: ts,
        physics_packet_id: read_i32(phys, 0), graphics_packet_id: 0,
        speed_kmh: read_f32(phys, 28), gas: read_f32(phys, 4), brake: read_f32(phys, 8),
        clutch: read_f32(phys, 364), steer_angle: read_f32(phys, 24),
        gear: read_i32(phys, 16), rpms: read_i32(phys, 20), fuel: read_f32(phys, 12),
    }
}

fn parse_motion_sample(tick: u64, ts: u64, phys: &[u8]) -> MotionSample {
    MotionSample {
        sample_tick: tick, timestamp_ns: ts,
        velocity: read_f32x3(phys, 32), acc_g: read_f32x3(phys, 44),
        local_velocity: read_f32x3(phys, 568), local_angular_vel: read_f32x3(phys, 296),
        heading: read_f32(phys, 208), pitch: read_f32(phys, 212), roll: read_f32(phys, 216),
    }
}

fn parse_tyres_sample(tick: u64, ts: u64, phys: &[u8]) -> TyreSample {
    TyreSample {
        sample_tick: tick, timestamp_ns: ts,
        wheel_slip: read_f32x4(phys, 56), wheel_load: read_f32x4(phys, 72),
        wheels_pressure: read_f32x4(phys, 88), wheel_angular_speed: read_f32x4(phys, 104),
        tyre_wear: read_f32x4(phys, 120), tyre_dirty_level: read_f32x4(phys, 136),
        tyre_core_temperature: read_f32x4(phys, 152), camber_rad: read_f32x4(phys, 168),
        suspension_travel: read_f32x4(phys, 184), slip_ratio: read_f32x4(phys, 640),
        slip_angle: read_f32x4(phys, 656), tyre_temp_i: read_f32x4(phys, 368),
        tyre_temp_m: read_f32x4(phys, 384), tyre_temp_o: read_f32x4(phys, 400),
        tyre_temp: read_f32x4(phys, 696), mz: read_f32x4(phys, 592),
        fx: read_f32x4(phys, 608), fy: read_f32x4(phys, 624),
        suspension_damage: read_f32x4(phys, 680), brake_temp: read_f32x4(phys, 348),
        brake_pressure: read_f32x4(phys, 716), pad_life: read_f32x4(phys, 740),
        disc_life: read_f32x4(phys, 756),
        tyre_contact_point: read_f32x12(phys, 420),
        tyre_contact_normal: read_f32x12(phys, 468),
        tyre_contact_heading: read_f32x12(phys, 516),
        number_of_tyres_out: read_i32(phys, 244),
        front_brake_compound: read_i32(phys, 732),
        rear_brake_compound: read_i32(phys, 736),
    }
}

fn parse_powertrain_sample(tick: u64, ts: u64, phys: &[u8]) -> PowertrainSample {
    PowertrainSample {
        sample_tick: tick, timestamp_ns: ts,
        turbo_boost: read_f32(phys, 276), ballast: read_f32(phys, 280),
        kers_charge: read_f32(phys, 256), kers_input: read_f32(phys, 260),
        kers_current_kj: read_f32(phys, 336), drs: read_f32(phys, 200),
        tc: read_f32(phys, 204), abs: read_f32(phys, 252),
        engine_brake: read_i32(phys, 316), ers_recovery_level: read_i32(phys, 320),
        ers_power_level: read_i32(phys, 324), ers_heat_charging: read_i32(phys, 328),
        ers_is_charging: read_i32(phys, 332), drs_available: read_i32(phys, 340),
        drs_enabled: read_i32(phys, 344), tc_in_action: read_i32(phys, 672),
        abs_in_action: read_i32(phys, 676), auto_shifter_on: read_i32(phys, 264),
        current_max_rpm: read_i32(phys, 588), p2p_activations: read_i32(phys, 580),
        p2p_status: read_i32(phys, 584), water_temp: read_f32(phys, 712),
    }
}

fn parse_session_sample(tick: u64, ts: u64, gfx: &[u8]) -> SessionSample {
    SessionSample {
        sample_tick: tick, timestamp_ns: ts,
        status: read_i32(gfx, 4), session: read_i32(gfx, 8),
        session_index: read_i32(gfx, 1316), completed_laps: read_i32(gfx, 132),
        position: read_i32(gfx, 136), session_time_left: read_f32(gfx, 152),
        number_of_laps: read_i32(gfx, 172), current_sector_index: read_i32(gfx, 164),
        normalized_car_position: read_f32(gfx, 248),
        is_in_pit: read_i32(gfx, 160), is_in_pit_lane: read_i32(gfx, 1232),
        mandatory_pit_done: read_i32(gfx, 1240), missing_mandatory_pits: read_i32(gfx, 1484),
        penalty_time: read_f32(gfx, 1220), penalty_type: read_i32(gfx, 1224),
        track_status: read_u16_array::<33>(gfx, 1416),
        clock: read_f32(gfx, 1488), replay_time_multiplier: read_f32(gfx, 244),
        is_valid_lap: read_i32(gfx, 1408),
        global_yellow: read_i32(gfx, 1500), global_yellow1: read_i32(gfx, 1504),
        global_yellow2: read_i32(gfx, 1508), global_yellow3: read_i32(gfx, 1512),
        global_white: read_i32(gfx, 1516), global_green: read_i32(gfx, 1520),
        global_chequered: read_i32(gfx, 1524), global_red: read_i32(gfx, 1528),
        gap_ahead_or_tail_value: read_i32(gfx, 1580),
    }
}

fn parse_timing_sample(tick: u64, ts: u64, gfx: &[u8]) -> TimingSample {
    TimingSample {
        sample_tick: tick, timestamp_ns: ts,
        i_current_time: read_i32(gfx, 140), i_last_time: read_i32(gfx, 144),
        i_best_time: read_i32(gfx, 148), i_split: read_i32(gfx, 1404),
        last_sector_time: read_i32(gfx, 168), i_delta_lap_time: read_i32(gfx, 1356),
        is_delta_positive: read_i32(gfx, 1396), i_estimated_lap_time: read_i32(gfx, 1392),
        fuel_estimated_laps: read_f32(gfx, 1412), fuel_x_lap: read_f32(gfx, 1280),
        used_fuel: read_f32(gfx, 1320), distance_traveled: read_f32(gfx, 156),
        current_time_str: read_u16_array::<15>(gfx, 12),
        last_time_str: read_u16_array::<15>(gfx, 42),
        best_time_str: read_u16_array::<15>(gfx, 72),
        split_str: read_u16_array::<15>(gfx, 102),
        delta_lap_time_str: read_u16_array::<15>(gfx, 1324),
        estimated_lap_time_str: read_u16_array::<15>(gfx, 1360),
        observed_slot_before_i_split: read_i32(gfx, 1400),
    }
}

fn parse_car_state_sample(tick: u64, ts: u64, phys: &[u8], gfx: &[u8]) -> CarStateSample {
    CarStateSample {
        sample_tick: tick, timestamp_ns: ts,
        car_damage: read_f32x5(phys, 224), pit_limiter_on: read_i32(phys, 248),
        ride_height: read_f32x2(phys, 268), ignition_on: read_i32(phys, 772),
        starter_engine_on: read_i32(phys, 776), is_engine_running: read_i32(phys, 780),
        is_ai_controlled: read_i32(phys, 416), cg_height: read_f32(phys, 220),
        brake_bias: read_f32(phys, 564), rain_lights: read_i32(gfx, 1284),
        flashing_lights: read_i32(gfx, 1288), lights_stage: read_i32(gfx, 1292),
        wiper_lv: read_i32(gfx, 1300),
        driver_stint_total_time_left: read_i32(gfx, 1304),
        driver_stint_time_left: read_i32(gfx, 1308),
        rain_tyres: read_i32(gfx, 1312), current_tyre_set: read_i32(gfx, 1572),
        strategy_tyre_set: read_i32(gfx, 1576),
        track_grip_status: read_i32(gfx, 1556),
        tyre_compound_str: read_u16_array::<33>(gfx, 176),
        mfd_tyre_set: read_i32(gfx, 1532), mfd_fuel_to_add: read_f32(gfx, 1536),
        mfd_tyre_pressure: [
            read_f32(gfx, 1540), read_f32(gfx, 1544),
            read_f32(gfx, 1548), read_f32(gfx, 1552),
        ],
        ideal_line_on: read_i32(gfx, 1228),
        is_setup_menu_visible: read_i32(gfx, 1252),
        main_display_index: read_i32(gfx, 1256),
        secondary_display_index: read_i32(gfx, 1260),
        direction_lights_left: read_i32(gfx, 1492),
        direction_lights_right: read_i32(gfx, 1496),
        tc_level: read_i32(gfx, 1264), tc_cut: read_i32(gfx, 1268),
        engine_map: read_i32(gfx, 1272), abs_level: read_i32(gfx, 1276),
        exhaust_temperature: read_f32(gfx, 1296),
        final_ff: read_f32(phys, 308), performance_meter: read_f32(phys, 312),
        kerb_vibration: read_f32(phys, 784), slip_vibrations: read_f32(phys, 788),
        g_vibrations: read_f32(phys, 792), abs_vibrations: read_f32(phys, 796),
    }
}

fn parse_environment_sample(tick: u64, ts: u64, phys: &[u8], gfx: &[u8]) -> EnvironmentSample {
    EnvironmentSample {
        sample_tick: tick, timestamp_ns: ts,
        air_density: read_f32(phys, 284), air_temp: read_f32(phys, 288),
        road_temp: read_f32(phys, 292), wind_speed: read_f32(gfx, 1244),
        wind_direction: read_f32(gfx, 1248), surface_grip: read_f32(gfx, 1236),
        rain_intensity: read_i32(gfx, 1560),
        rain_intensity_in_10min: read_i32(gfx, 1564),
        rain_intensity_in_30min: read_i32(gfx, 1568),
    }
}

fn parse_other_cars_sample(tick: u64, ts: u64, gfx: &[u8]) -> OtherCarsSample {
    OtherCarsSample {
        sample_tick: tick, timestamp_ns: ts,
        active_cars: read_i32(gfx, 252), player_car_id: read_i32(gfx, 1216),
        car_coordinates: read_f32_vec(gfx, 256, 180),
        car_id: read_i32_vec(gfx, 976, 60),
    }
}
