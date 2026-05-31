# ACC Live Telemetry Binary Format Design

> Status: design and implementation notes for ACTL v1.
> Target crate: `module_live_telemetry`
> File extension: `.acctlm`

## 1. Design Direction

`module_live_telemetry` is a standalone Rust library plus a small CLI. The library owns shared-memory reading, recording, binary encoding, binary decoding, chunk indexing, and raw-page analysis helpers. The CLI is a test and operations entry point.

The recording format is ACTL: a chunked, clustered, columnar binary file.

- `chunked`: samples are buffered and written in bounded groups such as 256, 512, or 1024 rows.
- `clustered`: related fields live together, for example `controls` or `raw_pages`.
- `columnar`: inside one chunk, each column is contiguous.
- `indexed`: a finished file has a footer index for fast chunk discovery.
- `schema-driven`: the file declares clusters and columns so future readers can skip unknown data.

The main goals are low CPU overhead during live recording, predictable file recovery after interruption, and fast selective reads.

## 2. Current Implementation

The crate currently implements:

| Area | Status | Main code |
|---|---|---|
| File header, schema, metadata, chunk headers, index, footer | Implemented | `src/format.rs` |
| Compact controls writer | Implemented | `src/writer.rs` |
| Raw shared-memory page writer | Implemented | `src/raw_writer.rs` |
| Binary reader and recovery scan | Implemented | `src/reader.rs` |
| Raw session/lap segmentation | Implemented | `src/laps.rs` |
| ACC Windows shared-memory reader | Implemented | `src/shmem.rs` |
| CLI commands | Implemented | `src/bin/acc-live-telemetry.rs` |
| More analysis clusters | Not implemented | Future work |
| Batch query API | Not implemented | Future work |
| Derived-value compute layer | Not implemented | Future work |

The active live recorder, `record-auto`, writes `raw_pages` chunks. This intentionally preserves the highest-fidelity source data first; more compact analysis views can be produced later from the raw pages.

## 3. Non-Goals for ACTL v1

- No per-sample text object format.
- No general-purpose compression in the live hot path.
- No derived lap-delta or driver-coaching values in raw recording chunks.
- No platform-specific assumptions in the binary reader. Only the shared-memory reader is Windows-specific.
- No schema churn for cosmetic field renames. Stable column IDs matter more than display names.

## 4. File Structure

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

See `DATA_PROTOCOL.md` for the byte-level protocol. This document focuses on design choices and extension plans.

## 5. FileHeader Design

The header is fixed at 128 bytes so a reader can open the file with one small read. It contains:

- Magic and version checks.
- Offsets to schema, metadata, first chunk, and footer.
- Creation time.
- Timebase and poll rate.
- Reserved bytes for later flags or offsets.

`footer_offset = 0` is deliberate. It makes interrupted recordings recoverable: the reader scans valid `CHNK` records from `first_chunk_offset` and rebuilds the index in memory.

## 6. Schema Design

ACTL v1 has a compact schema block:

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

## 7. Cluster Plan

Current cluster IDs:

| Cluster ID | Name | Status | Purpose |
|---:|---|---|---|
| `0x0100` | `controls` | Implemented | Compact core driving controls |
| `0x0200` | `raw_pages` | Implemented | Byte-for-byte ACC physics, graphics, and static pages |

Potential future clusters:

| Cluster ID | Name | Source | Typical fields |
|---:|---|---|---|
| `0x0110` | `motion` | Raw physics | Velocity, acceleration, local angular velocity, heading, pitch, roll |
| `0x0120` | `tyres` | Raw physics | Slip, load, pressure, angular speed, wear, temperatures |
| `0x0130` | `brakes` | Raw physics | Brake pressure, brake temperature, brake bias, pad life, disc life |
| `0x0140` | `powertrain` | Raw physics | Turbo boost, water temperature, engine map, max RPM, engine state |
| `0x0150` | `assists_status` | Raw physics and graphics | TC, ABS, intervention flags, pit limiter, tyres out |
| `0x0160` | `session_track` | Raw graphics and static | Lap state, sectors, position, track status, weather |
| `0x0170` | `damage` | Raw physics | Body and suspension damage |
| `0x0180` | `contact_patch` | Raw physics | Tyre contact point, normal, and heading vectors |
| `0x0300` | `derived_driver` | Computed | Throttle percentage, brake percentage, steering degrees, wheel speeds |
| `0x0310` | `derived_lap` | Computed | Lap distance, reference delta, predicted lap time |
| `0x8000..` | user extensions | External | Experiment or plugin data |

Design rule: raw clusters should preserve source facts. Derived clusters should clearly name their compute pipeline and inputs.

## 8. Chunk Design

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

## 9. Codec Strategy

ACTL v1 uses only codec `0`, plain little-endian bytes.

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

## 10. Live Recording Flow

Recommended live flow:

1. Poll ACC shared memory at `poll_hz`.
2. Skip samples unless graphics status is `LIVE`.
3. De-duplicate physics frames with `packet_id`.
4. Append samples to per-cluster column buffers.
5. Flush a chunk when the cluster reaches `chunk_rows`.
6. Periodically flush pending samples into recoverable chunks and flush the file handle.
7. On finish, append index and footer, then rewrite the header with `footer_offset`.

Current `record-auto` follows the same shape, writes only the `raw_pages` cluster, and defaults to `--flush-interval-ms 2000`. Setting `--flush-interval-ms 0` disables periodic flush.

Recommended defaults:

| Setting | Default |
|---|---:|
| `poll_hz` | `120` |
| `raw_pages.chunk_rows` | `256` |
| `controls.chunk_rows` | `256` for CLI mock generation, `1024` for library default |
| `status_interval_ms` | `2000` |
| `flush_interval_ms` | `2000` |

## 11. Recovery Model

ACTL should remain useful after an unclean shutdown:

- If the footer exists, use its index.
- If the footer is missing, scan chunks from `first_chunk_offset`.
- If a scanned chunk header or payload is invalid, stop at the last good chunk.
- If a chunk is complete but payload CRC fails, report an invalid format error when decoding that chunk.

This keeps already-written complete chunks readable.

## 12. Reader API Direction

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

## 13. Derived Data Direction

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

## 14. Shared-Memory Boundary

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

## 15. Testing Priorities

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

## 16. Implementation Roadmap

1. Add a footer-less recovery test.
2. Add query-by-cluster and query-by-time support.
3. Add selective column decoding for compact clusters.
4. Add `motion`, `tyres`, and `brakes` clusters if compact analysis files are needed.
5. Add a compute-on-read module for derived driver and lap values.
6. Add benchmarks for 30-minute 120 Hz recordings and selective reads.

The near-term priority should stay on faithful raw capture and reliable recovery. Once raw capture is dependable, compact analysis clusters and derived views can be layered on top without losing source detail.
