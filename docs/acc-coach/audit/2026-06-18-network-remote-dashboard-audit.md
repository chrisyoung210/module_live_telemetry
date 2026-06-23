# ACC Coach Network Remote Dashboard 新增代码审计报告

- 审计日期：2026-06-18
- 审计基线：上一轮已关闭审计报告的修复 commit `437705e`（关闭记录见 `2026-06-17-post-audit-new-code-review.md`）
- 审计范围：`437705e` 之后未提交的工作目录变更，重点覆盖 `src/dashboard/remote/` 新模块（8 个文件）、`src/ipc/mod.rs`、`src/main.rs`、`src/recording/auto.rs`、`src/recording/writer.rs` 的新增和修改代码，以及 `src-ui/components/DashboardView.tsx`、`src-ui/types.ts` 的前端变更。
- 结论：本轮新增代码存在 4 个 CRITICAL、5 个 HIGH、9 个 MEDIUM、6 个 LOW 问题。`cargo check`、`cargo test`、`npm run lint`、`npm test`、`npm run build` 通过；`cargo clippy --all-targets --all-features -- -D warnings` 未通过（5 个 error）。
- **关闭状态：⛔ OPEN — 未修复。**

## 审计背景

上一轮审计（`2026-06-17-post-audit-new-code-review.md`）已关闭于 commit `437705e`，关闭时修复了远程 dashboard 数据管线三处断裂（CRITICAL-1）、全局远程开关迁移（CRITICAL-2）等问题。

本轮变更在 `437705e` 之后对远程 dashboard 进行了**架构级重写**：

- **旧系统**（已删除）：`OutputProfilesConfig` / `OutputProfile` / `DashboardPublisher` — 基于 UDP/serial output profile 的单向外推
- **新系统**（新增）：`src/dashboard/remote/` — 基于 Discovery（UDP）→ Control（TCP）→ Data（UDP）三层分离的会话制协议

新系统引入了 8 个新 Rust 文件（约 1700 行），同时删除了 `auto.rs` 中旧的 `publish_dashboard_fields` 函数和 `SharedDashboardPublisher` 集成，移除了 `writer.rs` 中旧的 `dashboard_subscriptions_for_remote_dashboards` 函数，替换为基于 `RemoteSessionRegistry` 的 `dashboard_subscriptions_for_remote_sessions`。

前端 `DashboardView.tsx` 的 `RemoteDashboardTab` 被完全重写，从旧的 output profile 编辑界面变为新的设备发现/配对/会话状态展示界面。

本审计评估新架构的完整性、安全性、数据流连通性和代码质量。

## 问题清单

### CRITICAL-1：远程 telemetry 数据管线从未接线 — `broadcast_telemetry` 无调用方

相关位置：

- `src/dashboard/remote/session.rs:220`（`broadcast_telemetry` 定义）
- `src/dashboard/remote/data.rs:197`（`DataSender::broadcast` 定义）
- `src/recording/auto.rs:334-340`（录制循环收到 frame 后不再推送给任何 publisher）
- `src/dashboard/output.rs:138`（旧 `publish_frame` 定义，无调用方）
- `src/dashboard/output.rs:169`（旧 `publish_fields` 定义，无调用方）

问题说明：

新系统的 `RemoteSessionRegistry::broadcast_telemetry` 方法（`session.rs:220`）定义了将 `TelemetryFrame` 推送给所有活跃 session 的 `DataSender` 的入口。但全代码库中**此方法无任何调用方**。

同时，录制循环（`auto.rs`）在收到 `DashboardValuesFrame` 后的分支中，旧代码调用的 `publish_dashboard_fields(&dashboard_output, &status, &frame)` 已被**完全删除**。新代码仅调用 `merge_latest_dashboard_frame` 将帧合并到本地快照，不再将数据推送给任何远程发布器：

```rust
// auto.rs — 旧代码已删除 publish_dashboard_fields 调用
recv(dashboard_rx) -> message => {
    if let Ok(frame) = message {
        log_dashboard_queue_frame(&mut dashboard_rx_log, &frame, dashboard_rx.len());
        // publish_dashboard_fields 已删除
        merge_latest_dashboard_frame(&latest_dashboard_frame, frame);
        while let Ok(frame) = dashboard_rx.try_recv() {
            log_dashboard_queue_frame(&mut dashboard_rx_log, &frame, dashboard_rx.len());
            // publish_dashboard_fields 已删除
            merge_latest_dashboard_frame(&latest_dashboard_frame, frame);
        }
    }
```

旧的 `SharedDashboardPublisher` 也从 `AutoRecordingMonitor` 中移除。`DashboardPublisher`（旧）仍作为 Tauri state 存在，`save_output_profiles_config` 仍调用其 `set_config`，但不再有任何代码调用其 `publish_fields` 或 `publish_frame`。

这**完全重现了上一轮审计 CRITICAL-1 断裂点 C 的同一问题**：publisher/runtime 创建后没有代码将实时遥测数据喂入。

影响：

- 即使设备被发现、配对、连接、会话启动成功，遥测数据永远不会通过 UDP 发送到设备。
- 远程 dashboard 的数据面完全不工作。
- 与上一轮审计关闭时的状态相比，这是功能回归 — 上一轮已修复此断裂点，本轮重写再次引入。

修复建议：

1. 在 `AutoRecordingMonitor` 的 dashboard frame 接收分支中，将 `DashboardValuesFrame` 转换为 `TelemetryFrame` 并调用 `RemoteDashboardRuntime` 的 `session_registry.broadcast_telemetry`。
2. 由于 `AutoRecordingMonitor` 在独立线程中运行且 `RemoteDashboardRuntime` 是 Tauri state（`std::sync::Mutex`），需要将 `RemoteSessionRegistry` 或 `DataSender` 以 `Arc<Mutex<>>` 形式共享给录制线程，或在录制线程与 IPC 之间建立 channel 传递遥测帧。
3. 补充测试：录制循环收到 frame 后断言 `broadcast_telemetry` 被调用且 UDP 端口收到数据包。

### CRITICAL-2：Discovery 接收循环从未启动 — `run()` 无调用方

相关位置：

- `src/dashboard/remote/discovery.rs:237`（`DiscoveryService::run` 定义）
- `src/dashboard/remote/discovery.rs:207`（`poll_once` 定义）
- `src/dashboard/remote/runtime.rs:56-72`（`enable_network_remote` 仅 `bind` 不 `run`）

问题说明：

`DiscoveryService::bind()` 创建 UDP socket 并返回 `DiscoveryService`，`enable_network_remote` 将其存入 `self.discovery_service: Option<DiscoveryService>`。但 `DiscoveryService::run(self)` — 阻塞式接收循环 — **从未在任何线程中启动**。`poll_once()` 也从未被调用。

