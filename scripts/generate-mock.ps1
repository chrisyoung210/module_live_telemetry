param(
    [string]$Out = ".\data\mock.acctlm",
    [int]$Samples = 1000,
    [double]$PollHz = 120,
    [int]$ChunkRows = 256
)

$ErrorActionPreference = "Stop"
$outDir = Split-Path -Parent $Out
if ($outDir -and -not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir | Out-Null
}

cargo run --bin acc-live-telemetry -- generate-mock --out $Out --samples $Samples --poll-hz $PollHz --chunk-rows $ChunkRows
