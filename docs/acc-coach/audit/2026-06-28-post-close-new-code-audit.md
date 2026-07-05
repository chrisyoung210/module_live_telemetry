# ACC Coach 审计关闭后新增代码审计报告

- 审计日期：2026-06-28
- 审计基线：上一轮已关闭审计报告的关闭 commit `51f248c`（关闭记录见 `2026-06-18-network-remote-dashboard-audit.md` 第二次复审结论）
- 审计范围：`51f248c` 之后的所有已提交变更（9 个 commit：`f613ee4`..`ba6f6d1`）+ 工作区未提交变更。共约 9600+ 行新增，跨 60+ 文件。
- 变更性质：Dashboard 架构大重构 — 引入别名层、DashboardFrameBus、DashboardFeed/Transport 抽象、track map 自动采集、ACCTLM2 回放、text widget 表达式函数、layout 热重载。
- 结论：本轮新增代码存在 1 个 CRITICAL、5 个 HIGH、7 个 MEDIUM、5 个 LOW 问题。`cargo check`、`cargo test`、`npm run lint` 通过；`cargo clippy --all-targets --all-features -- -D warnings` 未通过（18 个 error）；`npm test` 未通过（1 个测试失败）；`npm run build` 未通过（TypeScript 类型错误）。
- **关闭状态：✅ CLOSED — 已关闭。关闭 commit：`8179427`。**

## 审计背景

上一轮审计（`2026-06-18-network-remote-dashboard-audit.md`）于 commit `51f248c` 关闭，关闭时修复了远程 dashboard 四条数据流断裂、clippy 门禁、旧系统清理等问题。

本轮变更在 `51f248c` 之后进行了 **dashboard 架构级重构**，核心变更：

1. **别名层**（`src/dashboard/alias.rs`，291 行）：从 `raw_catalog` 构建规范格式 ↔ 用户格式的唯一翻译点，取代原先散布于 writer.rs / output.rs / ipc/mod.rs / auto.rs 的 20+ 处硬编码别名表。
2. **DashboardFrameBus**（`module_local_dashboard` 提供，acc-coach 集成）：录制循环将帧推送到 bus，overlay 窗口通过 `poll_dashboard_frame` IPC 轮询。实现 local dashboard 数据自管理架构。
3. **DashboardFeed / DashboardTransport**（`src/dashboard/feed.rs`、`transport.rs`，442 行）：传输层抽象，含 MemoryTransport 和 SerialTransportStub。
4. **Track map 自动采集**（`src/recording/track_map.rs`，481 行）：通过 `LapCompletedCallback` 在有效圈完成时自动采集赛道坐标，存入 DB。新增 v9/v10/v11 三个 DB migration。
5. **ACCTLM2 回放**（`src/recording/replay.rs`，312 行）：通过 `RecordingController::start_replay_with_latest_dashboard` 回放录制文件，帧经 `poll_dashboard_frame` / `get_live_dashboard_frame` 输出。
6. **Text widget 表达式函数**（`DashboardDesignerView.tsx`，未提交）：新增 `abs`、`round` 内置函数，扩展表达式求值器。
7. **Layout 热重载**（`LocalDashboardOverlayWindow.tsx` + `LocalDashboardView.tsx`，未提交）：通过 localStorage 版本号触发 overlay 窗口重载布局。
8. **协议类型对齐**（`src/dashboard/layout.rs`）：re-export `module_dashboard_protocol` 类型，`text` → `textTemplate` 字段重命名，`DashboardLayoutPayload` 自定义 Serialize 兼容新旧格式。
9. **TelemetryPoint 扩展**（`src/types/telemetry.rs`）：新增 `world_pos_x`/`world_pos_z` 字段。

本审计评估新架构的完整性、数据流连通性、构建状态和代码质量。

## 验证结果

已执行：

| 命令 | 结果 |
|---|---|
| `cargo check` | ✅ 通过 |
| `cargo test` | ✅ 通过（130 单元测试 + e2e_pipeline、lap_stats、real_ld、regression_monza） |
| `cargo clippy --all-targets --all-features -- -D warnings` | ❌ **失败**，8 个 lib error + 18 个 lib test error，见 HIGH-1 |
| `cmd /c npm run lint` | ✅ 通过（2 个 warning：TrackMetadataView.tsx 两处 `any` 类型，0 error） |
| `cmd /c npm test` | ❌ **失败**，1 个测试失败（TrackMetadataView），见 HIGH-2 |
| `cmd /c npm run build` | ❌ **失败**，TypeScript 类型错误（LocalDashboardView.tsx:475），见 CRITICAL-1 |

## 问题清单

### CRITICAL-1：`npm run build` 失败 — LocalDashboardView trackPoints 类型与 DashboardRegionRenderer 契约不匹配

相关位置：

- `src-ui/components/LocalDashboardView.tsx:101-103`（`trackPointsCache` 状态定义）
- `src-ui/components/LocalDashboardView.tsx:136`（`ensureTrackMap` 的 invoke 类型）
- `src-ui/components/LocalDashboardView.tsx:475`（传给 `DashboardRegionRenderer` 的 `trackPoints` prop）
- `module_local_dashboard/.../dashboardRenderer.tsx:115`（`DashboardRegionRenderer` 的 `trackPoints` prop 类型定义）

问题说明：

`LocalDashboardView` 的 `trackPointsCache` 类型为：

```typescript
const [trackPointsCache, setTrackPointsCache] = useState<
  Record<string, { x: number; z: number }[]>
>({});
```

但 `module_local_dashboard` 导出的 `DashboardRegionRenderer` 组件的 `trackPoints` prop 期望：

