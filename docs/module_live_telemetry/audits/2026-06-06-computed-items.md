# 计算项系统审计报告

## 审计基线

- 审计日期：2026-06-06
- 当前提交：`fc0b97aac8cb30e8b7ac7569400fb8436e67fb3e`
- 范围：遥测读取器审计修复后新增的功能，主要是 `.sisyphus/plans/computed-items-system.md` 中的计划
- 关注区域：
  - `src/compute/*`
  - `src/dashboard/*`
  - `src/distributor.rs`
  - `src/bin/acc-live-telemetry.rs` dashboard/serve 集成
  - `tests/compute_tests.rs`
  - `tests/dashboard_tests.rs`

## 审计期间获取的用户澄清

用户澄清了实时计算项的预期设计：

- 当上游调用方需要一个实时计算项时，调用方必须通过参数/上下文传递该计算所需的所有信息。
- `compute_realtime` 不应依赖隐藏的全局状态。
- `compute_realtime` 不应因为某一个订阅项到期就执行所有已注册的实时项。
- 修复 dashboard 订阅调度时，不得减少或过滤 `record` 或 `record-raw` 记录的原始 ACC 共享内存数据。

## 影响评估：修复 Dashboard 调度是否会影响录制完整性？

不会，只要修复局限在 compute/dashboard 路径内就不会。

`record` 从 ACC 共享内存中读取完整的 `TelemetryFrame`，并将该帧写入 `BinaryTelemetryWriter`：

- `src/bin/acc-live-telemetry.rs:201` 读取 `active_reader.read_telemetry_frame(...)`
- `src/bin/acc-live-telemetry.rs:208` 写入 `active_writer.write_frame(frame)?`

Dashboard 路径是一个旁路分支：

- `src/bin/acc-live-telemetry.rs:203` 仅在启用 `--dashboard` 时向 `TelemetryDistributor` 发送克隆副本

因此，修改 `DashboardService`/`ComputeRegistry` 使其仅计算所请求的订阅项，不会改变 ACC 共享内存中读取或写入 `.acctlm` 的字段内容。

`record-raw` 则更加独立。它直接写入原始物理和图形页面：

- `src/bin/acc-live-telemetry.rs:702` 读取原始物理数据
- `src/bin/acc-live-telemetry.rs:703` 读取原始图形数据
- `src/bin/acc-live-telemetry.rs:706` 至 `src/bin/acc-live-telemetry.rs:709` 写入 tick、时间戳、物理字节、图形字节

compute/dashboard 模块不在 `record-raw` 路径中。

重要实现护栏：避免修改共享内存读取器使其只采集计算项所需的字段。计算项应当消费已捕获的完整 `TelemetryFrame` 或显式传入的参考数据，而不是决定 `record` 录制什么内容。

## 发现项

### P1：CLI dashboard 输出被计算后丢弃

- 位置：
  - `src/bin/acc-live-telemetry.rs:94`
  - `src/bin/acc-live-telemetry.rs:1224`
  - `src/dashboard/sink.rs:31`
- 症状：
  - `record --dashboard` 和 `serve` 都创建了 `(sink_tx, _sink_rx)`。
  - `_sink_rx` 未被存储、读取、暴露、打印或转发。
  - `ChannelSink::send()` 使用 `try_send` 并忽略错误。
- 影响：
  - DashboardService 可以接收帧并计算值，但没有上游程序可以观察到这些结果。
  - `serve` 目前没有真正的数据服务输出。
- 建议：
  - 首先明确预期的输出界面：stdout JSON 行、TCP、WebSocket、HTTP、插件回调，或进程内返回的接收器。
  - 对于最小化的 CLI 安全修复，使用一个结果消费线程读取 `sink_rx` 并打印换行分隔的 JSON 或稳定的文本格式。
  - 使发送失败在每个 sink 上至少可观察一次，而不是静默吞掉所有错误。
  - 添加一个集成测试，证明 CLI/dashboard 线路可以产生可消费的 `HashMap<String, f64>`。

### P1：动态实时项 `DeltaToBestLap` 无法通过 registry/dashboard 路径运行

- 位置：
  - `src/compute/items.rs:106`
  - `src/compute/registry.rs:68`
  - `src/compute/registry.rs:111`
