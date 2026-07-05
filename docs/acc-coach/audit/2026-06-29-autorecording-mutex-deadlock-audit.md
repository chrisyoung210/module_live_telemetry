# ACC Coach 自动录制 session 不显示 bug 审计与修复方案

- 审计日期：2026-06-29
- 审计基线：`8179427`（最近一次关闭审计 commit）+ 工作区未提交变更（`src/recording/auto.rs`）
- 问题报告：开始游戏 session 后，磁盘存在生成的 `.acctlm2` 文件，但 session 页面不显示对应条目；手动触发导入可以正常导入并显示。
- 结论：存在 1 个 **CRITICAL**（Mutex 重入死锁，直接导致 outcome 丢失）+ 2 个 HIGH（DB 并发健壮性、前端无自动刷新）问题。
- **关闭状态：✅ 已修复（`8e4a2fe`）**

## 审计背景

用户报告：自动录制完成后，`.acctlm2` 文件存在于磁盘（`~/.acc_coach/data/`），但 session 页面（`DataSummaryView`）不显示对应条目。手动点击"导入 ACCTLM"选择同一文件可以成功导入并显示，说明：

1. 文件格式正确，`parse_acctlm_file` 能解析；
2. `persist_acctlm_recording_outcome` 入库逻辑正确；
3. 问题出在 **自动录制的入库链路**，而非文件生成或入库函数本身。

此 bug 跨越多个 commit（`f613ee4` → `ba6f6d1` → `8179427` → 工作区未提交改动），期间已多次尝试修复（包括从 `select!` 改为优先级排空循环），但均未命中根因。

## 证据链

### 1. 调试日志分析

Rust 端 `debug_log`（`src/utils/mod.rs:11-21`）写入 `%TEMP%\acc_coach_steering_debug.log`。当前日志共 2568 行，关键统计：

| 关键字 | 出现次数 | 含义 |
|---|---|---|
| `drain: status: RecordingStarted` | 2 | 录制开始状态被 drain 循环接收 |
| `drain: status: Running` | **0** | 录制期间 Running 状态（仅 pause→resume 时发送，为 0 正常） |
| `drain: status: Stopping` | **0** | **录制结束状态未被接收** |
| `drain: received outcome` | **0** | **outcome 未被 drain 循环接收** |
| `persist_outcome` | **0** | **入库函数从未被调用** |
| `parse_acctlm_file outcome` | 4 | 来自**手动导入**（`session.rs:65`），非自动录制 |

**关键观察**：最后一次 `drain: status: RecordingStarted` 出现在第 2538 行，之后直到日志末尾（第 2568 行）**没有任何 `drain:` 消息**。drain 循环在处理 `RecordingStarted` 状态后永久冻结。

日志末尾的 `parse_acctlm_file outcome` 是用户手动导入时产生（走 `import_acctlm_file` IPC → `session.rs:65`），不走 drain 循环，因此能成功。

### 2. 日志事件序列

```
... (大量 WaitingForSharedMemory)
[ACC_COACH] drain: status: Connected
[ACC_COACH] drain: status: RecordingStarted    ← 最后一条 drain 消息
[ACC_COACH] parse_acctlm_file outcome for ...   ← 手动导入，非 drain
[ACC_COACH] acctlm summary: ... laps=2
[ACC_COACH] acctlm outcome debug: RecordingOutcome { ... }
```

`RecordingStarted` 之后 drain 循环完全停止，没有任何后续状态（`Stopping`/`Error`）或 outcome 处理。

### 3. 文件状态

- 磁盘文件存在：`mclaren_720s_gt3_evo_monza_1782738449.acctlm2`（20,487,604 字节）
- DB 中无对应条目（`persist_outcome` 从未执行）
- 手动导入该文件成功 → 入库逻辑本身正确

## 根因分析

### CRITICAL-1：`auto.rs` status Mutex 重入死锁

**位置**：`src/recording/auto.rs:559-573`

```rust
// auto.rs:559-573 (当前代码)
if is_recording_started || is_running {
    let current = status.lock().ok();        // ← 锁定 status (MutexGuard 存活到 573)
    let need_meta = current
        .as_ref()
        .map(|s| s.track_name.is_none())
        .unwrap_or(true);
    if need_meta {
        if let Some(meta) = ctrl.session_metadata() {
            set_status(&status, |s| {         // ← auto.rs:567 再次 status.lock() → 死锁!
                s.track_name = Some(meta.track_name);
                s.car_model = Some(meta.car_model);
            });
        }
    }
}                                             // ← current 的 MutexGuard 到此才释放
```

**`set_status` 实现**（`auto.rs:1094-1101`）：

