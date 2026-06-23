# ACC Coach 代码审计报告

- 审计日期：2026-06-13
- 审计版本：`76b9167df52ec4f9463d5c94672ca36c641895fd`
- 审计范围：当前仓库 `acc-coach` 的 Rust/Tauri 后端、React 前端、数据导入/同步、live recording、dashboard/overlay 相关代码。
- 复核结论：上一轮审计核心结论（P1 事务一致性、P2 锁范围和竞态、P3 攻击面和包体）仍然成立。本轮复核发现原审计存在若干行号偏移、遗漏和精度问题，同时补充了 6 项原审计未覆盖的新问题。

## 验证结果

- 初始审计时：`cargo test` 通过，`cmd /c npm run build` 通过；`cargo clippy --all-targets --all-features -- -D warnings` 和 `cmd /c npm run lint` 未通过。
- 修复后复验（2026-06-13）：`cmd /c npm run lint` 通过。
- 修复后复验（2026-06-13）：`cargo clippy --all-targets --all-features -- -D warnings` 通过。
- 修复后复验（2026-06-13）：`cargo check` 通过。
- 修复后复验（2026-06-13）：`cargo test` 通过，共 63 个库单元测试、1 个 e2e pipeline、1 个 lap stats、2 个 real LD、3 个 regression monza 测试通过。
- 修复后复验（2026-06-13）：`cmd /c npm run build` 通过。Vite 仍提示主 chunk `702.81 kB`，这是前端包体偏大的剩余 P3 问题，已按当前讨论暂缓处理。
## 问题清单

### P1：live/acctlm 重导入存在数据丢失风险

相关位置：

- `src/ipc/mod.rs:1505`
- `src/ipc/mod.rs:1774`
- `src/recording/session.rs:96`

问题说明：

live/acctlm 重导入路径会先调用 `remove_live_session_for_reimport` 删除旧 session、laps、bookmarks 和 analysis 关联，然后再执行新文件解析和写入。如果新文件解析失败、数据库写入中途失败，或进程异常退出，旧数据已经不可恢复。

影响：

- 用户重新导入同一 recording 时可能丢失已有圈速、书签和分析记录。
- 失败后界面可能只剩 data entry 或残缺 session 状态。

修复建议：

- 重导入流程改为事务化：在同一个 `BEGIN IMMEDIATE` 事务内删除旧 session、插入新 session/laps、更新 data entry、恢复 bookmark，失败时统一 rollback。
- 在删除旧数据之前先完成新文件解析，并把解析结果保存在内存结构中。
- 对空 recording、解析失败、hash 失败分别返回错误，不应进入删除旧 session 的阶段。
- 增加回归测试：构造已有 session，再模拟新文件解析失败，断言旧 session/laps/bookmarks 仍然存在。

### P1：目录同步 `.ld` 文件不是事务性的

相关位置：

- `src/sync/mod.rs:44`
- `src/sync/mod.rs:98`
- `src/sync/mod.rs:106`
- `src/sync/mod.rs:117`
- `src/sync/mod.rs:133`
- `src/sync/mod.rs:146`
- `src/sync/mod.rs:185`

问题说明：

`copy_new_files` 会先复制文件，再分步解析并写入 tracks、sessions、laps、data_entries。中途任一步失败都会把错误放进 `errors` 后继续处理下一个文件，但之前已经写入的 session/laps 不会回滚，复制出来的目标文件也不会清理。

影响：

- 数据库可能出现没有对应 data entry 的 session/laps。
- 目标 data 目录可能留下未登记或登记失败的文件。
- 后续查询、统计、重复导入判断可能被残留数据污染。

修复建议：

- 每个源文件单独使用数据库事务，tracks/sessions/laps/data_entries 任一步失败即 rollback。
- 文件复制可以放在事务前，但如果事务失败，应删除刚复制的目标文件。
- 或先解析源文件，确认可导入后再复制和写库。
- 增加测试：模拟 session 插入成功但 lap 或 data entry 插入失败，断言事务回滚且目标文件清理。

### P2：全局数据库锁覆盖大文件解析和分析计算

相关位置：

- `src/ipc/mod.rs:523`
- `src/ipc/mod.rs:528`
- `src/ipc/mod.rs:541`
- `src/ipc/mod.rs:552`
- `src/ipc/mod.rs:1734`
- `src/ipc/mod.rs:1798`

问题说明：

多个 Tauri command 在拿到全局 `Mutex<Database>` 后继续执行文件解析、telemetry 加载、lap 分析、目录同步等重操作。由于该锁是全局串行锁，一个慢操作会阻塞所有需要数据库的 IPC 请求。

影响：

- 大型 `.ld` 或 `.acctlm2` 文件解析时，界面其他数据请求会卡住。
- `analyze_laps` 读取两圈 telemetry 并计算时会阻塞其他 DB command。
- 目录同步大量文件时，数据摘要、书签、lap 状态更新等操作都会等待。

修复建议：

- 缩短锁持有范围：锁内只读取必要 row、session id、file path、track distance 等轻量信息。
- 释放 DB 锁后执行文件解析、telemetry 提取和分析计算。
- 需要写回报告或状态时，再短时间重新获取锁并用事务写入。
- 对同步导入可考虑后台任务队列或 `spawn_blocking`，并通过进度状态返回 UI。

### P2：telemetry cache 复制整圈数据，内存和 CPU 放大

相关位置：

- `src/ipc/mod.rs:68`
- `src/ipc/mod.rs:76`
- `src/ipc/mod.rs:340`
- `src/ipc/mod.rs:403`

问题说明：

`TelemetryCache::get` 在命中时 `.cloned()` 整个 `Vec<TelemetryPoint>`，`put` 前也会 `points.clone()`。每圈 telemetry 点数较多时，cache 命中依然会产生明显内存分配和复制成本。

影响：

