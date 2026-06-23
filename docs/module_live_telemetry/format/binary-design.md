# ACC Live Telemetry Binary Format Design

> Status: design and implementation notes for ACTL (updated 2026-06-02).
> Target crate: `module_live_telemetry`
> File extensions: `.acctlm2` (v2, current) / `.acctlm` (v1, legacy)
> Byte-level format specs:
> - V1 (chunked cluster): [`v1-acctlm-format-spec.md`](v1-acctlm-format-spec.md)
> - V2 (columnar row-group): [`v2-acctlm2-format-spec.md`](v2-acctlm2-format-spec.md)

## 1. Design Direction

`module_live_telemetry` is a standalone Rust library plus a small CLI. The library owns shared-memory reading, recording, binary encoding, binary decoding, chunk indexing, and raw-page analysis helpers. The CLI is a test and operations entry point.

The recording format is ACTL: a chunked, clustered, columnar binary file.

- `chunked`: samples are buffered and written in bounded groups such as 256, 512, or 1024 rows.
- `clustered`: related fields live together, for example `controls` or `motion`.
- `columnar`: inside one chunk, each column is contiguous.
- `indexed`: a finished file has a footer index for fast chunk discovery.
- `schema-driven`: the file declares clusters and columns so future readers can skip unknown data.

The main goals are low CPU overhead during live recording, predictable file recovery after interruption, and fast selective reads.

## 2. Data Families

### ACTL Binary Files (`.acctlm2`)

9 clusters × 120 Hz per-frame decoding:

| Cluster | ID | Columns | Key Fields |
|---|---:|---:|---|
| Controls | 0x0100 | 12 | speed_kmh, gas, brake, gear, rpms, fuel |
| Motion | 0x0200 | 9 | velocity, acc_g, heading, pitch, roll |
| Tyres | 0x0300 | 31 | wheel_slip, tyre_wear, brake_temp, camber_rad |
| Powertrain | 0x0400 | 24 | turbo_boost, kers_charge, drs, engine_brake |
| Session | 0x0500 | 32 | normalized_car_position, is_valid_lap, flag, gap_behind, global flags |
| Timing | 0x0600 | 21 | i_current_time, i_last_time, i_best_time, i_split |
| CarState | 0x0700 | 42 | car_damage, pit_limiter, tc_level, abs_level, engine_map |
| Environment | 0x0800 | 11 | air_temp, road_temp, wind_speed, rain_intensity |
| OtherCars | 0x0900 | 6 | car_coordinates[60], car_id[60], active_cars |

Total: 188 columns per frame, 172 unique data fields.

### Raw Binary Files (`.accraw`)

Complete ACC shared memory pages per frame:

| Item | Size |
|---|---|
| File header | 28 bytes |
| Static page (once) | ~684 bytes |
| Per frame: tick + ns + physics + graphics | ~2380 bytes |

Supports re-parsing with updated struct definitions without re-recording.

## 3. Current Implementation

The crate currently implements:

| Area | Status | Main code |
|---|---|---|
| File header, schema, metadata, chunk headers, index, footer | Implemented | `src/format.rs` |
| Binary writer (all clusters) | Implemented | `src/writer.rs` |
| Binary reader and recovery scan | Implemented | `src/reader.rs` |
| Lap boundary detection and indexing | Implemented | `src/bin/acc-live-telemetry.rs` (`laps`, `build-lap-index`) |
| ACC Windows shared-memory reader | Implemented | `src/shmem.rs` |
| CLI commands | Implemented | `src/bin/acc-live-telemetry.rs` |
| More analysis clusters | Not implemented | Future work |
| Batch query API | Not implemented | Future work |
| Derived-value compute layer | Not implemented | Future work |

The `record` command writes row-grouped V2 telemetry into `.acctlm2` files. Legacy `.acctlm` V1 files remain readable, and `parse-raw` can still write V1 `.acctlm` files for compatibility. The `record-raw` command stores raw ACC shared-memory pages into `.accraw` files for later re-parsing.

