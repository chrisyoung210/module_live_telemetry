# Dashboard 全链路实时性优化方案

日期：2026-06-29
范围：`module_live_telemetry` → `acc-coach` → `module_local_dashboard` / remote dashboard / serial dashboard

---

## 1. 背景与问题

用户通过录屏暂停对比 local dashboard 与 ACC 游戏内 HUD，发现存在可感知的数据延迟（约 0.2s~0.5s 级别）。经过全链路代码审计，当前数据流如下：

```
ACC shared memory
  │
  ① Recording Loop (module_live_telemetry)
     · poll_hz clamp 30~120，sleep_remaining 控制采样率
     · read_all_telemetry_frame → 读 physics+graphics 共享内存
     · dist.distribute(Arc::clone) → bounded(1) 通道 try_send
  │
  ② Dashboard Thread (module_live_telemetry)
     · process_frame → schedule_buckets 按 interval 分桶调度
     · 计算到期 item → 稀疏 HashMap<String,f64>
     · sink.send(frame.clone()) → ChannelSink::try_send → bounded(64) 通道
  │
  ③ auto_recording_loop (acc-coach)
     · dashboard_rx.try_recv() → 逐帧 drain
     · 每帧做：telemetry_tx 转发 + merge_latest_dashboard_frame + push_merged_to_bus
     · push_merged_to_bus：alias 翻译(新 HashMap 分配 + 逐 key 翻译) + bus.push_frame(clone)
     · 通道空时 sleep(1ms)
  │
  ④ Frontend setInterval(poll, 33ms) (module_local_dashboard)
     · invoke("poll_dashboard_frame") → Tauri IPC
     · bus.latest_frame() → Mutex + clone
     · serde JSON 序列化 → 跨进程 → JS 反序列化
  │
  ⑤ React Render + Paint
     · {...prev, ...values} 对象展开
     · setFullFrame + setHistoryVersion → React re-render
     · DashboardRegionRenderer → DynamicDashboardControl (已 memo)
     · browser paint
  │
  用户看到数值
```

### 延迟拆解估计（120Hz 后端 + 33ms 前端轮询）

| 段 | 估计延迟 | 说明 |
|---|---|---|
| ① 采样 | 0 ~ 8ms | poll_interval 平均等待 |
| ② 计算+发送 | 0 ~ 1ms | schedule bucket + HashMap + try_send |
| ③ acc-coach 转发 | 0 ~ 533ms | **bounded(64) 可积压 533ms@120Hz**，drain 后 0~1ms |
| ④ 前端 polling | 0 ~ 33ms | **setInterval 平均 17ms**，+ IPC 1~5ms |
| ⑤ React+paint | 4 ~ 16ms | 1~2 帧 |
| **总计** | **~26 ~ 55ms（正常），峰值 580ms+（积压）** | |

### 问题根因

1. **bounded(64) 通道积压**：120Hz 下 64 帧可缓存 533ms，是 0.5s 延迟的主要来源。
2. **前端 polling 等待**：33ms setInterval 平均引入 17ms 等待，是单一段最大延迟。
3. **录制线程 1ms sleep**：通道空时 sleep(1ms) 引入不必要延迟。
4. **alias 翻译在 120Hz 录制线程**：每帧分配 HashMap + 逐 key 翻译，但 IPC 只有 30Hz 读取。
5. **drain 循环中每帧做 bus push**：中间帧的 alias 翻译和 bus push 全部被覆写，纯浪费。
6. **HashMap<String,f64> string key**：每帧 string key 分配、IPC JSON 序列化 string key、前端 string key 查找，对 network/serial remote 更是带宽浪费。

---

## 2. 优化目标

| 指标 | 当前 | 目标 |
|---|---|---|
| 端到端延迟 P50 | ~30ms | <10ms |
| 端到端延迟 P95 | ~55ms | <25ms |
| 端到端延迟 P99 | ~580ms（积压） | <50ms |
| 后端积压 | 可达 533ms | 0（最新值覆写） |
| 前端等待 | 平均 17ms | 0（事件驱动） |

---

## 3. 优化点清单

### A. acc-coach 后端链路

