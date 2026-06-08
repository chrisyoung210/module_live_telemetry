use crate::error::{TelemetryError, TelemetryResult};
use crate::format::{
    self, crc32, encode_schema, ChunkHeader, ColumnEntry, FileHeader, IndexEntry,
    CODEC_PLAIN_LE, CAR_STATE_COLUMNS, CLUSTER_CAR_STATE, CLUSTER_CONTROLS, CLUSTER_ENVIRONMENT,
    CLUSTER_MOTION, CLUSTER_OTHER_CARS, CLUSTER_POWERTRAIN, CLUSTER_SESSION, CLUSTER_TIMING,
    CLUSTER_TYRES, CONTROL_COLUMNS, ENVIRONMENT_COLUMNS, FOOTER_MAGIC, INDEX_MAGIC, META_MAGIC,
    MOTION_COLUMNS_EX, OTHER_CARS_COLUMNS, POWERTRAIN_COLUMNS, SCHEMA_HASH, SESSION_COLUMNS,
    TIMING_COLUMNS, TYRES_COLUMNS,
};
use crate::types::{
    CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
    PowertrainSample, RecordingSummary, SessionMetadata, SessionSample, TimingSample, TyreSample,
};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

// ---- Config ----
#[derive(Debug, Clone)]
pub struct LiveTelemetryConfig { pub poll_hz: f64, pub chunk_rows: usize }
impl Default for LiveTelemetryConfig {
    fn default() -> Self { Self { poll_hz: 120.0, chunk_rows: 1024 } }
}

// ---- Unified frame ----
#[derive(Debug, Clone)]
pub struct TelemetryFrame {
    pub sample_tick: u64, pub timestamp_ns: u64,
    pub controls: ControlSample, pub motion: MotionSample, pub tyres: TyreSample,
    pub powertrain: PowertrainSample, pub session: SessionSample, pub timing: TimingSample,
    pub car_state: CarStateSample, pub environment: EnvironmentSample,
    pub other_cars: OtherCarsSample,
}

impl TelemetryFrame {
    /// 按完整路径读取 raw 字段值（用于 raw:xxx.yyy 解析）
    ///
    /// 支持路径格式：
    /// - `controls.speed_kmh` → 子结构体字段
    /// - `motion.velocity[0]` → 数组元素
    /// - `sample_tick` → 顶层字段
    pub fn raw_field_value(&self, path: &str) -> Option<f64> {
        if let Some((substruct, field)) = path.split_once('.') {
            match substruct {
                "controls"    => self.controls.raw_field_value(field),
                "motion"      => self.motion.raw_field_value(field),
                "tyres"       => self.tyres.raw_field_value(field),
                "powertrain"  => self.powertrain.raw_field_value(field),
                "session"     => self.session.raw_field_value(field),
                "timing"      => self.timing.raw_field_value(field),
                "car_state"   => self.car_state.raw_field_value(field),
                "environment" => self.environment.raw_field_value(field),
                "other_cars"  => self.other_cars.raw_field_value(field),
                _ => None,
            }
        } else {
            // 顶层字段
            match path {
                "sample_tick"  => Some(self.sample_tick as f64),
                "timestamp_ns" => Some(self.timestamp_ns as f64),
                _ => None,
            }
        }
    }

    /// 验证 raw 字段路径是否有效
    pub fn is_raw_field(path: &str) -> bool {
        if let Some((substruct, field)) = path.split_once('.') {
            match substruct {
                "controls" => crate::types::ControlSample::raw_field_names().contains(&field),
                "motion" => crate::types::MotionSample::raw_field_names().contains(&field),
                "tyres" => crate::types::TyreSample::raw_field_names().contains(&field),
                "powertrain" => crate::types::PowertrainSample::raw_field_names().contains(&field),
                "session" => crate::types::SessionSample::raw_field_names().contains(&field),
                "timing" => crate::types::TimingSample::raw_field_names().contains(&field),
                "car_state" => crate::types::CarStateSample::raw_field_names().contains(&field),
                "environment" => crate::types::EnvironmentSample::raw_field_names().contains(&field),
                "other_cars" => crate::types::OtherCarsSample::raw_field_names().contains(&field),
                _ => false,
            }
        } else {
            matches!(path, "sample_tick" | "timestamp_ns")
        }
    }
}

