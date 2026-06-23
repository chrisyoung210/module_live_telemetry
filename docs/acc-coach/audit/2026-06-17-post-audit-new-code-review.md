# ACC Coach 新增代码审计报告

- 审计日期：2026-06-17
- 审计基线：上一轮已关闭审计报告中记录的修复 commit `84d46cc549069d738fb198bdb47c4dc2c9c6ed6c`
- 审计范围：`84d46cc` 之后新增和修改的代码，重点覆盖 local/remote dashboard、overlay controller、recording auto dashboard subscriptions、IPC 配置同步、dashboard layout 注册和相关前端变更。
- 结论：本轮新增代码存在 2 个 CRITICAL、3 个 HIGH、5 个 MEDIUM、6 个 LOW 问题。`npm run lint`、`npm test`、`cargo check`、`cargo test`、`npm run build` 通过；`cargo clippy --all-targets --all-features -- -D warnings` 未通过。
- **关闭状态：✅ CLOSED — 修复 commit `437705e`，除 2 个明确暂缓项外全部修复并通过复验。**

## 审计背景

上一轮复审已关闭的审计报告已移入：

- `docs/acc-coach/audit/2026-06-13-acc-coach-code-audit.md`

该报告记录关闭版本为：

- 审计版本：`76b9167df52ec4f9463d5c94672ca36c641895fd`
- 修复 commit：`84d46cc549069d738fb198bdb47c4dc2c9c6ed6c`
- 关闭结论：除明确暂缓项外，其余要求修复的问题均已完成修复并通过复验。

因此，本次审计只关注 `84d46cc` 之后的新代码和变更行为。

本报告整合了原审计（P1-P3）与复审新增发现（P4-P11），按严重程度排序，并将同一条数据流上的关联问题合并为统一修复项。

## 问题清单

### CRITICAL-1：远程 dashboard 实时数据管线三处断裂，实时推送完全不工作

相关位置：

- `src/recording/writer.rs:30`（subscription 过滤）
- `src/recording/writer.rs:60`（`remote_dashboard_device_connected` 恒返回 false）
- `src/ipc/mod.rs:1510`（保存配置后触发 sync）
- `src/ipc/mod.rs:1540`（`dashboard_subscriptions_from_app_data` 只含本地）
- `src/ipc/mod.rs:1551`（合并订阅遗漏远程）
- `src/dashboard/output.rs:137`（`publish_frame` 无调用方）
- `src/dashboard/output.rs:167`（`publish_fields` 无调用方）
- `src/main.rs:80`（`DashboardPublisher` 创建后无数据喂入）
- `src/ipc/mod.rs:1509`（仅 `set_config` 被调用）
- `src/ipc/mod.rs:1660`（仅 `publish_layout` 被调用）

问题说明：

远程 dashboard 的实时数据流存在三个独立断裂点，任一单独修复都无法恢复功能：

**断裂点 A — 订阅过滤恒假（原 P1）：**

`dashboard_subscriptions_for_remote_dashboards` 过滤 profile 时要求三个条件同时为真，但 `remote_dashboard_device_connected` 恒返回 `false`：

```rust
pub(crate) fn remote_dashboard_device_connected(_profile: &OutputProfile) -> bool {
    false
}
```

因此即使用户启用了 UDP/serial profile 且全局开关为 true，远程 profile 仍被全部过滤掉，录制线程不会订阅这些字段。

**断裂点 B — 保存配置后 sync 不含远程订阅（复审新增，原 P5）：**

`save_output_profiles_config` 保存配置后调用 `sync_dashboard_subscriptions_from_disk`，后者通过 `dashboard_subscriptions_from_app_data` 重建订阅。但该函数只读取 overlay 和 layouts，只生成 `local_items`：

```rust
Ok(merge_dashboard_subscriptions([local_items]))
```

它不读取 `output_profiles.json`，也不调用 `dashboard_subscriptions_for_remote_dashboards`。这与启动路径 `load_dashboard_items`（`auto.rs:699-728`，合并 remote+local）形成不一致：保存配置时远程订阅被完全忽略，即使用户在 UI 启用远程 profile 并保存，录制线程订阅也不会更新。

**断裂点 C — publisher 从未被喂入实时数据（复审新增，原 P4）：**