- 长圈或高频采样数据会导致 IPC 响应期间额外内存峰值。
- cache 容量为 64 时，多个大圈缓存会放大驻留内存。
- 高频切换 lap 或进行对比分析时，CPU 时间消耗在复制上。

修复建议：

- 将 cache value 改为 `Arc<Vec<TelemetryPoint>>` 或 `Arc<[TelemetryPoint]>`。
- 内部分析路径尽量传引用或 `Arc`，只在 IPC 序列化边界按需拷贝。
- 为 cache 增加按字节估算的容量限制，而不是只按 lap 数限制。
- 增加基准测试：比较 cache hit 下 `Vec` clone 与 `Arc` clone 的耗时和内存。

当前修复状态（2026-06-13）：

- 已采用 `Arc<[TelemetryPoint]>` 作为 telemetry cache value，`TelemetryCache::get` 命中时只 clone `Arc` 指针，不再 clone 整圈 `Vec<TelemetryPoint>`。
- `TelemetryCache::put` 接收 `Arc<[TelemetryPoint]>`，miss 后新解析出的 telemetry 只在 `Vec -> Arc<[TelemetryPoint]>` 转换时移动/装箱一次，cache 保存共享只读数据。
- `TelemetryCache` 内部 `Mutex` 的 `.lock().unwrap()` 已改为 `unwrap_or_else(|poisoned| poisoned.into_inner())`，poisoned 场景下不再直接 panic。
- 新增 `LapTelemetry` 只读 trait，并让后端内部的 `SharedLap` 持有 `Arc<[TelemetryPoint]>`。`analysis`、`comparison`、`segmentation` 路径现在通过 telemetry slice 只读消费数据，避免为了后端计算复制整圈 telemetry。
- `get_lap_telemetry` 返回给前端时仍会 `to_vec()`，这是 Tauri IPC 序列化边界需要 owned data 的成本；后续如果仍有性能压力，可再评估分页/降采样/二进制传输。
- 尚未完成：按字节估算的 cache 容量限制和专门 benchmark 还未补充。

### P2：前端异步请求存在旧响应覆盖新状态的竞态

相关位置：

- `src-ui/App.tsx:58`
- `src-ui/App.tsx:61`
- `src-ui/App.tsx:64`
- `src-ui/App.tsx:71`
- `src-ui/App.tsx:91`
- `src-ui/App.tsx:93`
- `src-ui/App.tsx:103`
- `src-ui/App.tsx:104`

问题说明：

`loadChartData`、corner metadata 加载、compare result 加载都没有请求代次或取消保护。用户快速切换 teacher/student/lap 时，较慢的旧请求可能在新请求之后返回，并覆盖当前界面状态。

影响：

- 图表可能展示上一圈 telemetry。
- comparison 结果可能对应旧 teacher/student。
- corner metadata 可能对应旧 track，导致分析面板和图表标记不一致。

修复建议：

- 使用 `useRef` 保存递增 request id，每次发起请求时递增，响应回来时只接受当前 id。
- 或在每个 `useEffect` 中使用 `cancelled` 标志，cleanup 时置为 true。
- 将 `loadChartData` 的 effect 依赖补齐，避免闭包引用旧的 `selectedId` 或 `teacherId`。
- 增加前端测试：mock 两个不同 lap 请求，让旧请求延迟返回，断言不会覆盖新状态。

### P2：overlay 轮询没有间隔校验和 in-flight 防重入

相关位置：

- `src-ui/components/LocalDashboardOverlayWindow.tsx:118`
- `src-ui/components/LocalDashboardOverlayWindow.tsx:137`
- `src-ui/components/LocalDashboardOverlayWindow.tsx:187`

问题说明：

overlay 使用配置中的 `windowMs`、`statusMs`、`frameMs` 直接创建 `setInterval`。如果配置为 0、极小值，或单次 IPC 调用耗时超过间隔，轮询会堆叠并发请求。

影响：

- overlay 窗口可能高频占用 CPU。
- Tauri IPC 和 shared memory 读取压力上升。
- 网络/窗口状态异常时可能形成持续错误轮询。

修复建议：

- 对轮询间隔做最小值限制，例如 `windowMs >= 250`、`statusMs >= 500`、`frameMs >= 16/33`。
- 为每类轮询增加 `inFlight` 标志，上一轮未完成时跳过下一轮。
- 连续失败时增加退避策略，成功后恢复正常频率。
- 保存 overlay 配置时也做 schema 校验，防止坏配置落盘。

### P3：dashboard asset server 暴露面偏大且每连接创建线程

相关位置：

- `src/dashboard/assets.rs:43`
- `src/dashboard/assets.rs:55`
- `src/dashboard/assets.rs:57`

问题说明：

asset HTTP server 绑定 `0.0.0.0:0`，会监听所有网卡。每个 incoming 连接都会 spawn 一个线程，且没有 read timeout、连接数限制或简单限流。

影响：

- 若机器处在不可信局域网，其他设备可访问该临时 asset server。
- 大量连接或慢连接会造成线程数量增长。
- 字体等 dashboard asset 虽不一定敏感，但仍扩大了应用攻击面。

修复建议：

- 如果只服务本机 overlay，改为绑定 `127.0.0.1`。
- 如果必须服务平板设备，增加配置项明确开启局域网监听，并在 UI 上提示风险。
- 为 `TcpStream` 设置 read/write timeout。
- 使用线程池或异步 HTTP server，限制最大并发连接数。

### P3：前端包体偏大，影响启动性能

相关位置：

- `cmd /c npm run build` 输出显示主 chunk `assets/index-B34RMu0p.js` 为 `701.80 kB`，gzip 后 `222.25 kB`。

问题说明：

构建通过，但 Vite 提示存在超过 700 kB 的 chunk。当前依赖中 `jspdf`、`html2canvas`、dashboard designer 等功能较重，如果全部进入初始 chunk，会增加应用启动和首屏加载成本。

影响：

