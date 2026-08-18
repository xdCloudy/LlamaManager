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
$longConfigPath = Join-Path $externalRoot 'long-models.ini'
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

$longLines = [Collections.Generic.List[string]]::new()
$longLines.Add('; LlamaWave long rendered-editor responsiveness fixture')
$longLines.Add('[*]')
$longLines.Add('threads=8')
$longLines.Add('ctx-size=8192')
for ($i = 1; $i -le 4000; $i++) {
    $longLines.Add("; preserved comment line $i — 模型")
}
$longLines.Add('[large agent 模型]')
$longLines.Add('threads=10')
$longLines.Add('custom-user-key=preserve-this-too')
$longConfig = $longLines -join $crlf

[IO.File]::WriteAllText($configPath, $config, [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($longConfigPath, $longConfig, [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText(
    $invalidSamplePath,
    'this line is deliberately malformed',
    [Text.UTF8Encoding]::new($false)
)

$checklist = @"
LLAMAWAVE M3 CONFIG LAB — INTERACTIVE ACCEPTANCE
=================================================

External fixture:
$configPath

Long-file fixture:
$longConfigPath

Invalid raw sample:
$invalidSamplePath

A. NORMAL STRUCTURED + DIFF
1. Open CONFIG LAB -> OPEN EXTERNAL -> select the normal fixture above.
2. Verify [agent 模型], inherited/override badges, validation and empty diff.
3. STRUCTURED: change threads to 12. Verify DIFF FROM LOADED shows BEFORE/AFTER.

B. RAW ERROR / APPLY BLOCK
4. Switch to RAW. Add the invalid sample line anywhere outside a comment.
5. Verify RAW PARSE ERROR is visible and VALIDATE + SAVE is disabled.
6. Remove/fix the malformed line. Verify validation/diff recovers.

C. SAFE WRITE + RESTORE
7. Save. Verify the success banner reports a .bak path.
8. Use RESTORE BACKUP. Verify the pre-edit threads value returns.

D. MANAGED RESTART PERSISTENCE
9. OPEN MANAGED. Add a harmless key/value and save.
10. Close LlamaWave completely, reopen it, then OPEN MANAGED again and confirm the value persists.

E. LONG CONFIG + NARROW LAYOUT
11. OPEN EXTERNAL -> select the long-file fixture.
12. Switch RAW <-> STRUCTURED, scroll the long raw document and make one structured change. Verify the window remains responsive and no fake save/success state appears.
13. Resize to a narrow desktop window. Verify editor, validation/diff and bottom workspace switcher remain usable.

Capture screenshots of:
- normal structured editor with a non-empty diff
- raw parse-error/apply-blocked state
- narrow layout after successful save/restore (the long fixture can be used here)

Automated tests already cover invalid-write preservation, locked/read-only failures, backup/restore byte recovery, managed/external reopen, generated-profile semantic reopen, CRLF/comments/Unicode, and 10k-key canonical-session behavior. This checklist proves the remaining rendered desktop interaction/visual claims.
"@
[IO.File]::WriteAllText($checklistPath, $checklist, [Text.UTF8Encoding]::new($false))

$exe = Join-Path $repo 'target\release\llamamanager.exe'
if (-not (Test-Path $exe -PathType Leaf)) { throw "Release executable not found: $exe" }

Write-Host "Prepared M3 validation fixture: $configPath"
Write-Host "Prepared long rendered-editor fixture: $longConfigPath"
Write-Host "Checklist: $checklistPath"
Start-Process explorer.exe -ArgumentList $artifactRoot
Start-Process notepad.exe -ArgumentList $checklistPath
Start-Process -FilePath $exe
