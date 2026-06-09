use module_live_telemetry::{
    BinaryTelemetryReader, LiveTelemetryConfig, SessionMetadata,
    TelemetryFrame,
};
use module_live_telemetry::writer::BinaryTelemetryWriter;

/// Create a TelemetryFrame with distinctive non-zero values for all 9 clusters.
/// `seed` shifts field values so each frame is unique.
fn make_frame(seed: u64) -> TelemetryFrame {
    let s = seed as f32;
    TelemetryFrame {
        sample_tick: 100 + seed,
        timestamp_ns: 1_000_000_000 + seed * 8_333_333,
        controls: module_live_telemetry::ControlSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            physics_packet_id: (seed as i32).wrapping_mul(7),
            graphics_packet_id: (seed as i32).wrapping_mul(13),
            speed_kmh: 150.0 + s,
            gas: 0.5 + s * 0.01,
            brake: 0.1 + s * 0.001,
            clutch: 0.0,
            steer_angle: s * 0.02,
            gear: (1 + seed % 6) as i32,
            rpms: 6000 + seed as i32 * 100,
            fuel: 50.0 - s * 0.1,
        },
        motion: module_live_telemetry::MotionSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            velocity: [s, s + 1.0, s + 2.0],
            acc_g: [s * 0.1, s * 0.2, -9.81],
            local_velocity: [s * 0.5, 0.0, 0.0],
            local_angular_vel: [0.0, s * 0.01, 0.0],
            heading: s * 0.05,
            pitch: s * 0.01,
            roll: s * 0.02,
        },
        tyres: module_live_telemetry::TyreSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            wheel_slip: [s * 0.01; 4],
            wheel_load: [2000.0 + s; 4],
            wheels_pressure: [27.0 + s * 0.1; 4],
            wheel_angular_speed: [80.0 + s; 4],
            tyre_wear: [95.0 - s * 0.01; 4],
            tyre_dirty_level: [s * 0.1; 4],
            tyre_core_temperature: [80.0 + s * 0.1; 4],
            camber_rad: [s * 0.001; 4],
            suspension_travel: [0.02 + s * 0.001; 4],
            slip_ratio: [s * 0.005; 4],
            slip_angle: [s * 0.01; 4],
            tyre_temp_i: [75.0 + s * 0.1; 4],
            tyre_temp_m: [80.0 + s * 0.1; 4],
            tyre_temp_o: [85.0 + s * 0.1; 4],
            tyre_temp: [82.0 + s * 0.1; 4],
            mz: [s * 10.0; 4],
            fx: [s * 100.0; 4],
            fy: [s * 50.0; 4],
            suspension_damage: [0.0; 4],
            brake_temp: [400.0 + s; 4],
            brake_pressure: [10.0 + s * 0.1; 4],
            pad_life: [80.0 - s * 0.01; 4],
            disc_life: [90.0 - s * 0.01; 4],
            tyre_contact_point: [s * 0.1; 12],
            tyre_contact_normal: [s * 0.01; 12],
            tyre_contact_heading: [s * 0.02; 12],
            number_of_tyres_out: 0,
            front_brake_compound: 1,
            rear_brake_compound: 2,
        },
        powertrain: module_live_telemetry::PowertrainSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            turbo_boost: 1.5 + s * 0.01,
            ballast: 30.0 + s,
            kers_charge: 80.0 - s * 0.1,
            kers_input: s * 0.5,
            kers_current_kj: 1000.0 + s,
            drs: 0.0,
            tc: 2.0,
            abs: 3.0,
            engine_brake: 4,
            ers_recovery_level: 3,
            ers_power_level: 2,
            ers_heat_charging: 1,
            ers_is_charging: 0,
            drs_available: 1,
            drs_enabled: 0,
            tc_in_action: 0,
            abs_in_action: 0,
            auto_shifter_on: 1,
            current_max_rpm: 8000,
            p2p_activations: (seed % 5) as i32,
            p2p_status: 1,
            water_temp: 90.0 + s * 0.1,
        },
        session: module_live_telemetry::SessionSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            status: 2,
            session: 0,
            session_index: 0,
            completed_laps: (seed / 5) as i32,
            position: 3,
            session_time_left: 1800.0 - s * 10.0,
            number_of_laps: 15,
            current_sector_index: (seed % 3 + 1) as i32,
            normalized_car_position: s * 0.02,
            is_in_pit: 0,
            is_in_pit_lane: 0,
            mandatory_pit_done: 0,
            missing_mandatory_pits: 0,
            gap_ahead_or_tail_value: 0,
            gap_behind: 0,
            penalty_time: 0.0,
            penalty_type: 0,
            flag: 0,
            track_status: [0u16; 33],
            clock: s * 10.0,
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
        },
        timing: module_live_telemetry::TimingSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            i_current_time: seed as i32 * 1000,
            i_last_time: seed as i32 * 995,
            i_best_time: seed as i32 * 990,
            i_split: seed as i32 * 330,
            last_sector_time: seed as i32 * 330,
            i_delta_lap_time: seed as i32 * 5,
            is_delta_positive: 0,
            i_estimated_lap_time: seed as i32 * 980,
            fuel_estimated_laps: 10.0 - s * 0.01,
            fuel_x_lap: 2.5,
            used_fuel: s * 0.5,
            distance_traveled: s * 200.0,
            current_time_str: [0u16; 15],
            last_time_str: [0u16; 15],
            best_time_str: [0u16; 15],
            split_str: [0u16; 15],
            delta_lap_time_str: [0u16; 15],
            estimated_lap_time_str: [0u16; 15],
            observed_slot_before_i_split: 0,
        },
        car_state: module_live_telemetry::CarStateSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            car_damage: [0.0; 5],
            pit_limiter_on: 0,
            ride_height: [5.0 + s * 0.01; 2],
            ignition_on: 1,
            starter_engine_on: 1,
            is_engine_running: 1,
            is_ai_controlled: 0,
            cg_height: 0.35,
            brake_bias: 55.0 + s * 0.1,
            rain_lights: 0,
            flashing_lights: 0,
            lights_stage: 0,
            wiper_lv: 0,
            driver_stint_total_time_left: 3600,
            driver_stint_time_left: 1800 - seed as i32 * 100,
            rain_tyres: 0,
            current_tyre_set: 1,
            strategy_tyre_set: 2,
            track_grip_status: 2,
            tyre_compound_str: [0u16; 33],
            mfd_tyre_set: 1,
            mfd_fuel_to_add: 20.0 + s,
            mfd_tyre_pressure: [27.0 + s * 0.1; 4],
            ideal_line_on: 0,
            is_setup_menu_visible: 0,
            main_display_index: 0,
            secondary_display_index: 1,
            direction_lights_left: 0,
            direction_lights_right: 0,
            tc_level: 5,
            tc_cut: 3,
            engine_map: 2,
            abs_level: 5,
            exhaust_temperature: 800.0 + s,
            final_ff: 50.0 + s * 0.1,
            performance_meter: s * 0.5,
            kerb_vibration: s * 0.01,
            slip_vibrations: s * 0.02,
            g_vibrations: s * 0.03,
            abs_vibrations: s * 0.01,
        },
        environment: module_live_telemetry::EnvironmentSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            air_density: 1.2 + s * 0.001,
            air_temp: 22.0 + s * 0.1,
            road_temp: 30.0 + s * 0.1,
            wind_speed: s * 0.5,
            wind_direction: s * 5.0,
            surface_grip: 0.98 - s * 0.001,
            rain_intensity: 0,
            rain_intensity_in_10min: 0,
            rain_intensity_in_30min: 0,
        },
        other_cars: module_live_telemetry::OtherCarsSample {
            sample_tick: 100 + seed,
            timestamp_ns: 1_000_000_000 + seed * 8_333_333,
            active_cars: 20,
            player_car_id: 100 + seed as i32,
            car_coordinates: {
                let mut v = vec![0.0f32; 180];
                v[0] = s; v[1] = s + 1.0; v[2] = s + 2.0;
                v
            },
            car_id: {
                let mut v = vec![0i32; 60];
                v[0] = 100 + seed as i32;
                v
            },
        },
    }
}