| 编号 | 优化点 | 收益 |
|---|---|---|
| A1 | 切换 Legacy bounded(64) 通道为 LatestValueSink | 消除积压风险，533ms → 0 |
| A2 | 消除 1ms sleep，改为 Select 阻塞等待 | 减少 0~1ms 延迟 |
| A3 | drain 循环中只处理最终帧的 bus push | 降 CPU，不降延迟 |
| A4 | alias 翻译从录制线程移到 IPC 线程 | 降 CPU，120Hz → 30Hz |
| A5 | 减少 clone 次数 | 降 CPU/分配 |

### B. module_live_telemetry（低优先级，告知用户）

| 编号 | 优化点 | 收益 |
|---|---|---|
| B1 | process_frame 中避免不必要的 clone | 微优化 |
| B2 | DashboardCompactPatch 传输 | 被 D6 覆盖 |
| B3 | 录制循环读取粒度优化 | 高复杂度低收益 |

### C. module_local_dashboard 前端

| 编号 | 优化点 | 收益 |
|---|---|---|
| C1 | 前端从 polling 切换为 event push | 平均 17ms → 0ms |
| C2 | 降低 frameMs 默认值（33→16 或更低） | polling 模式下 8ms 降幅 |
| C3 | useSyncExternalStore 替代 useState | 渲染成本下降 |
| C4 | 控件级字段选择器 | 高频字段不拖累低频控件 |

### D. 字段 ID 化（跨模块）

| 编号 | 优化点 | 收益 |
|---|---|---|
| D1 | Field ID Registry（SQLite 持久化） | ID 化基础设施 |
| D2 | Layout 编译器（string key → id） | alias 翻译从每帧降到加载时一次 |
| D3 | ID-based FrameBus + IPC | IPC payload 减小，前端查找更快 |
| D4 | module_dashboard_protocol 新增 compiled 类型 | 类型契约 |
| D5 | module_local_dashboard 前端 id-based 渲染 | string key → number key |
| D6 | Remote 路径改用 CompactPatch | UDP 带宽 -40~55% |
| D7 | Serial 路径复用 CompactPatch | 串口带宽 -50~75% |

---

## 4. 阶段拆分

### 拆分原则

1. **拓扑顺序**：每阶段不依赖未完成的改动。
2. **优先级**：优先对当前 local dashboard 链路优化最明显的点。
3. **可测试性**：每阶段完成后用户可做完整功能测试。
4. **不破坏**：后置阶段不大规模破坏前置阶段的改动。
5. **模块独立性**：尽量让同一阶段内不同模块可并行开发。

### 阶段总览

| 阶段 | 名称 | 涉及模块 | 主要收益 | 依赖 |
|---|---|---|---|---|
| 1 | acc-coach 后端链路优化 | acc-coach | 后端延迟 533ms→0，CPU 下降 | 无 |
| 2 | 前端 polling → event push | acc-coach + module_local_dashboard | 前端延迟 17ms→0ms | 无（与阶段1协同但可独立） |
| 3 | 前端渲染优化 | module_local_dashboard | 渲染成本下降，50控件稳定 | 阶段2（event push 基础上） |
| 4 | Field ID Registry + 编译器 + 协议类型 | acc-coach + module_dashboard_protocol | ID 化基础设施就绪 | 无（独立于1-3） |
| 5 | Local Dashboard ID 化硬切换 | acc-coach + module_local_dashboard | alias 翻译消除，payload 减小 | 阶段4 |
| 6 | Remote Dashboard ID 化 | acc-coach | UDP 带宽 -40~55% | 阶段4 |
| 7 | Serial + module_live_telemetry 微优化 | acc-coach + module_live_telemetry | 串口带宽，微优化 | 阶段6 |

### 阶段间依赖关系

```
阶段1 (acc-coach 后端) ──────────────────────────────┐
                                                      │
阶段2 (event push) ────── 阶段3 (前端渲染) ───────────┤
                                                      │
阶段4 (ID 基础设施) ──── 阶段5 (local ID 切换) ───────┤
                         阶段6 (remote ID) ── 阶段7 ──┘
```

