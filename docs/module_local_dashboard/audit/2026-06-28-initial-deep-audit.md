# module_local_dashboard 首次深度审计

- 状态：**Closed** ✅
- 修复提交：`cdfe1ad`（第一/二轮修复）→ `a5b2364`（撤销误判 H2/M4，第三轮）
- 审计日期：2026-06-28
- 审计类型：首次深度审计（Full Deep Audit）
- 审计范围：`module_local_dashboard` 仓库全部代码（Rust 后端 + TypeScript 前端 + 配置 + 与 acc-coch 集成一致性）
- 审计基准：commit `2ed1474`（master, working tree clean）
- 审计工具：codegraph、grep、Read、cargo test、vitest、tsc
- 审计基线：**以代码为准**。审计过程中发现 `local-dashboard-overlay-plan.md`、`local-dashboard-public-api.md`、`integration-notes.md`、`acc-coach/prd/local-dashboard-self-managed-data.md` 四份文档均已过时且与代码冲突，经用户确认后删除。本报告所有结论以当前代码实现为唯一基准，不再引用上述文档。

## 0. 执行摘要

本仓库是一个**代码级子模块**（不是独立 Tauri 应用），为 acc-coch 提供：
- Rust 库 `acc_coach_local_dashboard_overlay`：overlay 窗口管理、ACC 窗口探测、配置持久化、帧总线
- TypeScript/React 特性目录 `src-ui/features/local-dashboard-overlay`：仪表盘渲染、格式化、编辑/预览组件

### 整体健康度

| 维度 | 评级 | 说明 |
|---|---|---|
| 编译/测试 | 🟢 良好 | `tsc --noEmit` 通过；`vitest` 41 passed；`cargo test` 7 passed |
| 架构设计 | 🟡 可改进 | 前端存在死代码（`overlayConfigApi.ts`）；帧轮询 hook 忽略配置频率 |
| 类型安全 | 🟡 有瑕疵 | 5 处不必要的 `as any` 逃逸 |
| 测试覆盖 | 🟡 不足 | 仅纯函数有测试；组件、hooks、IPC 路径无测试 |
| 死代码 | 🟡 存在 | `overlayConfigApi.ts`、`AccWindowMatchedBy::Fallback` 变体 |
| 文档状态 | 🟢 已清理 | 4 份过时文档已删除，`docs/module_local_dashboard/` 仅保留本审计报告 |

### 问题统计

| 严重度 | 数量 |
|---|---|
| Critical | 1 |
| High | 2 |
| Medium | 3 |
| Low | 7 |
| **合计** | **13** |

> 已删除的文档相关问题（原 C1、C2、M2、M3、M7、H5）不再计入。这些问题曾以"代码与文档不符"定性，文档删除后"不符"失去依据，代码本身的行为视为当前正确架构。
>
> 原 H1（`frame_ms` clamp 下限 4ms 过低）经用户确认 4ms 为设计决策，不计为问题。`config.rs:117` 的 `clamp(4, 1000)` 表示帧间隔最小 4ms（最高 250Hz）、最大 1000ms（最低 1Hz），此范围正确。
>
> **原 H2 和 M4 经修复后验证为误判，已撤销修复（详见第 14 节）。** 实际有效问题数从 15 降为 13。

---

## 1. 仓库结构与代码组织

### 1.1 目录布局

```
module_local_dashboard/
├── Cargo.toml                              # Rust crate: acc-coach-local-dashboard-overlay
├── src/
│   ├── lib.rs                              # crate root，re-export 公开 API
│   └── local_dashboard_overlay/
│       ├── mod.rs                          # 模块声明 + setup()
│       ├── config.rs                       # 配置结构、加载/保存/规范化（250行）
│       ├── window.rs                       # Tauri overlay 窗口管理（88行）
│       ├── acc_window.rs                   # Windows 平台 ACC 窗口探测（109行）
│       └── frame_bus.rs                    # DashboardFrameBus 帧总线（31行）
├── package.json                            # TS 项目：typecheck + test
├── tsconfig.json / tsconfig.node.json
├── vite.config.ts                          # alias + vitest 配置
└── src-ui/
    └── features/local-dashboard-overlay/
        ├── index.ts                        # 特性 barrel export
        ├── types.ts                        # 协议类型 re-export + 模块自有类型
        ├── telemetryFormat.ts              # 数值/档位/圈速格式化（134行）
        ├── textExpression.ts               # 沙盒表达式求值器（335行）
        ├── dashboardRenderer.tsx           # 区域/控件渲染（629行）
        ├── LocalDashboardOverlay.tsx       # overlay 运行时组件（99行）
        ├── OverlayRegionEditor.tsx         # 区域编辑器（122行）
        ├── OverlayRegionPreview.tsx        # 组合预览（74行）
        ├── overlayConfigApi.ts             # Tauri invoke 封装（35行）⚠️ 死代码
        ├── useDashboardFrame.ts            # 帧轮询 hook（128行）
        ├── useDashboardMetadata.ts         # 元数据轮询 hook（147行）
        ├── LocalDashboardOverlay.module.css
        ├── dashboardRenderer.test.ts       # 20 tests
        └── textExpression.test.ts          # 21 tests
```

### 1.2 组织评价

- **正面**：Rust 模块按职责清晰分文件（config/window/acc_window/frame_bus）；前端按特性目录组织；协议类型统一从 `module_dashboard_protocol` 引入，未在子模块内重复定义。
- **负面**：
  - `overlayConfigApi.ts` 是死代码（详见 C3），acc-coch 用自己的 `overlayApi.ts` 替代。
  - `useDashboardFrame.ts` 和 `useDashboardMetadata.ts` 使 `LocalDashboardOverlay` 成为自取数据组件——这本身是当前架构选择，但导致组件难以独立测试和复用。

