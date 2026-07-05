# 阶段 5：Local Dashboard ID 化硬切换

日期：2026-06-29
涉及模块：`acc-coach`（FrameBus/IPC/编译下发）+ `module_local_dashboard`（前端 ID 渲染）
依赖：阶段4（Registry + 编译器 + compiled 类型就绪）

---

## 1. 目标

将 local dashboard 数据链路从 string key 硬切换为 numeric field ID。alias 翻译从每帧 120Hz 消除，IPC payload 减小，前端查找更快。

**预期效果**：
- alias 翻译从 120Hz 降到 layout 加载时一次（编译时）
- IPC payload 从 `HashMap<String,f64>` → `Vec<(u32,f64)>`
- 前端 `frame.values["speedKmh"]` → `frame.values[42]`（number key）

## 2. 前置条件

- 阶段4已完成：Field Registry（SQLite 持久化）、Layout 编译器、`module_dashboard_protocol` compiled 类型
- 阶段1-3建议已完成（但不强制）：后端优化、event push、前端渲染优化

## 3. 改动方案

### 3.1 acc-coach 侧

#### 3.1.1 DashboardFrameBus 类型切换

**改动文件**：`module_local_dashboard/src/local_dashboard_overlay/frame_bus.rs`

> 注意：`DashboardFrameBus` 由 `module_local_dashboard` 提供，但类型定义来自 `module_dashboard_protocol`。此改动需要 module_local_dashboard 更新依赖类型。

```rust
// 从
use module_dashboard_protocol::DashboardValuesFrame;
pub struct DashboardFrameBus {
    inner: Mutex<Option<DashboardValuesFrame>>,
}

// 改为
use module_dashboard_protocol::DashboardValuesFrameV2;
pub struct DashboardFrameBus {
    inner: Mutex<Option<DashboardValuesFrameV2>>,
}

impl DashboardFrameBus {
    pub fn push_frame(&self, frame: &DashboardValuesFrameV2) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = Some(frame.clone());
        }
    }

    pub fn latest_frame(&self) -> Option<DashboardValuesFrameV2> {
        self.inner.lock().ok()?.clone()
    }
    // clear() 不变
}
```

**注意**：`DashboardValuesFrameV2.values` 是 `Vec<(u32, f64)>` 而非 `HashMap<String, f64>`。`push_frame` 中的 clone 成本：`Vec<(u32,f64)>` clone 比 `HashMap<String,f64>` clone 更轻量（连续内存，无 string 分配）。

#### 3.1.2 auto_recording_loop 帧编码

**改动文件**：`src/recording/auto.rs`

`module_live_telemetry` 仍输出 string key 的 `DashboardValuesFrame`。acc-coach 收到后用 Registry 编码为 `DashboardValuesFrameV2`。

```rust
// drain_dashboard_frames 中
match dashboard_rx.try_recv() {
    Ok(frame) => {
        // 用 Registry 编码 string key → id
        let id_values: Vec<(u32, f64)> = frame.values.iter()
            .filter_map(|(name, value)| {
                // 先 alias 翻译 canonical → user-facing
                let user_name = alias.to_user_facing(name)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| name.clone());
                // 再 registry 查 ID
                Some((registry.id_for(&user_name)?, *value))
            })
            .collect();

        let v2_frame = DashboardValuesFrameV2 {
            sample_tick: frame.sample_tick,
            timestamp_ns: frame.timestamp_ns,
            values: id_values,
        };

        // merge 到 latest cache（V2 合并逻辑）
        merge_latest_dashboard_frame_v2(&latest_dashboard_frame, v2_frame);

        // 远程转发（保持 string key，阶段6改 remote）
        if let Some(ref tx) = telemetry_tx {
            let _ = tx.try_send(AutoDashboardFrame::from(frame));
        }
    }
    // ...
}

// drain 完成后 push 到 bus
push_v2_to_bus(&latest_dashboard_frame, &bus);
```

**Registry 访问**：`auto_recording_loop` 需要 `FieldRegistry` 引用。Registry 在 `AutoRecordingMonitor::start` 时从 DB 加载，传入 `auto_recording_loop`。

**未注册字段处理**：`id_for` 返回 `None` 时跳过该字段（`filter_map`）。但这意味着该字段不会出现在 V2 frame 中。需要确保所有订阅字段在 layout 编译时已注册到 Registry。

**安全网**：在 `replace_dashboard_items` 时，acc-coach 应同步将新订阅的字段注册到 Registry。这样保证 Registry 覆盖所有当前订阅的字段。

#### 3.1.3 IPC 命令切换

**改动文件**：`src/ipc/mod.rs`

