# Replay Mock Telemetry — 基于 acctlm2 文件回放模拟实时遥测

## TL;DR

> **Quick Summary**: 新增从 acctlm2 文件回放遥测数据的功能，通过 `RecordingController::start_replay()` 暴露，支持可配置倍速，喂 dashboard + 触发 lap 回调 + 发送状态通知，但不写入任何文件。用于方便测试 dashboard 相关功能。
>
> **Deliverables**:
> - `ReplayRequest` 类型 + `validate()` (src/recording/request.rs)
> - `ReplayTelemetrySource` 实现 `TelemetrySource` trait (src/recording/source.rs)
> - `run_replay_loop` 函数 — 无文件写入的回放循环 (src/recording/engine.rs)
> - `RecordingController::start_replay()` + `start_replay_with_latest_dashboard()` (src/recording/controller.rs)
> - `StopReason::FramesExhausted` + `RecordingStatus::ReplayStarted` 枚举变体 (src/recording/status.rs)
> - 模块导出 (src/recording/mod.rs)
> - 全套单元测试 + 集成测试
>
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 3 waves
> **Critical Path**: Task 1/3 → Task 4 → Task 5 → F1-F4

---

## Context

### Original Request
用户计划增加一个功能，根据一个 acctlm2 文件来 mock 实时遥测数据，主要用于方便测试 dashboard 相关功能。让 `RecordingController::start()` 的调用方真实地收到数据，是 `start()` 所实现功能的一个子集：只有返回 dashboard item 这一项内容，可能还包括状态回调、lap 数据等等，但没有真实的录制，数据来源是 acctlm2 文件。

### Interview Summary
**Key Discussions**:
- **回放时序**: 可配置倍速 — 支持 `speed_multiplier` 参数 (1.0=实时, 2.0=2倍速, 0.5=慢放)
- **API 形式**: `RecordingController` 新方法 `start_replay()`，签名与 `start()` 相似
- **帧耗尽行为**: 停止并通知 — 发送 `Stopping(FramesExhausted)` 状态后退出
- **交付范围**: Rust 库 API + 单元测试，不含 CLI 命令

**Research Findings**:
- `TelemetrySource` trait (src/recording/source.rs:16) 是数据源抽象，真实实现 `AccTelemetrySource` 读共享内存，测试中已有 `ScriptedTelemetrySource`/`PauseResumeSource` mock
- `run_recording_loop` (src/recording/engine.rs:105) 与文件写入 (`BinaryTelemetryWriterV2`) 紧耦合，处理状态轮询 + lap 检测 + dashboard 分发 + 文件写入
- `RecordingController` (src/recording/controller.rs:54) `start()` 是构造函数，启动 holder 线程 → `setup_dashboard_thread` → `run_recording_loop`
- `BinaryTelemetryReader` (src/reader.rs) 读取 acctlm2 文件，有 `read_all_frames()` → `Vec<TelemetryFrame>`, `metadata()` (含 `poll_hz`), `summary()`, `read_lap_frames()`
- Dashboard 流: `TelemetryDistributor` → `DashboardService` (独立线程) → 计算值 → `DashboardValuesFrame` 到 sink
- `RecordingRequest` (src/recording/request.rs:11) 有 `poll_hz`, `output_dir`, `status_interval`, `dashboard_items`, `dashboard_realtime_items`

### Metis Review
**Identified Gaps** (addressed):
- **Q1 完成信号**: 通过 status channel 发送 `Stopping(FramesExhausted)`，无单独完成 channel
- **Q2 状态变体**: 复用 `Started`/`Running`/`Stopping`/`Error`；跳过 `Connected`/`WaitingForSharedMemory`/`Paused`/`RecordingStarted`；新增 `ReplayStarted`
- **Q3 Lap 检测源**: 使用 `completed_laps` delta 检测（与 engine 一致）
- **Q4 latest_dashboard 变体**: 提供 `start_replay_with_latest_dashboard()`
- **Q5 并发安全**: `start_replay()` 是构造函数返回 `Self`，与 `start()` 一样创建独立实例，无并发问题
- **Q6 时间戳**: 覆盖 `frame.sample_tick`/`timestamp_ns` 为回放计数值（与 engine 一致）
- **Q7 非均匀帧间隔**: MVP 用均匀步调 (poll_hz * speed)
- **Q8 测试文件**: 测试时用 `BinaryTelemetryWriterV2` 生成临时 acctlm2 文件
- **D1-D5 决策**: 全部以合理默认值解决，详见 Guardrails

---

## Work Objectives

### Core Objective
从 acctlm2 文件回放遥测数据，通过 `RecordingController::start_replay()` 暴露给调用方，使调用方收到 dashboard 数据 + 状态回调 + lap 回调，但不写入任何文件。

### Concrete Deliverables
- `ReplayRequest` 类型 (file_path, speed_multiplier, status_interval, dashboard_items, dashboard_realtime_items)
- `ReplayTelemetrySource` 实现 `TelemetrySource` trait
- `run_replay_loop` 函数 (无文件写入)
- `RecordingController::start_replay()` + `start_replay_with_latest_dashboard()`
- `StopReason::FramesExhausted` + `RecordingStatus::ReplayStarted`
- 模块导出
- 单元测试 + 集成测试

### Definition of Done
- [ ] `cargo build` 无错误
- [ ] `cargo test` 全部通过
- [ ] `cargo clippy` 无警告
- [ ] `RecordingController::start_replay()` 能从 acctlm2 文件回放，调用方收到 dashboard 数据
- [ ] 倍速回放 (2x) 完成时间约为 1x 的一半
- [ ] 帧耗尽后发送 `Stopping(FramesExhausted)` 并干净退出
- [ ] Lap 完成回调正确触发
- [ ] `stop()` 能中途停止回放

### Must Have
- `ReplayRequest` 新类型，不含 `output_dir`（与 `RecordingRequest` 区分）
- `ReplayTelemetrySource` 实现 `TelemetrySource` trait，从 acctlm2 文件读帧
- `run_replay_loop` 不创建 `BinaryTelemetryWriterV2`，不写任何文件
- `start_replay()` 签名与 `start()` 对称（无 `outcome_tx`，因为无录制结果）
- 倍速控制: `speed_multiplier > 0.0 && is_finite()`
- 状态序列: `Started → ReplayStarted → Running → Stopping(FramesExhausted|Manual)`
- Lap 检测: `completed_laps` delta（与 `run_recording_loop` 一致）
- Lap 回调在 dashboard 分发之后触发（匹配 engine.rs:272-297 的执行顺序语义）
- `try_send` 发送状态（非阻塞，与 engine 一致）
- `frame.sample_tick`/`timestamp_ns` 覆盖为回放计数值