- 冷启动和首次进入 UI 的加载时间上升。
- 低性能设备或 WebView 环境下更明显。
- 与 dashboard/pdf 导出无关的常规路径也承担了重依赖成本。

修复建议：

- 对 dashboard designer、PDF/图片导出相关组件使用 `React.lazy` 和动态 import。
- 将 `jspdf`、`html2canvas` 只在用户执行导出时动态加载。
- 使用 Vite/Rollup `manualChunks` 拆分稳定的大依赖。
- 增加 bundle analyze 脚本，持续观察 chunk 体积变化。

### P3：静态检查失败和文本编码异常会降低维护质量

相关位置：

- `src-ui/components/ChartArea.tsx:1`
- `src-ui/components/ChartArea.tsx:270`
- `src-ui/components/ChartArea.tsx:279`
- `src-ui/components/ChartArea.tsx:404`
- `src-ui/components/DashboardDesignerView.tsx:334`
- `src-ui/components/DataSummaryView.tsx:91`
- `src/analysis/coach.rs:735`
- `src-ui/App.tsx:151`

问题说明：

ESLint 当前失败，且部分源码包含 mojibake 或不可见字符。Rust clippy 在 `-D warnings` 下也失败。虽然其中不少是风格项，但 lint 不能通过会让后续 CI 很难区分新问题和旧债务。

影响：

- CI 若启用 lint 会阻断交付。
- 文案 mojibake 会直接影响用户界面和分析报告可读性。
- 不可见字符和控制字符正则会增加后续维护成本。

修复建议：

- 清理 `ChartArea.tsx` 文件头不可见字符，并把三元表达式语句改为显式 `if/else`。
- 调整 `DashboardDesignerView.tsx` 中控制字符占位方案，避免触发 `no-control-regex`，例如使用普通字符串 token 或集中 disable 单行规则并说明原因。
- 修正 `DataSummaryView.tsx` 中无意义的转义。
- 统一修复 mojibake 文案，保证源码以 UTF-8 保存。
- 对 clippy 风格项集中修复，或对确有设计原因的 `too_many_arguments` 做局部 allow 并注明理由。

---

## 原审计复核更正

### 行号偏移

原审计标注的若干行号与当前代码有偏移。以下是确认后的实际位置：

| 原审计描述 | 原标注行号 | 当前实际行号 | 说明 |
|---|---|---|---|
| `remove_live_session_for_reimport` 调用点 | 1505, 1774 | 1445, 1775 | `ensure_live_data_entry_imported` 中调用在 1445，`import_acctlm_file` 中在 1775 |
| `TelemetryCache::get/put` 的 `.cloned()/unwrap()` | 68, 76 | 68, 76 | ✅ 行号未变 |
| `get_lap_telemetry` 持有 DB 锁 | 523 | 517-530 | 函数签名从 517 行开始 |
| `analyze_laps` 持有 DB 锁 | 541, 552 | 533, 553 | 同上 |
| `sync_directory` 持有 DB 锁 | 1734 | 1724-1735 | 整个函数持有锁 |
| overlay 轮询 setInterval | 118, 137, 187 | 118, 137, 187 | ✅ 行号未变（已有 `cancelled` 标志的保护，见下方更正） |

### 原审计精确度更正

1. **P2 overlay 轮询**：原审计称"没有请求代次或取消保护"，但 `LocalDashboardOverlayWindow.tsx` 的三个轮询 effect（bounds:118、status:137、frame:187）**已经使用了 `cancelled` 标志和 `clearInterval` 清理**。然而，原审计指出的"请求堆叠"和"in-flight 防重入"问题确实存在——`cancelled` 只防止了卸载后的状态写入，但未防止同一轮轮询内的重入。对 P2 问题的描述应为"缺乏 in-flight 防重入和最小间隔校验"，而非"没有取消保护"。

2. **P2 前端竞态**：原审计指出 `loadChartData`（第 58 行）没有请求代次保护，这是正确的。但同时应指出 `App.tsx` 第 71 行的 `useEffect` 依赖数组为 `[teacherId, studentId]`，但 `loadChartData` 的闭包捕获了 `addToast`——虽然 `addToast` 是稳定的，但 `selectedId` 的更新不触发此 effect，意味着用户只点击 lap（不改变 teacher/student）时 `chartData` 不会自动刷新。

3. **P1 重导入**：原审计指出 `remove_live_session_for_reimport` 不是事务性的，这是正确的。但代码中 `ensure_live_data_entry_imported` 在 `EMPTY_LIVE_RECORDING_ERROR` 分支（1448-1452 行）调用了 `remove_live_data_entry` 来清理空 recording 的 data entry，这个清理路径同样不在事务中——即如果 `remove_live_data_entry` 后续又失败，data entry 已被删除但旧 session 也已被删除。

---

## 复核新增问题

### P2（新增）：TelemetryCache 内部 Mutex 的 `unwrap()` 在 poisoned 场景下会 panic

相关位置：

- `src/ipc/mod.rs:71`
- `src/ipc/mod.rs:79`

问题说明：

`TelemetryCache::get` 和 `TelemetryCache::put` 使用 `std::sync::Mutex` 并对 `.lock()` 结果调用 `.unwrap()`。如果缓存访问在持有锁期间 panic（例如 `LruCache` 容量计算溢出），锁会被 poison，后续所有缓存访问都会 panic 导致应用崩溃。

影响：

- 任何导致 `inner.lock()` poison 的 panic 会使整个进程陷入不可恢复状态。
- 虽然当前代码路径不太可能触发 poison，但防御性不足。

修复建议：

- 将 `.unwrap()` 改为 `.unwrap_or_else(|e| e.into_inner())`，在 poisoned 情况下恢复并继续，而非 panic。
- 或改为 `lock().map_err(|e| e.to_string())?` 返回 Result。

### P2（新增）：`interpolation::compare_laps_at_equidistant_points` 对空圈的 `.unwrap()` 不一致

相关位置：

