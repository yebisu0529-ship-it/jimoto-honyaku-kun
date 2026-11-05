$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot 'setup-audio.ps1'
$tokens = $null
$errors = $null
[void][Management.Automation.Language.Parser]::ParseFile($script, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$metadata = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'audio-runtime.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ($metadata.uv.sha256 -notmatch '^[a-f0-9]{64}$' -or $metadata.uv.bytes -ne 25710044) { throw 'uv pin is missing' }
$root = Join-Path ([IO.Path]::GetTempPath()) ('plamo-audio-plan-' + [Guid]::NewGuid().ToString('N'))
foreach ($spec in @(@('qwen','cpu'), @('qwen','cuda'), @('whisper','cpu'))) {
    $plan = & $script -InstallRoot $root -Backend $spec[0] -Device $spec[1] -Plan | ConvertFrom-Json
    if (-not $plan.isolated -or $plan.downloads_started -or $plan.backend -ne $spec[0] -or $plan.device -ne $spec[1]) { throw 'Unsafe plan' }
    if ($plan.model_revision -notmatch '^[a-f0-9]{40}$') { throw 'Model is not pinned' }
    if ($spec[0] -eq 'whisper' -and $plan.torch) { throw 'Whisper must not install torch' }
    if ($spec[1] -eq 'cuda' -and $plan.torch -ne 'torch==2.11.0+cu128') { throw 'CUDA torch pin is missing' }
}
if (Test-Path -LiteralPath $root) { throw 'Plan mode created files' }
$blocked = $false
try { & $script -InstallRoot 'C:\' -Backend qwen -Device cpu -Plan } catch { $blocked = $true }
if (-not $blocked) { throw 'Drive-root install was accepted' }
Write-Host 'PASS: parser + 3 offline plans; no executables or downloads were started.'