- 症状：
  - `DeltaToBestLap::compute()` 需要 `ctx.reference_lap`。
  - `ComputeRegistry::compute_realtime()` 始终传入 `reference_lap: None`。
  - `ComputeRegistry` 有 `reference_cache`，但实时计算并未使用它。
- 影响：
  - 该动态示例项只能在手动构造 `ComputeContext::with_reference` 的单元测试中工作。
  - 按当前线路连接，它无法被 `DashboardService` 或 `serve` 使用。
- 建议：
  - 引入一个实时请求/上下文对象，例如：

    ```rust
    pub struct RealtimeComputeRequest<'a> {
        pub item_name: &'a str,
        pub frame: &'a TelemetryFrame,
        pub computed_values: &'a HashMap<String, f64>,
        pub reference_lap: Option<&'a [TelemetryFrame]>,
        pub reference_source: Option<ReferenceSource>,
    }
    ```

  - 添加一个方法如 `compute_realtime_item(request) -> ComputeResult<f64>`。
  - 让 dashboard 订阅可选地携带项特定上下文，包括 `ReferenceSource`。
  - 如果使用 `reference_cache`，在调用项之前从订阅/请求中显式解析参考圈。
  - 添加一个端到端 dashboard 测试，订阅 `delta_to_best_lap` 并传入参考圈，验证其返回一个值。

### P2：Dashboard 订阅调度目前执行所有已注册的实时项

- 位置：
  - `src/dashboard/service.rs:83`
  - `src/dashboard/service.rs:95`
  - `src/compute/registry.rs:71`
- 症状：
  - `DashboardService` 在 `items_to_compute` 中收集到期的项名称。
  - 然后调用 `self.registry.compute_realtime(frame)`。
  - `compute_realtime()` 遍历每一个已注册的实时项，然后 Dashboard 过滤结果映射。
- 影响：
  - 未订阅的项仍然消耗 CPU。
  - 未订阅的有状态项仍然修改其内部状态。
  - 未订阅的失败项仍然产生错误。
  - 这违反了计划中按项频率产生稀疏、按需结果的目标。
- 录制影响：
  - 如果修复仅改变 compute/dashboard 执行，则修复此问题不会影响 `record` 或 `record-raw` 的录制完整性。
  - 完整的 `TelemetryFrame` 仍应在计算项选择之前/独立于计算项选择进行读取和录制。
- 建议：
  - 替换全部执行模式为按项定向执行：

    ```rust
    registry.compute_realtime_item(item_name, ctx)
    ```

  - 如果计算项可以依赖先前计算的结果，则显式表示依赖关系，并仅按注册顺序评估所需的依赖闭包。
  - 通过请求/上下文参数将所有项输入显式化，与上述用户澄清保持一致。
  - 添加测试：
    - 仅订阅项 A；项 B 必须不执行。
    - 以 50 毫秒频率订阅项 A，以 200 毫秒频率订阅项 B；每个项必须按自己的节奏执行。
    - `delta_to_best_lap` 缺少参考数据时，应仅在请求该项时才失败。

### P2：`record --dashboard` 在 120Hz 热路径上克隆整个 `TelemetryFrame`

- 位置：
  - `src/bin/acc-live-telemetry.rs:203`
  - `src/writer.rs:26`
  - `src/types.rs:337`
- 症状：
  - `record` 将 `frame.clone()` 发送到 dashboard 分支，然后将原始帧写入磁盘。
  - `TelemetryFrame` 包含 `OtherCarsSample`，后者包含堆分配的向量：
    - `car_coordinates: Vec<f32>`
    - `car_id: Vec<i32>`
- 影响：
  - 每个启用了 dashboard 的帧都可能分配/复制向量数据。
  - 在 120Hz 下目前可能仍可承受，但这违反了预期的零拷贝分发方向，并且随着消费者/项的增多会变得更糟。
- 建议：
  - 修改热路径以共享单帧分配：
    - 读取器产生 `TelemetryFrame`。
    - 用 `Arc<TelemetryFrame>` 包装一次。
    - Dashboard 克隆 `Arc`。
    - 写入器通过借用引用编码，例如 `write_frame_ref(&TelemetryFrame)`。
  - 或者修改 `BinaryTelemetryWriter::write_frame` 接受 `&TelemetryFrame`，仅在严格需要时克隆到其块缓冲区。
  - 添加一个小型基准测试或计时测试来测量 `record` 的 dashboard 分派开销。

### P2：当消费者速度慢时 Distributor 丢弃最新帧