### Must NOT Have (Guardrails)
- **MUST NOT** 修改 `TelemetrySource` trait 定义
- **MUST NOT** 修改现有 `run_recording_loop` 函数
- **MUST NOT** 创建 `BinaryTelemetryWriterV2`（零文件输出）
- **MUST NOT** 修改 `RecordingRequest` 或其 `validate()`
- **MUST NOT** 添加 CLI 命令
- **MUST NOT** 发送 `RecordingOutcome`
- **MUST NOT** 支持 pause/resume
- **MUST NOT** 支持 seek/skip（从指定 lap 开始）
- **MUST NOT** 支持多同时回放
- **MUST NOT** 支持循环重复回放
- **MUST NOT** 发送 `Connected`/`WaitingForSharedMemory`/`Paused`/`RecordingStarted` 状态（ACC 专用，回放无意义）
- **MUST NOT** 添加过度抽象或 JSDoc 式注释（避免 AI slop）

---

## Verification Strategy (MANDATORY)

> **ZERO HUMAN INTERVENTION** - ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (cargo test, Rust 内置 `#[test]`)
- **Automated tests**: YES (tests-after) — 每个任务实现后添加单元测试
- **Framework**: cargo test (Rust built-in)
- **Test data**: 测试时用 `BinaryTelemetryWriterV2` 生成临时 acctlm2 文件

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.omo/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Library/Module**: Use Bash (cargo test / cargo build) — 编译, 运行测试, 验证输出
- **API behavior**: Use Bash (cargo test with specific test names) — 验证状态序列, lap 回调, 倍速等

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately - foundation, 3 parallel tasks):
├── Task 1: status.rs — StopReason::FramesExhausted + RecordingStatus::ReplayStarted [quick]
├── Task 2: request.rs — ReplayRequest type + validate() [quick]
└── Task 3: source.rs — ReplayTelemetrySource + TelemetrySource impl [quick]

Wave 2 (After Wave 1 - core replay loop, depends on 1+3):
└── Task 4: engine.rs — run_replay_loop function [deep]

Wave 3 (After Wave 2 - controller wiring, depends on 1+2+4):
└── Task 5: controller.rs + mod.rs — start_replay() + exports [unspecified-high]

Wave FINAL (After ALL tasks — 4 parallel reviews):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real manual QA (unspecified-high)
└── Task F4: Scope fidelity check (deep)
-> Present results -> Get explicit user okay