- `src/utils/interpolation.rs:29`
- `src/utils/interpolation.rs:30`

问题说明：

函数开头有 `if ref_lap.is_empty() || cur_lap.is_empty()` 的空圈保护（返回空 Vec），但随后直接对 `ref_lap.last().unwrap()` 和 `cur_lap.last().unwrap()` 取值。这意味着如果传入仅含 `NaN` 距离的圈数据（长度 > 0 但 `distance_m` 排序后可能异常），`last()` 在逻辑上不会 panic，但如果保护条件被移除或绕过则会产生 `panic!`。更重要的是，**没有任何调用方验证圈遥测数据是否为空再传入**。

影响：

- 虽然当前路径中 `is_empty` 保护存在，但缺乏防御性注释或 explicit check。
- 圈数据损坏时（长度 > 0 但 distance 值全为 NaN/Inf），`unwrap` 在逻辑安全但维护者可能误判。

修复建议：

- 将 `.unwrap()` 替换为 `.expect("last point must exist after non-empty check")`，使意图显式。
- 或在保护条件后直接通过 `ref_lap[ref_lap.len() - 1].distance_m` 访问，避免 `Option` 解包。

### P2（新增）：数据库迁移不是事务性的

相关位置：

- `src/db/migrations.rs:3-54`

问题说明：

每个 migration 版本（v1 到 v7）的 SQL 执行和版本号插入是分开的两步操作。如果 v_N 的 SQL 部分执行成功但在 `INSERT INTO migrations` 前进程崩溃：

- 数据库已有 v_N 的表/列变更，但 migrations 表仍记录 v_(N-1)。
- 重启后 migration 会重新执行 v_N 的 SQL，对于 `ALTER TABLE ADD COLUMN` 会导致 "duplicate column" 错误（SQLite 不支持 IF NOT EXISTS 用于 ALTER COLUMN）。
- 对于 `CREATE TABLE IF NOT EXISTS` 是安全的，但 v4、v5、v7 的 `ALTER TABLE` 和 `UPDATE` 语句不幂等。

影响：

- 迁移中途崩溃后重启可能导致应用无法启动。
- v4 (`ALTER TABLE tracks ADD COLUMN`) 和 v5 (`ALTER TABLE laps ADD COLUMN`) 都是 ADD COLUMN，重跑会报 "duplicate column name"。

修复建议：

- 将每个 migration 版本包在 `BEGIN IMMEDIATE TRANSACTION ... COMMIT` 中，使 SQL 变更和版本号插入原子化。
- 或使用 `PRAGMA application_id` + `PRAGMA user_version` 代替手动 migrations 表。

### P2（新增）：`loadChartData` 依赖数组不完整导致状态不一致

相关位置：

- `src-ui/App.tsx:71`

问题说明：

```typescript
useEffect(() => { loadChartData(selectedId, teacherId); }, [teacherId, studentId]);
```

依赖数组为 `[teacherId, studentId]`，但 `loadChartData` 实际上依赖 `selectedId`（第一个参数 `viewId`）。当用户在 LapPanel 中点击新的 lap（调用 `selectLap`）时，`selectedId` 会更新，但该 effect 不会触发——因为 `teacherId` 和 `studentId` 没变。`selectLap` 函数内部直接调用了 `loadChartData`，但这种模式使 effect 和 imperative 调用两条路径容易出现不一致。

影响：

- 如果 `teacherId` 或 `studentId` 变化时 effect 触发，但此时 `selectedId` 还是旧值（闭包中），会加载旧 lap 的数据。
- `selectLap` 中使用了闭包捕获的 `teacherId`（通过 `useCallback`），如果 `teacherId` 变了而 `selectLap` 的依赖没更新，会发送旧的 `teacherId`。

修复建议：

- 将 `selectedId` 加入 effect 依赖：`[selectedId, teacherId]`。
- 或者统一数据加载入口，不在 `selectLap` 中 imperative 调用 `loadChartData`，改由 effect 统一驱动。
- 为 `loadChartData` 添加请求代次保护（如 useRef 递增 ID），解决已知的竞态问题。

### P3（新增）：`AppError` 类型定义但几乎未被使用——错误处理不统一

相关位置：

- `src/error.rs:1-13`

问题说明：

代码库定义了 `AppError` 枚举（包含 `Io`、`InvalidFormat`、`Database`、`Serialize` 变体），并实现了 `thiserror::Error`。但几乎所有 IPC command 都使用 `Result<_, String>` 作为错误类型，将底层错误通过 `.map_err(|e| e.to_string())` 转为字符串。`AppError` 仅在 `ld_parser` 模块中使用。

影响：

- 错误信息被扁平化为字符串，前端收到的错误没有结构化类型，难以做条件处理。
- 不同模块的错误处理风格不统一，增加维护成本。

修复建议：

- 将所有 IPC command 的错误类型统一为 `Result<_, AppError>`，让 Tauri 的序列化机制将错误类型信息传递到前端。
- 对于确实只面向人类阅读的错误，保留 `String`，但添加注释说明。

### P3（新增）：`ensure_live_data_entry_imported` 中 `unwrap_or` 静默回退可能导致数据不一致

相关位置：

- `src/ipc/mod.rs:1470`
- `src/ipc/mod.rs:1472`

问题说明：

live recording 重新导入时，如果文件元数据读取失败（`std::fs::metadata`），代码使用 `unwrap_or(entry.file_size_bytes)` 回退到旧值；如果 hash 计算失败，回退到 `entry.sha256_hash.clone()`。这意味着文件被重新导入但 `file_size_bytes` 和 `sha256_hash` 可能是**旧文件**的值——这恰好与重导入的目的矛盾。

影响：

- 文件大小和哈希与新文件不匹配，但 data entry 记录的是旧值。
- 后续通过 hash 去重的逻辑可能误判为新文件已存在（旧 hash 对不上新文件）。

修复建议：