`DashboardPublisher` 在 `main.rs:80` 创建后作为 Tauri state 托管。全代码库范围内，外部仅调用了 `set_config()` 和 `publish_layout()`。`publish_frame()` 和 `publish_fields()` 没有任何调用方——没有定时器、没有 IPC 命令、没有录制循环回调将实时遥测数据喂给 publisher。即使 A、B 修复后数据被正确订阅和收集，也没有代码将数据推送给 publisher 发出。

影响：

- 远程 dashboard layout 能通过 `publish_layout` 发出，但实时值永远不会更新。
- UDP/serial profile 中配置的 channels 不会进入 recording controller 的 dashboard item subscription。
- 用户保存远程 dashboard 配置后，当前录制线程订阅可能被覆盖为只包含本地 overlay 需要的字段。
- 整个远程遥测推送功能从数据流角度完全断开，三处断裂叠加导致功能零可用。

修复建议：

三处断裂需一并修复，缺一不可：

1. **修复断裂点 A**：移除 `remote_dashboard_device_connected` 过滤，或实现真实的连接状态检测后再参与过滤。对 UDP 广播/单播而言，通常不应要求预先 "connected"。最简方案是直接返回 `true` 或删除该条件。
2. **修复断裂点 B**：`dashboard_subscriptions_from_app_data` 应同时加载 `output_profiles.json`，合并 `dashboard_subscriptions_for_remote_dashboards(&outputs)` 与 local layout 订阅，保持与启动路径 `load_dashboard_items` 的一致性。
3. **修复断裂点 C**：在 `AutoRecordingMonitor` 的 dashboard frame 接收分支（`auto.rs:340`）中增加对 `DashboardPublisher::publish_fields` 的调用，将收到的 `DashboardValuesFrame` 按 profile.hz 节流后推送。或在 IPC 层增加定时推送循环，从 `latest_dashboard_frame` 取快照调用 `publish_fields`。
4. **补充测试**：
   - 单元测试：`networkRemoteEnabled=true`、UDP profile `enabled=true`、channels 包含 `speedKmh`，断言远程订阅包含 `raw:controls.speed_kmh`。
   - IPC 级测试：保存含 enabled UDP profile 的配置后，断言录制线程订阅包含对应字段且不丢失远程 channels。
   - 集成测试：启动录制后断言 UDP 端口收到遥测帧。

### CRITICAL-2：全局远程开关后端迁移缺失 + 前端 UI 缺口与陈旧状态覆盖

相关位置：

- `src/dashboard/mod.rs:78`（`#[serde(default)]` 默认 false）
- `src/dashboard/mod.rs:265`（`merge_missing_defaults` 不迁移全局开关）
- `src/dashboard/mod.rs:309`（Default 实现值为 false）
- `src-ui/components/TelemetryWorkspaceView.tsx:509`（缺少全局开关 UI）
- `src-ui/components/TelemetryWorkspaceView.tsx:242`（`saveOutputs` 用陈旧快照）
- `src-ui/components/TelemetryWorkspaceView.tsx:258`（`validateOutputs` 同问题）
- `src-ui/components/TelemetryWorkspaceView.tsx:123`（`outputsDraft` 类型不含全局开关）
- `src-ui/components/TelemetryWorkspaceView.tsx:141`（只提取 profiles 数组）

问题说明：

**后端 — 旧配置升级后全局开关被静默禁用（原 P2）：**

`OutputProfilesConfig` 新增了 `network_remote_enabled` 和 `serial_remote_enabled` 两个全局开关，使用 `#[serde(default)]`（默认 `false`）。旧版 `output_profiles.json` 不包含这两个字段时，serde 反序列化为 `false`。`merge_missing_defaults` 只补齐默认 profile 和 channels，不根据已有 `profile.enabled` 迁移这两个新开关。已有用户升级后 profile 仍为 `enabled=true`，但全局开关变成 `false`，远程输出被静默禁用。

**前端 — TelemetryWorkspaceView 缺少全局开关 UI（复审新增，原 P10-a）：**

`renderRemoteDashboards()`（`TelemetryWorkspaceView.tsx:509-555`）渲染了 profile 编辑和保存按钮，但没有 `networkRemoteEnabled`/`serialRemoteEnabled` 的开关 UI。这两个开关只存在于 `DashboardView.tsx` 的 `RemoteDashboardTab`。用户在 Telemetry Workspace 配置好远程 profile 后，必须切换到另一个视图才能启用全局开关。

**前端 — 保存时用陈旧状态覆盖全局开关（复审新增，原 P10-b）：**

