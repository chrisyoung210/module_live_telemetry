# PRD: module_local_dashboard 数据自管理

版本: 1.2
日期: 2026-06-26
面向: module_local_dashboard 开发者
依赖: `docs/acc-coach/public-protocol/dashboard-frame-distribution-protocol.md`

---

## 0. 本次更新摘要（v1.2）

- **Chart widget 与时间完全解耦**：删除 `chartWindowS`（时间窗口）契约，chart 仅由
  采样点数 `chartSampleCount`（N）决定显示多少个点。
- **每个 chart field 新增 `defaultValue`**（元数据），作为无真实数据时预填充缓冲的 Y 值。
- **缓冲模型变更**：module_local_dashboard 为每个 chart field 维护一个**定长 N 的环形缓冲**，
  初始化/清空时用 `defaultValue` 预填满 N 个点 → 渲染为一条平坦线；真实帧到达后逐步覆盖。
- 字段命名统一为 `chartSampleCount`（camelCase）/ `chart_sample_count`（snake_case），
  旧名 `defaultSampleCount` / `chartWindowS` 不再作为契约使用。

## 1. 背景

当前架构中，acc-coach 主模块承担了 dashboard 数据的缓存、累积和历史查询工作
（`TelemetryHistory`、`get_live_dashboard_frame_with_history()` 等）。
这违反了"数据分发"原则——acc-coach 应该只做分发，不做数据加工。

本次改造将数据管理职责下沉到 `module_local_dashboard`，使其自行从原始帧流中
管理状态，acc-coach 仅负责投递原始帧。

## 2. 功能需求

### FR-01: DashboardFrameBus — 帧接收器 (Rust)

**目的**: 接收 acc-coach recording loop 推送的每一帧原始数据。

**接口定义:**

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use module_dashboard_protocol::DashboardValuesFrame;

/// 线程安全的最新帧缓存。acc-coach 写入，overlay 窗口读取。
pub struct DashboardFrameBus {
    inner: Mutex<Option<DashboardValuesFrame>>,
}

impl DashboardFrameBus {
    /// 创建空缓存
    pub fn new() -> Self;

    /// acc-coach 调用：存入最新原始帧
    pub fn push_frame(&self, frame: &DashboardValuesFrame);

    /// overlay 窗口调用：读取最新帧（用于 poll）
    /// 返回 None 表示尚无数据，也用作清空信号
    pub fn latest_frame(&self) -> Option<DashboardValuesFrame>;

