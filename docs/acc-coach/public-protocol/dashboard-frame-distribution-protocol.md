# ACC Coach — Dashboard 帧分发协议

版本: 1.1
日期: 2026-06-26
面向: 所有 Dashboard 模块开发者
维护方: acc-coach

> **v1.1 变更**：Chart widget 与时间解耦——删除 `chartWindowS` 时间窗口契约，
> chart 仅由采样点数 `chartSampleCount`（N）决定显示点数；每个 chart field 新增
> `defaultValue` 元数据，由 Dashboard 模块在缓冲为空时预填充为平坦线。
> acc-coach 仍只做帧分发与元数据存取，不参与预填充。

---

## 1. 角色定义

acc-coach 是 **纯帧分发层 (Frame Bus)**。它从 `module_live_telemetry` 接收原始数据帧，
不做任何加工，原样转发给所有已注册的 Dashboard 消费者。

```
module_live_telemetry
        │
        │ DashboardValuesFrame (稀疏帧)
        ▼
┌───────────────────────────────────────────────┐
│               acc-coach (帧总线)                │
│                                               │
│  保证:                                          │
│  - 帧不丢失（至少投递一次）                        │
│  - 帧内容不做任何修改                              │
│  - 帧顺序保持 (sample_tick 单调递增)              │
│                                               │
│  不做:                                          │
│  - 历史累积                                     │
│  - 字段名映射/转换                                │
│  - 稀疏帧合并                                    │
│  - 数据格式化/单位转换                             │
│  - 依据 widget 类型裁剪字段                        │
│                                               │
│  ┌─ Local 路径   ─▶ DashboardFrameBus (Rust API)│
│  └─ Remote 路径  ─▶ UDP TelemetryFrame          │
└───────────────────────────────────────────────┘
```

## 2. 帧格式

所有路径使用统一的 `DashboardValuesFrame` 结构：

```rust
pub struct DashboardValuesFrame {
    /// 单调递增的遥测样本计数
    pub sample_tick: u64,
    /// 底层遥测帧的时间戳 (自系统启动/epoch 以来的纳秒)
    pub timestamp_ns: u64,
    /// 稀疏字段映射: 字段名 → 最新数值
    /// 仅包含本 tick 发生变化的字段
    pub values: HashMap<String, f64>,
}
```

**关键特性：稀疏帧。** 每 tick 只包含值发生变化的字段。消费者收到一帧后，
应将新值与上一帧合并才能获得完整状态。

**字段名规范：** 由 `module_live_telemetry` 定义，acc-coach 不做转换。
常见字段名如 `raw:controls.speed_kmh`、`raw:controls.brake`、`raw:controls.steer_angle` 等。
Dashboard 模块如需友好名称，应自行维护映射表。

## 3. 分发路径

### 3.1 Local Dashboard 路径

```
acc-coach recording loop
    │
    │ DashboardValuesFrame
    ▼
DashboardFrameBus::push_frame(&frame)    ← module_local_dashboard 提供的 Rust API
    │
    │ Tauri managed state (跨窗口共享)
    ▼
Overlay Window
    │
    │ poll_dashboard_frame()              ← module_local_dashboard 注册的 Tauri command
    ▼
React 组件自管理历史累积
```

**传输特性：**
- 同进程内存访问，零序列化开销
- Tauri managed state 天然跨窗口
- 帧送达即立即可读

### 3.2 Network Remote Dashboard 路径

```
acc-coach recording loop
    │
    │ DashboardValuesFrame
    ▼
RemoteDashboardRuntime.drain_telemetry()
    │
    │ Serialize values → fields_json
    ▼
DataSender.broadcast(TelemetryFrame)     ← UDP 单播到各 remote session
```

**传输特性：**
- UDP 传输，允许丢帧
- 节流：按 session 的 `accepted_hz` 限制发送频率
- 每 1s 发送一次 keyframe (完整快照) 用于同步
- Wire 格式见 `protocol-spec.md`，本节仅描述语义层

**关键：remote dashboard 设备端需自行从 UDP 流中累积历史。**

### 3.3 Serial Port Dashboard 路径 (预留)

传输介质为串口，帧语义复用 `DashboardValuesFrame`。
具体编码方案和帧格式待后续单独定义。

## 4. Dashboard 模块的契约义务

所有 Dashboard 模块 (local / remote / serial) **必须自行实现**：

| 义务 | 说明 |
|---|---|
| **历史累积** | 维护 per-field 环形缓冲区，从收到的每一帧中提取字段值入栈 |
| **稀疏帧合并** | 每次收到新帧时，与上一帧合并以获得完整字段状态，避免 "--" 闪烁 |
| **字段名映射** | 如需友好显示名，自行维护 `raw_key → friendly_name` 映射表 |
| **Chart 定长缓冲 + 默认值预填充** | chart 字段缓冲为**定长 N（=`chartSampleCount`）**；初始化/清空时用 `chartFields[].defaultValue` 预填满 N 个点→平坦线；真实帧到达后逐步覆盖 |
| **(Remote 专用) 丢包处理** | 检测 sequence gap，等待下一次 keyframe 补齐 |