```rust
// runtime.rs — enable_network_remote 仅 bind，不 spawn run()
pub fn enable_network_remote(&mut self) -> Result<(), String> {
    // ...
    if self.discovery_service.is_none() {
        let service = DiscoveryService::bind(...)?;
        self.discovery_service = Some(service); // 存入但从未 run()
    }
    Ok(())
}
```

无人读取 UDP socket，设备 announce 响应堆积在 socket buffer 中最终被丢弃。`refresh_freshness()` 和 `prune_stale()` 也从未执行。设备注册表永远为空。

用户点击 "Scan for Devices" 时，`send_probe()` 发送广播，设备回复 announce，但无人处理这些回复。前端每 2 秒轮询 `get_remote_dashboard_state`，`discovered_devices` 始终为空列表。

影响：

- 设备发现完全不工作。
- 用户点击扫描后看不到任何设备，功能零可用。

修复建议：

1. 在 `enable_network_remote` 中，`bind()` 后通过 `std::thread::spawn` 启动 `DiscoveryService::run()`。
2. 由于 `run(self)` 消费所有权，需在 spawn 前将 `DiscoveryService` 从 `Option` 中取出（`take()`），或将 registry 的 `Arc<Mutex<>>` clone 后传给线程，runtime 保留 probe 发送能力。
3. 注意 `send_probe` 仍需访问 socket — 考虑将 socket 包在 `Arc` 中或在 runtime 中保留一个独立的 probe sender。
4. 补充测试：bind 后 announce 到达时断言 registry 包含该设备。

### CRITICAL-3：无设备连接/配对/会话启动代码 — `handshake` 从未被调用

相关位置：

- `src/dashboard/remote/control.rs:101`（`ControlSession::handshake` 定义，无调用方）
- `src/dashboard/remote/session.rs:114`（`create_session` 定义，无调用方）
- `src/dashboard/remote/session.rs:174`（`register_control_session` 定义，无调用方）
- `src/ipc/mod.rs`（无 `connect_device` / `pair_device` / `start_stream` 等 IPC 命令）
- `src-ui/components/DashboardView.tsx:509`（"Connect" 按钮无 `onClick`）

问题说明：

新系统定义了完整的 control 层协议（握手、配对、认证、layout 传输、stream 启动），但**没有任何代码将这些步骤串联起来**：

- `ControlSession::handshake` — 定义了 TCP 握手流程，无调用方。
- `RemoteSessionRegistry::create_session` / `register_control_session` / `session_started` — 定义了 session 生命周期管理，无调用方。
- IPC 层仅有三个新命令：`get_remote_dashboard_state`（读取状态）、`toggle_network_remote`（开关 discovery）、`probe_remote_devices`（发送探测）。**缺少** `connect_device`、`pair_device`、`authenticate_device`、`prepare_layout`、`start_stream`、`stop_stream` 等命令。
- IPC 返回的 `active_sessions` 硬编码为 `Vec::new()`：

```rust
let active_sessions: Vec<_> = Vec::new(); // TODO: expose sessions through registry
```

- 前端 "Connect" 按钮无 `onClick` 处理器：

```tsx
<button className={styles.actionButton} type="button}>
    Connect
</button>
```

影响：

- 即使 CRITICAL-2 修复后发现了设备，用户也无法连接、配对或启动 telemetry 流。
- 整个 control + data 层是脚手架代码，不可用。

修复建议：

1. 新增 `connect_device` IPC 命令：接收 `device_id`，从 discovery registry 获取 IP 和 control port，发起 TCP 连接，执行 `ControlSession::handshake`，将结果注册到 `RemoteSessionRegistry`。
2. 新增 `pair_device` / `authenticate_device` IPC 命令：处理配对和认证流程，保存 trust token 到 `RemoteDevicesConfig`。
3. 新增 `start_stream` / `stop_stream` IPC 命令：发送 `setStreamProfile` + `startStream`/`stopStream`，注册 `DataSender` session。
4. 前端为 "Connect" 按钮添加 `onClick` 处理器，调用 `connect_device`。
5. 补充集成测试：使用 fake device server 验证完整 discovery → connect → pair → start stream 流程。

### CRITICAL-4：录制循环 `load_dashboard_items` 不再加载远程订阅 — restart 后远程订阅丢失

相关位置：

- `src/recording/auto.rs:517-538`（`load_dashboard_items` 仅加载 local overlay）
- `src/ipc/mod.rs:1521-1540`（`sync_dashboard_subscriptions_from_disk` 从 `RemoteDashboardState` 读取远程订阅）
- `src/recording/auto.rs:197-210`（`restart` 后重新调用 `load_dashboard_items`）
- `src/ipc/mod.rs:1133`、`1222`、`1421`、`1464`（4 处 `monitor.restart()` 调用）

问题说明：

录制循环的 `load_dashboard_items` 从旧版本的"合并 remote + local"改为**仅加载 local overlay 订阅**：

```rust
fn load_dashboard_items(app_data_dir: Option<&Path>) -> Vec<DashboardItemSubscription> {
    // 仅加载 overlay + layouts，不加载远程订阅
    overlay_and_layouts
        .as_ref()
        .map(|(overlay, layouts)| {
            if !overlay.enabled { return Vec::new(); }
            dashboard_subscriptions_for_local_layouts(&layouts.layouts, overlay)
        })
        .unwrap_or_default()
}
```

远程订阅仅在 IPC 的 `sync_dashboard_subscriptions_from_disk` 中通过 `dashboard_subscriptions_for_remote_sessions` 加载。但 `monitor.restart()` 在 4 个 IPC 命令中被调用（`save_workspace_settings`、`save_app_settings`、`save_live_capture_config`、`save_computed_channels_config`），restart 后录制循环重新调用 `load_dashboard_items`，**远程订阅被覆盖为空**。

这**重现了上一轮审计 CRITICAL-1 断裂点 B 的同一类问题**：sync 路径与 restart 路径不一致。

此外，由于 CRITICAL-3 导致无 session 被创建，`dashboard_subscriptions_for_remote_sessions` 的返回值始终为空集，`sync_dashboard_subscriptions_from_disk` 实际也不包含远程订阅。

影响：

- 录制线程永远不会订阅远程 session 需要的字段。
- 即使 session 系统被接线，`restart` 后远程订阅丢失。
- 旧系统通过 `output_profiles.json` 加载远程订阅的路径被删除，已配置 UDP profile 的用户升级后录制线程不再订阅这些字段。

修复建议：

1. `load_dashboard_items` 应与 `sync_dashboard_subscriptions_from_disk` 保持一致，同时加载远程和本地订阅。由于录制线程无法访问 Tauri state，需将 `RemoteSessionRegistry` 的活跃订阅信息通过 `Arc<Mutex<>>` 或 channel 传递给录制线程。
2. 或在 `restart` 时通过 IPC 层重新调用 `sync_dashboard_subscriptions_from_disk`，确保 restart 后订阅包含远程部分。
3. 补充测试：配置远程 session 后 `restart`，断言录制线程订阅包含远程字段。