`saveOutputs()`（`TelemetryWorkspaceView.tsx:242-256`）从 `workspace?.outputs`（上次 `load()` 的快照）捕获全局开关：

```ts
networkRemoteEnabled: workspace?.outputs.networkRemoteEnabled ?? false,
serialRemoteEnabled: workspace?.outputs.serialRemoteEnabled ?? false,
```

如果用户在 `DashboardView.tsx` 切换了全局开关（独立调用 `save_output_profiles_config`），TelemetryWorkspaceView 的缓存就过期了。从此处保存会用陈旧值覆盖最新开关状态。根因是 `outputsDraft` 只存 `OutputProfile[]`，不包含全局开关。`validateOutputs()`（`TelemetryWorkspaceView.tsx:258-272`）有相同问题。

影响：

- 已有用户升级后远程 dashboard 停止工作，配置文件里 profile 仍显示 enabled，难以排查。
- `build_diagnostics`、`DashboardPublisher::publish_layout`、`publish_frame`、`publish_fields` 都会把这些 profile 视为未启用。
- 用户在 Telemetry Workspace 无法启用远程输出，且保存时可能意外重置全局开关。
- CRITICAL-1 修复后，用户仍可能因全局开关被静默设为 false 而无法使用远程输出。

修复建议：

后端与前端需一并修复：

1. **后端迁移**：为 `OutputProfilesConfig` 增加显式迁移逻辑——当旧配置缺少全局开关字段时，根据已有 enabled profile 的 transport 推导：存在 enabled UDP profile 则 `network_remote_enabled = true`，存在 enabled serial profile 则 `serial_remote_enabled = true`。或将 serde default 改为兼容旧行为的 true，再由 UI 控制新配置的显式关闭。
2. **前端 UI**：在 `renderRemoteDashboards` 增加 `networkRemoteEnabled`/`serialRemoteEnabled` 开关 UI。
3. **前端状态**：将 `outputsDraft` 类型提升为完整 `OutputProfilesConfig`，使全局开关进入可编辑 draft 状态，消除从陈旧快照取值的隐患。保存前重新加载最新配置或在保存时合并最新磁盘状态。
4. **补充测试**：
   - 反序列化迁移测试：构造旧 JSON 只含 enabled UDP profile，`load_or_create` 后断言 `network_remote_enabled == true`。
   - 前端测试：在 TelemetryWorkspaceView 切换全局开关后保存，断言配置中全局开关值正确持久化。

### HIGH-1：finalize_completed_lap_samples 丢失每圈开头约 500ms 遥测

相关位置：

- `src/recording/import.rs:247`（函数入口）
- `src/recording/import.rs:253`（反向迭代）
- `src/recording/import.rs:260`（切片丢弃开头样本）

问题说明：

```rust
for (idx, sample) in samples.iter().enumerate().rev() {  // 反向迭代
    if sample.state.current_lap_time_ms.unwrap_or(i64::MAX) <= 500 {
        start_idx = idx;
        break;  // 命中正向最后一个匹配项（约 500ms 处）
    }
}
let telemetry = samples[start_idx..]  // 丢弃 samples[0..start_idx]
```

`pending_completed_samples` 中 `current_lap_time_ms` 从 ~0 单调递增到完整圈速。反向迭代命中的是 ≤500ms 窗口的**最后一个**样本（约 500ms 处），而非第一个（约 0ms 处）。`samples[start_idx..]` 丢弃了开头的 0~500ms 数据。

影响：

- 每个完成的圈次导入时丢失开头约 500ms 遥测（60Hz 约 30 个点，120Hz 约 60 个点）。
- 影响所有 live recording 导入的圈次，对起跑、牵引力控制等分析场景造成数据缺失。
- 此问题与远程 dashboard 无关，属于录制导入数据完整性 bug，原审计完全未覆盖。

修复建议：

- 将反向迭代改为正向迭代，找第一个 `current_lap_time_ms <= 500` 的样本作为 `start_idx`。
- 或直接 `start_idx = 0`（因为 `samples_include_lap_start` 已保证首段有起始样本）。
- 补充测试：构造 lap_time = [0, 100, 200, ..., 100000] 的样本，断言导入后 telemetry 首点 lap_time 接近 0 且样本数包含开头部分。

### HIGH-2：replace_dashboard_items 持锁等待 2 秒，与 restart 存在死锁风险

相关位置：

