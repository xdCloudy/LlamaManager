param(
    [switch]$SkipBuild,
    [string]$RouterHost = '127.0.0.1',
    [ValidateRange(1, 65535)]
    [int]$RouterPort = 8080,
    [switch]$NoLaunch
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows
)) {
    throw 'M6 telemetry validation requires Windows.'
}

if (-not [Environment]::UserInteractive) {
    throw 'M6 telemetry visual validation requires an interactive Windows desktop; headless CI is not visual evidence.'
}

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repo

if (-not $SkipBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release failed with exit code $LASTEXITCODE"
    }
}

$exe = Join-Path $repo 'target\release\llamamanager.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Release executable not found: $exe"
}

$artifactRoot = Join-Path $repo 'artifacts\m6-telemetry-validation'
New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
$checklistPath = Join-Path $artifactRoot 'M6_TELEMETRY_CHECKLIST.txt'
$environmentPath = Join-Path $artifactRoot 'environment.txt'

$tcpReachable = $false
$tcpError = $null
$client = [Net.Sockets.TcpClient]::new()
try {
    $connect = $client.ConnectAsync($RouterHost, $RouterPort)
    if (-not $connect.Wait([TimeSpan]::FromSeconds(2))) {
        $tcpError = 'connection timed out after 2 seconds'
    }
    elseif ($client.Connected) {
        $tcpReachable = $true
    }
    else {
        $tcpError = 'connection did not reach Connected state'
    }
}
catch {
    $tcpError = $_.Exception.Message
}
finally {
    $client.Dispose()
}

$exeHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
$endpoint = "${RouterHost}:$RouterPort"
$environment = @"
LLAMAWAVE M6 TELEMETRY VALIDATION ENVIRONMENT
=============================================
prepared_at_utc=$([DateTime]::UtcNow.ToString('o'))
executable=$exe
executable_sha256=$exeHash
router_endpoint=$endpoint
router_tcp_reachable=$tcpReachable
router_tcp_error=$tcpError
os=$([Environment]::OSVersion.VersionString)
powershell=$($PSVersionTable.PSVersion)

No API key or other secret is written by this helper.
"@
[IO.File]::WriteAllText($environmentPath, $environment, [Text.UTF8Encoding]::new($false))

$checklist = @"
LLAMAWAVE M6 TELEMETRY — INTERACTIVE ACCEPTANCE
===============================================

Prepared executable:
$exe
SHA-256: $exeHash

Configured router endpoint for this session:
$endpoint
TCP reachable during preparation: $tcpReachable

This checklist covers only the rendered/manual gates still open in issue #49.
Do not mark a step complete unless the real UI visibly demonstrated it.

A. LIVE PASSIVE + REQUEST-BOUND EVIDENCE
1. Open TELEMETRY.
2. Enter $RouterHost / $RouterPort and the API key in the UI if the runtime requires one. Do not put the key in this checklist or screenshots.
3. ATTACH PASSIVE MONITOR. Verify PASSIVE LIVE and verify values change while real inference is running.
4. When the one-slot model is actually free, RUN 4-TOKEN PROBE.
5. Verify request-bound PROMPT RATE, DECODE RATE, TTFT and REQUEST LATENCY become real values.
6. For MTP fields, accept only evidence actually exported by llama.cpp. If MTP remains UNAVAILABLE, record that limitation rather than treating it as zero or success.

B. DISCONNECT -> STALE / DISCONNECTED
7. With at least one good passive sample visible, stop the same runtime/router using its normal managed stop path.
8. Wait through more than one telemetry cadence. Verify the banner becomes DISCONNECTED or PASSIVE STALE instead of retaining a live-looking state.
9. Verify any retained values are explicitly labelled STALE and the latest poll error/reason remains visible.
10. Capture a screenshot of this state.

C. RECONNECT WITHOUT FAKE CONTINUITY
11. Restart the same runtime on $endpoint.
12. Verify passive telemetry automatically returns to PASSIVE LIVE with a new observation timestamp/fresh values.
13. If request-bound evidence existed before the interruption, verify it does NOT silently become LIVE again. Run a new 4-token probe before accepting fresh request evidence.
14. Capture a screenshot after recovery.

D. LIVE HISTORY — NORMAL + NARROW
15. Leave TELEMETRY open long enough to populate CPU/GPU LIVE HISTORY.
16. At a normal desktop size, verify chart labels, state, line/gap rendering, source identity and legend are readable with no clipping/overlap.
17. Resize to a narrow desktop window (roughly 650 px wide or the narrowest usable native window size).
18. Verify chart cards reflow, chart SVGs remain visible, labels/meta remain readable and the bottom workspace switcher remains usable.
19. Capture one normal and one narrow history screenshot.

E. ALERT FIRE + CLEAR
20. Scroll to EVIDENCE-BACKED ALERTS and note the current live CPU value.
21. Using the on-screen +/- controls, move the CPU TRIGGER/CLEAR thresholds only within the UI's valid hysteresis rules so the real CPU reading can sustain a trigger. Do not fabricate samples.
22. Wait for the documented 3 s / 4-live-sample window. Verify the instance progresses truthfully and a FIRED history event appears.
23. Move thresholds back so the real CPU reading is below CLEAR, wait through the debounce/window, and verify a RESOLVED history event appears.
24. Capture fired and resolved states. If ambient CPU cannot safely produce the transition, leave the visual gate open instead of running an unsafe load generator.

F. FINAL RECORD
25. Record exact LlamaWave source/build identity, llama.cpp version/build identity, model identity, router endpoint, screenshot names/hashes and any unsupported fields.
26. Confirm no screenshot contains an API key or other secret.
27. Keep any unsupported MTP/cache fields explicitly documented as runtime limitations.

Already covered by automation/runtime evidence and not repeated here:
- strict Rust fmt/check/test/Clippy/release pipeline;
- real Windows provider sanity and telemetry overhead budget;
- streaming probe/router fallback behavior;
- stale retention and transient child retry behavior;
- deterministic full-poll-failure -> next-cycle-fresh recovery.

The existence of this file is not visual acceptance evidence. The rendered checks above still require an interactive operator to observe them.
"@
[IO.File]::WriteAllText($checklistPath, $checklist, [Text.UTF8Encoding]::new($false))

Write-Host "Prepared M6 telemetry validation files: $artifactRoot"
Write-Host "Checklist: $checklistPath"
Write-Host "Environment: $environmentPath"
Write-Host "Router TCP reachable: $tcpReachable"
if ($tcpError) {
    Write-Warning "Router reachability detail: $tcpError"
}

if (-not $NoLaunch) {
    Start-Process explorer.exe -ArgumentList $artifactRoot
    Start-Process notepad.exe -ArgumentList $checklistPath
    Start-Process -FilePath $exe
}