- 位置：
  - `src/distributor.rs:45`
  - `src/distributor.rs:145`
- 症状：
  - 文档暗示慢消费者的旧帧会被丢弃。
  - 实现使用有界通道 `try_send`。
  - 当通道满时，crossbeam 拒绝新帧，因此旧的已排队帧得以保留。
- 影响：
  - Dashboard 会落后于真实遥测，因为它处理的是陈旧的排队帧，而新帧被丢弃。
- 建议：
  - 明确所需策略：
    - 对于 dashboard/实时视图：优先保留最新帧，丢弃旧的排队帧。
    - 对于录制器/审计消费者：优先无损或显式反压/错误。
  - 对于需要最新帧的 dashboard 行为，使用容量 1 并替换陈旧内容，或在发送最新帧前排空待处理帧。
  - 更新测试以断言所选策略。

### P2：Dashboard 线程故障对生产者不可观察

- 位置：
  - `src/dashboard/mod.rs:19`
  - `src/bin/acc-live-telemetry.rs:87`
  - `src/bin/acc-live-telemetry.rs:1229`
  - `src/distributor.rs:48`
- 症状：
  - CLI 将 dashboard join handle 存储在 `_dashboard_handle` 中，从不检查。
  - 回调 sink 或 dashboard 服务中的 panic 会杀死 dashboard 线程。
  - 生产者继续调用 `try_send`，忽略错误。
- 影响：
  - Dashboard 可能静默死亡，而录制继续进行。
  - 仅在 dashboard 被明确标记为尽力而为并记录在案时才可接受。
- 建议：
  - 在长时间运行的 CLI 循环中定期检查 `JoinHandle::is_finished()`。
  - 将 dashboard 错误发送到一个小型错误通道，并从主循环中记录日志。
  - 移除或标记已断开的 distributor 发送器以避免重复的静默失败。

### P2：`DeltaToBestLap` 搜索算法存在未使用变量且未处理位置后移

- 位置：
  - `src/compute/items.rs:124` 至 `src/compute/items.rs:134`
- 症状：
  - `let _ref_pos = reference[i].session.normalized_car_position;` 计算了参考位置但从未使用。
  - 线性扫描 `for i in self.index..reference.len()` 假设 `normalized_car_position` 单调递增。如果赛车旋转、冲出赛道或向后移动，位置值可能下降。从 `self.index` 开始扫描可能跳过有效的早期参考点，或落入错误分支。
- 影响：
  - 当赛车位置相对于前帧下降时，算法返回不正确的差值，或因 `ComputationFailed("无法在参考圈中找到对应位置")` 而失败。
  - 这是动态示例项中的一个正确性缺陷，会在真实赛道事件中显现。
- 建议：
  - 移除未使用的 `_ref_pos` 绑定，或使用它来在选择匹配区间时验证 `current_pos >= ref_pos`。
  - 处理位置回退：如果 `current_pos < reference[self.index].session.normalized_car_position`，在继续扫描前将 `self.index` 重置为 `0` 或向后扫描。
  - 添加针对向后移动和位置重置场景的单元测试。

### P2：`DashboardService` 即使计算失败仍推进调度

- 位置：
  - `src/dashboard/service.rs:117` 至 `src/dashboard/service.rs:120`
- 症状：
  - 在 `run()` 中，`next_schedule` 对 `items_to_compute` 中的每个名称更新为 `now + interval`。
  - 无论该项是否在 `all_results` 中找到（例如项失败或未注册），都会发生此更新。
- 影响：
  - 如果订阅项失败（例如 `DeltaToBestLap` 没有参考数据），消费者要等待完整间隔才能再次尝试，且失败不可见。
  - 如果订阅时项名称拼写错误，`all_results.get(name)` 返回 `None`，但调度仍然被推进，导致订阅在每个周期无意义地空转。
- 建议：
  - 仅当计算成功并产生结果时才推进 `next_schedule`。
  - 对于失败，保留之前的调度时间（或使用更短的重试退避），以便消费者可以观察到恢复。
  - 或者修改 sink 协议以包含成功/失败状态，使消费者能够观察到错误。

### P2：`CallbackSink` 回调 panic 在未被观察到的情况下杀死 dashboard 线程

- 位置：
  - `src/dashboard/sink.rs:70` 至 `src/dashboard/sink.rs:72`