- `src/recording/auto.rs:185`（获取 runtime 锁）
- `src/recording/auto.rs:195`（持锁状态下 recv_timeout 2 秒）
- `src/recording/auto.rs:207`（`restart` 也需要同一把锁）

问题说明：

`replace_dashboard_items`（`auto.rs:181-198`）持有 `self.runtime.lock()` 的同时执行 bounded channel `send` 和 `recv_timeout(COMMAND_TIMEOUT)`（2 秒）。`restart()`（`auto.rs:200-219`）和 `status()` 也需要同一把锁。如果 command channel 满了，`send` 本身也可能在持锁状态下阻塞；如果录制线程繁忙或没有及时返回 response，调用方还会继续持锁等待最长 2 秒。期间所有 `status()`/`latest_dashboard_frame()`/`restart()` 调用全部阻塞。

影响：

- IPC 调用 `save_output_profiles_config`、`save_local_dashboard_overlay` 等命令时可能阻塞 2 秒。
- 若 `replace_dashboard_items` 与 `restart` 从不同 IPC 并发触发，存在死锁可能。

修复建议：

- 在 `send` 和 `recv_timeout` 前释放 runtime 锁，将 `command_tx` 和 `response_rx` 的所有权移出锁作用域：先在锁内 clone `command_tx` 并创建 `response_rx`，然后释放锁，再在锁外执行 send 和 recv_timeout。
- 或将 `COMMAND_TIMEOUT` 缩短（如 500ms）并加重试逻辑。
- 补充并发测试：在 `replace_dashboard_items` 等待期间并发调用 `status()`，断言后者不被阻塞。

### HIGH-3：OutputTransport::Serial 的 baudRate/baud_rate 跨层序列化不匹配

相关位置：

- `src/dashboard/mod.rs:98`（`#[serde(tag = "type", rename_all = "snake_case")]`）
- `src/dashboard/mod.rs:102`（`Serial { port, baud_rate }`）
- `src-ui/types.ts:285`（前端定义为 `baudRate`）
- `src-ui/components/TelemetryWorkspaceView.tsx:506`（构造 `baudRate`）
- `src-ui/components/TelemetryWorkspaceView.tsx:543`（读取 `.baudRate`）
- `src-ui/components/DashboardView.tsx:394`（读取 `.baudRate`）

问题说明：

Rust 端 `OutputTransport` 使用 `#[serde(tag = "type", rename_all = "snake_case")]`，`Serial` variant 的 `baud_rate` 字段序列化为 `baud_rate`。前端 `types.ts:285` 定义为 `baudRate`（camelCase），并在多处以 `.baudRate` 访问。

两个方向均出错：

- **读取（后端→前端）**：后端发 `baud_rate`，前端访问 `.baudRate` 得到 `undefined`，UI 显示空/NaN。
- **写入（前端→后端）**：`setTransportType`（`TelemetryWorkspaceView.tsx:506`）构造 `{ baudRate: 921600 }`，Rust serde 期望 `baud_rate`。由于 `baud_rate: u32` 没有 `#[serde(default)]`，该 payload 更可能在后端反序列化阶段失败，而不是静默变成 0。

影响：

- 任何新建或切换 serial transport 的 profile，保存时可能因字段不匹配而失败。
- 已有 serial profile 读取时 UI 显示异常。
- serial 远程输出配置链路不可用；同时 serial transport 当前尚未实现，实际推送功能仍需另行补齐。

修复建议：

- 将 Rust `OutputTransport` 的 `rename_all` 改为 `"camelCase"` 以与项目其余类型约定一致，或将前端类型改为 `baud_rate`。推荐前者，因项目其他结构体均使用 camelCase。
- 同步修正前端所有 `.baudRate` 访问点（若改 Rust 侧则前端无需改动，因 `baudRate` 已是前端期望的 camelCase）。
- 补充反序列化往返测试：前端构造 serial profile → 保存 → 重新加载，断言 baudRate 值不变。

### MEDIUM-1：clippy 质量门禁重新失败（原 P3）

相关位置：

- `src/recording/auto.rs:197`（redundant closure）
- `src/recording/auto.rs:405`（manual `is_multiple_of`）
- `src/recording/auto.rs:710`（`and_then` 可改为 `map`）
- `src/ipc/mod.rs:1362`（manual `is_multiple_of`）
- `src/ipc/mod.rs:1454`（`too_many_arguments` 8/7）

问题说明：

上一轮审计关闭记录中 `cargo clippy --all-targets --all-features -- -D warnings` 已通过。本轮新增代码后该命令重新失败，当前 5 个 error：