```typescript
trackPoints: Record<string, {
  points: { x: number; z: number }[];
  angleDeg: number;
  flipX: number;
  flipZ: number;
}>;
```

`ensureTrackMap` 函数（line 132-152）仅通过 `invoke<{ pointsJson: string } | null>("get_track_map", ...)` 获取 `pointsJson`，解析后存入 `trackPointsCache`，**丢弃了 `angleDeg`、`flipX`、`flipZ` 字段**。

`tsc` 编译时报告 TS2322：

```
Type 'Record<string, { x: number; z: number; }[]>' is not assignable to type
'Record<string, { points: { x: number; z: number; }[]; angleDeg: number; flipX: number; flipZ: number; }>'.
```

影响：

- **生产构建完全失败**，应用无法发布。`npm run build`（`tsc && vite build`）在类型检查阶段中断。
- 即使绕过类型检查，LocalDashboardView 预览中的 MapWidget 无法应用旋转/翻转变换（angleDeg/flipX/flipZ 丢失），地图渲染方向错误。
- 这是 acc-coach 与 module_local_dashboard 之间的契约断裂：module_local_dashboard 的 `DashboardRegionRenderer` 已升级为接收包装格式（points + angleDeg + flipX + flipZ），但 acc-coach 的 LocalDashboardView 仍传递原始点数组。

修复建议：

1. `LocalDashboardView.ensureTrackMap` 的 `invoke` 类型改为完整的 `TrackMapRecord`（含 `angleDeg`/`flipX`/`flipZ`）。
2. `trackPointsCache` 状态类型改为 `Record<string, { points: { x; z }[]; angleDeg: number; flipX: number; flipZ: number }>`。
3. 存储时构造包装对象：`{ points, angleDeg: record.angleDeg ?? 0, flipX: record.flipX ?? 1, flipZ: record.flipZ ?? 1 }`。
4. 修复后验证 `npm run build` 通过。

### HIGH-1：`cargo clippy -D warnings` 失败（18 个 error）— 质量门禁回归

相关位置：

- `src/dashboard/alias.rs:23-25`（`doc_overindented_list_items` × 3）
- `src/dashboard/alias.rs:185,199,203,207,211,221,225,288`（`map_identity` × 8，测试代码中 `Some("...").map(|s| s)`）
- `src/dashboard/alias.rs:234`（`bool_assert_comparison` + `unnecessary_get_then_check` × 2）
- `src/dashboard/output.rs:932`（`type_complexity` — `&[(&str, fn(&LiveFrame) -> serde_json::Value)]`）
- `src/recording/auto.rs:176`（`too_many_arguments` 8/7 — `spawn_runtime`）
- `src/recording/replay.rs:74`（`new_without_default` — `ReplayState`）
- `src/ipc/mod.rs:1340,1347`（`redundant_closure` × 2 — `.map_err(|e| AppError::from(e))`）

问题说明：

上一轮审计关闭时 `cargo clippy --all-targets --all-features -- -D warnings` 已通过。本轮新增代码后该命令重新失败，8 个 lib error + 18 个 lib test error。

主要问题类型：
- `doc_overindented_list_items`：alias.rs 文档注释中子列表缩进过多（6 空格应为 5）。
- `map_identity`：测试中 `Some("raw:...").map(|s| s)` 是恒等映射，应直接用 `Some("raw:...")`。
- `bool_assert_comparison` / `unnecessary_get_then_check`：`assert_eq!(...is_some(), true)` 应为 `assert!(...contains_key(...))`。
- `type_complexity`：output.rs 中 `liveframe_to_canonical` 类型过于复杂，建议提取 type 别名。
- `too_many_arguments`：`spawn_runtime` 8 个参数（上一轮审计 HIGH-5 已有同类问题，加 `#[allow]` 或重构）。
- `new_without_default`：`ReplayState::new()` 存在但无 `Default` impl。
- `redundant_closure`：`.map_err(|e| AppError::from(e))` 应为 `.map_err(AppError::from)`。

影响：

- 质量门禁从上一轮关闭时的通过状态退回失败状态。
- CI 若启用 clippy `-D warnings` 会被阻断。

修复建议：

- 修复 alias.rs 文档缩进、测试中的 `map_identity` 和 `bool_assert_comparison`。
- 为 `ReplayState` 添加 `impl Default`。
- `spawn_runtime` 添加 `#[allow(clippy::too_many_arguments)]` 或引入参数结构体。
- `ipc/mod.rs` 中 `.map_err(|e| AppError::from(e))` 改为 `.map_err(AppError::from)`。
- output.rs `liveframe_to_canonical` 提取 type 别名。
- 修复后复验 `cargo clippy --all-targets --all-features -- -D warnings` 通过。

### HIGH-2：`npm test` 失败 — TrackMetadataView `angleDeg` 未定义处理

相关位置：

- `src-ui/components/TrackMetadataView.tsx:513`（`mapStatus` 计算）
- `src-ui/components/TrackMetadataView.test.tsx:109-142`（失败的测试）

问题说明：

```typescript
const mapStatus = !trackMap
  ? "No map data"
  : `${JSON.parse(trackMap.pointsJson).length} points from ${trackMap.source}${
      trackMap.angleDeg !== 0 ? ` @ ${trackMap.angleDeg}°` : ""
    }`;
```

当 `trackMap.angleDeg` 为 `undefined`（测试 mock 未设置该字段，或旧记录缺失），`undefined !== 0` 求值为 `true`，导致追加 ` @ undefined°`。实际 `mapStatus` 变为 `"4 points from telemetry @ undefined°"`。

