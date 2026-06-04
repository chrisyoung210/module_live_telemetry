# module_live_telemetry 审计报告

## 审计基线

- 审计日期：2026-06-03
- 基线 commit：`5202a74a8090aef7bfe1cabcbb436457ee41e5dc`
- 审计范围：Rust crate 源码、CLI、二进制读写格式、共享内存解析路径、测试入口与文档一致性
- 工作区状态：审计时存在未跟踪项 `.opencode/`、`session-ses_181a.md`；未纳入本次问题判断
- 后续增量审计建议：以下一次审计时的 `HEAD` 与本基线 commit 执行对比，重点查看 `src/reader.rs`、`src/writer.rs`、`src/shmem.rs`、`src/bin/acc-live-telemetry.rs`、`tests/` 的变更

## 验证命令

```powershell
cargo test
```

结果：失败。测试引用了不存在的 `module_live_telemetry::mock` 模块。

```powershell
cargo check --all-targets
```

结果：未完成。命令因 `target/debug/.cargo-lock` 访问拒绝失败，疑似本机 target 目录权限或锁文件问题；该结果不作为源码缺陷结论。

## 发现

### P1：测试当前无法编译

- 位置：`tests/binary_roundtrip.rs:1`
- 现象：测试引用 `module_live_telemetry::mock::generate_mock_controls`，但当前 crate 未定义或导出 `mock` 模块。
- 影响：`cargo test` 直接失败，回归测试链路不可用；后续格式修复或解析修复缺少自动保护。
- 建议：
  - 在测试内本地构造 `ControlSample` fixture；或
  - 恢复并导出 `mock` 模块；或
  - 将 mock 工具放到 `tests/common`，避免成为 public API。

### P1：Lap index 写入后 reader 基本读不到

- 写入位置：`src/writer.rs:82` 到 `src/writer.rs:89`
- 读取位置：`src/reader.rs:88` 到 `src/reader.rs:89`
- 现象：`finish()` 把 `footer_offset` 设置为 Index Block 起点，而不是 Footer 起点。reader 使用 `footer_offset + 24` 查找 `LAPS`，但 `footer_offset + 24` 仍位于 Index Block 或索引条目区域内，未跳过 `INDEX_MAGIC`、`entry_count`、全部 `IndexEntry` 和 Footer 本体。
- 影响：`record` 自动追加的 lap index 和 `build-lap-index` 手动追加的 lap index 大概率无法通过 `reader.lap_index()` 读取，相关按圈导出功能会回退扫描或失效。
- 建议：
  - 在 `read_index_from_footer` 中返回实际 footer 结束位置；或
  - 根据 `entry_count` 计算 `lap_index_offset = footer_offset + 12 + entry_count * INDEX_ENTRY_SIZE + FOOTER_SIZE`；
  - 为“写入后追加 lap index 再读取”的场景添加回归测试。

### P1：`parse_raw_frame` 存在未对齐引用的 unsafe 风险

- 位置：`src/shmem.rs:940` 到 `src/shmem.rs:941`
- 现象：函数把 `&[u8]` 直接 cast 为 `&SPageFilePhysicsControls` 和 `&SPageFileGraphicsRaw`。
- 影响：来自文件 buffer 的 slice 不保证满足目标 struct 对齐要求。在 Rust 中创建未对齐引用是未定义行为，可能在不同平台、优化级别或数据布局下触发崩溃或错误解析。
- 建议：
  - 使用 `std::ptr::read_unaligned` 读取 `Copy` 结构体值；或
  - 使用字段级 little-endian 解析；或
  - 引入明确支持 unaligned POD 读取的方案，并集中封装 unsafe。

### P2：`parse-raw` 对输入尺寸缺少边界检查，会 panic

- 位置：`src/bin/acc-live-telemetry.rs:712` 到 `src/bin/acc-live-telemetry.rs:715`
- 现象：`stat_bytes[68..134]` 和 `stat_bytes[134..200]` 直接切片，未校验 `stat_sz >= 200`。
- 影响：损坏或不兼容 `.accraw` 文件会导致 panic，而不是返回 `TelemetryError::InvalidFormat`。此外，`phys_sz`、`graph_sz`、`stat_sz` 来自文件头，缺少合理上限，异常文件可造成过大内存分配。
- 建议：
  - 在读取前校验静态页长度；
  - 对三类 page size 设置合理范围；
  - 对 `frame_size` 加溢出检查；
  - 添加畸形 `.accraw` 文件测试。

