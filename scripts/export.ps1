param(
    [Alias("Input")]
    [string]$InputPath = ".\data\mock.acctlm",
    [string]$Out = ".\data\mock.csv"
)

$ErrorActionPreference = "Stop"
$outDir = Split-Path -Parent $Out
if ($outDir -and -not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir | Out-Null
}

cargo run --bin acc-live-telemetry -- export --input $InputPath --out $Out --format csv