失败的测试期望精确匹配 `"4 points from telemetry"`：

```typescript
const statusText = await screen.findByText("4 points from telemetry");
```

因实际文本含 ` @ undefined°` 后缀，`findByText` 无法匹配，测试超时失败。

影响：

- `npm test` 质量门禁失败。
- 对于 `angleDeg` 缺失的 track map 记录（虽然 v10 migration 为旧行补 DEFAULT 0.0，但前端从 IPC 收到的 JSON 中该字段可能为 null/undefined），UI 会显示 `" @ undefined°"` 垃圾文本。

修复建议：

- 代码：`trackMap.angleDeg !== 0` 改为 `(trackMap.angleDeg ?? 0) !== 0`，或 `trackMap.angleDeg && trackMap.angleDeg !== 0`。
- 测试 mock：补全 `angleDeg: 0` 字段（与真实 `TrackMapRecord` 结构一致）。
- 修复后验证 `npm test` 通过。

### HIGH-3：calc 通道在帧翻译中被丢弃（未提交变更引入）

相关位置：

- `src/ipc/mod.rs:1413-1433`（`translate_dashboard_frame_values`，未提交变更移除了 `!is_user_format` 保留块）
- `src/recording/auto.rs:1043-1051`（`push_merged_to_bus`，未提交变更移除了 `!is_user_format` 保留块）
- `src/dashboard/alias.rs:30-106`（`ChannelAliasTable::from_catalog` 仅从 `raw_catalog` 构建）

问题说明：

别名表 `ChannelAliasTable::from_catalog()` 仅从 `module_live_telemetry::raw_catalog::all_raw_items()` 构建，只含 `raw:` 前缀的规范名。`calc:` 前缀的通道（如 `calc:delta_best`、`calc:delta_session`、`calc:car_x`）不在别名表中，`alias.to_user_facing("calc:delta_best")` 返回 `None`。

**已提交（HEAD）版本** 的 `translate_dashboard_frame_values` 和 `push_merged_to_bus` 含有保留块：

```rust
// 保留未登记通道的原始 key
if !alias.is_user_format(&key) {
    translated.insert(key, value);
}
```

此块确保 `calc:` 前缀的 key（`is_user_format` 返回 false）被保留。

**未提交的工作区变更** 删除了这两处保留块。删除后，`calc:delta_best` 等通道在帧翻译时被静默丢弃 —— 既不在别名表中（`to_user_facing` 返回 None），也不再通过保留块留存。

`calc:car_x`/`calc:car_z` 因 `translate_dashboard_frame_values` 中的 `CAR_POSITION_MAP` 特殊处理而幸存，但其余 calc 通道（delta_best、delta_session 等）全部丢失。

订阅路径不受影响：`recording_dashboard_item_for_field("calc:delta_best", alias)` 通过 `calc:` 前缀分支正确识别为 `CalculatedItem`。但帧数据中找不到该 key，widget 显示 "--"。

影响：

- 用户在 channel picker 中选择 calc 通道（如 `calc:delta_best`，注册表中存在）并配置到 text/chart widget 后，订阅成功但帧查找失败，widget 始终显示占位符。
- live 和回放两条路径均受影响。
- 这是订阅成功但数据丢失的"静默断裂"，难以诊断。

修复建议：

1. **恢复保留块**（最小修复）：在 `translate_dashboard_frame_values` 和 `push_merged_to_bus` 中恢复 `if !alias.is_user_format(&key) { ... insert(key, value) }` 块。
2. **或扩展别名表**（根本修复）：将 `calc:` 通道纳入 `ChannelAliasTable`，为每个 calc 通道定义 user-facing 名称（如 `calc:delta_best` → `deltaBest`），并在 `to_user_facing`/`to_canonical` 中支持。
3. 补充测试：构造含 `calc:delta_best` 的帧，经 `push_merged_to_bus` 和 `translate_dashboard_frame_values` 后断言 key 存在。

### HIGH-4：`DashboardPublisher` 僵尸代码仍存在且新增方法 — 违反上一轮关闭承诺

相关位置：

- `src/dashboard/output.rs:34`（`pub struct DashboardPublisher` 仍存在）
- `src/dashboard/output.rs:216`（本轮新增 `publish_values_frame` 方法）
- `src/dashboard/output.rs:151,182`（`publish_frame`/`publish_fields` 仍存在，无调用方）
- `src/dashboard/mod.rs:28`（`pub use output::DashboardPublisher` 仍导出）

问题说明：

上一轮审计 HIGH-2 的关闭承诺是"DashboardPublisher 已从 main.rs 和 ipc/mod.rs 完全移除"。关闭时确实从 Tauri state 和 IPC 中移除，但 **struct 定义和 `pub use` 导出仍保留在 output.rs / mod.rs 中**。

本轮变更不仅未清理这些僵尸代码，反而**新增了 `publish_values_frame` 方法**（output.rs:216），使僵尸代码增长。`DashboardPublisher` 现有 3 个 publish 方法（`publish_frame`、`publish_fields`、`publish_values_frame`），均无调用方。

`dashboard_fields_map`、`dashboard_fields_frame`、`project_fields` 等函数也仅为这些僵尸方法服务。

影响：

- 违反上一轮审计关闭承诺（"完全移除"）。
- 僵尸代码增长，增加维护负担和混淆。
- `output.rs` 中 `serial_remote_enabled` 字段、`OutputProfilesConfig` 等旧系统残留继续存在。

修复建议：

