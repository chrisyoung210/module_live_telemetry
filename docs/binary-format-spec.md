# ACTL 二进制遥测文件格式规范

> **版本**: 2  
> **魔数**: `ACTL\r\n\x1A\n` (8 bytes)  
> **Schema Hash**: `0x4143_544c_0000_0002`  
> **字节序**: 全字段小端序 (Little-Endian)

---

## 1. 文件整体布局

```
┌──────────────────────────┐ offset 0
│       File Header        │ 128 bytes (固定)
├──────────────────────────┤ header.schema_offset
│       Schema Block       │ 变长
├──────────────────────────┤ header.metadata_offset
│       Metadata Block     │ 变长
├──────────────────────────┤ header.first_chunk_offset
│     Chunk 1              │ 变长
│     Chunk 2              │ 变长
│     ...                  │
│     Chunk N              │ 变长
├──────────────────────────┤ footer_offset (如果写入)
│     Index Block          │ 变长
│     Footer               │ 24 bytes (固定)
└──────────────────────────┘ EOF
```

---

## 2. File Header（128 bytes 固定）

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 8 | `[u8; 8]` | magic | `"ACTL\r\n\x1A\n"` |
| 8 | 2 | `u16` | version | 格式版本号，当前为 `2` |
| 10 | 2 | `u16` | header_size | 固定为 `128` |
| 12 | 4 | `u32` | flags | 保留，当前为 `0` |
| 16 | 8 | `u64` | schema_offset | Schema Block 起始偏移 |
| 24 | 8 | `u64` | metadata_offset | Metadata Block 起始偏移 |
| 32 | 8 | `u64` | first_chunk_offset | 第一个 Chunk 的起始偏移 |
| 40 | 8 | `u64` | footer_offset | Index Block 偏移；为 0 表示无索引（需扫描） |
| 48 | 8 | `u64` | created_unix_ns | 创建时间（Unix 纳秒时间戳） |
| 56 | 4 | `u32` | timebase_hz | 时间基准，固定为 `1_000_000_000` |
| 60 | 4 | `u32` | poll_hz_x1000 | 采样频率 × 1000（如 120.0Hz → 120000） |
| 64 | 64 | `[u8; 64]` | reserved | 保留字节，全零 |

### 读取步骤

1. 读取 8 bytes 魔数，验证等于 `"ACTL\r\n\x1A\n"`
2. 读取 `version`，必须为 `2`
3. 读取 `header_size`，必须为 `128`
4. 依次读取其余字段
5. 根据 `schema_offset` → `metadata_offset` → `first_chunk_offset` → `footer_offset` 定位各段

---

## 3. Schema Block

Schema Block 描述文件中包含的 **所有列簇（cluster）** 的列定义。schema hash 用于验证文件的列定义与期望一致。

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 4 | `[u8; 4]` | magic | `"SCHM"` |
| 4 | 8 | `u64` | schema_hash | `0x4143_544c_0000_0002` |
| 12 | 2 | `u16` | cluster_count | 列簇数量（当前为 `9`） |

紧接着是 `cluster_count` 个 **Cluster Schema** 条目。

### Cluster Schema 条目

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 2 | `u16` | cluster_id | 列簇 ID |
| 2 | 2 | `u16` | column_count | 该列簇的列数 |

之后是 `column_count` 个 **Column Schema** 条目。

### Column Schema 条目

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 2 | `u16` | column_id | 列 ID |
| 2 | 1 | `u8` | value_type | 值类型（见下） |
| 3 | 1 | `u8` | name_len | 列名长度 |
| 4 | `name_len` | `[u8]` | name | 列名（UTF-8） |

### 值类型编码

| 值 | 类型 | 说明 |
|----|------|------|
| `1` | `TYPE_U64` | 64-bit 无符号整数 |
| `2` | `TYPE_F32` | 32-bit 浮点数 |
| `3` | `TYPE_I32` | 32-bit 有符号整数 |
| `4` | `TYPE_BYTES` | 变长字节序列 |

---

## 4. Metadata Block

