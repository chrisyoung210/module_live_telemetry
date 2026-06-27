# 仪表盘 overlay 数据绑定链路审计

> 类型：架构审计
>
> 审计日期：2026-06-27
>
> 涉及改动模块：`acc-coach`、`module_dashboard_protocol`、`module_local_dashboard`
>
> 参考模块（不改动）：`module_live_telemetry`（仅按既有规范格式产出数据，不受本次重构约束，详见 `channel-naming-convention.md` 第一节）

---

## 一、问题描述

Dashboard overlay 渲染时，所有 text widget 显示默认值 `"--"`，无法绑定到任何遥测数据。

调用 `list_registered_dashboard_layouts` 返回的布局数据中，所有 text widget 的 `telemetryField` 和 `textTemplate` 均为 `null`：

```json
{
  "id": "speed",
  "widgetType": "text",
  "telemetryField": null,
  "textTemplate": null
}
```

附带问题：frame 推送的 key 与别名表不一致（如 `speed_mph` vs `speed_kmh`、下划线 vs 点号）。

---

## 二、根因分析

### 2.1 编辑器未填充 `telemetryField`

`src-ui/components/DashboardDesignerView.tsx` 中：
- `makeControl()`（第 145 行）创建控件时 `telemetryField: null`
- `chooseTelemetryItem()`（第 1847 行）用户在遥测选择器中选中通道后，仅往 `text` 注入 token（`{speedKmh}`），**未调用 `updateSelected({ telemetryField: channelId })`**

### 2.2 字段名 schema 不一致

| 层面 | 字段名 | 值 |
|------|--------|-----|
| 编辑器 TypeScript | `text`, `textFormat` | `"{speedKmh}"`、`null` |
| Rust 序列化 | `text`, `textFormat` | 同编辑器 |
| 协议类型 | `textTemplate`, `format` | 读取不到 → `null` |
| overlay 渲染 | `textTemplate`, `format` | `null` → 显示 `"--"` |

链路由 `text` → `textTemplate` 断裂。

### 2.3 别名逻辑散布

别名翻译逻辑出现在以下所有位置：
- `src/recording/writer.rs`：正反两张硬编码别名表
- `src/dashboard/output.rs`：手工构造用户 key
- `src/ipc/mod.rs`：`.or_else()` 回退查两种 key
- `src/recording/auto.rs`：`dashboard_value()` 多 key 回退
- `module_local_dashboard/fieldNameMap.ts`：独立维护的 `FIELD_ALIASES` 表
- `module_dashboard_protocol`：`normalizeControl()` 蛇形/驼峰回退

20+ 处代码同时感知两种命名约定。违反单一职责原则。

---

## 三、解决方案概要

### 原则

1. **一个模块，一种格式**：别名层之外，任何代码只看到一种命名格式
2. **协议为尊**：`module_dashboard_protocol` 定义唯一类型契约
3. **单向依赖**：上层依赖协议，不反向

### 架构目标

```
module_live_telemetry
    ↓ 规范格式 (raw:controls.speed_kmh)
    ↓
alias.rs (唯一翻译点)
    ↓ 用户格式 (speedKmh)
    ↓
acc-coach 全部代码 + module_dashboard_protocol + module_local_dashboard
```

### 各模块变更

| 模块 | 变化量 | 核心改动 |
|------|--------|----------|
| `module_dashboard_protocol` | 中 | 补齐 `DashboardControl` 字段、重命名 `ChartFieldConfig`、删除兼容函数 |
| `acc-coach` | 大 | 新建别名层、Rust 字段重命名、5 个模块去双感知、编辑器修复 |
| `module_local_dashboard` | 小 | 删除别名表、删除 normalize 调用 |

### 相关文档

| 文档 | 读者 | 位置 |
|------|------|------|
| 通道命名规范 | 所有模块 | `public-protocol/channel-naming-convention.md` |
| 别名中间层设计 | acc-coach | `public-protocol/channel-alias-layer.md` |
| 协议类型对齐需求 | module_dashboard_protocol | `prd/module-dashboard-protocol-type-alignment.md` |
| 清理需求 | module_local_dashboard | `prd/module-local-dashboard-cleanup.md` |