```rust
fn set_status(
    status: &Arc<Mutex<AutoRecordingStatus>>,
    update: impl FnOnce(&mut AutoRecordingStatus),
) {
    if let Ok(mut status) = status.lock() {   // ← 第二次 lock 同一个 Mutex
        update(&mut status);
    }
}
```

**死锁机制**：

1. `auto.rs:560` `status.lock().ok()` 获取 `MutexGuard`，赋值给 `current`；
2. `current` 的生命周期延续到 `if` 块末尾（573 行）；
3. `auto.rs:567` `set_status(&status, ...)` 内部再次调用 `status.lock()`；
4. `std::sync::Mutex` **不可重入** — 同一线程第二次 `lock()` 会**永久阻塞**；
5. drain 循环线程冻结，永远无法处理后续的 `Stopping` 状态和 `RecordingOutcome`。

**触发条件**：`is_recording_started == true`（首次 `RecordingStarted` 时）且 `track_name` 为 `None`（首次录制时必然为 `None`）。因此**每次首次录制必死锁**。

**为什么 `select!` 版本也失败**：这段代码在状态消息**处理**阶段，不在通道**选择**阶段。从 `select!` 改为优先级排空只改了选择逻辑，没有触及死锁代码。

### 引入历史

- `f613ee4`（track map 自动采集功能）：引入 `need_meta` 检查和 `ctrl.session_metadata()` 调用，**首次引入死锁**。
- `ba6f6d1`（重构）：保留这段代码，死锁持续。
- `8179427`（关闭审计修复）：未触及此代码。
- 工作区未提交改动：从 `select!` 改为优先级排空 + 添加调试日志，**仍未修复死锁**（死锁在状态处理代码，不在通道选择代码）。

### HIGH-1：DB 并发健壮性不足

**位置**：`src/db/mod.rs:15-22`

```rust
pub fn open(path: &Path) -> SqlResult<Self> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(Self { conn, path: Some(path.to_path_buf()) })
}
```

**问题**：

1. **无 WAL 模式**：默认 `journal_mode=DELETE`，写入时阻塞所有读连接。`persist_outcome`（录制线程，独立 DB 连接）写入时，IPC 的 `query_data_entries`（主线程 DB 连接）会被阻塞。
2. **无 `busy_timeout`**：锁竞争时立即返回 `SQLITE_BUSY` 错误，而非等待。两个连接同时写入时可能失败。

**当前两条 DB 写入路径**：

- 自动录制：`persist_outcome` → `Database::open(db_path)` → 独立连接，无事务包裹
- 手动导入：`import_acctlm_file` → `db.lock().await` → 共享连接，`immediate_transaction` 包裹

自动录制路径缺少事务包裹，若 `persist_acctlm_recording_outcome` 中途失败（如 lap 插入后 data_entry 插入失败），会留下孤立的 session/laps 记录。

### HIGH-2：前端无自动刷新机制

**位置**：`src-ui/components/DataSummaryView.tsx:348-417`

`fetchData` 只在以下情况触发：
- 组件挂载（`useEffect` 依赖 `fetchData`，388-417 行）
- 筛选条件变化（`sortBy`/`filterTrack` 等依赖变化）
- `syncResult` 变化（手动同步后，419-438 行）
- 手动导入 ACCTLM 后（`handleImportAcctlm`，623-640 行）

**缺失**：自动录制完成时，前端**不会收到任何通知**，`fetchData` 不会被触发。即使用户切换到 session 页面，若组件已挂载过且筛选未变，`fetchData` 不会重新执行（`hasLoadedEntriesRef` 为 `true` 时跳过 `setLoading`，但 `fetchData(false)` 仍会执行 — 需确认）。

**影响**：即使 CRITICAL-1 修复后 outcome 正常入库，用户仍需手动点击刷新按钮才能看到新 session。这不是当前 bug 的直接原因（死锁才是），但影响用户体验。

## 修复方案

### 修复 1：CRITICAL-1 — 修复 Mutex 重入死锁

**文件**：`src/recording/auto.rs`

**方案**：将 `current` 限制在内层作用域，确保 `MutexGuard` 在 `set_status` 调用前释放。

```rust
// 修复后 (auto.rs:559-573)
if is_recording_started || is_running {
    let need_meta = {
        let current = status.lock().ok();    // ← 锁定
        current
            .as_ref()
            .map(|s| s.track_name.is_none())
            .unwrap_or(true)
    };                                        // ← MutexGuard 释放
    if need_meta {
        if let Some(meta) = ctrl.session_metadata() {
            set_status(&status, |s| {         // ← 安全：锁已释放
                s.track_name = Some(meta.track_name);
                s.car_model = Some(meta.car_model);
            });
        }
    }
}
```

