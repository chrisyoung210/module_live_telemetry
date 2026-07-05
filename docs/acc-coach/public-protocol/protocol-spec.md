# ACC Coach — Network Remote Dashboard 协议规范

版本: 1.0  
日期: 2026-06-18  
面向: Network Remote Dashboard 设备端开发者  
桌面端: ACC Coach Desktop (本仓库)

---

## 1. 架构概览

ACC Coach 桌面端与 Remote Dashboard 设备之间通过 **三层分离** 的协议进行通信：

```
┌─────────────────────────────────────────────────────────────────┐
│                        ACC Coach Desktop                        │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ Discovery    │  │ TCP Control  │  │ UDP Data Sender      │  │
│  │ (UDP :20776) │  │ (TCP :20778) │  │ (UDP → device:20779) │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                      │              │
└─────────┼─────────────────┼──────────────────────┼──────────────┘
          │                 │                      │
          ▼                 ▼                      ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Remote Dashboard Device                      │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ Discovery    │  │ TCP Control  │  │ UDP Data Receiver     │  │
│  │ Advertiser   │  │ Server       │  │ (:20779)              │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

| 层级 | 传输 | 默认端口 | 方向 | 职责 |
|---|---|---|---|---|
| **Discovery** | UDP | `20776` | 双向 | 设备宣告存在、桌面端探测 |
| **Control** | TCP | `20778` | 桌面端连设备 | 握手、配对、配置、状态、心跳 |
| **Data** | UDP | `20779` | 桌面端 → 设备 | 高频 telemetry 流 |

**核心原则：**

- **TCP 连接是在线状态的权威来源** — 设备是否 connected 由 TCP control session 决定
- **UDP 只传可丢弃的实时数据** — telemetry 允许丢帧，layout/配置/资源必须走 TCP
- **Discovery 只发现候选设备，不自动信任** — 必须经过 TCP handshake + pairing

---

## 2. Discovery 层 (UDP :20776)

### 2.1 设备 Announce

设备端启动后，**每 1~2 秒** 向广播地址 `255.255.255.255:20776` 发送 announce。桌面端据此发现设备。

```json
{
  "schema": "acc-coach.remote-dashboard.discovery.v1",
  "type": "announce",
  "protocolVersion": 1,
  "deviceId": "rd-01HR7YM3Q3X5K4Z9T2W8P6",
  "deviceName": "Pit Tablet",
  "instanceId": "boot-01HR7Z0W9J6N3V",
  "appVersion": "0.1.0",
  "platform": "android",
  "control": { "transport": "tcp", "port": 20778 },
  "data": { "transport": "udp", "port": 20779 },
  "capabilities": {
    "maxHz": 60,
    "encodings": ["json", "binary_v1"],
    "layoutAssets": true,
    "maxLayoutBytes": 26214400,
    "maxUdpPayloadBytes": 1200
  }
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `deviceId` | string | **稳定设备 ID**，卸载重装前保持不变 |
| `instanceId` | string | **本次进程启动 ID**，重启后变化，桌面端用此识别旧 session 已失效 |
| `deviceName` | string | 用户可读名称 |
| `control.port` | u16 | 设备 TCP control server 监听端口 |
| `data.port` | u16 | 设备 UDP telemetry receiver 监听端口 |
| `capabilities.maxHz` | u32 | 设备最大刷新率 (Hz) |
| `capabilities.encodings` | string[] | 支持的编码: `"json"`, `"binary_v1"` |
| `capabilities.maxUdpPayloadBytes` | u16 | 设备能处理的最大 UDP payload (建议 1200) |

### 2.2 桌面端 Probe

桌面端在启动、打开 Remote Dashboard 页面、或用户点击刷新时发送 probe：

```json
{
  "schema": "acc-coach.remote-dashboard.discovery.v1",
  "type": "probe",
  "protocolVersion": 1,
  "desktopInstanceId": "desktop-01HR7Z12ABCD",
  "requestedCapabilities": ["control_tcp", "data_udp"]
}
```

**设备收到 probe 后应立即回复一个 announce**，不必等下一次周期 announce。

### 2.3 设备超时

桌面端 discovery registry 按以下规则管理设备：

| 最后 seen | 状态 |
|---|---|
| ≤ 5s | `discovered` (活跃) |
| 5s ~ 15s | `stale` (即将过期) |
| > 15s | 从活跃列表移除 |

---

## 3. Control 层 (TCP :20778)

### 3.1 TCP Framing

所有 control 消息使用 **length-prefixed JSON frame**：

```
┌────────────────┬──────────────────────┐
│  u32_be length │  utf8_json_payload   │
│    (4 bytes)   │    (length bytes)    │
└────────────────┴──────────────────────┘
```

- 普通命令 frame: 最大 256 KiB
- Layout asset chunk frame: 最大 1 MiB
- 超限立即返回 protocol error 并断开连接

**实现提示：**
```
// 读取: 先读 4 字节 u32_be 获取长度，再读 length 字节
// 写入: 先写 4 字节 u32_be 长度，再写 JSON payload
```

### 3.2 握手 (Handshake)

TCP 连接建立后，**桌面端先发 `hello`，设备回复 `helloAck`**。

**桌面端 → 设备:**
```json
{
  "schema": "acc-coach.remote-dashboard.control.v1",
  "type": "hello",
  "messageId": "msg-hi-1",
  "protocolVersion": 1,
  "desktopInstanceId": "desktop-01HR7Z12ABCD",
  "appVersion": "0.1.0",
  "supportedEncodings": ["json", "binary_v1"],
  "supportedDataTransports": ["udp"],
  "requestedDeviceId": "rd-01HR7YM3Q3X5K4Z9T2W8P6"
}
```

**设备 → 桌面端 (成功):**
```json
{
  "schema": "acc-coach.remote-dashboard.control.v1",
  "type": "helloAck",
  "replyTo": "msg-hi-1",
  "messageId": "msg-2",
  "protocolVersion": 1,
  "deviceId": "rd-01HR7YM3Q3X5K4Z9T2W8P6",
  "deviceName": "Pit Tablet",
  "instanceId": "boot-01HR7Z0W9J6N3V",
  "capabilities": {
    "maxHz": 60,
    "encodings": ["json", "binary_v1"],
    "layoutAssets": true,
    "maxLayoutBytes": 26214400,
    "maxUdpPayloadBytes": 1200
  },
  "data": { "transport": "udp", "port": 20779 }
}
```

**设备 → 桌面端 (版本不兼容):**
```json
{
  "type": "error",
  "replyTo": "msg-hi-1",
  "code": "unsupported_protocol",
  "message": "Protocol version 1 is not supported"
}
```

### 3.3 消息格式约定

所有 control 消息遵循 **request/reply** 语义：

- 需要确认的请求消息包含 `messageId` (唯一请求 ID)
- 回复消息包含 `replyTo` (对应的请求 messageId)
- 成功回复使用带 `replyTo` 的类型特定消息或通用 `ack`
- 失败回复使用 `error` 消息

**通用 ACK:**
```json
{
  "type": "ack",
  "replyTo": "msg-10",
  "messageId": "msg-11",
  "status": "ok"
}
```

**通用 Error:**
```json
{
  "type": "error",
  "replyTo": "msg-10",
  "messageId": "msg-11",
  "code": "layout_asset_missing",
  "message": "Font asset font-aabbcc was not received"
}
```

### 3.4 配对 (Pairing)

首次连接需要配对。已配对设备在后续连接时用 trust token 认证。

**首次配对流程：**

1. 桌面端发 `pairRequest`
2. 设备回复 `pairResult` (paired=true + trustToken)
3. 双方保存 trust token

**桌面端 → 设备:**
```json
{
  "type": "pairRequest",
  "messageId": "msg-3",
  "desktopName": "ACC Coach Desktop",
  "desktopInstanceId": "desktop-01HR7Z12ABCD"
}
```

**设备 → 桌面端:**
```json
{
  "type": "pairResult",
  "replyTo": "msg-3",
  "messageId": "msg-4",
  "paired": true,
  "deviceId": "rd-01HR7YM3Q3X5K4Z9T2W8P6",
  "trustToken": "opaque-token-issued-by-device"
}
```

**后续连接认证:**
```json
{
  "type": "authenticate",
  "messageId": "msg-5",
  "deviceId": "rd-01HR7YM3Q3X5K4Z9T2W8P6",
  "trustToken": "opaque-token-issued-by-device"
}
```

**安全说明：**
- 首版使用 opaque token，重点是避免任意 LAN 设备自动接收 telemetry
- trust token 由设备端生成，桌面端存储
- 后续可升级为 challenge-response

### 3.5 Layout 与资源传输

Layout 和资源 **必须走 TCP**，分三步：

**步骤 1: prepareLayout**

桌面端发送 layout 元数据和 asset manifest：

```json
{
  "type": "prepareLayout",
  "messageId": "msg-20",
  "layoutId": "layout-main",
  "layoutVersion": "abc123def456...",
  "canvas": { "width": 1280, "height": 720 },
  "dynamicControls": [
    {
      "controlId": "speed",
      "fieldRefs": ["speedKmh"],
      "refreshHz": 30,
      "widgetType": "text",
      "textTemplate": "Speed: {{value}} km/h",
      "textFormat": "%.1f"
    }
  ],
  "assets": [
    {
      "assetId": "font-aabbcc",
      "kind": "font",
      "mime": "font/ttf",
      "sha256": "aabbcc...",
      "byteLength": 124000
    }
  ]
}
```

设备收到后应检查哪些 assets 已有缓存，可回复 `alreadyHaveAssets`。

**dynamicControl 字段：**

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `controlId` | string | 是 | 控件唯一标识 |
| `fieldRefs` | string[] | 是 | 绑定 telemetry 字段列表 |
| `refreshHz` | number | 否 | 期望更新频率，默认 30 |
| `widgetType` | string | 否 | 控件类型：`"text"`、`"chart"`、`"map"`、`"gear"` 等 |
| `textTemplate` | string | 否 | text widget 的显示模板，支持 `{{value}}` 和 `{{expr:...}}` 表达式 |
| `textFormat` | string | 否 | text widget 的数值格式化串，如 `"%.1f"`、`"%.0f"` |

`textTemplate` 和 `textFormat` 仅对 `widgetType` 为 `"text"` 的控件有效。
字段缺失时设备端按 `fieldRefs` 渲染原始值。

**步骤 2: putAssetChunk**

桌面端按 asset sha256 分 chunk 发送资源（字体、图片等）：

```json
{
  "type": "putAssetChunk",
  "messageId": "msg-21",
  "assetId": "font-aabbcc",
  "sha256": "aabbcc...",
  "chunkIndex": 0,
  "chunkCount": 4,
  "base64": "..."
}
```

**步骤 3: commitLayout**

```json
{
  "type": "commitLayout",
  "messageId": "msg-25",
  "layoutId": "layout-main",
  "layoutVersion": "abc123def456..."
}
```

**设备回复:**
```json
{
  "type": "layoutReady",
  "replyTo": "msg-25",
  "messageId": "msg-26",
  "layoutId": "layout-main",
  "layoutVersion": "abc123def456...",
  "missingAssets": [],
  "warnings": []
}
```

**规则：**
- 设备应缓存 asset sha256，已有资源不重复传输
- 任一 asset sha256 校验失败 → 返回 `asset_hash_mismatch` error
- `commitLayout` 成功前 **不得启动 stream**

### 3.6 Stream Profile

桌面端下发数据 profile，描述要发哪些 channel、以什么频率和编码：

```json
{
  "type": "setStreamProfile",
  "messageId": "msg-30",
  "profile": {
    "profileId": "main-60hz",
    "hz": 60,
    "encoding": "binary_v1",
    "mode": "snapshot_delta",
    "channels": [
      "speedKmh", "gear", "rpm", "throttlePct",
      "brakePct", "currentLapTimeMs", "bestLapDeltaTimeMs"
    ],
    "keyframeIntervalMs": 1000
  }
}
```

| 字段 | 说明 |
|---|---|
| `mode` | `"snapshot"` (每包全量) 或 `"snapshot_delta"` (delta + 周期 keyframe) |
| `keyframeIntervalMs` | delta 模式下 keyframe 间隔 (毫秒) |

### 3.7 Start / Stop Stream

**桌面端 → 设备:**
```json
{
  "type": "startStream",
  "messageId": "msg-40",
  "sessionId": "sess-01HR7ZABCDE",
  "layoutId": "layout-main",
  "profileId": "main-60hz",
  "data": { "transport": "udp", "targetIp": "192.168.1.50", "targetPort": 20779 }
}
```

**设备 → 桌面端 (确认):**
```json
{
  "type": "streamStarted",
  "replyTo": "msg-40",
  "messageId": "msg-41",
  "sessionId": "sess-01HR7ZABCDE",
  "acceptedHz": 60,
  "acceptedEncoding": "binary_v1",
  "udpReceivePort": 20779
}
```

桌面端 **只有收到 `streamStarted` 后才开始发送 UDP telemetry**。

**停止:**
```json
{ "type": "stopStream", "messageId": "msg-50", "sessionId": "sess-01HR7ZABCDE" }
```

### 3.8 设备状态上报

设备 **每 1 秒** 通过 TCP 上报 `status`；状态变化时 **立即上报**：

```json
{
  "type": "status",
  "messageId": "msg-100",
  "deviceId": "rd-01HR7YM3Q3X5K4Z9T2W8P6",
  "sessionId": "sess-01HR7ZABCDE",
  "state": "streaming",
  "layout": {
    "layoutId": "layout-main",
    "layoutVersion": "abc123def456...",
    "ready": true
  },
  "data": {
    "lastSequence": 12888,
    "receivedPackets": 12800,
    "droppedPackets": 88,
    "receiveHz": 59.7,
    "lastPacketAgeMs": 12
  },
  "render": {
    "fps": 60,
    "lastFrameAgeMs": 14,
    "warnings": []
  },
  "battery": {
    "levelPct": 76,
    "charging": true
  }
}
```

`data` 段中的丢包统计对桌面端链路质量展示至关重要。

### 3.9 心跳 (Heartbeat)

```
桌面端每 2s 发 ping → 设备 2s 内回 pong → 连续 3 次超时 → 桌面端断开
```

**Ping:**
```json
{ "type": "ping", "messageId": "msg-200", "timeMs": 1781690000000 }
```

**Pong:**
```json
{ "type": "pong", "replyTo": "msg-200", "messageId": "msg-201", "deviceTimeMs": 1781690000011 }
```

---

## 4. Data 层 (UDP :20779)

### 4.1 Binary Packet Header

UDP data 包使用固定 binary header，便于快速解析：

```
┌──────────────────────────────────────────────────────────────────┐
│ Offset │ Size  │ Field             │ Description                 │
├────────┼───────┼───────────────────┼─────────────────────────────┤
│   0    │   4   │ magic             │ "ACCD" (0x41 0x43 0x43 0x44)│
│   4    │   1   │ protocol_version  │ 1                           │
│   5    │   1   │ header_len        │ 34 (header bytes after magic)│
│   6    │   2   │ flags             │ Reserved (0), big-endian     │
│   8    │   8   │ session_id_hash   │ Hash of TCP sessionId (BE)   │
│  16    │   4   │ stream_id         │ Stream identifier (BE)       │
│  20    │   8   │ sequence          │ Monotonic, wraps (BE)        │
│  28    │   8   │ sent_unix_ms      │ Sender timestamp ms (BE)     │
│  36    │   1   │ payload_type      │ See table below              │
│  37    │   1   │ encoding          │ 0x01=json, 0x02=binary_v1   │
│  38    │   2   │ payload_len       │ Payload bytes (BE)           │
│  40    │   N   │ payload           │ Variable-length payload      │
└────────┴───────┴───────────────────┴─────────────────────────────┘
```

总 header 固定 40 字节 (含 magic)。

**Payload Type:**
| 值 | 含义 |
|---|---|
| `0x01` | `telemetry_snapshot` — 全量 telemetry |
| `0x02` | `telemetry_delta` — 增量 (仅变化字段) |
| `0x03` | `keyframe` — 完整 snapshot (delta 模式下的锚点) |
| `0x04` | `layout_values` — dashboard 动态控件值 |

### 4.2 JSON Payload

JSON 编码适合调试和早期兼容：

```json
{
  "kind": "snapshot",
  "sampleTick": 9234,
  "timestampMs": 1781690123456,
  "values": {
    "speedKmh": 243.1,
    "gear": 5,
    "rpm": 7210,
    "throttlePct": 98.5,
    "brakePct": 0.0
  }
}
```

### 4.3 Binary Payload (binary_v1)

用于降低带宽和解析成本。具体 encoding 待完整版本定义，当前建议优先实现 JSON。

### 4.4 MTU 限制

- 默认 UDP payload **不超过 1200 bytes**，避免 IP fragmentation
- 设备应在 announce 的 `capabilities.maxUdpPayloadBytes` 中声明自己的上限

### 4.5 Snapshot / Delta / Keyframe

| 模式 | 策略 |
|---|---|
| `snapshot` | 每个 UDP 包都包含所有 requested channels，最简单 |
| `snapshot_delta` | 高频发 delta，每 `keyframeIntervalMs` 发一次完整 keyframe |

**设备端处理：**
- 检测到 sequence gap → **不请求重传**，等待下一次 keyframe
- 旧 telemetry 帧没有价值，始终使用最新到达的数据渲染

---

## 5. 完整消息目录

### 5.1 Discovery 层

| type | 方向 | 说明 |
|---|---|---|
| `announce` | 设备 → 桌面端 | 设备宣告存在 |
| `probe` | 桌面端 → 设备 | 请求设备立即回复 announce |

### 5.2 Control 层

| type | 方向 | 说明 |
|---|---|---|
| `hello` | 桌面端 → 设备 | 发起握手 |
| `helloAck` | 设备 → 桌面端 | 握手确认 |
| `pairRequest` | 桌面端 → 设备 | 请求配对 |
| `pairResult` | 设备 → 桌面端 | 配对结果 |
| `authenticate` | 桌面端 → 设备 | trust token 认证 |
| `ack` | 双向 | 通用确认 |
| `error` | 双向 | 通用错误 |
| `prepareLayout` | 桌面端 → 设备 | 发送 layout 元数据 |
| `putAssetChunk` | 桌面端 → 设备 | 发送资源 chunk |
| `commitLayout` | 桌面端 → 设备 | 提交并校验 layout |
| `layoutReady` | 设备 → 桌面端 | layout 就绪 |
| `setStreamProfile` | 桌面端 → 设备 | 配置数据流 profile |
| `startStream` | 桌面端 → 设备 | 启动 telemetry 流 |
| `streamStarted` | 设备 → 桌面端 | 流启动确认 |
| `stopStream` | 桌面端 → 设备 | 停止流 |
| `pauseStream` | 桌面端 → 设备 | 暂停流 |
| `resumeStream` | 桌面端 → 设备 | 恢复流 |
| `getStatus` | 桌面端 → 设备 | 查询设备状态 |
| `status` | 设备 → 桌面端 | 设备状态上报 |
| `setDeviceName` | 桌面端 → 设备 | 修改设备名称 |
| `ping` | 桌面端 → 设备 | 心跳请求 |
| `pong` | 设备 → 桌面端 | 心跳回复 |

---

## 6. 设备状态机

```
                    ┌─────────┐
                    │ Unknown │ (初始/无发现)
                    └────┬────┘
                         │ 收到 probe / 发送 announce
                         ▼
                    ┌───────────┐
                    │Discovered │
                    └─────┬─────┘
                          │ TCP handshake
                          ▼
                    ┌───────────┐
                    │Connecting │
                    └─────┬─────┘
                          │ helloAck ok
                    ┌──────┴──────┐
                    │             │
               (未配对)       (已配对)
                    │             │
                    ▼             ▼
           ┌──────────────┐  ┌─────────┐
           │PairingRequired│  │Connected│
           └──────┬───────┘  └────┬────┘
                  │ 配对成功       │ prepareLayout / commitLayout
                  ▼               ▼
           ┌────────┐     ┌───────────────┐
           │ Paired │     │PreparingLayout│
           └────────┘     └───────┬───────┘
                                  │ layoutReady
                                  ▼
                           ┌─────────┐
                           │  Ready  │
                           └────┬────┘
                                │ startStream
                                ▼
                           ┌──────────┐      UDP 停滞
                           │Streaming │ ──────────────▶ ┌───────────┐
                           └────┬─────┘                 │DataStalled│
                                │ stopStream             └─────┬─────┘
                                ▼                       UDP 恢复
                           ┌─────────┐                       │
                           │  Ready  │◄──────────────────────┘
                           └─────────┘

                    任意状态 ── TCP断开 ──▶ Disconnected
```

---

## 7. 设备端实现检查清单

### 第一阶段: 最小可用 (Discovery + Control 握手)

- [ ] 启动后绑定 UDP discovery socket，周期性发送 announce (JSON, schema=v1)
- [ ] 收到 probe 后立即回复 announce
- [ ] 启动 TCP server 监听 control port (默认 20778)
- [ ] 实现 length-prefixed framing: 读 4 字节 u32_be 长度 → 读 payload → 解析 JSON
- [ ] 处理 `hello` → 回复 `helloAck` (含 deviceId、capabilities、data port)
- [ ] 版本不兼容时回复 `error` (code=`unsupported_protocol`)
- [ ] 实现 `ping`/`pong` 心跳 (2s 内回 pong)

### 第二阶段: 配对与安全

- [ ] 处理 `pairRequest` → 在设备端展示确认界面
- [ ] 生成并保存 opaque trust token
- [ ] 回复 `pairResult` (paired=true + trustToken)
- [ ] 处理 `authenticate` → 验证 trust token → 回复 ack 或 error
- [ ] 持久化已配对的桌面端信息

### 第三阶段: Layout 渲染

- [ ] 处理 `prepareLayout` → 缓存 asset manifest
- [ ] 处理 `putAssetChunk` → 按 chunk 重组资源、校验 sha256
- [ ] 校验失败时回复 `asset_hash_mismatch` error
- [ ] 处理 `commitLayout` → 最终校验 → 构建渲染状态
- [ ] 回复 `layoutReady` (含 missingAssets 列表)
- [ ] 渲染 static controls + 为 dynamic controls 预留数据绑定

### 第四阶段: Telemetry 接收与渲染

- [ ] 绑定 UDP data port (默认 20779)
- [ ] 实现 binary header 解析: magic="ACCD" → 版本 → session_id_hash → sequence → payload
- [ ] JSON payload 解析: 提取 values
- [ ] 更新 dynamic controls 对应的值
- [ ] 检测 sequence gap → 跳过，等下一帧 (不请求重传)
- [ ] 支持 keyframe 恢复 (delta 模式下)

### 第五阶段: 状态上报与诊断

- [ ] 每 1 秒通过 TCP 上报 `status` (含 data 丢包统计、render FPS、battery)
- [ ] 状态变化时立即上报
- [ ] 处理 `stopStream` → 停止接收 UDP、重置渲染状态
- [ ] TCP 断开时清理 session、停止 UDP 接收

---

## 8. 错误码

| code | 说明 |
|---|---|
| `unsupported_protocol` | 协议版本不兼容 |
| `pairing_rejected` | 用户拒绝配对 |
| `auth_failed` | trust token 验证失败 |
| `layout_asset_missing` | asset 未收到 |
| `asset_hash_mismatch` | asset sha256 校验失败 |
| `layout_invalid` | layout 定义非法 |
| `profile_unsupported` | stream profile 参数不被支持 |
| `stream_already_active` | 已有活跃 stream |
| `internal_error` | 设备内部错误 |

---

## 9. 兼容性与迁移

- **协议版本**: 当前 `protocolVersion = 1`
- **Schema 标识**: discovery 和 control 消息的 `schema` 字段区分协议版本，设备端应严格校验
- **端口可配置**: 端口可在 announce 中声明非默认值
- **binary_v1 encoding**: 当前可声明支持但不实现，桌面端会回退到 `json`

---

## 附录 A: 参考实现

桌面端协议实现位于本仓库 `src/dashboard/remote/`:

| 文件 | 内容 |
|---|---|
| `protocol.rs` | 所有消息结构体、常量、binary header 定义 |
| `discovery.rs` | UDP probe/announce、设备注册表 |
| `control.rs` | TCP framing、握手、配对、心跳 |
| `data.rs` | UDP sender、binary packet builder |
| `session.rs` | 设备状态机、session 管理 |

---

## 附录 B: 测试用 Fake Device

桌面端集成测试建议实现一个 fake remote dashboard server:

```text
1. 启动 UDP discovery advertiser (周期性发送 announce)
2. 启动 TCP control server (接受连接，完成握手)
3. 实现 pair → authenticate 流程
4. 接收 layout/profile/startStream
5. 接收 UDP telemetry，统计丢包
6. 通过 TCP 上报 status
```