### HIGH-1：`DATA_HEADER_SIZE` 常量错误（38 vs 实际 40 字节）

相关位置：

- `src/dashboard/remote/protocol.rs:26`（`DATA_HEADER_SIZE = 38`）
- `src/dashboard/remote/data.rs:102`（MTU 检查使用 `MAX_UDP_PAYLOAD_BYTES - DATA_HEADER_SIZE`）
- `src/dashboard/remote/data.rs:250`（`Vec::with_capacity(DATA_HEADER_SIZE + payload.len())`）
- `src/dashboard/remote/data.rs:295-328`（测试确认 payload 从 offset 40 开始）

问题说明：

`DATA_HEADER_SIZE` 定义为 38，但实际 binary header 为 40 字节：

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic "ACCD" |
| 4 | 1 | protocol_version |
| 5 | 1 | header_len |
| 6 | 2 | flags |
| 8 | 8 | session_id_hash |
| 16 | 4 | stream_id |
| 20 | 8 | sequence |
| 28 | 8 | sent_unix_ms |
| 36 | 1 | payload_type |
| 37 | 1 | encoding |
| 38 | 2 | payload_len |
| **40** | N | **payload** |

测试代码确认 payload 从 offset 40 开始：`&packet[40..]`。

MTU 检查使用 `MAX_UDP_PAYLOAD_BYTES - DATA_HEADER_SIZE = 1200 - 38 = 1162`，但实际应为 `1200 - 40 = 1160`。1161-1162 字节的 payload 会通过检查但产生 1201-1202 字节的 UDP 包，超过 MTU 限制。

影响：

- payload 在 1161-1162 字节范围时 UDP 包超过 MTU 限制，可能导致 IP 分片或发送失败。
- `Vec::with_capacity` 少分配 2 字节（不影响正确性，影响效率）。
- 协议规范 `protocol-spec.md` 也写了 "总 header 固定 40 字节"，但常量值与规范不一致。

修复建议：

- 将 `DATA_HEADER_SIZE` 改为 40。
- `header_len` 字段值 (`DATA_HEADER_SIZE - 4 = 36`) 随之修正。注意：`header_len` 的语义是"magic 之后的字节数"，应为 36 而非当前的 34。同步更新协议规范。
- 补充边界测试：payload 长度 1160 应通过，1161 应被拒绝。

### HIGH-2：旧 `OutputProfilesConfig` 系统处于僵尸状态 — 仍被写入但不再被录制循环读取

相关位置：

- `src/ipc/mod.rs:1476-1497`（`save_output_profiles_config` 仍存在且仍被前端调用）
- `src/ipc/mod.rs:1489-1492`（仍调用 `DashboardPublisher::set_config`）
- `src/ipc/mod.rs:1493`（仍调用 `sync_dashboard_subscriptions_from_disk`）
- `src-ui/components/TelemetryWorkspaceView.tsx:283`（前端仍调用 `save_output_profiles_config`）
- `src/dashboard/output.rs:138`、`169`（`publish_frame`/`publish_fields` 无调用方）

问题说明：

`save_output_profiles_config` IPC 命令仍存在，`TelemetryWorkspaceView` 仍调用它保存 output profiles。保存时：

1. 写入 `output_profiles.json` — 文件被写入但录制循环不再读取。
2. 调用 `DashboardPublisher::set_config` — publisher 配置被更新但不再被喂入数据（`publish_fields`/`publish_frame` 无调用方）。
3. 调用 `sync_dashboard_subscriptions_from_disk` — 此函数现在从 `RemoteDashboardState` 读取远程订阅（无活跃 session，返回空），不再从 `output_profiles.json` 读取。

用户在 Telemetry Workspace 配置 UDP profile 并保存后：
- 配置写入磁盘 ✓
- DashboardPublisher 收到配置 ✓
- 但录制线程不订阅这些字段 ✗
- 且 DashboardPublisher 不再推送数据 ✗

UI 不会提示任何失败，用户认为配置已生效但实际功能不工作。

影响：

- 用户在 TelemetryWorkspaceView 配置的远程 profile 不会产生任何遥测推送。
- 旧系统被部分删除但未完全清理，产生 confusing 的 zombie 代码路径。
- `DashboardPublisher` 仍占用 Tauri state 资源但功能为空。

修复建议：

1. 短期：在 `save_output_profiles_config` 中添加日志或 toast 警告，提示旧 output profiles 已被新远程 dashboard 系统取代。
2. 中期：将 `TelemetryWorkspaceView` 的远程 dashboard 配置迁移到新系统（`RemoteDevicesConfig` + `StreamProfilesConfig` + `RemoteDashboardBindingsConfig`），移除 `save_output_profiles_config` 和 `DashboardPublisher`。
3. 或在 `TelemetryWorkspaceView` 中移除远程 dashboard 配置 UI，仅保留本地 overlay 配置，远程配置统一在 `DashboardView` 的 `RemoteDashboardTab` 中管理。

### HIGH-3：无旧配置迁移 — `output_profiles.json` → `remote_*.json`

相关位置：

- `src/dashboard/remote/config.rs:56-64`（`RemoteDevicesConfig::default()` — `network_remote_enabled: false`）
- `src/dashboard/remote/runtime.rs:30-53`（`new` 加载三个新 config 文件，不读取旧 `output_profiles.json`）
- `src/dashboard/mod.rs`（旧 `OutputProfilesConfig` 仍存在）

问题说明：

旧系统使用单一 `output_profiles.json`，包含 `network_remote_enabled`、`serial_remote_enabled`、`profiles[]`（每个 profile 含 id/name/transport/encoding/hz/channels）。

新系统使用三个独立配置文件：
- `remote_devices.json`（`RemoteDevicesConfig` — 设备列表 + 全局开关）
- `remote_stream_profiles.json`（`StreamProfilesConfig` — 流 profile 定义）
- `remote_dashboard_bindings.json`（`RemoteDashboardBindingsConfig` — 设备-layout-profile 绑定）

**无任何迁移逻辑**将旧 `output_profiles.json` 的内容转换为新配置。`RemoteDevicesConfig::default()` 的 `network_remote_enabled` 为 `false`，无从旧 profile 推导。

这**回归了上一轮审计 CRITICAL-2 的后端迁移问题** — 上一轮修复了从 enabled UDP/serial profile 推导全局开关，新系统重新引入了"旧配置缺少全局开关时默认 false"的问题。

影响：

- 已配置 UDP profile 的用户升级后：
  - `remote_devices.json` 创建为默认值（`network_remote_enabled: false`，`devices: []`）
  - 旧 `output_profiles.json` 仍在磁盘上但录制循环不再读取
  - 远程 dashboard 功能被静默禁用
  - 用户需在新 UI 中重新配置所有设备和 profile