Critical Path: Task 1 → Task 4 → Task 5 → F1-F4
Parallel Speedup: ~40% faster than sequential (Wave 1 parallelizes 3 tasks)
Max Concurrent: 4 (Wave FINAL)
```

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1 | - | 4, 5 |
| 2 | - | 5 |
| 3 | - | 4 |
| 4 | 1, 3 | 5 |
| 5 | 1, 2, 4 | F1-F4 |
| F1 | 5 | - |
| F2 | 5 | - |
| F3 | 5 | - |
| F4 | 5 | - |

### Agent Dispatch Summary

- **Wave 1**: **3** — T1 → `quick`, T2 → `quick`, T3 → `quick`
- **Wave 2**: **1** — T4 → `deep`
- **Wave 3**: **1** — T5 → `unspecified-high`
- **FINAL**: **4** — F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

- [x] 1. 新增 StopReason::FramesExhausted + RecordingStatus::ReplayStarted 枚举变体

  **What to do**:
  - 在 `src/recording/status.rs` 的 `StopReason` 枚举中新增 `FramesExhausted` 变体（表示回放文件帧已耗尽）
  - 在 `src/recording/status.rs` 的 `RecordingStatus` 枚举中新增 `ReplayStarted` 变体（单元变体，类似 `RecordingStarted`，表示回放数据流已开始）
  - 更新 `StopReason` 的 `Display` impl，添加 `FramesExhausted => write!(f, "frames exhausted")`
  - 更新 `RecordingStatus` 的 `Display` impl（如果存在），添加 `ReplayStarted` 分支
  - 添加单元测试：验证新变体的 `Display` 输出、`PartialEq` 比较

  **Must NOT do**:
  - 不修改现有枚举变体
  - 不添加 `ReplayRunning` 变体（复用现有 `Running`，`bytes_written` 设为 0）
  - 不添加 CLI 相关代码

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 单文件修改，添加两个枚举变体 + Display impl + 测试，范围明确
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `git-master`: 不涉及 git 操作

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3)
  - **Blocks**: Tasks 4, 5
  - **Blocked By**: None (can start immediately)

  **References** (CRITICAL):

  **Pattern References** (existing code to follow):
  - `src/recording/status.rs:67-74` — `StopReason` 枚举定义，新增 `FramesExhausted` 变体在此追加
  - `src/recording/status.rs:76-84` — `StopReason` 的 `Display` impl，新增分支在此追加
  - `src/recording/status.rs:8-45` — `RecordingStatus` 枚举定义，新增 `ReplayStarted` 变体在此追加
  - `src/recording/status.rs:20` — `RecordingStarted` 单元变体，`ReplayStarted` 应采用相同形式

  **Test References** (testing patterns to follow):
  - `src/recording/status.rs:99-158` — 现有测试模块，新测试按此模式添加

  **WHY Each Reference Matters**:
  - `StopReason` 枚举: 需要在此追加 `FramesExhausted`，保持枚举顺序和风格一致
  - `Display` impl: 每个变体都需要 Display 实现，`FramesExhausted` 也不例外
  - `RecordingStarted`: `ReplayStarted` 应模仿其形式（单元变体，无字段）
  - 测试模块: 现有测试展示了如何测试枚举变体的 Display 输出

  **Acceptance Criteria**:

  **QA Scenarios (MANDATORY):**

  ```
  Scenario: 新枚举变体编译通过且 Display 正确
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 运行 `cargo build` — 确认编译无错误
      2. 运行 `cargo test status` — 确认 status 相关测试通过
      3. 在测试中验证 `StopReason::FramesExhausted.to_string()` 输出 "frames exhausted"
      4. 在测试中验证 `RecordingStatus::ReplayStarted` 可以被构造和 Debug 打印
    Expected Result: cargo build 成功，cargo test status 全部通过，Display 输出正确
    Failure Indicators: 编译错误，测试失败，Display 输出不匹配
    Evidence: .omo/evidence/task-1-enum-variants.txt

  Scenario: 现有枚举变体不受影响
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 运行 `cargo test status::tests` — 确认所有现有 status 测试仍通过
      2. 检查现有变体 (Manual, SessionEnd, ShmemLost) 的 Display 仍正确
    Expected Result: 所有现有测试通过，无回归
    Failure Indicators: 任何现有测试失败
    Evidence: .omo/evidence/task-1-no-regression.txt
  ```

  **Commit**: YES
  - Message: `feat(replay): add FramesExhausted stop reason and ReplayStarted status`
  - Files: `src/recording/status.rs`
  - Pre-commit: `cargo test status`

- [x] 2. 新增 ReplayRequest 类型 + validate()

  **What to do**:
  - 在 `src/recording/request.rs` 中新增 `ReplayRequest` 结构体，字段:
    - `file_path: PathBuf` — acctlm2 文件路径
    - `speed_multiplier: f64` — 回放倍速 (1.0=实时, 2.0=2倍速, 0.5=慢放)
    - `status_interval: Duration` — 状态回调间隔
    - `dashboard_items: Vec<DashboardItemSubscription>` — dashboard 订阅项
    - `dashboard_realtime_items: Vec<DashboardRealtimeItemRegistration>` — 自定义实时计算项
  - 添加 `#[derive(Debug, Clone)]`
  - 实现 `ReplayRequest::validate()` 方法，验证:
    - `file_path` 指向的文件存在且是文件 (`file_path.is_file()`)
    - `speed_multiplier > 0.0 && speed_multiplier.is_finite()`
    - `status_interval` 非零
    - `dashboard_realtime_items` 名称非空且唯一（复用 `RecordingRequest::validate()` 中的同名逻辑模式）
  - 添加单元测试：有效请求通过验证、各种无效请求被拒绝 (speed=0, speed=NaN, speed=Inf, 文件不存在, status_interval=0)

  **Must NOT do**:
  - 不修改现有 `RecordingRequest` 或其 `validate()`
  - 不添加 `output_dir` 字段（回放不写文件）
  - 不添加 `poll_hz` 字段（从文件 metadata 获取）
  - 不添加 CLI 相关代码

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 单文件修改，新增一个结构体 + validate 方法 + 测试，与现有 `RecordingRequest` 模式对称
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `git-master`: 不涉及 git 操作

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3)
  - **Blocks**: Task 5
  - **Blocked By**: None (can start immediately)

  **References** (CRITICAL):

  **Pattern References** (existing code to follow):
  - `src/recording/request.rs:8-24` — `RecordingRequest` 结构体定义，`ReplayRequest` 应模仿其 `#[derive]` 和字段风格
  - `src/recording/request.rs:26-85` — `RecordingRequest::validate()` 实现模式，`ReplayRequest::validate()` 应模仿其验证风格 (逐项检查，返回 `TelemetryError::InvalidArgument`)
  - `src/recording/request.rs:61-82` — `dashboard_realtime_items` 验证逻辑（名称非空、唯一、name 匹配），可直接复用此模式

  **API/Type References** (contracts to implement against):
  - `src/recording/mod.rs:43-49` — `DashboardItemSubscription`, `DashboardRealtimeItemRegistration` 的 re-export，确认这些类型可在此模块使用
  - `src/error.rs:6-11` — `TelemetryError::InvalidArgument(String)` 用于验证失败

  **Test References** (testing patterns to follow):
  - `src/recording/request.rs:88-157` — `RecordingRequest` 测试模块，`ReplayRequest` 测试应模仿其模式 (valid_request helper + 各 invalid 场景)

  **WHY Each Reference Matters**:
  - `RecordingRequest` 结构体: `ReplayRequest` 应与其风格对称，但用 `file_path` + `speed_multiplier` 替代 `output_dir` + `poll_hz`
  - `validate()` 模式: 复用相同的逐项检查 + `InvalidArgument` 返回模式
  - `dashboard_realtime_items` 验证: 这段逻辑与录制完全相同，应直接复用模式
  - 测试模式: 现有测试展示了如何构建有效/无效请求并断言

  **Acceptance Criteria**:

  **QA Scenarios (MANDATORY):**

  ```
  Scenario: 有效 ReplayRequest 通过验证
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 在测试中创建 `ReplayRequest { file_path: 指向存在的文件, speed_multiplier: 1.0, status_interval: Duration::from_secs(1), dashboard_items: vec![], dashboard_realtime_items: vec![] }`
      2. 调用 `validate()` — 确认返回 `Ok(())`
      3. 运行 `cargo test replay_request` — 确认测试通过
    Expected Result: validate() 返回 Ok(()), cargo test 通过
    Failure Indicators: validate() 返回 Err, 测试失败
    Evidence: .omo/evidence/task-2-valid-request.txt

  Scenario: 无效 speed_multiplier 被拒绝
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 测试 speed_multiplier=0.0 → validate() 应返回 Err
      2. 测试 speed_multiplier=-1.0 → validate() 应返回 Err
      3. 测试 speed_multiplier=f64::NAN → validate() 应返回 Err
      4. 测试 speed_multiplier=f64::INFINITY → validate() 应返回 Err
      5. 运行 `cargo test replay_request` — 确认所有 invalid 测试通过
    Expected Result: 所有无效 speed_multiplier 均被 validate() 拒绝
    Failure Indicators: 任何无效值通过验证
    Evidence: .omo/evidence/task-2-invalid-speed.txt

  Scenario: 文件不存在被拒绝
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 测试 file_path 指向不存在的路径 → validate() 应返回 Err
      2. 运行 `cargo test replay_request` — 确认测试通过
    Expected Result: 不存在的文件路径被 validate() 拒绝
    Failure Indicators: 不存在的路径通过验证
    Evidence: .omo/evidence/task-2-invalid-path.txt
  ```

  **Commit**: YES
  - Message: `feat(replay): add ReplayRequest type with validation`
  - Files: `src/recording/request.rs`
  - Pre-commit: `cargo test replay_request`

