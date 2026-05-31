use module_live_telemetry::mock::generate_mock_controls;
use module_live_telemetry::{
    BinaryTelemetryReader, BinaryTelemetryWriter, LiveTelemetryConfig, SessionMetadata,
    TelemetryFrame,
};
use std::io::Cursor;

#[test]
fn split_chunks_roundtrip() {
    let metadata = SessionMetadata::new("spa", "porsche_992_gt3_r", 120.0);
    let config = LiveTelemetryConfig {
        poll_hz: 120.0,
        chunk_rows: 7,
    };
    let samples = generate_mock_controls(23, 120.0);

    let cursor = Cursor::new(Vec::new());
    let mut writer = BinaryTelemetryWriter::create(cursor, metadata, config).unwrap();
    for &sample in &samples {
        writer.write_frame(TelemetryFrame {
            sample_tick: sample.sample_tick,
            timestamp_ns: sample.timestamp_ns,
            controls: sample,
            motion: Default::default(),
            tyres: Default::default(),
            powertrain: Default::default(),
            session: Default::default(),
            timing: Default::default(),
            car_state: Default::default(),
            environment: Default::default(),
            other_cars: Default::default(),
        }).unwrap();
    }
    let (cursor, summary) = writer.finish().unwrap();

    assert_eq!(summary.total_samples, 23);
    assert_eq!(summary.chunk_count, 9 * 4); // 9 clusters * 4 full+partial chunks

    let reader = BinaryTelemetryReader::from_bytes(cursor.into_inner()).unwrap();
    assert_eq!(reader.metadata().track_name, "spa");
    assert_eq!(reader.metadata().car_model, "porsche_992_gt3_r");
    let read_back = reader.read_all_controls().unwrap();
    assert_eq!(read_back.len(), samples.len());
    // compare field-by-field since the struct now has more fields
    for (a, b) in samples.iter().zip(read_back.iter()) {
        assert_eq!(a.sample_tick, b.sample_tick);
        assert_eq!(a.timestamp_ns, b.timestamp_ns);
        assert_eq!(a.gas, b.gas);
        assert_eq!(a.brake, b.brake);
        assert_eq!(a.speed_kmh, b.speed_kmh);
    }
}