1. 移除 `DashboardPublisher` struct 及其所有 publish 方法。
2. 移除 `dashboard/mod.rs` 中的 `pub use output::DashboardPublisher`。
3. 移除仅为 DashboardPublisher 服务的 `publish_frame`/`publish_fields`/`publish_values_frame`/`dashboard_fields_map`/`project_fields` 等函数（确认无其他调用方后）。
4. 若 `OutputProfilesConfig` 仍被 `TelemetryWorkspaceView` 用于只读校验（`validate_output_profiles_config`），保留配置类型但移除 publisher 运行时。

### HIGH-5：`DynamicControlInfo` wire 协议新增字段未更新 protocol-spec.md

相关位置：

- `src/dashboard/remote/protocol.rs:226-230`（`DynamicControlInfo` 新增 `text_template`、`text_format` 字段）
- `docs/acc-coach/public-protocol/protocol-spec.md`（未对应更新）

问题说明：

`DynamicControlInfo` 结构体新增两个可选字段：

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub text_template: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub text_format: Option<String>,
```

`skip_serializing_if = Option::is_none` 保证旧客户端不感知新字段（向后兼容），但 `protocol-spec.md` 是面向 network remote 设备端开发者的**公开 wire 协议契约**。新增字段未在规范中文档化，设备端开发者无法知晓 `textTemplate`/`textFormat` 字段的存在和语义。

影响：

- 设备端实现者按旧规范开发时，无法消费 `textTemplate`/`textFormat` 字段，text widget 渲染退化为无模板。
- 公开协议契约与实现不一致。

修复建议：

1. 更新 `protocol-spec.md` 中 `DynamicControlInfo` 的字段定义，补充 `textTemplate`（可选）、`textFormat`（可选）字段及语义说明。
2. 明确字段缺失时的回退行为（设备端按 `fieldRefs` 渲染原始值）。
3. 同步更新 `PrepareLayoutMessage` 的示例 JSON。

### MEDIUM-1：`history.rs` `TelemetryHistory` 仍存在且新增功能 — 违反架构设计

相关位置：

- `src/dashboard/history.rs:1-90`（`TelemetryHistory` 仍存在，本轮新增 `push_frame` 方法）
- `docs/acc-coach/dashboard-architecture.md` 第 2.3 节（"不再做历史累积（TelemetryHistory 移除）"）

问题说明：

架构设计文档（v1.1）第 2.3 节明确要求"不再做历史累积（`TelemetryHistory` 移除，各 dashboard 自行维护环形缓冲区）"。Wave 2 迁移路径第 1 条也要求"删除 `src/dashboard/history.rs` 中的 `TelemetryHistory`"。

但本轮变更不仅未删除 `TelemetryHistory`，反而**新增了 `push_frame` 方法**（line 47-52），使其能直接消费 `DashboardValuesFrame`。该 struct 仍被 `pub mod history` 导出，且有完整的单元测试（6 个测试）。

`TelemetryHistory` 当前是否有生产调用方需确认（`feed.rs` 的 `MockDashboardFeed` 未使用它），但其存在与架构设计直接矛盾。

影响：

- 架构设计与实现不一致，违反"主模块不再做历史累积"原则。
- 若未来开发者误用 `TelemetryHistory`，会重新引入主模块数据加工职责。

修复建议：

1. 若 local dashboard 已自管理历史（按 PRD `local-dashboard-self-managed-data.md`），则删除 `history.rs` 及其测试。
2. 若 local dashboard 尚未完成自管理实现，则在 `history.rs` 顶部标注 `#![deprecated]` 或注释说明"待 module_local_dashboard 完成历史自管理后移除"。
3. 同步更新架构设计文档的迁移状态。

### MEDIUM-2：`track_map.rs` `buffer` 字段为死代码

相关位置：

- `src/recording/track_map.rs:30`（`buffer: Vec<(f64, f64)>` 字段定义）
- `src/recording/track_map.rs:191`（`reset_for_new_session` 清空 buffer）
- `src/recording/track_map.rs:77-179`（`on_lap_completed` 直接使用参数 slices，不写 buffer）

问题说明：

`TrackMapCollector` 结构体定义了 `buffer: Vec<(f64, f64)>` 字段，文档注释为"Buffered (car_x, car_z) position samples from the current lap"。但 `on_lap_completed` 方法直接使用传入的 `car_x_values` 和 `car_z_values` slices，**从不写入 `buffer`**。`buffer` 仅在 `reset_for_new_session` 中被清空，且 `reset_for_new_session` 测试还断言 `buffer.is_empty()`。

该字段是死代码 —— 早期设计中录制循环会逐帧填充 buffer，但最终实现改为通过 `LapCompletedCallback` 一次性传递整圈数据，buffer 被遗弃但未删除。

影响：

- 误导性字段，开发者可能误以为 buffer 被填充并依赖它。
- 占用少量内存（始终为空 Vec）。

修复建议：

- 移除 `buffer` 字段及其在 `reset_for_new_session` 中的清空逻辑和测试断言。

### MEDIUM-3：`poll_dashboard_frame` 回放帧推入 live bus 但 `stop_replay` 不清 bus

相关位置：

- `src/ipc/mod.rs:1462-1470`（`poll_dashboard_frame` 在回放激活时将帧推入 `bus`）
- `src/ipc/mod.rs:1344-1348`（`stop_replay` 仅调用 `replay.stop()`，不清 bus）
- `src/recording/auto.rs:583`（录制停止时 `bus.clear()`）

问题说明：

`poll_dashboard_frame` 在回放激活时，从 `replay.latest_dashboard_frame()` 获取帧，翻译后**推入 `DashboardFrameBus`**，然后返回：

