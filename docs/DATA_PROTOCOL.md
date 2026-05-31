# ACC Live Telemetry Binary Protocol v1

> Status: current protocol for `module_live_telemetry`.
> Extension: `.acctlm`
> Format name: ACTL, short for ACC Coach Telemetry Log.

## 1. Scope

This project stores telemetry only as ACTL binary files. The recording hot path writes structured binary blocks with fixed little-endian fields, chunk payload checksums, and a footer index. Future recording formats should extend ACTL with new clusters, columns, or schema versions instead of adding another storage format.

The current implementation supports two data families:

| Cluster | ID | Writer | Reader API | Purpose |
|---|---:|---|---|---|
| `controls` | `0x0100` | `BinaryTelemetryWriter` | `read_all_controls()` | Compact driving controls and core car state |
| `raw_pages` | `0x0200` | `RawPageTelemetryWriter` | `read_all_raw_graphics_samples()`, `read_all_raw_graphics_pages()` | Byte-for-byte ACC shared-memory pages |

`record-auto` currently records the `raw_pages` cluster. `generate-mock` records the compact `controls` cluster.

## 2. Encoding Rules

- All integer and floating-point values are little-endian.
- Strings in protocol blocks are UTF-8 bytes with explicit lengths.
- Timestamps are unsigned nanoseconds.
- `poll_hz` is stored as `poll_hz_x1000`, for example `120.0 Hz` is `120000`.
- Raw ACC shared-memory pages are preserved byte-for-byte.
- The hot path writes complete chunks. A finished file adds a footer index; an unfinished file can still be recovered by scanning chunks from `first_chunk_offset`.
- Every chunk payload has a CRC32 checksum using the standard polynomial implemented in `format::crc32`.

## 3. File Layout

```text
+------------------------------+
| FileHeader                   | fixed 128 bytes
+------------------------------+
| SchemaBlock                  | starts at header.schema_offset
+------------------------------+
| MetadataBlock                | starts at header.metadata_offset
+------------------------------+
| ChunkRecord 0                | starts at header.first_chunk_offset
+------------------------------+
| ChunkRecord 1                |
+------------------------------+
| ...                          |
+------------------------------+
| ChunkIndex                   | starts at header.footer_offset after finish
+------------------------------+
| FileFooter                   |
+------------------------------+
```

`footer_offset = 0` means the file was not finalized. The reader then scans sequential `CHNK` records until it reaches a non-chunk block or end of file.

## 4. FileHeader

The file starts with a 128-byte header.

| Field | Type | Description |
|---|---|---|
| `magic` | `[u8; 8]` | `ACTL\r\n\x1A\n` |
| `version` | `u16` | Current value: `1` |
| `header_size` | `u16` | Current value: `128` |
| `flags` | `u32` | Reserved, currently `0` |
| `schema_offset` | `u64` | Offset of `SchemaBlock`; current value is `128` |
| `metadata_offset` | `u64` | Offset of `MetadataBlock` |
| `first_chunk_offset` | `u64` | Offset of first `ChunkRecord` |
| `footer_offset` | `u64` | Offset of `ChunkIndex`; `0` while recording or after an unclean shutdown |
| `created_unix_ns` | `u64` | Creation time in Unix nanoseconds |
| `timebase_hz` | `u32` | Current value: `1_000_000_000` |
| `poll_hz_x1000` | `u32` | Polling rate multiplied by `1000` |
| `reserved` | `[u8; 64]` | Must be zero when written; readers ignore it |

## 5. SchemaBlock

The schema block lets a reader discover which clusters and columns are present.

```text
schema_magic      [u8; 4]  "SCHM"
schema_hash       u64      current value: 0x4143544c00000001
cluster_count     u16
repeat cluster_count:
  cluster_id      u16
  column_count    u16
  repeat column_count:
    column_id     u16
    value_type    u8
    name_len      u8
    name_bytes    [name_len] UTF-8
```

Current value types:

| Value | Name | Meaning |
|---:|---|---|
| `1` | `u64` | Unsigned 64-bit integer |
| `2` | `f32` | 32-bit float |
| `3` | `i32` | Signed 32-bit integer |
| `4` | `bytes` | Fixed-size byte pages inside a chunk |

Current schema hash is fixed by `format::SCHEMA_HASH`. Increment it when published column IDs, value types, or cluster semantics change incompatibly.

## 6. MetadataBlock

```text
metadata_magic    [u8; 4]  "META"
created_unix_ns   u64
poll_hz_x1000     u32
chunk_rows        u32
track_len         u16
car_len           u16
track_name        [track_len] UTF-8
car_model         [car_len] UTF-8
```

`track_name` and `car_model` are taken from ACC static shared memory when available. Empty or unknown shared-memory values are normalized to fallback names by the shared-memory reader.