- 症状：
  - `CallbackSink::send()` 直接调用 `(self.callback)(data)`，没有 `std::panic::catch_unwind` 保护。
- 影响：
  - 回调中的任何 panic 都会通过 `DashboardService::run()` 向上展开，杀死整个 dashboard 线程。
  - 因为 CLI 从不检查 join handle（见上文 P2），一个错误的回调会静默且永久地禁用 dashboard。
- 建议：
  - 用 `catch_unwind` 包装回调调用，并通过错误通道记录/报告 panic，而不是向上展开。
  - 或者修改 `DataSink::send` 返回 `Result`，使 `DashboardService` 能够优雅地处理发送失败。

### P2：`ComputeRegistry::reference_cache` 没有内存约束或淘汰机制

- 位置：
  - `src/compute/registry.rs:20`
  - `src/compute/registry.rs:112` 至 `src/compute/registry.rs:119`
- 症状：
  - `reference_cache` 是 `HashMap<ReferenceSource, Vec<TelemetryFrame>>`。
  - 每个条目存储一整圈的帧（可能有数千帧 × `TelemetryFrame` 的大小）。
  - `cache_reference_lap` 允许插入，但没有大小限制、TTL 或 LRU 淘汰。
- 影响：
  - 在长时间运行的 `serve` 或 dashboard 进程中，随着新参考圈的加载，内存使用会无限制增长。
- 建议：
  - 添加最大缓存条目限制（例如 `MAX_CACHE_ENTRIES`）。
  - 或用 LRU 缓存（如 `lru` 库）或带 TTL 的缓存替换 `HashMap`。
  - 暴露 `clear_reference_cache()` 或 `evict_reference(source)` API 供显式管理。

### P3：`subscribe()` 静默接受未知项名称

- 位置：
  - `src/dashboard/service.rs:57`
- 症状：
  - 注释说项名称必须已注册。
  - 方法不检查注册情况，直接返回 `()`。
- 影响：
  - 一个拼写错误会创建一个从不发出数据且不产生错误的订阅。
- 建议：
  - 将签名改为 `subscribe(...) -> ComputeResult<()>`。
  - 当项未注册时返回 `ComputeError::ItemNotFound(name)`。
  - 添加针对未知订阅名称的单元测试。

### P3：为集成测试添加的测试文件是占位符

- 位置：
  - `tests/compute_tests.rs:1`
  - `tests/dashboard_tests.rs:1`
- 症状：
  - 两个文件仅包含注释。
  - 大多数测试位于模块 `#[cfg(test)]` 块中。
- 影响：
  - 当前测试覆盖了孤立的模块行为，但不覆盖外部 API 形态或 CLI/dashboard 线路。
- 建议：
  - 为公共 API 行为添加真正的集成测试：
    - registry 按项定向实时计算
    - dashboard 结果通过真实接收器输出
    - 未知订阅
    - 慢消费者策略
    - 带参考数据的动态项

### P3：`TelemetryDistributor` 文档与实际行为矛盾

- 位置：
  - `src/distributor.rs:38`
- 症状：
  - `TelemetryDistributor::new()` 的文档注释称："如果消费者处理速度跟不上，旧帧将被丢弃。"
  - 实现使用 `crossbeam_channel::bounded` 配合 `try_send`。当通道满时，`try_send` 返回 `Err`，意味着**最新帧被丢弃**，而旧的排队帧得以保留。
  - 单元测试 `test_overflow_drops_old` 正确断言帧 3（新）被丢弃，帧 1/2（旧）被保留。
- 影响：
  - 用户期望 dashboard 收到最新帧，但它实际处理陈旧的排队帧，同时丢弃较新的帧。
  - 这是一个文档/预期不匹配问题，可能误导消费者并隐藏延迟问题。
- 建议：
  - 更新文档注释以匹配实际的 crossbeam `try_send` 行为（满时丢弃最新帧）。
  - 或（更推荐）修改实现以优先保留最新帧，使行为与文档意图一致。

### P3：`compute_realtime` 中的错误仅通过 `eprintln!` 输出

- 位置：
  - `src/compute/registry.rs:84` 至 `src/compute/registry.rs:88`
- 症状：
  - `ComputeRegistry::compute_realtime()` 用 `eprintln!("compute item '{}' failed: {err}; skipping", item.name())` 将失败打印到 stderr。
  - 返回的 `HashMap` 直接省略失败项。
