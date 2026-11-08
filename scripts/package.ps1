[CmdletBinding()]
param(
    [string]$AppRoot = '',
    [string]$OutDir = '',
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?$')]
    [string]$Version = '0.5.0',
    [switch]$PreflightOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $AppRoot) { $AppRoot = Split-Path -Parent $PSScriptRoot }

# This script packages existing release binaries. It never compiles, downloads,
# installs, publishes, or recursively copies a source/application directory.
$taskRoot = (Resolve-Path -LiteralPath $AppRoot).ProviderPath
if (-not $OutDir) { $OutDir = Join-Path $taskRoot 'dist' }
$taskOutFull = [IO.Path]::GetFullPath($OutDir)
$taskName = 'Jimoto-Honyaku-kun-' + $Version + '-win-x64'
$taskZip = Join-Path $taskOutFull ($taskName + '.zip')
$taskSidecar = $taskZip + '.sha256'
$taskManifestOut = Join-Path $taskOutFull ($taskName + '.manifest.json')

# Relative source -> relative archive destination. Every shipped entry is explicit.
$taskFiles = @(
    @{ Source = 'target/x86_64-pc-windows-msvc/release/plamo-translate.exe'; Entry = 'plamo-translate.exe' },
    @{ Source = 'target/x86_64-pc-windows-msvc/release/jimoto-setup.exe'; Entry = 'jimoto-setup.exe' },
    @{ Source = 'Talk風Live翻訳.cmd'; Entry = 'Talk風Live翻訳.cmd' },
    @{ Source = 'start.cmd'; Entry = 'start.cmd' },
    @{ Source = 'start.ps1'; Entry = 'start.ps1' },
    @{ Source = 'setup.ps1'; Entry = 'setup.ps1' },
    @{ Source = 'scripts/asr_server.py'; Entry = 'scripts/asr_server.py' },
    @{ Source = 'scripts/setup-audio.ps1'; Entry = 'scripts/setup-audio.ps1' },
    @{ Source = 'scripts/audio-runtime.json'; Entry = 'scripts/audio-runtime.json' },
    @{ Source = 'scripts/requirements.txt'; Entry = 'scripts/requirements.txt' },
    @{ Source = 'extension/manifest.json'; Entry = 'extension/manifest.json' },
    @{ Source = 'extension/content.js'; Entry = 'extension/content.js' },
    @{ Source = 'extension/background.js'; Entry = 'extension/background.js' },
    @{ Source = 'extension/options.html'; Entry = 'extension/options.html' },
    @{ Source = 'extension/options.js'; Entry = 'extension/options.js' },
    @{ Source = 'extension/icons/icon-16.png'; Entry = 'extension/icons/icon-16.png' },
    @{ Source = 'extension/icons/icon-32.png'; Entry = 'extension/icons/icon-32.png' },
    @{ Source = 'extension/icons/icon-48.png'; Entry = 'extension/icons/icon-48.png' },
    @{ Source = 'extension/icons/icon-128.png'; Entry = 'extension/icons/icon-128.png' },
    @{ Source = 'extension/README.md'; Entry = 'extension/README.md' },
    @{ Source = 'README.md'; Entry = 'README.md' },
    @{ Source = 'LICENSE'; Entry = 'LICENSE' },
    @{ Source = 'docs/SPECIFICATIONS.md'; Entry = 'docs/SPECIFICATIONS.md' },
    @{ Source = 'docs/DISTRIBUTION.md'; Entry = 'docs/DISTRIBUTION.md' },
    @{ Source = 'docs/VALIDATION.md'; Entry = 'docs/VALIDATION.md' }
)
$taskFiles += @{ Source = 'assets/icons/jimoto.svg'; Entry = 'assets/icons/jimoto.svg' }
$taskFiles += @{ Source = 'assets/screenshots/first-run.png'; Entry = 'assets/screenshots/first-run.png' }
if (Test-Path -LiteralPath (Join-Path $taskRoot 'assets/fonts/OFL.txt') -PathType Leaf) {
    $taskFiles += @{ Source = 'assets/fonts/OFL.txt'; Entry = 'assets/fonts/OFL.txt' }
    $taskFiles += @{ Source = 'assets/fonts/README.md'; Entry = 'assets/fonts/README.md' }
}