---

## 2. 构建与测试状态

### 2.1 TypeScript

```
> tsc --noEmit        ✅ 通过，无类型错误
> vitest run          ✅ 2 files, 41 tests passed (342ms)
```

### 2.2 Rust

```
> cargo test          ✅ 7 tests passed (config.rs)
```

`acc_window.rs` 的 Windows 分支（`#[cfg(windows)]`）无单元测试，仅有非 Windows 的 `non_windows_returns_none` 测试。

### 2.3 缺失项

- `package.json` **无 `lint` 脚本**，无 ESLint/Prettier 配置 → 代码风格无自动化保障。
- `package.json` **无 `build` 脚本**（子模块由 acc-coch 构建，合理）。
- `vite.config.ts` 的 `test.environment: "node"` → 无法运行组件渲染测试（需要 `jsdom` 或 `happy-dom`）。

---

## 3. 严重问题（Critical）

### C3: `overlayConfigApi.ts` 是死代码

**位置**：`src-ui/features/local-dashboard-overlay/overlayConfigApi.ts`

**证据**：
- 该文件导出 7 个函数（`getLocalDashboardOverlayConfig`、`saveLocalDashboardOverlayConfig`、`getAccWindowBounds`、`showLocalDashboardOverlay`、`hideLocalDashboardOverlay`、`setLocalDashboardOverlayBounds`、`setLocalDashboardOverlayClickThrough`）。
- acc-coch 通过 `@local-dashboard-overlay` alias 导入时，**没有导入这些函数**（grep 确认 acc-coch 的 `LocalDashboardOverlayWindow.tsx`、`LocalDashboardOverlayController.tsx`、`LocalDashboardView.tsx` 都从自己的 `../overlayApi` 导入，而非从子模块导入）。
- acc-coch 的 `overlayApi.ts` 重新实现了完全相同的函数，并额外处理了 `dashboardWidth/dashboardHeight` 的 localStorage 持久化。
- `index.ts` 导出了 `./overlayConfigApi`，但无消费者。

**影响**：维护两份相同功能的 API 封装（子模块一份、主模块一份），容易出现行为漂移。

**建议**：删除 `overlayConfigApi.ts`，或将其标注为"仅供子模块内部测试使用"并从 `index.ts` 公开导出中移除。

---

## 4. 高优先级问题（High）

### H1: `useDashboardFrame.ts` 忽略配置的 `frameMs`，使用 `requestAnimationFrame`

**位置**：`src-ui/features/local-dashboard-overlay/useDashboardFrame.ts:110`

**代码**：
```ts
rafRef.current = requestAnimationFrame(poll);  // ~16ms @ 60Hz
```

**问题**：
- 配置中 `polling.frameMs`（当前 clamp 范围 4-1000ms）完全未被使用。
- `requestAnimationFrame` 以显示器刷新率（通常 60Hz = ~16ms）调用，不受配置控制。
- 这使得 `config.rs` 对 `frameMs` 的钳制逻辑形同虚设。

**影响**：
- overlay 窗口即使配置了 1000ms 轮询，实际仍以 60Hz 轮询 IPC，造成不必要的 CPU 开销。
- 配置 UI 让用户调整 `frameMs` 但无实际效果，误导用户。

**建议**：使用 `setInterval` 或在 rAF 回调中做节流，确保实际轮询频率受 `config.polling.frameMs` 控制。

---

### ~~H2: `useDashboardFrame.ts` 的 `fullFrameRef` 永久累积所有历史字段~~ [误判 — 已撤销]

> **⚠️ 本条为误判，已于第 14 节撤销修复。** `poll_dashboard_frame` 返回的是稀疏帧（每帧只含变化的字段），`fullFrameRef` 累积是**必要语义**——不做累积会导致未出现在当前帧的字段变 `undefined`，text widget 闪烁显示 `--`/`NaN`。保留原文供追溯。

**位置**：`src-ui/features/local-dashboard-overlay/useDashboardFrame.ts:60`

**原文（误判）**：
- `fullFrameRef.current` 是一个累积的 map，每帧只覆盖当前帧存在的字段，**不删除已消失的字段**。
- 如果某个遥测字段（如 `trackName`）在某帧出现，后续帧不再包含它，它的旧值仍会保留在 `fullFrameRef.current` 中并被渲染。
- `handleClear` 会在 `poll_dashboard_frame` 返回 `null` 时重置 `fullFrameRef`。

**实际正确行为**：acc-coch 的 `poll_dashboard_frame` 返回稀疏帧（仅含本帧变化的字段）。累积合并是让 text widget 在字段未变化时保持上一次有效值的必要机制。撤销修复后代码恢复为 `{ ...fullFrameRef.current, ...frame.values }`。

---

### H3: `AccWindowMatchedBy::Fallback` 是死变体

**位置**：`src/local_dashboard_overlay/acc_window.rs:16-19`

**代码**：
```rust
pub enum AccWindowMatchedBy {
    Title,
    Fallback,   // ← 从不被构造
}
```

`find_acc_window_bounds` 在找到窗口时返回 `matched_by: AccWindowMatchedBy::Title`（第 82 行），找不到时返回 `Ok(None)`（第 94 行）。`Fallback` 变体从不被构造。

但 acc-coch 的前端代码（`LocalDashboardOverlayWindow.tsx:64, 78`）在构造 fallback bounds 时使用 `matchedBy: "fallback"`，这与 Rust 端的 `Fallback` 变体对应——只是它由前端构造，而非 Rust 后端返回。

**影响**：
- Rust 端 `Fallback` 变体无用，但前端依赖该字符串值。序列化保持一致（`#[serde(rename_all = "camelCase")]` → `"fallback"`），所以类型层面兼容。
- 如果有人清理 Rust 端的 `Fallback` 变体，前端仍能工作（因为前端自己构造），但 `AccWindowBounds.matchedBy` 字段的类型契约会变得不精确。