修复建议：

1. 在 `RemoteDashboardRuntime::new` 中检查旧 `output_profiles.json` 是否存在且 `remote_devices.json` 不存在，执行一次性迁移：
   - 从旧 profiles 中提取 UDP profile，转换为 `RemoteDeviceEntry`（device_id 用 profile.id，last_known_ip 用 profile.transport.host）
   - 从旧 profiles 中提取 channels/hz/encoding，转换为 `StreamProfileEntry`
   - `network_remote_enabled` 从旧 `network_remote_enabled` 字段或 enabled UDP profile 推导
2. 迁移后可选择重命名旧文件为 `output_profiles.json.bak` 或删除。
3. 补充测试：构造旧 `output_profiles.json`（含 enabled UDP profile），断言迁移后 `remote_devices.json` 包含对应设备且 `network_remote_enabled == true`。

### HIGH-4：`toggle_network_remote` 持锁执行文件 I/O 和 socket 绑定

相关位置：

- `src/ipc/mod.rs:2330-2340`（`toggle_network_remote` 获取 `RemoteDashboardState` 锁）
- `src/dashboard/remote/runtime.rs:56-72`（`enable_network_remote` 在锁内执行 `save_devices_config` + `DiscoveryService::bind`）

问题说明：

```rust
fn toggle_network_remote(
    enabled: bool,
    remote: tauri::State<'_, RemoteDashboardState>,
) -> IpcResult<bool> {
    let mut guard = remote.lock()...; // 持锁
    if enabled {
        guard.enable_network_remote()?; // 文件写入 + socket 绑定
    } else {
        guard.disable_network_remote(); // 文件写入
    }
    Ok(guard.is_network_remote_enabled())
}
```

`enable_network_remote` 在持有 `std::sync::Mutex` 锁的情况下执行：
1. `save_devices_config()` — 文件 I/O
2. `DiscoveryService::bind()` — UDP socket 绑定（可能失败/阻塞）

持锁期间，前端每 2 秒轮询的 `get_remote_dashboard_state` 被阻塞，UI 冻结。

影响：

- 用户切换 Network Remote 开关时 UI 可能短暂冻结。
- 若 socket 绑定耗时较长（端口冲突、防火墙检查），冻结时间延长。

修复建议：

1. 在锁内仅修改内存状态和保存配置路径，将 socket 绑定移到锁外。
2. 或将 `RemoteDashboardState` 改为 `tokio::sync::Mutex`（Tauri 异步环境），避免阻塞 IPC 线程。
3. 或将文件 I/O 和 socket 绑定改为异步操作。

### HIGH-5：clippy 质量门禁失败（5 个 error）

相关位置：

- `src/dashboard/remote/discovery.rs:59-63`（`Default for DiscoveryRegistry` 可派生）
- `src/dashboard/remote/discovery.rs:265-274`（`AnnounceMessage::new` — `too_many_arguments` 8/7）
- `src/dashboard/remote/data.rs:161-170`（`DataSender::add_session` — `too_many_arguments` 8/7）
- 另有 2 个 `this impl can be derived` 错误

问题说明：

上一轮审计关闭时 `cargo clippy --all-targets --all-features -- -D warnings` 已通过。本轮新增代码后该命令重新失败，当前 5 个 error：

- `this impl can be derived` × 2 — 手动 `impl Default` 可用 `#[derive(Default)]` 替代
- `DiscoveryRegistry` 缺少 `Default` 实现
- `too_many_arguments` (8/7) — `AnnounceMessage::new`
- `too_many_arguments` (8/7) — `DataSender::add_session`

影响：

- 质量门禁从上一轮关闭时的通过状态退回失败状态。
- CI 若启用 clippy `-D warnings` 会被阻断。

修复建议：

- 将手动 `impl Default` 替换为 `#[derive(Default)]`（需为 `DiscoveryRegistry` 的字段添加 `Default` 支持）。
- 对 `AnnounceMessage::new` 和 `DataSender::add_session` 因参数较多导致的 `too_many_arguments`，考虑引入 builder pattern 或参数结构体，或沿用上一轮修复风格加局部 `#[allow(clippy::too_many_arguments)]`。
- 修复后补充一次 `cargo clippy --all-targets --all-features -- -D warnings` 复验。

### MEDIUM-1：`dashboard_subscriptions_for_remote_sessions` 无单元测试

相关位置：

- `src/recording/writer.rs:25-42`（新函数定义）
- `src/recording/writer.rs` tests 模块（旧测试 `remote_dashboard_subscriptions_include_enabled_udp_profiles` 已删除）

问题说明：

旧函数 `dashboard_subscriptions_for_remote_dashboards` 有对应测试 `remote_dashboard_subscriptions_include_enabled_udp_profiles`，该测试在本轮变更中被删除。新函数 `dashboard_subscriptions_for_remote_sessions` 无任何单元测试。

`merge_dashboard_subscriptions_keeps_fastest_interval` 测试被修改为使用硬编码 `DashboardItemSubscription` 而非通过新函数生成，降低了集成覆盖度。

影响：

- 新订阅路径无测试覆盖，active session → channel → subscription 的转换逻辑无回归保障。

修复建议：

- 补充测试：构造含活跃 Streaming session 的 `RemoteSessionRegistry` 和含 profile 的 `StreamProfilesConfig`，断言 `dashboard_subscriptions_for_remote_sessions` 返回的 channels 包含 profile 中定义的所有字段。
- 补充测试：Ready + start_when_live session 也应贡献 channels；Ready + !start_when_live 不贡献。

### MEDIUM-2：`DataEncoding::BinaryV1` 实际为 JSON 回退 — 头部 encoding 字段不匹配

相关位置：

- `src/dashboard/remote/data.rs:94-99`（`BinaryV1` 回退为 JSON payload）

问题说明：

```rust
let payload = match self.encoding {
    DataEncoding::Json => frame.fields_json.as_bytes().to_vec(),
    DataEncoding::BinaryV1 => {
        // Fallback to JSON for now — binary_v1 encoding deferred
        frame.fields_json.as_bytes().to_vec()
    }
};
```

`BinaryV1` 编码回退到 JSON，但 UDP 头部的 `encoding` 字段仍标记为 `0x02`（binary_v1）。设备端按 binary_v1 格式解析 payload 会失败。

影响：

- 若 session 协商了 binary_v1 编码，设备端收到的包头部声明 binary_v1 但 payload 实际是 JSON，解析失败。
- 协议规范建议优先实现 JSON，但未说明 binary_v1 回退时的头部 encoding 字段应如何标记。

修复建议：