**验证**：

1. `cargo build` 编译通过；
2. 启动应用 → 开始 ACC session → 退出 session → 检查 `acc_coach_steering_debug.log`：
   - 应出现 `drain: status: Stopping`
   - 应出现 `drain: received outcome`
   - 应出现 `persist_outcome: track=..., file=...`
   - 应出现 `persist_outcome OK: entry_id=..., file=...`
3. session 页面应显示新录制条目（可能需要手动刷新一次，见 HIGH-2）。

### 修复 2：HIGH-1 — DB 健壮性改进

**文件**：`src/db/mod.rs`、`src/recording/session.rs`

**2a. `Database::open` 加 WAL + busy_timeout**：

```rust
// src/db/mod.rs:15-22 修复后
pub fn open(path: &Path) -> SqlResult<Self> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(Self { conn, path: Some(path.to_path_buf()) })
}
```

WAL 模式允许并发读写（读不阻塞写，写不阻塞读），`busy_timeout=5000ms` 使锁竞争时等待而非立即失败。

**2b. `recording_result_to_data_entry` 加事务包裹**：

```rust
// src/recording/session.rs:11-28 修复后
pub fn recording_result_to_data_entry(
    conn: &rusqlite::Connection,
    result: &RecordingResult,
) -> Result<DataEntry, String> {
    if is_acctlm_recording_path(&result.file_path) {
        let id = Uuid::new_v4().to_string();
        let entry = crate::db::immediate_transaction(conn, || {
            import_acctlm_data_entry(
                conn,
                &result.file_path,
                Some(id.clone()),
                Some(result.sha256_hex.clone()),
                Some(chrono::Utc::now().to_rfc3339()),
            )
        })?;
        return Ok(entry);
    }
    Err(format!("Unsupported recording format: {}", result.file_path.display()))
}
```

与手动导入 `import_acctlm_file`（`ipc/mod.rs:2080`）已有的事务包裹保持一致。

**验证**：`cargo build` + `cargo test` 通过。

### 修复 3：清理调试日志

**文件**：`src/recording/auto.rs`

移除工作区未提交改动中为排查此 bug 添加的 `crate::utils::debug_log("[ACC_COACH] ...")` 语句（共约 15 处），**保留**：

- 优先级排空结构（`'inner: loop` + 4 段 drain）— 它是合理的改进，防止高频 dashboard 帧饿死 outcome 通道；
- `session.rs` 中的 `log_acctlm_import_outcome` — 在 bug 之前就存在。

移除的日志位置：
- `auto.rs:378` outer loop iteration start
- `auto.rs:471-476` calling RecordingController::start
- `auto.rs:485` RecordingController::start OK
- `auto.rs:489-491` RecordingController::start FAILED
- `auto.rs:522` drain: received outcome
- `auto.rs:527` drain: outcome channel disconnected
- `auto.rs:541-544` drain: status: {:?}
- `auto.rs:577` drain: status channel disconnected
- `auto.rs:593` drain: stop command received
- `auto.rs:621` drain: command channel disconnected
- `auto.rs:657` drain: dashboard channel disconnected
- `auto.rs:1002-1009` persist_outcome: track/car/samples/laps/file
- `auto.rs:1019` persist_outcome FAILED (hash)
- `auto.rs:1032-1036` persist_outcome OK
- `auto.rs:1043-1046` persist_outcome SKIP (empty)
- `auto.rs:1057-1059` persist_outcome FAILED (db)

### 不修改的文件

- `module_live_telemetry/*` — 子模块，不修改（AGENTS.md 禁止）
- `module_local_dashboard/*` — 子模块，不修改
- `acctlm_core/*` — 子模块，不修改
- `ld_to_acctlm/*` — 子模块，不修改
- `src-ui/*` — 用户未选择前端自动刷新（HIGH-2 暂不修复）

## 验证清单

| 项 | 命令/操作 | 预期结果 |
|---|---|---|
| 编译 | `cargo build` | 通过 |
| 测试 | `cargo test` | 通过 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 不引入新 warning |
| 功能 | 开始 ACC session → 退出 → 检查日志 | 出现 `persist_outcome OK` |
| 功能 | 同上 → 检查 session 页面 | 显示新录制条目（可能需手动刷新一次） |
| 回归 | 手动导入 ACCTLM 文件 | 仍然成功 |
| 回归 | dashboard overlay 实时数据 | 不受影响 |

## 影响范围

- `src/recording/auto.rs` — 死锁修复 + 日志清理
- `src/db/mod.rs` — WAL + busy_timeout
- `src/recording/session.rs` — 事务包裹

不涉及子模块修改，不涉及前端修改，不涉及 IPC 协议变更。