- 阶段 1、2、4 互不依赖，理论上可并行启动。
- 阶段 3 依赖阶段 2（在 event push 基础上做 store）。
- 阶段 5、6 依赖阶段 4。
- 阶段 7 依赖阶段 6。
- **建议按 1→2→3→4→5→6→7 顺序执行**，因为阶段 1-2 是最大收益，应最先完成。

### 后置阶段对前置阶段的影响

| 后置阶段 | 影响的前置阶段 | 影响描述 | 是否破坏 |
|---|---|---|---|
| 阶段2 | 阶段1 | event emit 代码加在 auto_recording_loop 中，与阶段1的 LatestValueSink 改动在同一函数 | 不破坏，阶段1改 drain 逻辑，阶段2加 emit |
| 阶段3 | 阶段2 | useSyncExternalStore 替代阶段2的 useState，event listener 不变 | 不破坏，store 是 useState 的演进 |
| 阶段5 | 阶段1 | FrameBus 类型从 string-key 改为 id-based，auto_recording_loop 编码逻辑变 | 不破坏，LatestValueSink 机制不变，只改编码内容 |
| 阶段5 | 阶段2 | event payload 从 string-key 改为 id-based | 不破坏，event push 机制不变，只改 payload 格式 |
| 阶段5 | 阶段3 | store key 类型从 string 改为 number | 不破坏，store 机制不变，只改 key 类型 |
| 阶段6 | 阶段5 | remote 路径复用阶段5的 registry + 编译器 | 不破坏，阶段5改 local 路径，阶段6改 remote 路径 |

---

## 5. 各阶段详细文档

| 阶段 | 文档 | 说明 |
|---|---|---|
| 1 | [stage-1-acc-coach-backend-pipeline.md](./stage-1-acc-coach-backend-pipeline.md) | acc-coach 后端：LatestValueSink + Select 阻塞 + drain 优化 + alias 移位 + clone 减少 |
| 2 | [stage-2-frontend-polling-to-event-push.md](./stage-2-frontend-polling-to-event-push.md) | event push：acc-coach emit + module_local_dashboard listen + rAF 合帧 + polling fallback |
| 3 | [stage-3-frontend-render-optimization.md](./stage-3-frontend-render-optimization.md) | module_local_dashboard：useSyncExternalStore + 控件级字段选择器 |
| 4 | [stage-4-field-id-registry-and-compiler.md](./stage-4-field-id-registry-and-compiler.md) | acc-coach Field Registry（SQLite）+ Layout 编译器 + module_dashboard_protocol compiled 类型 |
| 5 | [stage-5-local-dashboard-id-switch.md](./stage-5-local-dashboard-id-switch.md) | local dashboard ID 化硬切换：acc-coach FrameBus/IPC + module_local_dashboard 前端 |
| 6 | [stage-6-remote-dashboard-id-protocol.md](./stage-6-remote-dashboard-id-protocol.md) | remote 路径改用 CompactPatch + DynamicControlInfo field_refs 改 ID |
| 7 | [stage-7-serial-and-mlt-micro-optimization.md](./stage-7-serial-and-mlt-micro-optimization.md) | serial 路径 + module_live_telemetry 微优化 |

---

## 6. 背景参照文档

开发前建议先阅读以下文档了解现有架构：

| 文档 | 作用 |
|---|---|
| `docs/acc-coach/dashboard-architecture.md` | Dashboard 架构总览，模块边界与数据流 |
| `docs/acc-coach/live-telemetry-api-boundary.md` | module_live_telemetry API 边界 |
| `docs/acc-coach/public-protocol/dashboard-frame-distribution-protocol.md` | 帧分发协议（当前版本） |
| `docs/acc-coach/prd/local-dashboard-self-managed-data.md` | module_local_dashboard 数据自管理 PRD |
| `docs/acc-coach/2026-06-16-dashboard-display-performance-plan.md` | 早期性能优化计划（部分已完成） |

---

## 7. 关键代码位置索引

### module_live_telemetry

