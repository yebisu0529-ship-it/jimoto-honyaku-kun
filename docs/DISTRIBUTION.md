# 地元翻訳くん0.5.0のWindows配布版

Windows 10/11 x64向けのポータブル版です。ZIPを展開し、「地元翻訳くん.cmd」を開きます。RustやシステムPythonの事前インストールは不要です。起動補助にはWindows標準のWindows PowerShell 5.1を使います。

翻訳用のllama.cppには[Microsoft Visual C++ v14 再頒布可能パッケージ（x64）](https://learn.microsoft.com/ja-jp/cpp/windows/latest-supported-vc-redist?view=msvc-170)が必要です。未導入のPCでは公式のx64版をインストールしてください。なお、アプリ本体と初期設定ツールにはVisual C++ランタイムが静的に組み込まれています。

## 初回の準備

アプリ内でRAM・GPUの空き容量と保存先ディスクを確認し、翻訳モデルと実行方式を選びます。検出結果はこのPC内で処理します。14種類の公式GGUFからの推奨は容量と応答速度を考えた目安で、全モデルを実機比較した最適値ではありません。既存のモデルを利用者の選択なしに変更しません。

翻訳モデルと固定版llama.cppランタイムは、選択後に取得します。ダウンロードは中止でき、対応する未完了ファイルを残した場合は同じ構成で再開できます。サイズ・SHA-256の検証を通るまで完成モデルとして使いません。必要な取得は初回準備の画面から開始します。

NVIDIAの翻訳用CUDA12.4はWindowsドライバー551.61以上、Qwen音声用PyTorch CUDA12.8は570.65以上を別に確認します。AMD/Intelなどは実際のVulkanランタイムを確認できた場合に候補となります。GPUを使う条件を確認できない場合はCPUを候補にし、共有GPUメモリーは専用VRAMへ加算しません。

## 通常表示とミニ表示

初期準備は通常表示で行い、準備後に「ミニ表示」またはアプリ内の `Ctrl+Shift+M` で切り替えます。同じアプリウィンドウを使い、文章の入力・履歴・固定声の設定を保ちます。「通常表示」またはミニ表示中の `Esc` で通常表示へ戻れます。

ミニ表示には確定した訳文と上部の処理状況を表示します。ライブ入力の一時停止・再開、任意の最前面表示を使えます。最前面表示は初期状態ではオフです。範囲翻訳の原文と訳文は手動の対訳として保持し、「ライブへ戻る」でライブの対訳へ切り替えます。閉じるボタンはアプリ全体を終了し、アプリが起動したモデル処理も停止します。これらの最新のミニ表示操作は、実GUIでの確認が残っています。

## 字幕を別ウィンドウに表示する

通常画面の「字幕ウィンドウ」、またはミニ表示のメニューから、訳文専用の独立したウィンドウを開けます。ミニ表示と併用でき、字幕ウィンドウだけを閉じても翻訳・保存・読み上げは続きます。最前面表示、原文の併記、文字サイズを切り替えられます。動画の一時停止中や入力停止後も最後の対訳を保持し、新しい動画・シーク・新しい履歴では古い対訳を消します。

## 文字認識の方式を選ぶ

設定の「画面・画像の文字認識」で、Windows OCR・Indra OCR・PaddleOCR-VL-1.6を切り替えます。Windows OCRは導入済みのOS機能を使い、追加モデルは不要です。ほかの方式は「文字認識を準備」で利用条件を確認してから追加します。

Indra OCRはCPU用の軽量な選択肢です。日英9モデル約54MBに、日本語用CJK認識器も含みます。固定版uv・管理Python 3.13.14・Indraを含む23パッケージを合わせた初回取得は約184MBです。専用の環境をアプリの保存先に作るため、システムPythonの導入やPATHの変更は不要です。重み・実行環境を固定ハッシュで検証し、その後の推論ではモデルを自動取得しません。

PaddleOCR-VL-1.6は、公式GGUFと画像用モデルを合わせて約1.82GB取得します。翻訳用のllama.cpp実行環境を再利用し、Pythonは不要です。実行方式は「自動」または「CPU」を選べます。自動ではGPUと空き容量を確認します。展開用を含め約4GBの空きを目安にしてください。認識結果、とくに小さい文字や複雑な配置は原文と照合してください。

Indraの重みはModified OpenRAIL-M、PaddleOCRのモデルはApache-2.0です。Indraの利用条件は後述のライセンス欄も確認してください。

## 文章をつなぐライブ字幕

YouTube字幕と音声認識の原文を段落につなぎ、前の確定段落も参照して訳文を更新します。短い字幕窓や短い間では分割せず、更新中の訳文は上部の固定欄に表示し、確定した文章は履歴の下へ古い順に追加します。動画の一時停止中は未確定の文章を保持します。自動読み上げは文章の確定後に一度行います。中断した文章は未確定として保存し、TXT・JSONに含めます。SRTへの書き出しは確定した訳文だけが対象です。

英語の文章区切り補助は初回の翻訳モデル準備時に選べ、設定からも追加できます。取得は合計約86MBで、展開と取得キャッシュのため512MiB以上の空き容量を確認します。sherpa-onnx 2.0.1とCT-Transformer INT8をCPUで実行し、Microsoft Visual C++ v14 x64ランタイムが必要です。Pythonの追加や新しいサーバーの起動は行いません。

区切り補助は英語専用です。原文の単語・数字を厳格に照合し、内部の文末候補だけを使います。翻訳入力と履歴の原文は保持し、句読点を補った全文で置き換えません。未準備やエラー時も原文をつなぐ翻訳は続け、設定に状態を表示します。

## 音声認識は任意

文章・画面OCR・YouTubeの表示字幕の翻訳に音声認識環境は不要です。PCで再生中の音声も翻訳する場合に、アプリから追加します。英語専用のNemotron Speech Streaming English 0.6Bは、NVIDIA公式GGUFとNeMo-Speech.cpp 0.3.0を使い、Pythonなしで動作します。CPUまたはVulkanを選べ、追加準備には約2GBの空きを見込みます。

多言語用にはQwen3-ASR CUDAまたはWhisper base CPUも選べます。uv・管理Python・venv・モデル・キャッシュはアプリ専用領域へ用意します。初回準備の空き容量の目安は、Qwen CUDAが14GiB、Qwen CPUが8GiB、Whisper CPUが4GiBです。翻訳用の容量とは別に確認し、再開時は検証できたキャッシュを考慮して必要な残容量を減らします。

音声認識・翻訳はPC内で実行します。初回のソフトウェアや重みの取得にはネット接続を使いますが、字幕・音声・訳文を外部ASRやクラウド推論サービスへ送信する構成ではありません。

## 日本語の訳文を読み上げる

「読み上げを準備」からspeech.cpp 0.10.1と彩りTTSの小型モデル、Irodori-TTS v4.1 Small-MFを追加します。初回取得は合計約1.95GB、展開用を含む空き容量の目安は約4.4GiBです。TTS専用のPythonは不要です。準備だけでは読み上げや音声取得は始まりません。

日本語の訳文にある「読み上げ」で手動再生できます。「声を試す」で確認し、「読み上げを停止」で止められます。「日本語の訳文を自動で読み上げる」は初期状態でオフです。読み上げ中は再生音声の取得を一時停止し、自分の読み上げが再び翻訳へ流れることを防ぎます。合成・待機中の音声取得は続きます。

「標準の固定声」は最初の読み上げ時、または「固定声を準備」で作成します。自分の参照音声を使うときは、「参照音声から登録」で1〜120秒・64MiB以内の声が入ったWAVを選びます。元のWAVは変更せず、声の特徴をGGUFへ保存して、文章を分割した各区間で同じ固定声を使います。固定声ファイルはアプリの保存先に残ります。

基準速度とライブ時の自動加速を設定できます。遅れが大きいときは上限の範囲で速め、遅れが減ると基準速度へ戻します。通常の読み上げ音声はメモリーで扱い、音声ファイルを残しません。標準声の準備で作る一時WAVは、特徴を保存した後に削除します。初回の声準備には待ち時間があります。

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
- [sherpa-onnx固定版](https://github.com/k2-fsa/sherpa-onnx/blob/af2d8d81dbfa61c4cea4c568da78f3982edca005/LICENSE)と[CT-Transformer配布元](https://k2-fsa.github.io/sherpa/onnx/punctuation/pretrained_models.html#sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8)はApache-2.0です。区切り補助の保存先に、モデルの由来、Sherpaのライセンス、ONNX RuntimeのMIT Licenseと第三者通知を保持します。
- その他の配布元：[llama.cpp](https://github.com/ggml-org/llama.cpp)、[Qwen3-ASR](https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf)、[Whisper base](https://huggingface.co/Systran/faster-whisper-base)。
- [Nemotron Englishモデル](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b)は[NVIDIA Open Model License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/)、[NVIDIA NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp)はApache-2.0です。取得したランタイムの第三者通知も保持します。彩りTTS用speech.cppとは別の環境です。
- [Indra OCRのコード](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/LICENSE)はApache-2.0、[重み](https://huggingface.co/datalab-to/indra-ocr/blob/bd02167fdf2005a19962cc15188f8496a2c87d3b/LICENSE)はModified OpenRAIL-Mです。モデルカードと両方のライセンスを保存領域に保持します。
- [PaddleOCR-VL-1.6公式GGUF](https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6-GGUF/tree/511b09642bb324401f15f97cc23bc67e8f0a291d)には[モデルのApache-2.0](https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6/blob/c5630abae1d940eafe0697512a0325494b02ab42/LICENSE)が適用され、モデルカードと通知を保持します。

Indraの重みには、出力にも適用されるShare-a-Like、帰属表示、競合製品・サービスに関する条件があります。また、本人・所属組織等の前年売上または累計資金調達額が2,000万米ドルを超える場合の制限があり、この売上・調達額の条項には個人利用・研究目的の例外が記載されています。競合製品・サービスの条項とは別の条件です。追加や利用の前に、リンク先の原文で自分の用途に適用される条件を確認してください。アプリのMIT Licenseをモデルや出力へ一律に適用する説明ではありません。

フォントはNoto Sans JPを本体に埋め込み、SIL Open Font Licenseの通知をassets/fonts/OFL.txtへ同梱します。翻訳・文字認識・音声認識・読み上げのモデルと外部ランタイムは、ZIPに同梱せず必要に応じて取得します。

## 配布担当者向けの作成手順

検証済みのtarget/x86_64-pc-windows-msvc/release/plamo-translate.exeとtarget/x86_64-pc-windows-msvc/release/jimoto-setup.exeを用意します。作成スクリプトはビルド・ダウンロード・公開を行いません。

    # 必須ファイルと起動構成を確認するだけ
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -PreflightOnly

    # ローカルZIPを作成する
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package.ps1 -Version 0.5.0

出力はdist/Jimoto-Honyaku-kun-0.5.0-win-x64.zip、SHA-256の.zip.sha256、内容一覧の.manifest.jsonです。出力先は-OutDirで指定できます。同名の既存出力は上書きせず、バージョンまたは出力先を変更して作成します。

収録はファイル単位の許可リストです。GUI・セットアップCLI・起動スクリプト・任意の音声認識用スクリプトと固定メタデータ・ブラウザー拡張・README・LICENSE・配布資料を入れます。Indraと英語の文章区切り補助の準備コード・固定情報はGUI本体に組み込みます。モデル重み、取得済みランタイム、venv、固定声GGUF、参照WAV、検証素材、利用者の設定・履歴・接続キー・ログ、Git情報、一時ファイルは入りません。パッケージmanifestには相対パスとハッシュを記録し、機器名・ユーザー名・保存先の絶対パスは記録しません。

出力先の新しいGUIDステージへコピーし、場所と名前を確認してそのステージだけを後処理で削除します。利用者のファイルは移動しません。ZIPの全エントリーは許可リストと照合します。確認済みの範囲と未検証項目は[検証範囲](VALIDATION.md)を参照してください。GitHubへの公開・リリース作成は、このスクリプトでは行いません。
