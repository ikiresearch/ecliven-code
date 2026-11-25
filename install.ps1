param(
    [string]$Version = $env:ECLIVEN_VERSION,
    [string]$InstallDir = $env:ECLIVEN_INSTALL_DIR,
    [string]$Repo = $env:ECLIVEN_REPO,
    [string]$LocalBinary = $env:ECLIVEN_LOCAL_BINARY,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {}

if (-not $Repo) {
    $Repo = "ikiresearch/ecliven-code"
}

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "Ecliven\bin"
}

if ($env:ECLIVEN_UNINSTALL -eq "1") {
    $Uninstall = $true
}

function Broadcast-EnvChange {
    try {
        if (-not ("Ecliven.NativeMethods" -as [type])) {
            Add-Type -Namespace "Ecliven" -Name "NativeMethods" -MemberDefinition @"
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
public static extern IntPtr SendMessageTimeout(
    IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam,
    uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
"@
        }
        $result = [UIntPtr]::Zero
        [void][Ecliven.NativeMethods]::SendMessageTimeout(
            [IntPtr]0xffff, 0x001A, [UIntPtr]::Zero, "Environment", 2, 5000, [ref]$result
        )
    } catch {}
}

function Add-ToUserPath {
    param([string]$Dir)

    $normalizedTarget = $Dir.TrimEnd('\')
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @()
    if ($userPath) {
        $entries = $userPath -split ';' | Where-Object { $_ -and $_.Trim() -ne "" }
    }

    $alreadyPresent = $false
    foreach ($entry in $entries) {
        if ($entry.TrimEnd('\') -ieq $normalizedTarget) {
            $alreadyPresent = $true
            break
        }
    }

    if (-not $alreadyPresent) {
        $newPath = if ($entries.Count -gt 0) {
            ($entries + $normalizedTarget) -join ';'
        } else {
            $normalizedTarget
        }
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Broadcast-EnvChange
        Write-Host "Added '$normalizedTarget' to User PATH." -ForegroundColor Green
    }

    $sessionEntries = ($env:PATH -split ';') | Where-Object { $_ }
    $inSession = $false
    foreach ($entry in $sessionEntries) {
        if ($entry.TrimEnd('\') -ieq $normalizedTarget) {
            $inSession = $true
            break
        }
    }
    if (-not $inSession) {
        $env:PATH = "$normalizedTarget;$env:PATH"
    }
}

function Remove-FromUserPath {
    param([string]$Dir)

    $normalizedTarget = $Dir.TrimEnd('\')
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not $userPath) { return }

    $filtered = $userPath -split ';' | Where-Object {
        $_ -and $_.Trim() -ne "" -and ($_.TrimEnd('\') -ine $normalizedTarget)
    }
    [Environment]::SetEnvironmentVariable("Path", ($filtered -join ';'), "User")
    Broadcast-EnvChange
}

if ($Uninstall) {
    $binPath = Join-Path $InstallDir "ecliven.exe"
    if (Test-Path $binPath) {
        Remove-Item $binPath -Force
        Write-Host "Removed $binPath" -ForegroundColor Yellow
    }
    Remove-FromUserPath -Dir $InstallDir
    Write-Host "Ecliven Code uninstalled." -ForegroundColor Green
    return
}

$arch = if ($env:PROCESSOR_ARCHITECTURE) {
    $env:PROCESSOR_ARCHITECTURE.ToUpperInvariant()
} else {
    "AMD64"
}
if ($arch -ne "AMD64" -and $arch -ne "X64" -and $arch -ne "ARM64") {
    throw "Unsupported Windows architecture: $arch. Expected AMD64 or ARM64."
}

function Install-BinarySafely {
    param(
        [string]$Source,
        [string]$Destination
    )

    $backup = "$Destination.old"
    if (Test-Path $backup) {
        Remove-Item -Path $backup -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path $Destination) {
        try {
            Move-Item -Path $Destination -Destination $backup -Force
        } catch {
            # If move fails, fall back to direct copy
        }
    }
    Copy-Item -Path $Source -Destination $Destination -Force
    if (Test-Path $backup) {
        Remove-Item -Path $backup -Force -ErrorAction SilentlyContinue
    }
}

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
$destExe = Join-Path $InstallDir "ecliven.exe"

if ($LocalBinary) {
    if (-not (Test-Path $LocalBinary)) {
        throw "Local binary not found: $LocalBinary"
    }
    Write-Host "Installing Ecliven Code from local binary '$LocalBinary'..." -ForegroundColor Cyan
    Install-BinarySafely -Source $LocalBinary -Destination $destExe
} else {
    $target = "x86_64-pc-windows-msvc"
    $assetName = "ecliven-$target.zip"

    if ($Version) {
        if (-not $Version.StartsWith("v")) {
            $Version = "v$Version"
        }
        $baseDownload = "https://github.com/$Repo/releases/download/$Version"
    } else {
        $baseDownload = "https://github.com/$Repo/releases/latest/download"
    }

    $zipUrl = "$baseDownload/$assetName"
    $sumsUrl = "$baseDownload/SHA256SUMS.txt"

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("ecliven-install-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        $zipPath = Join-Path $tempDir $assetName
        $sumsPath = Join-Path $tempDir "SHA256SUMS.txt"

        Write-Host "Downloading Ecliven Code from $zipUrl ..." -ForegroundColor Cyan
        Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing

        try {
            Invoke-WebRequest -Uri $sumsUrl -OutFile $sumsPath -UseBasicParsing
            $expectedLine = Get-Content $sumsPath | Where-Object { $_ -match [regex]::Escape($assetName) } | Select-Object -First 1
            if ($expectedLine) {
                $expectedHash = ($expectedLine -split '\s+')[0].Trim().ToLowerInvariant()
                $actualHash = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($actualHash -ne $expectedHash) {
                    throw "SHA-256 checksum mismatch for $assetName! Expected $expectedHash, got $actualHash."
                }
                Write-Host "SHA-256 checksum verified." -ForegroundColor DarkGray
            }
        } catch {
            if ($_.Exception.Message -like "*checksum mismatch*") {
                throw
            }
            Write-Warning "Could not verify SHA256SUMS.txt ($($_.Exception.Message)); continuing."
        }

        $extractDir = Join-Path $tempDir "extracted"
        Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force

        $foundExe = Get-ChildItem -Path $extractDir -Filter "ecliven.exe" -Recurse | Select-Object -First 1
        if (-not $foundExe) {
            throw "ecliven.exe was not found inside $assetName."
        }

        Install-BinarySafely -Source $foundExe.FullName -Destination $destExe
    } finally {
        if (Test-Path $tempDir) {
            Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Add-ToUserPath -Dir $InstallDir

$installedVersion = ((& $destExe --version 2>&1) | Out-String)
$versionText = if ($installedVersion) { $installedVersion.Trim() } else { "installed" }
Write-Host ""
Write-Host "Ecliven Code installed successfully!" -ForegroundColor Green
Write-Host "  Binary:  $destExe"
Write-Host "  Version: $versionText"
Write-Host ""
Write-Host "Run 'ecliven' right now in this terminal (or any new terminal)." -ForegroundColor Cyan