- 回退时将 `encoding` 字段也改为 `0x01`（JSON），保持头部与 payload 一致。
- 或在 `setStreamProfile` 协商阶段即回退到 JSON，不将 binary_v1 作为 accepted encoding。
- 补充测试：binary_v1 profile 的 session 发送时，断言 UDP 包 encoding 字段为 JSON。

### MEDIUM-3：`discovery.rs` `poll_once` 缓冲区仅 2048 字节

相关位置：

- `src/dashboard/remote/discovery.rs:208`

问题说明：

```rust
let mut buf = [0u8; 2048];
```

Announce 消息包含 `device_id`、`device_name`、`instance_id`、`app_version`、`platform`、`capabilities` 等字段。长设备名、多编码 capabilities、长版本字符串组合时 JSON 可能超过 2048 字节。`recv_from` 截断后 JSON 解析失败，被静默忽略（`Err(_) => false`）。

影响：

- 大型 announce 消息被静默丢弃，设备不被发现。

修复建议：

- 将缓冲区增大至 4096 或 8192 字节（UDP 包最大 65535 字节，但 announce 不应超过 MTU）。
- 或在解析失败时记录 warn 日志而非静默忽略，便于诊断。
- 补充测试：发送 2048+ 字节 announce，断言设备被正确注册。

### MEDIUM-4：`active_subscription_channels` 与 `highest_active_hz` 状态过滤不一致

相关位置：

- `src/dashboard/remote/session.rs:228-251`（`active_subscription_channels` 包含 Streaming + Ready-with-start_when_live）
- `src/dashboard/remote/session.rs:254-268`（`highest_active_hz` 仅考虑 Streaming）

问题说明：

`active_subscription_channels` 包含 `Streaming` 和 `Ready`（且 `start_when_live == true`）状态的 session 的 channels。但 `highest_active_hz` 只考虑 `Streaming` 状态的 session 的 Hz。

若一个 session 处于 `Ready + start_when_live` 状态，其 channels 被订阅，但其 Hz 不参与 interval 计算。`dashboard_subscriptions_for_remote_sessions` 使用 `highest_active_hz` 计算 interval，可能导致 Ready session 的字段刷新率不足。

影响：

- Ready 状态 session 需要的字段以低于其 profile Hz 的刷新率被订阅，数据延迟。

修复建议：

- `highest_active_hz` 应与 `active_subscription_channels` 使用相同的状态过滤条件。
- 补充测试：Ready + start_when_live session 的 Hz 应参与 highest_hz 计算。

### MEDIUM-5：`control.rs` handshake 硬编码 message ID `"msg-hi-1"`

相关位置：

- `src/dashboard/remote/control.rs:114`

问题说明：

```rust
let hello = HelloMessage {
    message_id: "msg-hi-1".to_string(), // 硬编码
    ...
};
```

不使用 `next_msg_id()` 计数器。虽然 handshake 在 session 创建时仅执行一次，但与协议的 request/reply 语义不一致，且 `HandshakeResult` 中不返回 `message_id` 供后续追踪。

影响：

- 并发握手（虽然 unlikely）会产生相同 message ID，设备端无法区分。
- 与协议规范 "需要确认的请求消息包含唯一 messageId" 不一致。

修复建议：

- 使用动态生成的 message ID（如 UUID 或递增计数器）。
- 或在 `ControlSession::new` 中初始化 `next_message_id` 为 1，handshake 中使用 `next_msg_id()`。

### MEDIUM-6：`control.rs` `send_ping` 修改 stream `read_timeout` 影响后续读取

相关位置：

- `src/dashboard/remote/control.rs:240-242`

问题说明：

```rust
self.stream
    .set_read_timeout(Some(Duration::from_secs(2)))
```

`send_ping` 将 read_timeout 设为 2 秒。后续 `request` 调用会继承此超时。`try_read_status` 会重设为 100ms，但如果在 `send_ping` 和 `try_read_status` 之间有 `request` 调用，则使用 2 秒超时，对大 layout 操作可能过短。

影响：

- 心跳后的控制命令可能因 2 秒超时而失败。

修复建议：

- `send_ping` 后恢复原始 read_timeout，或每次 `request` 调用前显式设置适当的超时。
- 或在 `request` 方法中固定设置 read_timeout（如 10 秒），不依赖外部设置。

### MEDIUM-7：`RemoteDevicesConfig` 包含未使用的 `serial_remote_enabled` 字段

相关位置：

- `src/dashboard/remote/config.rs:23`、`59`

问题说明：

新系统仅支持网络传输（UDP discovery + TCP control + UDP data），无 serial 传输实现。`serial_remote_enabled` 字段为死代码，`RemoteDashboardStateResponse` 也不暴露此字段。前端 `DashboardView.tsx` 已移除 serial 开关 UI。

影响：

- 死字段可能误导开发者以为 serial 传输已支持或计划支持。
- 配置文件中残留无用字段。

修复建议：

- 移除 `serial_remote_enabled` 字段，或在注释中明确标注 "reserved for future serial transport"。

### MEDIUM-8：`hash_session_id` 使用 `DefaultHasher` — 跨版本不稳定

相关位置：

- `src/dashboard/remote/session.rs:310-316`

问题说明：

```rust
pub fn hash_session_id(session_id: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    hasher.finish()
}
```

`DefaultHasher`（SipHash 1-2-3）文档明确声明**不保证跨 Rust 版本稳定**。用于 wire protocol 的 `session_id_hash` 需要跨平台/跨版本一致，设备端也需要用相同算法计算。协议规范未定义哈希算法。

影响：

- Rust 版本升级后 `session_id_hash` 可能变化，设备端无法匹配。
- 设备端实现者不知道应使用哪种哈希算法。

修复建议：

- 使用固定算法（如 FNV-1a 或 CRC64），并在协议规范中明确声明。
- 或在 UDP 头部中直接包含完整 session_id（字符串），避免哈希不一致问题（代价是增加头部大小）。

### MEDIUM-9：`make_session_id` 使用毫秒时间戳可能碰撞

相关位置：

- `src/dashboard/remote/session.rs:300-307`

问题说明：

```rust
pub fn make_session_id(device_id: &str) -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    format!("sess-{device_id}-{ts:x}")
}
```

同一毫秒内为同一设备创建两个 session 会产生相同 ID。虽然概率低，但在快速连续操作或测试场景中可能发生。

影响：

- Session ID 碰撞导致 `HashMap` 覆盖，前一个 session 被丢失。

修复建议：

- 加入随机组件：`format!("sess-{device_id}-{ts:x}-{rand:x}")`，使用 `rand` crate 或 `uuid`。
- 或使用 UUID v4 作为 session ID。

### LOW-1：前端 "Connect" 按钮无 `onClick` 处理器

相关位置：

- `src-ui/components/DashboardView.tsx:509`

问题说明：

```tsx
<button className={styles.actionButton} type="button">
    Connect
</button>
```