- redundant closure：`map_err(|err| format_dashboard_subscription_error(err))`
- manual `is_multiple_of`：`count % gap == 0`、`self.seen % self.gap == 0`
- `Option.and_then(|x| Some(y))` 可改为 `map`
- `save_computed_channels_config` 超过 clippy 默认参数数量限制

影响：

- 质量门禁从上一轮关闭时的通过状态退回失败状态。
- 后续 CI 若启用 clippy `-D warnings` 会被阻断。
- 新增 warning 会掩盖后续真实回归。

修复建议：

- 按 clippy 提示修正 redundant closure（改为 `map_err(format_dashboard_subscription_error)`）、`is_multiple_of`（改为 `.is_multiple_of()`）和 `and_then`（改为 `map`）。
- 对 `save_computed_channels_config` 因 Tauri command 注入参数较多导致的 `too_many_arguments`，若短期不重构，可沿用上一轮修复风格加局部 `#[allow(clippy::too_many_arguments)]`，并保持范围只覆盖该 command。
- 修复后补充一次 `cargo clippy --all-targets --all-features -- -D warnings` 复验。

### MEDIUM-2：merge_latest_dashboard_frame 的 sparse frame 语义需要失效清理边界

相关位置：

- `src/recording/auto.rs:816`

问题说明：

```rust
snapshot.values.extend(frame.values);
```

`HashMap::extend` 只添加/更新新帧中存在的 key。这看起来支持 `DashboardValuesFrame` 的 sparse frame 语义：不同字段可以用不同刷新率到达，低频字段不应在高频字段帧到来时被清空。现有单元测试 `merge_latest_dashboard_frame_keeps_previous_sparse_values` 也明确锁定了这个行为。

因此，不能简单把 `extend` 改成全量替换。真正的风险在于字段失效边界：如果某个字段不再被订阅或数据源停止发布，而没有触发清理，旧值会继续残留在 snapshot 中。当前 `replace_dashboard_items` 成功后会清空 latest frame，但还缺少针对“字段源停止发布但订阅未刷新”的 TTL 或有效性边界。

影响：

- sparse frame 行为本身是合理的，但缺少字段级过期策略时，失效字段可能长期保留旧值，导致 IPC 层或 overlay 显示过期数据。

修复建议：

- 保留 sparse frame 合并语义，不要直接全量替换。
- 为 snapshot 增加字段级 timestamp / sample_tick，并按订阅 interval 或固定 TTL 清理过期字段。
- 在订阅变更、录制停止、controller 重启等边界继续确保清空 latest frame。
- 补充测试：字段 A 按低频刷新时不应被高频字段帧清除；字段 A 超过 TTL 或订阅移除后应从 latest frame 中消失。

### MEDIUM-3：insert_value 静默吞掉序列化失败

相关位置：

- `src/dashboard/output.rs:927`

问题说明：

```rust
let json = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
```

序列化失败时静默插入 `Null`，无任何诊断。`writer.rs:250-258` 同名函数用 `if let Ok` 处理，模式不一致。

影响：

- 遥测数据中可能静默出现 `Null` 值，远程 dashboard 收到后显示异常，且无法定位是哪个字段序列化失败。

修复建议：

- 改为 `match serde_json::to_value(value) { Ok(v) => fields.insert(...), Err(e) => log::warn!("serialize field {} failed: {}", name, e) }`，至少记录警告。
- 统一 `output.rs` 与 `writer.rs` 中同名函数的错误处理模式。

### MEDIUM-4：merge_dashboard_subscriptions 按 name 去重忽略 interval 差异

相关位置：

- `src/recording/writer.rs:105`

问题说明：

```rust
if seen.insert(item.item_name.clone()) {
    merged.push(item);
}
```

同一字段若同时出现在远程订阅（30Hz）和本地 layout 订阅（60Hz）中，先入者胜出。`auto.rs:727-728` 传入 `[remote, local]`，远程 30Hz 优先，本地需要的 60Hz 被静默降级。

影响：

- 本地 overlay 需要的高刷新率字段可能被远程 profile 的低刷新率静默覆盖，导致 overlay 显示卡顿。

修复建议：

- 合并时取两者中更短的 interval（更高的 Hz），而非先入者胜出。
- 补充测试：远程 30Hz + 本地 60Hz 订阅同一字段，断言合并后 interval 对应 60Hz。