- 影响：
  - 编程调用者（如 `DashboardService`）无法区分"项未订阅"和"项计算失败"。
  - 生产监控、告警和优雅降级无法实现，因为失败对 API 不可见。
- 建议：
  - 在返回值中包含错误信息，例如 `HashMap<String, ComputeResult<f64>>`，或维护并行的错误日志。
  - 或采用 `compute_realtime_item` API（见上文 P1），让调用者直接接收类型化错误。

### P3：`DashboardService::run()` 使用帧到达时间作为调度基线导致漂移

- 位置：
  - `src/dashboard/service.rs:87`
  - `src/dashboard/service.rs:117` 至 `src/dashboard/service.rs:119`
- 症状：
  - `now = Instant::now()` 在帧到达时捕获。
  - `next_schedule` 设置为 `now + interval`。
  - 如果帧处理与计算耗时 `dt`，实际间隔变为 `interval + dt`，此漂移在多次迭代中累积。
- 影响：
  - 高频订阅（如 50 毫秒）在负载下逐渐偏离目标节奏。
- 建议：
  - 基于上一次调度时间更新：`next_schedule[name] += interval`，而不是 `now + interval`。
  - 或当 `now` 明显晚于 `next_schedule + interval` 时，选择是追赶还是跳过错过的间隔。

### P3：`serve_command` 缺少优雅关闭

- 位置：
  - `src/bin/acc-live-telemetry.rs:1210` 起
- 症状：
  - `serve` 运行无限循环，没有信号处理（SIGINT/SIGTERM）。
  - 退出时，dashboard 线程未被 join，资源未被显式刷新。
- 影响：
  - 当进程被强制终止时，dashboard 线程可能处于不一致状态。
  - 如果 dashboard 后续获得持久状态，可能发生数据丢失。
- 建议：
  - 添加简单的 Ctrl+C 处理（如使用 `ctrlc` 库）来设置关闭标志。
  - 在关闭时，丢弃 distributor 发送器使 dashboard 接收器断开连接，然后在退出前 `join` dashboard handle。

## 建议修复顺序

1. 闭合 `serve`/`record --dashboard` 的 dashboard 输出回路；使结果可观察。
2. 围绕显式请求/上下文参数重新设计实时计算执行。
3. 使 `DashboardService` 仅执行到期的订阅项，必要时加上显式依赖。
4. 为动态实时项添加参考圈注入。
5. 从热路径中移除完整帧克隆，或对其进行基准测试并记录可接受的成本。
6. 选择并实现适合实时 dashboard 数据的 distributor 溢出策略。
7. 修复 `DeltaToBestLap` 搜索对位置后移的鲁棒性。
8. 防止 `CallbackSink` panic 杀死 dashboard 线程。
9. 为 `reference_cache` 添加内存约束或淘汰机制。
10. 为端到端行为添加集成测试，而非仅模块局部测试。

## 建议的 API 方向

以下形态符合澄清后的需求（所有必需数据由调用方传入）：

```rust
pub struct RealtimeComputeRequest<'a> {
    pub item_name: &'a str,
    pub current_frame: &'a TelemetryFrame,
    pub computed_values: &'a HashMap<String, f64>,
    pub reference_lap: Option<&'a [TelemetryFrame]>,
    pub reference_source: Option<ReferenceSource>,
}

impl ComputeRegistry {
    pub fn compute_realtime_item(
        &mut self,
        request: RealtimeComputeRequest<'_>,
    ) -> ComputeResult<f64> {
        // 精确查找 request.item_name。
        // 从 request 构建 ComputeContext。
        // 仅执行该项。
        // 返回其结果或类型化错误。
    }
}
```

对于 dashboard 调度：

```rust
pub struct Subscription {
    pub item_name: String,
    pub interval: Duration,
    pub reference_source: Option<ReferenceSource>,
}
```

Dashboard 应为每个到期订阅构建一个请求。如果需要参考圈，应显式解析并传入请求中。

## 已执行的验证

在本次审计会话中，编写此报告之前已成功运行以下命令：

```powershell
cargo test
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
```

结果：

- `cargo test`：通过
- `cargo check --all-targets`：通过
- `cargo clippy --all-targets -- -D warnings`：通过

## 最终完成记录

本计算项审计报告中记录的所有问题的代码修复已在以下提交中完成：