按钮无 `onClick`，点击无反应。用户无法理解为何无法连接。

修复建议：

- 设为 `disabled` 并添加 tooltip 说明 "Coming soon"。
- 或添加 `onClick` 调用未来的 `connect_device` IPC 命令。

### LOW-2：前端固定 2 秒轮询，空闲时浪费资源

相关位置：

- `src-ui/components/DashboardView.tsx:319`

问题说明：

```tsx
const interval = setInterval(fetchState, 2000);
```

无论是否有设备或活跃 session，始终每 2 秒轮询。无设备时可降低频率或暂停。

修复建议：

- 无设备时延长轮询间隔至 5-10 秒，有设备时保持 2 秒。
- 或在页面不可见时（`document.hidden`）暂停轮询。

### LOW-3：旧 `output_profiles.json` 未清理

相关位置：

- `src/ipc/mod.rs:1488`（仍写入 `output_profiles.json`）

问题说明：

旧配置文件仍被 `save_output_profiles_config` 写入但不再被录制循环读取。文件残留在磁盘上，用户可能困惑为何配置保存后无效果。

修复建议：

- 在完成 HIGH-2 的迁移后，删除 `output_profiles.json` 或重命名为 `.bak`。
- 在 `save_output_profiles_config` 中添加 deprecation 日志。

### LOW-4：`DiscoveryService::run(self)` 消费所有权 — 设计不适合线程启动

相关位置：

- `src/dashboard/remote/discovery.rs:237`

问题说明：

`run(self)` 取所有权。若要从 `RemoteDashboardRuntime` 中取出 service 传给线程，runtime 将失去对 `send_probe()` 的访问。当前 `send_probe` 通过 `self.discovery_service.as_ref()` 调用，若 service 被 `take()` 到线程中则无法发送 probe。

修复建议：

- 将 `DiscoveryService` 拆分为 `DiscoveryProbeSender`（Arc 包裹 socket）和 `DiscoveryReceiver`（线程持有）。
- 或将整个 `DiscoveryService` 包裹在 `Arc<Mutex<>>` 中，线程和 runtime 共享访问。

### LOW-5：`data.rs` 每个 session 独立绑定 UDP socket

相关位置：

- `src/dashboard/remote/data.rs:58`

问题说明：

```rust
let socket = UdpSocket::bind("0.0.0.0:0")
```

每个 session 创建新的 UDP socket。大量 session 会消耗大量文件描述符。

修复建议：

- 共享单个 UDP socket，通过 `send_to` 指定不同目标地址。
- 或限制最大 session 数量。

### LOW-6：Discovery announce 无输入长度/字符校验

相关位置：

- `src/dashboard/remote/discovery.rs:67-97`（`upsert` 直接接受 announce 字段）

问题说明：

`device_id`、`device_name`、`app_version`、`platform` 等字段直接从网络反序列化后存入 registry，无长度限制或字符过滤。`eprintln!` 日志直接打印这些字段，可能被用于日志注入。

影响：

- 恶意设备可发送超长字段或包含控制字符的 announce，影响日志可读性。
- 非阻塞性问题，LAN 环境下风险低。

修复建议：

- 对 `device_id`、`device_name` 等字段设置最大长度（如 256 字节）。
- 日志中过滤控制字符或使用 `{:?}` 格式化。

## 验证结果

已执行：

- `cargo check`：通过。
- `cargo test`：通过，62 个单元测试 + e2e pipeline、lap stats、real LD、regression monza 测试均通过。
- `cmd /c npm run lint`：通过。
- `cmd /c npm test`：通过，2 个 test files、5 个 tests。
- `cmd /c npm run build`：通过。Vite 仍提示主 chunk `719.93 kB`，该问题对应上一轮已明确暂缓的前端包体偏大问题。
- `cargo clippy --all-targets --all-features -- -D warnings`：**失败**，5 个 error，见 HIGH-5。

## 建议修复顺序

按严重程度和功能依赖关系排序：

1. **CRITICAL-1 + CRITICAL-2 + CRITICAL-3 + CRITICAL-4**（四条断裂点一并修复）：这四个问题构成远程 dashboard 功能的最小可用闭环。仅修复其中任何一个都无法恢复功能。需要同时：
   - 启动 discovery 接收线程（CRITICAL-2）
   - 实现设备连接/配对/会话启动 IPC 命令和前端 handler（CRITICAL-3）
   - 在录制循环中将 telemetry 喂入 `RemoteSessionRegistry::broadcast_telemetry`（CRITICAL-1）
   - 确保录制循环 `load_dashboard_items` 加载远程订阅且 restart 后不丢失（CRITICAL-4）
2. **HIGH-1**（`DATA_HEADER_SIZE` 常量错误）：修正常量值和 `header_len` 字段，同步更新协议规范。
3. **HIGH-2**（旧系统僵尸状态）：清理或迁移旧 `OutputProfilesConfig` 路径，消除混淆。
4. **HIGH-3**（无配置迁移）：实现旧 `output_profiles.json` → 新 `remote_*.json` 的一次性迁移。
5. **HIGH-4**（持锁 I/O）：将文件 I/O 和 socket 绑定移出 mutex 锁作用域。
6. **HIGH-5**（clippy 门禁）：修复 5 个 clippy error，恢复 `-D warnings` 通过状态。
7. **MEDIUM-1 至 MEDIUM-9**：按各自说明修正并补充对应单元测试。
8. **LOW-1 至 LOW-6**：后续清理中处理，不阻塞当前审计关闭。

## 当前状态

- 状态：⛔ **OPEN — 未修复。**
- 新增代码为远程 dashboard 架构重写的第一阶段脚手架。协议定义（`protocol.rs`）、配置模型（`config.rs`）、各层基础实现（`discovery.rs`、`control.rs`、`data.rs`、`session.rs`、`runtime.rs`）和前端 UI 框架已就位，但**四条关键数据流断裂**导致功能完全不可用。
- 构建状态：`cargo check`/`cargo test`/`npm lint`/`npm test`/`npm build` 通过；`cargo clippy -D warnings` 失败。
- 暂缓继承项：前端主 chunk 超过 700 kB 的包体问题仍按上一轮审计结论暂缓。
- 本轮未引入安全漏洞（协议规范已明确首版使用 opaque token + LAN only），但 `DefaultHasher` 跨版本不稳定（MEDIUM-8）和 announce 无输入校验（LOW-6）需在后续版本中关注。

---

## 复审记录（2026-06-18）

修复者声称 24 项问题已全部修复。复审逐项验证后结论：**18 项完全修复，4 项部分修复（有残留），2 项未修复。复审不通过。**

### 验证命令结果