### P2：索引读取可被异常 `entry_count` 触发巨大分配

- 位置：`src/reader.rs:321` 到 `src/reader.rs:324`
- 现象：reader 直接信任文件中的 `entry_count`，并执行 `Vec::with_capacity(entry_count)`。
- 影响：损坏或恶意文件可声明极大索引数量，导致内存压力、分配失败或 panic。
- 建议：
  - 基于剩余字节数计算最大合法索引条目数；
  - 限制 `entry_count <= (remaining - FOOTER_SIZE) / INDEX_ENTRY_SIZE`；
  - 对超限输入返回 `TelemetryError::InvalidFormat`。

### P3：写入 9 个 cluster，但公开读取 API 只覆盖 4 个

- 写入位置：`src/writer.rs:104` 到 `src/writer.rs:112`
- 读取位置：`src/reader.rs:117` 到 `src/reader.rs:163`
- 现象：writer 写入 controls、motion、tyres、powertrain、session、timing、car_state、environment、other_cars 共 9 个 cluster；reader 只公开 controls、session、timing、environment 的读取接口。
- 影响：文件中已有数据无法被 crate 消费；如果目标是完整遥测回放，这是功能缺口。如果目标只导出部分数据，则当前格式会额外占用磁盘空间。
- 建议：
  - 补齐 motion/tyres/powertrain/car_state/other_cars 的 decode/read API；或
  - 在配置层允许选择写入 cluster，减少不需要的数据体积。

## 优先修复顺序

1. 修复测试编译问题，恢复 `cargo test` 基线。
2. 修复 lap index offset 计算，并增加 roundtrip 测试。
3. 替换 `parse_raw_frame` 中的未对齐引用读取。
4. 为 `.accraw` header/page size 添加严格校验。
5. 为 footer/index entry count 添加边界校验。
6. 根据产品目标决定补齐 reader API 或支持选择性写入 cluster。

## 后续增量审计记录方式

下次审计时以本报告的基线 commit 为起点：

```powershell
git diff 5202a74a8090aef7bfe1cabcbb436457ee41e5dc..HEAD
```

重点检查：

- 已发现问题是否修复，是否有对应测试。
- 二进制格式是否改变，文档与实现是否同步。
- unsafe 代码是否被收敛到小范围并有长度、对齐、布局校验。
- CLI 对损坏文件是否稳定返回错误而不是 panic。

## 2026-06-04 复审记录

### 复审基线

- 复审日期：2026-06-04
- 当前 `HEAD`：`5202a74a8090aef7bfe1cabcbb436457ee41e5dc`
- 复审状态：修复仍位于未提交工作区，`HEAD` 尚未前进
- 说明：`tests/binary_roundtrip.rs` 被删除是因为 mock 功能已取消，本次不再将该测试删除作为问题记录

### R1：`read_all_tyres()` 与 writer 的 tyres 列尺寸表不一致

- 位置：`src/writer.rs:265`、`src/reader.rs:909` 到 `src/reader.rs:914`
- 现象：writer 的 tyres `sizes` 表把 `tyre_contact_point`、`tyre_contact_normal`、`tyre_contact_heading` 以及后续 scalar 列的尺寸映射错位。`tyre_contact_*` 实际每行写入 12 个 `f32`，即 48 bytes；但 `sizes` 表在对应位置仍给出 16 bytes，导致 `ColumnEntry.byte_len` 与真实 payload 布局不一致。
- 影响：新增的 `read_all_tyres()` 按 48 bytes 读取 contact 列时，会命中 `column_bytes()` 的长度校验并返回 `invalid byte range`；即 writer 自己写出的 tyres chunk 可能无法被 reader 新 API 正常读回。
- 建议：
  - 修正 `encode_tyres_chunk()` 的 `sizes` 表，使 31 个列尺寸与 `TYRES_COLUMNS` 顺序逐项一致；
  - 增加 tyres roundtrip 测试，覆盖 `[f32; 4]`、`[f32; 12]` 和尾部 `i32` scalar 字段。

### R2：Lap index offset 修复方向正确，但无 footer 文件仍会尝试计算读取