Session 元数据，紧跟 Schema Block 之后。

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 4 | `[u8; 4]` | magic | `"META"` |
| 4 | 8 | `u64` | created_unix_ns | 创建时间（Unix 纳秒） |
| 12 | 4 | `u32` | poll_hz_x1000 | 采样频率 × 1000 |
| 16 | 4 | `u32` | chunk_rows | 每 chunk 行数 |
| 20 | 2 | `u16` | track_name_len | 赛道名长度 |
| 22 | 2 | `u16` | car_model_len | 车型名长度 |
| 24 | `track_name_len` | `[u8]` | track_name | 赛道名（UTF-8） |
| 24+track_len | `car_model_len` | `[u8]` | car_model | 车型名（UTF-8） |
| — | 2 | `u16` | sm_version_len | 服务管理器版本长度 |
| — | 2 | `u16` | ac_version_len | ACC 版本长度 |
| — | 4 | `i32` | number_of_sessions | 会话数量 |
| — | 4 | `i32` | num_cars | 车辆数量 |
| — | `sm_version_len` | `[u8]` | sm_version | 服务管理器版本 |
| — | `ac_version_len` | `[u8]` | ac_version | ACC 版本 |

> **注意**: `sm_version`/`ac_version`/`number_of_sessions`/`num_cars` 为扩展字段。若剩余字节不足 12 字节，解析时填入默认值（空字符串 / 0）。

---

## 5. Chunk（数据块）

每个 Chunk 存储一个列簇在某一时间段内的列式数据。

### Chunk Header（72 bytes 固定）

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 4 | `[u8; 4]` | magic | `"CHNK"` |
| 4 | 2 | `u16` | header_size | 固定为 `72` |
| 6 | 2 | `u16` | cluster_id | 列簇 ID |
| 8 | 4 | `u32` | chunk_seq | Chunk 序号（全局递增） |
| 12 | 8 | `u64` | schema_hash | `0x4143_544c_0000_0002` |
| 20 | 8 | `u64` | base_sample_tick | 基准采样 tick |
| 28 | 4 | `u32` | sample_stride | 采样间隔 |
| 32 | 4 | `u32` | sample_count | 采样数 |
| 36 | 8 | `u64` | start_time_ns | 起始时间（纳秒） |
| 44 | 8 | `u64` | end_time_ns | 结束时间（纳秒） |
| 52 | 4 | `i32` | start_lap | 起始圈号 |
| 56 | 4 | `i32` | end_lap | 结束圈号 |
| 60 | 2 | `u16` | column_count | 列数 |
| 62 | 2 | `u16` | flags | 保留 |
| 64 | 4 | `u32` | payload_len | 数据载荷字节数 |
| 68 | 4 | `u32` | payload_crc32 | 载荷 CRC32 校验 |

### Column Entry（40 bytes 固定）

紧跟 Chunk Header 之后的是 `column_count` 个 Column Entry。

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 2 | `u16` | column_id | 列 ID |
| 2 | 1 | `u8` | codec | 编码方式，`0` = Plain LE（仅支持 0） |
| 3 | 1 | `u8` | value_type | 值类型（同 Schema 中的定义） |
| 4 | 1 | `u8` | lane_count | 通道数（向量列时 > 1，如 `[f32; 4]` 的 lane_count=4） |
| 5 | 1 | `u8` | flags | 保留 |
| 6 | 4 | `u32` | offset | 该列数据在 Payload 中的字节偏移 |
| 10 | 4 | `u32` | byte_len | 该列数据总字节数 |
| 14 | 4 | `u32` | null_offset | NULL 位图偏移（未使用时为 0） |
| 18 | 8 | `f64` | min_value | 最小值（统计信息） |
| 26 | 8 | `f64` | max_value | 最大值（统计信息） |
| 34 | 6 | `[u8; 6]` | reserved | 保留 |

### Payload

Chunk Header + Column Entries 之后是 Payload 数据。

