# 阶段 6：Remote Dashboard ID 化协议

日期：2026-06-29
涉及模块：`acc-coach`（remote 路径编码）
依赖：阶段4（Registry + 编译器）

---

## 1. 目标

将 network remote dashboard 的数据传输从 JSON string key 改为 binary CompactPatch（numeric field ID），降低 UDP 带宽 40~55%，消除设备端 JSON parse 开销。

**预期效果**：
- UDP payload 从 ~300-500 bytes JSON → ~150-250 bytes binary
- 设备端无需 JSON parse，直接 binary decode
- layout 下发从 `DynamicControlInfo.field_refs: Vec<String>` → `Vec<u32>`

## 2. 当前问题

### 2.1 JSON 编码 string key

`src/dashboard/remote/runtime.rs:93-97`：

```rust
let telemetry = TelemetryFrame {
    sample_tick: frame.sample_tick as u32,
    timestamp_ms: frame.timestamp_ns / 1_000_000,
    fields_json: serde_json::to_string(&frame.values).unwrap_or_default(),
};
```

`fields_json` 是 `HashMap<String,f64>` 序列化的 JSON 字符串，如 `{"speedKmh":243.1,"gear":5,...}`。

### 2.2 DynamicControlInfo field_refs string

`src/dashboard/remote/protocol.rs:227-235`：

```rust
pub struct DynamicControlInfo {
    pub control_id: String,
    pub field_refs: Vec<String>,  // string key
    pub text_template: Option<String>,
    pub text_format: Option<String>,
    pub refresh_hz: f64,
}
```

## 3. 改动方案

### 3.1 数据帧改用 CompactPatch

**改动文件**：`src/dashboard/remote/data.rs`、`src/dashboard/remote/runtime.rs`

#### 3.1.1 TelemetryFrame 改为 binary payload

`module_live_telemetry` 已提供 `DashboardCompactPatch`（`src/recording/dashboard.rs:114`）：

```rust
pub struct DashboardCompactPatch {
    pub subscription_generation: u64,
    pub sample_tick: u64,
    pub timestamp_ns: u64,
    pub values: Vec<(DashboardFieldId, f64)>,
}

impl DashboardCompactPatch {
    pub fn to_bytes(&self) -> Result<Vec<u8>, DashboardCompactPatchError>;
    pub fn from_bytes(input: &[u8]) -> Result<Self, DashboardCompactPatchError>;
}
```

Wire format：4B magic + 8B gen + 8B tick + 8B ts + 4B count + N×12B (4B id + 8B f64)。

**改动**：

```rust
// data.rs
pub struct TelemetryFrame {
    pub sample_tick: u32,
    pub timestamp_ms: u64,
    /// Binary-encoded DashboardCompactPatch bytes (replaces fields_json)
    pub payload: Vec<u8>,
}
```

`runtime.rs` 编码：

```rust
// runtime.rs drain_telemetry
if let Some(frame) = latest {
    // 用 Registry 编码为 CompactPatch
    let patch = DashboardCompactPatch {
        subscription_generation: 0,
        sample_tick: frame.sample_tick,
        timestamp_ns: frame.timestamp_ns,
        values: frame.values.iter()
            .filter_map(|(name, value)| {
                let user_name = alias.to_user_facing(name)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| name.clone());
                Some((registry.id_for(&user_name)?, *value))
            })
            .collect(),
    };
    let payload = patch.to_bytes().unwrap_or_default();

    let telemetry = TelemetryFrame {
        sample_tick: frame.sample_tick as u32,
        timestamp_ms: frame.timestamp_ns / 1_000_000,
        payload,
    };
    let (sent, errors) = self.session_registry.broadcast_telemetry(&telemetry);
}
```

#### 3.1.2 UDP 数据包 payload_type

`src/dashboard/remote/protocol.rs:48`：

```rust
pub enum PayloadType {
    LayoutValues = 0x04,  // TODO: values payload type uses DashboardValuesFrame
}
```

新增 payload type：

```rust
pub enum PayloadType {
    TelemetryCompactPatch = 0x05,  // binary DashboardCompactPatch
    LayoutValues = 0x04,
}
```

UDP 数据包 header 中 `payload_type` 标记为 `0x05`，设备端按 type 解码。

### 3.2 Layout 下发改用 compiled control

**改动文件**：`src/dashboard/remote/protocol.rs`

`DynamicControlInfo` 改为 ID-based：

