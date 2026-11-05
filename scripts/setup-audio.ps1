param(
    [Parameter(Mandatory = $true)][string]$InstallRoot,
    [ValidateSet('qwen', 'whisper')][string]$Backend = 'qwen',
    [ValidateSet('cpu', 'cuda')][string]$Device = 'cpu',
    [switch]$Plan
)
# Windows PowerShell 5.1. Invoked only after the user chooses audio setup.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

function Assert-LocalRoot([string]$Path) {
    if ($Path -notmatch '^[A-Za-z]:[\\/]' -or ($Path -split '[\\/]' | Where-Object { $_ -eq '..' })) {
        throw '保存先には、このPCの絶対パスを指定してください。'
    }
    $resolved = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    if ($resolved.Length -le 2) { throw 'ドライブのルートではなく、アプリ専用フォルダーを指定してください。' }
    return $resolved
}
function Assert-NoReparse([string]$Path) {
    $cursor = $Path
    while ($cursor) {
        if ([IO.File]::Exists($cursor) -or [IO.Directory]::Exists($cursor)) {
            if (([IO.File]::GetAttributes($cursor) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "リンクまたはジャンクションの保存先は使用できません: $cursor"
            }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}
function Ensure-Directory([string]$Path) {
    Assert-NoReparse $Path
    [void][IO.Directory]::CreateDirectory($Path)
    Assert-NoReparse $Path
}
function Resolve-OwnedCacheFile([string]$Path, [string]$CacheRoot) {
    # HF Hub 1.33 uses snapshot -> repo blob -> shared hub blob symlinks.
    # Follow only file symlinks whose every hop stays in our own HF cache.
    $prefix = [IO.Path]::GetFullPath($CacheRoot).TrimEnd('\', '/') + '\'
    $cursor = [IO.Path]::GetFullPath($Path)
    for ($hop = 0; $hop -lt 8; $hop++) {
        if (-not $cursor.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw '音声モデルのキャッシュがアプリ専用領域の外を参照しています。'
        }
        if (-not [IO.File]::Exists($cursor)) { return $null }
        $item = Get-Item -LiteralPath $cursor -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
            Assert-NoReparse $cursor
            return $cursor
        }
        if ($item.LinkType -ne 'SymbolicLink' -or @($item.Target).Count -ne 1) { return $null }
        $target = [string]@($item.Target)[0]
        if ([IO.Path]::IsPathRooted($target)) { $cursor = [IO.Path]::GetFullPath($target) }
        else { $cursor = [IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetDirectoryName($cursor)) $target)) }
    }
    return $null
}
function Invoke-Checked([string]$Executable, [string[]]$Arguments) {
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "準備コマンドが失敗しました（終了コード $LASTEXITCODE）。キャッシュは保持されています。" }
}
function Assert-OfficialDownload([Uri]$Uri) {
    $hosts = @('github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com', 'github-releases.githubusercontent.com')
    if ($Uri.Scheme -ne 'https' -or $Uri.Port -ne 443 -or $Uri.UserInfo -or $Uri.Fragment -or $hosts -notcontains $Uri.DnsSafeHost) {
        throw '公式配布先以外への接続は許可されていません。'
    }
}
function Download-Uv($Metadata, [string]$Target) {
    Assert-NoReparse $Target
    if ([IO.File]::Exists($Target)) {
        if ((Get-Item -LiteralPath $Target).Length -ne $Metadata.bytes -or
            (Get-FileHash -LiteralPath $Target -Algorithm SHA256).Hash -ne $Metadata.sha256) {
            throw '保存済みuvの検証に失敗しました。既存ファイルは変更していません。'
        }
        return
    }
    Add-Type -AssemblyName System.Net.Http
    $handler = New-Object Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect = $false
    $handler.UseProxy = $false
    $client = New-Object Net.Http.HttpClient($handler)
    $client.Timeout = [TimeSpan]::FromMinutes(10)
    $response = $null
    try {
        $uri = [Uri]$Metadata.url
        for ($redirect = 0; $redirect -le 10; $redirect++) {
            Assert-OfficialDownload $uri
            $response = $client.GetAsync($uri, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
                $location = $response.Headers.Location
                if (-not $location) { throw 'uv取得先のリダイレクトが不正です。' }
                $uri = New-Object Uri($uri, $location)
                $response.Dispose()
                $response = $null
                continue
            }
            break
        }
        if (-not $response -or -not $response.IsSuccessStatusCode) { throw 'uvの取得に失敗しました。再実行してください。' }
        if ($response.Content.Headers.ContentLength -and $response.Content.Headers.ContentLength -ne $Metadata.bytes) {
            throw 'uvの配布サイズが固定情報と一致しません。'
        }
        $partial = "$Target.partial"
        Assert-NoReparse $partial
        $output = [IO.File]::Open($partial, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
        try {
            $buffer = New-Object byte[] 131072
            [long]$total = 0
            while (($count = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                $total += $count
                if ($total -gt $Metadata.bytes) { throw 'uvの取得サイズが上限を超えました。' }
                $output.Write($buffer, 0, $count)
            }
            if ($total -ne $Metadata.bytes) { throw 'uvの取得が途中で終了しました。再実行してください。' }
            $output.Flush()
        } finally { $output.Dispose(); $inputStream.Dispose() }
        if ((Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ne $Metadata.sha256) { throw 'uvのSHA256が一致しません。実行しません。' }
        [IO.File]::Move($partial, $Target)
    } finally {
        if ($response) { $response.Dispose() }
        $client.Dispose()
        $handler.Dispose()
    }
}
function Expand-UvSafely([string]$Archive, [string]$Destination) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    Assert-NoReparse $Destination
    $reuse = [IO.Directory]::Exists($Destination)
    $stage = $Destination
    if (-not $reuse) {
        $stage = "$Destination.staging-$([Guid]::NewGuid().ToString('N'))"
        Ensure-Directory $stage
    }
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        if ($zip.Entries.Count -gt 64) { throw 'uvのZIPの項目数が不正です。' }
        [long]$total = 0
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            $segments = $name.TrimEnd('/').Split('/')
            if (-not $name -or $name.StartsWith('/') -or $name.Length -gt 240 -or
                ($segments | Where-Object { -not $_ -or $_ -eq '..' -or $_ -eq '.' -or $_.Contains(':') -or $_ -match '[. ]$|[\x00-\x1f]' -or $_ -match '^(?i:CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])(?:\.|$)' })) {
                throw 'uvのZIPに危険なパスがあります。'
            }
            $mode = ($entry.ExternalAttributes -shr 16) -band 0xF000
            if ($mode -ne 0 -and $mode -ne 0x8000 -and $mode -ne 0x4000) { throw 'ZIP内のリンクや特殊ファイルは展開できません。' }
            $path = [IO.Path]::GetFullPath([IO.Path]::Combine($stage, $name.Replace('/', '\')))
            if (-not $path.StartsWith($stage + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'ZIPの展開先が不正です。' }
            if ($name.EndsWith('/')) { Ensure-Directory $path; continue }
            $total += $entry.Length
            if ($total -gt 268435456) { throw 'uvのZIPの展開サイズが上限を超えました。' }
            Ensure-Directory ([IO.Path]::GetDirectoryName($path))
            $source = $entry.Open()
            if ($reuse) {
                Assert-NoReparse $path
                $hasher = [Security.Cryptography.SHA256]::Create()
                try { $expected = [BitConverter]::ToString($hasher.ComputeHash($source)).Replace('-', '') }
                finally { $source.Dispose(); $hasher.Dispose() }
                if (-not [IO.File]::Exists($path) -or (Get-Item -LiteralPath $path).Length -ne $entry.Length -or
                    (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $expected) {
                    throw '保存済みuvの実行ファイルが検証済みZIPと一致しません。既存ファイルは変更していません。'
                }
                continue
            }
            $target = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            try {
                $buffer = New-Object byte[] 131072
                [long]$copied = 0
                while (($count = $source.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $copied += $count
                    if ($copied -gt $entry.Length) { throw 'ZIP内のファイルサイズが不正です。' }
                    $target.Write($buffer, 0, $count)
                }
                if ($copied -ne $entry.Length) { throw 'ZIPの展開が途中で終了しました。' }
            } finally { $source.Dispose(); $target.Dispose() }
        }
    } finally { $zip.Dispose() }
    if (-not [IO.File]::Exists((Join-Path $stage 'uv.exe'))) { throw 'ZIPにuv.exeがありません。' }
    if (-not $reuse) { [IO.Directory]::Move($stage, $Destination) }
}

$root = Assert-LocalRoot $InstallRoot
Assert-NoReparse $root
$metadata = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'audio-runtime.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ($metadata.version -ne 1 -or $metadata.qwen.revision -notmatch '^[a-f0-9]{40}$' -or $metadata.whisper.revision -notmatch '^[a-f0-9]{40}$') { throw '音声環境の固定情報が不正です。' }
$audio = Join-Path $root 'audio'
$venv = Join-Path $audio '.venv'
$python = Join-Path $venv 'Scripts/python.exe'
$modelMetadata = $metadata.$Backend
if ($Backend -eq 'whisper' -and $Device -ne 'cpu') { throw 'Whisperの配布設定はCPU専用です。CPUを選択してください。' }
$initialPeak = $(if ($Backend -eq 'whisper') { 4GB } elseif ($Device -eq 'cuda') { 14GB } else { 8GB })
$uvArchive = Join-Path (Join-Path $audio 'tools') $metadata.uv.filename
$resuming = [IO.File]::Exists($python) -and [IO.File]::Exists($uvArchive)
if ($resuming) {
    Assert-NoReparse $python
    Assert-NoReparse $uvArchive
    $resuming = (Get-Item -LiteralPath $uvArchive).Length -eq $metadata.uv.bytes -and
        (Get-FileHash -LiteralPath $uvArchive -Algorithm SHA256).Hash -eq $metadata.uv.sha256
}
$missingModel = [long]$modelMetadata.model_bytes
$hubModelRoot = Join-Path (Join-Path $audio 'huggingface/hub') ('models--' + $modelMetadata.repo.Replace('/', '--'))
$cachedWeight = Join-Path (Join-Path $hubModelRoot ('snapshots/' + $modelMetadata.revision)) $modelMetadata.model_file
$cachedBlob = Join-Path (Join-Path $hubModelRoot 'blobs') $modelMetadata.model_sha256
foreach ($candidate in @($cachedWeight, $cachedBlob)) {
    $ownedWeight = Resolve-OwnedCacheFile $candidate (Join-Path $audio 'huggingface')
    if ($ownedWeight) {
        if ((Get-Item -LiteralPath $ownedWeight).Length -eq $modelMetadata.model_bytes -and
            (Get-FileHash -LiteralPath $ownedWeight -Algorithm SHA256).Hash -eq $modelMetadata.model_sha256) {
            $missingModel = 0
            break
        }
    }
}
# Initial peak includes copied package cache + venv. A resumed preparation does
# not need that entire peak again: retain a repair reserve plus two copies of an
# unfinished weight (HF blob + Windows snapshot). Existing data is never removed.
$requiredFree = [long]$initialPeak
if ($resuming) {
    $repairReserve = $(if ($Backend -eq 'whisper') { 1GB } elseif ($Device -eq 'cuda') { 4GB } else { 2GB })
    $requiredFree = [long]$repairReserve + 2 * $missingModel
}
if ($Plan) {
    [ordered]@{version=1; backend=$Backend; device=$Device; python_path=$python; model_repo=$modelMetadata.repo;
        model_revision=$modelMetadata.revision; uv_version=$metadata.uv.version; isolated=$true; downloads_started=$false;
        torch= $(if ($Backend -eq 'qwen') { $metadata.qwen.('torch_' + $Device) } else { $null });
        manifest_path=(Join-Path $root 'audio-installation.json'); required_free_bytes=$requiredFree;
        initial_peak_bytes=[long]$initialPeak; resuming=$resuming; missing_model_bytes=$missingModel;
        disk_requirement_is_estimate=$true} | ConvertTo-Json -Depth 5
    return
}
$drive = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($root))
if (-not $drive.IsReady) { throw '音声認識の保存先の空き容量を確認できませんでした。' }
$availableFree = $drive.AvailableFreeSpace
if ($availableFree -lt $requiredFree) {
    throw ('音声認識の準備には空き容量が約{0:N1} GiB必要です（現在{1:N1} GiB）。保存先の空きを確認してください。' -f ($requiredFree / 1GB), ($availableFree / 1GB))
}

$savedEnvironment = @{}
foreach ($name in @('UV_PYTHON_INSTALL_DIR', 'UV_PYTHON_BIN_DIR', 'UV_CACHE_DIR', 'UV_NO_PROGRESS', 'UV_LINK_MODE', 'HF_HOME', 'HF_HUB_CACHE', 'HUGGINGFACE_HUB_CACHE', 'TRANSFORMERS_CACHE', 'HF_HUB_OFFLINE', 'HF_HUB_DISABLE_TELEMETRY', 'HF_HUB_DISABLE_IMPLICIT_TOKEN')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    Write-Host '音声認識の準備を始めます。初回は数GBの取得が必要な場合があります。'
    foreach ($path in @($root, $audio, (Join-Path $audio 'tools'), (Join-Path $audio 'cache'), (Join-Path $audio 'python'))) { Ensure-Directory $path }
    $env:UV_PYTHON_INSTALL_DIR = Join-Path $audio 'python'
    $env:UV_PYTHON_BIN_DIR = Join-Path $audio 'bin'
    Ensure-Directory $env:UV_PYTHON_BIN_DIR
    $env:UV_CACHE_DIR = Join-Path $audio 'cache'
    $env:UV_NO_PROGRESS = '1'
    $env:UV_LINK_MODE = 'copy'
    $env:HF_HOME = Join-Path $audio 'huggingface'
    $env:HF_HUB_CACHE = Join-Path $env:HF_HOME 'hub'
    $env:HUGGINGFACE_HUB_CACHE = $env:HF_HUB_CACHE
    $env:TRANSFORMERS_CACHE = $null
    $env:HF_HUB_OFFLINE = $null
    $env:HF_HUB_DISABLE_TELEMETRY = '1'
    $env:HF_HUB_DISABLE_IMPLICIT_TOKEN = '1'
    # Always verify and use our own pinned uv. A matching uv on the user's PATH
    # is not evidence that its executable belongs to this isolated installation.
    Write-Host '固定版のPython準備ツールを取得・検証しています。'
    $archive = Join-Path (Join-Path $audio 'tools') $metadata.uv.filename
    Download-Uv $metadata.uv $archive
    $toolDirectory = Join-Path (Join-Path $audio 'tools') ('uv-' + $metadata.uv.version)
    Expand-UvSafely $archive $toolDirectory
    $uv = Join-Path $toolDirectory 'uv.exe'
    Invoke-Checked $uv @('--version')
    Write-Host 'このアプリ専用のPython 3.13を準備しています。'
    Invoke-Checked $uv @('python', 'install', $metadata.python)
    if (-not [IO.File]::Exists($python)) {
        if ([IO.Directory]::Exists($venv)) { throw '既存の音声用環境にPythonがありません。既存ファイルは変更していません。' }
        Invoke-Checked $uv @('venv', '--managed-python', '--python', $metadata.python, $venv)
    }
    Assert-NoReparse $python
    $probe = @'
import os,sys,site
assert sys.version_info[:2] == (3,13), 'Python 3.13 is required'
assert not site.ENABLE_USER_SITE, 'User packages must be disabled'
assert os.path.normcase(os.path.abspath(sys.prefix)) == os.path.normcase(os.path.abspath(sys.argv[1])), 'Wrong venv'
base = os.path.normcase(os.path.abspath(sys.base_prefix))
managed = os.path.normcase(os.path.abspath(sys.argv[2])) + os.sep
assert base.startswith(managed), 'Python must be managed inside this app'
with open(os.path.join(sys.prefix,'pyvenv.cfg'),encoding='utf-8') as configuration:
    assert not any('include-system-site-packages = true' in x.lower() for x in configuration), 'System packages must be disabled'
'@
    Invoke-Checked $python @('-I', '-c', $probe, $venv, $env:UV_PYTHON_INSTALL_DIR)
    if ($Backend -eq 'qwen') {
        Write-Host ('PyTorchを準備しています（' + $Device + '）。')
        $torchPackage = $metadata.qwen.('torch_' + $Device)
        $torchIndex = $metadata.qwen.($Device + '_index')
        Invoke-Checked $uv @('pip', 'install', '--python', $python, '--index-url', $torchIndex, $torchPackage)
        Write-Host 'Qwen3-ASRの依存パッケージを準備しています。'
        Invoke-Checked $uv @('pip', 'install', '--python', $python, '-r', (Join-Path $PSScriptRoot 'requirements.txt'))
    } else {
        Write-Host 'Whisper CPUの依存パッケージを準備しています。'
        $arguments = @('pip', 'install', '--python', $python) + @($metadata.whisper.packages)
        Invoke-Checked $uv $arguments
    }
    Write-Host '固定版の音声モデルを取得し、依存パッケージとキャッシュを確認しています。'
    $resultPath = Join-Path $audio ('model-result-' + [Guid]::NewGuid().ToString('N') + '.json')
    $prepare = @'
import json,os,sys
from huggingface_hub import snapshot_download
backend,device,repo,revision,result_path = sys.argv[1:]
if backend == 'qwen':
    import torch
    from transformers import AutoProcessor,AutoModelForMultimodalLM
    assert torch.__version__.startswith('2.11.0'), 'Unexpected torch version'
    if device == 'cuda':
        assert torch.cuda.is_available(), 'CUDA GPU is unavailable; select CPU explicitly'
else:
    import faster_whisper
model = snapshot_download(repo_id=repo,revision=revision,token=False)
assert snapshot_download(repo_id=repo,revision=revision,token=False,local_files_only=True) == model
if backend == 'qwen':
    AutoProcessor.from_pretrained(model,local_files_only=True)
else:
    assert os.path.isfile(os.path.join(model,'model.bin')), 'Whisper weights are missing'
with open(result_path,'x',encoding='utf-8') as handle:
    json.dump({'model_path':os.path.abspath(model)},handle)
'@
    Invoke-Checked $python @('-I', '-c', $prepare, $Backend, $Device, $modelMetadata.repo, $modelMetadata.revision, $resultPath)
    $result = Get-Content -LiteralPath $resultPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $modelPath = [IO.Path]::GetFullPath($result.model_path)
    if (-not $modelPath.StartsWith($env:HF_HOME + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'モデルの保存先がアプリ専用領域の外です。' }
    Assert-NoReparse $modelPath
    $manifest = [ordered]@{version=1; python_path=$python; backend=$Backend; device=$Device; model_path=$modelPath;
        model_repo=$modelMetadata.repo; model_revision=$modelMetadata.revision}
    $manifestPath = Join-Path $root 'audio-installation.json'
    Assert-NoReparse $manifestPath
    $temporaryManifest = Join-Path $root ('audio-installation-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    [IO.File]::WriteAllText($temporaryManifest, ($manifest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
    if ([IO.File]::Exists($manifestPath)) { [IO.File]::Replace($temporaryManifest, $manifestPath, $null) }
    else { [IO.File]::Move($temporaryManifest, $manifestPath) }
    Write-Host '音声認識の準備が完了しました。音声取得はアプリでオンにしたときだけ開始します。'
} catch {
    Write-Error ('音声環境を確定できませんでした。取得済みキャッシュは保持されています。 ' + $_.Exception.Message)
    exit 1
} finally {
    foreach ($name in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
}