| 符号 | 位置 | 说明 |
|---|---|---|
| `RecordingController::start` | `src/recording/controller.rs:95` | Legacy 通道入口 |
| `RecordingController::start_with_latest_dashboard` | `src/recording/controller.rs:115` | LatestValueSink 入口（阶段1使用） |
| `DashboardOutput::Legacy/Latest` | `src/recording/controller.rs:77` | 两种 sink 模式 |
| `LatestValueSender/Receiver` | `src/dashboard/sink.rs:86/163` | 最新值通道（覆写语义） |
| `latest_value_channel()` | `src/dashboard/sink.rs:285` | 创建最新值通道 |
| `LatestValueReceiver::notification_receiver` | `src/dashboard/sink.rs:237` | 返回 crossbeam `Receiver<()>`，阶段1用于 Select |
| `DashboardService::run` | `src/dashboard/service.rs:454` | Dashboard 计算线程主循环 |
| `DashboardService::process_frame` | `src/dashboard/service.rs:527` | 按桶调度计算 + 发送 |
| `DashboardFieldRegistry` | `src/recording/dashboard.rs:207` | 字段 ID 注册表（阶段4复用） |
| `DashboardCompactPatch` | `src/recording/dashboard.rs:114` | 紧凑帧编码（阶段6使用） |
| `run_recording_loop` | `src/recording/engine.rs:109` | 录制循环（采样+分发+写文件） |
| `TelemetryDistributor::distribute` | `src/distributor.rs:61` | 帧分发（try_send 到各 consumer） |

### acc-coach

| 符号 | 位置 | 说明 |
|---|---|---|
| `auto_recording_loop` | `src/recording/auto.rs:364` | 后端主循环（阶段1核心改动点） |
| `push_merged_to_bus` | `src/recording/auto.rs:1070` | alias 翻译 + bus push（阶段1优化，阶段5改为ID编码） |
| `merge_latest_dashboard_frame` | `src/recording/auto.rs:877` | 稀疏帧合并到缓存 |
| `poll_dashboard_frame` | `src/ipc/mod.rs:1454` | IPC 命令（阶段5改为ID-based） |
| `get_live_dashboard_frame` | `src/ipc/mod.rs:1434` | IPC 命令（旧版，保留或废弃） |
| `translate_dashboard_frame_values` | `src/ipc/mod.rs:1416` | alias 翻译（阶段1移到此处） |
| `ChannelAliasTable` | `src/dashboard/alias.rs:11` | 别名表（阶段4后逐步退役） |
| `RemoteDashboardRuntime::drain_telemetry` | `src/dashboard/remote/runtime.rs:85` | remote 帧编码（阶段6改动点） |
| `DynamicControlInfo` | `src/dashboard/remote/protocol.rs:227` | remote layout 下发类型（阶段6改 field_refs） |
| `Database::open/migrate` | `src/db/mod.rs:15/34` | SQLite 数据库（阶段4新增 registry 表） |

### module_local_dashboard

| 符号 | 位置 | 说明 |
|---|---|---|
| `DashboardFrameBus` | `src/local_dashboard_overlay/frame_bus.rs:5` | 帧缓存（阶段5改为ID-based） |
| `useDashboardFrame` | `src-ui/features/local-dashboard-overlay/useDashboardFrame.ts:10` | 前端帧轮询（阶段2改为event，阶段3改为store） |
| `LocalDashboardOverlay` | `src-ui/features/local-dashboard-overlay/LocalDashboardOverlay.tsx:17` | overlay 主组件 |
| `DashboardRegionRenderer` | `src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx:99` | 区域渲染 |
| `DynamicDashboardControl` | `src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx:186` | 控件渲染（已 memo） |
| `resolveControlText` | `src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx:336` | 文本模板解析 |
| `controlDependencies` | `src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx:285` | 控件依赖字段提取 |
| `evaluateTextExpression` | `src-ui/features/local-dashboard-overlay/textExpression.ts:314` | 表达式求值 |
| `OverlayPollingConfig::default` | `src/local_dashboard_overlay/config.rs:78` | 默认 frameMs=33（阶段2调整） |

### module_dashboard_protocol

| 符号 | 位置 | 说明 |
|---|---|---|
| `DashboardControl` | `src/lib.rs:82` | 控件定义（存储格式，不改） |
| `DashboardValuesFrame` | `src/lib.rs:348` | 数据帧（阶段4新增 V2 并行类型） |
| `DashboardLayoutPayload` | `src/lib.rs:284` | 布局定义（存储格式，不改） |