| 命令 | 结果 |
|---|---|
| `cargo check` | ✅ 通过 |
| `cargo test` | ✅ 通过（68 单元测试 + 集成测试，比审计时新增 6 个测试） |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ 通过（HIGH-5 已修复） |
| `npm run lint` | ✅ 通过 |
| `npm test` | ✅ 通过 |
| `npm run build` | ✅ 通过 |

### 逐项复审结果

| 问题 | 状态 | 复审说明 |
|---|---|---|
| CRITICAL-1 | ⚠️ 部分修复 | telemetry channel 已接线（`main.rs` 创建 channel → `auto.rs:399-407` 发送 → `runtime.rs:83` `drain_telemetry` 接收），但 `drain_telemetry` **仅在前端 2-5 秒轮询 `get_remote_dashboard_state` 时被调用**（`ipc/mod.rs:2307`）。协议设计 60Hz，实际遥测刷新率 0.2-0.5Hz。远程设备每 2-5 秒收到一帧，dashboard 几乎不实时。**功能性缺陷，必须修复。** |
| CRITICAL-2 | ✅ 已修复 | `enable_network_remote` 中 `thread::spawn` + `run_arc(Arc<Self>)`，新增 `stopped: Arc<AtomicBool>` 停止信号。`DiscoveryService` 改为持有 `Arc<UdpSocket>`，probe 发送和接收线程共享。 |
| CRITICAL-3 | ✅ 已修复 | 新增 `connect_device`/`disconnect_device`/`start_stream`/`stop_stream` IPC 命令（`ipc/mod.rs:2450-2492`），`RemoteDashboardRuntime` 实现 `connect_device`/`start_stream`/`stop_stream`/`disconnect_device`（`runtime.rs:204-450`），前端 Connect 按钮 `onClick={handleConnect}`（`DashboardView.tsx:549`）。 |
| CRITICAL-4 | ✅ 已修复 | 录制循环 `load_dashboard_items` 后通过 `remote_subscriptions` 共享状态合并远程订阅（`auto.rs:309-316`），`UpdateRemoteSubscriptions` 命令更新共享状态（`auto.rs:431-435`），`sync_dashboard_subscriptions_from_disk` 调用 `update_remote_subscriptions`（`ipc/mod.rs:1522`）。 |
| HIGH-1 | ✅ 已修复 | `DATA_HEADER_SIZE` 改为 40（`protocol.rs:26`），`header_len` 相应改为 36，MTU 边界测试 `mtu_boundary_payload_1160_passes_1161_rejected`（`data.rs:332`）。 |
| HIGH-2 | ❌ 未修复 | `DashboardPublisher` 仍作为 Tauri state（`main.rs:76-81`），`save_output_profiles_config` 仍存在并调用 `set_config`（`ipc/mod.rs:1476-1492`），`TelemetryWorkspaceView` 仍调用它。HIGH-3 的迁移会重命名旧文件，但此命令会重新写入新 `output_profiles.json`，产生僵尸路径。 |
| HIGH-3 | ✅ 已修复 | `try_migrate_from_legacy_output_profiles`（`config.rs:26-105`）完整实现：读取旧 `output_profiles.json`，转换 UDP profile 为 `StreamProfileEntry`，推导 `network_remote_enabled`，创建三个新配置文件，重命名旧文件为 `.migrated`。 |
| HIGH-4 | ⚠️ 部分修复 | `toggle_network_remote` 已分阶段锁（`ipc/mod.rs:2375-2436`），但 `connect_device`（TCP 连接 + handshake，可能 5+ 秒）和 `start_stream`（TCP request/reply）仍全程持有 `RemoteDashboardState` 锁。期间前端轮询的 `get_remote_dashboard_state` 被阻塞，UI 冻结。 |
| HIGH-5 | ✅ 已修复 | clippy `-D warnings` 通过。`#[derive(Default)]` 替代手动 impl，`#[allow(clippy::too_many_arguments)]` 标注 `AnnounceMessage::new`。 |
| MEDIUM-1 | ✅ 已修复 | `writer.rs` 新增 `remote_sessions_contribute_channels` 和 `ready_with_start_when_live_contributes` 测试（`writer.rs:269,306`）。 |
| MEDIUM-2 | ✅ 已修复 | `build_and_send` 中 `wire_encoding` 在 `BinaryV1` 回退时改为 `DataEncoding::Json`（`data.rs:112-115`），头部与 payload 一致。 |
| MEDIUM-3 | ✅ 已修复 | `poll_once` 缓冲区改为 8192 字节（`discovery.rs:234`），解析失败记录 `parse_failed` 日志（`discovery.rs:251-258`）。 |
| MEDIUM-4 | ✅ 已修复 | `highest_active_hz` 改为与 `active_subscription_channels` 相同的过滤条件（包含 Streaming + Ready-with-start_when_live），新增测试 `highest_active_hz_includes_ready_with_start_when_live`（`session.rs:352`）。 |
| MEDIUM-5 | ✅ 已修复 | handshake 的 `message_id` 改为 `format!("msg-hi-{}", uuid::Uuid::new_v4())`（`control.rs:115`）。 |
| MEDIUM-6 | ✅ 已修复 | `send_ping` 保存并恢复 `prev_timeout`（`control.rs:242,260`）。 |
| MEDIUM-7 | ✅ 已修复 | `serial_remote_enabled` 字段已从 `RemoteDevicesConfig` 移除（`config.rs:165-170`）。 |
| MEDIUM-8 | ✅ 已修复 | `hash_session_id` 改为 FNV-1a 64-bit 固定算法（`session.rs:335-344`），新增稳定性测试 `hash_session_id_is_stable`（`session.rs:403`）。 |
| MEDIUM-9 | ✅ 已修复 | `make_session_id` 加入 UUID 随机组件 `format!("sess-{device_id}-{ts:x}-{}", &rand_comp[..8])`（`session.rs:326-328`）。 |
| LOW-1 | ✅ 已修复 | Connect 按钮 `onClick={() => handleConnect(device.deviceId)}`（`DashboardView.tsx:549`），`handleConnect` 调用 `connect_device` IPC（`DashboardView.tsx:365-368`）。 |
| LOW-2 | ⚠️ 部分修复 | 空闲时 5 秒、有设备/会话时 2 秒（`DashboardView.tsx:330-332`），但未实现 `document.hidden` 页面不可见时暂停。 |
| LOW-3 | ⚠️ 部分修复 | 迁移时重命名为 `.migrated`（`config.rs:96`），但 `save_output_profiles_config` 仍存在并会重新写入 `output_profiles.json`（`ipc/mod.rs:1488`）。随 HIGH-2 一并处理。 |
| LOW-4 | ✅ 已修复 | `run_arc(self: Arc<Self>)` 替代 `run(self)`（`discovery.rs:276`），线程和 runtime 共享 `Arc<DiscoveryService>`。 |
| LOW-5 | ❌ 未修复 | `SessionSender::new` 仍每 session 独立 `UdpSocket::bind("0.0.0.0:0")`（`data.rs:58`）。少量 session 时影响小，暂缓可接受。 |
| LOW-6 | ✅ 已修复 | `sanitize_field` 截断至 256 字符并过滤非 ASCII graphic 字符（`discovery.rs:72-79`），`upsert` 对所有字符串字段调用（`discovery.rs:86-90`）。 |