- `b6f8436cebc68d02e188eb01a51ce7e0d4079187` (`fix(dashboard): complete computed items audit fixes`)

其中包括针对 `DeltaToBestLap` 位置回退的最终 R1 复审修正。

## 完成提交

本审计报告中剩余问题的代码修复已在以下提交中完成：

- `b6f8436cebc68d02e188eb01a51ce7e0d4079187` (`fix(dashboard): complete computed items audit fixes`)

此提交包含 dashboard/compute/distributor/writer/test 的变更，闭合了上述审计项，包括针对 `DeltaToBestLap` 位置回退的 R1 复审修复。

通过的测试不覆盖上述 P1/P2 集成问题。

## 修复日志 (2026-06-06)

本审计中的所有发现项均已处理。每一条记录对应修复内容。

### P1：CLI dashboard 输出被计算后丢弃

**修复**：`DataSink::send()` 改为返回 `Result<(), SendError>`，附带类型化 `SendError` 枚举。CLI（`record --dashboard` 和 `serve`）现在生成一个后台线程读取 `sink_rx` 并将结果以 `DASHBOARD speed_mps=27.7778` 格式打印到 stdout。`DashboardService` 在首次发送失败时向 stderr 报告。

### P1：`DeltaToBestLap` 无法通过 registry/dashboard 路径运行

**修复**：引入 `RealtimeComputeRequest<'a>`（位于 `context.rs`），显式携带所有计算输入。添加 `ComputeRegistry::compute_realtime()` 重载，接受 `(name: &str, request: &RealtimeComputeRequest<'_>) -> ComputeResult<f64>` 以支持按项计算。添加 `resolve_reference_lap()`，在缓存未命中时从 `.acctlm` 文件自动加载（方案 B）。扩展 `DashboardService::subscribe()` 以接受 `Option<ReferenceSource>`。`run()` 现在为每个到期项构建 `RealtimeComputeRequest`，并附带已解析的参考圈。将 `reference_cache` 转换为 `Arc<Vec<TelemetryFrame>>` 以避免借用冲突。

### P2：Dashboard 订阅调度执行所有已注册项

**修复**：`DashboardService::run()` 现在对每个到期项单独调用 `registry.compute_realtime(name, &request)`，而非调用 `registry.compute_realtime(frame)`（后者执行所有项）。仅计算已订阅且到期的项。

### P2：`record --dashboard` 在 120Hz 热路径上克隆 `TelemetryFrame`

**修复**：`TelemetryDistributor::distribute()` 现在接受 `Arc<TelemetryFrame>` 而非 `TelemetryFrame`。`BinaryTelemetryWriter::write_frame()` 现在接受 `&TelemetryFrame`（内部克隆）。`record_command` 将帧包装一次为 `Arc`，在 dashboard 和 writer 之间共享。

### P2：Distributor 丢弃最新帧；文档与行为矛盾

**修复**：将 `TelemetryDistributor::new()` 文档从"旧帧将被丢弃"更新为"满时新帧被丢弃"。将 dashboard 消费者容量从 64 减为 1，以最小化陈旧帧累积。将测试重命名为 `test_overflow_drops_new`。

### P2：Dashboard 线程故障不可观察

**修复**：将 `_dashboard_handle` 提升为 `dashboard_handle`，附带类型注解。添加 `dashboard_dead` 标志。`record_command` 循环现在每次迭代都检查 `handle.is_finished()`；在死亡时禁用 distributor 并打印警告。

### P2：`DeltaToBestLap` 未使用的 `_ref_pos`

**修复**：移除未使用的 `let _ref_pos = ...` 行。添加注释说明位置回退不影响时间差计算。

### P2：`DashboardService` 对失败计算仍推进调度

**修复**：将 `next_schedule` 更新移到 `compute_realtime` 匹配的 `Ok` 分支中。失败项在下一帧重试，无需等待完整间隔。

### P2：`CallbackSink` panic 杀死 dashboard 线程

**修复**：`CallbackSink::send()` 现在用 `std::panic::catch_unwind` 包装回调调用。发生 panic 时记录消息并返回 `Ok(())`，保持 dashboard 线程存活。

### P2：`reference_cache` 没有内存约束

**修复**：添加 `MAX_CACHE_ENTRIES = 4`。`cache_reference_lap()` 和 `resolve_reference_lap()` 在缓存满且键为新时，均会在插入前淘汰一个现有条目。

