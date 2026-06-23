# ACTL v2 二进制遥测文件格式规范 (`.acctlm2`)

> **Format version:** 2 (acctlm2)
> **File extension:** `.acctlm2`
> **Magic:** `ACT2` (4 bytes)
> **Byte order:** Little-endian throughout
> **Status:** Implemented (`src/format_v2.rs`, `src/encode_v2.rs`, `src/writer_v2.rs`, `src/reader_v2.rs`)
> **V1 格式文档**: 见 [v1-acctlm-format-spec.md](v1-acctlm-format-spec.md) — chunked cluster 格式 (`.acctlm`)

---

## 1. Overview & Motivation

acctlm2 is the second generation of the ACTL binary telemetry format used for recording Assetto Corsa Competizione (ACC) live telemetry. It is a **row-group-based columnar** format designed to replace the cluster-chunked layout of acctlm v1.

### Why v2 exists

acctlm v1 (`.acctlm`) writes 9 independent clusters per flush, each repeating
`sample_tick` and `timestamp_ns`. This causes three structural problems that
v2 solves:

| Problem | v1 behaviour | v2 solution |
|---|---|---|
| **Metadata duplication** | `sample_tick` and `timestamp_ns` stored in all 9 clusters (18 redundant instances per frame) | Stored once in the `FrameMeta` group |
| **All-or-nothing I/O** | Full-file read required; clusters are interspersed and cannot be skipped | mmap + skip index enables reading only the groups you need |
| **No data integrity** | No per-column checksum; bit-rot is silent | CRC32 per encoded column, verified on every decode |

### Performance characteristics

Benchmarks on 10,000 frames (120 Hz × 83 seconds of telemetry):

| Operation | v1 | v2 | Speedup |
|---|---|---|---|
| Full file read (all 9 substructures) | baseline | ~12× faster | Columnar layout + mmap eliminates per-cluster seek/decode overhead |
| Selective read (driver inputs only) | baseline (must read all) | ~27× faster | Skip index O(1) navigation — reads only `FrameMeta` + `DriverInputs` groups |

---

## 2. File Structure

```
┌──────────────────────────────┐ offset 0
│      FileHeaderV2 (64 B)      │
├──────────────────────────────┤ header.schema_offset
│      SchemaBlockV2            │ (column catalog)
├──────────────────────────────┤ header.metadata_offset
│      Metadata Block           │ (variable, META magic)
├──────────────────────────────┤ header.first_row_group_offset
│      Row Group 0              │
│      Row Group 1              │
│      ...                      │
│      Row Group N              │
├──────────────────────────────┤ footer_offset
│      FooterV2 (20 B fixed)    │
│      Skip Index entries       │ (skip_index_count × 32 B)
│      Lap Index entries        │ (lap_index_count × 32 B)
└──────────────────────────────┘ EOF
```

### 2.1 File Header (64 bytes)

| Offset | Size | Type | Field | Description |
|---|---|---|---|---|
| 0 | 4 | `[u8; 4]` | magic | `b"ACT2"` |
| 4 | 2 | `u16` | version | `2` |
| 6 | 8 | `u64` | schema_offset | Byte offset to SchemaBlockV2 |
| 14 | 8 | `u64` | metadata_offset | Byte offset to metadata block |
| 22 | 8 | `u64` | first_row_group_offset | Byte offset to first row group |
| 30 | 8 | `u64` | footer_offset | Byte offset to footer (0 until `finish()`) |
| 38 | 8 | `u64` | created_unix_ns | Unix nanosecond creation timestamp |
| 46 | 4 | `u32` | poll_hz | Sampling frequency in Hz (e.g. 120) |
| 50 | 14 | `[u8; 14]` | _reserved | Zero-padding |

The header is written twice: once with `footer_offset=0` at file creation, then rewritten
with the correct `footer_offset` during `finish()`. This makes interrupted recordings
recoverable — a reader can scan `RGHD` records forward from `first_row_group_offset`.

