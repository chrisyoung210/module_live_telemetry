# acctlm 文件导入 API 参考

> **相关文档**: [extract-telemetry.md](extract-telemetry.md) · [recording.md](recording.md#recordingoutcome)

将当前 `.acctlm2` 文件或旧版本软件录制的 `.acctlm` 文件解析为 [`RecordingOutcome`](recording.md#recordingoutcome)，
供主模块（m1）在导入历史录制时使用。

---

## 1. 快速开始

```rust
use module_live_telemetry::recording::parse_acctlm_file;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let outcome = parse_acctlm_file("recording_20260606.acctlm2")?;

    println!("赛道: {}", outcome.track_name);
    println!("车辆: {}", outcome.car_model);
    println!("赛段: {}", outcome.session_type);
    println!("帧数: {}", outcome.total_samples);
    println!("时长: {:.1}s", outcome.duration.as_secs_f64());
    println!("日期: {}", outcome.recording_date);   // "YYYY/MM/DD"
    println!("时间: {}", outcome.recording_time);   // "HH:MM:SS"
    println!("圈数: {}", outcome.laps.len());
    println!("路径: {}", outcome.file_path.display());
    println!("大小: {} bytes", outcome.file_size_bytes);

    for lap in &outcome.laps {
        if lap.is_valid {
            if let Some(t) = lap.lap_time {
                println!("  L{}: {:.3}s", lap.lap_number, t.as_secs_f64());
            }
        }
    }

    Ok(())
}
```

---

## 2. 函数签名

```rust
pub fn parse_acctlm_file(path: impl AsRef<Path>) -> TelemetryResult<RecordingOutcome>
```

**参数**：
- `path` — `.acctlm2` 或 `.acctlm` 文件路径。可以是 `&str`、`String`、`Path`、`PathBuf` 等实现 `AsRef<Path>` 的类型。

**返回值**：
- `Ok(RecordingOutcome)` — 文件合法，解析成功。
- `Err(TelemetryError)` — 文件不合法或无法读取。

---

## 3. 验证规则

函数不会做表面校验（如文件后缀），而是通过解析文件内部结构来验证合法性：

| 校验项 | 失败原因 | 错误类型 |
|---|---|---|
| 文件可读取 | 不存在 / 无权限 | `TelemetryError::Io` |
| Magic 字节 | 不是 V2 `b"ACT2"`，也不是 V1 `b"ACTL\r\n\x1A\n"` | `TelemetryError::InvalidFormat` |
| Header offsets | `schema_offset < HEADER_SIZE` 或 offset 顺序矛盾 | `TelemetryError::InvalidFormat` |
| First chunk offset | 超出文件末尾 | `TelemetryError::InvalidFormat` |
| Schema hash | 与编译期 `SCHEMA_HASH` 不匹配 | `TelemetryError::InvalidFormat` |
| 格式版本 | `version != FORMAT_VERSION` | `TelemetryError::UnsupportedVersion` |
| Metadata block | magic 不是 `b"META"` | `TelemetryError::InvalidFormat` |
| Index block | magic 不是 `b"INDX"` | `TelemetryError::InvalidFormat` |

任一校验失败即返回错误，不做部分解析。

---

## 4. 返回字段说明

详见 [RecordingOutcome](recording.md#recordingoutcome)。由 `parse_acctlm_file` 填充时有以下注意点：

| 字段 | 来源 |
|---|---|
| `track_name` | 从文件 Metadata block 读取 |
| `car_model` | 从文件 Metadata block 读取 |
| `session_type` | 从文件 v5 扩展字段读取，通过 `session_type_label()` 映射 |
| `session_type_raw` | 原始值（0-8），旧文件缺省时为 0 |
| `file_path` | 传入的 `path` 参数规范化为 `PathBuf` |
| `file_size_bytes` | 从 `RecordingSummary.total_bytes` 读取 |
| `total_samples` | 从 `RecordingSummary.total_samples` 读取 |
| `duration` | 估算值：`total_samples / poll_hz`（文件不存实际墙钟时长） |
| `recording_date` | 由 `created_unix_ns` 按 UTC+8 计算 |
| `recording_time` | 由 `created_unix_ns` 按 UTC+8 计算 |
| `laps` | 从 session/timing 数据聚合；`lap_time` 使用 ACC 的 `i_last_time`，`split_times` 恒为空 |

---

## 5. 圈速说明

- `LapSummary.lap_time` 来自 ACC timing 数据中的 `i_last_time`，单位毫秒；缺少有效 timing 数据时为 `None`。
- `LapSummary.split_times` 在导入场景下始终为空。扇区级别的分段时间未存储在 Lap Index 中。
- `LapSummary.is_valid` 直接对应 ACC 判定（`is_valid != 0`）。
- Out lap（`is_out_lap != 0`）的 `lap_time` 为 `None`。

---

## 6. 与 RecordingController 的关系

| | `RecordingController` | `parse_acctlm_file` |
|---|---|---|
| 用途 | 实时录制 | 历史文件导入 |
| 输入 | `RecordingRequest` + 遥测源 | 文件路径 |
| 输出 | 通过 channel 发送 `RecordingOutcome` | 同步返回 `RecordingOutcome` |
| 会话类型来源 | 录制时设为 `"UNKNOWN"` | 从文件中读取 |
| 圈速来源 | 录制结束后解析文件并聚合 | 从 session/timing 数据聚合 |
| 调用方 | m1（录制控制） | m1（导入管理） |

---

## 7. 错误处理示例

```rust
use module_live_telemetry::recording::parse_acctlm_file;
use module_live_telemetry::TelemetryError;

match parse_acctlm_file("unknown.acctlm2") {
    Ok(outcome) => {
        // 导入成功，由主模块管理 session 元数据
        save_to_database(&outcome);
    }
    Err(TelemetryError::Io(e)) => {
        eprintln!("文件读取失败: {}", e);
    }
    Err(TelemetryError::InvalidFormat(msg)) => {
        eprintln!("文件格式不合法: {}", msg);
    }
    Err(TelemetryError::UnsupportedVersion(v)) => {
        eprintln!("不支持的格式版本: {}", v);
    }
    Err(e) => {
        eprintln!("未知错误: {}", e);
    }
}

---

## 8. 相关 API

- [按圈提取遥测数据 (`extract_lap_telemetry`)](extract-telemetry.md) — 解析文件后按圈提取指定 raw 字段的逐帧数据
```