// ---- Encode result ----
struct EncodeResult { payload: Vec<u8>, entries: Vec<ColumnEntry>, first_tick: u64, last_tick: u64, first_time: u64, last_time: u64, sample_count: u32 }

// ---- Writer ----
pub struct BinaryTelemetryWriter<W: Write + Seek> {
    writer: W, header: FileHeader, config: LiveTelemetryConfig,
    frames: Vec<TelemetryFrame>, index_entries: Vec<IndexEntry>,
    total_frames: u64, next_chunk_seq: u32, finished: bool,
    bytes_written: u64,
}impl BinaryTelemetryWriter<File> {
    pub fn create_file(path: impl AsRef<Path>, metadata: SessionMetadata, config: LiveTelemetryConfig) -> TelemetryResult<Self> {
        let file = File::create(path)?;
        Self::create(file, metadata, config)
    }
    /// Create a file for writing, failing if the file already exists.
    ///
    /// Uses `OpenOptions::create_new(true)` for exclusive creation.
    /// The existing `create_file()` method uses `File::create()` for
    /// backward-compatible overwrite behaviour (used by CLI).
    pub fn create_file_exclusive(path: impl AsRef<Path>, metadata: SessionMetadata, config: LiveTelemetryConfig) -> TelemetryResult<Self> {
        use std::fs::OpenOptions;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.as_ref())
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    TelemetryError::InvalidArgument(format!(
                        "output file already exists: {}",
                        path.as_ref().display()
                    ))
                } else {
                    TelemetryError::Io(e)
                }
            })?;
        Self::create(file, metadata, config)
    }
}
impl<W: Write + Seek> BinaryTelemetryWriter<W> {
    pub fn create(mut writer: W, mut metadata: SessionMetadata, config: LiveTelemetryConfig) -> TelemetryResult<Self> {
        if config.chunk_rows == 0 { return Err(TelemetryError::InvalidArgument("chunk_rows must be > 0".into())); }
        metadata.poll_hz = config.poll_hz;
        metadata.chunk_rows = config.chunk_rows;
        let schema = encode_schema();
        let meta_bytes = encode_metadata(&metadata);
        let mut header = FileHeader::new(metadata.created_unix_ns, config.poll_hz);
        header.metadata_offset = header.schema_offset + schema.len() as u64;
        header.first_chunk_offset = header.metadata_offset + meta_bytes.len() as u64;
        writer.seek(SeekFrom::Start(0))?;
        header.write_to(&mut writer)?;
        writer.write_all(&schema)?;
        writer.write_all(&meta_bytes)?;
        writer.seek(SeekFrom::Start(0))?;
        header.write_to(&mut writer)?;
        writer.seek(SeekFrom::Start(header.first_chunk_offset))?;
        Ok(Self { writer, header, config, frames: Vec::with_capacity(metadata.chunk_rows), index_entries: Vec::new(), total_frames: 0, next_chunk_seq: 0, finished: false, bytes_written: 0 })
    }
    pub fn write_frame(&mut self, frame: &TelemetryFrame) -> TelemetryResult<()> {
        if self.finished { return Err(TelemetryError::InvalidArgument("cannot write after finish".into())); }
        self.frames.push(frame.clone());
        self.total_frames = self.total_frames.saturating_add(1);
        if self.frames.len() >= self.config.chunk_rows { self.flush_all_clusters()?; }
        Ok(())
    }
    pub fn flush_to_disk(&mut self) -> TelemetryResult<()> {
        if !self.frames.is_empty() { self.flush_all_clusters()?; }
        self.writer.flush()?;
        self.bytes_written = self.writer.stream_position().unwrap_or(self.bytes_written);
        Ok(())
    }
    /// Approximate number of bytes written so far.
    ///
    /// Accurately updated on `flush_to_disk()` and `finish()`.
    /// Between flushes the value is a lower bound.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
    pub fn finish(mut self) -> TelemetryResult<(W, RecordingSummary)> {
        if !self.frames.is_empty() { self.flush_all_clusters()?; }
        let footer_offset = self.writer.stream_position()?;
        self.writer.write_all(&INDEX_MAGIC)?;
        self.writer.write_all(&(self.index_entries.len() as u64).to_le_bytes())?;
        for e in &self.index_entries { e.write_to(&mut self.writer)?; }
        self.writer.write_all(&FOOTER_MAGIC)?;
        self.writer.write_all(&footer_offset.to_le_bytes())?;
        self.writer.write_all(&self.total_frames.to_le_bytes())?;
        self.writer.write_all(&self.next_chunk_seq.to_le_bytes())?;
        self.writer.write_all(&[0u8; 4])?;
        let total_bytes = self.writer.stream_position()?;
        self.header.footer_offset = footer_offset;
        self.writer.seek(SeekFrom::Start(0))?;
        self.header.write_to(&mut self.writer)?;
        self.writer.flush()?;
        self.finished = true;
        self.bytes_written = total_bytes;
        Ok((self.writer, RecordingSummary { total_samples: self.total_frames, chunk_count: self.next_chunk_seq, total_bytes, footer_offset, duration: Duration::ZERO }))
    }
    // ---- flush ----
    fn flush_all_clusters(&mut self) -> TelemetryResult<()> {
        if self.frames.is_empty() { return Ok(()); }
        let frames = std::mem::take(&mut self.frames);
        self.frames.reserve(self.config.chunk_rows);
        let n = frames.len();
        let r = encode_controls_chunk(&frames, n); self.write_chunk(CLUSTER_CONTROLS, &r)?;
        let r = encode_motion_chunk(&frames, n); self.write_chunk(CLUSTER_MOTION, &r)?;
        let r = encode_tyres_chunk(&frames, n); self.write_chunk(CLUSTER_TYRES, &r)?;
        let r = encode_powertrain_chunk(&frames, n); self.write_chunk(CLUSTER_POWERTRAIN, &r)?;
        let r = encode_session_chunk(&frames, n); self.write_chunk(CLUSTER_SESSION, &r)?;
        let r = encode_timing_chunk(&frames, n); self.write_chunk(CLUSTER_TIMING, &r)?;
        let r = encode_car_state_chunk(&frames, n); self.write_chunk(CLUSTER_CAR_STATE, &r)?;
        let r = encode_environment_chunk(&frames, n); self.write_chunk(CLUSTER_ENVIRONMENT, &r)?;
        let r = encode_other_cars_chunk(&frames, n); self.write_chunk(CLUSTER_OTHER_CARS, &r)?;
        Ok(())
    }
    fn write_chunk(&mut self, cluster_id: u16, r: &EncodeResult) -> TelemetryResult<()> {
        let file_offset = self.writer.stream_position()?;
        let byte_len = (ChunkHeader::byte_len(r.entries.len()) + r.payload.len()) as u32;
        let payload_crc32 = crc32(&r.payload);
        let header = ChunkHeader {
            cluster_id, chunk_seq: self.next_chunk_seq, schema_hash: SCHEMA_HASH,
            base_sample_tick: r.first_tick, sample_stride: 1,
            sample_count: r.sample_count,
            start_time_ns: r.first_time, end_time_ns: r.last_time,
            start_lap: -1, end_lap: -1, column_count: r.entries.len() as u16,
            flags: 0, payload_len: r.payload.len() as u32, payload_crc32,
        };
        header.write_to(&mut self.writer)?;
        for entry in &r.entries { entry.write_to(&mut self.writer)?; }
        self.writer.write_all(&r.payload)?;
        self.index_entries.push(IndexEntry {
            cluster_id, chunk_seq: self.next_chunk_seq, file_offset, byte_len,
            start_time_ns: r.first_time, end_time_ns: r.last_time,
            start_tick: r.first_tick, end_tick: r.last_tick,
        });
        self.next_chunk_seq = self.next_chunk_seq.saturating_add(1);
        Ok(())
    }
}

