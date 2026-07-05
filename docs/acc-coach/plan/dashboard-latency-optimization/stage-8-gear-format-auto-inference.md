# 阶段 8：Gear format 编译器自动推断

日期：2026-07-02
涉及模块：`acc-coach`（编译器 + 内置 layout + 设计器）
依赖：阶段 4（Field Registry + Layout 编译器 + compiled 类型）、阶段 5（Local Dashboard ID 化硬切换）
性质：阶段 5 的遗漏修复 + format 透明化

---

## 1. 目标

让档位（gear）数据在 local dashboard overlay 上自动正确显示为 `R/N/1/2…`（而非 ACC shared memory 的原始数值 `0/1/2…`），且**用户全程无感**：

- 用户不需要在设计器里手动为 gear 控件填写 `format: "gear"`
- 用户不需要知道 `format` 字段的存在
- 所有 layout（内置默认、数据库已存、设计器新建）均自动生效，无需数据库 migration

**预期效果**：
- 内置默认 layout 的 gear 控件编译后 `format = Some("gear")`、`telemetry_field_id = <gear 的 ID>`
- 用户在设计器创建 gear 控件（即使 `telemetry_field` 与 `format` 均留空，只要 `text_template` 含 `{gear}`）→ 编译后自动获得 `format = Some("gear")`
- 子模块 `module_local_dashboard` 的 `formatGear`（raw→R/N/1/2）与 `smoothGear`（换挡防抖）正常触发

---

## 2. 背景与问题根因

### 2.1 阶段 5 改造前（V1，string key 时代）

数据链路：
```
ACC shared memory
  → module_live_telemetry 输出 DashboardValuesFrame (HashMap<String,f64>)，key 为 canonical name "raw:controls.gear"
  → acc-coach 透传 string key 帧给 local_dashboard
  → 子模块 frame.values["gear"]  // key 是 "gear"
```

gear 识别机制（V1）：子模块通过 **string key** `"gear"` 识别档位字段。无论依据是 `control.telemetryField === "gear"`，还是 `text_template` 占位符名 `{gear}`，本质都依赖字符串 `"gear"`。识别后调用 `formatGear` 做 raw→display 映射。此阶段 `control.format` 无需设为 `"gear"`，key 本身即识别依据。

### 2.2 阶段 5 改造后（V2，numeric field ID）

阶段 5 将 string key 硬切换为 numeric field ID：

- 数据帧：`DashboardValuesFrame` (HashMap<String,f64>) → `DashboardValuesFrameV2` (Vec<(u32,f64)>)
- layout：`DashboardControl` (string key) → `CompiledDashboardControl` (id-based)，`text_template` 中 `{gear}` 编译为 `{<gear_id>}`（如 `{44}`）
- 子模块全面改用 ID 查找：`frame.values.get(fieldId)`（`Map<number,number>`），不再依赖 key 字符串

**子模块的 gear 识别机制随之改变**：V2 后占位符名变成数字 ID，子模块无法再通过字符串 `"gear"` 识别。子模块改为通过 **`control.format === "gear"`** 识别档位字段：

| 子模块文件 | 行 | V2 识别逻辑 |
|---|---|---|
| `dashboardRenderer.tsx` | 375 | `const isGearField = control.format === "gear";` → 触发 `formatGear` |
| `dashboardRenderer.tsx` | 376-378 | `isGearField && gearState` → `smoothGear` 防抖 |
| `LocalDashboardOverlay.tsx` | 25-29 | `control.format === "gear" && control.telemetryFieldId != null` → 取 `gearFieldId` |
| `useDashboardFrame.ts` | 88-91 | `gearFieldId` 从 `fullFrameValues.get(gearFieldId)` 取值喂给 `smoothGear` |
| `telemetryFormat.ts` | 89-93 | `formatGear(rawGear)`：`<=0→R`、`==1→N`、`else→rawGear-1` |

`formatGear` 的触发路径（`dashboardRenderer.tsx:381-383`）：
```typescript
if (isGearField) {
  return formatGear(Number(value));
}
return formatTelemetryValue(value, explicitFormat ?? control.format);
```

### 2.3 遗漏点（本阶段要修的根因）

阶段 5 改造了子模块的识别机制（key → format），但 **acc-coach 侧的 layout 定义没有同步给 gear 控件标识 `format: "gear"`**：

