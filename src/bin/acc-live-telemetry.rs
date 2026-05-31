use module_live_telemetry::{
    mock::generate_mock_controls, BinaryTelemetryReader, BinaryTelemetryWriter, ControlSample,
    LiveTelemetryConfig, SessionMetadata, TelemetryError, TelemetryFrame, TelemetryResult,
};
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> TelemetryResult<()> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };
    let args: Vec<String> = args.collect();
    match command.as_str() {
        "generate-mock" => generate_mock_command(&args),
        "inspect" => inspect_command(&args),
        "export" => export_command(&args),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => Err(TelemetryError::InvalidArgument(format!(
            "unknown command '{command}'"
        ))),
    }
}

fn generate_mock_command(args: &[String]) -> TelemetryResult<()> {
    let out = required_path(args, "--out")?;
    let samples = optional_usize(args, "--samples", 1_000)?;
    let poll_hz = optional_f64(args, "--poll-hz", 120.0)?;
    let chunk_rows = optional_usize(args, "--chunk-rows", 256)?;
    let track = optional_string(args, "--track", "monza");
    let car = optional_string(args, "--car", "mclaren_720s_gt3_evo");

    let metadata = SessionMetadata::new(track, car, poll_hz);
    let config = LiveTelemetryConfig {
        poll_hz,
        chunk_rows,
    };
    let mut writer = BinaryTelemetryWriter::create_file(&out, metadata, config)?;
    for sample in generate_mock_controls(samples, poll_hz) {
        let frame = TelemetryFrame {
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
        };
        writer.write_frame(frame)?;
    }
    let (_file, summary) = writer.finish()?;
    println!(
        "wrote {} samples in {} chunks to {} ({} bytes)",
        summary.total_samples,
        summary.chunk_count,
        out.display(),
        summary.total_bytes
    );
    Ok(())
}

fn inspect_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let reader = BinaryTelemetryReader::open(&input)?;
    let metadata = reader.metadata();
    let summary = reader.summary();
    println!("file: {}", input.display());
    println!("format: ACTL v{}", reader.header().version);
    println!("track: {}", metadata.track_name);
    println!("car: {}", metadata.car_model);
    println!("poll_hz: {:.3}", metadata.poll_hz);
    println!("sm_version: {}", metadata.sm_version);
    println!("ac_version: {}", metadata.ac_version);
    println!("chunks: {}", summary.chunk_count);
    println!("samples: {}", summary.total_samples);
    println!("bytes: {}", summary.total_bytes);
    println!("footer_offset: {}", summary.footer_offset);
    for entry in reader.chunk_index() {
        println!(
            "chunk #{:04} cluster=0x{:04x} samples={} ticks={}..{} time_ns={}..{} offset={} bytes={}",
            entry.chunk_seq,
            entry.cluster_id,
            entry.end_tick.saturating_sub(entry.start_tick).saturating_add(1),
            entry.start_tick,
            entry.end_tick,
            entry.start_time_ns,
            entry.end_time_ns,
            entry.file_offset,
            entry.byte_len
        );
    }
    Ok(())
}

fn export_command(args: &[String]) -> TelemetryResult<()> {
    let input = required_path(args, "--input")?;
    let out = optional_path(args, "--out");
    let format = optional_string(args, "--format", "csv");
    if format != "csv" {
        return Err(TelemetryError::InvalidArgument(
            "only --format csv is supported in the MVP".to_string(),
        ));
    }

    let reader = BinaryTelemetryReader::open(input)?;
    let samples = reader.read_all_controls()?;
    match out {
        Some(path) => {
            let file = File::create(path)?;
            let mut writer = BufWriter::new(file);
            write_csv(&mut writer, &samples)?;
            writer.flush()?;
        }
        None => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            write_csv(&mut lock, &samples)?;
        }
    }
    Ok(())
}

fn write_csv(writer: &mut impl Write, samples: &[ControlSample]) -> TelemetryResult<()> {
    writeln!(writer, "{}", ControlSample::csv_header())?;
    for sample in samples {
        writeln!(writer, "{}", sample.to_csv_row())?;
    }
    Ok(())
}

// ---- Argument helpers ----

fn required_path(args: &[String], flag: &str) -> TelemetryResult<PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing {flag} <path>")))
}

fn optional_path(args: &[String], flag: &str) -> Option<PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

fn optional_string<'a>(args: &'a [String], flag: &str, default: &'a str) -> &'a str {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or(default)
}

fn optional_usize(args: &[String], flag: &str, default: usize) -> TelemetryResult<usize> {
    match args.iter().position(|a| a == flag) {
        Some(i) => args
            .get(i + 1)
            .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing value for {flag}")))?
            .parse::<usize>()
            .map_err(|e| TelemetryError::InvalidArgument(format!("invalid {flag}: {e}"))),
        None => Ok(default),
    }
}

fn optional_f64(args: &[String], flag: &str, default: f64) -> TelemetryResult<f64> {
    match args.iter().position(|a| a == flag) {
        Some(i) => args
            .get(i + 1)
            .ok_or_else(|| TelemetryError::InvalidArgument(format!("missing value for {flag}")))?
            .parse::<f64>()
            .map_err(|e| TelemetryError::InvalidArgument(format!("invalid {flag}: {e}"))),
        None => Ok(default),
    }
}

fn print_usage() {
    println!(
        "ACC live telemetry\n\n\
commands:\n  \
generate-mock --out <file> [--samples 1000] [--poll-hz 120] [--chunk-rows 256]\n  \
inspect --input <file>\n  \
export --input <file> [--out <file>] [--format csv]\n  \
help"
    )
}