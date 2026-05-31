param(
    [string]$OutDir = ".\data",
    [string]$Out = "",
    [double]$PollHz = 120,
    [int]$ChunkRows = 256,
    [int]$FlushIntervalMs = 2000
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path $OutDir)) {
    New-Item -ItemType Directory -Path $OutDir | Out-Null
}

if ($Out -ne "") {
    cargo run --bin acc-live-telemetry -- record-auto --out $Out --poll-hz $PollHz --chunk-rows $ChunkRows --flush-interval-ms $FlushIntervalMs
} else {
    cargo run --bin acc-live-telemetry -- record-auto --out-dir $OutDir --poll-hz $PollHz --chunk-rows $ChunkRows --flush-interval-ms $FlushIntervalMs
}