**建议**：保留 `Fallback` 变体（因为它是类型契约的一部分，前端会构造该值），但在 Rust 端添加文档注释说明"Fallback 由调用方构造，`find_acc_window_bounds` 不会返回此值"。

---

## 5. 中优先级问题（Medium）

### M1: 5 处不必要的 `as any` 类型逃逸

**位置**：
- `useDashboardFrame.ts:24` — `(control as any).chartSampleCount ?? control.chartSampleCount ?? 600`
- `useDashboardFrame.ts:26` — `(field as any).telemetryField ?? ""`
- `useDashboardFrame.ts:33` — `(field as any).defaultValue ?? field.defaultValue ?? 0`
- `dashboardRenderer.tsx:463` — `(control as any).chartSampleCount ?? control.chartSampleCount ?? 600`
- `dashboardRenderer.tsx:482` — `(field as any).telemetryField ?? ""`

**问题**：`module_dashboard_protocol/types` 已明确定义了 `DashboardControl.chartSampleCount`（第 68 行）、`ChartFieldConfig.telemetryField`（第 13 行）、`ChartFieldConfig.defaultValue`（第 16 行）。这些 `as any` 完全多余，`(control as any).X ?? control.X` 等价于 `control.X`。

**建议**：删除所有 `as any`，直接使用 `control.chartSampleCount ?? 600`、`field.telemetryField`、`field.defaultValue ?? 0`。

---

### ~~M4: 硬编码 track ID fallback `"monza"`~~ [误判 — 已撤销]

> **⚠️ 本条为误判，已于第 14 节撤销修复。** `"monza"` 是整个项目（acc-coch 设计器 `DashboardDesignerView.tsx:1551`、`LocalDashboardView.tsx:170`）统一的 `trackId` 为 null 时的 fallback。map widget 在设计器里创建时 `trackId` 默认就是 `null`。审计时只看子模块内部未查 acc-coch 约定，误判为硬编码错误。保留原文供追溯。

**位置**：
- `dashboardRenderer.tsx:533` — `const effectiveTrackId = trackId || "monza";`
- `useDashboardMetadata.ts:118` — `const tid = control.trackId || "monza";`

**原文（误判）**：当 `trackId` 为空时，默认使用 "monza" 赛道数据。如果 acc-coch 的 track map 数据集中没有 "monza"，会静默失败。

**实际正确行为**：acc-coch 的 `DashboardDesignerView.tsx:1550-1551` 和 `LocalDashboardView.tsx:170` 都用 `control.trackId || "monza"` 做 fallback。map widget 创建时 `trackId` 默认 `null`（`DashboardDesignerView.tsx:204`，有测试 `DashboardDesignerView.test.tsx:173` 确认）。`"monza"` 是项目级约定，子模块应保持一致。撤销修复后三处 fallback 均恢复为 `|| "monza"`。

---

### M5: 模块级单例状态导致测试隔离风险

**位置**：
- `useDashboardMetadata.ts:29` — `const TRACK_POINTS_CACHE: Record<string, TrackPointsData> = {};`（模块级单例缓存）
- `dashboardRenderer.tsx:283` — `const controlDependencyCache = new WeakMap<DashboardControl, string[]>();`（模块级缓存）

**问题**：
- `TRACK_POINTS_CACHE` 在所有组件实例、所有测试之间共享。测试中若预填充了某个 trackId，后续测试会命中缓存而跳过 IPC，导致测试行为不确定。
- `controlDependencyCache` 用 `WeakMap`，key 是 `DashboardControl` 对象引用。每次渲染若 control 对象引用变化（如来自新 JSON parse），缓存失效。但模块级单例在多窗口场景下可能共享意外数据。

**建议**：
- `TRACK_POINTS_CACHE` 改为 React context 或 hook 内部的 `useRef`，避免模块级共享。
- `controlDependencyCache` 可保留模块级（WeakMap 随 key GC 自动清理），但应在文档中注明。

---

### M6: `Cargo.toml` 包含未使用的 `windows-sys` features

**位置**：`Cargo.toml:17-22`

```toml
windows-sys = { version = "0.59", features = [
  "Win32_Foundation",
  "Win32_System_Memory",       # ← 未使用
  "Win32_UI_WindowsAndMessaging",
  "Win32_Graphics_Gdi"         # ← 未使用
] }
```

`acc_window.rs` 只 import 了 `Win32::Foundation` 和 `Win32::UI::WindowsAndMessaging`。`Win32_System_Memory` 和 `Win32_Graphics_Gdi` 未被引用。

**影响**：增加不必要的编译依赖，拖慢编译。

**建议**：移除未使用的两个 feature。

---

## 6. 低优先级问题（Low）

### L1: `acc_window.rs` unsafe 代码缺少安全注释和边界检查

**位置**：`src/local_dashboard_overlay/acc_window.rs:30-85`

- 第 41 行 `vec![0u16; title_len as usize + 1]`：`title_len` 为 `i32`，若返回极大值可能整数溢出或大分配。建议 `title_len as usize + 1` 前加上上限检查。
- 第 70-71 行 `width = rect.right - rect.left`：`i32` 减法，极端情况可能为负，随后 `as u32` 会得到巨大值。Windows 通常保证 `right >= left`，但防御性检查更安全。
- 第 72 行 `if width < 800 || height < 450`：阈值硬编码，建议提取为常量 `const MIN_ACC_WINDOW_WIDTH: i32 = 800`。
- 整个 `enum_window` 回调无 `// SAFETY:` 注释说明为何 unsafe 是安全的。

### L2: `window.rs` 初始窗口大小和阈值硬编码

