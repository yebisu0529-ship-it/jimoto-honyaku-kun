param([switch]$WithoutAudioModel)
$ErrorActionPreference = 'Stop'
$taskAppRoot = $PSScriptRoot
$taskExe = Join-Path $taskAppRoot 'plamo-translate.exe'
if (-not (Test-Path -LiteralPath $taskExe)) { $taskExe = Join-Path $taskAppRoot 'target/release/plamo-translate.exe' }
if (-not (Test-Path -LiteralPath $taskExe)) { throw 'アプリ本体がありません。配布ZIPを展開し直すか、ソース版をビルドしてください。' }
$env:PLAMO_APP_DIR = $taskAppRoot
# The Rust app owns model preparation, readiness and only its own subprocesses.
# Audio capture is always off initially and starts only after the user turns it on.
$taskGui = Start-Process -FilePath $taskExe -WorkingDirectory $taskAppRoot -PassThru
$taskGui.WaitForExit()
