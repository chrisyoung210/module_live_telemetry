param(
    [Alias("Input")]
    [string]$InputPath = ".\data\live_1780226240_monza_mclaren_720s_gt3_evo.acctlm"
)

$ErrorActionPreference = "Stop"
cargo run --bin acc-live-telemetry -- raw-lap-segments --input $InputPath