**内置默认 layout**（`src/dashboard/layout.rs:252-281`）的 gear 控件：
```rust
DashboardControl {
    id: "gear".to_string(),
    // ...
    text_template: "{gear}".to_string(),
    telemetry_field: None,   // ← 遗漏：导致编译后 telemetryFieldId = None
    format: None,            // ← 遗漏：导致子模块 isGearField = false
    // ...
}
```

**设计器**（`src-ui/components/DashboardDesignerView.tsx`）：
- 创建控件默认 `format: null`（:194）、`telemetryField: null`（:193）
- `format` 是一个自由文本输入框（:3061-3070），用户需手动输入 `"gear"` 才能触发档位映射

**后果**：
- 编译后的 `CompiledDashboardControl.format` 为 `None` → 子模块 `isGearField = false` → 不调用 `formatGear` → gear 显示原始数值（`0/1/2` 而非 `R/N/1`）
- `telemetry_field: None` → `telemetry_field_id = None` → 子模块 `smoothGear` 防抖不工作（`LocalDashboardOverlay.tsx:28` 要求 `telemetryFieldId != null`）

### 2.4 关键澄清

用户原始描述"实时数据中不再发送字段 key 导致子模块处理不了 gear"的归因不完全准确：

- **数据帧 V2 不带 key 是设计如此**（阶段 5 核心目标），子模块已用 `Map<number,number>` 适配，**不是问题源**
- **真正问题在 layout 层**：gear 控件缺 `format: "gear"` 标识，而子模块 V2 恰好靠 `control.format === "gear"` 触发映射
- 修复点在 acc-coach 的 layout/编译器/设计器，**不涉及禁改子模块**（`module_local_dashboard`、`module_live_telemetry`、`acctlm_core`、`ld_to_acctlm`）

---

## 3. 设计原理

### 3.1 为什么在编译器推断（而非其他位置）

| 候选位置 | 评估 | 选择 |
|---|---|---|
| **编译器 `compile_control`** | 所有下发路径（内置 layout、数据库 layout、设计器保存的 layout）都经 `compile_layout` → 一处生效；无需数据库 migration；用户完全无感 | ✅ 选 |
| 内置 `layout.rs` 直接写 `format: Some("gear")` | 只覆盖内置默认 layout；用户自定义/数据库已存 layout 不受益 | ❌ |
| 设计器创建控件时自动设 format | 只覆盖设计器新建的控件；内置/数据库已存不受益；且用户编辑 text_template 时可能绕过 | ❌ |
| acc-coach 编码 V2 帧时映射 gear 值 | display 值是字符串（R/N/1/2），数据帧是 f64，无法承载字符串；且会与子模块 `formatGear` 二次映射冲突 | ❌ 不可行 |

### 3.2 为什么"尊重显式设置，仅 None 时推断"

`format` 字段有两类用途：

| 类型 | 示例 | 能否自动推断 |
|---|---|---|
| **语义型** | `"gear"`、`"lapTime"`、`"delta"`、`"percent"`、`"integer"`、`"number"` | 可（由字段名推断） |
| **数值格式型** | `"0.0"`（补零）、`"ss.fff"`（时间串）等任意字符串 | 不可（是用户表达显示精度的方式） |

本阶段只推断 **gear**（用户已确认）。为兼容未来可能的显式数值格式型 format，推断逻辑采用 `control.format.clone().or_else(|| infer_format(...))`：

- `control.format` 显式设了（如用户/存量 layout 写了 `"integer"`）→ 尊重，不推断
- `control.format` 为 `None` → 用推断结果

数值格式型 format 的用户可见入口在本阶段后改由 `text_template` 的 `{field|format}` 语法承载（子模块 `dashboardRenderer.tsx:385` 已支持 `explicitFormat ?? control.format`，`explicitFormat` 即来自模板 `{field|0.0}` 的 `0.0`）。

### 3.3 推断依据：telemetry_field 优先 + text_template 兜底

- **优先 `control.telemetry_field`**：若控件显式绑定了字段（如 `Some("gear")`），经 alias 解析为 user-facing name 后匹配规则。这是最可靠的依据。
- **兜底 `control.text_template` 占位符名**：内置默认 layout 的 gear 控件 `telemetry_field: None` 但 `text_template: "{gear}"`（`layout.rs:265`）。解析占位符名 `"gear"` 作为推断依据。本阶段改动 2 会给内置 layout 补 `telemetry_field`，但兜底逻辑仍保留，以覆盖"用户只在模板里写 `{gear}` 未绑定字段"的场景。