**位置**：`src/local_dashboard_overlay/window.rs:27`

`.inner_size(1280.0, 720.0)` 硬编码初始大小。虽随后被 `set_overlay_bounds` 覆盖，但建议提取为常量 `const DEFAULT_OVERLAY_WIDTH: f64 = 1280.0`。

### L3: `package.json` 缺少 lint 脚本和 ESLint 配置

无 `lint` 脚本，无 ESLint/Prettier 配置文件。建议添加基础 ESLint 配置（至少 `@typescript-eslint/recommended` + `react-hooks/rules-of-hooks`）。

### L4: 测试覆盖不足

| 模块 | 测试状态 |
|---|---|
| `config.rs` | ✅ 7 个单元测试（覆盖 default/load/save/normalize/validate/corrupt） |
| `acc_window.rs` | ⚠️ 仅非 Windows 1 个测试；Windows 分支（unsafe EnumWindows）无测试 |
| `frame_bus.rs` | ❌ 无测试（Mutex + Option 逻辑简单，但 push/latest/clear 应有测试） |
| `window.rs` | ❌ 无测试（依赖 Tauri AppHandle，难以单测，可接受） |
| `telemetryFormat.ts` | ✅ 通过 `dashboardRenderer.test.ts` 间接覆盖 |
| `textExpression.ts` | ✅ 21 个测试（函数、运算符、三元、嵌套、错误路径） |
| `dashboardRenderer.ts` | ✅ 20 个测试（纯函数：computeRegionRect/resolveControlText/conditionalRules/memoization） |
| `LocalDashboardOverlay.tsx` | ❌ 无组件渲染测试 |
| `useDashboardFrame.ts` | ❌ 无 hook 测试 |
| `useDashboardMetadata.ts` | ❌ 无 hook 测试 |
| `OverlayRegionEditor.tsx` | ❌ 无组件测试 |
| `OverlayRegionPreview.tsx` | ❌ 无组件测试 |

**建议**：优先补充 `frame_bus.rs` 和 `acc_window.rs` Windows 分支的测试（可 mock 或用 `#[cfg(test)]` 隔离）；前端 hooks 用 `@testing-library/react-hooks` 补充。

### L5: `tsconfig.json` paths 与 `vite.config.ts` alias 不一致

**位置**：
- `tsconfig.json:18-20`：`"module_dashboard_protocol/*": ["../module_dashboard_protocol/*"]`（通配）
- `vite.config.ts:7-10`：只 alias 了 `module_dashboard_protocol/types`（单路径）

**影响**：当前代码只 import `module_dashboard_protocol/types`，所以无实际问题。但若将来 import 其他路径（如 `module_dashboard_protocol/foo`），tsc 能解析但 vite 不能。

**建议**：vite alias 改为 `"module_dashboard_protocol": path.resolve(__dirname, "../module_dashboard_protocol")` 以支持通配。

### L6: `vite.config.ts` 使用 `vite` 的 `defineConfig` 但包含 `test` 字段

**位置**：`vite.config.ts:4-17`

```ts
import { defineConfig } from "vite";  // ← vite 的 defineConfig
export default defineConfig({
  // ...
  test: {  // ← vitest 字段，vite 类型不认识
    environment: "node",
    include: ["src-ui/**/*.test.ts", "src-ui/**/*.test.tsx"]
  }
});
```

LSP 报错：`No overload matches this call. ... 'test' does not exist in type 'UserConfigExport'.`。`tsc --noEmit` 通过是因为 `tsconfig.node.json` 不做严格类型检查。vitest 能运行是因为它在运行时合并了 `test` 字段。

**建议**：改为 `import { defineConfig } from "vitest/config";`（vitest 提供了扩展了 `test` 字段的 `defineConfig` 类型）。

### L7: `useDashboardMetadata.ts` 的 `prefetchTrackMaps` 用空 trackId 预取

**位置**：`useDashboardMetadata.ts:113-127`

```ts
const tid = control.trackId || "monza";  // 空时用 monza
if (!seen.has(tid)) { seen.add(tid); loadTrackMap(tid); }
```

对 `widgetType !== "map"` 的控件，`trackId` 通常为 `null`，也会触发预取 "monza"。这会无条件加载 monza track map，即使布局中没有任何 map widget。

**建议**：只对 `widgetType === "map"` 的控件预取 track map。

---

## 7. 子模块交互与协议一致性

### 7.1 Rust 公开 API

| API | 代码实现 | 状态 |
|---|---|---|
| `local_dashboard_overlay::setup(app)` | `mod.rs:6` — 仅调用 `ensure_overlay_window` | ✅ 正常 |
| `AccWindowBounds` / `AccWindowMatchedBy` | `acc_window.rs:5,16` | ✅ 正常 |
| `LocalDashboardOverlayConfig` 及子结构 | `config.rs:13-59` | ✅ 正常 |
| `find_acc_window_bounds()` | `acc_window.rs:22` | ✅ 正常 |
| `window::ensure/show/hide/set_bounds/set_click_through` | `window.rs:7,34,45,54,67` | ✅ 正常 |
| `DashboardFrameBus` | `frame_bus.rs:5` — `new`/`push_frame`/`latest_frame`/`clear` 均 pub | ✅ 正常 |
| Rust 端不暴露 `#[tauri::command]` | 确认无命令定义 | ✅ 正常 |

注：`setup()` 仅创建 overlay 窗口，`DashboardFrameBus` 的创建和 managed state 注册由 acc-coch 的 `main.rs:93` 自行完成。这是当前的架构分工。

### 7.2 acc-coch 集成验证

