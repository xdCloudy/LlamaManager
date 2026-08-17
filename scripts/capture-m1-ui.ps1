[CmdletBinding()]
param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot '..\target\release\llamamanager.exe'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts\m1-ui'),
    [int]$StartupTimeoutSeconds = 20,
    [switch]$KeepRunning
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$isWindowsHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows
)
if (-not $isWindowsHost) {
    throw 'M1 UI capture requires an interactive Windows desktop.'
}

if (-not [Environment]::UserInteractive) {
    throw 'M1 UI capture requires an interactive user session; headless CI is not valid visual evidence.'
}

$resolvedExecutable = (Resolve-Path -LiteralPath $ExecutablePath).Path
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$resolvedOutput = (Resolve-Path -LiteralPath $OutputDirectory).Path

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class LlamaManagerUiCaptureNative
{
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool MoveWindow(IntPtr hWnd, int X, int Y, int nWidth, int nHeight, bool bRepaint);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
'@

function Wait-ForMainWindow {
    param(
        [Parameter(Mandatory)] [System.Diagnostics.Process] $Process,
        [Parameter(Mandatory)] [int] $TimeoutSeconds
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($Process.HasExited) {
            throw "Application exited before opening a window (exit code $($Process.ExitCode))."
        }

        $Process.Refresh()
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) {
            return $Process.MainWindowHandle
        }

        Start-Sleep -Milliseconds 200
    }

    throw "Timed out after $TimeoutSeconds seconds waiting for the application window."
}

function Capture-Window {
    param(
        [Parameter(Mandatory)] [IntPtr] $Handle,
        [Parameter(Mandatory)] [int] $Width,
        [Parameter(Mandatory)] [int] $Height,
        [Parameter(Mandatory)] [string] $FilePath
    )

    $workingArea = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    if ($Width -gt $workingArea.Width -or $Height -gt $workingArea.Height) {
        throw "Requested ${Width}x${Height} capture exceeds the primary working area $($workingArea.Width)x$($workingArea.Height). Use a display large enough to show the complete window; clipped screenshots are not valid evidence."
    }

    [LlamaManagerUiCaptureNative]::ShowWindow($Handle, 9) | Out-Null
    if (-not [LlamaManagerUiCaptureNative]::MoveWindow($Handle, $workingArea.Left, $workingArea.Top, $Width, $Height, $true)) {
        throw "MoveWindow failed with Win32 error $([Runtime.InteropServices.Marshal]::GetLastWin32Error())."
    }

    [LlamaManagerUiCaptureNative]::SetForegroundWindow($Handle) | Out-Null
    Start-Sleep -Milliseconds 900

    $rect = New-Object LlamaManagerUiCaptureNative+RECT
    if (-not [LlamaManagerUiCaptureNative]::GetWindowRect($Handle, [ref]$rect)) {
        throw "GetWindowRect failed with Win32 error $([Runtime.InteropServices.Marshal]::GetLastWin32Error())."
    }

    $actualWidth = $rect.Right - $rect.Left
    $actualHeight = $rect.Bottom - $rect.Top
    if ($actualWidth -ne $Width -or $actualHeight -ne $Height) {
        throw "Window size mismatch after resize: requested ${Width}x${Height}, observed ${actualWidth}x${actualHeight}."
    }

    $bitmap = New-Object System.Drawing.Bitmap($actualWidth, $actualHeight)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size, [System.Drawing.CopyPixelOperation]::SourceCopy)
        $bitmap.Save($FilePath, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }

    return [ordered]@{
        width = $actualWidth
        height = $actualHeight
        path = $FilePath
    }
}

$process = $null
try {
    $process = Start-Process -FilePath $resolvedExecutable -PassThru
    $handle = Wait-ForMainWindow -Process $process -TimeoutSeconds $StartupTimeoutSeconds

    $captures = @()
    $captures += Capture-Window -Handle $handle -Width 1280 -Height 720 -FilePath (Join-Path $resolvedOutput 'llamawave-1280x720.png')
    $captures += Capture-Window -Handle $handle -Width 1600 -Height 900 -FilePath (Join-Path $resolvedOutput 'llamawave-1600x900.png')

    $process.Refresh()
    $manifest = [ordered]@{
        schema_version = 1
        captured_at_utc = [DateTime]::UtcNow.ToString('o')
        executable = $resolvedExecutable
        executable_sha256 = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        process_id = $process.Id
        window_title = $process.MainWindowTitle
        os = [Environment]::OSVersion.VersionString
        powershell = $PSVersionTable.PSVersion.ToString()
        captures = $captures
        evidence_note = 'Screenshots require human visual inspection. Their existence alone does not satisfy M1.1.'
    }

    $manifestPath = Join-Path $resolvedOutput 'manifest.json'
    $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestPath -Encoding utf8

    Write-Host "Captured M1 UI evidence to: $resolvedOutput"
    Write-Host 'Inspect both PNGs manually using docs/validation/M1_UI_VISUAL_VERIFICATION.md before recording acceptance.'
}
finally {
    if ($process -and -not $KeepRunning -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
    }
}