> 注：Chart widget **与时间无关**——不再有 `chartWindowS` 时间窗口，缓冲容量与渲染点数
> 均由 `chartSampleCount` (N) 决定，X 轴按采样点序号映射。

### 4.1 历史累积伪代码

```
收到 DashboardValuesFrame frame:
    now_ms = frame.timestamp_ns / 1_000_000

    for each (field, value) in frame.values:
        ring_buffer[field].push((now_ms, value))   // 定长 N，push 自动淘汰最旧点
```

### 4.2 Chart 默认值预填充伪代码

```
// 初始化 / 清空信号后重建 chart 缓冲时
function initializeAndPrefillChartBuffers(activeLayouts):
    for each chart_widget in activeLayouts:
        N = chart_widget.chartSampleCount ?? 600
        for each field in chart_widget.chartFields:
            capacity = max(N, existingCapacityFor(field))   // 同一字段被多 widget 引用取最大
            buf = RingBuffer(capacity)
            for i in 0..N-1:
                buf.push({ t: 0, v: field.defaultValue ?? 0 })   // 预填满 → 平坦线
            historyBuffers[field.name] = buf
```

## 5. 元数据分发

数据的完整交互分为两类，交互模式不同：

| 类别 | 频率 | 内容 | 交互模式 |
|---|---|---|---|
| **实时数据** | 每 tick (≤16ms) | `DashboardValuesFrame` (稀疏帧) | acc-coach 推送，dashboard 接收并自管理 |
| **元数据** | 按需 (用户编辑/赛道切换) | 布局定义、叠层配置、赛道地图、字体 | acc-coach 提供查询 IPC，dashboard 自行调用加载 |

### 5.1 元数据：acc-coach 提供查询 IPC

元数据由 acc-coach 存储和管理，Dashboard 模块通过 Tauri IPC 自行查询加载。
acc-coach **只负责存取，不做格式转换或字段适配**。

| 数据 | IPC 命令 | 返回类型 |
|---|---|---|
| 叠层配置 | `get_local_dashboard_overlay_config` | `LocalDashboardOverlayConfig` |
| 已注册布局列表 | `list_registered_dashboard_layouts` | `RegisteredDashboardLayout[]` |
| 单个布局 | `get_registered_dashboard_layout(id)` | `RegisteredDashboardLayout \| null` |
| 赛道地图 (按 ID) | `get_track_map(trackId)` | `TrackMapRecord \| null` |
| 赛道地图 (按名称) | `resolve_track_map(trackName)` | `TrackMapRecord \| null` |
| 字体资源 | (待定，当前通过 layout payload 内嵌 base64) | -- |

**约束：**
- Dashboard 模块不应从 acc-coach 的 React props 接收布局数据
- Dashboard 模块自行调用 IPC 加载元数据
- 布局中的控件字段使用协议层类型 (`DashboardControl`)，不做任何转换
- 若布局中的数据格式与 Dashboard 内部格式不一致，Dashboard 自行适配

### 5.2 元数据：布局定义结构

```rust
// acc-coach 存储并查询返回
pub struct RegisteredDashboardLayout {
    pub layout_id: String,
    pub name: String,
    pub registered_at: String,
    pub layout: DashboardLayoutPayload,   // 来自 module_dashboard_protocol
}

pub struct DashboardLayoutPayload {
    pub canvas_width: f64,
    pub canvas_height: f64,
    pub image_mime: String,
    pub static_image_base64: String,
    pub controls: Vec<DashboardControl>,   // 统一 controls 数组
}
```

`DashboardControl` 包含每个 widget 的完整定义：
- `id`, `widgetType`, `x`, `y`, `width`, `height` — 位置与类型
- `telemetryField`, `textTemplate`, `format` — 数据绑定
- `chartFields[]`（每项含 `fieldName`, `color`, `defaultValue`）、`chartSampleCount` — Chart 配置（**与时间无关，无 `chartWindowS`**）
- `trackId`, `dotColor`, `dotSize` — Map 配置
- `conditionalRules[]`, `font`, `fontSize`, `textColor`, `backgroundColor` — 外观

**Chart 相关字段说明（v1.1 契约）:**

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `chartSampleCount` | `number` | `600` | 固定显示的采样点数 N（缓冲容量）；chart widget 与时间无关 |
| `chartFields[].defaultValue` | `number \| null` | `0` | 该 field 无真实数据时的默认 Y 值，用于预填充平坦线 |
| ~~`chartWindowS`~~ | — | — | **已废弃**，新版 Dashboard 模块应忽略该字段即使存在 |
| ~~`defaultSampleCount`~~ | — | — | 旧名，归一为 `chartSampleCount`；旧布局缺失时按 `600` 兜底 |