```rust
if let Some(frame) = replay.latest_dashboard_frame() {
    let mut result = LiveDashboardFrame::from(frame);
    translate_dashboard_frame_values(&mut result.values, &alias);
    bus.push_frame(&module_dashboard_protocol::DashboardValuesFrame { ... });
    return Ok(Some(result));
}
```

`stop_replay` IPC 仅调用 `replay.stop()`，**不调用 `bus.clear()`**。

回放停止后：
- 若录制未激活，`bus` 中残留最后一帧回放数据。
- 下次 `poll_dashboard_frame` 调用时，`replay.latest_dashboard_frame()` 返回 None（replay 已停止），进入 `bus.latest_frame()` 分支，返回残留的回放帧。
- overlay 窗口在回放停止后会短暂显示回放数据，直到录制开始推送新帧或 bus 被其他路径清除。

影响：

- 回放停止后 overlay 可能显示陈旧的回放数据，用户困惑。
- live bus 被回放数据污染，语义混乱（bus 应为 live 帧总线）。

修复建议：

1. 在 `stop_replay` IPC 命令中，`replay.stop()` 后调用 `bus.clear()`。
2. 或移除 `poll_dashboard_frame` 中将回放帧推入 bus 的逻辑（回放帧应直接返回，不污染 live bus）。
3. 补充测试：回放停止后 `poll_dashboard_frame` 在无 live 数据时返回 None。

### MEDIUM-4：别名表未覆盖 calc 通道

相关位置：

- `src/dashboard/alias.rs:30-106`（`from_catalog` 仅从 `raw_catalog` 构建）
- `src/ipc/mod.rs:1418-1421`（`CAR_POSITION_MAP` 硬编码 `calc:car_x`/`calc:car_z` 特殊处理）
- `src/dashboard/mod.rs:417-431`（`build_channel_registry` 中 calc 通道仍以 `calc:` 前缀 ID 暴露给前端）

问题说明：

别名表仅覆盖 `raw:` 通道。`calc:` 通道（`calc:car_x`、`calc:car_z`、`calc:delta_best`、`calc:delta_session` 等）无 user-facing 映射。`translate_dashboard_frame_values` 通过 `CAR_POSITION_MAP` 硬编码处理 `calc:car_x`/`calc:car_z`，但其余 calc 通道无处理（见 HIGH-3）。

`build_channel_registry` 中，raw 通道的 `id` 已改为 user-facing（通过 `alias.to_user_facing`），但 calc 通道的 `id` 仍为 `calc:delta_best` 形式。前端 channel picker 显示混合命名（user-facing 和 canonical 共存），不一致。

影响：

- calc 通道在帧翻译中需逐个特殊处理（当前仅 car_x/car_z），扩展性差。
- channel picker 中 calc 通道显示 `calc:` 前缀，与 raw 通道的 user-format 显示不一致。
- 与 HIGH-3 共同构成 calc 通道数据断裂。

修复建议：

1. 扩展 `ChannelAliasTable`，为 calc 通道定义 user-facing 名称（如 `calc:delta_best` → `deltaBest`，`calc:car_x` → `carX`）。
2. 移除 `CAR_POSITION_MAP` 硬编码，统一走别名表。
3. `build_channel_registry` 中 calc 通道的 `id` 改为 user-facing。
4. 同步更新 `channel-alias-layer.md` 文档。

### MEDIUM-5：`output.rs` `steerRawAngle` 语义不一致 + `steeringDeg` 不再产出

相关位置：

- `src/dashboard/output.rs:932-945`（`liveframe_to_canonical` 表，`raw:controls.steer_angle` → `f.steering_deg`）
- `src/dashboard/alias.rs:48-53`（`steerRawAngle` 主名，`steeringDeg` 为同义词）

问题说明：

`dashboard_fields_map` 中 `liveframe_to_canonical` 表将 `raw:controls.steer_angle` 映射到 `f.steering_deg`（LiveFrame 中已转换为度数的值）。通过 `alias.to_user_facing("raw:controls.steer_angle")` 得到主名 `steerRawAngle`。

结果：
- `steerRawAngle` 的值实际为**度数**（来自 `LiveFrame.steering_deg`），但名称暗示"原始角度比值"（-1..1）。语义不一致。
- `steeringDeg`（同义词）不再通过 `to_user_facing` 产出（只返回主名）。使用 `steeringDeg` 的 widget 无法在帧中找到值。

旧代码（writer.rs 已删除的 `recording_dashboard_fields`）同时产出 `steerRawAngle`（原始比值）和 `steeringDeg`（度数 = 比值 × 225），两者值不同。新代码只产出一个值（度数），且挂在 `steerRawAngle` 名下。

影响：

- 此路径仅在 `DashboardPublisher` 被调用时生效，而 `DashboardPublisher` 当前未接线（见 HIGH-4），故**当前无运行时影响**。
- 若未来重新启用 output profile，使用 `steeringDeg` 的 widget 将显示 "--"，使用 `steerRawAngle` 的 widget 会收到度数值而非比值。

修复建议：

1. 明确 `steerRawAngle` 语义：若应为原始比值，则 `liveframe_to_canonical` 应读取原始 steer_angle 而非 `steering_deg`。
2. 若 `steeringDeg` 仍需作为独立通道，在别名表中为其定义独立的 canonical 映射（可能需要 calc 通道支持度数转换）。
3. 随 HIGH-4 一并处理（移除僵尸 publisher 后此问题消失）。

### MEDIUM-6：`recording_dashboard_fields` 死代码

