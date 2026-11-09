# Windows配布パッケージの構成と作成手順

## 利用者向け：配布ZIPの使い方

Windows 10 / 11（x64）向けのポータブル版です。

1. ZIPを任意のフォルダに展開し、`Talk風Live翻訳.cmd` を開きます（RustやシステムPythonの事前導入は不要です）。
2. 未導入のPCでは、翻訳エンジンの実行に必要な [Microsoft Visual C++ v14 再頒布可能パッケージ（x64）](https://learn.microsoft.com/ja-jp/cpp/windows/latest-supported-vc-redist?view=msvc-170) をインストールしてください。
3. 初回起動時の画面で、PCの空き容量に応じた翻訳モデルを選択してダウンロードします。

各機能の詳細や必要な空き容量、対応GPUドライバー、モデルごとのライセンス条件は [詳細仕様・動作要件・ライセンス](SPECIFICATIONS.md) をご覧ください。YouTube字幕連携の手順は [extension/README.md](../extension/README.md) に記載しています。

### データの保存場所

- 設定・翻訳履歴・ダウンロードしたモデルやランタイムは、既定でユーザーの `Local AppData` 配下に保存されます（実際のパスはアプリ内で確認できます）。
- 配布ZIPを展開したフォルダを削除しても、`Local AppData` 内のモデルや履歴は消えません。

---

## 配布担当者向け：パッケージ作成手順

`scripts/package.ps1` はビルド済みのリリースバイナリから配布用ZIPを作成します（スクリプト内でのコンパイル・ダウンロード・外部公開は行いません）。

### 1. リリースバイナリの準備

以下の2ファイルがビルド済みであることを確認します。

- `target/x86_64-pc-windows-msvc/release/plamo-translate.exe`
- `target/x86_64-pc-windows-msvc/release/jimoto-setup.exe`

### 2. ダウンロード先の確認

アプリは、翻訳モデルや音声認識・読み上げ・文字認識の準備で、固定した配布先からファイルを取得します。配布元がファイルを更新すると、古い固定先から取得できなくなることがあります（2026-11-06には、PLaMo 3 Translateの更新で翻訳モデル14件すべてがHTTP 403になりました）。パッケージを作る前に、すべての固定先から取得できることを確かめます。

```powershell
cargo run --release --locked --bin download-check
```

固定したファイルごとに、アプリと同じ接続設定（許可する配布先とリダイレクトの扱い）で先頭の1バイトだけを取得し、配布元が示すサイズが固定サイズと同じかを確かめます。SHA-256はダウンロード時に照合するため、ここでは確かめません。

- すべて取得できると「〇件すべてを取得できます。」と表示され、終了コード0で終わります。
- 取得できないファイルは `NG` の行に配布先と理由が表示され、終了コード1で終わります。その固定先を配布元の現在の版（コミット・サイズ・SHA-256）に更新してから、もう一度確かめてください。
- 音声認識の準備でuvが名前とバージョンから取得するPython本体とパッケージ（PyPI・PyTorch）は対象外です。

### 3. パッケージの検証と作成

```powershell
# 必須ファイルと起動構成の事前チェックのみ
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -PreflightOnly

# 配布用ZIPの作成
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -Version 0.6.0
```

### 4. 出力ファイルと収録ルール

`dist/`（または `-OutDir` で指定した先）に以下の3ファイルを出力します。同名ファイルが既にある場合は上書きせず終了します。

- `Jimoto-Honyaku-kun-0.6.0-win-x64.zip`
- `Jimoto-Honyaku-kun-0.6.0-win-x64.zip.sha256`
- `Jimoto-Honyaku-kun-0.6.0-win-x64.manifest.json`

**収録対象（ホワイトリスト方式）**
- アプリ本体（GUI）・セットアップCLI・起動スクリプト
- 音声認識セットアップ用スクリプト・固定メタデータ
- ブラウザ拡張機能（`extension/`）
- 同梱フォント通知・アイコン・スクリーンショット
- ドキュメント（`README.md`, `LICENSE`, `docs/SPECIFICATIONS.md`, `docs/DISTRIBUTION.md`, `docs/VALIDATION.md`）

**除外対象（ZIPに含めないもの）**
- モデル重み、ダウンロード済みランタイム、Python仮想環境（`.venv`）、固定声GGUF、参照WAV
- ユーザー設定、翻訳履歴、接続トークン、ログ、検証用データ、Gitメタデータ
- `package-manifest.json` には相対パスとSHA-256のみを記録し、PC名・ユーザー名・絶対パスは含めません。