- `acc-coch/src/overlay_commands.rs`：正确调用子模块的 `find_acc_window_bounds`、`window::show/hide/set_bounds/set_click_through`、`LocalDashboardOverlayConfig::load_or_create/save`。✅
- `acc-coch/src/main.rs:93`：自行创建 `Arc::new(DashboardFrameBus::new())` 并 manage。✅
- `acc-coch/src/ipc/mod.rs:3136-3186`：注册了所有 7 个 overlay 命令 + `poll_dashboard_frame` + `list_registered_dashboard_layouts` + `get_track_map`。✅
- `acc-coch/src-ui/overlayApi.ts`：重新实现了子模块 `overlayConfigApi.ts` 的全部函数并扩展。⚠️ 重复（见 C3）
- `acc-coch/src-ui/components/LocalDashboardOverlayWindow.tsx:372-379`：使用 `LocalDashboardOverlay` 的实际 props 签名（`visible/viewportWidth/viewportHeight/showClosePreview/onClosePreview`）。✅

### 7.3 协议类型一致性

| 协议类型（module_dashboard_protocol/types） | 子模块使用 | 一致性 |
|---|---|---|
| `DashboardValuesFrame` | `useDashboardFrame.ts`、`dashboardRenderer.tsx` | ✅ |
| `DashboardControl` | `dashboardRenderer.tsx` | ✅（但 5 处 `as any`，见 M1） |
| `DashboardLayoutPayload` | `dashboardRenderer.tsx`、`useDashboardMetadata.ts` | ✅ |
| `RegisteredDashboardLayout` | `useDashboardMetadata.ts` | ✅ |
| `WidgetType` | `dashboardRenderer.tsx` | ✅ |
| `ChartFieldConfig` | `dashboardRenderer.tsx`、`useDashboardFrame.ts` | ✅ |
| `DashboardConditionalRule` | `dashboardRenderer.tsx` | ✅ |
| `DashboardTextFormat` | `telemetryFormat.ts` | ✅ |

---

## 8. 安全性审查

### 8.1 `textExpression.ts` 沙盒表达式求值器

- **不使用 `eval` / `Function` 构造器** ✅
- 手写递归下降解析器，支持有限运算符和 `abs`/`round` 两个内置函数 ✅
- 未知函数抛错（`Unknown function`）✅
- 未知标识符抛错（`Unknown telemetry field`）✅
- **无原型链访问、无全局对象访问** ✅
- 结论：表达式求值器是安全的沙盒实现。

### 8.2 `acc_window.rs` unsafe 代码

- `EnumWindows` 回调中通过 `lparam` 传递裸指针 ✅（Windows API 标准模式）
- 指针来源是栈上 `&mut found as *mut Option<AccWindowBounds>`，生命周期在 `EnumWindows` 同步调用范围内 ✅
- 无缓冲区溢出风险（`GetWindowTextW` 接受 buffer 长度参数）✅
- 建议：添加 `// SAFETY:` 注释（见 L1）。

### 8.3 配置文件处理

- `config.rs` 使用 `serde_json::from_str` 解析，无 `eval` 风险 ✅
- `validate_identity` 校验 schema 和 version ✅
- corrupt JSON 不会覆盖原文件 ✅（有测试覆盖）
- 无路径遍历风险（路径由主模块 `app_data_dir` 提供）✅

### 8.4 未发现密钥/凭证泄露

代码中无硬编码密钥、token、URL。

---

## 9. 性能审查

### 9.1 帧轮询频率（见 H1）

`useDashboardFrame` 使用 `requestAnimationFrame`（~60Hz）而非配置的 `frameMs`，导致过度轮询。

### 9.2 `DashboardFrameBus` 锁竞争

`frame_bus.rs` 使用 `Mutex<Option<DashboardValuesFrame>>`：
- `push_frame`：锁 → clone → 替换 → 解锁
- `latest_frame`：锁 → clone → 解锁

在高频录制循环（push）和 overlay 轮询（latest）同时发生时，锁竞争可能导致短暂阻塞。但 `Option<Frame>` 的 clone 是固定开销（frame.values 是 `HashMap`，clone 为 O(n)）。对于 ACC 遥测（~30 字段），单次 clone < 1μs，可接受。

### 9.3 前端 `useMemo` 依赖

`LocalDashboardOverlay.tsx:36-47` 的 `trackPoints` useMemo 每次从 `trackPointsCache` 重建新对象。这会破坏 `MapWidget` 的 `controlPropsAreEqual` 浅比较，导致不必要的重渲染。但 `trackPointsCache` 只在 track map 加载时变化，所以实际影响小。

### 9.4 `controlDependencyCache` WeakMap

`dashboardRenderer.tsx:283` 缓存了每个 control 的依赖字段列表，避免每次渲染都解析模板。合理。但若 control 对象引用频繁变化（如每帧新 parse），缓存命中率低。acc-coch 的 layout 注册机制应保证 control 引用稳定。

---

## 10. 修复优先级建议

| 优先级 | 问题 | 建议动作 | 涉及模块 |
|---|---|---|---|
| P0 | C3 | 删除 `overlayConfigApi.ts` 或从 `index.ts` 移除其导出 | module_local_dashboard |
| P1 | H1 | 帧轮询改为 `setInterval` 受 `frameMs` 控制 | module_local_dashboard |
| P1 | H2 | `fullFrameRef` 不再累积，每帧直接用 `frame.values` | module_local_dashboard |
| P1 | M1 | 删除 5 处 `as any` | module_local_dashboard |
| P2 | H3 | Rust 端为 `Fallback` 变体添加文档注释 | module_local_dashboard |
| P2 | M6 | 移除未使用的 windows-sys features | module_local_dashboard |
| P3 | M4 + M5 + L1-L7 | 硬编码常量、lint 配置、测试补充、vite alias 等 | module_local_dashboard |

---

## 11. 附录

### 11.1 审计文件清单