- [x] 3. 新增 ReplayTelemetrySource — 实现 TelemetrySource trait

  **What to do**:
  - 在 `src/recording/source.rs` 中新增 `ReplayTelemetrySource` 结构体:
    - 内部持有 `frames: VecDeque<TelemetryFrame>` — 从 acctlm2 文件预加载的帧队列
    - 内部持有 `metadata: SessionMetadata` — 文件元数据 (track_name, car_model, poll_hz 等)
    - 内部持有 `raw_static_bytes: Vec<u8>` — 原始静态页字节
    - 内部持有 `session_info: AccSessionInfo` — 从 metadata 构建的会话信息
  - 实现 `ReplayTelemetrySource::open(file_path: impl AsRef<Path>) -> TelemetryResult<Self>`:
    - 用 `BinaryTelemetryReader::open(file_path)` 打开文件
    - 调用 `reader.read_all_frames()` 预加载所有帧到 `VecDeque`
    - 获取 `metadata()` 和 `raw_static_bytes`
    - 从 metadata 构建 `AccSessionInfo { track_name, car_model }`
  - 实现 `TelemetrySource` trait for `ReplayTelemetrySource`:
    - `status()`: 第一帧前返回 `AccGameStatus::Live`，帧耗尽后返回 `AccGameStatus::Off`
    - `session_info()`: 返回从 metadata 构建的 `AccSessionInfo`
    - `read_static_bytes()`: 返回预加载的 `raw_static_bytes`
    - `read_telemetry_frame(sample_tick, timestamp_ns)`: 弹出下一帧，覆盖 `frame.sample_tick = sample_tick`, `frame.timestamp_ns = timestamp_ns`，返回 `Ok(Some(frame))`；帧耗尽返回 `Ok(None)`
    - `read_all_telemetry_frame(sample_tick, timestamp_ns)`: 同 `read_telemetry_frame`（回放无去重需求，每次调用都返回一帧）
  - 确认 `ReplayTelemetrySource: Send`（`VecDeque<TelemetryFrame>`, `SessionMetadata`, `Vec<u8>`, `AccSessionInfo` 均为 `Send`）
  - 添加单元测试：打开有效文件并读取帧、帧耗尽返回 None、空文件处理

  **Must NOT do**:
  - 不修改 `TelemetrySource` trait 定义
  - 不修改 `AccTelemetrySource`
  - 不添加流式读取（MVP 用 `read_all_frames()` 全量加载）
  - 不添加 CLI 相关代码

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 单文件修改，新增一个结构体 + trait 实现 + 测试，与现有 `AccTelemetrySource` 模式对称
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `git-master`: 不涉及 git 操作

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2)
  - **Blocks**: Task 4
  - **Blocked By**: None (can start immediately)

  **References** (CRITICAL):

  **Pattern References** (existing code to follow):
  - `src/recording/source.rs:61-107` — `AccTelemetrySource` 结构体 + `TelemetrySource` impl，`ReplayTelemetrySource` 应模仿此模式
  - `src/recording/source.rs:16-54` — `TelemetrySource` trait 定义，5 个方法签名
  - `src/recording/source.rs:65-73` — `AccTelemetrySource::open()` 模式，`ReplayTelemetrySource::open()` 应模仿

  **API/Type References** (contracts to implement against):
  - `src/reader.rs:96-113` — `BinaryTelemetryReader::open()` 公共 API，自动检测 V1/V2 格式
  - `src/reader.rs:478-483` — `BinaryTelemetryReader::read_all_frames()` → `Vec<TelemetryFrame>`
  - `src/reader.rs:191-196` — `BinaryTelemetryReader::metadata()` → `&SessionMetadata`
  - `src/shmem.rs` — `AccGameStatus::Live`, `AccGameStatus::Off`, `AccSessionInfo` 定义
  - `src/types.rs` — `SessionMetadata` 结构体 (含 `track_name`, `car_model`, `poll_hz`, `raw_static_bytes` 等字段)

  **Test References** (testing patterns to follow):
  - `src/recording/engine.rs:429-488` — `PauseResumeSource` 测试 mock，展示了如何实现 `TelemetrySource` 的所有方法
  - `src/recording/engine.rs:385-411` — `make_frame()` helper，可在测试中构建帧
  - `src/reader_v2.rs:1776-1793` — reader 测试模式

  **WHY Each Reference Matters**:
  - `AccTelemetrySource`: `ReplayTelemetrySource` 的兄弟实现，应模仿其结构体定义和 trait impl 风格
  - `TelemetrySource` trait: 必须实现这 5 个方法，签名不能偏离
  - `BinaryTelemetryReader::open/read_all_frames/metadata`: 这是读取 acctlm2 文件的公共 API
  - `AccGameStatus`: `status()` 需要返回此类型，`Live` 表示有帧可读，`Off` 表示帧耗尽
  - `PauseResumeSource`: 测试中已有的 mock 实现，展示了 `read_telemetry_frame`/`read_all_telemetry_frame` 的返回模式
  - `SessionMetadata`: 需要从中获取 `track_name`, `car_model`, `raw_static_bytes` 构建 source 内部状态

  **Acceptance Criteria**:

  **QA Scenarios (MANDATORY):**

  ```
  Scenario: ReplayTelemetrySource 从 acctlm2 文件加载帧
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 在测试中用 BinaryTelemetryWriterV2 生成一个临时 acctlm2 文件 (3帧)
      2. 调用 `ReplayTelemetrySource::open(&tmp_file)` — 确认返回 Ok
      3. 调用 `source.status()` — 确认返回 Ok(AccGameStatus::Live)
      4. 调用 `source.read_all_telemetry_frame(0, 0)` — 确认返回 Ok(Some(frame)), frame.sample_tick == 0
      5. 调用 `source.read_all_telemetry_frame(1, 1)` — 确认返回 Ok(Some(frame)), frame.sample_tick == 1
      6. 调用 `source.read_all_telemetry_frame(2, 2)` — 确认返回 Ok(Some(frame)), frame.sample_tick == 2
      7. 调用 `source.read_all_telemetry_frame(3, 3)` — 确认返回 Ok(None) (帧耗尽)
      8. 调用 `source.status()` — 确认返回 Ok(AccGameStatus::Off)
      9. 运行 `cargo test replay_source` — 确认测试通过
    Expected Result: 3帧依次读取成功，第4次返回 None，status 从 Live 变为 Off
    Failure Indicators: open 失败，帧数不匹配，status 不正确
    Evidence: .omo/evidence/task-3-load-frames.txt

  Scenario: session_info 和 static_bytes 正确返回
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 在测试中生成临时 acctlm2 文件，metadata 含 track_name="test_track", car_model="test_car"
      2. 调用 `ReplayTelemetrySource::open(&tmp_file)` 
      3. 调用 `source.session_info()` — 确认 track_name=="test_track", car_model=="test_car"
      4. 调用 `source.read_static_bytes()` — 确认返回 Ok(Vec<u8>)
      5. 运行 `cargo test replay_source` — 确认测试通过
    Expected Result: session_info 返回正确的 track/car，static_bytes 返回原始字节
    Failure Indicators: track_name 或 car_model 不匹配
    Evidence: .omo/evidence/task-3-session-info.txt

  Scenario: 不存在的文件返回 Err
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 调用 `ReplayTelemetrySource::open("nonexistent.acctlm2")` — 确认返回 Err
      2. 运行 `cargo test replay_source` — 确认测试通过
    Expected Result: open 返回 Err (TelemetryError::Io)
    Failure Indicators: open 对不存在的文件返回 Ok
    Evidence: .omo/evidence/task-3-invalid-file.txt
  ```

  **Commit**: YES
  - Message: `feat(replay): add ReplayTelemetrySource impl`
  - Files: `src/recording/source.rs`
  - Pre-commit: `cargo test replay_source`