```rust
pub struct DynamicControlInfo {
    pub control_id: String,
    pub field_refs: Vec<u32>,           // String → u32 (field ID)
    pub text_template: Option<String>,   // compiled template ({id} 占位符)
    pub text_format: Option<String>,
    pub refresh_hz: f64,
    pub conditional_rules: Vec<CompiledConditionalRuleInfo>,  // 新增
}

pub struct CompiledConditionalRuleInfo {
    pub target: String,
    pub field_id: u32,
    pub operator: String,
    pub compare_value: f64,
    pub color: String,
}
```

**注意**：`PrepareLayoutMessage` 的 wire format（JSON）会变化（`fieldRefs` 从 string array → number array）。由于 remote 设备端未开发，无遗留兼容问题。

#### 3.2.1 layout 编译时机

remote 设备连接时，acc-coach 用阶段4的编译器编译 layout：

```rust
// runtime.rs connect_device 流程中
let compiled = compile_layout(&registered_layout, &mut registry, conn, &alias);
let dynamic_controls: Vec<DynamicControlInfo> = compiled.controls.iter()
    .filter(|c| c.widget_type != "static")
    .map(|c| DynamicControlInfo {
        control_id: c.id.clone(),
        field_refs: collect_field_refs(c),  // [telemetry_field_id, chart_fields[].field_id, conditional_rules[].field_id]
        text_template: Some(c.compiled_text_template.clone()),
        text_format: c.format.clone(),
        refresh_hz: c.refresh_hz,
        conditional_rules: c.conditional_rules.iter().map(|r| CompiledConditionalRuleInfo {
            target: r.target.clone(),
            field_id: r.field_id,
            operator: r.operator.clone(),
            compare_value: r.compare_value,
            color: r.color.clone(),
        }).collect(),
    })
    .collect();

let prepare_msg = PrepareLayoutMessage {
    dynamic_controls,
    // ...
};
```

### 3.3 Keyframe 机制

当前 `SessionSender` 有 keyframe 机制（`src/dashboard/remote/data.rs:44`，每 1s 发一次完整快照）。ID 化后 keyframe 仍需发送完整快照（所有字段的最新值），但格式改为 CompactPatch。

设备端检测到 sequence gap 时等待下一次 keyframe 补齐（与现有语义一致）。

### 3.4 设备端协议文档更新

**更新文件**：`docs/acc-coach/public-protocol/protocol-spec.md`

需要更新 wire format 文档，说明：
- `payload_type 0x05` = CompactPatch binary
- `DynamicControlInfo.fieldRefs` 改为 `number[]`（field ID）
- `DynamicControlInfo.conditionalRules` 新增
- `textTemplate` 中的占位符从 `{fieldName}` → `{fieldId}`

## 4. 模块间开发顺序

本阶段仅涉及 `acc-coach`（remote 路径）。remote 设备端是独立项目，需按更新后的 `protocol-spec.md` 实现。

| 模块 | 改动 | 依赖 |
|---|---|---|
| acc-coach | remote 数据帧 + layout 下发改 ID | 阶段4 Registry + 编译器 |
| remote 设备端 | 按 protocol-spec.md v2 实现 | acc-coach 改完后的协议 |

acc-coach 改动可独立测试（用 mock 设备或单元测试验证 binary 编码）。

## 5. 验收标准

| 验收项 | 验证方法 |
|---|---|
| binary 编码正确 | 单元测试：构造 frame → encode → 验证 bytes → decode → 比较 |
| payload size 减小 | 对比 JSON vs binary 的 byte length |
| layout 下发正确 | 单元测试：编译后 DynamicControlInfo field_refs 为 number[] |
| keyframe 正常 | 模拟 sequence gap → keyframe 补齐 |
| local dashboard 不受影响 | local 路径在阶段5已改 ID，remote 改动独立 |
| protocol-spec.md 更新 | 文档反映新 wire format |

## 6. 风险

| 风险 | 缓解 |
|---|---|
| remote 设备端未开发，无法端到端测试 | acc-coach 侧用单元测试验证编码；设备端开发时按文档实现 |
| CompactPatch 字节序 | `to_bytes` 已用 little-endian，设备端按 LE 解码 |
| 字段 ID 未注册 | 编码时 `filter_map` 跳过；应在 layout 编译时确保所有字段已注册 |
| keyframe 频率 | 保持 1s 间隔不变；CompactPatch keyframe 发送所有字段最新值 |

## 7. 参照文档

- [stage-4-field-id-registry-and-compiler.md](./stage-4-field-id-registry-and-compiler.md) — Registry + 编译器
- `docs/acc-coach/public-protocol/protocol-spec.md` — network remote wire 协议（本阶段更新）
- `docs/acc-coach/dashboard-architecture.md` — 4.2 节 Network Remote Dashboard 路径
- README.md 关键代码位置索引 — `DashboardCompactPatch` / `RemoteDashboardRuntime` 条目