相关位置：

- `src/recording/writer.rs:195-237`（`recording_dashboard_fields` 函数定义，无调用方）

问题说明：

`recording_dashboard_fields` 函数经 codegraph 确认**无任何调用方**。该函数仍包含旧式别名逻辑（现已被 alias 表取代），是 `recording_dashboard_item_for_field` 重构后的遗留物。

影响：

- 死代码，维护负担。
- 内部别名逻辑与新架构不一致，可能误导开发者。

修复建议：

- 确认无调用方后删除该函数。

### MEDIUM-7：`save_track_map` 不校验 `source` 字段长度

相关位置：

- `src/ipc/mod.rs:2470-2500`（`save_track_map` 命令）

问题说明：

`save_track_map` 对 `track_id` 调用了 `validate_string_arg`（限制 256 字符），对 `points_json` 做了 JSON 合法性校验，但对 `source` 字段**无任何校验**。前端可传入任意长度、任意内容的 `source` 字符串。

影响：

- 低风险（本地应用，source 仅用于显示），但与其他字段的校验不一致。
- 超长 source 字符串存入 DB 后可能在 UI 中溢出。

修复建议：

- 添加 `validate_string_arg("Source", &source, 256)?;`。

### LOW-1：`serial_remote_enabled` 僵尸字段仍存在于 `OutputProfilesConfig`

相关位置：

- `src/dashboard/output.rs`（`OutputProfilesConfig` 仍含 `serial_remote_enabled`）
- `src/dashboard/output.rs:216`（`publish_values_frame` 仍引用 `serial_remote_enabled`）

问题说明：

上一轮审计 MEDIUM-7 从 `RemoteDevicesConfig` 移除了 `serial_remote_enabled`。但 `OutputProfilesConfig`（旧 output profile 系统）中该字段仍存在，且本轮新增的 `publish_values_frame` 仍读取它。随 HIGH-4 一并处理。

### LOW-2：`shared_memory.rs` 文件名与内容不符

相关位置：

- `src/live/shared_memory.rs`（文件名 "shared_memory"，内容为 `LiveFrame`/`LiveSessionInfo`/`LiveBestLapReference` 等类型）

问题说明：

文件顶部注释"ACC shared-memory access lives in `module_live_telemetry`"，实际内容是 live telemetry 数据类型定义（`LiveFrame` 等），非 shared memory 访问代码。文件名误导。

修复建议：

- 重命名为 `live_types.rs` 或 `frame.rs`，或合并到 `src/live/mod.rs`。

### LOW-3：`rotate_track_map`/`flip_track_map` delete-then-insert 非原子

相关位置：

- `src/ipc/mod.rs:2612-2613`（rotate: delete 后 insert）
- `src/ipc/mod.rs:2682-2683`（flip: delete 后 insert）

问题说明：

旋转/翻转 track map 时先 `delete_track_map` 再 `insert_track_map`。若进程在两步之间崩溃，track map 丢失。本地单用户应用风险低。

修复建议：

- 使用 `INSERT OR REPLACE` 或事务包裹 delete + insert。

### LOW-4：`TrackMetadataView` 使用 `any` 类型（lint 警告）

相关位置：

- `src-ui/components/TrackMetadataView.tsx:242,403`（两处 `any` 类型）

问题说明：

`npm run lint` 通过但有 2 个 warning：`@typescript-eslint/no-explicit-any`。不阻塞构建，但应收敛类型。

### LOW-5：`logs/` 目录和 `opencode.json` 被提交到版本控制

相关位置：

- `logs/1.logs`（149 行日志，已 tracked）
- `logs/console.log`（39 行日志，已 tracked）
- `opencode.json`（29 行，已 tracked）

问题说明：

`logs/` 目录含运行时日志文件，不应纳入版本控制。`opencode.json` 若为本地工具配置也应加入 `.gitignore`。这些文件在 `51f248c` 之后被提交。

修复建议：

- `git rm --cached logs/ opencode.json`，添加到 `.gitignore`。

## 正面发现

本轮变更中以下方面质量良好：

1. **别名层设计**（`alias.rs`）：单一翻译点、自动生成 + 覆盖表 + 同义词 + 跨子系统消歧，测试覆盖完整（6 个测试）。取代了 20+ 处散布的硬编码别名表，显著改善代码质量。
2. **Track map 自动采集**（`track_map.rs`）：`LapCompletedCallback` 集成正确（`car_coordinates[0]`/`[2]` 为玩家坐标，与 `calc:car_x`/`calc:car_z` 一致），有效圈过滤（not out lap + is_valid）、存在性检查、手动覆盖、从 acctlm 文件采集，测试覆盖完整（7 个测试）。
3. **DashboardFrameBus 集成**：录制循环 `push_merged_to_bus` + 停止/替换时 `bus.clear()`，符合 PRD 清空信号契约。
4. **DB migrations v9/v10/v11**：纯增量、幂等、有对应测试（`test_track_map_crud` 等）。
5. **Text widget 表达式函数**（未提交）：`abs`/`round` 内置函数，求值器扩展（tokenize/parse/evaluate），22 个新测试覆盖正常/边界/错误用例。
6. **Layout 热重载**（未提交）：localStorage 版本号 + storage 事件 + `key` 强制 remount，机制简洁。
7. **Replay 功能**：`ReplayState` 状态机完整（start/stop/poll/latest_frame/replace_items），自动清理（FramesExhausted/Error），session metadata 查询。
8. **协议类型对齐**：`layout.rs` re-export `module_dashboard_protocol` 类型，自定义 Serialize 兼容新旧 JSON 格式（同时输出 `staticControls`/`dynamicControls`/`controls`）。
9. **禁止模块约束遵守**：`module_live_telemetry`、`module_local_dashboard`、`ld_to_acctlm`、`acctlm_core` 的源码（.rs/.ts/.tsx）未被修改，仅 docs 目录有文档变更。