**列式存储**：每列数据按列连续排列。对于固定大小类型（`TYPE_U64`、`TYPE_F32`、`TYPE_I32`），每行占对应字节数；对于 `TYPE_BYTES`，每行是一个变长字节序列，具体长度由列的 `lane_count` 决定。

列的内存布局：
```
Payload = [Column_0 data][Column_1 data]...[Column_N data]
```

每列的 `offset` 在 `ColumnEntry` 中指定。列数据按行顺序存储：先所有行的第一列，再所有行的第二列……

> **CRC32 算法**: 使用多项式 `0xEDB88320`，初始值 `0xFFFFFFFF`，结果取反。即标准的 CRC-32 (ISO 3309)。

---

## 6. Index Block 与 Footer

如果文件正常关闭（写入器调用了 `finish()`），文件末尾会包含 Index Block 和 Footer，用于快速随机访问。

### Index Block

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 4 | `[u8; 4]` | magic | `"INDX"` |
| 4 | 8 | `u64` | entry_count | 索引条目数 |

之后是 `entry_count` 个 **Index Entry**（56 bytes 固定）。

### Index Entry（56 bytes 固定）

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 2 | `u16` | cluster_id | 列簇 ID |
| 2 | 2 | `[u8; 2]` | reserved | 保留 |
| 4 | 4 | `u32` | chunk_seq | Chunk 序号 |
| 8 | 8 | `u64` | file_offset | Chunk 在文件中的起始偏移 |
| 16 | 4 | `u32` | byte_len | Chunk 总字节数（含 header） |
| 20 | 4 | `[u8; 4]` | reserved | 保留 |
| 24 | 8 | `u64` | start_time_ns | 起始时间（纳秒） |
| 32 | 8 | `u64` | end_time_ns | 结束时间（纳秒） |
| 40 | 8 | `u64` | start_tick | 起始 tick |
| 48 | 8 | `u64` | end_tick | 结束 tick |

### Footer（24 bytes 固定）

| 偏移 | 长度 | 类型 | 字段 | 说明 |
|------|------|------|------|------|
| 0 | 4 | `[u8; 4]` | magic | `"FOOT"` |
| 4 | 8 | `u64` | index_offset | Index Block 的文件偏移 |
| 12 | 8 | `u64` | total_samples | 总采样数 |
| 20 | 4 | `u32` | chunk_count | Chunk 总数 |

---

## 7. Schema 定义的列簇与列

当前 Schema Hash `0x4143544C00000002` 包含以下 9 个列簇：

### 7.1 Controls (0x0100) — 驾驶控制

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | 采样 tick |
| 2 | timestampNs | u64 | 时间戳（纳秒） |
| 3 | physicsPacketId | i32 | 物理帧 ID |
| 4 | graphicsPacketId | i32 | 图形帧 ID |
| 10 | speedKmh | f32 | 速度 (km/h) |
| 11 | gas | f32 | 油门踏板 [0, 1] |
| 12 | brake | f32 | 刹车踏板 [0, 1] |
| 13 | clutch | f32 | 离合器 [0, 1] |
| 14 | steerAngle | f32 | 方向盘角度 |
| 15 | gear | i32 | 档位 |
| 16 | rpms | i32 | 发动机转速 |
| 17 | fuel | f32 | 燃油量 |

### 7.2 Motion (0x0200) — 车辆运动

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 20 | velocity | bytes(3×f32) | 世界坐标系速度 [vx, vy, vz] |
| 21 | accG | bytes(3×f32) | 加速度 G [x, y, z] |
| 22 | localVelocity | bytes(3×f32) | 车体坐标系速度 |
| 23 | localAngularVel | bytes(3×f32) | 车体坐标系角速度 |
| 24 | heading | f32 | 航向角 |
| 25 | pitch | f32 | 俯仰角 |
| 26 | roll | f32 | 翻滚角 |

> **注意**: Motion 列簇可以选择性包含额外的 `roll` 列（`MOTION_COLUMNS_EX` 定义了 9 列版本含 roll），`MOTION_COLUMNS` 定义了 8 列版本不含 roll。写入时使用 `MOTION_COLUMNS_EX`（含 roll），读取时通过 Column Entry 动态判断。

