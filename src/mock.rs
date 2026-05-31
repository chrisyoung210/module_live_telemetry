use crate::types::ControlSample;

pub fn generate_mock_controls(samples: usize, poll_hz: f64) -> Vec<ControlSample> {
    let step_ns = (1_000_000_000.0 / poll_hz.max(1.0)).round() as u64;
    let mut out = Vec::with_capacity(samples);
    for i in 0..samples {
        let t = i as f32 / poll_hz as f32;
        let throttle = ((t * 0.7).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        let brake = if i % 180 > 145 { 0.72 } else { 0.0 };
        let speed = 80.0 + (t * 1.2).sin() * 35.0 + throttle * 110.0 - brake * 55.0;
        let gear = ((speed / 48.0).floor() as i32).clamp(1, 6);
        out.push(ControlSample {
            sample_tick: i as u64,
            timestamp_ns: i as u64 * step_ns,
            physics_packet_id: i as i32,
            graphics_packet_id: i as i32,
            speed_kmh: speed.max(0.0),
            gas: throttle,
            brake,
            clutch: 0.0,
            steer_angle: (t * 1.8).sin() * 0.45,
            gear,
            rpms: (2200.0 + speed * 31.0 + throttle * 1800.0) as i32,
            fuel: 62.0 - i as f32 * 0.002,
        });
    }
    out
}