## 建议修复顺序

按严重程度和依赖关系排序：

1. **CRITICAL-1**（构建失败）：修复 `LocalDashboardView` trackPoints 类型契约，恢复 `npm run build`。
2. **HIGH-2**（测试失败）：修复 `TrackMetadataView` angleDeg 处理，恢复 `npm test`。
3. **HIGH-1**（clippy 门禁）：修复 18 个 clippy error，恢复 `cargo clippy -D warnings` 通过。
4. **HIGH-3**（calc 通道丢弃）：恢复 `translate_dashboard_frame_values` 和 `push_merged_to_bus` 的保留块（或扩展别名表）。
5. **HIGH-4**（僵尸代码）：移除 `DashboardPublisher` 及其服务函数。
6. **HIGH-5**（协议文档）：更新 `protocol-spec.md` 补充 `textTemplate`/`textFormat` 字段。
7. **MEDIUM-1 至 MEDIUM-7**：按各自说明修正。
8. **LOW-1 至 LOW-5**：后续清理中处理，不阻塞当前审计关闭。

## 当前状态

- 状态：⛔ **OPEN — 未修复。**
- 本轮变更是 dashboard 架构大重构的第一阶段，别名层、FrameBus、track map、回放、表达式函数等核心能力已就位且设计良好。但存在 1 个阻断发布的构建失败（CRITICAL-1）、1 个测试失败（HIGH-2）、clippy 门禁回归（HIGH-1），以及 calc 通道数据断裂（HIGH-3）等问题。
- 构建状态：`cargo check`/`cargo test`/`npm lint` 通过；`cargo clippy -D warnings` 失败（18 error）；`npm test` 失败（1 test）；`npm run build` 失败（TS 类型错误）。
- 暂缓继承项：前端主 chunk 超过 700 kB 的包体问题仍按上一轮审计结论暂缓。
- 安全：本轮未引入安全漏洞。track map 采集来自本地遥测数据，IPC 命令有路径/字符串/JSON 校验。

## 复审结论

- 复审日期：2026-06-28
- 复审范围：开发人员对审计报告中问题的修复结果
- 复审人：OpenCode
- **结论：阻断性问题已全部修复，构建/测试/Clippy 门禁均恢复通过，但仍有 4 个非阻断项未处理，本轮审计尚不能关闭。**

### 门禁复验结果

| 命令 | 结果 |
|---|---|
| `cargo check --all-targets --all-features` | 通过 |
| `cargo clippy --all-targets --all-features -- -D warnings` | 通过 |
| `cmd /c npm run lint` | 通过（0 warning / 0 error） |
| `cmd /c npm test -- --run` | 通过（4 文件 44 测试） |
| `cmd /c npm run build` | 通过（仅有 chunk >700 kB 提示，属上一轮暂缓项） |
| `cargo test` | 通过（128 单元测试 + e2e/lap_stats/real_ld/regression_monza） |

### 已修复问题

| 编号 | 状态 | 复审说明 |
|---|---|---|
| CRITICAL-1 | 已修复 | `LocalDashboardView.tsx:105-106` 状态类型已改为 `{ points, angleDeg, flipX, flipZ }`；`ensureTrackMap` 已按 `TrackMapRecord` 取值并补齐默认值。`npm run build` 通过。 |
| HIGH-1 | 已修复 | `cargo clippy -D warnings` 已通过。`alias.rs` 文档缩进、`map_identity`、`bool_assert_comparison` 已清理；`ReplayState` 已加 `Default`；`spawn_runtime` 已加 `#[allow(clippy::too_many_arguments)]`；`ipc/mod.rs` 冗余 closure 已改；`output.rs` 整文件移除，顺带消除 `type_complexity` 源。 |
| HIGH-2 | 已修复 | `TrackMetadataView.tsx:513` 已改为 `(trackMap.angleDeg ?? 0) !== 0`；测试 mock 补全 `angleDeg: 0`。`npm test` 通过。 |
| HIGH-3 | 已修复 | `ipc/mod.rs:1434-1437` 与 `auto.rs:1050-1052` 的“保留未登记通道”块已恢复，`calc:delta_best` 等不再被丢弃。 |
| HIGH-4 | 已修复 | `src/dashboard/output.rs` 已整文件替换为占位注释，`DashboardPublisher` 及全部 publish 方法、`dashboard_fields_map`、`project_fields` 等已随之一并移除；`dashboard/mod.rs` 不再 `pub use DashboardPublisher`。 |
| HIGH-5 | 已修复 | `docs/acc-coach/public-protocol/protocol-spec.md` 已在 3.5 节补充 `textTemplate`、`textFormat` 字段定义、默认值说明及示例 JSON。 |
| MEDIUM-2 | 已修复 | `src/recording/track_map.rs` 已移除 `buffer: Vec<(f64, f64)>` 字段及对应清空逻辑，单元测试也已同步调整。 |
| MEDIUM-3 | 已修复 | `ipc/mod.rs:1348` `stop_replay` 在 `replay.stop()` 后已调用 `bus.clear()`。 |
| MEDIUM-6 | 已修复 | `src/recording/writer.rs` 中 `recording_dashboard_fields` 死代码已删除。 |
| MEDIUM-7 | 已修复 | `ipc/mod.rs:2485` `save_track_map` 已增加 `validate_string_arg("Source", &source, 256)?`。 |
| MEDIUM-5 | 随移除解决 | `output.rs` 已整体移除，`steerRawAngle`/`steeringDeg` 语义问题不再存在。 |
| LOW-1 | 随移除解决 | `OutputProfilesConfig` / `serial_remote_enabled` 已随 `output.rs` 移除而消失。 |
| LOW-4 | 已修复 | `TrackMetadataView.tsx` 中两处显式 `any` 已消除，`npm run lint` 0 warning。 |
| LOW-5 | 已修复 | `logs/`、`opencode.json` 已从版本控制移除，`.gitignore` 已追加对应规则。 |

