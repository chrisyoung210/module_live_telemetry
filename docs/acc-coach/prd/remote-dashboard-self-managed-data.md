# PRD: Remote Dashboard 数据自管理

版本: 1.1
日期: 2026-06-26
面向: Remote Dashboard 设备端开发者
依赖: `docs/acc-coach/public-protocol/dashboard-frame-distribution-protocol.md`
参考: `docs/acc-coach/public-protocol/protocol-spec.md`

---

## 0. 本次更新摘要（v1.1）

- **Chart widget 与时间完全解耦**：删除 `chartWindowS`（时间窗口）契约，chart 仅由
  采样点数 `chartSampleCount`（N）决定显示多少个点。
- **每个 chart field 新增 `defaultValue`**（元数据），作为无真实数据时预填充缓冲的 Y 值。
- **缓冲模型变更**：设备端为每个 chart field 维护一个**定长 N 的环形缓冲**，
  初始化/清空时用 `defaultValue` 预填满 N 个点 → 渲染为一条平坦线；真实帧到达后逐步覆盖。
- 字段命名统一为 `chartSampleCount`（camelCase）/ `chart_sample_count`（snake_case），
  旧名 `defaultSampleCount` / `chartWindowS` 不再作为契约使用。

## 1. 背景

acc-coach 通过 UDP 向 remote dashboard 设备实时推送 `TelemetryFrame`。
acc-coach 不做任何数据加工，每帧就是原始 `DashboardValuesFrame` values 的 JSON 序列化。
本 PRD 定义 remote dashboard 设备端如何从 UDP 流中自管理数据状态，
不依赖服务端提供历史累积或数据加工。

## 2. 数据接收

### 2.1 接收通道

| 通道 | 传输 | 端口 | 内容 |
|---|---|---|---|
| Data | UDP 单播 | `20779`(默认) | `TelemetryFrame` 序列化帧 |
| Control | TCP | `20778`(默认) | 布局配置、配对、心跳 |

Data 通道的完整 wire 格式见 `protocol-spec.md`。本节仅描述语义层的处理要求。

### 2.2 TelemetryFrame 语义结构

帧的 payload 为 JSON 时，内容为 `DashboardValuesFrame` 的 values 映射：

```json
{
  "raw:controls.speed_kmh": 243.1,
  "raw:controls.brake": 0.0,
  "raw:controls.gear": 5
}
```

**关键：每帧是稀疏帧，仅含变化的字段。**

## 3. 功能需求

### FR-01: 稀疏帧合并

收到每一帧后，与本地合并状态结合：

```
on_frame_received(frame):
    for each (field, value) in frame.values:
        merged_state[field] = value
        // 未出现在本帧中的字段保持上一次的值
```

**验收标准:**
- TextWidget 显示最新值，不出现 "--" 闪烁
- 首次收到帧前，所有 TextWidget 显示默认占位符 (如 "--")

### FR-02: Per-Field 定长环形缓冲区

为所有 chart widget 涉及的字段维护**定长**时间序列缓冲：

```
on_frame_received(frame):
    for each (field, value) in frame.values:
        buf = ring_buffer[field]
        if buf:
            buf.push({ t: frame.timestamp_ms, v: value })   // 定长 N，push 自动淘汰最旧点
```

**缓冲容量规则（新）：**
- 每个 chart field 的缓冲容量 = 引用该 field 的所有 chart widget 中
  `chartSampleCount` 的**最大值**；若没有被任何 chart widget 引用，则不创建缓冲。
- 例：某 field 被两个 chart widget 引用，`chartSampleCount` 分别为 600 和 300，
  则该 field 缓冲容量 = 600。
- 缓冲一旦创建，容量在布局热切换前保持不变；布局热切换时按新布局重新计算并重建缓冲
  （并重新预填充，见 FR-03）。
- **缓冲与时间窗口无关**：不再用 `max_window_s × hz` 估算容量，容量就是显示点数 N。

**验收标准:**
- 环形缓冲区为定长 N，新点 push 即淘汰最旧点；容量固定不增长
- 字段名查不到时返回空数组（理论上预填充后不会发生）

### FR-03: ChartWidget — 从本地定长缓冲读取并渲染

ChartWidget **与时间无关**：不再读取 `chartWindowS`，不再按时间裁剪点。固定绘制
`N = control.chartSampleCount` 个点，X 轴按采样点序号映射（第 i 个点 → x = i/(N-1) × width）。

```
render_chart(control, ring_buffers):
    N = control.chartSampleCount ?? 600        // 该 widget 固定点数
    width  = control.width
    height = control.height

    for each chart_field in control.chartFields:
        buf = ring_buffers[chart_field.fieldName]    // RingBuffer，容量 >= N
        if buf.length < N:
            // 预填充保证缓冲永不为空；此分支仅作防御
            buf.prependDefaults(N - buf.length, chart_field.defaultValue ?? 0)

        // 取最近 N 个点（按 push 顺序，最新在末尾）
        points = buf.last(N)

        // X 按 index 映射，Y 按 value 映射（沿用现有 Y 轴规则）
        for i in 0..N-1:
            x = (i / (N - 1)) * width
            y = mapValueToY(points[i].v, axisMin, axisMax)
            polyline.add(x, y)

        draw_polyline(canvas, polyline, chart_field.color)
```

**验收标准:**
- ChartWidget 按采样点序号（非时间）映射 X 轴，固定绘制 N 个点
- 不读取 `control.chartWindowS`（即使存在也忽略），chart 完全时间无关
- 缓冲永不为空（预填充保证），不再出现 "No data" 占位
- 真实帧到达后曲线从右端开始变化、左端逐步替换，平滑过渡、无闪烁
- 多个 chart widget 引用同一 field 但 N 不同时，各自取"最近 N 个点"互不影响