### 7.3 Tyres (0x0300) — 轮胎数据

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 30 | wheelSlip | bytes(4×f32) | 四轮滑移率 |
| 31 | wheelLoad | bytes(4×f32) | 四轮载荷 |
| 32 | wheelsPressure | bytes(4×f32) | 四轮胎压 |
| 33 | wheelAngularSpeed | bytes(4×f32) | 四轮角速度 |
| 34 | tyreWear | bytes(4×f32) | 四轮轮胎磨损 |
| 35 | tyreDirtyLevel | bytes(4×f32) | 四轮轮胎脏污 |
| 36 | tyreCoreTemperature | bytes(4×f32) | 四轮胎芯温度 |
| 37 | camberRad | bytes(4×f32) | 四轮外倾角(rad) |
| 38 | suspensionTravel | bytes(4×f32) | 四轮悬挂行程 |
| 39 | slipRatio | bytes(4×f32) | 四轮滑移比 |
| 40 | slipAngle | bytes(4×f32) | 四轮滑移角 |
| 41 | tyreTempI | bytes(4×f32) | 四轮内侧胎温 |
| 42 | tyreTempM | bytes(4×f32) | 四轮中侧胎温 |
| 43 | tyreTempO | bytes(4×f32) | 四轮外侧胎温 |
| 44 | tyreTemp | bytes(4×f32) | 四轮平均胎温 |
| 45 | mz | bytes(4×f32) | 四轮 Mz 力矩 |
| 46 | fx | bytes(4×f32) | 四轮 Fx 力 |
| 47 | fy | bytes(4×f32) | 四轮 Fy 力 |
| 48 | suspensionDamage | bytes(4×f32) | 四轮悬挂损伤 |
| 49 | brakeTemp | bytes(4×f32) | 四轮刹车温度 |
| 50 | brakePressure | bytes(4×f32) | 四轮刹车压力 |
| 51 | padLife | bytes(4×f32) | 四轮刹车片寿命 |
| 52 | discLife | bytes(4×f32) | 四轮刹车盘寿命 |
| 53 | tyreContactPoint | bytes(4×3×f32) | 四轮接地点坐标 |
| 54 | tyreContactNormal | bytes(4×3×f32) | 四轮接地点法线 |
| 55 | tyreContactHeading | bytes(4×3×f32) | 四轮接地点方向 |
| 56 | numberOfTyresOut | i32 | 轮胎出界数 |
| 57 | frontBrakeCompound | i32 | 前刹车配方 |
| 58 | rearBrakeCompound | i32 | 后刹车配方 |

> `bytes(N×f32)` 表示该列为 `N` 个 `f32` 值连续排列，`lane_count = N`。`bytes(4×3×f32)` 表示每个轮有 3 个 f32（三维向量），共 `12` 个 f32，`lane_count = 12`。

### 7.4 Powertrain (0x0400) — 动力总成

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 60 | turboBoost | f32 | 涡轮增压 |
| 61 | ballast | f32 | 压舱物 |
| 62 | kersCharge | f32 | KERS 充电量 |
| 63 | kersInput | f32 | KERS 输入 |
| 64 | kersCurrentKj | f32 | KERS 当前能量(kJ) |
| 65 | drs | f32 | DRS |
| 66 | tcPhysics | f32 | 牵引力控制(物理) |
| 67 | absPhysics | f32 | ABS(物理) |
| 68 | engineBrake | i32 | 发动机制动 |
| 69 | ersRecoveryLevel | i32 | ERS 回收等级 |
| 70 | ersPowerLevel | i32 | ERS 功率等级 |
| 71 | ersHeatCharging | i32 | ERS 热充电 |
| 72 | ersIsCharging | i32 | ERS 充电中 |
| 73 | drsAvailable | i32 | DRS 可用 |
| 74 | drsEnabled | i32 | DRS 启用 |
| 75 | tcInAction | i32 | TC 激活 |
| 76 | absInAction | i32 | ABS 激活 |
| 77 | autoShifterOn | i32 | 自动换挡 |
| 78 | currentMaxRpm | i32 | 当前最大转速 |
| 79 | p2pActivations | i32 | Push-to-Pass 次数 |
| 80 | p2pStatus | i32 | Push-to-Pass 状态 |
| 81 | waterTemp | f32 | 水温 |