// ---- Metadata encoding ----
pub(crate) fn encode_metadata(metadata: &SessionMetadata) -> Vec<u8> {
    let mut out = Vec::new();
    let track = metadata.track_name.as_bytes();
    let car = metadata.car_model.as_bytes();
    let sm = metadata.sm_version.as_bytes();
    let ac = metadata.ac_version.as_bytes();
    out.extend_from_slice(&META_MAGIC);
    out.extend_from_slice(&metadata.created_unix_ns.to_le_bytes());
    out.extend_from_slice(&format::hz_to_x1000(metadata.poll_hz).to_le_bytes());
    out.extend_from_slice(&(metadata.chunk_rows as u32).to_le_bytes());
    out.extend_from_slice(&(track.len() as u16).to_le_bytes());
    out.extend_from_slice(&(car.len() as u16).to_le_bytes());
    out.extend_from_slice(track); out.extend_from_slice(car);
    // v2 extended fields (appended after car for backward compat)
    out.extend_from_slice(&(sm.len() as u16).to_le_bytes());
    out.extend_from_slice(&(ac.len() as u16).to_le_bytes());
    out.extend_from_slice(&metadata.number_of_sessions.to_le_bytes());
out.extend_from_slice(&metadata.num_cars.to_le_bytes());
    out.extend_from_slice(sm); out.extend_from_slice(ac);
    // v3 extended static fields (after v2 for backward compat)
    out.extend_from_slice(&metadata.sector_count.to_le_bytes());
    out.extend_from_slice(&metadata.max_rpm.to_le_bytes());
    out.extend_from_slice(&metadata.max_torque.to_le_bytes());
    out.extend_from_slice(&metadata.max_power.to_le_bytes());
    out.extend_from_slice(&metadata.max_fuel.to_le_bytes());
    out.extend_from_slice(&metadata.penalties_enabled.to_le_bytes());
    // v4: raw static page bytes (backward compat: empty vec → skip)
    if !metadata.raw_static_bytes.is_empty() {
        out.extend_from_slice(&(metadata.raw_static_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&metadata.raw_static_bytes);
    }
    // v5: session_type (backward compatible, Option<i32>)
    if let Some(st) = metadata.session_type {
        out.extend_from_slice(&1i32.to_le_bytes());  // has_session_type flag
        out.extend_from_slice(&st.to_le_bytes());
    } else {
        out.extend_from_slice(&0i32.to_le_bytes());  // no session_type
    }
    out
}

// ---- Column entry builder ----
fn build_entries(columns: &[crate::format::ColumnSpec], n: usize, sizes: &[usize]) -> Vec<ColumnEntry> {
    let mut offset = 0u32;
    let mut entries = Vec::with_capacity(columns.len());
    for (i, col) in columns.iter().enumerate() {
        let item_size = sizes[i];
        let byte_len = (n * item_size) as u32;
        entries.push(ColumnEntry { column_id: col.id, codec: CODEC_PLAIN_LE, value_type: col.value_type, lane_count: 1, flags: 0, offset, byte_len, null_offset: 0, min_value: 0.0, max_value: 0.0 });
        offset = offset.saturating_add(byte_len);
    }
    entries
}

// ---- Controls encode ----
fn encode_controls_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let first_tick = frames[0].controls.sample_tick;
    let last_tick = frames[n-1].controls.sample_tick;
    let first_time = frames[0].controls.timestamp_ns;
    let last_time = frames[n-1].controls.timestamp_ns;
    let mut p = Vec::with_capacity(n * 52);
    for f in frames { p.extend_from_slice(&f.controls.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.timestamp_ns.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.physics_packet_id.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.graphics_packet_id.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.speed_kmh.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.gas.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.brake.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.clutch.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.steer_angle.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.gear.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.rpms.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.controls.fuel.to_le_bytes()); }
    let sizes = [8usize,8,4,4,4,4,4,4,4,4,4,4];
    EncodeResult { entries: build_entries(&CONTROL_COLUMNS, n, &sizes), payload: p, first_tick, last_tick, first_time, last_time, sample_count: n as u32 }
}