### 3.4 为什么同时补 `telemetry_field`（不只是 format）

光有 `format: "gear"` 不够。子模块 `smoothGear`（换挡 N 挡闪烁防抖）依赖 `control.telemetryFieldId != null`（`LocalDashboardOverlay.tsx:28`、`useDashboardFrame.ts:88-91`）。`telemetryFieldId` 由编译器从 `control.telemetry_field` 编译而来（`compiler.rs:43-46`）。若 `telemetry_field: None`，`telemetryFieldId` 为 `None`，`smoothGear` 不工作，gear 换挡时会有 N 挡闪烁。因此内置 layout 的 gear 控件必须补 `telemetry_field: Some("gear")`。

## 4. 改动方案

### 4.1 改动 1：编译器自动推断 gear format

**改动文件**：`src/dashboard/compiler.rs`

#### 4.1.1 新增 `infer_format` 及辅助函数

在 `compile_control` 附近新增三个函数：

```rust
/// 推断控件的语义型 format。
/// 当 control.format 为 None 时由 compile_control 调用。
/// 依据 telemetry_field（优先）与 text_template 占位符名（兜底），
/// 经 alias 归一化为 user-facing name 后匹配规则。
/// 目前只推断 gear。未来可扩展 lapTime/delta 等。
fn infer_format(control: &DashboardControl, alias: &ChannelAliasTable) -> Option<String> {
    // 优先：telemetry_field 显式绑定的字段
    if let Some(field) = &control.telemetry_field {
        if let Some(inferred) = infer_format_for_field(field, alias) {
            return Some(inferred);
        }
    }
    // 兜底：解析 text_template 的所有占位符名
    for field in iter_template_field_names(&control.text_template) {
        if let Some(inferred) = infer_format_for_field(&field, alias) {
            return Some(inferred);
        }
    }
    None
}

/// 判断单个字段名是否对应某种语义型 format。
/// 接受 user-facing name（如 "gear"）或 canonical name（如 "raw:controls.gear"）。
fn infer_format_for_field(field: &str, alias: &ChannelAliasTable) -> Option<String> {
    let user_name = if alias.is_user_format(field) {
        field
    } else {
        alias.to_user_facing(field).unwrap_or(field)
    };
    match user_name {
        "gear" => Some("gear".to_string()),
        // 未来扩展（本阶段不做）：
        // "bestLapDeltaTimeMs" | "sessionLapDeltaTimeMs" => Some("delta".to_string()),
        // "bestLapTimeMs" | "sessionBestLapTimeMs" | "predictedLapTimeByBest"
        //   | "predictedLapTimeBySession" => Some("lapTime".to_string()),
        _ => None,
    }
}

/// 扫描 text_template，提取所有 {field} / {field|format} 占位符的 field 名。
/// 跳过 {value}（运行时绑定）与 {{expr:...}}（表达式块）。
/// 解析逻辑与 compile_text_template（compiler.rs:125-186）的占位符扫描保持一致。
fn iter_template_field_names(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            continue;
        }
        if chars.peek() == Some(&'{') {
            // {{expr:...}} — 跳过整个双花括号块（按深度计数，支持嵌套）
            chars.next();
            let mut depth = 1usize;
            while let Some(c) = chars.next() {
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            continue;
        }
        // {field} 或 {field|format}
        let mut token = String::new();
        while let Some(tc) = chars.next() {
            if tc == '}' {
                break;
            }
            token.push(tc);
        }
        let field = token.split('|').next().unwrap_or("").trim();
        if !field.is_empty() && field != "value" {
            names.push(field.to_string());
        }
    }
    names
}
```

> 注：`iter_template_field_names` 的扫描逻辑刻意与 `compile_text_template`（compiler.rs:155-178）的单花括号分支一致，确保推断依据的字段名与编译时注册 ID 的字段名同源。双花括号 `{{expr:...}}` 的跳过逻辑参考 `compile_text_template:136-154`，并改为按深度计数以正确处理嵌套（原实现假设非嵌套，这里更稳健）。

#### 4.1.2 接入 `compile_control`

**改动位置**：`src/dashboard/compiler.rs:83`

```rust
// 原来（compiler.rs:83）
format: control.format.clone(),

// 改为
format: control.format.clone().or_else(|| infer_format(control, alias)),
```