### 7.5 Session (0x0500) — 会话状态

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 90 | status | i32 | 会话状态 |
| 91 | session | i32 | 会话类型 |
| 92 | sessionIndex | i32 | 会话索引 |
| 93 | completedLaps | i32 | 已完成圈数 |
| 94 | position | i32 | 排名 |
| 95 | sessionTimeLeft | f32 | 剩余时间 |
| 96 | numberOfLaps | i32 | 总圈数 |
| 97 | currentSectorIndex | i32 | 当前扇区 |
| 98 | normalizedCarPosition | f32 | 归一化位置 |
| 99 | isInPit | i32 | 是否在维修区 |
| 100 | isInPitLane | i32 | 是否在维修通道 |
| 101 | mandatoryPitDone | i32 | 强制进站完成 |
| 102 | missingMandatoryPits | i32 | 缺少强制进站数 |
| 103 | penaltyTime | f32 | 罚时 |
| 104 | penaltyType | i32 | 罚时类型 |
| 105 | trackStatus | bytes(4×i32) | 赛道状态标志 |
| 106 | clock | f32 | 时钟 |
| 107 | replayTimeMultiplier | f32 | 回放倍速 |
| 108 | isValidLap | i32 | 是否有效圈 |
| 109 | globalYellow | i32 | 全局黄旗 |
| 110 | globalYellow1 | i32 | 全局黄旗1 |
| 111 | globalYellow2 | i32 | 全局黄旗2 |
| 112 | globalYellow3 | i32 | 全局黄旗3 |
| 113 | globalWhite | i32 | 全局白旗 |
| 114 | globalGreen | i32 | 全局绿旗 |
| 115 | globalChequered | i32 | 全局方格旗 |
| 116 | globalRed | i32 | 全局红旗 |
| 117 | gapAheadOrTailValue | i32 | 与前车间距 |

### 7.6 Timing (0x0600) — 计时数据

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 120 | iCurrentTime | i32 | 当前圈时间(ms) |
| 121 | iLastTime | i32 | 上一圈时间(ms) |
| 122 | iBestTime | i32 | 最佳圈时间(ms) |
| 123 | iSplit | i32 | 扇区分段 |
| 124 | lastSectorTime | i32 | 上一扇区时间(ms) |
| 125 | iDeltaLapTime | i32 | 差值时间(ms) |
| 126 | isDeltaPositive | i32 | 差值是否为正 |
| 127 | iEstimatedLapTime | i32 | 估计圈时间(ms) |
| 128 | fuelEstimatedLaps | f32 | 估计剩余圈数 |
| 129 | fuelXLap | f32 | 每圈油耗 |
| 130 | usedFuel | f32 | 已用燃油 |
| 131 | distanceTraveled | f32 | 行驶距离(m) |
| 132 | currentTimeStr | bytes | 当前圈时间字符串 |
| 133 | lastTimeStr | bytes | 上一圈时间字符串 |
| 134 | bestTimeStr | bytes | 最佳圈时间字符串 |
| 135 | splitStr | bytes | 扇区分段字符串 |
| 136 | deltaLapTimeStr | bytes | 差值时间字符串 |
| 137 | estimatedLapTimeStr | bytes | 估计圈时间字符串 |
| 138 | observedSlotBeforeISplit | i32 | 观察到的分段槽位 |