`poll_dashboard_frame` 返回类型从 `LiveDashboardFrame`（HashMap）改为 `DashboardValuesFrameV2`（Vec）：

```rust
#[tauri::command]
async fn poll_dashboard_frame(
    bus: tauri::State<'_, Arc<DashboardFrameBus>>,
    replay: tauri::State<'_, ReplayStateType>,
) -> IpcResult<Option<DashboardValuesFrameV2>> {
    // replay 路径也需返回 V2（replay 路径的 ID 化见下方说明）
    if let Some(frame) = replay.latest_dashboard_frame() {
        // replay 路径：从 LatestValueReceiver 读取，编码为 V2
        // ... encoding logic ...
        return Ok(Some(v2_frame));
    }
    Ok(bus.latest_frame())  // live 路径：bus 已存 V2
}
```

**alias 翻译消除**：`poll_dashboard_frame` 不再需要 `alias: tauri::State<'_, ChannelAliasTable>` 参数，因为 alias 翻译已在编码阶段（auto_recording_loop）完成。

**`get_live_dashboard_frame` IPC**：同样改为返回 V2，或废弃（前端统一用 `poll_dashboard_frame`）。

#### 3.1.4 Layout 编译下发

**改动文件**：`src/ipc/mod.rs`

`list_registered_dashboard_layouts` 返回 compiled layout：

```rust
#[tauri::command]
async fn list_registered_dashboard_layouts(
    db: tauri::State<'_, DatabaseState>,
    registry: tauri::State<'_, FieldRegistryState>,
    alias: tauri::State<'_, ChannelAliasTable>,
) -> IpcResult<Vec<CompiledDashboardLayout>> {
    let layouts = load_layouts_from_storage(&db)?;
    let mut registry = registry.lock();
    let compiled = layouts.iter()
        .map(|layout| compile_layout(layout, &mut registry, db.conn(), &alias))
        .collect();
    Ok(compiled)
}
```

**新增 IPC** 或修改现有 IPC，使 module_local_dashboard 调用时获得 compiled layout。

#### 3.1.5 Event push payload 切换

**改动文件**：`src/recording/auto.rs`

如果阶段2已实现 event push，`emit_to("dashboard://frame", &payload)` 的 payload 改为 `DashboardValuesFrameV2`。

#### 3.1.6 Replay 路径 ID 化

Replay 路径也通过 `LatestValueReceiver` 输出 string key frame。需要同样的编码逻辑。建议抽取公共编码函数：

```rust
fn encode_to_v2(
    frame: &DashboardValuesFrame,
    registry: &FieldRegistry,
    alias: &ChannelAliasTable,
) -> DashboardValuesFrameV2 {
    let values = frame.values.iter()
        .filter_map(|(name, value)| {
            let user_name = alias.to_user_facing(name)
                .map(|s| s.to_string())
                .unwrap_or_else(|| name.clone());
            Some((registry.id_for(&user_name)?, *value))
        })
        .collect();
    DashboardValuesFrameV2 {
        sample_tick: frame.sample_tick,
        timestamp_ns: frame.timestamp_ns,
        values,
    }
}
```

### 3.2 module_local_dashboard 侧

#### 3.2.1 前端帧类型切换

**改动文件**：`src-ui/features/local-dashboard-overlay/types.ts`、`useDashboardFrame.ts`

```typescript
// types.ts
export interface DashboardValuesFrameV2 {
    sampleTick: number;
    timestampNs: number;
    values: [number, number][];  // Vec<(u32, f64)> → tuple array
}
```

`useDashboardFrame` 中稀疏帧合并改为 ID-based：

```typescript
// 从
fullFrameRef.current = { ...fullFrameRef.current, ...frame.values };

// 改为
const merged = new Map(fullFrameRef.current);
for (const [id, value] of frame.values) {
    merged.set(id, value);
}
fullFrameRef.current = merged;
```

`fullFrame` state 类型从 `Record<string, number>` → `Map<number, number>`。

#### 3.2.2 historyBuffer key 切换

**改动文件**：`useDashboardFrame.ts`

```typescript
// 从
const historyRef = useRef<Map<string, BufferEntry[]>>(new Map());

// 改为
const historyRef = useRef<Map<number, BufferEntry[]>>(new Map());
```

`rebuildBuffers` 中 `field.telemetryField`（string）→ `field.fieldId`（number）。

#### 3.2.3 dashboardRenderer ID 查找

**改动文件**：`src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx`

```typescript
// 从
const value = frame.values[rule.telemetryField];

// 改为
const value = frame.values.get(rule.fieldId);
```

`controlDependencies` 返回 `number[]` 替代 `string[]`。

`resolveControlText` 中 `{value}` 绑定到 `control.telemetryFieldId`，`{42}` 直接用 number key 查 `frame.values.get(42)`。

