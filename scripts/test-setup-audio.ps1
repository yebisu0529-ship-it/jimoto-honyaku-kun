param([string]$PythonExecutable = '')
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
    $expectedPeak = $(if ($spec[0] -eq 'whisper') { 4GB } elseif ($spec[1] -eq 'cuda') { 14GB } else { 8GB })
    if ($plan.initial_peak_bytes -ne $expectedPeak -or $plan.required_free_bytes -ne $expectedPeak -or $plan.resuming) { throw 'Initial disk estimate is incorrect' }
    if (-not $plan.disk_requirement_is_estimate -or $plan.missing_model_bytes -le 0) { throw 'Disk estimate is missing' }
}
if (Test-Path -LiteralPath $root) { throw 'Plan mode created files' }
$blocked = $false
try { & $script -InstallRoot 'C:\' -Backend qwen -Device cpu -Plan } catch { $blocked = $true }
if (-not $blocked) { throw 'Drive-root install was accepted' }
Write-Host 'PASS: parser + 3 offline plans; no install commands or downloads were started.'
if ($PythonExecutable) {
    # Run the actual embedded probe on UTF-8 config in a Unicode directory.
    $source = Get-Content -LiteralPath $script -Raw -Encoding UTF8
    $probeMatch = [regex]::Match($source, '(?s)\$probe = @''\r?\n(.*?)\r?\n''@')
    if (-not $probeMatch.Success) { throw 'Installation probe could not be located.' }
    $regression = @'
import pathlib,sys,tempfile
probe = sys.argv[1]
original = (sys.prefix,sys.base_prefix,sys.argv)
with tempfile.TemporaryDirectory(prefix='jimoto-cfg-日本語-') as temporary:
    root = pathlib.Path(temporary).resolve()
    assert root.parent == pathlib.Path(tempfile.gettempdir()).resolve()
    assert root.name.startswith('jimoto-cfg-')
    venv,managed = root/'利用者環境',root/'管理Python'
    venv.mkdir(); managed.mkdir()
    config = venv/'pyvenv.cfg'
    config.write_text('home = 日本語の管理Python\ninclude-system-site-packages = false\n',encoding='utf-8')
    sys.prefix,sys.base_prefix = str(venv),str(managed/'cpython-3.13')
    sys.argv = ['probe',str(venv),str(managed)]
    try:
        exec(probe,{})
        config.write_text('home = 日本語の管理Python\ninclude-system-site-packages = true\n',encoding='utf-8')
        try:
            exec(probe,{})
        except AssertionError as error:
            assert str(error) == 'System packages must be disabled'
        else:
            raise AssertionError('A venv importing system packages was accepted')
    finally:
        sys.prefix,sys.base_prefix,sys.argv = original
print('PASS: UTF-8 Unicode-path venv probe and rejected system-site packages')
'@
    & $PythonExecutable -I -c $regression $probeMatch.Groups[1].Value
    if ($LASTEXITCODE -ne 0) { throw 'Unicode-path installation probe regression failed.' }
}
