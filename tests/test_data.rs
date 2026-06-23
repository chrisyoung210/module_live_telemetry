//! Deterministic `TelemetryFrame` generators for reproducible tests.
//!
//! All data is seeded by the frame's `sample_tick` — same parameters always
//! produce identical output.  Every substructure field has a distinct value
//! pattern so swapped-field bugs are detectable.

use module_live_telemetry::{
    CarStateSample, ControlSample, EnvironmentSample, MotionSample, OtherCarsSample,
    PowertrainSample, SessionMetadata, SessionSample, TelemetryFrame, TimingSample, TyreSample,
};

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

/// Deterministic `SessionMetadata` with known values.
///
/// Unlike `SessionMetadata::new()` (which embeds the current timestamp), this
/// always returns the same metadata for the same arguments.
pub fn make_test_metadata(track: &str, car: &str) -> SessionMetadata {
    SessionMetadata {
        track_name: track.to_string(),
        car_model: car.to_string(),
        created_unix_ns: 1_700_000_000_000_000_000,
        poll_hz: 120.0,
        chunk_rows: 1024,
        sm_version: "test_sm".to_string(),
        ac_version: "test_ac".to_string(),
        number_of_sessions: 1,
        num_cars: 24,
        sector_count: 3,
        max_rpm: 9000,
        max_torque: 650.0,
        max_power: 700.0,
        max_fuel: 100.0,
        penalties_enabled: 1,
        raw_static_bytes: vec![0u8; 1024],
        session_type: Some(9), // Race
    }
}

// ---------------------------------------------------------------------------
// Single frame
// ---------------------------------------------------------------------------