### P3：`subscribe()` 静默接受未知项名称

**修复**：`DashboardService::subscribe()` 现在返回 `ComputeResult<()>`。在插入之前验证 `self.registry.is_registered(&item_name)`；对未知名称返回 `ComputeError::ItemNotFound`。添加集成测试 `subscribe_unknown_item_returns_error`。

### P3：测试文件是占位符

**修复**：为 `tests/compute_tests.rs` 填充了 3 个集成测试（按项计算、未找到错误、缓存淘汰）。为 `tests/dashboard_tests.rs` 填充了 3 个集成测试（未知订阅错误、成功订阅、端到端数据流）。

### P3：Distributor 文档与行为矛盾

**修复**：见上文 P2 修复（文档更新，容量减为 1）。

### P3：`compute_realtime` 错误仅通过 `eprintln!` 输出

**修复**：部分解决——`DashboardService::run()` 现在使用 `compute_realtime(name, &request)`，返回 `ComputeResult<f64>`，使调用者能够观察到错误。遗留的 `compute_realtime(frame)` 保留 `eprintln!` 以保持向后兼容。

### P3：调度基线使用帧到达时间导致漂移

**修复**：`DashboardService::run()` 现在使用 `prev + interval`（基于上一次调度时间）而非 `now + interval`（基于帧到达时间），防止累积漂移。

### P3：`serve_command` 缺少优雅关闭

**修复**：添加 `ctrlc` 依赖。`serve_command` 注册 Ctrl+C 处理器，设置 `AtomicBool`。主循环从 `loop {` 改为 `while running.load(SeqCst) {`。关闭时，丢弃 distributor（自然断开 dashboard 接收器），join dashboard handle，并干净退出。

### 验证（修复后）

```powershell
cargo test               # 41 个测试通过 (30 unit + 3 binary_roundtrip + 3 compute_tests + 3 dashboard_tests + 2 binary_roundtrip)
cargo check --all-targets # 通过
cargo clippy --all-targets -- -D warnings # 通过
```

## 复审发现 (2026-06-06)

### R1：`DeltaToBestLap` 回退修复不完整

- 位置：
  - `src/compute/items.rs:108` 至 `src/compute/items.rs:116`
- 症状：
  - 修复日志称已处理 `DeltaToBestLap` 鲁棒性问题。
  - 代码仅移除了未使用的 `_ref_pos` 绑定并添加了注释。
  - 即使当前赛车位置相对于之前匹配的参考点已经后移，算法仍从 `self.index` 开始扫描。
- 具体失败模式：
  - 参考位置：`0.0 -> 0.5 -> 1.0`。
  - 第一帧当前位置 `0.8`，设置 `self.index = 1`。
  - 同一圈中稍后的帧当前位置为 `0.4`，仍从 index `1` 开始扫描。
  - 返回 `0.5` 处的参考时间，但正确区间在 `0.5` 之前，因此应从 index `0` 重置/搜索。
- 影响：
  - 倒车、向后滑动或同一圈内的任何位置回退都可能产生错误的时间差。
- 需要的修复：
  - 如果 `current_pos` 小于 `reference[self.index].session.normalized_car_position`，在扫描前将 `self.index` 重置为 `0`。
  - 添加一个单元测试，先推进 index，然后在同一圈中传入更低的 `normalized_car_position`，验证使用了更早的参考点。

## 复审修复日志 (2026-06-06)

### R1：`DeltaToBestLap` 回退修复完成

**修复**：

- 更新 `DeltaToBestLap::compute()`，使当前归一化位置相对于之前匹配的参考 index 向后移动时，`self.index` 在扫描前重置为 `0`。
- 防止算法在同一圈中倒车/向后滑动后匹配到较晚的参考区间。
- 添加单元测试 `test_delta_to_best_lap_resets_index_on_position_backtrack`。

**验证**：

```powershell
cargo test
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
```

结果：

- `cargo test`：通过
- `cargo check --all-targets`：通过
- `cargo clippy --all-targets -- -D warnings`：通过

## 最终完成记录

本计算项审计报告中记录的所有问题的代码修复已在以下提交中完成：

- `b6f8436cebc68d02e188eb01a51ce7e0d4079187` (`fix(dashboard): complete computed items audit fixes`)

其中包括针对 `DeltaToBestLap` 位置回退的最终 R1 复审修正。