## 4. Non-Goals for ACTL v2

- No per-sample text object format.
- No general-purpose compression in the live hot path.
- No derived lap-delta or driver-coaching values in raw recording chunks.
- No platform-specific assumptions in the binary reader. Only the shared-memory reader is Windows-specific.
- No schema churn for cosmetic field renames. Stable column IDs matter more than display names.

## 5. File Structure

```text
+------------------------------+
| FileHeader                   |
+------------------------------+
| SchemaBlock                  |
+------------------------------+
| MetadataBlock                |
+------------------------------+
| ChunkRecord 0                |
+------------------------------+
| ChunkRecord 1                |
+------------------------------+
| ...                          |
+------------------------------+
| ChunkIndex                   |
+------------------------------+
| FileFooter                   |
+------------------------------+
```

## 6. FileHeader Design

The header is fixed at 128 bytes so a reader can open the file with one small read. It contains:

- Magic and version checks.
- Offsets to schema, metadata, first chunk, and footer.
- Creation time.
- Timebase and poll rate.
- Reserved bytes for later flags or offsets.

`footer_offset = 0` is deliberate. It makes interrupted recordings recoverable: the reader scans valid `CHNK` records from `first_chunk_offset` and rebuilds the index in memory.

## 7. Metadata

Session-level information stored once per file in the Metadata Block. Backward-compatible with versioned extensions:

| Version | Fields | Size |
|---|---|---|
| v1 | track_name, car_model, poll_hz, chunk_rows | base |
| v2 | sm_version, ac_version, number_of_sessions, num_cars | +variable |
| v3 | sector_count, max_rpm, max_torque, max_power, max_fuel, penalties_enabled | +24 bytes |
| v4 | raw_static_bytes (full SPageFileStatic, 47 fields, ~684 bytes) | +variable |

## 8. Schema Design

ACTL v2 has a compact schema block:

```text
"SCHM"
schema_hash
cluster_count
repeat cluster:
  cluster_id
  column_count
  repeat column:
    column_id
    value_type
    name_len
    name_bytes
```

The current schema keeps only what the reader needs today. Future schema revisions can add:

- Units.
- Source kind: raw, minimal, derived, or external.
- Semantic source paths.
- Lane counts for vector columns.
- Codec hints.
- Deprecated-name aliases.

These additions should be made in a format version bump if they change the schema block layout.

## 9. Cluster Plan

All 9 clusters are implemented:

| Cluster ID | Name | Status | Purpose |
|---:|---|---|---|
| `0x0100` | `controls` | Implemented | Compact core driving controls (speed, gas, brake, gear, rpms, fuel) |
| `0x0200` | `motion` | Implemented | Velocity, acceleration, heading, pitch, roll |
| `0x0300` | `tyres` | Implemented | Slip, load, pressure, wear, temperatures per wheel |
| `0x0400` | `powertrain` | Implemented | Turbo boost, KERS, DRS, engine brake, water temp |
| `0x0500` | `session` | Implemented | Lap state, sectors, position, track status, flags, gap |
| `0x0600` | `timing` | Implemented | Lap times, split times, delta, fuel estimates |
| `0x0700` | `car_state` | Implemented | Damage, pit limiter, TC/ABS, engine map, ride height |
| `0x0800` | `environment` | Implemented | Air/road temp, wind, rain, surface grip |
| `0x0900` | `other_cars` | Implemented | Car coordinates, car IDs, active cars count |

Potential additions through format version bumps:

| Area | Description |
|---|---|
| Derived/driver coaching | Lap distance, reference delta, predicted lap time |
| User extensions | Plugin or experiment data (cluster ID range `0x8000..`)

## 10. Lap Index

Optional block appended after the file Footer. Enables O(1) random access to lap boundaries without scanning session data.

| Magic | Count | Per Lap Entry (32 bytes) |
|---|---|---|
| "LAPS" | u32 | lap_number(i32) + start_tick(u64) + end_tick(u64) + sample_count(u32) + is_valid(i32) + is_out_lap(i32) |