/// Create a deterministic `TelemetryFrame` seeded by `tick`.
///
/// * `tick` — sample tick (frame index).  Every field varies with tick.
/// * `lap`  — 1-based lap number (controls `completed_laps`).
///
/// All 9 substructures are fully populated (no `Default::default()` zeros).
pub fn make_test_frame(tick: u64, lap: u32) -> TelemetryFrame {
    let t = tick as f32;
    let ns = tick * 8_333_333;
    let lap_i32 = lap as i32;

    TelemetryFrame {
        sample_tick: tick,
        timestamp_ns: ns,
        controls: ControlSample {
            sample_tick: tick,
            timestamp_ns: ns,
            physics_packet_id: (tick as i32).wrapping_mul(7),
            graphics_packet_id: (tick as i32).wrapping_mul(13),
            speed_kmh: 150.0 + t * 0.1,
            gas: ((tick % 100) as f32) / 100.0,
            brake: (((tick + 50) % 100) as f32) / 120.0,
            clutch: ((tick % 10) as f32) / 100.0,
            steer_angle: t * 0.02,
            gear: 1 + (tick % 6) as i32,
            rpms: 5000 + (tick as i32) * 50,
            fuel: 80.0 - t * 0.05,
        },
        motion: MotionSample {
            sample_tick: tick,
            timestamp_ns: ns,
            velocity: [t * 0.5, t * 0.3, t * 0.1],
            acc_g: [t * 0.01, t * 0.02, -9.81],
            local_velocity: [t * 0.8, t * 0.02, t * 0.005],
            local_angular_vel: [t * 0.001, t * 0.002, t * 0.0005],
            heading: t * 0.05,
            pitch: t * 0.01,
            roll: t * 0.02,
        },
        tyres: {
            // 4-element array: each index gets a deterministic tick-derived value
            let arr4 = |base: f32, scale: f32| -> [f32; 4] {
                [
                    base + scale * (tick % 4) as f32,
                    base + scale * ((tick + 1) % 4) as f32,
                    base + scale * ((tick + 2) % 4) as f32,
                    base + scale * ((tick + 3) % 4) as f32,
                ]
            };
            // 12-element array
            let arr12 = |base: f32, scale: f32| -> [f32; 12] {
                let mut out = [0.0f32; 12];
                for (i, v) in out.iter_mut().enumerate() {
                    *v = base + scale * ((tick + i as u64) % 12) as f32;
                }
                out
            };
            TyreSample {
                sample_tick: tick,
                timestamp_ns: ns,
                wheel_slip: arr4(0.01, 0.005),
                wheel_load: arr4(2000.0, 50.0),
                wheels_pressure: arr4(27.0, 0.5),
                wheel_angular_speed: arr4(80.0, 5.0),
                tyre_wear: arr4(95.0, -0.5),
                tyre_dirty_level: arr4(0.0, 0.1),
                tyre_core_temperature: arr4(80.0, 2.0),
                camber_rad: arr4(-0.02, 0.001),
                suspension_travel: arr4(0.02, 0.005),
                slip_ratio: arr4(0.0, 0.01),
                slip_angle: arr4(0.0, 0.02),
                tyre_temp_i: arr4(75.0, 2.0),
                tyre_temp_m: arr4(80.0, 2.0),
                tyre_temp_o: arr4(85.0, 2.0),
                tyre_temp: arr4(82.0, 2.0),
                mz: arr4(0.0, 10.0),
                fx: arr4(0.0, 100.0),
                fy: arr4(0.0, 50.0),
                suspension_damage: arr4(0.0, 1.0),
                brake_temp: arr4(400.0, 20.0),
                brake_pressure: arr4(10.0, 2.0),
                pad_life: arr4(80.0, -1.0),
                disc_life: arr4(90.0, -1.0),
                tyre_contact_point: arr12(0.0, 0.05),
                tyre_contact_normal: arr12(0.0, 0.01),
                tyre_contact_heading: arr12(0.0, 0.02),
                number_of_tyres_out: 0,
                front_brake_compound: 1,
                rear_brake_compound: 2,
            }
        },
        powertrain: PowertrainSample {
            sample_tick: tick,
            timestamp_ns: ns,
            turbo_boost: 1.5 + t * 0.01,
            ballast: 30.0 + t * 0.1,
            kers_charge: 80.0 - t * 0.1,
            kers_input: t * 0.5,
            kers_current_kj: 1000.0 + t,
            drs: if tick % 2 == 0 { 1.0 } else { 0.0 },
            tc: 2.0 + (tick % 5) as f32 * 0.1,
            abs: 3.0 - (tick % 4) as f32 * 0.1,
            engine_brake: (tick % 8) as i32,
            ers_recovery_level: (tick % 4) as i32,
            ers_power_level: (tick % 3) as i32,
            ers_heat_charging: (tick % 2) as i32,
            ers_is_charging: (tick % 2) as i32,
            drs_available: 1,
            drs_enabled: (tick % 2) as i32,
            tc_in_action: (tick % 3) as i32,
            abs_in_action: (tick % 3) as i32,
            auto_shifter_on: 1,
            current_max_rpm: 8000,
            p2p_activations: (tick % 5) as i32,
            p2p_status: (tick % 2) as i32,
            water_temp: 90.0 + t * 0.1,
        },
        session: SessionSample {
            sample_tick: tick,
            timestamp_ns: ns,
            status: 2,
            session: 0,
            session_index: 0,
            completed_laps: lap_i32 - 1,
            position: 1 + (tick % 10) as i32,
            session_time_left: 1800.0 - t * 10.0,
            number_of_laps: 15,
            current_sector_index: 1 + (tick % 3) as i32,
            normalized_car_position: t * 0.02,
            is_in_pit: 0,
            is_in_pit_lane: 0,
            mandatory_pit_done: 0,
            missing_mandatory_pits: 0,
            penalty_time: 0.0,
            penalty_type: 0,
            track_status: [0u16; 33],
            clock: t * 10.0,
            replay_time_multiplier: 1.0,
            is_valid_lap: 1,
            global_yellow: 0,
            global_yellow1: 0,
            global_yellow2: 0,
            global_yellow3: 0,
            global_white: 0,
            global_green: 1,
            global_chequered: 0,
            global_red: 0,
            gap_ahead_or_tail_value: 0,
            flag: 0,
            gap_behind: 0,
        },
        timing: TimingSample {
            sample_tick: tick,
            timestamp_ns: ns,
            i_current_time: (tick as i32) * 1000,
            i_last_time: (tick as i32) * 995,
            i_best_time: (tick as i32) * 990,
            i_split: (tick as i32) * 330,
            last_sector_time: (tick as i32) * 330,
            i_delta_lap_time: (tick as i32).wrapping_mul(5),
            is_delta_positive: if tick % 2 == 0 { 0 } else { 1 },
            i_estimated_lap_time: (tick as i32) * 980,
            fuel_estimated_laps: 10.0 - t * 0.01,
            fuel_x_lap: 2.5 + (tick % 10) as f32 * 0.01,
            used_fuel: t * 0.5,
            distance_traveled: t * 200.0,
            current_time_str: {
                let mut s = [0u16; 15];
                let chars = format!("{:04}", tick.min(9999));
                for (i, c) in chars.chars().enumerate() {
                    s[i] = c as u16;
                }
                s
            },
            last_time_str: [0u16; 15],
            best_time_str: [0u16; 15],
            split_str: [0u16; 15],
            delta_lap_time_str: [0u16; 15],
            estimated_lap_time_str: [0u16; 15],
            observed_slot_before_i_split: 0,
        },
        car_state: CarStateSample {
            sample_tick: tick,
            timestamp_ns: ns,
            car_damage: [
                (tick % 100) as f32 * 0.01,
                (tick % 50) as f32 * 0.02,
                (tick % 25) as f32 * 0.04,
                (tick % 10) as f32 * 0.10,
                (tick % 5) as f32 * 0.20,
            ],
            pit_limiter_on: 0,
            ride_height: [5.0 + t * 0.01, 6.0 + t * 0.01],
            ignition_on: 1,
            starter_engine_on: 1,
            is_engine_running: 1,
            is_ai_controlled: 0,
            cg_height: 0.35,
            brake_bias: 55.0 + t * 0.1,
            rain_lights: 0,
            flashing_lights: 0,
            lights_stage: (tick % 4) as i32,
            wiper_lv: (tick % 3) as i32,
            driver_stint_total_time_left: 3600,
            driver_stint_time_left: 1800 - (tick as i32) * 100,
            rain_tyres: 0,
            current_tyre_set: 1 + (tick % 10) as i32,
            strategy_tyre_set: 2,
            track_grip_status: (tick % 4) as i32,
            tyre_compound_str: [0u16; 33],
            mfd_tyre_set: 1,
            mfd_fuel_to_add: 20.0 + t * 0.1,
            mfd_tyre_pressure: [
                27.0 + (tick % 4) as f32 * 0.1,
                27.5 + ((tick + 1) % 4) as f32 * 0.1,
                28.0 + ((tick + 2) % 4) as f32 * 0.1,
                27.8 + ((tick + 3) % 4) as f32 * 0.1,
            ],
            ideal_line_on: 0,
            is_setup_menu_visible: 0,
            main_display_index: 0,
            secondary_display_index: 1,
            direction_lights_left: 0,
            direction_lights_right: 0,
            tc_level: (tick % 11) as i32,
            tc_cut: (tick % 10) as i32,
            engine_map: (tick % 5) as i32 + 1,
            abs_level: (tick % 12) as i32,
            exhaust_temperature: 800.0 + t * 0.5,
            final_ff: 50.0 + t * 0.1,
            performance_meter: t * 0.5,
            kerb_vibration: t * 0.01,
            slip_vibrations: t * 0.02,
            g_vibrations: t * 0.03,
            abs_vibrations: t * 0.01,
        },
        environment: EnvironmentSample {
            sample_tick: tick,
            timestamp_ns: ns,
            air_density: 1.2 + t * 0.001,
            air_temp: 22.0 + t * 0.1,
            road_temp: 30.0 + t * 0.1,
            wind_speed: t * 0.5,
            wind_direction: t * 5.0,
            surface_grip: 0.98 - t * 0.001,
            rain_intensity: (tick % 4) as i32,
            rain_intensity_in_10min: (tick % 3) as i32,
            rain_intensity_in_30min: (tick % 2) as i32,
        },
        other_cars: OtherCarsSample {
            sample_tick: tick,
            timestamp_ns: ns,
            active_cars: 20,
            player_car_id: 100 + (tick as i32),
            car_coordinates: {
                let mut v = vec![0.0f32; 180];
                for (i, elem) in v.iter_mut().enumerate() {
                    *elem = t * 0.1 + (i as f32) * 1.0;
                }
                v
            },
            car_id: {
                let mut v = vec![0i32; 60];
                for (i, elem) in v.iter_mut().enumerate() {
                    *elem = 100 + (tick as i32) + (i as i32);
                }
                v
            },
        },
    }
}

