# 地元翻訳くん

<img src="assets/icons/jimoto.svg" alt="地元翻訳くんのアイコン" width="96" height="96">

translated by PLaMo

Windows用のローカル翻訳アプリです。文章、画面の範囲、YouTubeの表示字幕、PCで再生中の音声を翻訳できます。範囲翻訳とライブ翻訳は同時に使えます。

**[Windows版のダウンロード](https://github.com/yebisu0529-ship-it/jimoto-honyaku-kun/releases)** · **プレビュー版 0.4.0**

0.3.0ではZIPを新しいフォルダーへ展開し、空の保存領域から翻訳モデル・音声認識・読み上げを導入して、実際の推論と再生を確認しました。YouTube拡張による字幕受信と日本語の訳文表示も確認しています。範囲選択のマウス操作と、追加したNemotronのGUI操作は未検証のプレビュー版です。詳しくは[検証範囲](docs/VALIDATION.md)をご覧ください。

![初回のモデル準備画面（実コードから描画したデモ）](assets/screenshots/first-run.png)

## はじめに

Windows 10 / 11 x64向けのポータブル版は、ZIPを展開して `地元翻訳くん.cmd` を開きます。利用者がRustやPythonを事前に入れる必要はありません。`start.ps1` はGUIを開くための起動補助で、モデルの準備・起動・終了はアプリが管理します。起動直後に音声取得は始まりません。

初回は、まずアプリ内で**翻訳モデル**を準備します。テキスト・画面OCR・YouTube字幕だけなら、Pythonや音声認識モデルは不要です。

翻訳用のllama.cppには[Microsoft Visual C++ v14 再頒布可能パッケージ（x64）](https://learn.microsoft.com/ja-jp/cpp/windows/latest-supported-vc-redist?view=msvc-170)が必要です。未導入のPCでは、公式のx64版をインストールしてから翻訳エンジンを起動してください。なお、アプリ本体と初期設定ツールにはVisual C++ランタイムが静的に組み込まれています。

1. 「PC情報と保存先」でRAM、GPUの専用VRAMと現在の空き、保存先ディスクの空きを確認します。機器情報はPC内で処理し、外部へ送信しません。
2. 「速度を優先」「バランス」「精度を優先」から方針を選び、推奨モデル・必要容量・取得先を確認します。
3. 自分でダウンロードを開始します。途中で中止でき、同じ構成なら未完了ファイルを使って再開できます。サイズとSHA256の検証後にモデルを確定します。
4. 準備が完了したら翻訳を使えます。保存先は「PC情報と保存先」から変更できます。

モデルの候補はPFN公式の0.8B / 2B / 8B / 31B、計14種類のGGUFです。取得先コミット・サイズ・SHA256を固定しています。手動選択でも容量・実行方式の条件を確認し、収まらないモデルは選べません。既存モデルを自動で大きなものに置き換えません。

推奨は応答速度と容量を考えた目安です。リアルタイム優先ではGPUの空きに応じて最大2Bまたは8B、CPUでは0.8Bを基本にします。バランス優先はGPUで最大8B、CPUで最大2B。品質優先は容量内の大きなモデルを候補にします。すべてのPCでの速度や品質を保証するものではありません。

実行環境はllama.cpp b12119のCPU / Vulkan / CUDAから選びます。共有GPUメモリーを専用VRAMとして数えず、GPUや対応ドライバーを確認できない場合はCPUを候補にします。NVIDIAの翻訳用CUDA12.4はWindowsドライバー551.61以上が基準です。GGUF用の画面なので、FP8・NVFP4形式は選択肢に含みません。

## 使い方

- **範囲翻訳**：「範囲を翻訳」または `Ctrl+Alt+T` → ドラッグ。Windows OCRで読み取って翻訳します。複数画面では対象画面を選べます。範囲選択はEscで中止できます。
- **文章・画像**：文章の入力・貼り付け、画像ファイル、画像ドロップから翻訳できます。`Ctrl+Enter`で翻訳。結果は読み取り専用で、コピー・保存できます。
- **ライブ・タイムライン**：時刻・原文・訳文を自動保存します。原文表示、自動スクロール、字幕の別窓表示を切り替えられます。過去を読むときは自動スクロールをオフにできます。
- **ファイル**：TXT / Markdown / SRT / WebVTTを別ファイルへ翻訳します。字幕は元の開始・終了時刻を維持します。
- **翻訳設定**：53言語、用語集、参考文脈、日本語の常体・敬体に対応します。詳細設定は折りたたみ内にあります。

画面は16〜20pxを中心とした文字サイズと色の役割を揃え、日本語用のNoto Sans JPを同梱しています。フォントには同梱のOFL通知が適用されます。

## YouTubeの表示字幕

1. Chrome / Edgeの拡張機能画面で、同梱の `extension` フォルダーを手動で読み込みます。
2. アプリで接続キーをコピーし、拡張機能の設定に保存します。
3. YouTube側で英語CCなどの字幕を表示し、アプリの「YouTube字幕」をオンにします。

詳しくは [extension/README.md](extension/README.md)。拡張機能の読み込みやブラウザーのログイン操作を自動では行いません。

YouTube字幕と音声認識は独立にオン・オフできます。両方を使うときは表示中の字幕を優先し、字幕がないときは音声認識を使います。一時停止中の古い字幕は別窓に残しません。字幕は動画の再生位置、音声は音声専用の経過時間を記録します。

## 再生音声の翻訳は追加機能

必要なときに、設定の「音声認識を準備」から導入します。英語には**Nemotron Speech Streaming English 0.6B**を標準候補にし、多言語用のQwenとWhisperも選べます。既存の設定は、利用者が切り替えるまで保持します。

- **Nemotron Speech Streaming English 0.6B / CPU・Vulkan**：英語専用です。約700MBの公式Q8_0 GGUFとNVIDIA NeMo-Speech.cpp 0.3.0を取得し、Pythonを使わず動作します。160ms単位で音声を連続送信し、認識途中の英文を表示します。確定した文だけを翻訳し、時刻付きで保存します。設定で560msまたは160msのモデル動作単位を選べます。実際の表示までには文の区切りや翻訳の待ち時間も含まれます。

- **多言語Whisper base / CPU**：対応GPUを確認できないPCの候補です。
- **Qwen3-ASR-0.6B-hf / CUDA**：対応GPUと空き容量を確認できるPCの候補です。音声用PyTorch CUDA12.8はドライバー570.65以上を別に確認します。翻訳用CUDAが動くことだけでは、この条件を満たした扱いにしません。

Nemotronの追加準備には約2GBの空き容量を見込みます。Qwen CUDAは14GiB、Qwen CPUは8GiB、Whisper CPUは4GiBが初回の目安です。翻訳モデル用とは別に必要です。QwenとWhisper用の固定版uv・管理Python 3.13・独立した環境はアプリ専用領域に用意し、システムPythonやグローバルPATHは変更しません。

モデルと依存関係の確認が完了してから音声認識環境を登録します。失敗時は既存のモデル登録を保持し、取得済みキャッシュを残します。NemotronとQwen・Whisperは別の保存領域を使います。

準備後に「PC音声」または `Ctrl+Alt+L` を押すと、Windowsの既定出力で再生中の音を取得します。マイク入力ではありません。Nemotronは無音も含む連続音声、QwenとWhisperは2〜4秒の区間を認識します。待ち行列に上限を設け、音声が欠落した場合はNemotronの認識状態をリセットします。逐語の同時通訳ではなく、確定した区間ごとの字幕翻訳です。音声翻訳中も範囲OCRを使えます。

## 日本語の読み上げ

設定の「読み上げ」で、**Irodori-TTS v4.1 Small-MF**と**speech.cpp 0.10.1**を追加できます。小型の日本語モデルを使い、Pythonは不要です。初回は約1.95GBを取得し、展開用を含む空き容量の目安は約4.4GiBです。Windows版の実行には、CPUを使う場合もVulkan対応ドライバーが必要です。

訳文やタイムラインの読み上げボタンで再生できます。「日本語の訳文を自動で読み上げる」は初期状態ではオフです。停止や停止後の再生ができ、読み上げ中はアプリ自身の音声を音声翻訳へ取り込まないようにします。GPUの空きを起動直前にも確認し、4GiB未満または確認できない場合は、その起動だけCPUを使います。

読み上げの基準速度は0.8〜1.6倍で選べます。ライブ翻訳では、待ち時間に応じて生成速度を段階的に上げ、遅れが減ると基準速度へ戻します。自動調整の上限は2.0倍です。手動の読み上げは基準速度で全文を読みます。ライブで新しい訳文が届いた場合は、古い訳文の残りを区切り位置で切り上げます。

音声の生成・再生はPC内のメモリーで行い、生成した音声ファイルを保存しません。初回のモデル読み込みや長文の生成には待ち時間があり、ライブ字幕と同じ速度で読み上げ続けられることは保証していません。

## 保存・通信

既定の保存先は利用者のLocal AppData配下です。実際の場所はアプリ内に表示します。設定・英語や日本語の原文と訳文・履歴を再起動後も保持し、最新のタイムラインを復元します。「新しいタイムライン」でも以前のファイルは残ります。保存中断からの回復では原本を保全します。

タイムラインはTXT / JSON / SRTへ書き出せます。音声とYouTube、または複数動画が混在する履歴はSRTへの書き出しを拒否し、TXT / JSONを案内します。画面画像・生の音声を翻訳履歴へ保存する構成ではありません。

翻訳・音声認識・読み上げはPC内で実行し、原文・字幕・音声をクラウド推論へ送りません。初回のソフトウェアとモデルの取得にはネット接続を使います。ローカル接続は翻訳17860、Qwen・Whisper音声認識17861、字幕ブリッジ17862、読み上げ17863、Nemotron17864です。アプリは自分で起動したモデルプロセスだけを管理します。

## CLI・開発・配布

配布版の通常利用ではGUIから準備できます。`setup.ps1` は方針・モデル・保存先をセットアップCLIへ渡す補助です。PC情報と推奨の確認だけなら、取得せず次を実行できます。

```powershell
.\setup.ps1 -Policy realtime -PlanOnly
# CLIを直接使う場合。保存先は自分の専用フォルダーへ変更します。
.\jimoto-setup.exe --root C:\LocalModels\Jimoto --policy balanced --probe
```

ソース版のビルドにはRust MSVC環境とWindows SDKが必要ですが、配布ZIPの利用者には不要です。`cargo build --release --locked --target x86_64-pc-windows-msvc`でビルドできます。パッケージの作成手順・収録範囲は [docs/DISTRIBUTION.md](docs/DISTRIBUTION.md) を参照してください。モデル重み・利用者の履歴・ログ・認証情報はZIPへ入れません。

検証範囲は [docs/VALIDATION.md](docs/VALIDATION.md) をご覧ください。Opus 5.5でUI/UXを2回監査し、配色・フォント・操作状態・初回導線を改善しました。公開する日本語はAntigravity CLIのGemini 4 Argonで校正しました。CPUでの速度、すべてのGPUでの動作、14種類すべての実測性能は保証していません。

開発・検証時は、絶対パスの環境変数`PLAMO_DATA_DIR`で設定・履歴・モデルの保存領域を分離できます。相対パスは採用しません。通常利用では指定不要です。

## モデルとライセンス

アプリのソースはMITです。モデル・外部ランタイム・フォントには、それぞれの条件が適用されます。モデルをアプリのMITライセンスで配布できるという意味ではありません。利用条件は公式本文を確認してください。

- [PLaMo Community License](https://plamo.preferredai.jp/info/plamo-community-license-ja)
- PFN公式GGUF：[0.8B](https://huggingface.co/pfnet/plamo-3-translate-0.8b-GGUF) / [2B](https://huggingface.co/pfnet/plamo-3-translate-2b-GGUF) / [8B](https://huggingface.co/pfnet/plamo-3-translate-8b-GGUF) / [31B](https://huggingface.co/pfnet/plamo-3-translate-31b-GGUF)
- [llama.cpp](https://github.com/ggml-org/llama.cpp) / [Qwen3-ASR](https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf) / [Whisper base](https://huggingface.co/Systran/faster-whisper-base)
- [speech.cpp](https://github.com/nyosegawa/speech.cpp) / [Irodori-TTS v4.1 Small-MF GGUF](https://huggingface.co/sakasegawa/Irodori-TTS-v4.1-Small-MF-GGUF)
- [Nemotron Speech Streaming English 0.6B](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b) / [NVIDIA NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) / [NVIDIA Open Model License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/)

読み上げの準備では固定したモデルカードとMIT・音声コーデックのApache-2.0通知も取得します。

NemotronのモデルにはNVIDIA Open Model License、NeMo-Speech.cppにはApache-2.0が適用されます。彩りTTS用speech.cppとは別の実行環境です。取得したランタイムのライセンス・第三者通知は保持します。