$taskRootPrefix = $taskRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
foreach ($taskFile in $taskFiles) {
    $taskSource = [IO.Path]::GetFullPath((Join-Path $taskRoot $taskFile.Source))
    if (-not $taskSource.StartsWith($taskRootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw ('Source escaped AppRoot: ' + $taskFile.Entry)
    }
    if (-not (Test-Path -LiteralPath $taskSource -PathType Leaf)) {
        throw ('Required package file is missing: ' + $taskFile.Source)
    }
    if ((Get-Item -LiteralPath $taskSource).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw ('Reparse-point source is not permitted: ' + $taskFile.Entry)
    }
    if ($taskFile.Entry -match '(^|/)(models|runtime|\.venv|verification|userdata|data|logs|\.git|temp)(/|$)') {
        throw ('Forbidden archive entry: ' + $taskFile.Entry)
    }
}
$taskLauncher = Get-Content -LiteralPath (Join-Path $taskRoot 'start.ps1') -Raw -Encoding UTF8
if (-not [regex]::IsMatch($taskLauncher, 'Join-Path\s+\$taskAppRoot\s+[''"]plamo-translate\.exe[''"]')) {
    throw 'start.ps1 does not select the portable root executable. Update the launcher before packaging.'
}
foreach ($taskOutput in @($taskZip, $taskSidecar, $taskManifestOut)) {
    if (Test-Path -LiteralPath $taskOutput) { throw ('Output already exists; use a new version or OutDir: ' + [IO.Path]::GetFileName($taskOutput)) }
}
Write-Output ('Preflight passed: ' + $taskFiles.Count + ' explicitly allowed files; no weights, runtime, settings or tokens.')
if ($PreflightOnly) {
    $taskFiles | ForEach-Object { Write-Output $_.Entry }
    return
}

New-Item -ItemType Directory -Path $taskOutFull -Force | Out-Null
$taskOutCanonical = (Resolve-Path -LiteralPath $taskOutFull).ProviderPath.TrimEnd('\', '/')
$taskStageName = '.jimoto-package-' + [Guid]::NewGuid().ToString('N')
$taskStage = Join-Path $taskOutCanonical $taskStageName
if (Test-Path -LiteralPath $taskStage) { throw 'Unique staging directory unexpectedly exists.' }
New-Item -ItemType Directory -Path $taskStage | Out-Null
$taskStageCanonical = (Resolve-Path -LiteralPath $taskStage).ProviderPath
$taskOutPrefix = $taskOutCanonical + [IO.Path]::DirectorySeparatorChar
if (-not $taskStageCanonical.StartsWith($taskOutPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Staging directory is outside the resolved output directory.'
}

try {
    $taskEntries = @()
    foreach ($taskFile in $taskFiles) {
        $taskDestination = Join-Path $taskStageCanonical $taskFile.Entry
        $taskParent = Split-Path -Parent $taskDestination
        New-Item -ItemType Directory -Path $taskParent -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $taskRoot $taskFile.Source) -Destination $taskDestination
        $taskEntries += [ordered]@{
            path = $taskFile.Entry
            bytes = (Get-Item -LiteralPath $taskDestination).Length
            sha256 = (Get-FileHash -LiteralPath $taskDestination -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $taskManifest = [ordered]@{
        app = 'Jimoto-Honyaku-kun'
        version = $Version
        platform = 'windows-x86_64'
        files = $taskEntries
        additional_entry = 'package-manifest.json'
        excluded_by_design = @('model weights', 'downloaded runtimes', 'virtual environments', 'verification', 'user settings', 'tokens', 'logs', 'repository metadata')
    } | ConvertTo-Json -Depth 6
    $taskUtf8 = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText((Join-Path $taskStageCanonical 'package-manifest.json'), $taskManifest, $taskUtf8)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    # CreateFromDirectory uses CreateNew semantics; an existing ZIP is never replaced.
    [IO.Compression.ZipFile]::CreateFromDirectory($taskStageCanonical, $taskZip, [IO.Compression.CompressionLevel]::Optimal, $false)
    $taskArchive = [IO.Compression.ZipFile]::OpenRead($taskZip)
    try {
        $taskExpected = @($taskFiles | ForEach-Object { $_.Entry }) + @('package-manifest.json')
        $taskActual = @($taskArchive.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
        $taskUnexpected = @(Compare-Object ($taskExpected | Sort-Object) ($taskActual | Sort-Object))
        if ($taskUnexpected.Count -ne 0) { throw 'ZIP entry verification failed.' }
    } finally { $taskArchive.Dispose() }
    $taskHash = (Get-FileHash -LiteralPath $taskZip -Algorithm SHA256).Hash.ToLowerInvariant()
    foreach ($taskPair in @(
        @{ Path = $taskSidecar; Text = ($taskHash + '  ' + [IO.Path]::GetFileName($taskZip) + "`r`n") },
        @{ Path = $taskManifestOut; Text = $taskManifest }
    )) {
        $taskStream = [IO.FileStream]::new($taskPair.Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $taskBytes = $taskUtf8.GetBytes($taskPair.Text); $taskStream.Write($taskBytes, 0, $taskBytes.Length); $taskStream.Flush() }
        finally { $taskStream.Dispose() }
    }
    Write-Output ('Created: ' + [IO.Path]::GetFileName($taskZip))
    Write-Output ('SHA256: ' + $taskHash)
} finally {
    # Delete ONLY this invocation's fresh GUID staging directory. Re-resolve and
    # check containment/name/reparse flags immediately before recursive deletion.
    if (Test-Path -LiteralPath $taskStageCanonical) {
        $taskCleanup = (Resolve-Path -LiteralPath $taskStageCanonical).ProviderPath
        $taskItem = Get-Item -LiteralPath $taskCleanup
        if (-not $taskCleanup.StartsWith($taskOutPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            [IO.Path]::GetFileName($taskCleanup) -ne $taskStageName -or
            ($taskItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Staging cleanup safety check failed; the directory was left in place.'
        }
        Remove-Item -LiteralPath $taskCleanup -Recurse -Force
    }
}