`infer_format` 只读 `control.telemetry_field` / `control.text_template` / `alias`，不触碰 `registry` / `conn`，无副作用，可在 `compile_control` 末尾构造结构体时直接内联调用。

#### 4.1.3 单元测试

在 `compiler.rs` 的 `#[cfg(test)] mod tests` 中新增（测试 helper `setup_db`、`setup_registry`、`test_alias` 复用 compiler.rs:220+ 现有测试基础设施）：

```rust
#[test]
fn infer_format_from_telemetry_field_for_gear() {
    let conn = setup_db();
    let mut registry = setup_registry(&conn);
    let alias = test_alias();
    let _ = registry.register("gear", &conn);

    let control = DashboardControl {
        id: "gear".into(),
        widget_type: WidgetType::Text,
        telemetry_field: Some("gear".into()),
        text_template: "{gear}".into(),
        format: None,
        refresh_hz: 30.0,
        x: 0.0, y: 0.0, width: 100.0, height: 100.0,
        visible: true,
        visible_expression: None,
        font_family: "Arial".into(),
        font_size: 24.0,
        text_color: "#fff".into(),
        text_color_expression: None,
        text_fallback: None,
        background_color: "#000".into(),
        background_color_expression: None,
        background_image_data_url: None,
        horizontal_center: false,
        vertical_center: false,
        chart_fields: vec![],
        chart_sample_count: None,
        track_id: None,
        dot_color: None,
        dot_size: None,
        conditional_rules: vec![],
    };

    let compiled = compile_control(&control, &mut registry, &conn, &alias);
    assert_eq!(compiled.format, Some("gear".to_string()));
}

#[test]
fn infer_format_from_template_when_telemetry_field_none() {
    // 内置 layout 改动前的场景：telemetry_field 为 None，但模板含 {gear}
    let conn = setup_db();
    let mut registry = setup_registry(&conn);
    let alias = test_alias();

    let mut control = test_control_with_telemetry_field(None);
    control.text_template = "{gear}".into();
    control.format = None;

    let compiled = compile_control(&control, &mut registry, &conn, &alias);
    assert_eq!(compiled.format, Some("gear".to_string()));
    // telemetry_field 未绑定，telemetry_field_id 仍为 None（smoothGear 不工作，
    // 这正是改动 2 要给内置 layout 补 telemetry_field 的原因）
    assert_eq!(compiled.telemetry_field_id, None);
}

#[test]
fn infer_format_respects_explicit_format() {
    // control.format 显式设置时不被推断覆盖
    let conn = setup_db();
    let mut registry = setup_registry(&conn);
    let alias = test_alias();

    let mut control = test_control_with_telemetry_field(Some("gear".into()));
    control.format = Some("integer".into());

    let compiled = compile_control(&control, &mut registry, &conn, &alias);
    assert_eq!(compiled.format, Some("integer".to_string()));
}

#[test]
fn infer_format_none_for_non_gear_field() {
    let conn = setup_db();
    let mut registry = setup_registry(&conn);
    let alias = test_alias();

    let mut control = test_control_with_telemetry_field(Some("speedKmh".into()));
    control.format = None;

    let compiled = compile_control(&control, &mut registry, &conn, &alias);
    assert_eq!(compiled.format, None);
}

#[test]
fn infer_format_accepts_canonical_gear_key() {
    // canonical name "raw:controls.gear" 经 alias 翻译为 "gear" 也应命中
    let conn = setup_db();
    let mut registry = setup_registry(&conn);
    let alias = test_alias();

    let mut control = test_control_with_telemetry_field(Some("raw:controls.gear".into()));
    control.format = None;

    let compiled = compile_control(&control, &mut registry, &conn, &alias);
    assert_eq!(compiled.format, Some("gear".to_string()));
}
```

> 若 `test_control_with_telemetry_field` helper 不存在，实现为一个返回带默认字段的 `DashboardControl`、仅 `telemetry_field` 参数化的工厂函数，供上述测试复用。

### 4.2 改动 2：内置默认 layout gear 控件补字段绑定

**改动文件**：`src/dashboard/layout.rs:267`