### MEDIUM-5：ComputedChannelsConfig::save 忽略 self，load_or_create 逻辑反转（潜在数据丢失）

相关位置：

- `src/dashboard/mod.rs:210`（`load_or_create` 入口）
- `src/dashboard/mod.rs:213`（文件存在时覆盖为默认值）
- `src/dashboard/mod.rs:220`（`save` 方法）
- `src/dashboard/mod.rs:221`（`let _ = self` 丢弃实例数据）

问题说明：

```rust
pub fn load_or_create(path: &Path) -> Result<Self, String> {
    let config = Self::default();
    if path.exists() {
        config.save(path)?;   // 文件存在时反而覆盖为默认值
        return Ok(config);     // 返回默认值，从不读取文件
    }
    save_json(path, &config)?;
    Ok(config)
}

pub fn save(&self, path: &Path) -> Result<(), String> {
    let _ = self;                      // 丢弃 self
    save_json(path, &Self::default())  // 始终保存默认值
}
```

`load_or_create` 逻辑反转：文件存在时应读取，却覆盖为默认值并返回默认值。`save` 用 `let _ = self` 主动丢弃实例数据，始终写入 `Self::default()`。

影响：

- 当前无代码调用此函数（实际使用 `load_from_db` / `save_to_db`），因此不是当前运行路径上的数据丢失。
- 一旦未来有代码启用文件版 computed channels 配置，会静默丢失配置并覆盖磁盘文件。
- 该项属于潜在数据丢失地雷，建议修复，但不应按当前路径作为 CRITICAL 阻塞项。

修复建议：

- `save` 应序列化 `self`：`save_json(path, self)`，移除 `let _ = self`。
- `load_or_create` 文件存在时应读取并反序列化，参照 `OutputProfilesConfig::load_or_create`（`mod.rs:245-258`）的正确实现。
- 补充测试：写入非默认 config → 重新加载 → 断言字段一致。

### LOW-1：Serial transport 未实现，每次 publish 报错浪费 CPU

相关位置：

- `src/dashboard/output.rs:334`

问题说明：

```rust
OutputTransport::Serial { .. } => Err("serial transport is configured but not implemented in dashboard v1".to_string()),
```

Serial profiles 通过所有过滤后，每次 `publish_profile` 都返回此错误，每帧重复触发。

修复建议：

- 在 `publish_frame`/`publish_fields` 的 profile 过滤阶段跳过 serial profiles（类似 `profile_due` 检查），避免无效调用。
- 或在 `set_config` 时标记 serial profiles 为 inactive，publish 时直接跳过。

### LOW-2：set_config 清 sinks 但不清 active_layout

相关位置：

- `src/dashboard/output.rs:51`

问题说明：

```rust
pub fn set_config(&mut self, config: OutputProfilesConfig) {
    self.config = config;
    self.sinks.clear();
    self.last_sent_at.clear();
    // active_layout 未清除
}
```

更新配置时 UDP sockets 和发送时间戳被重置，但 `active_layout` 保留。旧 layout 可能被引用到新配置下不存在的 profile。

修复建议：

- 在 `set_config` 中增加 `self.active_layout = None;`，或在下次 `publish_layout` 时自然覆盖。前者更安全。

### LOW-3：lap_reset_detected 有符号减法，数据腐化时可能假触发

相关位置：

- `src/recording/import.rs:224`

问题说明：

```rust
(Some(prev), Some(next)) => prev - next > LAP_RESET_THRESHOLD_MS,
```

`prev` 和 `next` 为 `i64`。正常 ACC 遥测两者均为正，但数据腐化产生负值时 `prev - next` 可能 wrap（release 模式），产生大正值假触发 lap reset。

修复建议：

- 使用 `saturating_sub` 或在校验阶段过滤负值：`if prev < 0 || next < 0 { return false; }`。

### LOW-4：空录制文件删除失败被静默吞掉

相关位置：

- `src/recording/auto.rs:927`

问题说明：

```rust
let _ = std::fs::remove_file(&result.file_path);
```

零圈录制的空 JSONL 文件删除失败时错误被 `let _ =` 丢弃，空文件残留磁盘。

修复建议：

- 改为 `if let Err(e) = std::fs::remove_file(&result.file_path) { set_status(status, |s| { s.last_error = Some(format!("Remove empty recording: {e}")); }); }`，至少记录到状态中。

### ~~LOW-5：JSONL 录制导入无去重检查，可能产生重复 data entry~~ ✅ CLOSED — JSONL 代码已全量删除