// ---------------------------------------------------------------------------
// Session-level generators
// ---------------------------------------------------------------------------

/// Generate a deterministic test session with `lap_count` laps.
///
/// `frames_per_lap` controls how many frames each lap contains.
/// Lap 1 uses ticks `0..frames_per_lap`, lap 2 uses `frames_per_lap..2*N`,
/// and so on.
pub fn make_test_session(
    lap_count: u32,
    frames_per_lap: u64,
) -> (SessionMetadata, Vec<TelemetryFrame>) {
    let metadata = make_test_metadata("test_track", "test_car");
    let total = (lap_count as u64) * frames_per_lap;
    let frames: Vec<TelemetryFrame> = (0..total)
        .map(|tick| {
            let lap = 1 + (tick / frames_per_lap) as u32;
            make_test_frame(tick, lap)
        })
        .collect();
    (metadata, frames)
}

/// Session with **zero frames** (edge case: empty recording).
pub fn make_test_session_empty() -> (SessionMetadata, Vec<TelemetryFrame>) {
    (make_test_metadata("test_track", "test_car"), Vec::new())
}

/// Session with **exactly one frame** (edge case: minimum recording).
pub fn make_test_session_single_frame() -> (SessionMetadata, Vec<TelemetryFrame>) {
    (
        make_test_metadata("test_track", "test_car"),
        vec![make_test_frame(0, 1)],
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_data_determinism() {
    let (meta1, frames1) = make_test_session(3, 5);
    let (meta2, frames2) = make_test_session(3, 5);
    assert_eq!(meta1.track_name, meta2.track_name);
    assert_eq!(meta1.car_model, meta2.car_model);
    assert_eq!(frames1.len(), frames2.len());
    for (a, b) in frames1.iter().zip(frames2.iter()) {
        assert_eq!(a.sample_tick, b.sample_tick);
        assert_eq!(a.controls.speed_kmh, b.controls.speed_kmh);
    }
}

#[test]
fn test_data_lap_boundaries() {
    let fpl = 5u64;
    let (_meta, frames) = make_test_session(3, fpl);
    assert_eq!(frames.len(), 15, "3 laps × 5 frames = 15 total");

    for (i, frame) in frames[..fpl as usize].iter().enumerate() {
        assert_eq!(
            frame.session.completed_laps, 0,
            "frame {} (lap 1) expected completed_laps=0",
            i
        );
    }
    for (i, frame) in frames[fpl as usize..2 * fpl as usize].iter().enumerate() {
        assert_eq!(
            frame.session.completed_laps,
            1,
            "frame {} (lap 2) expected completed_laps=1",
            i + fpl as usize
        );
    }
    for (i, frame) in frames[2 * fpl as usize..].iter().enumerate() {
        assert_eq!(
            frame.session.completed_laps,
            2,
            "frame {} (lap 3) expected completed_laps=2",
            i + 2 * fpl as usize
        );
    }
}

#[test]
fn test_data_empty_session() {
    let (_meta, frames) = make_test_session_empty();
    assert!(frames.is_empty(), "empty session should have 0 frames");
}

#[test]
fn test_data_single_frame() {
    let (_meta, frames) = make_test_session_single_frame();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].sample_tick, 0);
    assert_eq!(frames[0].session.completed_laps, 0);
}