### 仍未修复问题

| 编号 | 状态 | 复审说明 |
|---|---|---|
| MEDIUM-1 | 未修复 | `src/dashboard/history.rs` 仍存在，`TelemetryHistory` 结构体及 `push_frame` 方法仍在，且 `dashboard/mod.rs:16` 仍 `pub mod history;`。`codegraph` 确认其无生产调用方，但文件与架构设计文档“不再做历史累积”直接冲突。 |
| MEDIUM-4 | 部分缓解，未根治 | `HIGH-3` 通过“保留块”缓解了 `calc:` 通道被丢弃的问题，但 `ChannelAliasTable::from_catalog()` 仍只从 `raw_catalog` 构建，`calc:delta_best`、`calc:delta_session` 等仍无 user-facing 映射；`build_channel_registry` 中 calc 通道的 `id` 仍是 `calc:xxx`，channel picker 命名不统一；`CAR_POSITION_MAP` 硬编码仍保留。 |
| LOW-2 | 未修复 | `src/live/shared_memory.rs` 文件名仍为 `shared_memory.rs`，实际内容是 `LiveFrame` 等数据类型定义，文件名与内容不符。 |
| LOW-3 | 未修复 | `rotate_track_map`（`ipc/mod.rs:2620-2621`）和 `flip_track_map`（`ipc/mod.rs:2690-2691`）仍是 `delete_track_map` 后再 `insert_track_map`，非原子。 |

### 最终状态

- 状态：⛔ **OPEN — 未关闭。**
- 所有阻断发布与质量门禁的问题（CRITICAL-1、HIGH-1/2/3/4/5）均已修复，构建、测试、Clippy 全绿。
- 剩余 4 项均未阻塞构建/测试，但 `MEDIUM-1` 和 `MEDIUM-4` 属于架构/数据契约一致性问题，建议在关闭本轮审计前处理。
- 暂缓继承项：前端主 chunk 超过 700 kB 的包体问题仍按上一轮审计结论暂缓。

## 第二次复审结论

- 复审日期：2026-06-28
- 复审范围：针对第一次复审中标记为“未修复”的 4 项问题（MEDIUM-1、MEDIUM-4、LOW-2、LOW-3）的修复结果
- 复审人：OpenCode
- **结论：4 项遗留问题已全部修复，构建/测试/Clippy/lint/build 全绿，本轮审计可以关闭。**

### 修复详情

| 编号 | 状态 | 复审说明 |
|---|---|---|
| MEDIUM-1 | 已修复 | `src/dashboard/history.rs` 已删除，`src/dashboard/mod.rs` 已移除 `pub mod history;`。`codegraph` 确认无生产调用方残留。 |
| MEDIUM-4 | 已修复 | `src/dashboard/alias.rs:103-111` 已将 `calc:` 内置计算通道纳入别名表，自动生成 user-facing 名称（`calc:delta_best` → `deltaBest` 等）；`src/ipc/mod.rs` 已移除 `CAR_POSITION_MAP` 硬编码；`src/dashboard/mod.rs` 的 `builtin_calculated_channel_definition` 已使用 `alias.to_user_facing` 输出统一 ID。 |
| LOW-2 | 已修复 | `src/live/shared_memory.rs` 已删除，内容迁移至新建文件 `src/live/live_types.rs`；`src/live/mod.rs` 已更新为 `pub mod live_types;` 并重新导出相关类型。 |
| LOW-3 | 已修复 | `src/ipc/mod.rs:2610-2625`（`rotate_track_map`）和 `ipc/mod.rs:2695-2709`（`flip_track_map`）已使用 `BEGIN`/`COMMIT`/`ROLLBACK` 事务包裹 `delete_track_map` + `insert_track_map`，崩溃时不再丢失数据。 |

### 门禁复验结果

| 命令 | 结果 |
|---|---|
| `cargo check --all-targets --all-features` | 通过 |
| `cargo clippy --all-targets --all-features -- -D warnings` | 通过 |
| `cmd /c npm run lint` | 通过（0 warning / 0 error） |
| `cmd /c npm test -- --run` | 通过（4 文件 44 测试） |
| `cmd /c npm run build` | 通过（仅有 chunk >700 kB 提示，属上一轮暂缓项） |
| `cargo test --lib` | 通过（122 单元测试） |
| `cargo test`（含 e2e/lap_stats/real_ld/regression_monza/doc-tests） | 全部通过 |

### 最终状态

- 状态：✅ **CLOSED — 已关闭。**
- 审计报告中全部 18 项问题（1 CRITICAL、5 HIGH、7 MEDIUM、5 LOW）均已修复或已随相关代码移除而解决。
- 构建、测试、Clippy、lint、生产构建全部通过。
- 暂缓继承项：前端主 chunk 超过 700 kB 的包体问题仍按上一轮审计结论暂缓，不阻塞本轮关闭。
