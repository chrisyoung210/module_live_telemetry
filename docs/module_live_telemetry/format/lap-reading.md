# acctlm 文件格式 —— 圈速数据读取

> **相关文档**: [v1-acctlm-format-spec.md](v1-acctlm-format-spec.md) · [v2-acctlm2-format-spec.md](v2-acctlm2-format-spec.md)

本文档描述如何从 acctlm 二进制文件中读取圈速数据（`iLastTime` / `iBestTime`），不涉及 ACC 共享内存的读取过程。

---

## 文件结构总览

```
┌─ FileHeader (128 B)
├─ Schema Block
├─ Metadata Block
├─ Chunk 0 (cluster_id=0x0100, controls)
├─ Chunk 1 (cluster_id=0x0200, motion)
├─ ...
├─ Chunk N (cluster_id=0x0600, timing)        ★ 圈速在这里
├─ ...
├─ Footer = "INDX" + IndexEntry[] + "FOOT"
├─ (可选) "LAPS" block — lap index
└─ (可选) "LPTM" block — lap times
```

所有整数均为 **little-endian**。

---

## Step 1: 读取 FileHeader（固定 128 字节）

| Offset | Size | 字段 | 说明 |
|--------|------|------|------|
| 0 | 8 | MAGIC | `ACTL\r\n\x1A\n` |
| 8 | 2 | version | 2 |
| 10 | 2 | header_size | 128 |
| 12 | 4 | flags | — |
| 16 | 8 | schema_offset | — |
| 24 | 8 | metadata_offset | — |
| 32 | 8 | first_chunk_offset | — |
| 40 | 8 | **footer_offset** | **→ Step 2** |

---

## Step 2: 跳转到 `footer_offset` 读取 Index

Footer 结构：

```
"INDX" (4B)
entry_count (8B, u64)
IndexEntry × entry_count  (各 56 B)
"FOOT" (4B)
... (total_samples, chunk_count 等)
```

### IndexEntry 结构（56 字节）

| Offset | Size | 字段 | 说明 |
|--------|------|------|------|
| 0 | 2 | **cluster_id** | `0x0600` = timing |
| 2 | 2 | (padding) | — |
| 4 | 4 | chunk_seq | 序号 |
| 8 | 8 | **file_offset** | **chunk 在文件中的绝对偏移 → Step 3** |
| 16 | 4 | byte_len | chunk 总字节数 |
| 20 | 4 | (padding) | — |
| 24 | 8 | start_time_ns | — |
| 32 | 8 | end_time_ns | — |
| 40 | 8 | start_tick | 起始采样 tick |
| 48 | 8 | end_tick | 结束采样 tick |

**过滤条件**: `cluster_id == 0x0600`

---

## Step 3: 跳转到 `file_offset`，解析 ChunkHeader

### ChunkHeader 结构（72 字节）

| Offset | Size | 字段 | 说明 |
|--------|------|------|------|
| 0 | 4 | "CHNK" | magic |
| 4 | 2 | header_size | 72 |
| 6 | 2 | cluster_id | 应为 `0x0600` |
| 8 | 4 | chunk_seq | — |
| 12 | 8 | schema_hash | — |
| 20 | 8 | base_sample_tick | — |
| 28 | 4 | sample_stride | — |
| 32 | 4 | **sample_count** | **该列的行数** |
| 36 | 8 | start_time_ns | — |
| 44 | 8 | end_time_ns | — |
| 52 | 4 | start_lap | — |
| 56 | 4 | end_lap | — |
| 60 | 2 | **column_count** | **列描述符数量，timing=21** |
| 62 | 2 | flags | — |
| 64 | 4 | payload_len | payload 总字节数 |
| 68 | 4 | payload_crc32 | — |

---

## Step 4: 解析 ColumnEntry 数组

紧接 ChunkHeader，共 `column_count` 个，每个固定 **40 字节**。

### ColumnEntry 结构（40 字节）