## 7. ChunkRecord

Each chunk belongs to exactly one cluster. Payloads are columnar: all values for column A are stored contiguously, then all values for column B, and so on.

```text
chunk_magic       [u8; 4]  "CHNK"
header_size       u16      current value: 72
cluster_id        u16
chunk_seq         u32
schema_hash       u64
base_sample_tick  u64
sample_stride     u32
sample_count      u32
start_time_ns     u64
end_time_ns       u64
start_lap         i32      currently -1 when not populated
end_lap           i32      currently -1 when not populated
column_count      u16
flags             u16      reserved, currently 0
payload_len       u32
payload_crc32     u32
repeat column_count:
  ColumnEntry
payload           [payload_len]
```

`sample_stride` is inferred from the first two sample ticks in the chunk. If the chunk has fewer than two samples, it is `1`.

### ColumnEntry

Each column entry is 40 bytes.

| Field | Type | Description |
|---|---|---|
| `column_id` | `u16` | Stable column identifier |
| `codec` | `u8` | Current value: `0`, plain little-endian |
| `value_type` | `u8` | One of the schema value types |
| `lane_count` | `u8` | Current value: `1` |
| `flags` | `u8` | Reserved, currently `0` |
| `offset` | `u32` | Byte offset inside payload |
| `byte_len` | `u32` | Byte length of this column in payload |
| `null_offset` | `u32` | Current value: `0`; reserved for future null bitmaps |
| `min_value` | `f64` | Numeric min for the chunk, or `0` for byte pages |
| `max_value` | `f64` | Numeric max for the chunk, or `0` for byte pages |
| `reserved` | `[u8; 6]` | Must be zero when written; readers ignore it |

## 8. `controls` Cluster

Cluster ID: `0x0100`

The compact controls cluster stores a small, analysis-friendly subset of telemetry.

| Column ID | Name | Type | Source |
|---:|---|---|---|
| `1` | `sampleTick` | `u64` | Recorder sample counter |
| `2` | `timestampNs` | `u64` | Recorder monotonic timestamp |
| `10` | `speedKmh` | `f32` | ACC physics `speed_kmh` |
| `11` | `gas` | `f32` | ACC physics `gas` |
| `12` | `brake` | `f32` | ACC physics `brake` |
| `13` | `clutch` | `f32` | ACC physics `clutch` |
| `14` | `steerAngle` | `f32` | ACC physics `steer_angle` raw ratio |
| `15` | `gear` | `i32` | ACC physics `gear` |
| `16` | `rpms` | `i32` | ACC physics `rpms` |
| `17` | `fuel` | `f32` | ACC physics `fuel` |

Payload order:

```text
sampleTick[0..count)     u64
timestampNs[0..count)    u64
speedKmh[0..count)       f32
gas[0..count)            f32
brake[0..count)          f32
clutch[0..count)         f32
steerAngle[0..count)     f32
gear[0..count)           i32
rpms[0..count)           i32
fuel[0..count)           f32
```

The current implementation does not clamp or normalize these values before writing.

## 9. `raw_pages` Cluster

Cluster ID: `0x0200`

This cluster preserves the ACC shared-memory pages for later decoding and re-interpretation.

| Column ID | Name | Type | Source |
|---:|---|---|---|
| `1001` | `sampleTick` | `u64` | Recorder sample counter |
| `1002` | `timestampNs` | `u64` | Recorder monotonic timestamp |
| `1003` | `rawPhysicsPage` | `bytes` | `Local\acpmf_physics` |
| `1004` | `rawGraphicsPage` | `bytes` | `Local\acpmf_graphics` |
| `1005` | `rawStaticPage` | `bytes` | `Local\acpmf_static` |

Payload order:

```text
sampleTick[0..count)                 u64
timestampNs[0..count)                u64
rawPhysicsPage[0..count)             fixed bytes per sample
rawGraphicsPage[0..count)            fixed bytes per sample
rawStaticPage[0..count)              fixed bytes per sample
```

The writer requires stable page sizes for the whole file. On Windows, current page sizes are:

| Page | Bytes | Flattened fields | Notes |
|---|---:|---:|---|
| Physics | `800` | `200` | Full `SPageFilePhysicsControls` prefix used by the project |
| Graphics | `1584` | `472` | Full `SPageFileGraphicsRaw` prefix used by the project |
| Static | `200` | `98` | Static identity prefix used for track and car metadata |

See `RAW_TELEMETRY_FIELDS.md` for field offsets inside these raw pages.

## 10. ChunkIndex and FileFooter

After all chunks have been written, `finish()` appends an index and footer.