- 文件元数据和 hash 计算失败应返回错误，不应回退到旧值。
- 如果确实需要在某些情况下跳过，应当记录警告日志。

### P3（新增）：CSP 策略允许 `'unsafe-inline'` 用于样式

相关位置：

- `tauri.conf.json:17`

问题说明：

CSP 中 `style-src` 包含 `'unsafe-inline'`，这虽然在 CSS 上下文中的风险低于 `script-src` 的 `'unsafe-inline'`，但仍允许任意内联样式注入。对于桌面应用影响有限，但如果 WebView 加载了任何外部内容（虽然当前 `default-src` 为 `'self'`），CSS 注入仍可能影响 UI 展示。

影响：

- 桌面应用场景下风险较低，但不满足最严格的 CSP 要求。

修复建议：

- 如果内联样式仅用于 CSS Modules 或动态计算的少量样式，考虑使用 nonce 或 hash 替代。
- 如果内联样式确实必要可保留，但需在文档中注明原因。

### P2（新增）：数据库缺少 CASCADE 删除约束，依赖手动清理

相关位置：

- `src/db/migrations_v1.sql`（所有 FK 声明）
- `src/db/migrations_v2.sql`（bookmarked_laps FK 声明）
- `src/db/migrations_v7.sql`

问题说明：

所有外键关系均没有 `ON DELETE CASCADE` 或 `ON DELETE SET NULL`，完全依赖应用代码在删除父行时手动清理子行。当前只有 `remove_live_session_for_reimport` 和 `remove_live_data_entry` 做了手动清理，但 `remove_empty_live_recording_entries` 在删除 `data_entries` 行时**没有先删除关联的 `bookmarked_laps` 行**，会产生孤立记录。

此外，`laps` 表的删除不会自动级联到 `analysis_reports` 和 `bookmarked_laps`，如果未来添加新的删除路径，容易遗漏清理步骤。

影响：

- `bookmarked_laps` 中可能出现引用已删除 `data_entries` 的孤立记录。
- 任何新增的删除路径如果遗漏手动清理，都会产生数据完整性问题。
- SQLite 虽然 `PRAGMA foreign_keys = ON` 已启用，但缺少 CASCADE 意味着删除父行会因 FK 约束而失败（而非级联删除），需要应用层显式处理。

修复建议：

- 在新 migration (v8) 中为所有 FK 添加 `ON DELETE CASCADE`。
- 或至少修复 `remove_empty_live_recording_entries` 中的遗漏：在删除 `data_entries` 行之前，先删除关联的 `bookmarked_laps` 行。

### P2（新增）：缺少关键数据库索引

相关位置：

- `src/db/migrations_v1.sql` ~ `src/db/migrations_v7.sql`
- `src/db/mod.rs:88-95`（`get_valid_laps_by_track` 查询）

问题说明：

以下频繁查询的列缺少索引：

1. `sessions(track_id)` — `get_valid_laps_by_track` 使用 `WHERE s.track_id = ?1` 进行全表扫描。
2. `data_entries(session_id)` — `set_lap_validity`、`live_data_entry_for_session`、`remove_empty_live_recording_entries` 等多个查询通过此列关联。

影响：

- 当 sessions 数量增长（多赛道、多 session 会话），`get_valid_laps_by_track` 查询性能会明显下降。
- live recording 的 lazy import 和 data entry 查询在 data_entries 表行数增长后也会变慢。

修复建议：

- 添加 migration v8：`CREATE INDEX IF NOT EXISTS idx_sessions_track ON sessions(track_id);` 和 `CREATE INDEX IF NOT EXISTS idx_data_entries_session ON data_entries(session_id);`。

### P2（新增）：`import_acctlm_file` 存在 TOCTOU 竞态条件

相关位置：

- `src/ipc/mod.rs:1754-1786`

问题说明：

在 `import_acctlm_file` 中，哈希检查和实际导入之间存在锁释放间隙：

```rust
let mut existing_entry: Option<DataEntry> = None;
{
    let db = db.lock().await;           // 获取锁
    let existing = get_data_entry_by_hash(db.conn(), &sha256_hash)?;
    // ... 处理已有条目
}                                         // 释放锁 —— TOCTOU 间隙
let import_path = {
    let paths = workspace_paths.lock().await;
    copy_file_to_data_dir(path, &paths.data_dir)?   // 锁外文件复制
};
let db = db.lock().await;                // 重新获取锁
// ... 使用可能过期的 existing_entry
```

在两次获取锁之间，另一个 IPC 命令可能修改或删除了 `existing_entry`，导致重导入逻辑基于过期数据。

影响：

- 同一文件并发导入可能绕过去重检查，导致重复数据。
- 已删除条目的 `existing_entry` 可能引用不存在的行，导致 FK 约束失败。

修复建议：

- 将整个 import 流程包在同一个锁持有期间（缩小到仅必要的数据库操作），或在重新获取锁后重新读取 `existing_entry`。

### P2（新增）：前端多处异步请求缺少竞态保护（补充）

相关位置：

- `src-ui/App.tsx:91-96`（`get_cached_corners` 无取消保护）
- `src-ui/App.tsx:99-106`（`compare_laps` 无取消保护）
- `src-ui/components/AnalysisPanel.tsx:57`（`loadCorners` 无取消保护）
- `src-ui/components/AnalysisPanel.tsx:76-81`（`runAnalysis` 无取消保护）
- `src-ui/components/DataSummaryView.tsx:199-216`（多个并发的 fire-and-forget 请求）
- `src-ui/components/SessionDetail.tsx:46-60`（并发的 `get_session_laps` 和 `get_bookmarked_laps`）

问题说明：

除了原审计指出的 `loadChartData` 竞态外，以下 effect 也缺少请求代次或取消保护：