- [x] 4. 新增 run_replay_loop 函数 — 无文件写入的回放循环

  **What to do**:
  - 在 `src/recording/engine.rs` 中新增 `run_replay_loop` 函数 (放在 `run_recording_loop` 之后):
    ```
    pub fn run_replay_loop(
        speed_multiplier: f64,
        source: &mut impl TelemetrySource,
        poll_hz: f64,
        dashboard_distributor: Option<&TelemetryDistributor>,
        stop_rx: Receiver<()>,
        lap_completed: Option<&LapCompletedCallback>,
        status_tx: Option<&StatusSender>,
    ) -> TelemetryResult<StopReason>
    ```
  - 循环逻辑 (参考 `run_recording_loop` 但移除文件写入):
    - 计算 `poll_interval = 1.0 / (poll_hz * speed_multiplier)` — 基础速率乘以倍速
    - 初始化 lap 检测状态: `last_completed_laps: Option<u32>`, `current_lap_frames: Vec<TelemetryFrame>`, `current_lap_is_valid = true`
    - 发送 `RecordingStatus::ReplayStarted` (通过 status_tx)
    - 主循环:
      1. 检查 `stop_rx.try_recv()` — 收到停止信号则发送 `Stopping(Manual)` 并返回 `Ok(StopReason::Manual)`
      2. 调用 `source.status()` — 如果 `Off` (帧耗尽)，发送 `Stopping(FramesExhausted)` 并返回 `Ok(StopReason::FramesExhausted)`
      3. 计算 `timestamp_ns` = replay 开始后的纳秒数 (与 engine.rs:263-266 一致)
      4. 调用 `source.read_all_telemetry_frame(sample_tick, timestamp_ns)`:
         - `Ok(Some(frame))`:
           a. Lap 检测 (复制 engine.rs:273-291 逻辑): 比较 `completed_laps` delta, 触发 `LapCompletedCallback`
           b. Dashboard 分发: `dist.distribute(Arc::clone(&frame_arc))` (与 engine.rs:295-297 一致)
           c. `sample_tick += 1`
           d. 定期发送 `Running { sample_count, bytes_written: 0, elapsed, fps }` (按 status_interval 间隔)
         - `Ok(None)`: 帧耗尽 — 发送 `Stopping(FramesExhausted)` 并返回
         - `Err(err)`: 发送 `Error` 状态并返回 `Err(err)`
      5. `sleep_remaining(tick_start, poll_interval)` — 控制回放速率
  - 确保 lap 回调在 dashboard 分发之后触发 (匹配 engine.rs 的语义: 当前帧先分发到 dashboard，然后检测是否完成 lap)
  - 使用 `send_status` helper (复用 engine.rs 中现有的 `send_status` 函数)
  - 添加单元测试:
    - 基本回放: 3帧文件 → 验证状态序列 (ReplayStarted → Running → Stopping(FramesExhausted))
    - Lap 回调: 用 `make_lap_frame` 构建含 lap 完成的帧序列 → 验证回调触发
    - 倍速: 验证 speed_multiplier=10.0 时回放速度加快 (用时少于 1x)
    - 停止信号: 回放中发送 stop → 验证 Stopping(Manual)

  **Must NOT do**:
  - 不修改现有 `run_recording_loop` 函数
  - 不创建 `BinaryTelemetryWriterV2` 或任何文件写入
  - 不发送 `RecordingStarted`/`Connected`/`WaitingForSharedMemory`/`Paused` 状态
  - 不发送 `RecordingOutcome`
  - 不支持 pause/resume
  - 不支持 seek/skip

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 核心循环逻辑，需要仔细复制 lap 检测 + dashboard 分发模式，同时确保不引入文件写入。逻辑复杂度高，需要深度思考。
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `git-master`: 不涉及 git 操作
    - `debugging`: 不涉及运行时调试

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 (sequential, single task)
  - **Blocks**: Task 5
  - **Blocked By**: Task 1 (需要 ReplayStarted + FramesExhausted 枚举变体), Task 3 (需要 ReplayTelemetrySource)

  **References** (CRITICAL):

  **Pattern References** (existing code to follow):
  - `src/recording/engine.rs:105-354` — `run_recording_loop` 完整实现，`run_replay_loop` 应参考其循环结构、lap 检测、dashboard 分发、状态发送模式
  - `src/recording/engine.rs:132-158` — stop 信号检查模式
  - `src/recording/engine.rs:268-304` — 帧读取 + lap 检测 + dashboard 分发的核心逻辑，`run_replay_loop` 应复制此模式但移除 `writer.write_frame()` 调用 (engine.rs:300-302)
  - `src/recording/engine.rs:273-291` — lap 完成检测逻辑: `completed_laps` delta 比较 + `LapCompletedEvent::new()` + `current_lap_frames` 积累 + `current_lap_is_valid` AND 累积
  - `src/recording/engine.rs:295-297` — dashboard 分发: `dist.distribute(Arc::clone(&frame_arc))`
  - `src/recording/engine.rs:352-353` — `sleep_remaining(tick_start, poll_interval)` 速率控制模式
  - `src/recording/engine.rs:365-367` — `send_status` helper 函数 (在 engine.rs 内定义，可直接复用)

  **API/Type References** (contracts to implement against):
  - `src/recording/status.rs:8-45` — `RecordingStatus` 枚举，需使用 `ReplayStarted` (Task 1 新增), `Running`, `Stopping`, `Error` 变体
  - `src/recording/status.rs:67-74` — `StopReason` 枚举，需使用 `FramesExhausted` (Task 1 新增), `Manual` 变体
  - `src/recording/engine.rs:37-52` — `LapCompletedEvent`, `LapCompletedCallback` 类型
  - `src/recording/source.rs:16-54` — `TelemetrySource` trait (由 `ReplayTelemetrySource` 实现)

  **Test References** (testing patterns to follow):
  - `src/recording/engine.rs:490-541` — `test_engine_emits_running_when_pause_resumes_live` — 测试模式: 构建 source + config + channels → 调用 loop → 收集 status → 断言
  - `src/recording/engine.rs:543-597` — `test_engine_records_fake_source` — 测试模式: ScriptedTelemetrySource + 验证结果
  - `src/recording/engine.rs:599-665` — `test_first_completed_valid_lap_is_emitted_as_reference_candidate` — lap 回调测试模式: Arc<Mutex<Vec>> 收集事件 + 断言 lap_number/is_valid/lap_time_ms
  - `src/recording/engine.rs:667-722` — `test_engine_stop_signal` — 停止信号测试模式: 单独线程发送 stop + 验证 StopReason
  - `src/recording/engine.rs:385-427` — `make_frame()` 和 `make_lap_frame()` helpers，可在回放测试中复用

  **WHY Each Reference Matters**:
  - `run_recording_loop` 完整实现: `run_replay_loop` 是其"无文件写入"版本，必须匹配其循环结构、lap 检测、dashboard 分发模式
  - 帧读取核心逻辑 (268-304): 这是最关键的参考 — 需要复制 lap 检测 + dashboard 分发，但移除 `writer.write_frame()` (300-302)
  - lap 检测逻辑 (273-291): 必须精确复制 `completed_laps` delta 比较和 `current_lap_frames` 积累逻辑
  - `sleep_remaining`: 回放速率控制的关键 — `poll_interval = 1.0 / (poll_hz * speed_multiplier)`
  - `send_status` helper: 已在 engine.rs 中定义，可直接调用
  - 测试模式: 现有 engine 测试展示了如何构建 source/config/channels、调用 loop、收集 status、断言结果

  **Acceptance Criteria**:

  **QA Scenarios (MANDATORY):**

  ```
  Scenario: 基本回放循环 — 帧耗尽后干净退出
    Tool: Bash (cargo)
    Preconditions: Task 1, 3 已完成，项目可编译
    Steps:
      1. 在测试中用 BinaryTelemetryWriterV2 生成临时 acctlm2 文件 (3帧, poll_hz=1000)
      2. 用 ReplayTelemetrySource::open 加载文件
      3. 创建 status_channel(16) 和 stop channel
      4. 调用 run_replay_loop(speed_multiplier=100.0, &mut source, 1000.0, None, stop_rx, None, Some(&status_tx))
      5. 收集所有 status: statuses = status_rx.iter().collect()
      6. 断言 statuses[0] == ReplayStarted
      7. 断言最后一个 status == Stopping { reason: StopReason::FramesExhausted }
      8. 运行 `cargo test replay_loop` — 确认测试通过
    Expected Result: 状态序列以 ReplayStarted 开始, Stopping(FramesExhausted) 结束
    Failure Indicators: 缺少 ReplayStarted, 帧耗尽后不发送 Stopping, panic
    Evidence: .omo/evidence/task-4-basic-replay.txt

  Scenario: Lap 完成回调正确触发
    Tool: Bash (cargo)
    Preconditions: Task 1, 3 已完成，项目可编译
    Steps:
      1. 在测试中用 make_lap_frame 构建 4帧序列: lap0 pos0.10, lap0 pos0.90, lap1 pos0.01 (completed_laps 0→1), Off
      2. 用 ReplayTelemetrySource 加载 (或直接用 ScriptedTelemetrySource 包装这些帧)
      3. 创建 LapCompletedCallback 用 Arc<Mutex<Vec<LapCompletedEvent>>> 收集事件
      4. 调用 run_replay_loop(speed_multiplier=1000.0, ..., lap_completed=Some(&callback), ...)
      5. 断言 events.len() == 1
      6. 断言 events[0].lap_number == 1
      7. 断言 events[0].is_valid == true
      8. 断言 events[0].lap_frames.len() == 2 (lap 0 的两帧)
      9. 运行 `cargo test replay_loop` — 确认测试通过
    Expected Result: 1 个 lap 回调触发, lap_number=1, 2帧
    Failure Indicators: 回调未触发, lap_number 错误, 帧数不匹配
    Evidence: .omo/evidence/task-4-lap-callback.txt

  Scenario: 倍速回放 — 2x 速度快于 1x
    Tool: Bash (cargo)
    Preconditions: Task 1, 3 已完成，项目可编译
    Steps:
      1. 生成 100帧 acctlm2 文件 (poll_hz=100)
      2. 用 Instant::now() 计时 1x 回放: run_replay_loop(1.0, ..., 100.0, ...)
      3. 记录 elapsed_1x
      4. 重新加载文件, 计时 2x 回放: run_replay_loop(2.0, ..., 100.0, ...)
      5. 记录 elapsed_2x
      6. 断言 elapsed_2x < elapsed_1x * 0.75 (2x 至少快 25%)
      7. 运行 `cargo test replay_loop_speed` — 确认测试通过
    Expected Result: 2x 回放时间显著短于 1x
    Failure Indicators: 2x 不比 1x 快, 速度无差异
    Evidence: .omo/evidence/task-4-speed-multiplier.txt

  Scenario: 停止信号中断回放
    Tool: Bash (cargo)
    Preconditions: Task 1, 3 已完成，项目可编译
    Steps:
      1. 生成 1000帧 acctlm2 文件 (poll_hz=100)
      2. 创建 stop channel, 在单独线程中 sleep(50ms) 后发送 stop 信号
      3. 调用 run_replay_loop(1.0, ..., stop_rx, ...)
      4. 断言返回值 == Ok(StopReason::Manual)
      5. 断言最后一个 status == Stopping { reason: StopReason::Manual }
      6. 运行 `cargo test replay_loop_stop` — 确认测试通过
    Expected Result: 收到停止信号后立即停止, 返回 Manual
    Failure Indicators: 不响应停止信号, 返回 FramesExhausted 而非 Manual
    Evidence: .omo/evidence/task-4-stop-signal.txt
  ```

  **Commit**: YES
  - Message: `feat(replay): add run_replay_loop without file writing`
  - Files: `src/recording/engine.rs`
  - Pre-commit: `cargo test replay_loop`