### 7.7 CarState (0x0700) — 车辆状态

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 150 | carDamage | bytes(5×f32) | 车辆损伤 |
| 151 | pitLimiterOn | i32 | 维修限速激活 |
| 152 | rideHeight | bytes(2×f32) | 离地间隙 |
| 153 | ignitionOn | i32 | 点火开启 |
| 154 | starterEngineOn | i32 | 启动电机开启 |
| 155 | isEngineRunning | i32 | 引擎运行中 |
| 156 | isAiControlled | i32 | AI 控制 |
| 157 | cgHeight | f32 | 重心高度 |
| 158 | brakeBias | f32 | 刹车偏置 |
| 159 | rainLights | i32 | 雨灯 |
| 160 | flashingLights | i32 | 闪光灯 |
| 161 | lightsStage | i32 | 灯光等级 |
| 162 | wiperLv | i32 | 雨刷等级 |
| 163 | driverStintTotalTimeLeft | i32 | 车手总剩余时间(s) |
| 164 | driverStintTimeLeft | i32 | 车手剩余时间 |
| 165 | rainTyres | i32 | 雨胎 |
| 166 | currentTyreSet | i32 | 当前轮胎组 |
| 167 | strategyTyreSet | i32 | 策略轮胎组 |
| 168 | trackGripStatus | i32 | 赛道抓地力状态 |
| 169 | tyreCompoundStr | bytes | 轮胎配方字符串 |
| 170 | mfdTyreSet | i32 | MFD 轮胎组 |
| 171 | mfdFuelToAdd | f32 | MFD 添加燃油 |
| 172 | mfdTyrePressure | bytes(4×f32) | MFD 胎压 |
| 173 | idealLineOn | i32 | 理想路线显示 |
| 174 | isSetupMenuVisible | i32 | 设置菜单可见 |
| 175 | mainDisplayIndex | i32 | 主显示索引 |
| 176 | secondaryDisplayIndex | i32 | 副显示索引 |
| 177 | directionLightsLeft | i32 | 左转向灯 |
| 178 | directionLightsRight | i32 | 右转向灯 |
| 179 | tcLevel | i32 | TC 等级 |
| 180 | tcCut | i32 | TC 切断 |
| 181 | engineMap | i32 | 引擎映射 |
| 182 | absLevel | i32 | ABS 等级 |
| 183 | exhaustTemperature | f32 | 排气温度 |
| 184 | finalFf | f32 | 最终力反馈 |
| 185 | performanceMeter | f32 | 性能表 |
| 186 | kerbVibration | f32 | 路肩振动 |
| 187 | slipVibrations | f32 | 打滑振动 |
| 188 | gVibrations | f32 | G 力振动 |
| 189 | absVibrations | f32 | ABS 振动 |

### 7.8 Environment (0x0800) — 环境条件

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 220 | airDensity | f32 | 空气密度 |
| 221 | airTemp | f32 | 气温 |
| 222 | roadTemp | f32 | 路面温度 |
| 223 | windSpeed | f32 | 风速 |
| 224 | windDirection | f32 | 风向 |
| 225 | surfaceGrip | f32 | 路面抓地力 |
| 226 | rainIntensity | i32 | 降雨强度 |
| 227 | rainIntensityIn10min | i32 | 10分钟内降雨强度 |
| 228 | rainIntensityIn30min | i32 | 30分钟内降雨强度 |

### 7.9 OtherCars (0x0900) — 其他车辆

| 列 ID | 列名 | 类型 | 说明 |
|-------|------|------|------|
| 1 | sampleTick | u64 | |
| 2 | timestampNs | u64 | |
| 250 | activeCars | i32 | 活跃车辆数 |
| 251 | playerCarId | i32 | 玩家车辆 ID |
| 252 | carCoordinates | bytes | 车辆坐标数据 |
| 253 | carId | bytes | 车辆 ID 数据 |

---

## 8. 列式数据编码规则

### 8.1 固定类型编码 (CODEC_PLAIN_LE = 0)

当前唯一支持的编码。每个样本的每个列值以小端序直接存储：

- **TYPE_U64** (id=1): 每值 8 字节，小端序
- **TYPE_F32** (id=2): 每值 4 字节，IEEE 754 小端序
- **TYPE_I32** (id=3): 每值 4 字节，小端序有符号
- **TYPE_BYTES** (id=4): 变长字节序列，长度 = `lane_count × sample_count × 单元素字节数`