```rust
// 原来（layout.rs:252-281 的 gear 控件）
DashboardControl {
    id: "gear".to_string(),
    x: 1800.0,
    y: 460.0,
    width: 360.0,
    height: 340.0,
    widget_type: WidgetType::Text,
    refresh_hz: 30.0,
    visible: true,
    visible_expression: None,
    background_color: "#00000000".to_string(),
    background_color_expression: None,
    background_image_data_url: None,
    text_template: "{gear}".to_string(),
    text_fallback: None,
    telemetry_field: None,   // ← 改这一行
    format: None,
    // ...
},

// 改为
    telemetry_field: Some("gear".to_string()),
    format: None,  // 仍为 None，由编译器 infer_format 自动推断为 "gear"
```

**为什么 `format` 仍留 `None`**：验证编译器推断路径生效。若 `format` 显式写 `Some("gear")`，则测不到 `infer_format` 的兜底价值。留 `None` 让编译器推断，与用户自定义 layout（`format: null`）走同一条路径，行为一致。

**作用**：
1. 编译后 `telemetry_field_id = Some(<gear 的 ID>)` → 子模块 `smoothGear` 防抖工作（`LocalDashboardOverlay.tsx:28` 条件满足）
2. 作为 `infer_format` 的优先推断依据（`telemetry_field = Some("gear")` → 命中规则）

### 4.3 改动 3：设计器隐藏 format 输入框

**改动文件**：`src-ui/components/DashboardDesignerView.tsx:3061-3070`

#### 4.3.1 移除 "Text format" 输入框

```tsx
// 原来（DashboardDesignerView.tsx:3061-3070）
<label className={styles.field}>
  Text format
  <input
    className={styles.input}
    value={selected.format ?? ""}
    onChange={(event) =>
      updateSelected({ format: event.target.value || null })
    }
  />
</label>

// 改为：直接删除整个 <label> 块（无替换内容）
```

#### 4.3.2 数据结构保留 `format` 字段

**不修改** `DashboardControl.format` 字段定义（`module_dashboard_protocol` 类型）与设计器的 `format: null` 默认值（`DashboardDesignerView.tsx:194`）。理由：

- `control.format` 字段在数据结构中保留，编译器仍读它做兼容（`control.format.clone().or_else(infer)`）
- 存量 layout 若 `format` 非空（如用户之前手动设过 `"integer"`），编译器尊重显式值，行为不变
- 默认 `null` 时由编译器推断，用户无感

#### 4.3.3 数值格式型 format 的用户入口

隐藏 format 输入框后，用户若需指定数值格式（如 `"0.0"` 补零、`"ss.fff"` 时间串），改在 `text_template` 中用 `{field|format}` 语法：

| 需求 | 原来（control.format） | 改后（text_template） |
|---|---|---|
| 速度保留 1 位小数 | `format: "0.0"` | `text_template: "{speedKmh|.1f}"` 或 `{speedKmh|0.0}` |
| 圈时显示 | `format: "lapTime"` | `text_template: "{bestLapTimeMs}"` + 未来扩展推断（本阶段不做） |
| 档位 | `format: "gear"` | 无需用户操作，编译器自动推断 |

子模块 `dashboardRenderer.tsx:385` 的 `formatTelemetryValue(value, explicitFormat ?? control.format)` 已支持模板内 `explicitFormat` 优先，**无需子模块改动**。

#### 4.3.4 存量兼容

已确认（grep `format: Some(` 全仓仅 `compiler.rs:560` 测试代码一处；`layout.rs`/`mod.rs` 全为 `None`）：

- 内置默认 layout：`format` 全 `None` → 编译器推断兜底，无影响
- 设计器默认新建控件：`format: null`（:194）→ 编译器推断兜底
- 数据库已存用户 layout：若用户曾手动在 format 框输入过值，该值仍被尊重（`or_else` 语义）；若为 `null`，编译器推断

**无需数据库 migration**。隐藏输入框后，用户无法再新建/修改 `control.format`，但存量值仍被编译器读取尊重。

## 5. 涉及文件清单

| 文件 | 改动 | 行数估计 | 模块归属 |
|---|---|---|---|
| `src/dashboard/compiler.rs` | 新增 `infer_format` + `infer_format_for_field` + `iter_template_field_names`；改 `compile_control:83` 一行；新增 5 个单测 | +~90 | acc-coach ✅ |
| `src/dashboard/layout.rs` | gear 控件 `telemetry_field: None` → `Some("gear")`（:267 一行） | 1 | acc-coach ✅ |
| `src-ui/components/DashboardDesignerView.tsx` | 删除 "Text format" label+input（:3061-3070） | -10 | acc-coach ✅ |