```text
index_magic       [u8; 4]  "INDX"
entry_count       u64
repeat entry_count:
  cluster_id      u16
  reserved        u16
  chunk_seq       u32
  file_offset     u64
  byte_len        u32
  reserved        u32
  start_time_ns   u64
  end_time_ns     u64
  start_tick      u64
  end_tick        u64
footer_magic      [u8; 4]  "FOOT"
index_offset      u64
total_samples     u64
chunk_count       u32
reserved          u32
```

The header is then rewritten with `footer_offset = index_offset`.

## 11. Shared-Memory Recording Rules

`record-auto` uses the Windows shared-memory reader:

1. Open physics, graphics, and static shared-memory mappings.
2. Wait until ACC graphics status is `LIVE`.
3. Start a new `.acctlm` file with track and car metadata from the static page.
4. Record one raw sample per new physics `packet_id`.
5. While ACC is `PAUSE`, keep the file open but do not append samples.
6. On session end, shared-memory loss, or read failure after recording starts, finish the file and write the footer index.

Default CLI settings:

| Setting | Default |
|---|---:|
| `poll_hz` | `120.0` |
| `chunk_rows` for `record-auto` | `256` |
| `chunk_rows` for `generate-mock` | `256` |
| `flush_interval_ms` for `record-auto` | `2000` |
| Output directory | `.\data` |

`record-auto` periodically calls `flush_to_disk()`, which emits any pending raw samples as a recoverable chunk and flushes the file handle. Use `--flush-interval-ms 0` to disable periodic flush.

Default live recording file name:

```text
live_{unix_secs}_{track}_{car}.acctlm
```

## 12. Reader Behavior

`BinaryTelemetryReader::from_bytes` validates:

- ACTL magic.
- Supported format version.
- Header size.
- Header offsets.
- Schema block magic and schema hash.
- Metadata block magic.
- Footer index when `footer_offset > 0`.
- Chunk payload CRC before decoding a chunk.

If the footer is missing, the reader scans chunks starting at `first_chunk_offset` and builds an in-memory index from valid chunk headers.

## 13. Session and Lap Segmentation

The reader can derive a session outline from the raw graphics page stream:

```rust
pub fn segment_raw_session(&self) -> TelemetryResult<RawSessionSegments>;
```

`RawSessionSegments` contains:

| Field | Meaning |
|---|---|
| `metadata` | Track, car, creation time, poll rate, and chunk rows |
| `session_type` | Raw graphics `session` value from offset `8` |
| `session_kind` | Best-effort label such as `practice`, `qualify`, `race`, or `hotlap` |
| `sample_count` | Number of decoded raw graphics samples |
| `start_time_ns` / `end_time_ns` | Session sample time range |
| `laps` | One `RawLapSegment` per detected lap or partial lap |

Each `RawLapSegment` stores a half-open sample range `[start_sample_index, end_sample_index)`, ACC `completed_laps` range, tick range, time range, lap completion reason, lap time when complete, validity when known, normalized position range, distance range, and the dynamic clusters available for that lap. The current implementation has `raw_pages` as the available dynamic cluster; future compact clusters can use the same tick/time ranges.

Lap boundaries are detected from raw graphics samples by:

- `current_lap_time_ms` resetting from more than `10_000 ms` to less than `2_000 ms`.
- `completed_laps` changing when no nearby reset boundary was already detected.

Adjacent reset/completed-lap changes are de-duplicated so one finish line crossing produces one segment boundary.

## 14. CLI Commands

```powershell
cargo run --bin acc-live-telemetry -- generate-mock --out .\data\mock.acctlm --samples 1000
cargo run --bin acc-live-telemetry -- record-auto --out-dir .\data --poll-hz 120 --chunk-rows 256 --flush-interval-ms 2000
cargo run --bin acc-live-telemetry -- inspect --input .\data\live_...acctlm
cargo run --bin acc-live-telemetry -- export --input .\data\mock.acctlm --out .\data\mock.csv --format csv
cargo run --bin acc-live-telemetry -- raw-info
cargo run --bin acc-live-telemetry -- raw-laps --input .\data\live_...acctlm
cargo run --bin acc-live-telemetry -- raw-lap-segments --input .\data\live_...acctlm
cargo run --bin acc-live-telemetry -- raw-valid-scan --input .\data\live_...acctlm
```

`export` currently reads the compact `controls` cluster. Files recorded by `record-auto` contain `raw_pages`; those are inspected through the raw page reader APIs and raw analysis commands.

## 15. Extension Rules

- Add new telemetry by introducing a new cluster or appending new column IDs to an existing cluster.
- Do not reuse column IDs after publishing them.
- Readers must skip unknown clusters and columns when possible.
- Bump `SCHEMA_HASH` for incompatible schema changes.
- Bump `FORMAT_VERSION` only when the top-level file layout or existing field semantics become incompatible.
- Prefer raw facts in recording clusters. Derived values should be computed on read or written to an explicitly derived cluster in a later protocol revision.
