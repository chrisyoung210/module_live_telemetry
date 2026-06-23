# Module Live Telemetry — 文档导航

`module_live_telemetry` 是一个 Rust 库 + CLI 工具，用于 Assetto Corsa Competizione (ACC) 的实时遥测数据采集、录制、回放与实时 Dashboard 数据分发。

---

## 文档结构

```
docs/module_live_telemetry/
├── README.md                              ← 你在这里
├── api/                                   ← API 参考（面向 m1 调用方）
│   ├── recording.md                       ← RecordingController — 启动/停止录制服务
│   ├── calculated-item.md                 ← Calculated Item — 自定义计算项注册与执行
│   ├── extract-telemetry.md               ← Lap Telemetry 提取 — 从文件中提取圈数据
│   ├── import.md                          ← 文件导入 — parse_acctlm_file()
│   ├── lap-completed-callback.md          ← 圈完成回调 — LapCompletedEvent
│   └── raw-item.md                        ← Raw Item — TelemetryFrame 字段自动映射
├── format/                                ← 二进制文件格式规范
│   ├── binary-design.md                   ← 格式设计理念与架构
│   ├── v1-acctlm-format-spec.md           ← V1 格式规范 (`.acctlm`, chunked cluster)
│   ├── v2-acctlm2-format-spec.md          ← V2 格式规范 (`.acctlm2`, columnar row-group)
│   └── lap-reading.md                     ← 如何从二进制文件中读取圈速
├── reference/                             ← 参考资料
│   ├── raw-telemetry-fields.md            ← ACC Shared Memory 原始字段映射
│   ├── sector-calculated-items.md         ← 扇区计算项详细逻辑
│   └── computed-telemetry-logic.md        ← ⚠️ 已过期 — 旧 CLI 计算逻辑（仅历史参考）
└── audits/                                ← 审计报告（历史记录）
    ├── 2026-06-03.md
    ├── 2026-06-06-computed-items.md
    └── 2026-06-09-post-b6f8436-audit.md
```

---

## 快速导航

### 我要...

| 目标 | 文档 |
|------|------|
| 启动/停止录制服务 | [api/recording.md](api/recording.md) |
| 自定义遥测计算逻辑 | [api/calculated-item.md](api/calculated-item.md) |
| 从录制文件中提取圈数据 | [api/extract-telemetry.md](api/extract-telemetry.md) |
| 处理圈完成事件 | [api/lap-completed-callback.md](api/lap-completed-callback.md) |
| 了解可用的 Raw 遥测字段 | [api/raw-item.md](api/raw-item.md) + [reference/raw-telemetry-fields.md](reference/raw-telemetry-fields.md) |
| 理解 `.acctlm2` 文件格式 | [format/v2-acctlm2-format-spec.md](format/v2-acctlm2-format-spec.md) |
| 理解旧 `.acctlm` 文件格式 | [format/v1-acctlm-format-spec.md](format/v1-acctlm-format-spec.md) |
| 了解扇区计算项逻辑 | [reference/sector-calculated-items.md](reference/sector-calculated-items.md) |
| 查看审计/历史问题 | [audits/](audits/) |

---

## 架构概览

```
┌─────────────────────────────────────────────────────────────┐
│                       m1 (调用方)                            │
│  通过同步 API + channel 与 m2 通信                           │
└─────────────┬───────────────────────────────────────────────┘
              │ RecordingController::start()
              ▼
┌─────────────────────────────────────────────────────────────┐
│                  RecordingController                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │ Recording    │  │ Dashboard    │  │ Compute          │  │
│  │ Engine       │  │ Service      │  │ Registry         │  │
│  │ (holder线程) │  │ (独立线程)   │  │ (Realtime/Batch) │  │
│  └──────┬───────┘  └──────┬───────┘  └────────┬─────────┘  │
│         │                 │                    │            │
│  ┌──────▼───────┐  ┌──────▼───────┐            │            │
│  │ ACC Shared   │  │ Distributor  │◄───────────┘            │
│  │ Memory       │  │ (Arc fan-out)│                         │
│  └──────────────┘  └──────┬───────┘                         │
│                           │                                  │
│                    ┌──────▼───────┐                          │
│                    │ V2 Writer    │                          │
│                    │ (.acctlm2)   │                          │
│                    └──────────────┘                          │
└─────────────────────────────────────────────────────────────┘
```

**核心概念**:
- **Item 体系**: `{raw|calc|system}:{path}` 统一标识，三种类型共存
- **Raw Item**: 直接从 `TelemetryFrame` 读取字段，无需注册
- **Calculated Item**: 自定义计算逻辑，通过 `ComputeRegistry` 注册
- **RecordingController**: 长期录制服务，管理 holder 线程生命周期
- **Dashboard Service**: 按 item 独立频率调度，输出 `DashboardValuesFrame`
- **二进制格式**: V1 (`.acctlm`, chunked) 和 V2 (`.acctlm2`, columnar + skip-index) 两代格式共存

---

## 依赖

```toml
[dependencies]
crossbeam-channel = "0.5"
crc32fast = "1"
ctrlc = "3.4"
memmap2 = "0.9"
```

---

## 相关项目文档

- `docs/acc-coach/` — ACC Coach 集成文档
- `docs/acctlm_core/` — 核心格式文档
- `docs/centra/` — Centra 集成文档
- `docs/ld_to_acctlm/` — Local Dashboard → acctlm 转换
- `docs/module_local_dashboard/` — Local Dashboard 模块文档