1. `App.tsx:91-96` 的 `get_cached_corners` — 快速切换 track 时旧响应覆盖新状态。
2. `App.tsx:99-106` 的 `compare_laps` — 快速切换 student/teacher 时旧比较结果覆盖新结果。
3. `AnalysisPanel.tsx:57` 的 `loadCorners` — 快速切换 teacher 时旧弯角数据覆盖。
4. `AnalysisPanel.tsx:76-81` 的 `runAnalysis` — 快速切换 teacher/student 时旧分析报告覆盖。
5. `DataSummaryView.tsx:199-216` — 3 个并发的 `invoke` 调用没有取消保护，如果组件卸载，`setState` 会在已卸载组件上调用。
6. `SessionDetail.tsx:46-60` — 类似的并发请求无取消保护。

注意：`App.tsx:75-88` 的 `get_track_for_lap` 已经正确使用了 `cancelled` 标志模式，可作为参考实现。

影响：

- 用户快速切换选项时会看到闪烁的旧数据或旧分析报告。
- 组件卸载后的 setState 调用会导致 React 警告和潜在状态不一致。

修复建议：

- 为每个异步 effect 添加 `cancelled` 标志和 cleanup 函数（参考 `App.tsx:75-88` 的模式）。
- 或实现通用的 `useRequestWithCancellation` hook，统一处理竞态。

### P2（新增）：前端 `DashboardDesignerView` 中可重排控件使用数组索引作为 key

相关位置：

- `src-ui/components/DashboardDesignerView.tsx:1914`
- `src-ui/components/DashboardDesignerView.tsx:1972`

问题说明：

```tsx
key={`${control.id || "control"}-${index}`}
```

Dashboard designer 控件有拖拽重排功能（`reorderControls`），但 key 包含了 `index`。当控件被重排后，React 会错误地复用 DOM 节点，可能导致焦点丢失、拖拽状态错乱和渲染不一致。

影响：

- 控件重排后可能出现显示错误或交互异常。
- 拖拽操作后 React diff 算法无法正确匹配新旧节点。

修复建议：

- 移除 key 中的 `index`，使用 `control.id` 作为唯一 key（`validateControlIds` 函数已确保 id 唯一）。

### P2（新增）：Toast 计时器重置 Bug

相关位置：

- `src-ui/components/Toast.tsx:45-51`

问题说明：

```typescript
useEffect(() => {
    if (toasts.length === 0) return;
    const timer = setTimeout(() => {
      setToasts((prev) => prev.slice(1));
    }, 5000);
    return () => clearTimeout(timer);
}, [toasts]);
```

effect 依赖数组为 `[toasts]`，每次新 toast 添加时都会重新创建 timer 并清除旧的。这意味着：添加新 toast 时，第一个 toast 的 5 秒倒计时会重置，导致首个 toast 在高频 toast 场景下停留时间远超 5 秒。

影响：

- 高频 toast 场景下，最早的消息被延迟清除，用户可能错过重要提示。
- 极端情况下，旧 toast 可能停留 15-30 秒。

修复建议：

- 为每个 toast 使用独立的 timer（基于 `createdAt` 时间戳），或使用 `useRef` 维护独立的定时器。

### P3（新增）：前端仅顶层 ErrorBoundary，无组件级错误边界

相关位置：

- `src-ui/App.tsx:355-367`
- `src-ui/main.tsx:8-10`

问题说明：

整个应用只包裹了一个顶层 `ErrorBoundary`。如果 `ChartArea`（Canvas 绘制）、`DashboardDesignerView`（表达式求值）或 `TelemetryWorkspaceView`（复杂状态管理）中的任何一个抛出渲染错误，整个应用将显示崩溃界面。

影响：

- 某个组件的 Canvas 渲染错误会导致整个应用不可用。
- 表达式求值错误（用户输入恶意公式）会使整个 dashboard 页面崩溃。

修复建议：

- 在 `ChartArea`、`DashboardDesignerView` 组件外层各包裹一个 `ErrorBoundary`。
- 在 `AnalysisPanel` 外层也考虑添加，避免分析错误影响整个视图。

### P3（新增）：`query_data_entries` 中动态 SQL 构建模式

相关位置：

- `src/db/metadata.rs:160-172`

问题说明：

`query_data_entries` 使用 `format!()` 拼接 `ORDER BY`、`LIMIT`、`OFFSET` 子句到 SQL 字符串中，而非使用参数化查询。虽然 `sort_col` 和 `sort_dir` 经过白名单验证（`limit` 和 `offset` 是 `Option<u32>` 类型），但 `format!()` 在 SQL 中使用是一种反模式。

影响：

- 当前无 SQL 注入风险（因为白名单和类型约束），但维护风险高——未来如果白名单逻辑被绕过或类型被改为 String，会直接引入注入漏洞。
- `LIMIT {}` 和 `OFFSET {}` 可以改为 `?` 参数绑定。

修复建议：

- 将 `LIMIT` 和 `OFFSET` 改为参数化查询：`LIMIT ? OFFSET ?`，绑定 `limit` 和 `offset` 值。
- 对 `ORDER BY` 保持白名单，但添加注释说明安全假设。

### P3（新增）：IPC 命令参数缺少大小限制

相关位置：

- `src/ipc/mod.rs`（所有接受 `String` 参数的 command）

问题说明：

所有 IPC 命令的 String 参数（如 `path`、`corners_json`、`filter_json`、`layout` 中的 `static_image_base64`）都没有最大长度校验。恶意或异常大的前端请求可以导致内存压力。

影响：

- 超长的 JSON payload 可导致解析和内存消耗问题。
- `register_dashboard_layout` 和 `send_dashboard_layout_to_tablet` 中的 base64 图片数据没有大小限制。

修复建议：

- 为 IPC 字符串参数添加合理的大小限制（如路径 4096 字节、JSON 1 MiB）。
- 对 `static_image_base64` 添加最大尺寸校验（如 10 MiB）。

### P3（新增）：CSP 策略缺少关键指令

相关位置：

- `tauri.conf.json:13-21`

问题说明：

当前 CSP 配置缺少以下指令：

