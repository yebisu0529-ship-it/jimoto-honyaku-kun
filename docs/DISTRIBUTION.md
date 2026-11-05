# 地元翻訳くんのWindows配布版

Windows 10/11 x64向けのポータブル版です。ZIPを展開し、「地元翻訳くん.cmd」を開きます。配布版の利用者がRustやシステムPythonを事前にインストールする必要はありません。起動補助にはWindows標準のWindows PowerShell 5.1を使います。

## 初回の準備

初回はアプリ内でPCのRAM・GPUの空き容量と保存先ディスクを確認し、翻訳モデルと実行方式を選びます。容量情報はこのPC内で処理され、送信しません。推奨は容量と応答速度を考えた目安です。14種類の公式GGUFすべてを実機比較して最速・最高品質を保証した結果ではありません。既存のモデルを利用者の選択なしに大きなモデルへ変更しません。

モデルとllama.cppランタイムは選択後にインターネットから取得します。ZIPには重みを同梱しません。必要容量と取得先を確認してから開始してください。ダウンロードは中止でき、対応する未完了ファイルを残した場合は同じ構成で再開できます。ハッシュ検証を通るまで完成モデルとして使いません。ダウンロードの開始・再開やモデルの読み込みは、アプリ内の状態表示に従ってください。

GPUは搭載容量だけでなく現在の空きを確認します。NVIDIAの翻訳用CUDA12.4はWindowsドライバー551.61以上を基準にし、Qwen音声用PyTorch CUDA12.8は570.65以上を別に確認します。AMD/Intelなどは実際のVulkanランタイムを確認できた場合に候補となります。対応・空き容量を確認できない場合はCPUを候補にします。共有GPUメモリーを専用VRAMとして数えません。ドライバーやGPU世代によって実行時に使えない場合があるため、アプリの診断表示も確認してください。

## 音声認識は任意

テキスト・画面OCR・YouTubeの表示字幕だけを翻訳する場合、音声認識環境は不要です。PCで再生中の音声も翻訳したいときに追加します。ASR環境の導入は別の操作で明示的に開始します。uvと管理Pythonをアプリ用のローカル領域へ導入し、必要な音声モデルを取得します。既存のシステムPythonを前提にしません。

対応GPUとドライバー・空き容量を確認できればQwen3-ASR CUDA、それ以外は多言語Whisper base CPUを候補にします。音声認識も翻訳もPC内で実行し、字幕・音声・翻訳文を外部ASRやクラウド推論サービスへ送る構成ではありません。初回のソフトウェア・重みの取得にはネット接続を使用します。

YouTube連携は同梱`extension/README.md`に従い、拡張機能を手動で読み込んで接続トークンを保存します。ブラウザーの英語CCなど、表示されている字幕が対象です。トークン・Cookie・ブラウザーのログイン情報はZIPに含めません。

設定・翻訳履歴・取得済みランタイム・モデルなどはアプリが選んだ保存先を使います。既定は利用者のLocal AppData配下です。開発者固有のドライブやユーザーフォルダーを指定する必要はありません。実際の場所はアプリ内の保存先表示を確認してください。ZIPの展開先を削除してもLocal AppDataの履歴やモデルが残る場合があります。

## ライセンス

アプリ本体のコードは同梱`LICENSE`のMIT Licenseです。モデルや外部ランタイムにはそれぞれ別の利用条件があります。配布前・利用前にモデルの公式条件を確認してください。商用利用の適否をこの文書では判断していません。

- [PLaMo Community License公式本文](https://plamo.preferredai.jp/info/plamo-community-license-en)
- [PFN公式0.8B GGUF](https://huggingface.co/pfnet/plamo-3-translate-0.8b-GGUF)
- [PFN公式2B GGUF](https://huggingface.co/pfnet/plamo-3-translate-2b-GGUF)
- [PFN公式8B GGUF](https://huggingface.co/pfnet/plamo-3-translate-8b-GGUF)
- [PFN公式31B GGUF](https://huggingface.co/pfnet/plamo-3-translate-31b-GGUF)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)、[Qwen3-ASR](https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf)、[Whisper base配布元](https://huggingface.co/Systran/faster-whisper-base)

フォントはNoto Sans JPを本体に埋め込み、SIL Open Font Licenseの通知を`assets/fonts/OFL.txt`に同梱します。

## 配布担当者向けの作成手順

事前に検証済みの`target/release/plamo-translate.exe`と`target/release/jimoto-setup.exe`を用意します。このパッケージ作成スクリプトはビルド・ダウンロード・公開を行いません。

```powershell
# 必須ファイルとポータブル起動構成を確認するだけ
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -PreflightOnly

# 確認用のローカルZIPを作成する
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -Version 0.2.1
```

既定の出力は`dist/Jimoto-Honyaku-kun-0.2.1-win-x64.zip`、SHA-256の`.zip.sha256`、内容一覧の`.manifest.json`です。出力先は`-OutDir`で指定できます。同名の既存出力は上書きせずエラーにします。バージョンまたは出力先を変更してください。

収録はファイル単位の許可リストです。GUI/セットアップCLI、起動スクリプト、任意ASR導入のスクリプトと固定メタデータ、ブラウザー拡張、README・LICENSE・本文書を入れます。モデル重み、取得済みruntime、`.venv`、検証素材、利用者の設定・履歴・トークン・ログ、Git情報、一時ファイルは入りません。機器名・ユーザー名・絶対パスもパッケージmanifestへ記録しません。

一時ステージは出力先直下に新しいGUID名で作り、その場所と名前を再確認してから、そのステージだけを後処理で削除します。利用者のファイルを移動しません。ZIP作成後に全エントリーを許可リストと照合します。スクリプトの実行成功だけで動作品質は保証されないため、配布前には新しいWindows利用環境で起動・初回導入・中止再開・CPU/GPU切替・任意ASR・終了後の子プロセスを確認してください。

GitHub公開・アップロード・リリース作成は別の作業です。このスクリプトでは実施しません。