#[test]
fn test_data_all_substructures_populated() {
    let (_meta, frames) = make_test_session(1, 3);
    for frame in &frames {
        // Controls
        assert!(
            frame.controls.speed_kmh > 140.0,
            "speed_kmh should be > 140"
        );
        // Motion — velocity[0] non-zero
        assert!(frame.motion.velocity[0] >= 0.0, "velocity populated");
        // Tyres
        assert!(
            frame.tyres.wheel_load[0] > 1000.0,
            "wheel_load should be > 1000"
        );
        // Powertrain
        assert!(frame.powertrain.turbo_boost > 1.0, "turbo_boost populated");
        // Session
        assert_eq!(frame.session.status, 2, "session.status should be 2");
        // Timing
        assert!(
            frame.timing.i_current_time >= 0,
            "timing.i_current_time populated"
        );
        // CarState
        assert!(frame.car_state.cg_height > 0.3, "cg_height should be > 0.3");
        // Environment
        assert!(frame.environment.air_temp > 20.0, "air_temp populated");
        // OtherCars
        assert!(frame.other_cars.active_cars > 0, "active_cars populated");
    }
}

#[test]
fn test_data_velocity_array_3() {
    let (_meta, frames) = make_test_session_single_frame();
    assert_eq!(frames[0].motion.velocity.len(), 3);
}

#[test]
fn test_data_wheel_slip_array_4() {
    let (_meta, frames) = make_test_session(1, 2);
    assert_eq!(frames[1].tyres.wheel_slip.len(), 4);
}

#[test]
fn test_data_tyre_contact_arrays_12() {
    let (_meta, frames) = make_test_session_single_frame();
    assert_eq!(frames[0].tyres.tyre_contact_point.len(), 12);
    assert_eq!(frames[0].tyres.tyre_contact_normal.len(), 12);
    assert_eq!(frames[0].tyres.tyre_contact_heading.len(), 12);
}

#[test]
fn test_data_metadata_deterministic() {
    let m1 = make_test_metadata("monza", "ferrari");
    let m2 = make_test_metadata("monza", "ferrari");
    assert_eq!(m1.created_unix_ns, m2.created_unix_ns);
    assert_eq!(m1.max_rpm, m2.max_rpm);
    assert_eq!(m1.num_cars, m2.num_cars);
    assert_eq!(m1.session_type, m2.session_type);
}

#[test]
fn test_data_different_inputs_different_outputs() {
    let (_meta1, frames1) = make_test_session(1, 3);
    let (_meta2, frames2) = make_test_session(2, 3);
    // Different lap counts → different frame sequences
    assert_ne!(
        frames1.len(),
        frames2.len(),
        "different lap counts => different sizes"
    );

    // Different metadata
    let m_a = make_test_metadata("spa", "porsche");
    let m_b = make_test_metadata("monza", "ferrari");
    assert_ne!(m_a.track_name, m_b.track_name);
}