**Rust（6 文件，496 行）**：
- `src/lib.rs`（10 行）
- `src/local_dashboard_overlay/mod.rs`（8 行）
- `src/local_dashboard_overlay/config.rs`（250 行，含测试）
- `src/local_dashboard_overlay/window.rs`（88 行）
- `src/local_dashboard_overlay/acc_window.rs`（109 行，含测试）
- `src/local_dashboard_overlay/frame_bus.rs`（31 行）

**TypeScript（13 文件，~1900 行）**：
- `src-ui/features/local-dashboard-overlay/` 下 13 个文件（含 2 个测试文件）

**配置（5 文件）**：
- `Cargo.toml`、`package.json`、`tsconfig.json`、`tsconfig.node.json`、`vite.config.ts`

**文档**：
- 原 `docs/module_local_dashboard/` 下 3 份设计文档（`local-dashboard-overlay-plan.md`、`local-dashboard-public-api.md`、`integration-notes.md`）均因过时与代码冲突，经用户确认后于本次审计中删除。
- 原 `docs/acc-coch/prd/local-dashboard-self-managed-data.md` 同样因过时删除。
- `docs/module_local_dashboard/audit/` 目录于本次审计创建，仅含本报告。

### 11.2 验证命令结果

```
$ npm run typecheck   → ✅ 通过
$ npm test            → ✅ 41 passed
$ cargo test          → ✅ 7 passed
```

### 11.3 acc-coch 集成关键文件（只读审计）

- `acc-coach/src/overlay_commands.rs`（61 行）— 子模块 Rust API 的 IPC 包装
- `acc-coach/src/ipc/mod.rs`（3000+ 行）— 命令注册和 `poll_dashboard_frame` 实现
- `acc-coach/src/main.rs` — `DashboardFrameBus` 创建和 manage
- `acc-coach/src-ui/overlayApi.ts`（103 行）— 子模块 `overlayConfigApi.ts` 的主模块版本
- `acc-coach/src-ui/components/LocalDashboardOverlayWindow.tsx`（383 行）— overlay 窗口运行时

### 11.4 本次审计中的文档清理记录

审计过程中发现以下 4 份文档与当前代码实现严重冲突，经用户确认后删除：

| 文档 | 冲突原因 |
|---|---|
| `docs/module_local_dashboard/local-dashboard-overlay-plan.md` | 初始开发计划，描述子模块暴露 Tauri 命令，与实际架构不符 |
| `docs/module_local_dashboard/local-dashboard-public-api.md` | v2 契约文档，描述前端为受控组件不调用 invoke，与实际代码相反；协议类型描述与 `module_dashboard_protocol/types` 不符 |
| `docs/module_local_dashboard/integration-notes.md` | 配套集成说明，描述了过时的 `LocalDashboardOverlayProps` 签名 |
| `docs/acc-coach/prd/local-dashboard-self-managed-data.md` | PRD 要求 `setup()` 注册 `DashboardFrameBus` 为 managed state，实际由 acc-coch 自行创建 |

决策：以代码为准，删除过时文档。后续如需文档，应基于当前代码重新撰写。

---

## 12. 第一轮修复验证（2026-06-28）

开发人员根据本审计报告完成第一轮修复，本节记录验证结果。

### 12.1 构建测试结果

```
$ npm run typecheck   → ✅ 通过
$ npm test            → ✅ 2 files, 41 tests passed (316ms)
$ cargo test          → ✅ 7 passed
```

LSP 不再报告 `vite.config.ts` 类型错误。

### 12.2 逐项核对清单

| 问题 | 状态 | 验证证据 |
|---|---|---|
| **C3** `overlayConfigApi.ts` 死代码 | ✅ 已修复 | 文件已删除；`index.ts` 不再导出；grep 确认子模块内无 `overlayConfigApi` 残留引用；acc-coch 的 4 处 `@local-dashboard-overlay` 导入不依赖该文件 |
| **H1** `useDashboardFrame` 忽略 `frameMs` | ✅ 已修复 | `useDashboardFrame.ts:10` 改为接收 `frameMs: number = 33` 参数；`useDashboardFrame.ts:110` 改为 `setInterval(poll, frameMs)`；`LocalDashboardOverlay.tsx:25` 从 `config?.polling.frameMs ?? 33` 传入 |
| **H2** `fullFrameRef` 永久累积字段 | ❌ 误判 — 已撤销 | 第一轮"修复"移除了 `fullFrameRef` 累积，导致稀疏帧下未变化字段变 `undefined`，text widget 闪烁显示 `--`/`NaN`（Bug 2）。第 14 节撤销修复，恢复累积逻辑 |
| **H3** `AccWindowMatchedBy::Fallback` 缺注释 | ✅ 已修复 | `acc_window.rs:18-19` 添加 doc comment：「Fallback bounds constructed by the caller (acc-coach frontend). `find_acc_window_bounds` never returns this variant.」 |
| **M1** 5 处 `as any` 类型逃逸 | ✅ 已修复 | grep 确认 `as any` 为 0；改为直接 `control.chartSampleCount`、`field.telemetryField`、`field.defaultValue` |
| **M4** 硬编码 `"monza"` fallback | ❌ 误判 — 已撤销 | 第一轮"修复"删除了 `"monza"` fallback，导致 map widget 在 `trackId` 为 null 时不加载 track map、赛道和赛车位置都不渲染（Bug 1）。`"monza"` 是 acc-coch 项目级约定（`DashboardDesignerView.tsx:1551`、`LocalDashboardView.tsx:170`）。第 14 节撤销修复，恢复三处 `\|\| "monza"` |
| **M5** 模块级单例 `TRACK_POINTS_CACHE` | ✅ 已修复 | `useDashboardMetadata.ts:87` 改为 `trackPointsCacheRef = useRef<Record<string, TrackPointsData>>({})`；grep `TRACK_POINTS_CACHE` 为 0 |
| **M6** 未使用的 `windows-sys` features | ✅ 已修复 | `Cargo.toml` 移除 `Win32_System_Memory` 和 `Win32_Graphics_Gdi` |
| **L1** unsafe 缺 SAFETY 注释和边界检查 | ✅ 已修复 | `acc_window.rs:36-39,98-100` 添加两处 `// SAFETY:` 注释；`MAX_TITLE_LENGTH=1024` 上限检查（第 47 行）；`saturating_sub` 防溢出（第 80-81 行）；`MIN_ACC_WINDOW_WIDTH/HEIGHT` 常量化（第 23-24 行） |
| **L2** 硬编码 `1280.0, 720.0` | ✅ 已修复 | `window.rs:6-7` 提取为 `DEFAULT_OVERLAY_WIDTH: f64 = 1280.0` 和 `DEFAULT_OVERLAY_HEIGHT: f64 = 720.0` |
| **L3** 缺 lint 脚本 | ⚠️ 占位修复 | `package.json` 添加 `"lint": "tsc --noEmit"`，但这只是 typecheck 的别名，不是真正的 ESLint 代码风格检查。可作为最小占位，若需真正的 lint 需补 ESLint 配置 |
| **L4** 测试覆盖不足 | ❌ 未修复 | `frame_bus.rs`、`acc_window.rs` Windows 分支、前端 hooks/组件均仍无测试（此项为改进建议，非阻塞） |
| **L5** tsconfig/vite alias 不一致 | ✅ 已修复 | `vite.config.ts:8-10` 改为 `"module_dashboard_protocol": resolve(__dirname, "../module_dashboard_protocol")`，支持通配 |
| **L6** `vite.config.ts` defineConfig 类型错误 | ✅ 已修复 | `vite.config.ts:1` 改为 `import { defineConfig } from "vitest/config"` |
| **L7** `prefetchTrackMaps` 对非 map 控件预取 | ✅ 已修复 | `useDashboardMetadata.ts:123` 加 `if (control.widgetType !== "map") continue;`；第 138 行 key 计算也加了 `.filter((c) => c.widgetType === "map")` |