### 2.2 Schema Block

The schema declares every column organised by access group. It is written immediately
after the header and is immutable for the file's lifetime.

```
[u16] group_count
For each group:
  [u16] group_id       (0–6)
  [u16] column_count
  For each column:
    [u16] column_id    (1–213)
    [u8]  value_type   (0=current schema placeholder; row groups use 1=U64, 2=I32, 3=F32, 4=F64, 5=BYTES, 6=BYTES_F32, 7=BYTES_U16, 8=BYTES_I32)
    [u8]  name_len
    [u8; name_len] name  (UTF-8 column name, e.g. "speed_kmh")
```

The schema **does not store the assigned value type** at write time — the
`value_type` field is written as `0` in the current implementation and is
available only in the per-column `ColumnEntryV2` within each row group.

### 2.3 Metadata Block

The metadata block shares the same layout as v1: `META` magic + session info
(track name, car model, poll rate, SM/AC versions, session count, sector count,
engine specs, raw static bytes, session type). See `decode_metadata_v2()` in
`src/reader_v2.rs:38–112` for the complete format.

### 2.4 Row Groups

Each row group stores up to `chunk_rows` frames (default: **1024**). Within a
row group, data is laid out columnar: all values for column X come before all
values for column X+1. See [Section 5](#5-row-group-layout) for the detailed
binary layout.

### 2.5 Footer & Indexes

The footer is written at `finish()` time at the end of the file:

```
[ 4] b"FTR2"                  footer_magic
[ 8] u64                      footer_offset (self-referential)
[ 4] u32                      skip_index_count
[ 4] u32                      lap_index_count
-------------------------------------- 20 B fixed
[skip_index_count × 32 B]     SkipIndexEntry[]
[lap_index_count × 32 B]      LapIndexEntryV2[]
```

---

## 3. Logical Access Groups

acctlm2 groups all 172 telemetry columns into **7 access groups** aligned by
query pattern. Unlike v1's 9 clusters (Controls, Motion, Tyres, Powertrain,
Session, Timing, CarState, Environment, OtherCars), v2 merges related clusters
and splits based on **read frequency** rather than data origin.

| GroupId | Name | Column Count | Id Ranges | Access |
|---|---|---|---|---|
| 0 | `FrameMeta` | 4 | 1–4 | Always read first |
| 1 | `DriverInputs` | 30 | 10–17, 60–81 | Hot — every tick |
| 2 | `VehicleDynamics` | 7 | 20–26 | Warm — motion analysis |
| 3 | `Tyres` | 29 | 30–58 | Warm — tyre analysis |
| 4 | `Timing` | 26 | 93–94, 96–99, 108, 120–138 | Hot — lap/sector |
| 5 | `Environment` | 14 | 90–91, 95, 106–107, 200–208 | Cool — session conditions |
| 6 | `ColdStorage` | 62 | 92, 100–105, 109–119, 150–189, 210–213 | Rare — setup, flags, other cars |

### 3.1 Group 0 — FrameMeta (4 columns)

Frame identity. Always read first to establish tick/timestamp context. These two
fields (`sample_tick`, `timestamp_ns`) appeared in every v1 cluster as redundant
copies; here they are defined once.

| ColumnId | Name | Type |
|---|---|---|
| 1 | `sample_tick` | U64 |
| 2 | `timestamp_ns` | U64 |
| 3 | `physics_packet_id` | I32 |
| 4 | `graphics_packet_id` | I32 |

### 3.2 Group 1 — DriverInputs (30 columns)

Driver controls (IDs 10–17) and powertrain/ERS state (IDs 60–81). This group
corresponds to v1's `Controls` + `Powertrain` clusters.

**Controls (10–17):** `speed_kmh` (F32), `gas` (F32), `brake` (F32),
`clutch` (F32), `steer_angle` (F32), `gear` (I32), `rpms` (I32), `fuel` (F32).

