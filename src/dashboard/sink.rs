//! DataSink — 数据回传抽象
//!
//! 定义计算结果回传的接口，提供两种实现：
//! - [`ChannelSink`] — 通过 crossbeam 通道发送
//! - [`CallbackSink`] — 通过回调函数处理

use crate::recording::DashboardValuesFrame;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Sink 发送错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// 通道已满
    ChannelFull,
    /// 通道已断开
    Disconnected,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelFull => write!(f, "sink channel full"),
            Self::Disconnected => write!(f, "sink channel disconnected"),
        }
    }
}

impl std::error::Error for SendError {}

/// 数据回传接口
///
/// 计算结果通过此接口发送给上游程序。
/// 实现必须是 `Send` 的，以便跨线程使用。
pub trait DataSink: Send {
    /// 发送计算结果
    ///
    /// `frame.values` 为 item_name → value 的映射，只包含本轮计算有值的项（稀疏结构）。
    fn send(&self, frame: DashboardValuesFrame) -> Result<(), SendError>;

    /// Discard any frame buffered for a consumer.
    ///
    /// Queue-backed legacy sinks cannot implement this safely and therefore
    /// keep the default no-op. Latest-value sinks clear atomically.
    fn clear_pending(&self) {}

    /// Advance the accepted subscription generation and clear older pending data.
    fn advance_subscription_generation(&self, _generation: u64) {
        self.clear_pending();
    }
}

/// Point-in-time counters for a latest-value dashboard channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LatestValueStats {
    pub published_frames: u64,
    pub overwritten_frames: u64,
    pub received_frames: u64,
    pub cleared_frames: u64,
    pub stale_generation_frames: u64,
}

#[derive(Default)]
struct LatestValueState {
    slot: Option<DashboardValuesFrame>,
    generation_floor: u64,
}

struct LatestValueShared {
    state: Mutex<LatestValueState>,
    ready: Condvar,
    senders: AtomicUsize,
    receivers: AtomicUsize,
    published: AtomicU64,
    overwritten: AtomicU64,
    received: AtomicU64,
    cleared: AtomicU64,
    stale_generation: AtomicU64,
    notify_tx: Sender<()>,
    notify_rx: Receiver<()>,
}

/// Sending side of a capacity-one, overwrite-on-full dashboard channel.
pub struct LatestValueSender {
    shared: Arc<LatestValueShared>,
}