// ---- Motion encode ----
fn encode_motion_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let first_tick = frames[0].motion.sample_tick; let last_tick = frames[n-1].motion.sample_tick;
    let first_time = frames[0].motion.timestamp_ns; let last_time = frames[n-1].motion.timestamp_ns;
    let mut p = Vec::with_capacity(n * 76);
    for f in frames { p.extend_from_slice(&f.motion.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.motion.timestamp_ns.to_le_bytes()); }
    for f in frames { for v in &f.motion.velocity { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.motion.acc_g { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.motion.local_velocity { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.motion.local_angular_vel { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.motion.heading.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.motion.pitch.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.motion.roll.to_le_bytes()); }
    let sizes = [8usize,8,12,12,12,12,4,4,4];
    EncodeResult { entries: build_entries(&MOTION_COLUMNS_EX, n, &sizes), payload: p, first_tick, last_tick, first_time, last_time, sample_count: n as u32 }
}

// ---- Tyres encode ----
fn encode_tyres_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let ft = frames[0].tyres.sample_tick; let lt = frames[n-1].tyres.sample_tick;
    let fn_ = frames[0].tyres.timestamp_ns; let ln_ = frames[n-1].tyres.timestamp_ns;
    let mut p = Vec::with_capacity(n * 560);
    for f in frames { p.extend_from_slice(&f.tyres.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.tyres.timestamp_ns.to_le_bytes()); }
    for f in frames { for v in &f.tyres.wheel_slip { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.wheel_load { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.wheels_pressure { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.wheel_angular_speed { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_wear { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_dirty_level { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_core_temperature { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.camber_rad { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.suspension_travel { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.slip_ratio { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.slip_angle { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_temp_i { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_temp_m { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_temp_o { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_temp { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.mz { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.fx { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.fy { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.suspension_damage { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.brake_temp { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.brake_pressure { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.pad_life { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.disc_life { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_contact_point { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_contact_normal { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.tyres.tyre_contact_heading { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.tyres.number_of_tyres_out.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.tyres.front_brake_compound.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.tyres.rear_brake_compound.to_le_bytes()); }
    let sizes: [usize;31] = [8,8, 16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16,16, 48,48,48, 4,4,4];
    EncodeResult { entries: build_entries(&TYRES_COLUMNS, n, &sizes), payload: p, first_tick: ft, last_tick: lt, first_time: fn_, last_time: ln_, sample_count: n as u32 }
}

// ---- Powertrain encode ----
fn encode_powertrain_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let ft = frames[0].powertrain.sample_tick; let lt = frames[n-1].powertrain.sample_tick;
    let fn_ = frames[0].powertrain.timestamp_ns; let ln_ = frames[n-1].powertrain.timestamp_ns;
    let mut p = Vec::with_capacity(n * 104);
    for f in frames { p.extend_from_slice(&f.powertrain.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.timestamp_ns.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.turbo_boost.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.ballast.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.kers_charge.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.kers_input.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.kers_current_kj.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.drs.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.tc.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.abs.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.engine_brake.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.ers_recovery_level.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.ers_power_level.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.ers_heat_charging.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.ers_is_charging.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.drs_available.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.drs_enabled.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.tc_in_action.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.abs_in_action.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.auto_shifter_on.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.current_max_rpm.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.p2p_activations.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.p2p_status.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.powertrain.water_temp.to_le_bytes()); }
    let sizes: [usize;24] = [8,8,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4];
    EncodeResult { entries: build_entries(&POWERTRAIN_COLUMNS, n, &sizes), payload: p, first_tick: ft, last_tick: lt, first_time: fn_, last_time: ln_, sample_count: n as u32 }
}

// ---- Session encode ----
fn encode_session_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let ft = frames[0].session.sample_tick; let lt = frames[n-1].session.sample_tick;
    let fn_ = frames[0].session.timestamp_ns; let ln_ = frames[n-1].session.timestamp_ns;
    let mut p = Vec::with_capacity(n * 200);
    for f in frames { p.extend_from_slice(&f.session.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.timestamp_ns.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.status.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.session.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.session_index.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.completed_laps.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.position.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.session_time_left.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.number_of_laps.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.current_sector_index.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.normalized_car_position.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.is_in_pit.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.is_in_pit_lane.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.mandatory_pit_done.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.missing_mandatory_pits.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.penalty_time.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.penalty_type.to_le_bytes()); }
    for f in frames { for v in &f.session.track_status { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.session.clock.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.replay_time_multiplier.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.is_valid_lap.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_yellow.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_yellow1.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_yellow2.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_yellow3.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_white.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_green.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_chequered.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.global_red.to_le_bytes()); }
for f in frames { p.extend_from_slice(&f.session.gap_ahead_or_tail_value.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.flag.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.session.gap_behind.to_le_bytes()); }
    let sizes: [usize;32] = [8,8,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,66,4,4,4,4,4,4,4,4,4,4,4,4,4,4];
    EncodeResult { entries: build_entries(&SESSION_COLUMNS, n, &sizes), payload: p, first_tick: ft, last_tick: lt, first_time: fn_, last_time: ln_, sample_count: n as u32 }
}

// ---- Timing encode ----
fn encode_timing_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let ft = frames[0].timing.sample_tick; let lt = frames[n-1].timing.sample_tick;
    let fn_ = frames[0].timing.timestamp_ns; let ln_ = frames[n-1].timing.timestamp_ns;
    let mut p = Vec::with_capacity(n * 272);
    for f in frames { p.extend_from_slice(&f.timing.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.timestamp_ns.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.i_current_time.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.i_last_time.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.i_best_time.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.i_split.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.last_sector_time.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.i_delta_lap_time.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.is_delta_positive.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.i_estimated_lap_time.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.fuel_estimated_laps.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.fuel_x_lap.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.used_fuel.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.timing.distance_traveled.to_le_bytes()); }
    for f in frames { for v in &f.timing.current_time_str { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.timing.last_time_str { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.timing.best_time_str { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.timing.split_str { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.timing.delta_lap_time_str { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { for v in &f.timing.estimated_lap_time_str { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.timing.observed_slot_before_i_split.to_le_bytes()); }
    let sizes: [usize;21] = [8,8,4,4,4,4,4,4,4,4,4,4,4,4,30,30,30,30,30,30,4];
    EncodeResult { entries: build_entries(&TIMING_COLUMNS, n, &sizes), payload: p, first_tick: ft, last_tick: lt, first_time: fn_, last_time: ln_, sample_count: n as u32 }
}

// ---- Car State encode ----
fn encode_car_state_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let ft = frames[0].car_state.sample_tick; let lt = frames[n-1].car_state.sample_tick;
    let fn_ = frames[0].car_state.timestamp_ns; let ln_ = frames[n-1].car_state.timestamp_ns;
    let mut p = Vec::with_capacity(n * 280);
    for f in frames { p.extend_from_slice(&f.car_state.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.timestamp_ns.to_le_bytes()); }
    for f in frames { for v in &f.car_state.car_damage { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.car_state.pit_limiter_on.to_le_bytes()); }
    for f in frames { for v in &f.car_state.ride_height { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.car_state.ignition_on.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.starter_engine_on.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.is_engine_running.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.is_ai_controlled.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.cg_height.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.brake_bias.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.rain_lights.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.flashing_lights.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.lights_stage.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.wiper_lv.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.driver_stint_total_time_left.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.driver_stint_time_left.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.rain_tyres.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.current_tyre_set.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.strategy_tyre_set.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.track_grip_status.to_le_bytes()); }
    for f in frames { for v in &f.car_state.tyre_compound_str { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.car_state.mfd_tyre_set.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.mfd_fuel_to_add.to_le_bytes()); }
    for f in frames { for v in &f.car_state.mfd_tyre_pressure { p.extend_from_slice(&v.to_le_bytes()); } }
    for f in frames { p.extend_from_slice(&f.car_state.ideal_line_on.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.is_setup_menu_visible.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.main_display_index.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.secondary_display_index.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.direction_lights_left.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.direction_lights_right.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.tc_level.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.tc_cut.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.engine_map.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.abs_level.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.exhaust_temperature.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.final_ff.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.performance_meter.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.kerb_vibration.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.slip_vibrations.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.g_vibrations.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.car_state.abs_vibrations.to_le_bytes()); }
    let sizes: [usize;42] = [8,8,20,4,8,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,66,4,4,16,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4];
    EncodeResult { entries: build_entries(&CAR_STATE_COLUMNS, n, &sizes), payload: p, first_tick: ft, last_tick: lt, first_time: fn_, last_time: ln_, sample_count: n as u32 }
}

// ---- Environment encode ----
fn encode_environment_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let ft = frames[0].environment.sample_tick; let lt = frames[n-1].environment.sample_tick;
    let fn_ = frames[0].environment.timestamp_ns; let ln_ = frames[n-1].environment.timestamp_ns;
    let mut p = Vec::with_capacity(n * 52);
    for f in frames { p.extend_from_slice(&f.environment.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.timestamp_ns.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.air_density.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.air_temp.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.road_temp.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.wind_speed.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.wind_direction.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.surface_grip.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.rain_intensity.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.rain_intensity_in_10min.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.environment.rain_intensity_in_30min.to_le_bytes()); }
    let sizes: [usize;11] = [8,8,4,4,4,4,4,4,4,4,4];
    EncodeResult { entries: build_entries(&ENVIRONMENT_COLUMNS, n, &sizes), payload: p, first_tick: ft, last_tick: lt, first_time: fn_, last_time: ln_, sample_count: n as u32 }
}

// ---- Other Cars encode ----
fn encode_other_cars_chunk(frames: &[TelemetryFrame], n: usize) -> EncodeResult {
    let ft = frames[0].other_cars.sample_tick; let lt = frames[n-1].other_cars.sample_tick;
    let fn_ = frames[0].other_cars.timestamp_ns; let ln_ = frames[n-1].other_cars.timestamp_ns;
    let mut p = Vec::with_capacity(n * 984);
    for f in frames { p.extend_from_slice(&f.other_cars.sample_tick.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.other_cars.timestamp_ns.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.other_cars.active_cars.to_le_bytes()); }
    for f in frames { p.extend_from_slice(&f.other_cars.player_car_id.to_le_bytes()); }
    for f in frames {
        let s = f.other_cars.car_coordinates.as_slice();
        for v in s.iter().take(180) { p.extend_from_slice(&v.to_le_bytes()); }
        for _ in s.len()..180 { p.extend_from_slice(&0f32.to_le_bytes()); }
    }
    for f in frames {
        let s = f.other_cars.car_id.as_slice();
        for v in s.iter().take(60) { p.extend_from_slice(&v.to_le_bytes()); }
        for _ in s.len()..60 { p.extend_from_slice(&0i32.to_le_bytes()); }
    }
    let sizes: [usize;6] = [8,8,4,4,720,240];
    EncodeResult { entries: build_entries(&OTHER_CARS_COLUMNS, n, &sizes), payload: p, first_tick: ft, last_tick: lt, first_time: fn_, last_time: ln_, sample_count: n as u32 }
}
