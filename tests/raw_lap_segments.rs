use module_live_telemetry::{AccSessionKind, BinaryTelemetryReader};
use std::path::Path;

#[test]
fn fixture_recording_segments_into_laps() {
    let path = Path::new("data/live_1780226240_monza_mclaren_720s_gt3_evo.acctlm");
    if !path.exists() {
        eprintln!(
            "skipping fixture test because {} is missing",
            path.display()
        );
        return;
    }

    let reader = BinaryTelemetryReader::open(path).unwrap();
    let session = reader.segment_raw_session().unwrap();

    assert_eq!(session.metadata.track_name, "monza");
    assert_eq!(session.metadata.car_model, "mclaren_720s_gt3_evo");
    assert_eq!(session.session_kind, Some(AccSessionKind::Hotlap));
    assert_eq!(session.sample_count, 49_076);
    assert_eq!(session.laps.len(), 5);

    let ranges: Vec<(usize, usize)> = session
        .laps
        .iter()
        .map(|lap| (lap.start_sample_index, lap.end_sample_index))
        .collect();
    assert_eq!(
        ranges,
        vec![
            (0, 5703),
            (5703, 19939),
            (19939, 33828),
            (33828, 48506),
            (48506, 49076)
        ]
    );

    assert_eq!(session.laps[1].lap_time_ms, Some(121_250));
    assert_eq!(session.laps[2].lap_time_ms, Some(118_317));
    assert_eq!(session.laps[3].lap_time_ms, Some(125_032));
    assert_eq!(session.laps[4].is_complete, false);
}