/// Write `n` frames with chunk_rows, finish, then read back and verify all 9 clusters.
fn roundtrip_all_clusters(n: usize, chunk_rows: usize) {
    let metadata = SessionMetadata::new("monza", "ferrari_296_gt3", 120.0);
    let config = LiveTelemetryConfig {
        poll_hz: 120.0,
        chunk_rows,
    };

    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = BinaryTelemetryWriter::create(cursor, metadata, config).unwrap();

    for i in 0..n {
        let frame = make_frame(i as u64);
        writer.write_frame(&frame).unwrap();
    }

    let (cursor, summary) = writer.finish().unwrap();
    assert_eq!(summary.total_samples, n as u64);

    let reader = BinaryTelemetryReader::from_bytes(cursor.into_inner()).unwrap();
    assert_eq!(reader.metadata().track_name, "monza");
    assert_eq!(reader.metadata().car_model, "ferrari_296_gt3");

    // Verify all 9 clusters
    verify_cluster(&reader, n, "controls");
    verify_cluster(&reader, n, "motion");
    verify_cluster(&reader, n, "tyres");
    verify_cluster(&reader, n, "powertrain");
    verify_cluster(&reader, n, "session");
    verify_cluster(&reader, n, "timing");
    verify_cluster(&reader, n, "car_state");
    verify_cluster(&reader, n, "environment");
    verify_cluster(&reader, n, "other_cars");
}