### FR-03-a: Chart 默认值预填充

**目的**: 在遥测尚未开始、session 刚启动未收到帧、丢包未恢复等"无真实数据"场景下，
ChartWidget 不显示空白或 "No data"，而是显示由每个 field 的 `defaultValue` 构成的
一条水平平坦线。真实数据到达后逐步覆盖。

**数据来源（元数据，由 acc-coach 在 layout 编辑阶段填写并随布局下发）:**

| 字段 | 类型 | 所属层级 | 说明 |
|---|---|---|---|
| `chartSampleCount` | `number` | `DashboardControl`（整个 chart widget） | 固定显示的采样点数 N，默认 600，范围 60–1200 |
| `chartFields[].defaultValue` | `number \| null` | `ChartFieldConfig`（每个 field 独立） | 该 field 无真实数据时的默认 Y 值，默认 0，范围 0–1，允许小数 |

**预填充逻辑:**
1. 缓冲初始化（首次收到布局、收到清空信号后重建缓冲时）：
   - 对当前激活布局中每个 chart widget 引用的 field，创建容量 = max(N) 的环形缓冲。
   - 用 N 个 `(t=0, v=defaultValue ?? 0)` **预填满**该缓冲。
2. 每收到一帧真实数据：
   - 对帧中出现的每个 chart field，`buf.push(...)`，淘汰最旧的一个默认点。
   - 帧中未出现的 field，缓冲保持不变（仍是默认平坦线）。
3. 渲染（FR-03）：始终绘制缓冲中最近 N 个点。预填充期 N 个点全是 `defaultValue` → 平坦线；
   真实点陆续入栈后，平坦线从最旧端开始被真实折线逐点替代。

**验收标准:**
- 启动/清空瞬间所有 ChartWidget 显示水平平坦线（Y = 该 field 的 `defaultValue`），而非 "No data"
- 真实帧到达后曲线从右端开始变化、左端逐步替换，平滑过渡、无闪烁
- 若 field 未配置 `defaultValue`，按 `0` 处理
- 若 chart widget 未配置 `chartSampleCount`，按 `600` 处理
- acc-coach 不参与预填充，只负责把 `defaultValue` 和 `chartSampleCount` 作为元数据下发

### FR-04: 丢包与 Sequence Gap 处理

UDP 传输可能丢包。设备端需检测并处理：

```
last_sequence = 0

on_frame_received(frame):
    if last_sequence != 0 and frame.sequence != last_sequence + 1:
        // 检测到丢包，标记数据缺口（用于绘制断点；不影响定长缓冲的 N 点显示）
        mark_data_gap()

    last_sequence = frame.sequence

    if frame.payload_type == KEYFRAME:
        clear_data_gap_flag()
        // Keyframe 包含完整状态快照，可用于纠正所有状态
```

**验收标准:**
- 丢包时不崩溃、不显示错误数据
- 检测到 gap 后，ChartWidget 在对应采样点处显示断点（因定长 N 渲染，断点出现在最近 N 个点的某个位置）
- 收到 keyframe 后 TextWidget 恢复到完整/正确状态
- 丢包期间 chart field 缓冲不增加新点（保持上一次的值或默认值），缓冲仍为定长 N

### FR-05: Keyframe 处理

服务端每 1 秒发送一次 keyframe，payload 为所有订阅字段的完整快照（非稀疏帧）。

```
on_keyframe_received(frame):
    // 直接替换 merged_state (不做稀疏合并)
    merged_state = frame.values

    // 所有字段推入缓冲区
    for each (field, value) in frame.values:
        ring_buffer[field].push({ t: frame.timestamp_ms, v: value })
```

**验收标准:**
- Keyframe 到达后，所有 TextWidget 显示的值与 keyframe 一致
- Keyframe 中的字段覆盖之前所有累积状态

### FR-06: 字段名映射

与 local dashboard 要求一致：如果 layout 中的字段名与帧中的字段名不一致，
设备端自行维护映射表。完整映射表参见
`docs/acc-coach/prd/local-dashboard-self-managed-data.md` 的 FR-06。

## 4. 非功能需求

| 需求 | 描述 |
|---|---|
| 帧处理延迟 | 单帧从 UDP socket 收到到 UI 更新 < 5ms |
| 缓冲区内存 | 每个 chart field 缓冲 = max `chartSampleCount`（典型 600，上限 1200），与时间无关 |
| 帧率 | 按 session 约定的 `accepted_hz`（最高 120Hz）处理 |
| UI 帧率 | 独立于数据接收帧率，建议 30-60 FPS |
| 断线重连 | TCP 断线后，清空所有本地状态（merged_state + ring_buffers），等待重新配对；重连成功后重新用 `defaultValue` 预填充缓冲 |

## 5. 与服务端的交互时序

```
设备端                                     acc-coach
  │                                          │
  ├─ announce (UDP 20776) ──────────────────▶│ Discover
  │                                          │
  ◀─────────────── TCP connect :20778 ───────│ Control
  │                                          │
  ◀─────────────── handshake ────────────────┤
  ├─ pairing confirm ───────────────────────▶│
  │                                          │
  ◀─────────────── layout payload ───────────┤ 布局定义
  │                                          │
  ◀── stream start (port, hz, encoding) ─────┤ 开始数据流
  │                                          │
  ◀── TelemetryFrame (snapshot) ×N ──────────┤ 稀疏帧
  ◀── TelemetryFrame (keyframe) ×1/s ────────┤ 全量快照
  ◀── TelemetryFrame (snapshot) ×N ──────────┤
  │                                          │
  ├─ stream stop ───────────────────────────▶│ 停止数据流
  │                                          │
```

设备端从收到第一个 `TelemetryFrame` 开始累积历史，到 `stream stop` 时清空。