**不涉及禁改子模块**：`module_local_dashboard`、`module_live_telemetry`、`acctlm_core`、`ld_to_acctlm` 均无改动。`module_dashboard_protocol` 类型也无改动（`format` 字段保留）。

---

## 6. 数据链路验证（改动后预期）

```
ACC shared memory (gear 原始值: 0/1/2/...)
  → module_live_telemetry: DashboardValuesFrame { values: {"raw:controls.gear": 2.0, ...} }
  → acc-coach auto_recording_loop: alias 翻译 "raw:controls.gear"→"gear"，registry 注册 "gear"→ID 44，
    编码为 DashboardValuesFrameV2 { values: [(44, 2.0), ...] }
  → DashboardFrameBus / poll_dashboard_frame 返回 V2 帧

acc-coach list_registered_dashboard_layouts:
  内置 layout gear 控件 (telemetry_field: Some("gear"), format: None, text_template: "{gear}")
  → compile_control:
      telemetry_field_id = resolve_field_id("gear") = 44
      compiled_text_template = compile_text_template("{gear}") = "{44}"
      format = None.or_else(infer_format(...)) = Some("gear")   ← 改动1 生效
  → 下发 CompiledDashboardControl { telemetry_field_id: 44, format: "gear", compiled_text_template: "{44}", ... }

子模块 module_local_dashboard:
  useDashboardMetadata 收到 compiled layout
  → LocalDashboardOverlay: control.format==="gear" && telemetryFieldId===44 → gearFieldId = 44   ← 改动2 生效
  → useDashboardFrame: gearFieldId=44, fullFrameValues.get(44)=2.0 → smoothGear(state, 2.0, ms)
  → dashboardRenderer.resolveControlText: isGearField=true → formatGear(2.0) = "1"   ← 改动1 生效
  → overlay 显示 "1"（2 档），而非原始 "2"
```

**边界值映射表**（`telemetryFormat.ts:89-93`）：

| ACC raw gear | formatGear 输出 | 含义 |
|---|---|---|
| `<= 0` | `R` | 倒挡 |
| `1` | `N` | 空挡 |
| `2` | `1` | 1 档 |
| `3` | `2` | 2 档 |
| `n` (n>=2) | `n-1` | (n-1) 档 |

---

## 7. 验收标准

| 验收项 | 验证方法 | 责任方 |
|---|---|---|
| `cargo test` 通过（含 5 个新单测） | `cargo test --package acc-coach` | acc-coach |
| `infer_format_from_telemetry_field_for_gear` 通过 | 单测断言 `compiled.format == Some("gear")` | acc-coach |
| `infer_format_from_template_when_telemetry_field_none` 通过 | 单测断言模板兜底推断生效 | acc-coach |
| `infer_format_respects_explicit_format` 通过 | 单测断言显式 format 不被覆盖 | acc-coach |
| `infer_format_accepts_canonical_gear_key` 通过 | 单测断言 `raw:controls.gear` 经 alias 命中 | acc-coach |
| 前端 `npm run typecheck` 通过 | 删除 format 输入框后无类型错误 | acc-coach |
| 前端 `npm run lint` 通过 | ESLint 无 error | acc-coach |
| 内置 layout gear 控件编译后有 `telemetry_field_id` | 单测或运行时 `ACC_DASHBOARD_LOG=1` 观察 | acc-coach |
| overlay 实车显示 gear 为 R/N/1/2 | 启动 ACC + recording，观察 overlay 档位 | 集成验证 |
| gear 换挡无 N 挡闪烁 | 实车换挡，观察档位不闪烁 `N` | 集成验证（smoothGear） |
| 设计器无 "Text format" 输入框 | 打开 DashboardDesignerView，确认属性面板无该字段 | acc-coach |
| 数据库已存 layout 不回归 | 加载已有 layout，gear 控件仍正确显示 | 集成验证 |
| 不触碰禁改子模块 | `git diff` 确认 `module_local_dashboard`/`module_live_telemetry`/`acctlm_core`/`ld_to_acctlm` 无改动 | acc-coach |

---

## 8. 风险与缓解