完整定义见 `module_dashboard_protocol/src/lib.rs`: `DashboardControl`。

### 5.3 元数据加载时序 (Local Dashboard)

```
Overlay 窗口挂载
    │
    ├─ invoke("get_local_dashboard_overlay_config")  ──▶ 叠层设置
    ├─ invoke("list_registered_dashboard_layouts")   ──▶ 所有布局
    │
    │ 根据 config.regions 中启用的 layoutId，
    │ 在布局列表中匹配对应布局，获取其 controls 列表
    │
    ├─ 对每个 Map control 的 trackId:
    │   invoke("get_track_map", { trackId })          ──▶ 赛道几何数据
    │
    └─ 开始渲染: 用 controls 定义确定画布位置/大小/外观
                    用实时帧确定控件内容
                    用本地缓冲区为 ChartWidget 提供历史
```

所有 `invoke()` 调用由 Dashboard 模块自行发起，不依赖 acc-coach 的中间代码。

### 5.4 元数据加载时序 (Remote Dashboard)

```
TCP Control 通道建立
    │
    ◀── layout payload (JSON) ── acc-coach 推送
    │
    │ 设备端解析布局，提取 widgets 定义
    │
    ├─ 字体/资源通过 TCP asset 通道下载
    │
    └─ 开始渲染: UDP 流接收实时帧 + 本地历史累积
```

详见 `protocol-spec.md`，本节仅说明元数据不经过 acc-coach 转换。

## 6. acc-coach 的接口清单

### 6.1 提供给 Local Dashboard 的接口

| 接口 | 提供方 | 形式 | 性质 |
|---|---|---|---|
| `DashboardFrameBus::push_frame()` | `module_local_dashboard` | Rust struct API | 实时数据写入 |
| `poll_dashboard_frame()` | `module_local_dashboard` | Tauri command | 实时数据读取 |
| `get_local_dashboard_overlay_config` | acc-coach | Tauri command | 元数据查询 |
| `list_registered_dashboard_layouts` | acc-coach | Tauri command | 元数据查询 |
| `get_registered_dashboard_layout(id)` | acc-coach | Tauri command | 元数据查询 |
| `get_track_map(trackId)` | acc-coach | Tauri command | 元数据查询 |
| `resolve_track_map(trackName)` | acc-coach | Tauri command | 元数据查询 |

### 6.2 提供给 Remote Dashboard 的接口

| 接口 | 提供方 | 形式 | 性质 |
|---|---|---|---|
| TelemetryFrame 流 | acc-coach `RemoteDashboardRuntime` | UDP 单播 | 实时数据推送 |
| 布局配置 | acc-coach | TCP control channel | 元数据推送 |
| 资源文件 (字体等) | acc-coach | TCP asset 通道 | 元数据推送 |

### 6.3 acc-coach 的 Thin Shell 职责

acc-coach 的 `LocalDashboardOverlayWindow.tsx` 仅保留窗口级职责：

| 职责 | 说明 |
|---|---|
| 窗口生命周期 | show/hide, bounds 跟踪, click-through |
| ACC 窗口跟随 | 检测 ACC 窗口位置，调整 overlay bounds |
| 运行时上下文 | 决定 `visible` 状态（基于 config + recording status + preview mode） |
| 视口尺寸 | 计算 `viewportWidth/Height`，传给渲染组件 |

**明确不做的**：布局加载、字段映射、帧合并、历史累积、track map 加载、类型转换。

## 7. 迁移对照

## 7. 迁移对照

| 原 acc-coach 职责 | 迁移到 |
|---|---|
| `TelemetryHistory` 环形缓冲区 | 各 dashboard 模块自维护 |
| `get_live_dashboard_frame_with_history()` | 删除，dashboard 模块从自有缓冲区读取 |
| `friendly_to_raw_keys()` 字段名映射 | 各 dashboard 模块自维护 |
| `overlayControl()` 字段名转换 + 类型适配 | `module_local_dashboard` 内部处理 |
| `overlayLayout()` / `overlayLayouts()` 布局转换 | `module_local_dashboard` 内部处理 |
| 稀疏帧合并 (`{...prev, ...nextFrame}`) | 各 dashboard 模块前端处理 |
| `get_live_dashboard_frame()` | 替换为 `poll_dashboard_frame()`，由 `module_local_dashboard` 提供 |
| 布局加载 (`list_registered_dashboard_layouts`) | 由 `module_local_dashboard` 自行调用 IPC |
| Track map 加载 (`get_track_map`) | 由 `module_local_dashboard` 自行调用 IPC |
| 布局作为 React props 传递 | 删除，Dashboard 自行加载