### 12.3 修复统计

| 状态 | 数量 |
|---|---|
| ✅ 完全修复 | 11 |
| ⚠️ 占位修复 | 1（L3，第二轮已真正修复） |
| ❌ 未修复 | 1（L4，改进建议） |
| ❌ 误判 — 已撤销 | 2（H2、M4，详见第 14 节） |
| **合计** | **15**（有效问题 13） |

### 12.4 修复中引入的新问题

#### N1: `dashboardRenderer.tsx:463` 缩进错位（Low）

**位置**：`src-ui/features/local-dashboard-overlay/dashboardRenderer.tsx:463`

**现状**：
```tsx
function ChartWidget({ control, historyBuffer, historyVersion }: ChartWidgetProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fields = control.chartFields ?? [];
      const N = control.chartSampleCount ?? 600;   // ← 多了 4 个空格
  const { width, height } = control;
```

**原因**：删除 `(control as any).chartSampleCount ??` 前缀时，保留了原有的多余缩进。

**影响**：不影响编译和运行，但代码风格不一致。

**建议**：将第 463 行的 6 空格缩进改为与上下文一致的 2 空格缩进。

### 12.5 待处理事项

开发人员需继续处理以下两项，以达到"完全没问题"：

1. **N1（新引入）**：修正 `dashboardRenderer.tsx:463` 的缩进错位。
2. **L3（占位修复）**：决定是否引入真正的 ESLint 配置。若用户认可 `"lint": "tsc --noEmit"` 作为最终方案，则在审计报告中标注为可接受；否则需添加 ESLint + Prettier 配置。

可选改进（非阻塞）：
- **L4**：补充 `frame_bus.rs` 单元测试、`acc_window.rs` Windows 分支测试、前端 hooks/组件测试。

---

## 13. 第二轮修复验证（2026-06-28）

开发人员根据第 12 节的待处理事项完成第二轮修复，本节记录验证结果。

### 13.1 构建测试结果

```
$ npm run typecheck   → ✅ 通过
$ npm test            → ✅ 2 files, 41 tests passed (341ms)
$ npm run lint        → ✅ 通过（0 error, 0 warning）
$ cargo test          → ✅ 7 passed
```

新增 `npm run lint` 基于 ESLint，不再仅是 typecheck 别名。

### 13.2 待处理事项核对

| 事项 | 状态 | 验证证据 |
|---|---|---|
| **N1** `dashboardRenderer.tsx:463` 缩进错位 | ✅ 已修复 | `fields` 和 `N` 两行移入 `useEffect` 内部（第 470-471 行），缩进与上下文一致；原第 463 行的多余空格已消除 |
| **L3** lint 脚本（占位 → 真正 ESLint） | ✅ 已修复 | 新增 `eslint.config.js`（flat config，含 `@typescript-eslint/recommended` + `eslint-plugin-react-hooks`）；`package.json` 的 `lint` 脚本改为 `"eslint src-ui/"`；devDependencies 添加 `eslint`、`@typescript-eslint/eslint-plugin`、`@typescript-eslint/parser`、`eslint-plugin-react-hooks`；`npm run lint` 实际运行 0 error 0 warning |

### 13.3 最终状态（第二轮，后被第 14 节推翻）

> ⚠️ 本节的"全部修复"结论已被第 14 节推翻：H2 和 M4 经修复后验证为误判，已撤销修复。

| 状态 | 数量 |
|---|---|
| ✅ 完全修复 | 15（全部） |
| ⚠️ 占位修复 | 0 |
| ❌ 未修复 | 0 |
| 💡 可选改进（非阻塞） | 1（L4） |
| **合计** | **15** |

