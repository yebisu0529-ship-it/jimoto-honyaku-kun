param([Parameter(Mandatory=$true)][string]$InstallRoot, [switch]$Plan)
$ErrorActionPreference='Stop'
Set-StrictMode -Version 2

function Assert-Safe([string]$Path) {
    $cursor=[IO.Path]::GetFullPath($Path)
    while($cursor) {
        if(([IO.File]::Exists($cursor) -or [IO.Directory]::Exists($cursor)) -and (([IO.File]::GetAttributes($cursor) -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {throw 'リンクを含むIndraの保存先は使用できません。'}
        $parent=[IO.Path]::GetDirectoryName($cursor)
        if($parent -eq $cursor){break}; $cursor=$parent
    }
}
function File-Sha256([string]$Path) {
    $stream=[IO.File]::OpenRead($Path)
    $hash=[Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $stream.Dispose(); $hash.Dispose() }
}
function Run([string]$Executable,[string[]]$Arguments) {
    & $Executable @Arguments
    if($LASTEXITCODE -ne 0){throw "Indraの準備コマンドが失敗しました（終了コード $LASTEXITCODE）。取得済みファイルは保持しています。"}
}
if($InstallRoot -notmatch '^[A-Za-z]:[\\/]' -or ($InstallRoot -split '[\\/]' | Where-Object {$_ -eq '..'})){throw 'Indraの保存先には、このPCの絶対パスを指定してください。'}
$root=[IO.Path]::GetFullPath($InstallRoot).TrimEnd('\','/')
if($root.Length -le 3){throw 'Indra専用フォルダーを指定してください。'}
Assert-Safe $root
$metadataPath=Join-Path $root 'metadata.json'
$metadata=Get-Content -LiteralPath $metadataPath -Raw -Encoding UTF8 | ConvertFrom-Json
$python=Join-Path $root '.venv/Scripts/python.exe'
$uv=Join-Path $root ('tools/uv-'+$metadata.uv.version+'/uv.exe')
if($Plan){[ordered]@{version=1;python=$metadata.python.version;python_path=$python;isolated=$true;model_revision=$metadata.model_revision;wheel_count=@($metadata.wheels).Count;downloads_started=$false}|ConvertTo-Json;return}
foreach($directory in @('python','cache','bin')){ $path=Join-Path $root $directory;Assert-Safe $path;[void][IO.Directory]::CreateDirectory($path) }
$env:UV_PYTHON_INSTALL_DIR=Join-Path $root 'python'
$env:UV_PYTHON_BIN_DIR=Join-Path $root 'bin'
$env:UV_CACHE_DIR=Join-Path $root 'cache'
$env:UV_LINK_MODE='copy'
$env:UV_NO_PROGRESS='1'
$env:UV_NO_CONFIG='1'
$env:UV_PYTHON_INSTALL_MIRROR=$null
$env:UV_OFFLINE='1'
$env:PYTHONUTF8='1'
$env:PYTHONNOUSERSITE='1'
$env:HF_HUB_OFFLINE='1'
$env:HF_HUB_DISABLE_IMPLICIT_TOKEN='1'
$archive=Join-Path $root ('downloads/'+$metadata.python.name)
Assert-Safe $archive;Assert-Safe $uv
if((Get-Item -LiteralPath $archive).Length -ne $metadata.python.bytes -or (File-Sha256 $archive) -ne $metadata.python.sha256){throw 'Indra用Pythonの固定ハッシュが一致しません。'}
$url=([Uri]$archive).AbsoluteUri
$entry=[ordered]@{name='cpython';arch=@{family='x86_64';variant=$null};os='windows';libc='none';major=3;minor=13;patch=14;prerelease='';url=$url;sha256=$metadata.python.sha256;variant=$null;build=$metadata.python.build}
$catalog=[ordered]@{'cpython-3.13.14-windows-x86_64-none'=$entry}
$catalogPath=Join-Path $root 'runtime/python-downloads.json'
Assert-Safe $catalogPath
[IO.File]::WriteAllText($catalogPath,($catalog|ConvertTo-Json -Depth 6),(New-Object Text.UTF8Encoding($false)))
Write-Host 'Indra専用のPythonを検証済みファイルから準備しています。'
Run $uv @('python','install',$metadata.python.version,'--python-downloads-json-url',$catalogPath,'--no-bin','--no-registry','--offline','--no-config')
$venv=Join-Path $root '.venv'
Assert-Safe $venv
if(-not [IO.File]::Exists($python)){
    if([IO.Directory]::Exists($venv)){throw '既存のIndra環境にPythonがありません。既存ファイルは変更していません。'}
    Run $uv @('venv','--managed-python','--python',$metadata.python.version,'--offline','--no-config',$venv)
}
Assert-Safe $python
$probe=@'
import os,sys,site
from pathlib import Path
root=Path(sys.argv[1]).resolve()
assert sys.version_info[:3]==(3,13,14)
assert Path(sys.prefix).resolve()==root/'.venv'
assert Path(sys.base_prefix).resolve().is_relative_to(root/'python')
assert not site.ENABLE_USER_SITE
assert 'include-system-site-packages = true' not in (root/'.venv/pyvenv.cfg').read_text(encoding='utf-8').lower()
'@
Run $python @('-I','-X','utf8','-c',$probe,$root)
Write-Host '固定ハッシュ付きのIndra依存パッケージをオフラインで準備しています。'
Run $uv @('pip','install','--python',$python,'--require-hashes','--only-binary',':all:','--no-index','--find-links',(Join-Path $root 'wheels'),'--offline','--no-config','-r',(Join-Path $root 'runtime/indra-requirements.lock'))
Run $uv @('pip','check','--python',$python,'--offline','--no-config')
Write-Host 'Indraの日英モデルとCPU実行環境を確認しています。'
Run $python @('-I','-X','utf8',(Join-Path $root 'runtime/indra_server.py'),'--root',$root,'--check')
Write-Host 'Indraの依存パッケージと日英モデルの確認が完了しました。'
