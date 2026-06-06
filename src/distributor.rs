//! 遥测数据分发器
//!
//! 使用 `Arc<TelemetryFrame>` 将遥测帧零拷贝分发给多个消费者，
//! 支持录制功能和 Dashboard 服务同时运行。

use crate::TelemetryFrame;
use crossbeam_channel::{self, Receiver, Sender};
use std::sync::Arc;

/// 遥测数据分发器
///
/// 从单一数据源接收 `Arc<TelemetryFrame>`，零拷贝分发给所有已注册的消费者。
/// 每个消费者通过独立的通道接收数据，互不干扰。
///
/// # 使用示例
///
/// ```no_run
/// use module_live_telemetry::compute::ComputeRegistry;
/// use module_live_telemetry::distributor::TelemetryDistributor;
/// use std::sync::Arc;
///
/// let mut distributor = TelemetryDistributor::new(64);
/// let recorder_rx = distributor.add_consumer();
/// let dashboard_rx = distributor.add_consumer();
///
/// // 在录制循环中：
/// // let frame = reader.read_telemetry_frame(...)?;
/// // let frame_arc = Arc::new(frame);
/// // distributor.distribute(Arc::clone(&frame_arc));
/// ```
pub struct TelemetryDistributor {
    senders: Vec<Sender<Arc<TelemetryFrame>>>,
    capacity: usize,
}

impl TelemetryDistributor {
    /// 创建新的分发器
    ///
    /// `capacity` 指定每个消费者通道的缓冲大小。
    /// 推荐 dashboard 消费者使用 capacity=1，以最小化旧帧排队。
    /// 使用 non-blocking `try_send`——通道满时新帧被丢弃，旧帧保留在队列中。
    pub fn new(capacity: usize) -> Self {
        Self {
            senders: Vec::new(),
            capacity,
        }
    }

    /// 添加新的消费者，返回其接收端
    pub fn add_consumer(&mut self) -> Receiver<Arc<TelemetryFrame>> {
        let (tx, rx) = crossbeam_channel::bounded(self.capacity);
        self.senders.push(tx);
        rx
    }

    /// 分发遥测帧给所有消费者
    ///
    /// 接收一个 `Arc<TelemetryFrame>`，克隆 Arc（零拷贝引用计数）发送给每个消费者。
    /// 调用方应在传入前创建 Arc，以便在录制和 dashboard 路径之间共享同一份数据。
    /// 使用 `try_send` 避免阻塞——如果消费者通道已满，该帧对该消费者被丢弃。
    pub fn distribute(&self, frame: Arc<TelemetryFrame>) {
        for sender in &self.senders {
            // try_send: 非阻塞发送，消费者满时丢弃新帧
            let _ = sender.try_send(Arc::clone(&frame));
        }
    }

    /// 返回当前消费者数量
    pub fn consumer_count(&self) -> usize {
        self.senders.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CarStateSample, ControlSample, EnvironmentSample, MotionSample,
        OtherCarsSample, PowertrainSample, SessionSample, TimingSample, TyreSample,
    };

    fn make_frame(tick: u64) -> TelemetryFrame {
        TelemetryFrame {
            sample_tick: tick,
            timestamp_ns: tick * 8_333_333,
            controls: ControlSample::default(),
            motion: MotionSample::default(),
            tyres: TyreSample::default(),
            powertrain: PowertrainSample::default(),
            session: SessionSample::default(),
            timing: TimingSample::default(),
            car_state: CarStateSample::default(),
            environment: EnvironmentSample::default(),
            other_cars: OtherCarsSample::default(),
        }
    }

    #[test]
    fn test_single_consumer() {
        let mut distributor = TelemetryDistributor::new(10);
        let rx = distributor.add_consumer();

        distributor.distribute(Arc::new(make_frame(1)));
        distributor.distribute(Arc::new(make_frame(2)));

        let f1 = rx.try_recv().unwrap();
        let f2 = rx.try_recv().unwrap();

        assert_eq!(f1.sample_tick, 1);
        assert_eq!(f2.sample_tick, 2);
    }

    #[test]
    fn test_multiple_consumers() {
        let mut distributor = TelemetryDistributor::new(10);
        let rx1 = distributor.add_consumer();
        let rx2 = distributor.add_consumer();

        distributor.distribute(Arc::new(make_frame(42)));

        let f1 = rx1.try_recv().unwrap();
        let f2 = rx2.try_recv().unwrap();

        assert_eq!(f1.sample_tick, 42);
        assert_eq!(f2.sample_tick, 42);
    }

    #[test]
    fn test_arc_shared_data() {
        let mut distributor = TelemetryDistributor::new(10);
        let rx1 = distributor.add_consumer();
        let rx2 = distributor.add_consumer();

        distributor.distribute(Arc::new(make_frame(100)));

        let f1 = rx1.try_recv().unwrap();

        // f1 and the frame still in rx2 should share the same allocation
        assert_eq!(f1.sample_tick, 100);
        // f1 is still alive when we recv rx2 — confirms Arc sharing
        let _f2 = rx2.try_recv().unwrap();
        drop(f1);
    }

    #[test]
    fn test_overflow_drops_new() {
        let mut distributor = TelemetryDistributor::new(2);
        let rx = distributor.add_consumer();

        // Don't recv — fill channel to capacity
        distributor.distribute(Arc::new(make_frame(1)));
        distributor.distribute(Arc::new(make_frame(2)));
        // This new frame should be dropped (try_send on full channel)
        distributor.distribute(Arc::new(make_frame(3)));

        // Should receive frames 1 and 2 (frame 3 was dropped)
        let f1 = rx.try_recv().unwrap();
        let f2 = rx.try_recv().unwrap();

        assert_eq!(f1.sample_tick, 1);
        assert_eq!(f2.sample_tick, 2);
        // Frame 3 was never sent (try_send returned Err)
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_consumer_count() {
        let mut distributor = TelemetryDistributor::new(10);
        assert_eq!(distributor.consumer_count(), 0);

        distributor.add_consumer();
        assert_eq!(distributor.consumer_count(), 1);

        distributor.add_consumer();
        assert_eq!(distributor.consumer_count(), 2);
    }
}