impl Clone for LatestValueSender {
    fn clone(&self) -> Self {
        self.shared.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for LatestValueSender {
    fn drop(&mut self) {
        if self.shared.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            let _ = self.shared.notify_tx.try_send(());
            self.shared.ready.notify_all();
        }
    }
}

impl LatestValueSender {
    pub fn send_latest(&self, frame: DashboardValuesFrame) -> Result<(), SendError> {
        if self.shared.receivers.load(Ordering::Acquire) == 0 {
            return Err(SendError::Disconnected);
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| SendError::Disconnected)?;
        self.shared.published.fetch_add(1, Ordering::Relaxed);
        if frame.subscription_generation < state.generation_floor {
            self.shared.stale_generation.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        if frame.subscription_generation > state.generation_floor {
            state.generation_floor = frame.subscription_generation;
            if state.slot.take().is_some() {
                self.shared.overwritten.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Some(pending) = state.slot.as_mut() {
            if pending.subscription_generation != frame.subscription_generation {
                *pending = frame;
            } else {
                pending.sample_tick = frame.sample_tick;
                pending.timestamp_ns = frame.timestamp_ns;
                pending.values.extend(frame.values);
            }
            self.shared.overwritten.fetch_add(1, Ordering::Relaxed);
        } else {
            state.slot = Some(frame);
        }
        let _ = self.shared.notify_tx.try_send(());
        drop(state);
        self.shared.ready.notify_one();
        Ok(())
    }

    pub fn stats(&self) -> LatestValueStats {
        latest_value_stats(&self.shared)
    }

    /// Atomically discard the pending patch and its readiness notification.
    pub fn clear_pending(&self) -> bool {
        clear_latest_pending(&self.shared)
    }

    /// Advance the generation floor and atomically discard older pending data.
    pub fn advance_subscription_generation(&self, generation: u64) -> bool {
        advance_latest_generation(&self.shared, generation)
    }
}

/// Receiving side of a latest-value dashboard channel.
pub struct LatestValueReceiver {
    shared: Arc<LatestValueShared>,
}

impl Clone for LatestValueReceiver {
    fn clone(&self) -> Self {
        self.shared.receivers.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for LatestValueReceiver {
    fn drop(&mut self) {
        self.shared.receivers.fetch_sub(1, Ordering::AcqRel);
    }
}

impl LatestValueReceiver {
    pub fn try_recv(&self) -> Result<DashboardValuesFrame, TryRecvError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| TryRecvError::Disconnected)?;
        if let Some(frame) = state.slot.take() {
            let _ = self.shared.notify_rx.try_recv();
            self.shared.received.fetch_add(1, Ordering::Relaxed);
            return Ok(frame);
        }
        if self.shared.senders.load(Ordering::Acquire) == 0 {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<DashboardValuesFrame, RecvTimeoutError> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| RecvTimeoutError::Disconnected)?;
        let (mut state, _wait) = self
            .shared
            .ready
            .wait_timeout_while(state, timeout, |state| {
                state.slot.is_none() && self.shared.senders.load(Ordering::Acquire) > 0
            })
            .map_err(|_| RecvTimeoutError::Disconnected)?;
        if let Some(frame) = state.slot.take() {
            let _ = self.shared.notify_rx.try_recv();
            self.shared.received.fetch_add(1, Ordering::Relaxed);
            Ok(frame)
        } else if self.shared.senders.load(Ordering::Acquire) == 0 {
            Err(RecvTimeoutError::Disconnected)
        } else {
            Err(RecvTimeoutError::Timeout)
        }
    }

    pub fn stats(&self) -> LatestValueStats {
        latest_value_stats(&self.shared)
    }

    /// Return a crossbeam notification receiver suitable for `select!`.
    ///
    /// A notification means a pending frame may be available or all senders
    /// disconnected. After receiving it, call [`Self::try_recv`]. Notifications
    /// are coalesced to capacity one.
    pub fn notification_receiver(&self) -> Receiver<()> {
        self.shared.notify_rx.clone()
    }

    /// Atomically discard the pending patch and its readiness notification.
    pub fn clear_pending(&self) -> bool {
        clear_latest_pending(&self.shared)
    }
}

fn clear_latest_pending(shared: &LatestValueShared) -> bool {
    let Ok(mut state) = shared.state.lock() else {
        return false;
    };
    let cleared = state.slot.take().is_some();
    let _ = shared.notify_rx.try_recv();
    if cleared {
        shared.cleared.fetch_add(1, Ordering::Relaxed);
    }
    cleared
}

fn advance_latest_generation(shared: &LatestValueShared, generation: u64) -> bool {
    let Ok(mut state) = shared.state.lock() else {
        return false;
    };
    if generation > state.generation_floor {
        state.generation_floor = generation;
    }
    let cleared = state.slot.take().is_some();
    let _ = shared.notify_rx.try_recv();
    if cleared {
        shared.cleared.fetch_add(1, Ordering::Relaxed);
    }
    cleared
}

fn latest_value_stats(shared: &LatestValueShared) -> LatestValueStats {
    LatestValueStats {
        published_frames: shared.published.load(Ordering::Relaxed),
        overwritten_frames: shared.overwritten.load(Ordering::Relaxed),
        received_frames: shared.received.load(Ordering::Relaxed),
        cleared_frames: shared.cleared.load(Ordering::Relaxed),
        stale_generation_frames: shared.stale_generation.load(Ordering::Relaxed),
    }
}

/// Create a dashboard channel that always retains the newest frame.
pub fn latest_value_channel() -> (LatestValueSender, LatestValueReceiver) {
    let (notify_tx, notify_rx) = bounded(1);
    let shared = Arc::new(LatestValueShared {
        state: Mutex::new(LatestValueState::default()),
        ready: Condvar::new(),
        senders: AtomicUsize::new(1),
        receivers: AtomicUsize::new(1),
        published: AtomicU64::new(0),
        overwritten: AtomicU64::new(0),
        received: AtomicU64::new(0),
        cleared: AtomicU64::new(0),
        stale_generation: AtomicU64::new(0),
        notify_tx,
        notify_rx,
    });
    (
        LatestValueSender {
            shared: Arc::clone(&shared),
        },
        LatestValueReceiver { shared },
    )
}

/// Data sink backed by [`latest_value_channel`].
pub struct LatestValueSink {
    sender: LatestValueSender,
}

impl LatestValueSink {
    pub fn new(sender: LatestValueSender) -> Self {
        Self { sender }
    }

    pub fn stats(&self) -> LatestValueStats {
        self.sender.stats()
    }
}

impl DataSink for LatestValueSink {
    fn send(&self, frame: DashboardValuesFrame) -> Result<(), SendError> {
        self.sender.send_latest(frame)
    }

    fn clear_pending(&self) {
        self.sender.clear_pending();
    }

    fn advance_subscription_generation(&self, generation: u64) {
        self.sender.advance_subscription_generation(generation);
    }
}

// ---------------------------------------------------------------------------
// ChannelSink
// ---------------------------------------------------------------------------

/// 通过 crossbeam 通道发送计算结果
///
/// Legacy bounded-channel adapter. Prefer [`LatestValueSink`] for live HUDs;
/// this adapter cannot evict an old frame because it does not own a receiver.
pub struct ChannelSink {
    sender: Sender<DashboardValuesFrame>,
}

impl ChannelSink {
    /// 创建新的 ChannelSink
    pub fn new(sender: Sender<DashboardValuesFrame>) -> Self {
        Self { sender }
    }
}

impl DataSink for ChannelSink {
    fn send(&self, frame: DashboardValuesFrame) -> Result<(), SendError> {
        // 非阻塞发送：如果通道满，丢弃本次数据
        self.sender.try_send(frame).map_err(|e| match e {
            crossbeam_channel::TrySendError::Full(_) => SendError::ChannelFull,
            crossbeam_channel::TrySendError::Disconnected(_) => SendError::Disconnected,
        })
    }
}

// ---------------------------------------------------------------------------
// CallbackSink
// ---------------------------------------------------------------------------

/// 通过回调函数处理计算结果
///
/// 每次有数据时调用传入的回调函数。回调函数应快速返回，避免阻塞数据流。
pub struct CallbackSink {
    callback: Box<dyn Fn(DashboardValuesFrame) + Send + Sync>,
}

impl CallbackSink {
    /// 创建新的 CallbackSink
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(DashboardValuesFrame) + Send + Sync + 'static,
    {
        Self {
            callback: Box::new(callback),
        }
    }
}

impl DataSink for CallbackSink {
    fn send(&self, frame: DashboardValuesFrame) -> Result<(), SendError> {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.callback)(frame);
        }));
        match result {
            Ok(()) => Ok(()),
            Err(panic_info) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                eprintln!("CallbackSink: callback panicked: {msg}");
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// NullSink (for testing)
// ---------------------------------------------------------------------------

/// 丢弃所有数据的 sink（用于测试）
pub struct NullSink;

impl DataSink for NullSink {
    fn send(&self, _frame: DashboardValuesFrame) -> Result<(), SendError> {
        // 丢弃
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn values_frame(values: HashMap<String, f64>) -> DashboardValuesFrame {
        DashboardValuesFrame {
            subscription_generation: 0,
            sample_tick: 7,
            timestamp_ns: 99,
            values,
        }
    }

    #[test]
    fn test_channel_sink_sends_data() {
        let (tx, rx) = bounded::<DashboardValuesFrame>(10);
        let sink = ChannelSink::new(tx);

        let mut data = HashMap::new();
        data.insert("speed_mps".to_string(), 27.78);
        sink.send(values_frame(data)).unwrap();

        let received = rx.try_recv().unwrap();
        assert_eq!(received.sample_tick, 7);
        assert_eq!(received.timestamp_ns, 99);
        assert_eq!(received.values.get("speed_mps"), Some(&27.78));
    }

    #[test]
    fn test_channel_sink_overflow() {
        let (tx, rx) = bounded::<DashboardValuesFrame>(1);
        let sink = ChannelSink::new(tx);

        let mut data1 = HashMap::new();
        data1.insert("a".to_string(), 1.0);
        let mut data2 = HashMap::new();
        data2.insert("b".to_string(), 2.0);

        // Fill channel (capacity 1)
        sink.send(values_frame(data1)).unwrap();
        // This will fail because channel is full
        assert_eq!(sink.send(values_frame(data2)), Err(SendError::ChannelFull));

        // Only first should be received
        let received = rx.try_recv().unwrap();
        assert_eq!(received.values.get("a"), Some(&1.0));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn latest_value_channel_overwrites_old_frame() {
        let (tx, rx) = latest_value_channel();
        let sink = LatestValueSink::new(tx);

        let mut old = HashMap::new();
        old.insert("speed".to_string(), 100.0);
        old.insert("gear".to_string(), 4.0);
        let mut latest = HashMap::new();
        latest.insert("speed".to_string(), 123.0);
        sink.send(DashboardValuesFrame {
            subscription_generation: 0,
            sample_tick: 7,
            timestamp_ns: 99,
            values: old,
        })
        .unwrap();
        sink.send(DashboardValuesFrame {
            subscription_generation: 0,
            sample_tick: 8,
            timestamp_ns: 100,
            values: latest,
        })
        .unwrap();

        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.sample_tick, 8);
        assert_eq!(frame.timestamp_ns, 100);
        assert_eq!(frame.values.get("speed"), Some(&123.0));
        assert_eq!(frame.values.get("gear"), Some(&4.0));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(
            rx.stats(),
            LatestValueStats {
                published_frames: 2,
                overwritten_frames: 1,
                received_frames: 1,
                cleared_frames: 0,
                stale_generation_frames: 0,
            }
        );
    }

    #[test]
    fn latest_value_notification_works_with_crossbeam_select() {
        let (tx, rx) = latest_value_channel();
        let notification = rx.notification_receiver();
        tx.send_latest(values_frame(HashMap::from([("rpm".to_string(), 7000.0)])))
            .unwrap();

        crossbeam_channel::select! {
            recv(notification) -> ready => ready.unwrap(),
            default(Duration::from_millis(100)) => panic!("latest-value notification timed out"),
        }
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.values.get("rpm"), Some(&7000.0));
    }

    #[test]
    fn latest_value_notification_wakes_on_disconnect() {
        let (tx, rx) = latest_value_channel();
        let notification = rx.notification_receiver();
        drop(tx);

        crossbeam_channel::select! {
            recv(notification) -> ready => ready.unwrap(),
            default(Duration::from_millis(100)) => panic!("disconnect notification timed out"),
        }
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn clear_pending_removes_frame_and_notification() {
        let (tx, rx) = latest_value_channel();
        let notification = rx.notification_receiver();
        tx.send_latest(values_frame(HashMap::from([("gear".to_string(), 3.0)])))
            .unwrap();

        assert!(rx.clear_pending());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(matches!(notification.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(rx.stats().cleared_frames, 1);
    }

    #[test]
    fn latest_value_never_merges_across_subscription_generations() {
        let (tx, rx) = latest_value_channel();
        tx.send_latest(DashboardValuesFrame {
            subscription_generation: 1,
            sample_tick: 10,
            timestamp_ns: 100,
            values: HashMap::from([("old".to_string(), 1.0)]),
        })
        .unwrap();
        tx.send_latest(DashboardValuesFrame {
            subscription_generation: 2,
            sample_tick: 11,
            timestamp_ns: 110,
            values: HashMap::from([("new".to_string(), 2.0)]),
        })
        .unwrap();

        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.subscription_generation, 2);
        assert_eq!(frame.values, HashMap::from([("new".to_string(), 2.0)]));

        tx.send_latest(DashboardValuesFrame {
            subscription_generation: 1,
            sample_tick: 12,
            timestamp_ns: 120,
            values: HashMap::from([("late-old".to_string(), 3.0)]),
        })
        .unwrap();
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(rx.stats().stale_generation_frames, 1);
    }

    #[test]
    fn test_callback_sink_invokes_closure() {
        let received = Arc::new(std::sync::Mutex::new(values_frame(HashMap::new())));
        let received_clone = Arc::clone(&received);

        let sink = CallbackSink::new(move |data| {
            *received_clone.lock().unwrap() = data;
        });

        let mut data = HashMap::new();
        data.insert("test".to_string(), 42.0);
        sink.send(values_frame(data)).unwrap();

        let guard = received.lock().unwrap();
        assert_eq!(guard.values.get("test"), Some(&42.0));
    }

    #[test]
    fn test_null_sink_does_not_panic() {
        let mut data = HashMap::new();
        data.insert("any".to_string(), 1.0);
        NullSink.send(values_frame(data)).unwrap();
        // Just verifying no panic
    }
}