fn verify_cluster(reader: &BinaryTelemetryReader, expected_count: usize, cluster: &str) {
    match cluster {
        "controls" => {
            let data = reader.read_all_controls().unwrap();
            assert_eq!(data.len(), expected_count, "controls count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.sample_tick, f.controls.sample_tick, "controls[{i}].sample_tick");
                assert_eq!(d.speed_kmh, f.controls.speed_kmh, "controls[{i}].speed_kmh");
                assert_eq!(d.gas, f.controls.gas, "controls[{i}].gas");
                assert_eq!(d.brake, f.controls.brake, "controls[{i}].brake");
                assert_eq!(d.steer_angle, f.controls.steer_angle, "controls[{i}].steer_angle");
                assert_eq!(d.gear, f.controls.gear, "controls[{i}].gear");
                assert_eq!(d.rpms, f.controls.rpms, "controls[{i}].rpms");
                assert_eq!(d.fuel, f.controls.fuel, "controls[{i}].fuel");
            }
        }
        "motion" => {
            let data = reader.read_all_motion().unwrap();
            assert_eq!(data.len(), expected_count, "motion count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.velocity, f.motion.velocity, "motion[{i}].velocity");
                assert_eq!(d.acc_g, f.motion.acc_g, "motion[{i}].acc_g");
                assert_eq!(d.local_velocity, f.motion.local_velocity, "motion[{i}].local_velocity");
                assert_eq!(d.local_angular_vel, f.motion.local_angular_vel, "motion[{i}].local_angular_vel");
                assert_eq!(d.heading, f.motion.heading, "motion[{i}].heading");
                assert_eq!(d.pitch, f.motion.pitch, "motion[{i}].pitch");
                assert_eq!(d.roll, f.motion.roll, "motion[{i}].roll");
            }
        }
        "tyres" => {
            let data = reader.read_all_tyres().unwrap();
            assert_eq!(data.len(), expected_count, "tyres count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.wheel_slip, f.tyres.wheel_slip, "tyres[{i}].wheel_slip");
                assert_eq!(d.wheel_load, f.tyres.wheel_load, "tyres[{i}].wheel_load");
                assert_eq!(d.tyre_contact_point, f.tyres.tyre_contact_point, "tyres[{i}].tyre_contact_point");
                assert_eq!(d.tyre_contact_normal, f.tyres.tyre_contact_normal, "tyres[{i}].tyre_contact_normal");
                assert_eq!(d.tyre_contact_heading, f.tyres.tyre_contact_heading, "tyres[{i}].tyre_contact_heading");
                assert_eq!(d.number_of_tyres_out, f.tyres.number_of_tyres_out, "tyres[{i}].number_of_tyres_out");
                assert_eq!(d.front_brake_compound, f.tyres.front_brake_compound, "tyres[{i}].front_brake_compound");
                assert_eq!(d.rear_brake_compound, f.tyres.rear_brake_compound, "tyres[{i}].rear_brake_compound");
            }
        }
        "powertrain" => {
            let data = reader.read_all_powertrain().unwrap();
            assert_eq!(data.len(), expected_count, "powertrain count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.turbo_boost, f.powertrain.turbo_boost, "powertrain[{i}].turbo_boost");
                assert_eq!(d.water_temp, f.powertrain.water_temp, "powertrain[{i}].water_temp");
                assert_eq!(d.current_max_rpm, f.powertrain.current_max_rpm, "powertrain[{i}].current_max_rpm");
            }
        }
        "session" => {
            let data = reader.read_all_session().unwrap();
            assert_eq!(data.len(), expected_count, "session count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.status, f.session.status, "session[{i}].status");
                assert_eq!(d.completed_laps, f.session.completed_laps, "session[{i}].completed_laps");
                assert_eq!(d.position, f.session.position, "session[{i}].position");
            }
        }
        "timing" => {
            let data = reader.read_all_timing().unwrap();
            assert_eq!(data.len(), expected_count, "timing count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.i_current_time, f.timing.i_current_time, "timing[{i}].i_current_time");
                assert_eq!(d.i_last_time, f.timing.i_last_time, "timing[{i}].i_last_time");
                assert_eq!(d.i_best_time, f.timing.i_best_time, "timing[{i}].i_best_time");
            }
        }
        "car_state" => {
            let data = reader.read_all_car_state().unwrap();
            assert_eq!(data.len(), expected_count, "car_state count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.ride_height, f.car_state.ride_height, "car_state[{i}].ride_height");
                assert_eq!(d.ignition_on, f.car_state.ignition_on, "car_state[{i}].ignition_on");
                assert_eq!(d.brake_bias, f.car_state.brake_bias, "car_state[{i}].brake_bias");
                assert_eq!(d.mfd_tyre_pressure, f.car_state.mfd_tyre_pressure, "car_state[{i}].mfd_tyre_pressure");
            }
        }
        "environment" => {
            let data = reader.read_all_environment().unwrap();
            assert_eq!(data.len(), expected_count, "environment count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.air_density, f.environment.air_density, "environment[{i}].air_density");
                assert_eq!(d.air_temp, f.environment.air_temp, "environment[{i}].air_temp");
                assert_eq!(d.road_temp, f.environment.road_temp, "environment[{i}].road_temp");
            }
        }
        "other_cars" => {
            let data = reader.read_all_other_cars().unwrap();
            assert_eq!(data.len(), expected_count, "other_cars count mismatch");
            for (i, d) in data.iter().enumerate() {
                let f = make_frame(i as u64);
                assert_eq!(d.player_car_id, f.other_cars.player_car_id, "other_cars[{i}].player_car_id");
                assert_eq!(d.active_cars, f.other_cars.active_cars, "other_cars[{i}].active_cars");
                assert_eq!(d.car_coordinates.len(), 180, "other_cars[{i}].car_coordinates len");
                assert_eq!(d.car_id.len(), 60, "other_cars[{i}].car_id len");
                assert_eq!(&d.car_coordinates[0..3], &f.other_cars.car_coordinates[0..3], "other_cars[{i}].car_coordinates[0..3]");
                assert_eq!(d.car_id[0], f.other_cars.car_id[0], "other_cars[{i}].car_id[0]");
            }
        }
        _ => panic!("unknown cluster: {cluster}"),
    }
}

// ---- Tests ----

#[test]
fn single_chunk_all_clusters() {
    // chunk_rows=10, frames=3 → all fit in one chunk per cluster
    roundtrip_all_clusters(3, 10);
}

#[test]
fn multi_chunk_all_clusters() {
    // chunk_rows=7, frames=23 → requires 4 chunks per cluster (3 full + 1 partial)
    roundtrip_all_clusters(23, 7);
}

#[test]
fn single_frame_all_clusters() {
    roundtrip_all_clusters(1, 1024);
}
