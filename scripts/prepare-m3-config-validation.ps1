param(
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repo

if (-not $SkipBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed with exit code $LASTEXITCODE" }
}

$artifactRoot = Join-Path $repo 'artifacts\m3-config-validation'
$externalRoot = Join-Path $artifactRoot 'External Config 模型 with spaces'
New-Item -ItemType Directory -Force -Path $externalRoot | Out-Null

$configPath = Join-Path $externalRoot 'models.ini'
$invalidSamplePath = Join-Path $artifactRoot 'INVALID_RAW_SAMPLE.txt'
$checklistPath = Join-Path $artifactRoot 'M3_GUI_CHECKLIST.txt'

$crlf = "`r`n"
$config = @(
    '; LlamaWave M3 interactive validation fixture'
    '# comments, CRLF, Unicode and unknown keys must survive safe edits'
    ''
    '[*]'
    'threads=8'
    'ctx-size=8192'
    'future-option=keep this unknown value 模型'
    ''
    '[agent 模型]'
    'model=C:\AI Models\Agent 模型.gguf'
    'threads=10'
    'custom-user-key=preserve-me'
    ''
) -join $crlf

[IO.File]::WriteAllText($configPath, $config, [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($invalidSamplePath, "this line is deliberately malformed", [Text.UTF8Encoding]::new($false))

$checklist = @"
LLAMAWAVE M3 CONFIG LAB — INTERACTIVE ACCEPTANCE
=================================================

External fixture:
$configPath

Invalid raw sample:
$invalidSamplePath

1. Open CONFIG LAB -> OPEN EXTERNAL -> select the fixture above.
2. Normal width: verify [agent 模型], inherited/override badges, validation and empty diff.
3. STRUCTURED: change threads to 12. Verify the diff shows BEFORE/AFTER and the unknown/comment-heavy source remains intact.
4. RAW: add the invalid sample line. Verify the raw parse error is visible and VALIDATE + SAVE is disabled.
5. Remove/fix the malformed line. Save. Verify success reports a .bak path.
6. Use RESTORE BACKUP. Verify the pre-edit value returns.
7. OPEN MANAGED. Add a harmless key/value, save, close LlamaWave, reopen it, then OPEN MANAGED again and confirm persistence.
8. Resize to a narrow desktop window and verify editor, validation/diff and bottom workspace switcher remain usable.

Capture screenshots of:
- normal structured editor + diff
- raw parse-error state
- narrow layout after successful save/restore

No need to create a deliberately corrupt external file outside this fixture; automated tests already cover failed-write and restore recovery semantics.
"@
[IO.File]::WriteAllText($checklistPath, $checklist, [Text.UTF8Encoding]::new($false))

$exe = Join-Path $repo 'target\release\llamamanager.exe'
if (-not (Test-Path $exe -PathType Leaf)) { throw "Release executable not found: $exe" }

Write-Host "Prepared M3 validation fixture: $configPath"
Write-Host "Checklist: $checklistPath"
Start-Process explorer.exe -ArgumentList $artifactRoot
Start-Process notepad.exe -ArgumentList $checklistPath
Start-Process -FilePath $exe
