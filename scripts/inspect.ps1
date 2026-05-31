param(
    [Alias("Input")]
    [string]$InputPath = ".\data\mock.acctlm"
)

$ErrorActionPreference = "Stop"
cargo run --bin acc-live-telemetry -- inspect --input $InputPath
