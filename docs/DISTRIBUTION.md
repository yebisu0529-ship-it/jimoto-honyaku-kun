# 地元翻訳くん0.4.0のWindows配布版

Windows 10/11 x64向けのポータブル版です。ZIPを展開し、「地元翻訳くん.cmd」を開きます。RustやシステムPythonの事前インストールは不要です。起動補助にはWindows標準のWindows PowerShell 5.1を使います。

翻訳用のllama.cppには[Microsoft Visual C++ v14 再頒布可能パッケージ（x64）](https://learn.microsoft.com/ja-jp/cpp/windows/latest-supported-vc-redist?view=msvc-170)が必要です。未導入のPCでは公式のx64版をインストールしてください。なお、アプリ本体と初期設定ツールにはVisual C++ランタイムが静的に組み込まれています。

## 初回の準備

アプリ内でRAM・GPUの空き容量と保存先ディスクを確認し、翻訳モデルと実行方式を選びます。検出結果はこのPC内で処理します。14種類の公式GGUFからの推奨は容量と応答速度を考えた目安で、全モデルを実機比較した最適値ではありません。既存のモデルを利用者の選択なしに変更しません。

翻訳モデルと固定版llama.cppランタイムは、選択後に取得します。ダウンロードは中止でき、対応する未完了ファイルを残した場合は同じ構成で再開できます。サイズ・SHA-256の検証を通るまで完成モデルとして使いません。必要な取得は初回準備の画面から開始します。

NVIDIAの翻訳用CUDA12.4はWindowsドライバー551.61以上、Qwen音声用PyTorch CUDA12.8は570.65以上を別に確認します。AMD/Intelなどは実際のVulkanランタイムを確認できた場合に候補となります。GPUを使う条件を確認できない場合はCPUを候補にし、共有GPUメモリーは専用VRAMへ加算しません。

## 音声認識は任意

テキスト・画面OCR・YouTubeの表示字幕の翻訳に音声認識環境は不要です。PCで再生中の音声も翻訳する場合に、アプリから追加します。英語専用のNemotron Speech Streaming English 0.6Bは、NVIDIA公式GGUFとNeMo-Speech.cpp 0.3.0を使い、Pythonなしで動作します。CPUまたはVulkanを選べ、追加準備には約2GBの空きを見込みます。

多言語用にはQwen3-ASR CUDAまたはWhisper base CPUも選べます。uv・管理Python・venv・モデル・キャッシュはアプリ専用領域へ用意します。初回準備の空き容量の目安は、Qwen CUDAが14GiB、Qwen CPUが8GiB、Whisper CPUが4GiBです。翻訳用の容量とは別に確認し、再開時は検証できたキャッシュを考慮して必要な残容量を減らします。

音声認識・翻訳はPC内で実行します。初回のソフトウェアや重みの取得にはネット接続を使いますが、字幕・音声・訳文を外部ASRやクラウド推論サービスへ送信する構成ではありません。

## 日本語の訳文を読み上げる

「読み上げを準備」からspeech.cpp 0.10.1と彩りTTSの小型モデル、Irodori-TTS v4.1 Small-MFを追加します。初回取得は合計約1.95GB、展開用を含む空き容量の目安は約4.4GiBです。TTS専用のPythonは不要です。準備だけでは読み上げや音声取得は始まりません。

日本語の訳文にある「読み上げ」で手動再生できます。「声を試す」で確認し、「読み上げを停止」で止められます。「日本語の訳文を自動で読み上げる」は初期状態でオフです。読み上げ中は再生音声の取得を一時停止し、自分の読み上げが再び翻訳へ流れることを防ぎます。合成・待機中の音声取得は続きます。

基準速度とライブ時の自動加速を設定できます。遅れが大きいときは上限の範囲で速め、遅れが減ると基準速度へ戻します。生成音声はメモリーで扱い、音声ファイルとして保存しません。

読み上げ用サーバーを起動するときに空きVRAMを再確認し、余裕を確認できない場合は今回の実行だけCPUへ切り替えます。保存済みの設定は変更しません。本配布の公式speech.cpp Windowsランタイムは、CPU実行でもシステムのVulkanローダーが必要です。

## YouTube連携と保存先

同梱の[拡張機能の手順](../extension/README.md)に従い、拡張機能を手動で読み込みます。アプリの「設定と接続」で接続キーをコピーし、拡張機能へ保存して接続確認を行います。アプリの「YouTube字幕を使う」をオンにし、YouTube側で英語CCなどの字幕を表示します。表示中の字幕が対象です。

設定・翻訳履歴・取得済みランタイム・モデルなどの既定の保存先はLocal AppData配下です。実際の場所はアプリ内で確認できます。開発者固有のドライブやユーザーフォルダーを指定する必要はありません。ZIPの展開先を削除しても、Local AppDataの履歴やモデルは残る場合があります。

## ライセンス

アプリ本体は同梱[LICENSE](../LICENSE)のMIT Licenseです。モデル・外部ランタイムには別の利用条件があります。配布前・利用前には、以下の公式本文とモデルカードを確認してください。

- [PLaMo Community License公式本文](https://plamo.preferredai.jp/info/plamo-community-license-en)：PLaMo翻訳モデルの条件。
- PFN公式GGUF：[0.8B](https://huggingface.co/pfnet/plamo-3-translate-0.8b-GGUF)、[2B](https://huggingface.co/pfnet/plamo-3-translate-2b-GGUF)、[8B](https://huggingface.co/pfnet/plamo-3-translate-8b-GGUF)、[31B](https://huggingface.co/pfnet/plamo-3-translate-31b-GGUF)。
- [speech.cppの固定版MIT License](https://github.com/nyosegawa/speech.cpp/blob/03555100287c118b41d1b475044e5ab1e07baa8c/LICENSE)：読み上げランタイムの条件。
- [Irodori-TTS固定モデルカード](https://huggingface.co/sakasegawa/Irodori-TTS-v4.1-Small-MF-GGUF/blob/9db7da1bec3d1069ef8e29732e0497be60c7b047/README.md)：読み上げモデルと記載の利用条件。[モデルのMIT通知](https://huggingface.co/sakasegawa/Irodori-TTS-v4.1-Small-MF-GGUF/blob/9db7da1bec3d1069ef8e29732e0497be60c7b047/LICENSE-MIT)と[codec由来のApache-2.0通知](https://huggingface.co/sakasegawa/Irodori-TTS-v4.1-Small-MF-GGUF/blob/9db7da1bec3d1069ef8e29732e0497be60c7b047/LICENSE-APACHE-2.0)を保持します。
- その他の配布元：[llama.cpp](https://github.com/ggml-org/llama.cpp)、[Qwen3-ASR](https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf)、[Whisper base](https://huggingface.co/Systran/faster-whisper-base)。
- [Nemotron Englishモデル](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b)は[NVIDIA Open Model License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/)、[NVIDIA NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp)はApache-2.0です。取得したランタイムの第三者通知も保持します。彩りTTS用speech.cppとは別の環境です。

フォントはNoto Sans JPを本体に埋め込み、SIL Open Font Licenseの通知をassets/fonts/OFL.txtへ同梱します。翻訳・音声認識・読み上げのモデル重みは、いずれもZIPに同梱せず必要に応じて取得します。

## 配布担当者向けの作成手順

検証済みのtarget/x86_64-pc-windows-msvc/release/plamo-translate.exeとtarget/x86_64-pc-windows-msvc/release/jimoto-setup.exeを用意します。作成スクリプトはビルド・ダウンロード・公開を行いません。

    # 必須ファイルと起動構成を確認するだけ
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -PreflightOnly

    # ローカルZIPを作成する
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -Version 0.4.0

出力はdist/Jimoto-Honyaku-kun-0.4.0-win-x64.zip、SHA-256の.zip.sha256、内容一覧の.manifest.jsonです。出力先は-OutDirで指定できます。同名の既存出力は上書きせず、バージョンまたは出力先を変更する形です。

収録はファイル単位の許可リストです。GUI・セットアップCLI・起動スクリプト・任意ASR用のスクリプトと固定メタデータ・ブラウザー拡張・README・LICENSE・配布資料を入れます。モデル重み、取得済みruntime、venv、検証素材、利用者の設定・履歴・接続キー・ログ、Git情報、一時ファイルは入りません。パッケージmanifestには相対パスとハッシュを記録し、機器名・ユーザー名・保存先の絶対パスは記録しません。

出力先の新しいGUIDステージへコピーし、場所と名前を確認してそのステージだけを後処理で削除します。利用者のファイルは移動しません。ZIPの全エントリーは許可リストと照合します。確認済みの範囲と未検証項目は[検証範囲](VALIDATION.md)を参照してください。GitHubへの公開・リリース作成は、このスクリプトでは行いません。