### 阻塞关闭的问题（必须修复）

**1. CRITICAL-1 残留 — 遥测刷新率严重不达标**

数据管线已接线，但 `drain_telemetry` 的调用时机依赖前端 `get_remote_dashboard_state` 轮询（2-5 秒），而非独立定时器。`drain_telemetry` 内部丢弃旧帧只保留最新一帧（`runtime.rs:88-93`）。结果：协议设计 60Hz，实际 0.2-0.5Hz，远程设备每 2-5 秒收到一帧遥测，dashboard 几乎不实时。

修复建议：在 `RemoteDashboardRuntime` 中新增独立定时器线程，按目标 Hz（如 60Hz，即每 ~16ms）周期性调用 `drain_telemetry`，不依赖前端轮询。或在录制线程中直接持有 `DataSender` 的 `Arc<Mutex<>>` 引用，收到帧后直接广播，绕过 channel 中转。

**2. HIGH-2 未修复 — 旧系统僵尸状态**

`DashboardPublisher` 仍作为 Tauri state，`save_output_profiles_config` 仍存在并调用 `set_config`，`TelemetryWorkspaceView` 仍调用它。HIGH-3 的迁移逻辑会重命名旧 `output_profiles.json`，但 `save_output_profiles_config` 会重新写入新文件，产生 confusing 的僵尸路径。用户在 TelemetryWorkspaceView 保存配置后，文件被写入但功能不工作。

修复建议：移除 `save_output_profiles_config` IPC 命令和 `DashboardPublisher` Tauri state，将 `TelemetryWorkspaceView` 的远程配置 UI 迁移到新系统或移除。

**3. HIGH-4 部分残留 — connect_device/start_stream 持锁网络操作**

`toggle_network_remote` 已分阶段锁（改善），但 `connect_device`（TCP 连接 + handshake，可能 5+ 秒）和 `start_stream`（TCP request/reply）仍全程持有 `RemoteDashboardState` 锁。期间前端 2 秒轮询的 `get_remote_dashboard_state` 被阻塞，UI 冻结。

修复建议：将 `connect_device` 和 `start_stream` 的 TCP 网络操作移出锁作用域，或改为异步执行（`tokio::spawn`），锁内仅更新状态。

### 可暂缓的问题

- **LOW-2**（无 `document.hidden` 检查）— 已有动态间隔，优化项
- **LOW-3**（`save_output_profiles_config` 仍写入）— 随 HIGH-2 一并处理
- **LOW-5**（每 session 独立 socket）— 少量 session 时影响小

### 复审结论

**⛔ 复审不通过。** CRITICAL-1 残留导致核心功能（实时遥测）不达标，必须修复后方可关闭。HIGH-2 和 HIGH-4 建议同步修复。其余 18 项修复质量良好，新增 6 个测试，clippy 门禁恢复通过。

---

## 第二次复审记录（2026-06-18）

针对第一次复审提出的 3 个阻塞问题（CRITICAL-1 残留、HIGH-2 未修复、HIGH-4 部分残留）进行复审。

### 验证命令结果

| 命令 | 结果 |
|---|---|
| `cargo check` | ✅ 通过 |
| `cargo test` | ✅ 通过（68 单元测试 + 集成测试） |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ 通过 |
| `npm run lint` | ✅ 通过 |
| `npm test` | ✅ 通过（5 tests） |
| `npm run build` | ✅ 通过 |

### 阻塞问题复审

**1. CRITICAL-1 残留 — ✅ 已修复**

`main.rs:101-114` 新增独立定时器线程，每 50ms（20Hz）调用 `drain_telemetry`，不再依赖前端轮询：

```rust
std::thread::spawn(move || {
    let interval = Duration::from_millis(50);
    loop {
        std::thread::sleep(interval);
        if let Ok(mut guard) = timer_state.lock() {
            guard.drain_telemetry();
        }
    }
});
```

遥测刷新率从 0.2-0.5Hz 提升至 20Hz，满足实时性要求。`RemoteDashboardRuntime` 改为 `Arc<std::sync::Mutex<>>` 以便定时器线程共享。

**2. HIGH-2 未修复 — ✅ 已修复**

- `DashboardPublisher` 已从 `main.rs` 和 `ipc/mod.rs` 完全移除（grep 无匹配）。
- `save_output_profiles_config` IPC 命令已删除。
- `DashboardOutputState` 类型别名已删除。
- `TelemetryWorkspaceView.tsx:275-277` 注释说明 `saveOutputs` 已移除，远程配置改由 `DashboardView` Remote tab 管理。
- `TelemetryWorkspaceView` 保留 `validate_output_profiles_config`（只读校验，不写入磁盘），可接受。

**3. HIGH-4 部分残留 — ✅ 已修复**

`connect_device`（`ipc/mod.rs:2407-2449`）改为三阶段锁：
- Phase 1：锁内提取连接信息（`connect_device_prepare`，快速）
- Phase 2：锁外执行 TCP connect + handshake（5+ 秒，不持锁）
- Phase 3：锁内注册 control session（`connect_device_commit`，快速）

`start_stream`（`ipc/mod.rs:2462-2489+`）同样三阶段：
- Phase 1：锁内提取 binding/profile/device_ip（`start_stream_prepare`，快速）
- Phase 2：锁外取出 control session 执行 TCP request/reply（`take_control_session` + `start_stream_tcp_ops`）
- Phase 3：锁内注册 DataSender 和 session

TCP 网络操作均在锁外执行，前端轮询不再被阻塞。

### 其他可暂缓项确认

- **LOW-2**（无 `document.hidden`）— 未变，暂缓可接受
- **LOW-3**（`save_output_profiles_config` 仍写入）— 随 HIGH-2 已解决，IPC 命令已删除
- **LOW-5**（每 session 独立 socket）— 未变，少量 session 时影响小，暂缓可接受

### 第二次复审结论

**✅ 复审通过。** 3 个阻塞问题均已修复：

1. CRITICAL-1 残留：独立 20Hz 定时器线程，遥测实时性达标
2. HIGH-2：旧 `DashboardPublisher`/`save_output_profiles_config` 完全移除
3. HIGH-4：`connect_device`/`start_stream` 分阶段锁，TCP I/O 在锁外

全部验证命令通过（cargo check/test/clippy、npm lint/test/build）。24 项问题中 21 项完全修复，3 项可暂缓（LOW-2、LOW-5 及随 HIGH-2 一并解决的 LOW-3）。

**审计关闭。**
