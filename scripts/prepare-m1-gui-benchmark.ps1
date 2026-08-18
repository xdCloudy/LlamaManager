[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts\m1-gui-benchmark'),
    [switch]$SkipBuild,
    [switch]$NoLaunch
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$isWindowsHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows
)
if (-not $isWindowsHost) {
    throw 'M1 GUI benchmark verification requires Windows.'
}
if (-not [Environment]::UserInteractive) {
    throw 'M1 GUI benchmark verification requires an interactive Windows desktop.'
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $OutputDirectory).Path

$releaseTag = 'b10472'
$archiveUrl = 'https://github.com/ggml-org/llama.cpp/releases/download/b10472/llama-b10472-bin-win-cpu-x64.zip'
$archiveSha256 = 'ef495329c85c171991972fd3226a179c1900368cab66e2ebba8b21a7471a74e5'
$modelUrl = 'https://huggingface.co/ggml-org/tiny-llamas/resolve/6e091d820cbe8f22eeb604d136403eca290b8c1e/stories15M-q4_0.gguf?download=true'
$modelSha256 = '6151b1929d7f5aa3385d9ddef3393e55587c0a55de661562322bc51dfda93a04'
$expectedServerSha256 = '76b0a5f72243ccb99079ca71ebd0332f123c52668d815c9d6716a89d46415668'
$expectedBenchSha256 = '97495a77c5f6d528f9eeff0a43a692574951a6b98e673e637366dd7dfce07d4f'

$downloadRoot = Join-Path $outputRoot 'downloads'
$runtimeRoot = Join-Path $outputRoot 'llama cpp runtime with spaces'
$modelRoot = Join-Path $outputRoot 'Model Files with spaces'
$archivePath = Join-Path $downloadRoot 'llama-b10472-win-cpu-x64.zip'
$modelPath = Join-Path $modelRoot 'stories 15M benchmark.gguf'

New-Item -ItemType Directory -Force -Path $downloadRoot, $modelRoot | Out-Null

function Assert-Sha256 {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Expected,
        [Parameter(Mandatory)] [string] $Label
    )

    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Expected.ToLowerInvariant()) {
        throw "$Label SHA-256 mismatch. Expected $Expected, got $actual"
    }
    return $actual
}

Write-Host "Preparing M1 GUI benchmark evidence in: $outputRoot" -ForegroundColor Cyan

if (-not (Test-Path -LiteralPath $archivePath)) {
    Write-Host 'Downloading pinned llama.cpp Windows CPU release...'
    Invoke-WebRequest -Uri $archiveUrl -OutFile $archivePath
}
$verifiedArchiveSha = Assert-Sha256 -Path $archivePath -Expected $archiveSha256 -Label 'llama.cpp archive'

if (Test-Path -LiteralPath $runtimeRoot) {
    Remove-Item -LiteralPath $runtimeRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
Expand-Archive -LiteralPath $archivePath -DestinationPath $runtimeRoot -Force

$server = Get-ChildItem -LiteralPath $runtimeRoot -Filter 'llama-server.exe' -File -Recurse | Select-Object -First 1
$bench = Get-ChildItem -LiteralPath $runtimeRoot -Filter 'llama-bench.exe' -File -Recurse | Select-Object -First 1
if (-not $server -or -not $bench) {
    throw 'Pinned llama.cpp archive did not contain llama-server.exe and llama-bench.exe.'
}
$serverSha = Assert-Sha256 -Path $server.FullName -Expected $expectedServerSha256 -Label 'llama-server.exe'
$benchSha = Assert-Sha256 -Path $bench.FullName -Expected $expectedBenchSha256 -Label 'llama-bench.exe'

if (-not (Test-Path -LiteralPath $modelPath)) {
    Write-Host 'Downloading pinned tiny real GGUF model...'
    Invoke-WebRequest -Uri $modelUrl -OutFile $modelPath
}
$verifiedModelSha = Assert-Sha256 -Path $modelPath -Expected $modelSha256 -Label 'GGUF model'

$exePath = Join-Path $repoRoot 'target\release\llamamanager.exe'
if (-not $SkipBuild) {
    Write-Host 'Building current release...'
    Push-Location $repoRoot
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $exePath)) {
    throw "Release executable is missing: $exePath"
}
$exeSha = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
$sourceCommit = (& git -C $repoRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to resolve repository source commit.'
}

$instructionsPath = Join-Path $outputRoot 'GUI_BENCHMARK_CHECK.txt'
$manifestPath = Join-Path $outputRoot 'prepared-evidence.json'

$instructions = @"
M1 C5 — final interactive GUI benchmark check

Prepared source commit:
$sourceCommit

Llama.cpp root to select in LlamaManager:
$runtimeRoot

GGUF model to select in LlamaManager:
$modelPath

Human interaction required:
1. In the real LlamaManager release window, select the llama.cpp installation above.
2. Select/add the GGUF model above.
3. From the GUI, start the benchmark.
4. Verify the UI shows the real llama-bench invocation/result rather than placeholder state.
5. Verify the run succeeds and real benchmark metrics/history are visible.
6. Capture a screenshot of the successful GUI benchmark/history state.

Expected identities:
llama-server SHA-256: $serverSha
llama-bench  SHA-256: $benchSha
model        SHA-256: $verifiedModelSha
LlamaManager SHA-256: $exeSha

This interaction is the remaining C5 evidence gate. Do not mark it passed merely because this preparation script succeeds.
"@
Set-Content -LiteralPath $instructionsPath -Value $instructions -Encoding utf8

$manifest = [ordered]@{
    schema_version = 1
    prepared_at_utc = [DateTime]::UtcNow.ToString('o')
    source_commit = $sourceCommit
    llamamanager_executable = $exePath
    llamamanager_sha256 = $exeSha
    llama_release_tag = $releaseTag
    llama_archive_url = $archiveUrl
    llama_archive_sha256 = $verifiedArchiveSha
    llama_root = $runtimeRoot
    llama_server = $server.FullName
    llama_server_sha256 = $serverSha
    llama_bench = $bench.FullName
    llama_bench_sha256 = $benchSha
    model_url = $modelUrl
    model_path = $modelPath
    model_sha256 = $verifiedModelSha
    os = [Environment]::OSVersion.VersionString
    powershell = $PSVersionTable.PSVersion.ToString()
    evidence_state = 'prepared_not_human_verified'
    remaining_manual_action = 'Launch a real benchmark through the LlamaManager GUI and record the successful UI result.'
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestPath -Encoding utf8

Write-Host ''
Write-Host 'Preparation complete.' -ForegroundColor Green
Write-Host "Instructions: $instructionsPath"
Write-Host "Manifest:     $manifestPath"
Write-Host "Runtime root: $runtimeRoot"
Write-Host "Model path:   $modelPath"

if (-not $NoLaunch) {
    Write-Host 'Launching the real release window...' -ForegroundColor Cyan
    Start-Process -FilePath $exePath | Out-Null
    Start-Process explorer.exe -ArgumentList "`"$outputRoot`"" | Out-Null
    Start-Process notepad.exe -ArgumentList "`"$instructionsPath`"" | Out-Null
}