### 13.4 ESLint 配置审查

新增的 `eslint.config.js` 配置合理：

- ✅ 使用 flat config 格式（ESLint 10 兼容）
- ✅ 仅对 `src-ui/**/*.ts` / `src-ui/**/*.tsx` 生效
- ✅ 启用 `@typescript-eslint/recommended` 规则集
- ✅ 启用 `react-hooks` 推荐规则（`rules-of-hooks` / `exhaustive-deps`）
- ✅ `no-unused-vars` 设为 warn 并忽略 `^_` 前缀参数
- ✅ `no-explicit-any` 设为 warn（不阻塞但会提示）
- ⚠️ 关闭了 `react-hooks/refs` 和 `react-hooks/set-state-in-effect` 两条规则——可接受，但建议在代码稳定后重新评估是否开启

### 13.5 结论（第二轮，后被第 14 节推翻）

> ⚠️ 本节结论已被第 14 节推翻。

~~第二轮修复后，本审计报告所列 15 个问题已全部完全修复，四项构建/测试/lint 命令均通过。审计闭环。~~

---

## 14. 第三轮：误判撤销与 Bug 修复（2026-06-28）

第二轮修复后，开发人员报告出现两个 Bug。经调查，**两个 Bug 均由第一/二轮的误判修复引起**，现撤销相关修复。

### 14.1 Bug 1: map widget 不渲染

**现象**：replay 模式下，map widget 的赛道地图和赛车位置都不渲染。

**根因**：M4（误判）删除了 `control.trackId || "monza"` 的 fallback。当 `control.trackId` 为 `null` 时（dashboard 设计器创建 map widget 的默认值，见 `DashboardDesignerView.tsx:204` 及测试 `DashboardDesignerView.test.tsx:173`），`prefetchTrackMaps` 跳过加载、`MapWidget` 的 `effectiveTrackId` 为空字符串，`points` 恒为 `undefined`，effect 提前 return，什么都不画。

**项目约定**：`"monza"` 是 acc-coch 项目级 fallback，`DashboardDesignerView.tsx:1550-1551` 和 `LocalDashboardView.tsx:170` 都用 `control.trackId || "monza"`。审计时只看子模块内部未查 acc-coch 约定，误判为硬编码错误。

**修复**：恢复三处 `"monza"` fallback：
- `dashboardRenderer.tsx:534` — `trackId || "monza"`
- `useDashboardMetadata.ts:124` — `control.trackId || "monza"`
- `useDashboardMetadata.ts:138` — `c.trackId ?? "monza"`

### 14.2 Bug 2: 稀疏数据导致 text widget 闪烁

**现象**：overlay 中显示的数据一会是正常值，一会是 `--` 或 `NaN`。

**根因**：H2（误判）移除了 `fullFrameRef` 的字段累积逻辑。`poll_dashboard_frame` 返回的是**稀疏帧**——每帧只含本帧发生变化的字段，不含未变化字段。不做累积合并时，未出现在当前帧的字段变 `undefined`，text widget 走 `--`/`NaN` 分支。字段在下一次变化时才恢复，造成闪烁。

**正确语义**：`fullFrameRef.current = { ...fullFrameRef.current, ...frame.values }` 是让 text widget 在字段未变化时保持上一次有效值的必要机制。`handleClear` 仍会在 `poll_dashboard_frame` 返回 `null`（live session 结束/清空信号）时重置 `fullFrameRef`，不存在"永久残留"问题。

**修复**：恢复 `useDashboardFrame.ts` 的 `fullFrameRef` 累积逻辑：
- `pushFrame`：`fullFrameRef.current = { ...fullFrameRef.current, ...frame.values }`，`values: { ...fullFrameRef.current }`
- `handleClear`：`fullFrameRef.current = {}`

### 14.3 撤销后的构建验证

```
$ npm run typecheck   → ✅ 通过
$ npm test            → ✅ 2 files, 41 tests passed
$ npm run lint        → ✅ 通过（0 error, 0 warning）
$ cargo test          → ✅ 7 passed
```

### 14.4 最终问题统计（修正后）

| 严重度 | 有效数量 | 说明 |
|---|---|---|
| Critical | 1 | C3 |
| High | 2 | H1、H3（H2 为误判，撤销） |
| Medium | 3 | M1、M5、M6（M4 为误判，撤销） |
| Low | 7 | L1-L7 |
| **合计** | **13** | 原 15 减去 2 个误判 |

### 14.5 最终修复状态

| 状态 | 数量 |
|---|---|
| ✅ 完全修复 | 11 |
| ✅ 真正修复（第二轮 ESLint） | 1（L3） |
| ❌ 误判 — 已撤销 | 2（H2、M4） |
| 💡 可选改进（非阻塞） | 1（L4） |
| **合计** | **15**（有效问题 13，全部已修复或为改进建议） |

### 14.6 误判原因复盘

两条误判的共同原因：**审计时只看子模块内部代码，未验证 acc-coch 主模块的数据契约**。

| 误判 | 根因 |
|---|---|
| H2 | 不知道 `poll_dashboard_frame` 返回稀疏帧。累积合并是应对稀疏帧的必要机制，不是 bug。 |
| M4 | 不知道 `"monza"` 是 acc-coch 项目级 fallback 约定。设计器创建 map widget 时 `trackId` 默认 `null`，整个项目统一用 `\|\| "monza"` 兜底。 |

**教训**：子模块审计必须交叉验证主模块的数据契约和约定，不能只看子模块内部代码就下结论。

### 14.7 结论

误判撤销后，本审计报告有效问题 13 个，全部已修复或为非阻塞改进建议。四项构建/测试/lint 命令均通过。审计闭环。