Auto-generated by `record` command. Manual: `build-lap-index --input <file>`.

## 11. Chunk Design

Each `ChunkRecord` owns one cluster. This makes selective reads cheap: loading core controls does not require touching raw page payloads, and loading tyres later will not require scanning unrelated columns.

The chunk header stores:

- Cluster and sequence number.
- Schema hash.
- Sample tick range.
- Time range.
- Optional lap range placeholders.
- Column directory length.
- Payload length and checksum.

The column directory stores offsets and lengths for each column. This lets a reader jump directly to one column inside a chunk.

## 12. Codec Strategy

ACTL v2 uses only codec `0`, plain little-endian bytes.

Future codecs can be added per column or per payload:

| Codec | Candidate use | Notes |
|---:|---|---|
| `0` | Plain little-endian | Current default |
| `1` | Boolean bitset | Good for flags |
| `2` | Delta integer stream | Good for monotonic timestamps and sample ticks |
| `3` | Small enum bytes | Good for status fields |
| `4` | Fixed-scale integers | Good when lossy quantization is explicitly acceptable |
| `5` | Block compression | Good for cold data, not default for live recording |

Lossless raw recording remains the baseline. Any lossy codec must be opt-in and documented per column.

## 13. Live Recording Flow

Recommended live flow:

1. Poll ACC shared memory at `poll_hz`.
2. Skip samples unless graphics status is `LIVE`.
3. De-duplicate physics frames with `packet_id`.
4. Append samples to per-cluster column buffers.
5. Flush a chunk when the cluster reaches `chunk_rows`.
6. Periodically flush pending samples into recoverable chunks and flush the file handle.
7. On finish, append index and footer, then rewrite the header with `footer_offset`.

Current `record` command writes all 9 telemetry clusters, and defaults to `--flush-interval-ms 2000`. Setting `--flush-interval-ms 0` disables periodic flush.

Recommended defaults:

| Setting | Default |
|---:|---|
| `poll_hz` | `120` |
| `chunk_rows` | `256` for CLI, `1024` for library default |
| `status_interval_ms` | `2000` |
| `flush_interval_ms` | `2000` |

## 14. Recovery Model

ACTL should remain useful after an unclean shutdown:

- If the footer exists, use its index.
- If the footer is missing, scan chunks from `first_chunk_offset`.
- If a scanned chunk header or payload is invalid, stop at the last good chunk.
- If a chunk is complete but payload CRC fails, report an invalid format error when decoding that chunk.

This keeps already-written complete chunks readable.

## 15. Reader API Direction

Current reader APIs are eager and simple:

```rust
pub fn read_all_controls(&self) -> TelemetryResult<Vec<ControlSample>>;
pub fn read_all_raw_graphics_samples(&self) -> TelemetryResult<Vec<RawGraphicsSample>>;
pub fn read_all_raw_graphics_pages(&self) -> TelemetryResult<Vec<RawGraphicsPageSample>>;
pub fn segment_raw_session(&self) -> TelemetryResult<RawSessionSegments>;
```

`segment_raw_session()` builds a session outline from raw graphics samples: metadata, session type, total sample range, and one `RawLapSegment` per detected lap. Each lap segment stores a half-open sample range `[start, end)`, ACC `completed_laps` range, tick range, time range, lap time when complete, validity when known, and the cluster set currently available for that lap.

Future reader APIs should support batch and column queries:

```rust
pub struct TelemetryQuery {
    pub time_range_ns: Option<std::ops::Range<u64>>,
    pub tick_range: Option<std::ops::RangeInclusive<u64>>,
    pub clusters: Vec<u16>,
    pub columns: Vec<u16>,
    pub batch_rows: usize,
}

pub struct ColumnBatch<'a> {
    pub cluster_id: u16,
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub sample_count: usize,
    pub columns: Vec<ColumnView<'a>>,
}

pub enum ColumnView<'a> {
    U64(&'a [u64]),
    F32(&'a [f32]),
    I32(&'a [i32]),
    Bytes {
        page_size: usize,
        bytes: &'a [u8],
    },
}
```