    /// 会话复位时清空
    pub fn clear(&self);
}
```

**验收标准:**
- `push_frame()` 线程安全，可被 recording loop 高频调用
- `latest_frame()` 总是返回最新一帧，不会丢帧
- `clear()` 后 `latest_frame()` 返回 `None`

### FR-02: Tauri Command — poll_dashboard_frame

**目的**: overlay 窗口的前端代码通过此命令获取最新帧。

```rust
#[tauri::command]
fn poll_dashboard_frame(bus: tauri::State<'_, DashboardFrameBus>)
    -> Option<DashboardValuesFrame>;
```

**验收标准:**
- 每次调用返回自上次调用以来最新的一帧
- 跨窗口可访问（recording loop 在主窗口写入，overlay 窗口读取）

### FR-03: setup() 增强 — 注册 DashboardFrameBus

**目的**: 在 `setup(app)` 中创建 `DashboardFrameBus` 实例并注册为 Tauri managed state。

```rust
pub fn setup(app: &tauri::App) -> Result<(), String> {
    // ... 现有 overlay 窗口创建逻辑保持不变 ...

    let bus = DashboardFrameBus::new();
    app.manage(bus);

    Ok(())
}
```

**验收标准:**
- `setup(app)` 调用后，可通过 `app.state::<DashboardFrameBus>()` 获取实例
- 已有的窗口创建逻辑不受影响

### FR-04: 前端 — 帧轮询与稀疏帧合并 (TypeScript/React)

**目的**: overlay 窗口定期拉取最新帧，合并到本地完整状态，并维护 chart 字段的定长历史缓冲。

**位置**: `LocalDashboardOverlay.tsx` 或新建的 data hook (`useDashboardFrame.ts`)

**逻辑:**

```typescript
// 每 tick 行为
const rawFrame = await invoke<DashboardValuesFrame | null>("poll_dashboard_frame");
if (rawFrame === null) {
    // 清空信号：见 FR-04-a
    return;
}

// 稀疏帧合并: 用新值覆盖旧值，旧字段保持不变
setFullFrame(prev => ({ ...prev, ...rawFrame.values }));

// 历史累积: 每个 chart 字段推入定长环形缓冲区
const tsMs = rawFrame.timestampNs / 1_000_000;
for (const [field, value] of Object.entries(rawFrame.values)) {
    const buf = historyBuffers.current.get(field);
    if (buf) {
        buf.push({ t: tsMs, v: value });   // push 自动淘汰最旧的一个
    }
}
```

**环形缓冲区容量规则（新）:**
- 每个 chart field 的缓冲容量 = 引用该 field 的所有 chart widget 中
  `chartSampleCount` 的**最大值**；若没有被任何 chart widget 引用，则不创建缓冲。
- 例：某 field 被两个 chart widget 引用，`chartSampleCount` 分别为 600 和 300，
  则该 field 缓冲容量 = 600。
- 缓冲一旦创建，容量在布局热切换前保持不变；布局热切换时按新布局重新计算并重建缓冲
  （见 FR-08）。
- **缓冲与时间窗口无关**：不再用 `maxWindowS * Hz` 估算容量，容量就是显示点数 N。

**验收标准:**
- 收到稀疏帧后，上一帧中未出现的字段值保留，不出现 "--" 闪烁
- 环形缓冲区为定长 N，新点 push 即淘汰最旧点
- 缓冲区容量 = max `chartSampleCount`，缓冲真实点（含默认值预填充点）
- 圈数/会话切换时缓冲区清空并重新预填充（见 FR-04-a、FR-07）

### FR-04-a: 缓冲区清空信号

**目的**: 明确 module_local_dashboard 何时应重置历史缓冲和合并帧状态，
并在重置后用 `defaultValue` 重新预填充缓冲。

**信号机制**: `poll_dashboard_frame` 返回 `None` 即为清空信号。

当以下事件发生时，acc-coach 的 recording loop 调用 `bus.clear()`，
`poll_dashboard_frame` 下一次调用返回 `None`：

| 事件 | 触发时机 |
|---|---|
| Recording 停止 (Stop) | 录制管线关闭 |
| Subscription 更换 (ReplaceDashboardItems) | 用户切换布局、增减字段 |
| 圈数切换 | recording loop 重新启动时 bus 被 clear |

**module_local_dashboard 的处理逻辑:**

```typescript
const rawFrame = await invoke<DashboardValuesFrame | null>("poll_dashboard_frame");
if (rawFrame === null) {
    // 清空信号：重置所有状态
    historyBuffers.current.clear();  // 清空所有字段的环形缓冲
    setFullFrame(null);               // 重置合并帧状态
    // 重新创建并预填充缓冲（见 FR-07）：
    //   对每个 chart field，用 defaultValue 填满 N 个点
    initializeAndPrefillChartBuffers();
    return;
}
// ... 正常帧处理 ...
```

**验收标准:**
- 收到 `null` 后，所有 ChartWidget 重新显示由 `defaultValue` 构成的平坦线（不是空白 "No data"）
- 下一次收到真实帧时，新点逐步覆盖默认点，不残留旧的真实数据
- TextWidget 在首次收到帧前显示占位符（如 "--"）

### FR-05: ChartWidget — 从本地定长缓冲读取并渲染

**目的**: ChartWidget 不再从外部 props 接收预先计算好的 `FieldHistory[]`，
改为从本地定长环形缓冲读取，固定绘制 `chartSampleCount` 个点。

**接口变更:**

```typescript
// 当前 (改造前): ChartWidget 从外部 props 接收预计算的历史数组
interface ChartWidgetProps {
    control: DashboardControl;
    history: FieldHistory[];  // ← 外部传入
}

// 目标 (改造后): ChartWidget 从共享的本地定长缓冲读取
interface ChartWidgetProps {
    control: DashboardControl;
    historyBuffer: Map<string, RingBuffer<{ t: number; v: number }>>;
}
```

**渲染模型（关键变更）:**

- Chart widget **与时间无关**：不再有 `chartWindowS` 时间窗口，不再按时间裁剪点。
- 固定绘制 `N = control.chartSampleCount` 个点。
- X 轴按**采样点序号**线性映射：第 `i` 个点（0-indexed）→ x = `(i / (N - 1)) * canvasWidth`。
  当 N == 1 时退化为单像素点。
- Y 轴按值映射（沿用现有 Y 轴规则），值 = 该缓冲槽位的 `v`。

**绘制伪代码:**

```
render_chart(control, historyBuffer):
    N = control.chartSampleCount ?? 600        // 该 widget 固定点数
    width  = control.width
    height = control.height

    for each chart_field in control.chartFields:
        buf = historyBuffer[chart_field.fieldName]   // RingBuffer，容量 >= N
        if buf.length < N:
            // 缓冲尚未填满：用 defaultValue 补齐末尾（理论上预填充后不会发生，
            // 此分支作为防御，保证永不出现 "No data"）
            missing = N - buf.length
            buf.prependDefaults(missing, chart_field.defaultValue ?? 0)

        // 取最近 N 个点（按 push 顺序，最新在末尾）
        points = buf.last(N)

        // X 按 index 映射，Y 按 value 映射
        for i in 0..N-1:
            x = (i / (N - 1)) * width
            y = mapValueToY(points[i].v, axisMin, axisMax)
            polyline.add(x, y)

        draw_polyline(canvas, polyline, chart_field.color)

    // 注意：不再有 "所有 chartFields 均无数据时显示 No data" 的分支——
    // 预填充保证缓冲永不为空。
```

**验收标准:**
- ChartWidget 按采样点序号（非时间）映射 X 轴，固定绘制 N 个点
- 不读取 `control.chartWindowS`（即使存在也忽略），chart 完全时间无关
- 缓冲永不为空（预填充保证），不再出现 "No data" 占位
- Canvas 绘制逻辑（Y 轴映射、polyline）沿用现有实现
- 多个 chart widget 引用同一 field 但 N 不同时，各自取"最近 N 个点"互不影响

### FR-06: 字段名映射

**目的**: 如果 layout 中的 `chartFields[].fieldName` 与 `DashboardValuesFrame.values`
中的 key 不一致，local_dashboard 自行维护映射。

**当前已知需映射的 key:**

| 友好名 | Raw Key(s) |
|---|---|
| `speedKmh` | `raw:controls.speed_kmh`, `speed_kmh` |
| `throttlePct` | `raw:controls.gas`, `gas` |
| `brakePct` | `raw:controls.brake`, `brake` |
| `clutch` | `raw:controls.clutch`, `clutch` |
| `steerRawAngle` | `raw:controls.steer_angle`, `steer_angle` |
| `steeringDeg` | `raw:controls.steer_angle`, `steer_angle` |
| `gear` | `raw:controls.gear`, `gear` |
| `rpm` | `raw:controls.rpms`, `rpms` |
| `fuel` | `raw:controls.fuel`, `fuel` |
| `absLevel` | `raw:car_state.abs_level` |
| `tcLevel` | `raw:car_state.tc_level` |
| `absInAction` | `raw:powertrain.abs_in_action` |
| `tcInAction` | `raw:powertrain.tc_in_action` |
| `isValidLap` | `raw:session.is_valid_lap` |
| `currentLapTimeMs` | `raw:timing.i_current_time` |
| `lastLapTimeMs` | `raw:timing.i_last_time` |
| `bestLapTimeMs` / `sessionBestLapTimeMs` | `raw:timing.i_best_time` |
| `bestLapDeltaTimeMs` / `sessionLapDeltaTimeMs` | `raw:timing.i_delta_lap_time` |
| `predictedLapTimeByBest` / `predictedLapTimeBySession` | `raw:timing.i_estimated_lap_time` |
| `carX` | `calc:car_x` |
| `carZ` | `calc:car_z` |
| `speedMs` | `speed_mps` |

**验收标准:**
- ChartWidget 和 TextWidget 能通过多种 key 格式查找对应数据
- 映射表可配置/扩展

### FR-07: Chart 默认值预填充（核心：defaultValue → 平坦线）

**目的**: 在遥测未开始、session 刚启动尚未收到真实帧、replay 未开始、auto-recording 未运行
等"无真实数据"场景下，ChartWidget 不显示空白或 "No data"，而是显示由每个 field 的
`defaultValue` 构成的一条水平平坦线。真实数据到达后逐步覆盖。

**数据来源（元数据，由 acc-coach 在 layout 编辑阶段填写并随布局下发）:**

| 字段 | 类型 | 所属层级 | 说明 |
|---|---|---|---|
| `chartSampleCount` | `number` | `DashboardControl`（整个 chart widget） | 固定显示的采样点数 N，如 600 |
| `chartFields[].defaultValue` | `number \| null` | `ChartFieldConfig`（每个 field 独立） | 该 field 无真实数据时的默认 Y 值，如 gas=0、brake=0 |

**预填充逻辑:**

1. 缓冲初始化（overlay 启动、布局加载、收到清空信号后重建缓冲时）：
   - 对当前激活布局中每个 chart widget 引用的 field，创建容量 = N 的环形缓冲。
   - 用 N 个 `(t=0, v=defaultValue ?? 0)` **预填满**该缓冲。
2. 每收到一帧真实数据：
   - 对帧中出现的每个 chart field，`buf.push({ t: tsMs, v: value })`，
     淘汰最旧的一个默认点。
   - 帧中未出现的 field，缓冲保持不变（仍是默认平坦线）。
3. 渲染（FR-05）：始终绘制缓冲中最近 N 个点。预填充期 N 个点全是 `defaultValue` → 平坦线；
   真实点陆续入栈后，平坦线从最旧端开始被真实折线逐点替代。

**时序示例（N=600，defaultValue=0）:**

```
t=0       缓冲 = [0,0,0,...,0] (600 个 0)  → 渲染：水平线 (Y=0)
t=16ms    收到 1 个真实点 v=0.3            → 缓冲 = [0,0,...,0, 0.3]
          渲染：最右 1 个点抬起，其余仍为 0
t=32ms    收到第 2 个真实点 v=0.5           → 缓冲 = [0,0,...,0, 0.3, 0.5]
...       满 600 个真实点后，默认点全部被淘汰，整条曲线为真实数据
```

**验收标准:**
- 启动/清空瞬间所有 ChartWidget 显示水平平坦线（Y = 该 field 的 `defaultValue`），而非 "No data"
- 真实帧到达后曲线从右端开始变化、左端逐步替换，平滑过渡、无闪烁
- 若某 field 未配置 `defaultValue`，按 `0` 处理
- 若 chart widget 未配置 `chartSampleCount`，按 `600` 处理
- acc-coach 不参与预填充，只负责把 `defaultValue` 和 `chartSampleCount` 作为元数据下发

### FR-08: 元数据自加载 — 布局与叠层配置

**目的**: `LocalDashboardOverlay` 组件不再从 React props 接收布局和配置数据，
而是自行通过 Tauri IPC 加载。

**需调用的 IPC 命令:**

| 命令 | 用途 | 调用时机 |
|---|---|---|
| `get_local_dashboard_overlay_config` | 获取叠层配置（区域列表、每个区域的 layoutId） | overlay 窗口挂载时 |
| `list_registered_dashboard_layouts` | 获取所有已注册的布局定义 | overlay 窗口挂载时、窗口聚焦时（用户可能在其他窗口编辑了布局） |

**逻辑:**

```typescript
// LocalDashboardOverlay 组件内部
useEffect(() => {
    const [config, layouts] = await Promise.all([
        invoke("get_local_dashboard_overlay_config"),
        invoke("list_registered_dashboard_layouts"),
    ]);

    // 根据 config.regions 中启用的 region，
    // 在 layouts 列表中匹配对应的 layoutId
    const activeLayouts = config.regions
        .filter(r => r.enabled)
        .map(r => ({
            region: r,
            layout: layouts.find(l => l.layoutId === r.layoutId),
        }))
        .filter(item => item.layout);
    
    setActiveLayouts(activeLayouts);

    // 布局确定后，按其中所有 chart widget 的 chartSampleCount 最大值
    // 重新计算各 field 的缓冲容量并预填充（见 FR-07）
    rebuildChartBuffersFromLayouts(activeLayouts);
}, []);
```

**窗口聚焦时重新加载布局:**
```typescript
useEffect(() => {
    const onFocus = () => {
        // 用户可能在其他窗口编辑了布局，重新加载
        invoke("list_registered_dashboard_layouts").then(layouts => {
            setLayouts(layouts);
            rebuildChartBuffersFromLayouts(/* ... */);
        });
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
}, []);
```

**验收标准:**
- overlay 窗口启动后能正确显示所有已启用区域的布局
- 非启用区域不渲染
- 切换窗口后布局自动刷新（用户在其他窗口编辑布局后切回来能看到更新）
- 布局加载/热切换后，各 chart field 缓冲容量按新布局的 `chartSampleCount` 重新计算并用 `defaultValue` 预填充
- 从 props 中移除 `config`、`layouts` 等元数据传递

### FR-09: 元数据自加载 — 赛道地图

**目的**: MapWidget 需要的赛道几何数据，由 local_dashboard 自行加载。

**需调用的 IPC 命令:**

| 命令 | 用途 | 调用时机 |
|---|---|---|
| `get_track_map(trackId)` | 按 trackId 获取赛道几何数据 | Map control 出现时（trackId 已知） |
| `resolve_track_map(trackName)` | 按 trackName 查找赛道（动态解析） | 录制/回放中根据赛道名解析 |

**逻辑:**

```typescript
useEffect(() => {
    // 遍历所有 Map control，加载其 trackId 对应的地图
    for (const control of mapControls) {
        if (control.trackId) {
            const record = await invoke("get_track_map", { trackId: control.trackId });
            if (record?.pointsJson) {
                const points = JSON.parse(record.pointsJson);
                trackPointsCache.set(control.trackId, {
                    points,
                    angleDeg: record.angleDeg ?? 0,
                    flipX: record.flipX ?? 1,
                    flipZ: record.flipZ ?? 1,
                });
            }
        }
    }
}, [mapControls]);
```

**验收标准:**
- MapWidget 能正确根据 trackId 加载并显示赛道轨迹
- Map control 切换 trackId 时,新赛道地图自动加载
- 从 props 中移除 `trackPoints`、`sessionTrackName` 等元数据传递

### FR-10: 布局数据适配

**目的**: acc-coach 的 IPC (`list_registered_dashboard_layouts`) 返回的 `DashboardLayoutPayload`
使用协议层类型（与 `module_dashboard_protocol` 一致）。如果 local_dashboard 内部使用了
不同的类型或字段命名，需自行完成适配。

**当前已知需处理的差异:**

| 协议层 (Rust serde camelCase) | 可能是旧格式 |
|---|---|
| `controls: DashboardControl[]` (统一数组) | `staticControls[]` + `dynamicControls[]` (分离数组) |
| `widgetType` (string enum) | `isDynamic` (boolean) |
| `chartFields[].fieldName` (string) | 历史代码中名为 `telemetryField` |
| `chartSampleCount` (number) | 旧名 `defaultSampleCount`；如收到旧名应归一为 `chartSampleCount` |
| `chartFields[].defaultValue` (number \| null) | 新增字段，旧布局可能缺失 → 视为 `0` |

**验收标准:**
- 兼容新旧两种布局 JSON 格式
- `normalizeLayoutPayload` (已存在于 `module_dashboard_protocol/types`) 用于规范化
- 适配逻辑独立模块，不侵入渲染代码
- 对缺失 `chartSampleCount` / `defaultValue` 的旧布局按默认值（600 / 0）兜底

## 3. 非功能需求

| 需求 | 描述 |
|---|---|
| 缓冲区容量 | 每个 chart field 缓冲 = max `chartSampleCount`（典型 600），**与时间无关** |
| 轮询间隔 | overlay 窗口 ≥ 16ms（60Hz），受 UI 线程调度影响 |
| 内存上限 | 所有字段缓冲区总和 < 10MB |
| 跨窗口安全 | `DashboardFrameBus` 在 Tauri managed state 中，任意窗口可访问 |

## 4. 现有接口保持不变

以下现有 API 保持不变：

- `LocalDashboardOverlayConfig` 配置加载/保存
- `OverlayRegionConfig` 区域配置
- 窗口管理函数 (`show/hide/set_bounds/set_click_through`)
- `setup()` 函数签名 (内部新增 `DashboardFrameBus` 注册)
- React 组件导出列表 (内部 props 变更，外部导入路径不变)

## 5. Cargo.toml 新增依赖

```toml
[dependencies]
# 新增: 帧数据结构由共享协议包定义
module_dashboard_protocol = { path = "../module_dashboard_protocol" }
```

## 6. 补充说明

### 6.1 Tauri command 注册归属

`DashboardFrameBus` (Rust struct) 由 `module_local_dashboard` 在 `setup(app)` 中创建并注册为 managed state (`app.manage(bus)`)。
`poll_dashboard_frame` (Tauri command) **由 acc-coach 注册**（`src/ipc/mod.rs`），acc-coach 通过 `State<'_, DashboardFrameBus>` 访问 managed state 实例。

module_local_dashboard **不需要注册任何 Tauri command**。Tauri 的 command 必须在 builder 阶段注册，`setup()` 中无法动态添加。

### 6.2 窗口管理命令归属

`overlayConfigApi.ts` 中的所有命令（`get/save_local_dashboard_overlay_config`、`show/hide/bounds/click_through`、`get_acc_window_bounds`）仍然由 acc-coach 注册。module_local_dashboard 不接管窗口管理职责。

### 6.3 GearSmootherState 位置

`GearSmootherState` 及其相关函数（`smoothGear`、`createInitialGearSmootherState`）已在 `module_local_dashboard/src-ui/.../telemetryFormat.ts` 中定义。
改造后 module_local_dashboard 内部自维护，从 `DashboardValuesFrame` 中自行读取 `gear` 字段并平滑。acc-coach 不再参与任何 gear 状态管理。

### 6.4 关于 chart 元数据的职责边界

- **acc-coach 负责**：在 Dashboard Designer 的 chart widget 属性面板提供 `chartSampleCount`
  输入框、为每个 chart field 提供 `defaultValue` 输入框，并随布局 JSON 下发。
  acc-coach **不**做预填充、**不**维护历史、**不**按时间窗口裁剪。
- **module_local_dashboard 负责**：读取元数据中的 `chartSampleCount` 与
  `chartFields[].defaultValue`，建立定长缓冲并用 `defaultValue` 预填充，
  在真实帧到达后逐步替换预填充点并渲染。

## 7. 验收用例

1. **基础帧分发**: 启动 recording 后，overlay 窗口的 `TextWidget` 能实时显示速度/档位等字段值，无 "--" 闪烁
2. **Chart 历史累积**: 驾驶一段后，ChartWidget 显示真实折线；缓冲恒为定长 N，不随时间增长
3. **默认值预填充（新增重点）**: 新布局加载瞬间 / session 刚启动 / preview 模式 / 收到清空信号后，
   ChartWidget 立刻显示水平平坦线（Y = `defaultValue`），**不是** "No data"
4. **真实数据覆盖**: 真实帧到达后平坦线从右端开始被真实点逐个替代，左端逐步更新，无闪烁
5. **无时间窗口**: 即使驾驶 30 分钟，ChartWidget 仍只显示最近 N 个点（N 由 `chartSampleCount` 决定），
   不随驾驶时长变长
6. **圈数切换**: 完成一圈后缓冲区清空并重新用 `defaultValue` 预填充，新圈真实数据到达后逐步覆盖
7. **布局热切换**: 切换到不同 `chartSampleCount` 的布局后，缓冲按新 N 重建并预填充
8. **字段名映射**: 使用 `speedKmh` 等友好名的 layout 能正确匹配 `raw:controls.speed_kmh` 等 raw key
9. **元数据自加载**: overlay 窗口挂载时自行加载布局和配置，不依赖 acc-coach 的 React props 传入布局数据
10. **布局刷新**: 在 DashboardDesignerView 中编辑布局后，切回 overlay 窗口看到更新
11. **赛道地图自加载**: Map control 指定 trackId 后，地图自动加载并显示
12. **缺失元数据兜底**: 旧布局未填 `chartSampleCount` / `defaultValue` 时，按 600 / 0 兜底，不报错