| 缺失指令 | 风险 | 说明 |
|---|---|---|
| `base-uri` | 中 | 攻击者可注入 `<base>` 标签劫持相对 URL |
| `frame-ancestors` | 中 | 无点击劫持保护，应用可被 iframe 嵌入 |
| `form-action` | 低 | 无限制的表单提交目标 |

虽然 `default-src 'self'` 提供了基础保护，但缺少 `frame-ancestors 'none'` 在桌面 WebView 场景下仍是不完整的。

修复建议：

- 添加 `base-uri 'self'`、`frame-ancestors 'none'`、`form-action 'self'` 到 CSP 配置。

---

## 建议修复顺序

1. 优先修复 P1 的事务一致性问题：live/acctlm 重导入、`.ld` 同步导入。
2. 其次处理 P2 的响应性和数据完整性问题：
   - 缩短 DB 锁范围
   - 优化 telemetry cache
   - 前端请求竞态（含 `loadChartData` 依赖补充、`get_cached_corners`/`compare_laps`/AnalysisPanel 的全部竞态）
   - overlay 轮询防重入
   - TelemetryCache unwrap 安全性
   - interpolation unwrap 防御
   - 数据库迁移事务化
   - 数据库 FK CASCADE 和索引补全
   - `import_acctlm_file` TOCTOU 竞态
   - DashboardDesignerView 数组索引 key
   - Toast 计时器重置
3. 然后处理 P3：asset server 暴露面、前端包体拆分、lint 和编码清理、AppError 统一、unwrap_or 静默回退、CSP 完善、动态 SQL 重构、IPC 参数大小限制、组件级 ErrorBoundary。

## 备注

当前工作区在审计前为干净的 tracked 状态，仅存在被 `.gitignore` 忽略的目录。`docs/` 目录本身也在 `.gitignore` 中，因此本报告默认作为本地审计产物保存。

---

## 修复记录（2026-06-13）

本节记录审计后已经落地的修复方法和复验结果。问题编号中的 1、2（dashboard asset server 暴露面、前端包体拆分）按当前讨论暂缓，后续会随整体逻辑调整另行处理。

### 已复验命令

- `cmd /c npm run lint`：通过。
- `cargo clippy --all-targets --all-features -- -D warnings`：通过。
- `cargo check`：通过。
- `cargo test`：通过，库单元测试、e2e、real LD、regression 测试均通过。
- `cmd /c npm run build`：通过。Vite 仍提示主 chunk 超过 700 kB，这是暂缓处理的前端包体问题。

### P1/P2 已修复项

1. live/acctlm 重导入事务一致性

   修复方法：将重导入流程改为“先解析、后短事务写入”。`prepare_live_import` 在不持有 DB 全局锁的情况下完成 live/acctlm 文件解析、hash 和 metadata 准备；`apply_prepared_live_import_if_needed` 在重新读取最新 data entry 后，用 `immediate_transaction` 原子删除旧 session/laps/bookmarks/analysis 并写入新 session/laps/data entry，失败时统一 rollback。`import_acctlm_file` 也改为先复制并解析，再在事务中处理已存在 entry 或新 entry；解析失败会删除刚复制的文件。

2. `.ld` 目录同步事务一致性

   修复方法：`sync::copy_new_files` 每个源文件单独使用 `immediate_transaction` 写入 tracks/sessions/laps/data_entries；文件先解析成功后再复制，若 DB 注册失败会删除刚复制的目标文件。新增回归测试覆盖无效 LD 不落盘、不登记，以及注册失败时 DB rollback 并清理复制文件。

3. 全局 DB 锁范围过大

   修复方法：在 `src/ipc/mod.rs` 引入 `LapLoadPlan`，`get_lap_telemetry`、`analyze_laps`、`compare_laps`、`detect_corners` 只在锁内读取 lap row、entry、path、track distance 等轻量信息，释放锁后再解析文件、提取 telemetry 和执行分析。需要写 report 时再短暂重新拿锁。`sync_directory` 改为根据 DB path 打开独立连接执行同步，避免长时间占用全局 `Mutex<Database>`。`get_session_laps` 的 lazy import 也拆成 prepare/apply 两阶段，解析不持锁，写入短事务。

4. TelemetryCache 整圈 clone 和 Mutex poison panic

   修复方法：cache value 从 `Vec<TelemetryPoint>` 改为 `Arc<[TelemetryPoint]>`，命中时只 clone `Arc` 指针；内部分析路径通过 `LapTelemetry` trait 读取 telemetry slice，避免后端计算复制整圈数据。Tauri IPC 边界 `get_lap_telemetry` 仍按序列化需求 `to_vec()`。`TelemetryCache` 的 `Mutex::lock().unwrap()` 改为 `unwrap_or_else(|poisoned| poisoned.into_inner())`，poison 后不再持续 panic。

5. overlay 轮询堆叠

   修复方法：`LocalDashboardOverlayWindow.tsx` 为 bounds/status/frame 三类轮询增加最小间隔限制和 `inFlight` 防重入；上一轮 IPC 未结束时跳过当前 tick，避免极小 interval 或慢 IPC 造成并发堆叠。

6. interpolation 空圈 unwrap

   修复方法：`src/utils/interpolation.rs` 在读取首尾 telemetry 前保持空输入保护，避免空 lap 数据进入 `.last().unwrap()` 路径导致 panic。

7. migration 事务化、FK cascade、关键索引

   修复方法：`src/db/migrations.rs` 的单个 migration 应用改为事务包裹，SQL 变更和 migration version 写入原子化。新增 `migrations_v8.sql`，重建关联表并为外键补 `ON DELETE CASCADE`，同时增加 `idx_sessions_track`、`idx_data_entries_session` 等热点索引。`src/db/mod.rs` 增加 v8 cascade/index 回归测试。

8. DashboardDesignerView index key

   修复方法：可重排 control 列表的 React key 改为稳定的 `control.id`，不再拼接数组 index，避免拖拽重排后 DOM 复用错误。