> 2026-06-17 更新：JSONL 格式已废弃。当前录制统一使用 `.acctlm2` 二进制格式。所有 JSONL 相关代码（import.rs 880行、session.rs 中的 `PersistedLiveImport`/`persist_live_recording_as_session`/`persist_live_recording_import_as_session`、ipc/mod.rs 中的 `LapLoadPlan::LiveJsonl`/`PreparedLiveImport::Jsonl`/`live_data_entry_needs_import` 等）已全量删除，净减少约 1200 行。非 acctlm2 格式的旧录制导入路径返回 `"Live recording uses deprecated JSONL format"` 错误。此项 CLOSE。

### LOW-6：hot-path 闭包中 expect 有理论 panic 风险

相关位置：

- `src/recording/auto.rs:760`

问题说明：

```rust
.expect("validated dashboard calculated item")
```

该 `expect` 在传给 `RecordingController` 的闭包中。验证（line 753）与执行时空分离，理论上若闭包 panic 会穿过线程边界 unwind，可能终止录制线程。不过当前闭包使用的是同一份 `name` / `expression`，且前面已经用 `ExpressionCalculatedItem::new(name.clone(), &expression)?` 预验证，实际失败概率较低。

影响：

- 极端情况下录制线程意外终止，录制中断，且错误信息不明确。
- 当前更接近防御性改进，不建议作为本轮关闭阻塞项。

修复建议：

- 替换为 `match` 或 `?` 传播错误，或在闭包中 catch panic 并记录到 `last_error` 状态。
- 最低限度：添加注释说明此 panic 终止录制任务的行为，并在 `RecordingController` 层面增加 panic 捕获。

## 验证结果

已执行：

- `cmd /c npm run lint`：通过。
- `cmd /c npm test`：通过，2 个 test files、5 个 tests。
- `cargo check`：通过。
- `cargo test`：通过，Rust 单元测试、e2e pipeline、lap stats、real LD、regression monza 测试均通过。
- `cmd /c npm run build`：通过。Vite 仍提示主 chunk `715.96 kB`，该问题对应上一轮已明确暂缓的前端包体偏大问题。
- `cargo clippy --all-targets --all-features -- -D warnings`：失败，见 MEDIUM-1。
- `git diff --check 84d46cc549069d738fb198bdb47c4dc2c9c6ed6c --`：通过；仅出现本机 `C:\Users\congj/.config/git/ignore` 权限 warning。

## 建议修复顺序

按严重程度和功能依赖关系排序：

1. **CRITICAL-1**（远程 dashboard 数据管线三处断裂）：一并修复断裂点 A/B/C，接入 `publish_fields` 调用、修正 subscription 过滤、统一 sync 路径。这是远程 dashboard 功能能否工作的前提。
2. **CRITICAL-2**（全局远程开关后端迁移 + 前端 UI）：修复旧配置迁移逻辑，补全前端开关 UI 并修正陈旧状态覆盖。这是 CRITICAL-1 修复后用户能否正确启用远程输出的前提。
3. **HIGH-1**（圈首 500ms 遥测丢失）：修正 `finalize_completed_lap_samples` 迭代方向，恢复录制导入数据完整性。与远程 dashboard 无关但影响核心数据。
4. **HIGH-2**（持锁死锁风险）：释放 `replace_dashboard_items` 持锁 send / 等待 response，消除状态查询和 restart 被阻塞的风险。
5. **HIGH-3**（baudRate/baud_rate 不匹配）：统一 `OutputTransport` 序列化命名，恢复 serial profile 配置链路。
6. **MEDIUM-1**（clippy 质量门禁失败）与 **MEDIUM-4**（订阅合并忽略 interval）：建议作为本轮关闭阻塞项同步修复。
7. **MEDIUM-2、MEDIUM-3、MEDIUM-5**：按各自说明修正，补充对应单元测试；其中 MEDIUM-2 应保留 sparse frame 语义，不应简单全量替换。
8. **LOW-1 至 LOW-6**：后续清理中处理，不阻塞当前审计关闭。
9. **补充回归测试**：针对远程 dashboard subscription 合并、旧配置迁移、圈首遥测完整性、serial 往返序列化各增加专项测试。

## 当前状态