#### 3.2.4 textExpression ID 支持

**改动文件**：`src-ui/features/local-dashboard-overlay/textExpression.ts`

`evaluateTextExpression` 的占位符替换逻辑改为支持 number key：

```typescript
// 从
const value = frame[field];

// 改为
const value = frame.get(Number(field));  // field 是 ID 字符串 "42"
```

**注意**：编译后的模板中 `{42}` 的 `42` 是数字字符串，`Number("42")` → `42`，然后 `Map.get(42)` 查找。`{value}` 保持特殊处理：绑定到 `control.telemetryFieldId`。

#### 3.2.5 Layout 加载切换

**改动文件**：`src-ui/features/local-dashboard-overlay/useDashboardMetadata.ts`

`list_registered_dashboard_layouts` 返回 `CompiledDashboardLayout[]`，前端用 compiled 类型。

## 4. 模块间开发顺序

本阶段是**硬切换**，两边必须同步完成才能联调。开发顺序：

```
1. module_dashboard_protocol compiled 类型（阶段4已完成）
                    ↓
2. acc-coach (D3) 和 module_local_dashboard (D5) 并行开发
   - acc-coach: FrameBus V2 + IPC V2 + 编译下发 + 帧编码
   - module_local_dashboard: 前端 V2 类型 + ID 查找 + 模板 ID 支持
   两边按 module_dashboard_protocol 的类型契约独立实现。
                    ↓
3. 联调测试
   - acc-coach 和 module_local_dashboard 都改完后才能完整测试
   - 测试前功能 break（IPC 类型不匹配），这是硬切换的固有代价
```

### 降低联调风险的建议

虽然硬切换，但可以分步切换降低风险：

1. **acc-coach 先内部改 FrameBus 为 V2，IPC 层做 V2→V1 转换**（临时转换层，module_local_dashboard 不改，功能正常）
2. **确认 V2 FrameBus 工作正常后，IPC 切换为 V2 + module_local_dashboard 切换为 V2**（两边同步，联调）
3. **删除临时转换层**

这样步骤1可独立测试，步骤2是真正的硬切换点（但范围已缩小到 IPC 层）。

## 5. 验收标准

| 验收项 | 验证方法 |
|---|---|
| overlay 正常显示所有控件 | 启动 ACC + recording，观察 speed/rpm/gear/lap-time 等 |
| 无 "--" 闪烁 | 不同刷新率字段不闪 |
| Chart 正常 | ChartWidget 从 ID-based buffer 渲染 |
| Map 正常 | MapWidget 用 ID 查找位置字段 |
| 条件规则正常 | 条件变色/显隐按 ID 字段值触发 |
| 文本模板正常 | `{value}` 和 `{id}` 占位符正确渲染 |
| 表达式正常 | `{{expr:round({id}, 2)}}` 正确求值 |
| 布局切换 | 切换 layout 后正确显示，无 stale value |
| Replay 正常 | 回放时 overlay 显示回放数据 |
| Remote 不受影响 | remote 路径仍用 string key（阶段6改） |
| alias 翻译消除 | `ACC_DASHBOARD_LOG=1` 观察录制线程无 alias 翻译日志 |
| ID 跨会话稳定 | 重启 acc-coach 后 ID 不变 |

## 6. 风险

| 风险 | 缓解 |
|---|---|
| 硬切换期间功能 break | 分步切换：先内部 V2 + IPC 转换层，再同步切 IPC |
| 未注册字段丢失 | `replace_dashboard_items` 时同步注册到 Registry；编码时 `filter_map` 跳过未注册字段需告警 |
| Registry 并发访问 | `auto_recording_loop` 线程与 IPC 线程共享 Registry → `Arc<Mutex<FieldRegistry>>` |
| replay 路径遗漏 | replay 也用 `LatestValueReceiver` + string key，需同样编码为 V2 |
| 前端 Map<number, number> 性能 | number key 比 string key 更快（hash 整数 vs hash 字符串） |
| `get_live_dashboard_frame` 旧 IPC | 评估是否废弃或同步改 V2 |

## 7. 参照文档

- [stage-4-field-id-registry-and-compiler.md](./stage-4-field-id-registry-and-compiler.md) — Registry 和编译器实现
- `docs/acc-coach/public-protocol/dashboard-frame-distribution-protocol.md` — 帧分发协议（本阶段更新为 V2）
- `docs/acc-coach/prd/local-dashboard-self-managed-data.md` — module_local_dashboard 数据自管理 PRD
- README.md 关键代码位置索引 — `DashboardFrameBus` / `poll_dashboard_frame` / `useDashboardFrame` 条目