9. App 主界面异步竞态和 loadChartData 依赖不完整

   修复方法：移除 imperative `loadChartData` 双路径，改为由 `[selectedId, teacherId, addToast]` effect 统一加载 selected/teacher telemetry，并用 `cancelled` 保护旧响应。track corner、compare result effect 也增加取消保护，切换 teacher/student/track 时旧响应不会覆盖新状态。

10. AnalysisPanel、DataSummaryView、SessionDetail 异步取消保护

    修复方法：`AnalysisPanel` 的 `get_cached_corners` 和 `analyze_laps` effect 增加 `cancelled`。`DataSummaryView` 增加 `mountedRef`、entries request id、session-lap request id，避免卸载后 setState 或旧请求覆盖新列表。`SessionDetail` 在 entry 切换时清空旧数据，并对 laps/bookmarks 并发请求增加取消保护。

11. Toast 计时器重置

    修复方法：`Toast.tsx` 改为为每个 toast 维护独立 timer，dismiss 时清理对应 timer，组件卸载时清理全部 timer。`App.tsx` 将真实 `dismissToast` 传给 `ToastContainer`，不再使用空回调。

### P3 本轮已修复项（问题 3-9）

3. 静态检查失败和文本/规则异常

   修复方法：清理 `ChartArea.tsx` 文件头异常字符，将 canvas 绘制中的三元表达式语句改为显式 `if/else`；`DashboardDesignerView.tsx` 将表达式占位从 `\u0000` 控制字符改为私用区占位字符，避免 `no-control-regex`；`DataSummaryView.tsx` 修复无意义转义。Rust 侧按 clippy 提示修复 `map_or`、可 derive 的 `Default`、range loop、大小写比较等；确有 Tauri command 注入需要的多参数函数加局部 `allow(clippy::too_many_arguments)`。

4. AppError 错误处理不统一

   修复方法：`src/error.rs` 增加 `AppResult<T>` 别名，为 `AppError` 增加 `Message` 通用变体、`code()`、`Serialize` 实现、`From<String>`/`From<&str>`/`From<std::io::Error>` 和 `From<AppError> for String`。`ld_parser` 改为返回 `AppResult`。`src/ipc/mod.rs` 中所有 `#[tauri::command]` 的返回类型已从 `Result<_, String>` 统一迁移为 `IpcResult<_>`，命令边界通过 `to_ipc_result` 将现有 helper 的 `String`/底层错误收口到 `AppError`。内部 helper 仍可保留 `Result<_, String>`，但 IPC 对外错误类型已经结构化统一。

5. CSP 允许 `style-src 'unsafe-inline'`

   修复方法：`tauri.conf.json` 中 `style-src` 改为仅 `'self'`，移除 `'unsafe-inline'`。注意：当前 React 组件仍大量使用 inline style，构建不会暴露运行时 CSP 拦截，因此该项需要后续用真实 Tauri 窗口做一次冒烟验证。

6. 组件级 ErrorBoundary 缺失

   修复方法：保留 `App.tsx` 主内容区按 `activeView` reset 的视图级 `ErrorBoundary` 作为兜底，同时补充组件级边界：comparison 视图中 `ChartArea` 和 `AnalysisPanel` 分别独立包裹；`DataSummaryView`、`TrackBestLapsView`、`TrackMetadataView`、`TelemetryWorkspaceView`、`DashboardView` 各自有独立边界；`DashboardView` 内部的 `LocalDashboardView`、`RemoteDashboardTab`、`LayoutCentralTab` 和 `DashboardDesignerView` 也分别包裹 `ErrorBoundary`。这样单个高风险组件渲染异常不会直接拖垮整个应用或整个 dashboard 页面。

7. `query_data_entries` 动态 SQL 构建

   修复方法：`src/db/metadata.rs` 保留 `ORDER BY` 白名单表达式（SQLite 不支持用绑定参数替代表达式/列名），并添加注释说明安全边界；`LIMIT` 和 `OFFSET` 改为 `?` 参数绑定，值通过 `param_values` 传入。

8. IPC 参数缺少大小限制

   修复方法：`src/ipc/mod.rs` 新增 `validate_string_arg`、`validate_path_arg`、`validate_json_arg`、`validate_dashboard_layout_payload`。路径参数限制为 4096 字符，JSON 限制为 1 MiB，dashboard 静态图 base64 限制为 16 MiB，字体 data URL 限制为 4 MiB，字体数量限制为 16；并应用到 `load_ld_file`、`sync_directory`、`import_acctlm_file`、`query_data_entries`、corner JSON、workspace path、bookmark/lap/data entry id、dashboard layout 注册/发送等 IPC 边界。

9. CSP 缺少关键指令

   修复方法：`tauri.conf.json` 增加 `base-uri 'self'`、`form-action 'self'`、`frame-ancestors 'none'`、`object-src 'none'`，补齐桌面 WebView 的基础防护指令。

### 暂缓项

- P3 dashboard asset server 暴露面偏大且每连接创建线程：按当前讨论暂缓，后续随 dashboard/asset 逻辑整体调整。
- P3 前端包体偏大：按当前讨论暂缓。当前 `cmd /c npm run build` 仍会提示主 chunk 超过 700 kB。

---

## 审计关闭记录（2026-06-13）

- 状态：Closed。
- 关闭结论：本审计中除已明确暂缓的 P3 `dashboard asset server 暴露面` 和 P3 `前端包体偏大` 外，其余已讨论要求修复的问题均已完成修复并通过复验。
- 修复 commit：`84d46cc549069d738fb198bdb47c4dc2c9c6ed6c`。
- 复验命令：`cmd /c npm run lint`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo check`、`cargo test`、`cmd /c npm run build`。
- 备注：`cmd /c npm run build` 仍提示主 chunk 超过 700 kB，该项对应暂缓处理的前端包体问题，不阻塞本次审计关闭。