- 状态：✅ CLOSED — 修复 commit `437705e`。除 MEDIUM-2（sparse frame TTL）和 LOW-6（hot-path expect）明确暂缓外，所有问题已修复并通过复验。
- 已修复并复验：CRITICAL-1、CRITICAL-2、HIGH-1、HIGH-2、HIGH-3、MEDIUM-1、MEDIUM-3、MEDIUM-4、MEDIUM-5、LOW-1、LOW-2、LOW-3、LOW-4。
- 仍可暂缓：MEDIUM-2（sparse frame 字段级 TTL/失效清理边界）、LOW-6（hot-path `expect` 防御性改进）。
- 已通过删除 JSONL 代码关闭：LOW-5（JSONL 录制导入去重）。JSONL 格式已全量废弃，所有相关代码已移除。
- 暂缓继承项：前端主 chunk 超过 700 kB 的包体问题仍按上一轮审计结论暂缓，不作为本次新增代码审计关闭阻塞项。
- 报告整合说明：本报告整合了原审计（原 P1-P3）与复审新增发现（原 P4-P11）。原 P1+P4+P5 合并为 CRITICAL-1（同一数据流三处断裂），原 P2+P10 合并为 CRITICAL-2（同一功能后端+前端两面），原 P3 调整为 MEDIUM-1，原 P6 调整为 HIGH-3，原 P7 调整为 MEDIUM-5，原 P8 调整为 HIGH-1，原 P9 调整为 HIGH-2，原 P11 拆分为 MEDIUM-2 至 MEDIUM-4 和 LOW-1 至 LOW-6。

## 修复记录（2026-06-17）

- 远程 dashboard 数据流：录制线程收到 `DashboardValuesFrame` 后通过共享 `DashboardPublisher` 调用 `publish_fields`；保存远程配置、注册/删除 layout 后的订阅同步现在同时合并 remote profiles 与 local layouts。
- 远程订阅：移除“设备已连接”恒 false 的阻断条件；订阅合并按字段取更短 interval，保留更高刷新率需求。
- 配置迁移与前端状态：旧 `output_profiles.json` 缺少全局开关时，会从 enabled UDP/serial profile 推导；Telemetry Workspace 的 outputs draft 升级为完整 `OutputProfilesConfig`，并补齐 Network/Serial 全局开关 UI。
- serial 字段契约：后端 serial transport 序列化为 `baudRate`，并兼容旧 `baud_rate` 输入。
- 录制导入：`finalize_completed_lap_samples` 改为保留圈首样本；`lap_reset_detected` 改为非负校验和 saturating subtraction。
- 并发与质量门禁：`replace_dashboard_items` 释放 runtime 锁后再 send/等待 response；clippy 失败项已清理。
- 其他清理：`DashboardPublisher::set_config` 清空 active layout；实时 publish 跳过尚未实现的 serial transport；dashboard output `insert_value` 不再静默写入 Null；空录制删除失败会写入状态错误。
- 新增回归测试：远程订阅、订阅 interval 合并、旧配置迁移、serial baudRate 兼容、computed channels 文件 roundtrip、live recording 圈首样本保留。
- JSONL 废弃清理（2026-06-17）：删除 `src/recording/import.rs`（880行→6行stub）、`src/recording/session.rs` 中的 `PersistedLiveImport`/`persist_live_recording_as_session`/`persist_live_recording_import_as_session` 及 JSONL 分支、`src/ipc/mod.rs` 中的 `LapLoadPlan::LiveJsonl`/`PreparedLiveImport::Jsonl`/`live_data_entry_needs_import` 等，删除 `examples/reimport_live_recording.rs` 和 `examples/live_recording_summary.rs`。净删除约 1200 行。非 acctlm2 的旧录制导入返回 `"Live recording uses deprecated JSONL format"` 错误。LOW-5 随之 CLOSE。
- ~~空 acctlm2 录制清理缺失（审计外发现，2026-06-17）~~：`persist_acctlm_recording_outcome` 在 `outcome.laps.is_empty()` 时不返回错误，导致空录制文件残留磁盘且产生无用 data entry，原 JSONL 路径已有此检查而 acctlm2 路径遗漏。修复：在 `persist_acctlm_recording_outcome` 开头增加 early return `Err(EMPTY_LIVE_RECORDING_ERROR)`，复用 `auto.rs` 中已有的空录制删除逻辑。
- 复验命令：`cmd /c npm run lint`、`cmd /c npm test`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`、`cmd /c npm run build` 均通过；`npm run build` 仍保留已暂缓的主 chunk 体积警告。