| Offset | Size | 字段 | 说明 |
|--------|------|------|------|
| 0 | 2 | **column_id** | **列 ID → 121/122 为目标** |
| 2 | 1 | codec | 0 (PLAIN_LE) |
| 3 | 1 | **value_type** | **3 = TYPE_I32** |
| 4 | 1 | lane_count | — |
| 5 | 1 | flags | — |
| 6 | 4 | **offset** | **该列数据在 payload 内的字节偏移** |
| 10 | 4 | **byte_len** | **该列数据总字节数（= sample_count × 4）** |
| 14 | 4 | null_offset | — |
| 18 | 8 | min_value | f64 |
| 26 | 8 | max_value | f64 |
| 34 | 6 | (reserved) | — |

**目标列**:

| column_id | 名称 | 含义 | value_type | 每行大小 |
|-----------|------|------|------------|----------|
| **121** | `iLastTime` | 上一圈用时 (ms) | 3 (i32) | 4 字节 |
| **122** | `iBestTime` | 本场最佳圈速 (ms) | 3 (i32) | 4 字节 |

---

## Step 5: 从 Payload 读取数据

### Payload 起始位置计算

```
payload_start = file_offset + ChunkHeader 大小 + column_count × ColumnEntry 大小
              = file_offset + 72 + 21 × 40
              = file_offset + 912
```

### 读取 iLastTime (column_id=121)

```
data_start = payload_start + col_121.offset
data_len   = col_121.byte_len  (= sample_count × 4)

// 连续读取 sample_count 个 i32 LE
for i in 0..sample_count:
    pos = data_start + i * 4
    value = i32_le(bytes[pos .. pos+4])
```

### 读取 iBestTime (column_id=122)

同上，使用 `col_122.offset`。

### 值的含义

- `0` — 该采样时刻无有效圈速
- `> 0` — 圈速，单位 **毫秒**
- 有效范围通常 `< 2,000,000`（约 33 分钟，超出视为异常值）

---

## 伪代码汇总

```python
# Step 1: 读 FileHeader
fh = read_struct(file, 0, FileHeader)
footer_offset = fh.footer_offset

# Step 2: 读 Index，过滤 timing chunks
file.seek(footer_offset)
assert file.read(4) == b"INDX"
entry_count = u64_le(file.read(8))
timing_chunks = []
for _ in range(entry_count):
    entry = read_struct(file, IndexEntry)  # 56 bytes
    if entry.cluster_id == 0x0600:
        timing_chunks.append(entry)

# Step 3-5: 解析每个 timing chunk
all_lap_times = []
all_best_times = []

for entry in timing_chunks:
    file.seek(entry.file_offset)

    # ChunkHeader (72 bytes)
    ch = read_struct(file, ChunkHeader)
    assert ch.cluster_id == 0x0600

    # ColumnEntry 数组 (21 × 40 = 840 bytes)
    col_121 = None  # iLastTime
    col_122 = None  # iBestTime
    for _ in range(ch.column_count):
        col = read_struct(file, ColumnEntry)  # 40 bytes
        if col.column_id == 121:
            col_121 = col
        if col.column_id == 122:
            col_122 = col

    # Payload 起始
    payload_start = entry.file_offset + 72 + ch.column_count * 40

    # 读取 iLastTime
    if col_121:
        offset = payload_start + col_121.offset
        file.seek(offset)
        for _ in range(ch.sample_count):
            val = i32_le(file.read(4))
            all_lap_times.append(val)

    # 读取 iBestTime
    if col_122:
        offset = payload_start + col_122.offset
        file.seek(offset)
        for _ in range(ch.sample_count):
            val = i32_le(file.read(4))
            all_best_times.append(val)
```

---

## 常量速查表

| 常量 | 值 |
|------|-----|
| FileHeader 大小 | 128 B |
| ChunkHeader 大小 | 72 B |
| ColumnEntry 大小 | 40 B |
| CLUSTER_TIMING | `0x0600` |
| COL_I_LAST_TIME | `121` |
| COL_I_BEST_TIME | `122` |
| TYPE_I32 | `3` |
| Timing column_count | `21` |
| Payload 起始偏移 | `file_offset + 912` |
| 每行 i32 | 4 字节 (LE) |
| 时间单位 | 毫秒 |