- 位置：`src/reader.rs:120` 到 `src/reader.rs:123`
- 现象：reader 现在通过 `footer_offset + 12 + index_entries.len() * IndexEntry::BYTE_LEN + 28` 计算 lap index 起点。对正常 `finish()` 写完并追加 `LAPS` 的文件，该方向是正确的；但当文件没有 footer、走 `scan_chunks()` 回退路径时，`footer_offset` 为 0，代码仍会计算一个偏移并尝试读取 lap index。
- 影响：通常只会返回空 lap index，但语义不干净，也可能在特殊旧文件或损坏文件上造成误判。
- 建议：
  - 只在 `footer_offset > 0` 时尝试读取 lap index；
  - 为“finish 后追加 LAPS 并读取”和“无 footer 文件读取”分别增加测试。

### R3：`cargo clippy --all-targets -- -D warnings` 仍失败

- 位置：`src/shmem.rs:866`、`src/reader.rs:697`、`src/reader.rs:775`、`src/reader.rs:842`、`src/shmem.rs:808`、`src/shmem.rs:971` 到 `src/shmem.rs:973`
- 现象：`cargo check --all-targets` 已通过，但严格 clippy 仍失败。当前错误包括 `static_full` 未使用、可省略显式生命周期、数组填充循环触发 `needless_range_loop`、`std::io::Error::new(ErrorKind::Other, ...)` 可替换为 `std::io::Error::other(...)`，以及 `transmute` 缺少类型标注。
- 影响：如果 CI 或后续质量门禁启用 clippy deny warnings，当前修复无法通过；其中 `transmute` 也说明 unsafe 边界仍可继续收敛。
- 建议：
  - 删除或标注 `static_full`；
  - 按 clippy 建议简化生命周期和循环；
  - 将 `std::io::Error::new(ErrorKind::Other, ...)` 改为 `std::io::Error::other(...)`；
  - 用已有的扁平化辅助函数替代 `transmute`，或至少显式标注 `transmute::<[[f32; 3]; 4], [f32; 12]>`。

### R4：新增 5 个 reader API 缺少 roundtrip 验证

- 位置：`src/reader.rs:263` 到 `src/reader.rs:325`，以及对应 decode 函数
- 现象：reader 已新增 `read_all_motion()`、`read_all_tyres()`、`read_all_powertrain()`、`read_all_car_state()`、`read_all_other_cars()`，但当前测试数量为 0，没有覆盖这些 API 与 writer 的列布局一致性。
- 影响：类似 tyres size 表错位的问题只能靠人工审计发现，后续字段调整容易再次破坏二进制兼容。
- 建议：
  - 新增不依赖 mock 模块的集成测试，直接构造 `TelemetryFrame`；
  - 至少覆盖 9 个 cluster 的一帧和跨 chunk 多帧 roundtrip；
  - 对数组字段使用非零、非默认值，避免默认值掩盖列错位。

### 复审验证结果

```powershell
cargo check --all-targets
```

结果：通过，但保留 `static_full` dead code warning。

```powershell
cargo test
```

结果：通过，但当前为 0 tests；不作为二进制 roundtrip 正确性的证明。

```powershell
cargo clippy --all-targets -- -D warnings
```

结果：失败，错误见 R3。

## 2026-06-04 最终修复结论

### 结论

复审记录中的 R1 到 R4 已完成修复并通过验证。本次最终状态如下：

- R1 `read_all_tyres()` 与 writer 的 tyres 列尺寸表不一致：已修复。`encode_tyres_chunk()` 的 31 列 `sizes` 表已与 `TYRES_COLUMNS` 顺序及实际 payload 写入尺寸对齐。
- R2 Lap index 在无 footer 文件上仍尝试读取：已修复。reader 现在只在 `footer_offset > 0` 时计算并读取 lap index。
- R3 `cargo clippy --all-targets -- -D warnings` 失败：已修复。严格 clippy 已通过。
- R4 新增 5 个 reader API 缺少 roundtrip 验证：已修复。`tests/binary_roundtrip.rs` 已恢复为不依赖 mock 的集成测试，并覆盖 single frame、single chunk、multi chunk 的 9 cluster roundtrip。

### 额外测试增强

`other_cars` 的 roundtrip 断言已加强，除长度、`active_cars`、`player_car_id` 外，还校验：

- `car_coordinates[0..3]`
- `car_id[0]`

### 最终验证结果

```powershell
cargo test
```

结果：通过。集成测试 `tests/binary_roundtrip.rs` 中 3 个测试全部通过。

```powershell
cargo check --all-targets
```

结果：通过。

```powershell
cargo clippy --all-targets -- -D warnings
```

结果：通过。