| 风险 | 严重度 | 缓解 |
|---|---|---|
| `infer_format` 的 `iter_template_field_names` 与 `compile_text_template` 占位符解析不一致 → 推断的字段名与编译注册 ID 的字段名不同源 → 推断命中但 ID 查不到值 | 中 | 扫描逻辑刻意与 `compile_text_template:155-178` 单花括号分支一致；单测 `infer_format_from_template_when_telemetry_field_none` 覆盖此路径 |
| 用户存量 layout 的 `control.format` 含非 `"gear"` 的语义型值（如 `"integer"`）→ 编译器尊重显式值不推断，行为与改动前一致 | 低 | `or_else` 语义保证兼容；这是设计意图，非 bug |
| 隐藏 format 输入框后，用户无法新建数值格式型 `control.format` | 低 | 数值格式型改走 `{field\|format}` 模板语法（子模块已支持）；存量无此用法（已确认） |
| `smoothGear` 依赖 `telemetryFieldId`，若用户自定义 gear 控件未绑 `telemetry_field` → 防抖不工作 | 低 | `infer_format` 的模板兜底会设 `format="gear"`，但 `telemetryFieldId` 仍为 None；防抖不工作但 `formatGear` 仍生效（显示正确，仅换挡瞬间可能闪 N）。可通过设计器引导用户绑定字段缓解，本阶段不强制 |
| 未来扩展 lapTime/delta 推断时字段名匹配过宽 | 低 | `match` 精确匹配字段名，非子串；扩展时需列举完整字段名集合 |
| 设计器删除 format 输入框后，`selected.format` 的 TS 类型仍保留 → 死字段 | 低 | 类型保留是刻意的（编译器读它做兼容）；ESLint 不会报，因为 `updateSelected` 仍接受 `format` 字段 |

---

## 9. 模块间开发顺序

本阶段**全部在 acc-coach 侧**，不涉及子模块改动，无联调依赖：

```
1. 改动1（compiler.rs：infer_format + 单测）   ← 独立，可单测验证
2. 改动2（layout.rs：补 telemetry_field）        ← 依赖改动1的推断才完整生效
3. 改动3（设计器：隐藏 format 输入框）           ← 独立 UI 改动
4. 集成验证：cargo test + npm run typecheck/lint + 实车 overlay
```

三处改动可并行开发，最后一起集成验证。

---

## 10. 参照文档

- [stage-4-field-id-registry-and-compiler.md](./stage-4-field-id-registry-and-compiler.md) — Field Registry + Layout 编译器 + compiled 类型
- [stage-5-local-dashboard-id-switch.md](./stage-5-local-dashboard-id-switch.md) — Local Dashboard ID 化硬切换（本阶段为其遗漏修复）
- [stage-6-remote-dashboard-id-protocol.md](./stage-6-remote-dashboard-id-protocol.md) — Remote Dashboard ID 协议（remote 路径不在本阶段范围）
- `module_local_dashboard/audit/2026-06-28-initial-deep-audit.md` — 子模块审计（确认 V2 改造、format 识别机制）
- `module_dashboard_protocol/types/index.ts:102-106` — `DashboardTextFormat` 类型定义
- `module_local_dashboard/src-ui/features/local-dashboard-overlay/telemetryFormat.ts:89-93` — `formatGear` 映射实现
- `module_local_dashboard/src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx:344-387` — `resolveControlText`（gear 识别与触发）
- `src/dashboard/compiler.rs:37-108` — `compile_control`（改动1 接入点）
- `src/dashboard/compiler.rs:110-186` — `resolve_field_id` / `compile_text_template`（alias 用法与占位符解析参考）
- `src/dashboard/layout.rs:252-281` — 内置默认 layout gear 控件（改动2 接入点）
- `src-ui/components/DashboardDesignerView.tsx:3061-3070` — format 输入框（改动3 接入点）

---

## 11. 后续扩展（不在本阶段）

本阶段只推断 `gear`。`infer_format_for_field` 的 `match` 已预留扩展注释。未来可按需追加：

| 字段 | 推断 format | 触发的子模块格式化 |
|---|---|---|
| `bestLapDeltaTimeMs`、`sessionLapDeltaTimeMs` | `delta` | `formatDelta`（`+/-.xxxs`） |
| `bestLapTimeMs`、`sessionBestLapTimeMs`、`predictedLapTimeByBest`、`predictedLapTimeBySession` | `lapTime` | `formatLapTime`（`m:ss.fff`） |

扩展时只需在 `infer_format_for_field` 的 `match` 增加分支，无需改架构。但需逐项确认子模块对应 `format*` 函数的触发条件（是否也靠 `control.format === "xxx"`）。



