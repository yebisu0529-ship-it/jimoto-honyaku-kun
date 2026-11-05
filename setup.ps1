param(
    [ValidateSet('realtime','balanced','quality')][string]$Policy = 'realtime',
    [string]$ModelId = '',
    [string]$InstallRoot = '',
    [switch]$PlanOnly,
    [switch]$SkipAsr,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$taskAppRoot = $PSScriptRoot
Set-Location -LiteralPath $taskAppRoot
$taskHelper = Join-Path $taskAppRoot 'jimoto-setup.exe'
if (-not (Test-Path -LiteralPath $taskHelper)) { $taskHelper = Join-Path $taskAppRoot 'target/release/jimoto-setup.exe' }
if (-not (Test-Path -LiteralPath $taskHelper)) {
    if ($SkipBuild) { throw '初期設定ツールがありません。配布ZIPを展開し直してください。' }
    & cargo build --release --locked --bin jimoto-setup
    if ($LASTEXITCODE -ne 0) { throw '初期設定ツールのビルドに失敗しました。' }
}
if (-not $InstallRoot) { $InstallRoot = Join-Path $env:LOCALAPPDATA 'LocalTools/PlamoTranslate/data/engine' }
$taskArgs = @('--root', $InstallRoot, '--policy', $Policy)
if ($ModelId) { $taskArgs += @('--model', $ModelId) }
$taskOldOutputEncoding = $OutputEncoding
$taskOldConsoleEncoding = [Console]::OutputEncoding
try {
    $OutputEncoding = New-Object Text.UTF8Encoding($false)
    [Console]::OutputEncoding = $OutputEncoding
    if ($PlanOnly) {
        & $taskHelper @taskArgs --probe
        if ($LASTEXITCODE -ne 0) { throw 'PCの確認に失敗しました。' }
        return
    }
    $taskProbe = & $taskHelper @taskArgs --probe | Out-String | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'PCの確認に失敗しました。' }
    # Native progress is JSON on stderr; the native exit status remains authoritative.
    $ErrorActionPreference = 'Continue'
    & $taskHelper @taskArgs --install
    $taskInstallExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($taskInstallExit -ne 0) { throw 'モデルの準備に失敗しました。途中の取得データは残っています。' }
    if (-not $SkipAsr) {
        $taskAudioRoot = Join-Path $env:LOCALAPPDATA 'LocalTools/PlamoTranslate/data'
        & (Join-Path $taskAppRoot 'scripts/setup-audio.ps1') -InstallRoot $taskAudioRoot -Backend $taskProbe.recommendation.asr_backend -Device $taskProbe.recommendation.asr_device
        if ($LASTEXITCODE -ne 0) { throw '音声認識の準備に失敗しました。翻訳モデルは保持しています。' }
    }
    Write-Host '準備が完了しました。地元翻訳くん.cmdで起動できます。'
} finally {
    $OutputEncoding = $taskOldOutputEncoding
    [Console]::OutputEncoding = $taskOldConsoleEncoding
}