**Powertrain (60–81):** `turbo_boost` (F32), `ballast` (F32), `kers_charge` (F32),
`kers_input` (F32), `kers_current_kj` (F32), `drs` (F32), `tc_physics` (F32),
`abs_physics` (F32), `engine_brake` (I32), `ers_recovery_level` (I32),
`ers_power_level` (I32), `ers_heat_charging` (I32), `ers_is_charging` (I32),
`drs_available` (I32), `drs_enabled` (I32), `tc_in_action` (I32),
`abs_in_action` (I32), `auto_shifter_on` (I32), `current_max_rpm` (I32),
`p2p_activations` (I32), `p2p_status` (I32), `water_temp` (F32).

### 3.3 Group 2 — VehicleDynamics (7 columns)

Vehicle motion (v1's `Motion` cluster minus `sample_tick`/`timestamp_ns`).

| 20 | `velocity` | TYPE_BYTES_F32 (3 x f32) |
| 21 | `acc_g` | TYPE_BYTES_F32 (3 x f32) |
| 22 | `local_velocity` | TYPE_BYTES_F32 (3 x f32) |
| 23 | `local_angular_vel` | TYPE_BYTES_F32 (3 x f32) |
| 24 | `heading` | F32 |
| 25 | `pitch` | F32 |
| 26 | `roll` | F32 |

### 3.4 Group 3 — Tyres (29 columns)

Wheel and tyre data (IDs 30–58). Four-wide arrays per wheel (FL/FR/RL/RR) are
encoded as `TYPE_BYTES_F32` with `sub_value_count=4`; 12-wide contact geometry
arrays use the same typed bytes encoding with `sub_value_count=12`.

Key columns: `wheel_slip`, `wheel_load`, `wheels_pressure`, `wheel_angular_speed`,
`tyre_wear`, `tyre_dirty_level`, `tyre_core_temperature`, `camber_rad`,
`suspension_travel`, `slip_ratio`, `slip_angle`, all temperature arrays
(`tyre_temp_i/m/o`, `tyre_temp`), force/moment (`mz`, `fx`, `fy`),
`suspension_damage`, `brake_temp`, `brake_pressure`, `pad_life`, `disc_life`,
contact geometry (`tyre_contact_point/normal/heading` — 12-wide arrays),
`number_of_tyres_out` (I32), `front_brake_compound` (I32), `rear_brake_compound` (I32).

### 3.5 Group 4 — Timing (26 columns)

Lap/sector timing plus session position fields split from v1's `Session` cluster.

Session position: `completed_laps` (93), `position` (94), `number_of_laps` (96),
`current_sector_index` (97), `normalized_car_position` (98), `is_in_pit` (99),
`is_valid_lap` (108).

Timing (120–138): `i_current_time`, `i_last_time`, `i_best_time`, `i_split`,
`last_sector_time`, `i_delta_lap_time`, `is_delta_positive`,
`i_estimated_lap_time`, `fuel_estimated_laps`, `fuel_x_lap`, `used_fuel`,
`distance_traveled`, and 7 string-formatted timing fields
(`current_time_str` through `estimated_lap_time_str`), plus
`observed_slot_before_i_split`.

### 3.6 Group 5 — Environment (14 columns)

Session state and environmental conditions.

IDs 90–91, 95, 106–107: `status` (I32), `session` (I32), `session_time_left` (F32),
`clock` (F32), `replay_time_multiplier` (F32).

IDs 200–208: `air_density`, `air_temp`, `road_temp`, `wind_speed`,
`wind_direction`, `surface_grip` (F32), plus `rain_intensity`,
`rain_intensity_in_10min`, and `rain_intensity_in_30min` (I32).

### 3.7 Group 6 — ColdStorage (62 columns)

Rarely-accessed fields: session metadata (IDs 92, 100–105, 109–119), car state
and setup (IDs 150–189), and other cars (IDs 210–213).

Car state highlights: `car_damage`, `pit_limiter_on`, `ride_height`, `brake_bias`,
`rain_lights`, `lights_stage`, `wiper_lv`, driver stint times, `tyre_compound_str`
(33-wide TYPE_BYTES_U16 array), MFD settings, `tc_level`, `tc_cut`, `engine_map`,
`abs_level`, `exhaust_temperature`, vibration fields, `car_coordinates`
(60 cars x 3 TYPE_BYTES_F32 values), and `car_id` (60 TYPE_BYTES_I32 values).

---

## 4. Column Encoding

Each column's data is stored as an independent byte sequence with a 6-byte header:

```
[codec: u8] [value_type: u8] [value_count: u32 LE] [payload...]
```

| Field | Size | Description |
|---|---|---|
| `codec` | 1 | `0x00` = PLAIN, `0x01` = DELTA |
| `value_type` | 1 | `0x01`=U64, `0x02`=I32, `0x03`=F32, `0x04`=F64, `0x05`=BYTES, `0x06`=BYTES_F32, `0x07`=BYTES_U16, `0x08`=BYTES_I32 |
| `value_count` | 4 | Number of logical items (rows) |
| `payload` | variable | Encoded values |

### 4.1 Type Constants (v2)

| Constant | Value | Element size | Notes |
|---|---|---|---|
| `TYPE_U64` | `0x01` | 8 bytes | |
| `TYPE_I32` | `0x02` | 4 bytes | |
| `TYPE_F32` | `0x03` | 4 bytes | |
| `TYPE_F64` | `0x04` | 8 bytes | New in v2 (v1 max type was BYTES=4) |
| `TYPE_BYTES` | `0x05` | variable | Per-item: `[sub_count: u8] [f64 LE × sub_count]` |
| `TYPE_BYTES_F32` | `0x06` | variable | Per-item: `[sub_count: u8] [f32 LE × sub_count]` |
| `TYPE_BYTES_U16` | `0x07` | variable | Per-item: `[sub_count: u8] [u16 LE × sub_count]` |
| `TYPE_BYTES_I32` | `0x08` | variable | Per-item: `[sub_count: u8] [i32 LE × sub_count]` |

> **Important:** v2 type constants differ from v1. In v1: `TYPE_U64=1, TYPE_F32=2, TYPE_I32=3, TYPE_BYTES=4`.
> In v2: `TYPE_U64=1, TYPE_I32=2, TYPE_F32=3, TYPE_F64=4, TYPE_BYTES=5`, with typed bytes variants at `6..8`.

### 4.2 PLAIN Encoding (`codec = 0x00`)

Each value stored as raw little-endian bytes. For `TYPE_BYTES`, each logical item is:

```
[sub_value_count: u8] [val0: f64 LE] [val1: f64 LE] ...
```

For typed bytes variants, the same leading `sub_value_count` is used, but the
payload element width is the declared subtype: `f32`, `u16`, or `i32`.

Example: a velocity vector `[12.5, 3.2, -1.0]` encoded as `TYPE_BYTES_F32`
with sub_count=3:
```
03 | 00 00 48 41 | CD CC 4C 40 | 00 00 80 BF
```

### 4.3 DELTA Encoding (`codec = 0x01`)

Applies to `TYPE_U64`, `TYPE_I32`, and generic `TYPE_BYTES` (with sub-count > 0)
only. Floats (`TYPE_F32`, `TYPE_F64`) and typed bytes variants fall back to PLAIN.
The current writer uses `CODEC_PLAIN` for all columns in `flush_row_group()`.

**U64 DELTA layout:**
```
[first_value: u64 LE] [delta_1: i64 LE] [delta_2: i64 LE] ...
```
Where `delta_n = value_n - value_(n-1)` cast to `i64`.

**I32 DELTA layout:**
```
[first_value: i32 LE] [delta_1: i32 LE] [delta_2: i32 LE] ...
```

**BYTES DELTA layout:**
For each logical item N (where N ≥ 1):
- Item 0: stored as plain `[sub_count][f64 LE × sub_count]`
- Items 1+: each sub-value stored as `i64` delta from previous item's corresponding sub-value

`value_count` stores the number of **logical items** (not the total sub-values), so
`value_count = values.len() / sub_value_count`.

### 4.4 CRC32 Integrity

A CRC32 (IEEE 802.3, computed by `crc32fast::hash`) covers the **entire encoded
column buffer** — the 6-byte header plus the payload. The CRC32 value is stored
in `ColumnEntryV2.crc32` (see §5.1) and verified in `decode_column()` before
returning decoded values. A mismatch returns `TelemetryError::InvalidFormat`.

---

## 5. Row Group Layout

Each row group stores up to `chunk_rows` frames (default 1024). Within a row group,
data is organised **group-first, columnar within group**:

```
┌──────────────────────────────┐
│  RowGroupHeader (variable)   │  RGHD magic, row count, frame tick range,
│                              │  7 GroupEntryV2 entries
├──────────────────────────────┤
│  Group 0 block (FrameMeta)   │  [gid:u16][col_count:u16][ColumnEntryV2 × N][data...]
├──────────────────────────────┤
│  Group 1 block (DriverInputs)│
├──────────────────────────────┤
│  Group 2 block (VehDynamics) │
├──────────────────────────────┤
│  Group 3 block (Tyres)       │
├──────────────────────────────┤
│  Group 4 block (Timing)      │
├──────────────────────────────┤
│  Group 5 block (Environment) │
├──────────────────────────────┤
│  Group 6 block (ColdStorage) │
└──────────────────────────────┘
```

### 5.1 RowGroupHeader

**Fixed portion (32 bytes):**

| Offset | Size | Field | Description |
|---|---|---|---|
| 0 | 4 | magic | `b"RGHD"` |
| 4 | 4 | row_count | `u32` — frames in this row group |
| 8 | 8 | frame_start_tick | `u64` — first sample_tick |
| 16 | 8 | frame_end_tick | `u64` — last sample_tick |
| 24 | 2 | group_count | `u16` — always 7 |
| 26 | 6 | _reserved | Zero-padding |

**Followed by `group_count` × 10-byte GroupEntryV2 entries:**

| Offset | Size | Field | Description |
|---|---|---|---|
| 0 | 2 | group_id | `u16` — 0..6 |
| 2 | 4 | offset | `u32` — byte offset from RGHD start to this group's block |
| 6 | 4 | byte_len | `u32` — total bytes of this group's block |

### 5.2 Group Block Layout

```
[group_id: u16 LE]
[column_count: u16 LE]
[ColumnEntryV2 × column_count]   — 40 bytes each
[encoded_column_0]                — variable, byte-aligned contiguously
[encoded_column_1]
...
```

### 5.3 ColumnEntryV2 (40 bytes)

| Offset | Size | Field | Description |
|---|---|---|---|
| 0 | 2 | column_id | `u16` — ColumnId |
| 2 | 1 | codec | `u8` — 0x00=PLAIN, 0x01=DELTA |
| 3 | 1 | value_type | `u8` — 0x01..0x08 |
| 4 | 4 | byte_len | `u32` — encoded data length (header + payload) |
| 8 | 4 | crc32 | `u32` — CRC32 of encoded column buffer |
| 12 | 8 | min_value | `f64` — minimum value in this chunk |
| 20 | 8 | max_value | `f64` — maximum value in this chunk |
| 28 | 12 | _reserved | Zero-padding |

The column data for column N starts at:
```
group_block_start + 4 + column_count × 40 + Σ(ColumnEntry[i].byte_len for i < N)
```

---

## 6. Skip Index

The skip index enables **O(1) column-level random access** without reading the
entire file. It is stored after the footer and maps every column in every row
group to a precise byte range.

### 6.1 SkipIndexEntry (32 bytes)

| Offset | Size | Field | Description |
|---|---|---|---|
| 0 | 2 | access_group | `u16` — GroupId (0–6) |
| 2 | 2 | column_id | `u16` — ColumnId (1–213) |
| 4 | 8 | frame_start | `u64` — first sample_tick in this entry |
| 12 | 8 | frame_end | `u64` — last sample_tick in this entry |
| 20 | 4 | row_group_index | `u32` — which row group (0-based) |
| 24 | 4 | offset_in_group | `u32` — byte offset within the group block |
| 28 | 4 | byte_len | `u32` — encoded column data length |

### 6.2 How Selective Reads Work

`read_group_frames()` (in `reader_v2.rs`) implements the selective access path:

1. **Filter skip entries** by `access_group == target_group` and `frame_start/frame_end` overlap with the requested range.
2. **Deduplicate row group indices** — entries for different columns in the same group point to the same row group.
3. **For each row group index:**
   - Seek to `row_group_offsets[rg_idx]` in the mmap'd byte slice.
   - Parse the `RowGroupHeader` to find the `GroupEntryV2` for the target group.
   - Jump to `group_data_start = rg_offset + ge.offset`.
   - Read `[gid:u16][col_count:u16][ColumnEntryV2 × N][data...]`.
   - For each requested column, extract `group_bytes[data_offset..data_offset+entry.byte_len]` and call `decode_column()`.
4. **Merge** column data across row groups by concatenation.
5. **Apply frame-range filtering** using `FrameMeta` tick values as the reference timeline.

**Example:** reading only driver inputs (`read_all_controls_v2()`) requests
groups `[FrameMeta, DriverInputs]`. The skip index skips VehicleDynamics, Tyres,
Timing, Environment, and ColdStorage entirely — the mmap never touches those
regions. At 1024 rows per group and 172 total columns, this skips ~95% of the
file for a typical query.

---

## 7. Lap Index

Lap boundaries are recorded in the lap index stored after the skip index in the
footer block.

### 7.1 LapIndexEntryV2 (32 bytes)

| Offset | Size | Field | Description |
|---|---|---|---|
| 0 | 4 | lap_number | `i32` — lap number |
| 4 | 8 | start_tick | `u64` — first sample_tick of this lap |
| 12 | 8 | end_tick | `u64` — last sample_tick of this lap |
| 20 | 4 | sample_count | `u32` — number of frames in this lap |
| 24 | 4 | is_valid | `i32` — 1 = valid lap |
| 28 | 4 | is_out_lap | `i32` — 1 = out-lap |

Lap boundaries are detected during `write_frame()`: when `frame.session.completed_laps + 1`
changes, the current lap is closed with `end_tick = tick - 1` and a new lap begins.
The final lap is closed in `finish()`.

Lap-indexed reading (`read_lap_frames()`) uses the skip index: the lap entry's
`start_tick`/`end_tick` are passed as `[start_frame, end_frame]` to
`read_group_frames()`, which filters skip entries overlapping that range.

---

## 8. Comparison with acctlm v1

### 8.1 Structural Comparison

| Aspect | acctlm v1 (`.acctlm`) | acctlm v2 (`.acctlm2`) |
|---|---|---|
| **Magic** | `ACTL\r\n\x1A\n` (8 bytes) | `ACT2` (4 bytes) |
| **Header size** | 128 bytes | 64 bytes |
| **Organisation** | 9 independent clusters, chunked | 7 access groups, row-grouped |
| **Chunk size** | `chunk_rows` per cluster (default 1024) | `chunk_rows` frames per row group (all groups together) |
| **Write strategy** | 9 separate chunks flushed per `chunk_rows` frames | 1 row group containing all 7 groups, flushed together |
| **Data grouping** | By origin (Controls, Motion, Tyres, Powertrain, Session, Timing, CarState, Environment, OtherCars) | By access pattern (FrameMeta, DriverInputs, VehicleDynamics, Tyres, Timing, Environment, ColdStorage) |
| **Column count** | 188 per frame (with duplicates) | 172 per frame (deduplicated) |
| **sample_tick / timestamp_ns** | Present in all 9 clusters (18 instances) | Present in FrameMeta group only (2 instances) |
| **Column selectivity** | Cluster-level only — must read entire cluster | Group-level + skip index — O(1) to any column |
| **Read pattern** | Full-file read into memory, then parse all clusters | mmap the file, parse header/footer up front, read row groups on demand |
| **Data integrity** | No checksum | CRC32 per encoded column |
| **Encoding** | PLAIN only (`CODEC_PLAIN_LE = 0x00`) | PLAIN (`0x00`) + DELTA (`0x01`) |
| **Type system** | U64=1, F32=2, I32=3, BYTES=4 | U64=1, I32=2, F32=3, F64=4, BYTES=5 |
| **ColumnEntry size** | 40 bytes (v1: `ColumnEntry`) | 40 bytes (v2: `ColumnEntryV2` with crc32, min/max) |
| **Cluster/Group header** | 72 bytes per chunk (`CHNK` magic) | Variable: 32 + 10 × group_count per row group (`RGHD` magic) |
| **Footer** | `FOOT` magic, 1 index entry per chunk | `FTR2` magic, skip index + lap index |
| **V1 cluster merge** | — | Powertrain → DriverInputs; Session split into Timing/Environment/ColdStorage |
| **Metadata** | `META` block (same format) | `META` block (same format) |

### 8.2 Why Each Change Matters

**Row groups replace clusters.**
V1 writes 9 chunk records per flush — all 9 clusters must be written before the
writer can accept the next batch. This couples all data streams and means a
reader must ingest all 9 clusters to reconstruct a frame. V2 writes one row
group containing all 7 groups; the reader reads only the groups it needs.

**Skip index enables selective reads.**
V1's `CHNK` index maps `sample_tick` ranges to file offsets at the cluster
level. Reading `controls` alone is impossible — you must parse at least the
controls cluster plus every other cluster to align frames. V2's skip index maps
every individual column to its exact byte range, so reading `controls` means
reading exactly two groups (`FrameMeta` + `DriverInputs`).

**Deduplication saves space.**
In v1, `sample_tick` and `timestamp_ns` appear in all 9 clusters — 18
redundant 8-byte values per frame = 144 wasted bytes per frame. For a 1-hour
recording at 120 Hz (432,000 frames), that's ~62 MB of redundant data. In v2,
these fields are stored once.

**CRC32 catches corruption.**
Silent bit-rot in telemetry files can produce plausible-looking but incorrect
numbers (e.g., a `speed_kmh` that quietly drops from 250 to 25). V2 CRC32s
are verified on every decode — corrupted columns raise errors rather than
returning garbage.

**Type system divergence.**
V2 adds `TYPE_F64` (native `f64` elements) and renumbers the constants. V1
only had BYTES for multi-value columns. V2 uses BYTES for array columns but
supports F64 for native double-precision single-value columns. The type
constant mapping changes: v1 `TYPE_F32=2` becomes v2 `TYPE_F32=3`, etc.
This means v2 decoders **cannot** read v1 type codes and vice versa.

---

## 9. Reading & Writing

### 9.1 Writing — `BinaryTelemetryWriterV2`

```rust
use module_live_telemetry::{
    BinaryTelemetryWriterV2, LiveTelemetryConfig, TelemetryFrame,
};

let meta = SessionMetadata { /* ... */ };
let config = LiveTelemetryConfig { poll_hz: 120.0, chunk_rows: 1024 };

let mut writer = BinaryTelemetryWriterV2::create_file("session.acctlm2", meta, config)?;

// Write frames one at a time — writer buffers internally
for frame in &frames {
    writer.write_frame(frame)?;
}

// Finish flushes the last partial row group and writes footer + indexes
let summary = writer.finish()?;
// summary.total_samples, summary.chunk_count (row group count), summary.total_bytes
```

Internally:
- `create_file()` writes the header (footer_offset=0), schema, and metadata.
- `write_frame()` buffers frames; when `buffer.len() >= chunk_rows`, calls `flush_row_group()`.
- `flush_row_group()` encodes all 7 groups, writes the RGHD header + group blocks, and appends `SkipIndexEntry`s.
- `finish()` writes the footer, skip index entries, lap index entries, then seeks back to rewrite the header with the correct `footer_offset`.

### 9.2 Reading — `BinaryTelemetryReaderV2`

```rust
use module_live_telemetry::BinaryTelemetryReaderV2;

let reader = BinaryTelemetryReaderV2::open("session.acctlm2")?;

// Full read — reconstructs all frames
let all_frames = reader.read_all_frames()?;

// Selective read — only driver controls (FrameMeta + DriverInputs)
let controls = reader.read_all_controls_v2()?;

// Selective read — only vehicle dynamics (FrameMeta + VehicleDynamics)
let motion = reader.read_all_motion_v2()?;

// Lap-indexed read
let lap_5_frames = reader.read_lap_frames(5)?;

// Metadata
let meta = reader.metadata();
```

Available selective read methods:
- `read_all_controls_v2()` → `Vec<ControlSample>`
- `read_all_motion_v2()` → `Vec<MotionSample>`
- `read_all_tyres_v2()` → `Vec<TyreSample>`
- `read_all_powertrain_v2()` → `Vec<PowertrainSample>`
- `read_all_session_v2()` → `Vec<SessionSample>`
- `read_all_timing_v2()` → `Vec<TimingSample>`
- `read_all_car_state_v2()` → `Vec<CarStateSample>`
- `read_all_environment_v2()` → `Vec<EnvironmentSample>`
- `read_all_other_cars_v2()` → `Vec<OtherCarsSample>`

Each reads exactly two groups: `FrameMeta` (for tick alignment) plus the
substructure's group. The skip index ensures only those two groups' bytes are
touched.

### 9.3 Unified Reader

The public `BinaryTelemetryReader` (in `src/reader.rs`) transparently dispatches
to v1 or v2 based on the file magic at open time:

```rust
use module_live_telemetry::BinaryTelemetryReader;

// Works for both .acctlm (v1) and .acctlm2 (v2)
let reader = BinaryTelemetryReader::open("session.acctlm2")?;
let frames = reader.read_all_frames()?;         // unified API
let controls = reader.read_all_controls()?;      // unified API
```

---

## 10. Migration from v1 to v2

The `acctlm-to-acctlm2` CLI converter performs a lossless v1 → v2 conversion.

### Installation

The binary is part of the `module_live_telemetry` crate. Build with:

```
cargo build --release --bin acctlm-to-acctlm2
```

### Usage

```
acctlm-to-acctlm2 [--force] <input.acctlm> [output.acctlm2]
```

If `output.acctlm2` is omitted, it's derived by appending `"2"` to the input
path (e.g., `monza_race.acctlm` → `monza_race.acctlm2`).

Use `--force` to overwrite an existing output file.

### Conversion Process

1. **Open v1 file** via `BinaryTelemetryReader` and read all 9 cluster types.
2. **Validate** that all clusters have the same frame count.
3. **Assemble** `TelemetryFrame` structs by joining clusters by index position.
4. **Write v2 file** via `BinaryTelemetryWriterV2`, passing each assembled frame.
5. **Verify roundtrip** by reading the v2 file back and comparing key fields
   (`sample_tick`, `timestamp_ns`, `speed_kmh`, `gas`, `brake`, `velocity`).

### Example

```
$ acctlm-to-acctlm2 monza_race.acctlm
Reading monza_race.acctlm...
Converting 54000 frames...
  1000/54000 frames written...
  2000/54000 frames written...
  ...
Finishing output file...
Verifying output...
Verification passed: all 54000 frames roundtrip correctly
Converted 54000 frames from Monza (Ferrari 296 GT3) — output: monza_race.acctlm2 (45210368 bytes, 53 row groups)
```
