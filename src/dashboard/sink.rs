//! DataSink — 数据回传抽象
//!
//! 定义计算结果回传的接口，提供两种实现：
//! - [`ChannelSink`] — 通过 crossbeam 通道发送
//! - [`CallbackSink`] — 通过回调函数处理

use crossbeam_channel::Sender;
use std::collections::HashMap;

/// 数据回传接口
///
/// 计算结果通过此接口发送给上游程序。
/// 实现必须是 `Send` 的，以便跨线程使用。
pub trait DataSink: Send {
    /// 发送计算结果
    ///
    /// `data` 为 item_name → value 的映射，只包含本轮计算有值的项（稀疏结构）。
    fn send(&self, data: HashMap<String, f64>);
}

// ---------------------------------------------------------------------------
// ChannelSink
// ---------------------------------------------------------------------------

/// 通过 crossbeam 通道发送计算结果
///
/// 使用非阻塞 `try_send`，如果通道已满则丢弃数据（避免阻塞实时数据生产）。
pub struct ChannelSink {
    sender: Sender<HashMap<String, f64>>,
}

impl ChannelSink {
    /// 创建新的 ChannelSink
    pub fn new(sender: Sender<HashMap<String, f64>>) -> Self {
        Self { sender }
    }
}

impl DataSink for ChannelSink {
    fn send(&self, data: HashMap<String, f64>) {
        // 非阻塞发送：如果通道满，丢弃本次数据
        let _ = self.sender.try_send(data);
    }
}

// ---------------------------------------------------------------------------
// CallbackSink
// ---------------------------------------------------------------------------

/// 通过回调函数处理计算结果
///
/// 每次有数据时调用传入的回调函数。回调函数应快速返回，避免阻塞数据流。
pub struct CallbackSink {
    callback: Box<dyn Fn(HashMap<String, f64>) + Send + Sync>,
}

impl CallbackSink {
    /// 创建新的 CallbackSink
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(HashMap<String, f64>) + Send + Sync + 'static,
    {
        Self {
            callback: Box::new(callback),
        }
    }
}

impl DataSink for CallbackSink {
    fn send(&self, data: HashMap<String, f64>) {
        (self.callback)(data);
    }
}

// ---------------------------------------------------------------------------
// NullSink (for testing)
// ---------------------------------------------------------------------------

/// 丢弃所有数据的 sink（用于测试）
pub struct NullSink;

impl DataSink for NullSink {
    fn send(&self, _data: HashMap<String, f64>) {
        // 丢弃
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;
    use std::sync::Arc;

    #[test]
    fn test_channel_sink_sends_data() {
        let (tx, rx) = bounded::<HashMap<String, f64>>(10);
        let sink = ChannelSink::new(tx);

        let mut data = HashMap::new();
        data.insert("speed_mps".to_string(), 27.78);
        sink.send(data);

        let received = rx.try_recv().unwrap();
        assert_eq!(received.get("speed_mps"), Some(&27.78));
    }

    #[test]
    fn test_channel_sink_overflow() {
        let (tx, rx) = bounded::<HashMap<String, f64>>(1);
        let sink = ChannelSink::new(tx);

        let mut data1 = HashMap::new();
        data1.insert("a".to_string(), 1.0);
        let mut data2 = HashMap::new();
        data2.insert("b".to_string(), 2.0);

        // Fill channel (capacity 1)
        sink.send(data1);
        // This will fail silently (try_send returns Err)
        sink.send(data2);

        // Only first should be received
        let received = rx.try_recv().unwrap();
        assert_eq!(received.get("a"), Some(&1.0));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_callback_sink_invokes_closure() {
        let received = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let received_clone = Arc::clone(&received);

        let sink = CallbackSink::new(move |data| {
            *received_clone.lock().unwrap() = data;
        });

        let mut data = HashMap::new();
        data.insert("test".to_string(), 42.0);
        sink.send(data);

        let guard = received.lock().unwrap();
        assert_eq!(guard.get("test"), Some(&42.0));
    }

    #[test]
    fn test_null_sink_does_not_panic() {
        let mut data = HashMap::new();
        data.insert("any".to_string(), 1.0);
        NullSink.send(data);
        // Just verifying no panic
    }
}