- [x] 5. 新增 RecordingController::start_replay() + start_replay_with_latest_dashboard() + 模块导出

  **What to do**:
  - 在 `src/recording/controller.rs` 中新增两个关联函数 (构造函数，与 `start()` 对称):

    **`start_replay()`**:
    ```
    pub fn start_replay(
        request: ReplayRequest,
        status_tx: StatusSender,
        dashboard_tx: Option<Sender<DashboardValuesFrame>>,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<Self>
    ```
    - 调用 `request.validate()?`
    - 创建 stop channel: `bounded::<()>(1)`
    - 创建 dash_cmd channel: `bounded::<DashboardCommand>(16)`
    - 初始化 `stopped`, `dash_handle`, `dashboard_items`, `dashboard_calculated_items`, `dashboard_stats` (与 `start_with_output` 一致)
    - 打开 ReplayTelemetrySource: `ReplayTelemetrySource::open(&request.file_path)?`
    - 获取文件 metadata 的 `poll_hz`
    - spawn holder 线程 (name="replay-holder"):
      - 发送 `RecordingStatus::Started { thread_id: 0 }`
      - 调用 `setup_dashboard_thread()` (复用现有函数，与 `run_recording_holder` 一致)
      - 构建 `LapCompletedCallback` (复用 `build_lap_completed_callback`)
      - 调用 `run_replay_loop(request.speed_multiplier, &mut source, poll_hz, Some(&distributor), stop_rx, Some(&lap_completed), Some(&status_tx))`
      - 处理返回的 `StopReason` (日志输出，不需要发 outcome)
    - 返回 `Self { stop_tx, dash_cmd_tx, handle, dash_handle, stopped, dashboard_items, dashboard_calculated_items, dashboard_stats }`

    **`start_replay_with_latest_dashboard()`**:
    ```
    pub fn start_replay_with_latest_dashboard(
        request: ReplayRequest,
        status_tx: StatusSender,
        lap_completed: Option<LapCompletedCallback>,
    ) -> TelemetryResult<(Self, LatestValueReceiver)>
    ```
    - 与 `start_with_latest_dashboard()` 对称: 创建 `latest_value_channel()`, 调用 `start_replay` 内部逻辑 (DashboardOutput::Latest), 返回 `(controller, receiver)`

  - 在 `src/recording/mod.rs` 中添加 re-exports:
    - `pub use request::ReplayRequest;` (在现有 `pub use request::RecordingRequest;` 旁)
    - `pub use source::ReplayTelemetrySource;` (如果有 re-export source 类型的模式)
  - 添加集成测试:
    - 端到端: 生成 acctlm2 文件 → start_replay → 收到 dashboard 数据 → 收到 lap 回调 → Stopping(FramesExhausted)
    - start_replay_with_latest_dashboard: 验证 LatestValueReceiver 收到数据
    - stop() 中途停止: start_replay → sleep → stop() → 验证干净退出
    - 无效请求: 不存在的文件 → start_replay 返回 Err

  **Must NOT do**:
  - 不修改现有 `start()`/`start_with_output()`/`start_with_latest_dashboard()` 方法
  - 不添加 `outcome_tx` 参数 (回放无录制结果)
  - 不添加 CLI 命令
  - 不发送 `RecordingOutcome`
  - 不添加 `output_dir` 到 `ReplayRequest`

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: 多文件集成任务 (controller.rs + mod.rs)，需要理解现有 controller 初始化模式并精确复制，同时编写端到端集成测试。工作量较大但模式明确。
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `git-master`: 不涉及 git 操作
    - `visual-engineering`: 不涉及 UI

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (sequential, single task)
  - **Blocks**: F1, F2, F3, F4
  - **Blocked By**: Task 1 (ReplayStarted + FramesExhausted), Task 2 (ReplayRequest), Task 4 (run_replay_loop)

  **References** (CRITICAL):

  **Pattern References** (existing code to follow):
  - `src/recording/controller.rs:90-104` — `start()` 公共 API 签名和实现，`start_replay()` 应模仿其结构
  - `src/recording/controller.rs:110-125` — `start_with_latest_dashboard()` 实现，`start_replay_with_latest_dashboard()` 应模仿
  - `src/recording/controller.rs:127-179` — `start_with_output()` 核心初始化逻辑 (创建 channels, spawn 线程, 返回 Self)，`start_replay` 内部应模仿此模式
  - `src/recording/controller.rs:144-167` — `std::thread::Builder::new().name(...).spawn(...)` 模式
  - `src/recording/controller.rs:367-497` — `run_recording_holder()` 函数，回放 holder 应模仿其结构: 发送 Started → setup_dashboard_thread → build_lap_completed_callback → 调用 loop
  - `src/recording/controller.rs:632-738` — `setup_dashboard_thread()` 函数 (可直接复用，接受 `&RecordingRequest`，需要适配 `ReplayRequest` 或提取公共字段)
  - `src/recording/controller.rs:574-607` — `build_lap_completed_callback()` 函数 (可直接复用)

  **API/Type References** (contracts to implement against):
  - `src/recording/request.rs` — `ReplayRequest` (Task 2 新增), `RecordingRequest` (现有，参考其字段)
  - `src/recording/source.rs` — `ReplayTelemetrySource::open()` (Task 3 新增)
  - `src/recording/engine.rs` — `run_replay_loop()` (Task 4 新增), `LapCompletedCallback` 类型
  - `src/recording/status.rs` — `RecordingStatus::ReplayStarted` (Task 1 新增), `StopReason::FramesExhausted` (Task 1 新增)
  - `src/dashboard/sink.rs` — `latest_value_channel()`, `LatestValueReceiver`, `LatestValueSender`
  - `src/recording/controller.rs:72-75` — `DashboardOutput` 枚举 (Legacy/Latest)，回放也需此枚举

  **Test References** (testing patterns to follow):
  - `src/recording/controller.rs:950-971` — `test_start_validates_request`, `test_start_valid_request_spawns_holder` — controller 测试模式
  - `src/recording/controller.rs:923-937` — `test_frame()` helper
  - `src/recording/controller.rs:903-911` — `unique_dir()` helper

  **WHY Each Reference Matters**:
  - `start()` / `start_with_output()`: `start_replay()` 是其回放版本，必须匹配其构造函数模式: 创建 channels → spawn 线程 → 返回 Self
  - `run_recording_holder()`: 回放 holder 线程应模仿其结构: 发送 Started → setup_dashboard_thread → build_lap_completed_callback → 调用 loop (但调用 run_replay_loop 而非 run_recording_loop)
  - `setup_dashboard_thread()`: 此函数接受 `&RecordingRequest`，需要确认是否可直接用于 `ReplayRequest` (两者都有 `dashboard_items` 和 `dashboard_realtime_items`)。如果不能直接复用，需要提取公共参数或创建回放专用的 setup 函数
  - `build_lap_completed_callback()`: 可直接复用，接受 `dash_cmd_tx` + `session_best_tracker` + external callback
  - `start_with_latest_dashboard()`: `start_replay_with_latest_dashboard()` 应模仿其 `latest_value_channel()` + 返回 `(Self, receiver)` 模式
  - mod.rs re-exports: 确保新类型通过 `module_live_telemetry::recording::ReplayRequest` 等路径可访问

  **Acceptance Criteria**:

  **QA Scenarios (MANDATORY):**

  ```
  Scenario: 端到端回放 — 调用方收到 dashboard 数据 + lap 回调 + 停止通知
    Tool: Bash (cargo)
    Preconditions: Tasks 1-4 已完成，项目可编译
    Steps:
      1. 在测试中用 BinaryTelemetryWriterV2 生成 3-lap acctlm2 文件 (每lap 60帧, poll_hz=1000)
      2. 创建 status_channel(32), outcome_channel() (不用, 但确认无 outcome 发送)
      3. 创建 latest_value_channel() 获取 dashboard receiver
      4. 创建 LapCompletedCallback 用 Arc<AtomicU32> 计数
      5. 构建 ReplayRequest { file_path, speed_multiplier: 100.0, status_interval: Duration::from_millis(10), dashboard_items: vec![DashboardItemSubscription { item_name: "raw:controls.speed_kmh".into(), ... }], ... }
      6. 调用 RecordingController::start_replay_with_latest_dashboard(request, status_tx, lap_cb)
      7. 从 LatestValueReceiver 接收数据 — 确认收到至少1个 DashboardValuesFrame, values 包含 "raw:controls.speed_kmh"
      8. 等待回放结束 (status_rx 收到 Stopping)
      9. 断言 lap_count == 3 (3个 lap 回调)
      10. 断言最后一个 status == Stopping { reason: StopReason::FramesExhausted }
      11. 断言 outcome_rx 为空 (无 RecordingOutcome 发送)
      12. 运行 `cargo test start_replay` — 确认测试通过
    Expected Result: dashboard 数据收到, 3个 lap 回调, Stopping(FramesExhausted), 无 outcome
    Failure Indicators: 无 dashboard 数据, lap 回调数量错误, 发送了 outcome
    Evidence: .omo/evidence/task-5-e2e-replay.txt

  Scenario: stop() 中途停止回放
    Tool: Bash (cargo)
    Preconditions: Tasks 1-4 已完成，项目可编译
    Steps:
      1. 生成 1000帧 acctlm2 文件 (poll_hz=100)
      2. 调用 start_replay_with speed_multiplier=1.0
      3. sleep(100ms) 后调用 controller.stop()
      4. 确认 stop() 返回 (线程 join 完成)
      5. 断言最后一个 status == Stopping { reason: StopReason::Manual }
      6. 运行 `cargo test start_replay_stop` — 确认测试通过
    Expected Result: stop() 干净停止, Stopping(Manual)
    Failure Indicators: stop() 阻塞, status 不是 Manual
    Evidence: .omo/evidence/task-5-stop-mid-replay.txt

  Scenario: 无效文件路径返回 Err
    Tool: Bash (cargo)
    Preconditions: Tasks 1-4 已完成，项目可编译
    Steps:
      1. 构建 ReplayRequest { file_path: "nonexistent.acctlm2".into(), ... }
      2. 调用 RecordingController::start_replay(request, status_tx, None, None)
      3. 断言返回 Err
      4. 运行 `cargo test start_replay_invalid` — 确认测试通过
    Expected Result: 不存在的文件返回 Err
    Failure Indicators: 返回 Ok
    Evidence: .omo/evidence/task-5-invalid-file.txt

  Scenario: 模块导出可访问
    Tool: Bash (cargo)
    Preconditions: 项目可编译
    Steps:
      1. 运行 `cargo build` — 确认无错误
      2. 在测试中验证 `module_live_telemetry::recording::ReplayRequest` 可访问
      3. 运行 `cargo test replay` — 确认所有 replay 测试通过
    Expected Result: 所有新类型通过公共路径可访问
    Failure Indicators: 编译错误, 类型不可访问
    Evidence: .omo/evidence/task-5-exports.txt
  ```

  **Commit**: YES
  - Message: `feat(replay): add RecordingController::start_replay methods`
  - Files: `src/recording/controller.rs`, `src/recording/mod.rs`
  - Pre-commit: `cargo test replay`

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .omo/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo build` + `cargo clippy` + `cargo test`. Review all changed files for: `as any`/`unwrap()` in non-test code, empty catches, println! in prod (eprintln! for logging is OK matching existing pattern), commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction, generic names.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [x] F3. **Real Manual QA** — `unspecified-high`
  Start from clean state. Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Test cross-task integration: start_replay → receive dashboard data → receive lap callbacks → stop. Test edge cases: empty file, speed=2x timing, stop() mid-replay. Save to `.omo/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [x] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (git log/diff). Verify 1:1 — everything in spec was built (no missing), nothing beyond spec was built (no creep). Check "Must NOT do" compliance. Detect cross-task contamination: Task N touching Task M's files. Flag unaccounted changes. Verify no CLI command added, no RecordingOutcome sent, no run_recording_loop modified, no TelemetrySource trait modified.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **1**: `feat(replay): add FramesExhausted stop reason and ReplayStarted status` - src/recording/status.rs
- **2**: `feat(replay): add ReplayRequest type with validation` - src/recording/request.rs
- **3**: `feat(replay): add ReplayTelemetrySource impl` - src/recording/source.rs
- **4**: `feat(replay): add run_replay_loop without file writing` - src/recording/engine.rs
- **5**: `feat(replay): add RecordingController::start_replay methods` - src/recording/controller.rs, src/recording/mod.rs

---

## Success Criteria

### Verification Commands
```bash
cargo build                    # Expected: no errors
cargo test                     # Expected: all tests pass
cargo clippy                   # Expected: no warnings
cargo test replay              # Expected: all replay-related tests pass
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] `cargo build` + `cargo clippy` clean
