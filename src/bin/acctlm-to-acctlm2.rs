//! CLI converter: losslessly convert .acctlm (v1) to .acctlm2 (v2) format.
//!
//! Usage:
//!   acctlm-to-acctlm2 [--force] <input.acctlm> [output.acctlm2]
//!
//! If output is not specified, it is derived from the input path by
//! appending "2" (e.g., "foo.acctlm" → "foo.acctlm2").

use module_live_telemetry::writer_v2::BinaryTelemetryWriterV2;
use module_live_telemetry::{
    BinaryTelemetryReader, LiveTelemetryConfig, TelemetryError, TelemetryFrame, TelemetryResult,
};
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> TelemetryResult<()> {
    // ---- Argument parsing ----
    let args: Vec<String> = env::args().collect();
    let mut force = false;
    let mut positional: Vec<String> = Vec::new();

    for arg in &args[1..] {
        if arg == "--force" || arg == "-f" {
            force = true;
        } else if arg.starts_with('-') {
            eprintln!("error: unknown flag '{}'", arg);
            print_usage();
            return Ok(());
        } else {
            positional.push(arg.clone());
        }
    }

    if positional.is_empty() {
        print_usage();
        return Ok(());
    }

    let input_path = PathBuf::from(&positional[0]);
    if !input_path.exists() {
        return Err(TelemetryError::InvalidArgument(format!(
            "input file not found: '{}'",
            input_path.display()
        )));
    }

    let output_path = if positional.len() >= 2 {
        PathBuf::from(&positional[1])
    } else {
        derive_output_path(&input_path)
    };

    // Check if output already exists
    if output_path.exists() && !force {
        eprintln!(
            "error: output file '{}' already exists. Use --force to overwrite.",
            output_path.display()
        );
        std::process::exit(1);
    }

    // ---- Step 1: Open v1 file ----
    eprintln!("Reading {}...", input_path.display());
    let reader = BinaryTelemetryReader::open(&input_path).map_err(|e| {
        TelemetryError::InvalidArgument(format!("failed to open '{}': {}", input_path.display(), e))
    })?;
    let metadata = reader.metadata().clone();

    // ---- Step 2: Read all cluster samples ----
    let controls = reader.read_all_controls()?;
    let motion = reader.read_all_motion()?;
    let tyres = reader.read_all_tyres()?;
    let powertrain = reader.read_all_powertrain()?;
    let session = reader.read_all_session()?;
    let timing = reader.read_all_timing()?;
    let car_state = reader.read_all_car_state()?;
    let environment = reader.read_all_environment()?;
    let other_cars = reader.read_all_other_cars()?;

    let n = controls.len();

    // Validate all clusters have the same frame count
    let counts = [
        ("motion", motion.len()),
        ("tyres", tyres.len()),
        ("powertrain", powertrain.len()),
        ("session", session.len()),
        ("timing", timing.len()),
        ("car_state", car_state.len()),
        ("environment", environment.len()),
        ("other_cars", other_cars.len()),
    ];
    for (name, count) in &counts {
        if *count != n {
            return Err(TelemetryError::InvalidFormat(format!(
                "frame count mismatch: controls have {n} frames, {name} has {count}"
            )));
        }
    }

    // ---- Step 3: Assemble TelemetryFrames ----
    let frames: Vec<TelemetryFrame> = (0..n)
        .map(|i| TelemetryFrame {
            sample_tick: controls[i].sample_tick,
            timestamp_ns: controls[i].timestamp_ns,
            controls: controls[i],
            motion: motion[i],
            tyres: tyres[i].clone(),
            powertrain: powertrain[i],
            session: session[i].clone(),
            timing: timing[i].clone(),
            car_state: car_state[i].clone(),
            environment: environment[i],
            other_cars: other_cars[i].clone(),
        })
        .collect();

    // ---- Step 4: Write v2 file ----
    eprintln!("Converting {n} frames...");
    let config = LiveTelemetryConfig {
        poll_hz: metadata.poll_hz,
        chunk_rows: metadata.chunk_rows,
    };

    let mut writer = BinaryTelemetryWriterV2::create_file(&output_path, metadata.clone(), config)
        .map_err(|e| {
        TelemetryError::InvalidArgument(format!(
            "failed to create output file '{}': {}",
            output_path.display(),
            e
        ))
    })?;

    for (idx, frame) in frames.iter().enumerate() {
        writer.write_frame(frame).map_err(|e| {
            TelemetryError::InvalidArgument(format!("failed to write frame {idx}: {e}"))
        })?;
        if (idx + 1) % 1000 == 0 {
            eprintln!("  {}/{} frames written...", idx + 1, n);
        }
    }

    eprintln!("Finishing output file...");
    let summary = writer.finish().map_err(|e| {
        TelemetryError::InvalidArgument(format!("failed to finish output file: {e}"))
    })?;

    // ---- Step 5: Verify roundtrip ----
    eprintln!("Verifying output...");
    let v2_reader = BinaryTelemetryReader::open(&output_path).map_err(|e| {
        TelemetryError::InvalidArgument(format!("failed to open output for verification: {e}"))
    })?;
    let v2_frames = v2_reader.read_all_frames()?;

    if v2_frames.len() != n {
        return Err(TelemetryError::InvalidFormat(format!(
            "frame count mismatch — input: {n}, output: {}",
            v2_frames.len()
        )));
    }

    let mut mismatches = 0u64;
    for i in 0..n {
        let f1 = &frames[i];
        let f2 = &v2_frames[i];
        if f1.sample_tick != f2.sample_tick
            || f1.timestamp_ns != f2.timestamp_ns
            || f1.controls.speed_kmh != f2.controls.speed_kmh
            || f1.controls.gas != f2.controls.gas
            || f1.controls.brake != f2.controls.brake
            || f1.motion.velocity != f2.motion.velocity
        {
            mismatches += 1;
            if mismatches <= 5 {
                eprintln!(
                    "  mismatch at frame {i}: tick {}/{}",
                    f1.sample_tick, f2.sample_tick
                );
            }
        }
    }
    if mismatches > 0 {
        return Err(TelemetryError::InvalidFormat(format!(
            "{mismatches} frames have field mismatches"
        )));
    }
    eprintln!("Verification passed: all {n} frames roundtrip correctly");

    // ---- Step 6: Summary ----
    let output_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(summary.total_bytes);

    eprintln!(
        "Converted {n} frames from {} ({}) — output: {} ({} bytes, {} row groups)",
        metadata.track_name,
        metadata.car_model,
        output_path.display(),
        output_size,
        summary.chunk_count,
    );

    Ok(())
}

fn print_usage() {
    eprintln!("Usage: acctlm-to-acctlm2 [--force] <input.acctlm> [output.acctlm2]");
    eprintln!();
    eprintln!("Losslessly convert a v1 .acctlm file to the v2 .acctlm2 format.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --force, -f    Overwrite existing output file");
    eprintln!();
    eprintln!("If output path is not specified, it is derived from the input path");
    eprintln!("by appending \"2\" (e.g., \"data.acctlm\" → \"data.acctlm2\").");
}

fn derive_output_path(input: &Path) -> PathBuf {
    let s = input.to_string_lossy();
    if s.ends_with(".acctlm") {
        // "foo.acctlm" → "foo.acctlm2"
        PathBuf::from(format!("{s}2"))
    } else {
        // "foo.bar" → "foo.bar.acctlm2"
        PathBuf::from(format!("{s}.acctlm2"))
    }
}