### 8.2 列布局

Payload 中各列数据**按列连续排列**（列式存储），每列的 `offset` 和 `byte_len` 在 `ColumnEntry` 中指定。

对于 `TYPE_BYTES` 列，`byte_len = lane_count × sample_count × element_size`:
- `lane_count` 在 ColumnEntry 中指定
- `element_size` 由上下文推断（f32 为 4, i32 为 4）

### 8.3 CRC32 校验

Chunk 的 payload_crc32 字段覆盖 **Payload 数据**（不含 Chunk Header 和 Column Entry），使用标准 CRC-32 (多项式 `0xEDB88320`)。

---

## 9. 解析流程总结

1. **读取 Header**: 验证魔数、版本号(2)、header_size(128)
2. **读取 Schema**: 定位到 `schema_offset`，验证 `SCHEMA_MAGIC` 和 `schema_hash` (0x4143544C00000002)
3. **读取 Metadata**: 定位到 `metadata_offset`，验证 `META_MAGIC`，解析赛道/车型等
4. **定位 Chunk 数据**:
   - 若 `footer_offset > 0`，从 Footer 读取 Index 偏移
   - 否则从 `first_chunk_offset` 开始逐个扫描 Chunk
5. **解析每个 Chunk**:
   - 读取 Chunk Header（验证 `CHNK` 魔数）
   - 读取 Column Entry 列表
   - 读取 Payload（验证 CRC32）
   - 根据 `cluster_id` 识别列簇类型，按 Column Entry 中的 offset/byte_len 解码各列
6. **解码列数据**: 按列式布局逐列解码，根据 value_type 确定每个值的字节宽度

---

## 10. 常量速查表

| 常量 | 值 | 说明 |
|------|------|------|
| MAGIC | `"ACTL\r\n\x1A\n"` | 文件魔数 (8 bytes) |
| FORMAT_VERSION | `2` | 格式版本 |
| HEADER_SIZE | `128` | Header 大小 |
| TIMEBASE_HZ | `1_000_000_000` | 时间基准 (ns) |
| SCHEMA_HASH | `0x4143_544c_0000_0002` | Schema 校验哈希 |
| CHUNK_MAGIC | `"CHNK"` | Chunk 魔数 (4 bytes) |
| CHUNK_HEADER_SIZE | `72` | Chunk Header 大小 |
| COLUMN_ENTRY_SIZE | `40` | Column Entry 大小 |
| INDEX_MAGIC | `"INDX"` | Index 魔数 (4 bytes) |
| FOOTER_MAGIC | `"FOOT"` | Footer 魔数 (4 bytes) |
| META_MAGIC | `"META"` | Metadata 魔数 (4 bytes) |
| SCHEMA_MAGIC | `"SCHM"` | Schema 魔数 (4 bytes) |
| TYPE_U64 | `1` | 值类型: u64 |
| TYPE_F32 | `2` | 值类型: f32 |
| TYPE_I32 | `3` | 值类型: i32 |
| TYPE_BYTES | `4` | 值类型: 变长字节 |
| CODEC_PLAIN_LE | `0` | 编码: 小端序直接存储 |
| CLUSTER_CONTROLS | `0x0100` | 列簇: 驾驶控制 |
| CLUSTER_MOTION | `0x0200` | 列簇: 车辆运动 |
| CLUSTER_TYRES | `0x0300` | 列簇: 轮胎数据 |
| CLUSTER_POWERTRAIN | `0x0400` | 列簇: 动力总成 |
| CLUSTER_SESSION | `0x0500` | 列簇: 会话状态 |
| CLUSTER_TIMING | `0x0600` | 列簇: 计时数据 |
| CLUSTER_CAR_STATE | `0x0700` | 列簇: 车辆状态 |
| CLUSTER_ENVIRONMENT | `0x0800` | 列簇: 环境条件 |
| CLUSTER_OTHER_CARS | `0x0900` | 列簇: 其他车辆 |