The reader should be able to skip irrelevant chunks and only decode requested columns.

## 16. Derived Data Direction

Derived values should not overwrite raw facts. They can be produced on read or stored in explicitly derived clusters.

Examples:

| Derived value | Inputs |
|---|---|
| `brakePct` | Raw `brake` clamped to `0..1` |
| `throttlePct` | Raw `gas` clamped to `0..1` |
| `steerRealAngle` | Raw `steerAngle` multiplied by car steering lock |
| `latG` | Raw `acc_g[0]` |
| `lonG` | Raw `acc_g[1]` |
| `yawRate` | Raw `local_angular_vel[1]` |
| `wheelSpeedKmh` | Raw `wheel_angular_speed` multiplied by `3.6` when the source unit is meters per second |

If derived clusters are persisted later, each cluster should record:

- Compute pipeline name.
- Compute pipeline version.
- Input schema hash.
- Input cluster IDs.
- Creation time.

## 17. Shared-Memory Boundary

The binary file format is cross-platform. The ACC shared-memory reader is Windows-only.

The shared-memory reader currently opens:

| Mapping | Use |
|---|---|
| `Local\acpmf_physics` | Controls, motion, tyres, brakes, raw physics page |
| `Local\acpmf_graphics` | Game status, lap/session values, raw graphics page |
| `Local\acpmf_static` | Track name, car model, raw static page |

For live recording, the graphics status controls whether samples are written:

| Status | Behavior |
|---|---|
| `Off` | Wait or finish an active recording |
| `Replay` | Wait or finish an active recording |
| `Live` | Record new physics packets |
| `Pause` | Keep file open and suspend sampling |
| `Unavailable` | Non-Windows or shared memory not open |

## 18. Testing Priorities

Current tests cover:

- Header/chunk roundtrip for compact controls.
- Multi-chunk roundtrip.
- CRC32 known vector.
- Stable chunk magic.
- Periodic flush emitting a recoverable partial raw chunk.
- Raw lap segmentation with synthetic data.
- Raw lap segmentation against the checked-in sample recording when present.

Recommended next tests:

- Footer-less recovery scan.
- Bad payload CRC detection.
- Unknown cluster skip behavior.
- Unknown column skip behavior.
- Metadata decoding edge cases.
- Empty recording behavior.

## 19. Implementation Roadmap

1. Add a footer-less recovery test.
2. Add query-by-cluster and query-by-time support.
3. Add selective column decoding for compact clusters.
4. Add `motion`, `tyres`, and `brakes` clusters if compact analysis files are needed.
5. Add a compute-on-read module for derived driver and lap values.
6. Add benchmarks for 30-minute 120 Hz recordings and selective reads.

The near-term priority should stay on faithful raw capture and reliable recovery. Once raw capture is dependable, compact analysis clusters and derived views can be layered on top without losing source detail.

## 20. Backward Compatibility

- New columns (`flag`, `gap_behind`) use `read_i32_column_opt` → missing in old files → default 0
- Metadata v3/v4 extensions detected by remaining byte count → old files → default values
- Lap index uses "LAPS" magic → old files without index → reader returns empty
- `.accraw` stores page sizes in header → parse-raw adapts to any ACC version

## 21. CLI Commands

| Command | Description |
|---|---|
| `record` | Record from ACC → `.acctlm2` (V2 footer includes lap index) |
| `record-raw` | Record ACC memory pages → `.accraw` (static page written once) |
| `parse-raw` | `.accraw` → `.acctlm` (re-parse with current struct) |
| `inspect` | View file metadata and chunk index |
| `export` | Export controls to CSV |
| `laps` | Scan `.acctlm` or `.acctlm2` for lap boundaries and print lap times |
| `build-lap-index` | Build V1 lap index for existing `.acctlm` files; `.acctlm2` already carries a V2 lap index |
| `serve` | Start dashboard HTTP server |
