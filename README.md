# 地元翻訳くん

<img src="assets/icons/jimoto.svg" alt="地元翻訳くんのアイコン" width="96" height="96">

translated by PLaMo

Windows用のローカル翻訳アプリです。文章、画面の範囲、YouTubeの表示字幕、PCで再生中の音声を翻訳できます。範囲翻訳とライブ翻訳は同時に使えます。

**[Windows版のダウンロード](https://github.com/yebisu0529-ship-it/jimoto-honyaku-kun/releases)** · **プレビュー版 0.5.0**

0.5.0では画面OCRの方式を選べるようにし、日本語の読み上げでは保存した固定声を使うようにしました。NemotronでPC音声を取得した際の字幕時計も修正しました。ライブ字幕は原文を段落につなぎ、文章全体の訳文を更新する方式に変更しています。確認済みの処理と、実操作の確認が残る項目は[検証範囲](docs/VALIDATION.md)をご覧ください。

![初回のモデル準備画面（実コードから描画したデモ）](assets/screenshots/first-run.png)

## はじめに

Windows 10 / 11 x64向けのポータブル版は、ZIPを展開して `地元翻訳くん.cmd` を開きます。利用者がRustやPythonを事前に入れる必要はありません。`start.ps1` はGUIを開くための起動補助で、モデルの準備・起動・終了はアプリが管理します。起動直後に音声取得は始まりません。

初回は、まずアプリ内で**翻訳モデル**を準備します。文章・Windows OCR・YouTube字幕の翻訳には、Python環境や音声認識モデルの追加は不要です。Indra OCRを選ぶ場合は、専用Python環境をアプリが用意します。

翻訳用のllama.cppには[Microsoft Visual C++ v14 再頒布可能パッケージ（x64）](https://learn.microsoft.com/ja-jp/cpp/windows/latest-supported-vc-redist?view=msvc-170)が必要です。未導入のPCでは、公式のx64版をインストールしてから翻訳エンジンを起動してください。なお、アプリ本体と初期設定ツールにはVisual C++ランタイムが静的に組み込まれています。

1. 「PC情報と保存先」でRAM、GPUの専用VRAMと現在の空き、保存先ディスクの空きを確認します。機器情報はPC内で処理し、外部へ送信しません。
2. 「速度を優先」「バランス」「精度を優先」から方針を選び、推奨モデル・必要容量・取得先を確認します。
3. 自分でダウンロードを開始します。途中で中止でき、同じ構成なら未完了ファイルを使って再開できます。サイズとSHA256の検証後にモデルを確定します。
4. 準備が完了したら翻訳を使えます。保存先は「PC情報と保存先」から変更できます。

モデルの候補はPFN公式の0.8B / 2B / 8B / 31B、計14種類のGGUFです。取得先コミット・サイズ・SHA256を固定しています。手動選択でも容量・実行方式の条件を確認し、収まらないモデルは選べません。既存モデルを自動で大きなものに置き換えません。

推奨は応答速度と容量を考えた目安です。リアルタイム優先ではGPUの空きに応じて最大2Bまたは8B、CPUでは0.8Bを基本にします。バランス優先はGPUで最大8B、CPUで最大2Bです。品質優先は容量内の大きなモデルを候補にします。すべてのPCでの速度や品質を保証するものではありません。

実行環境はllama.cpp b12119のCPU / Vulkan / CUDAから選びます。共有GPUメモリーを専用VRAMとして数えず、GPUや対応ドライバーを確認できない場合はCPUを候補にします。NVIDIAの翻訳用CUDA12.4はWindowsドライバー551.61以上が基準です。GGUF用の画面なので、FP8・NVFP4形式は選択肢に含みません。

## 使い方

- **範囲翻訳**：「範囲を翻訳」または `Ctrl+Alt+T` → ドラッグ。設定で選んだOCRで読み取って翻訳します。複数画面では対象画面を選べ、Escで中止できます。
- **文章・画像**：文章の入力・貼り付け、画像ファイル、画像ドロップから翻訳できます。`Ctrl+Enter`で翻訳。結果は読み取り専用で、コピー・保存できます。
- **ライブ・タイムライン**：処理状況を画面上部にまとめ、確定した字幕を新しい順に表示します。時刻・原文・訳文を自動保存し、原文表示、自動スクロール、字幕の別窓表示を切り替えられます。
- **ファイル**：TXT / Markdown / SRT / WebVTTを別ファイルへ翻訳します。字幕は元の開始・終了時刻を維持します。
- **翻訳設定**：53言語、用語集、参考文脈、日本語の常体・敬体に対応します。詳細設定は折りたたみ内にあります。

画面は16〜20pxを中心とした文字サイズと色の役割を揃え、日本語用のNoto Sans JPを同梱しています。フォントには同梱のOFL通知が適用されます。

## ミニ表示で使う

初回のモデル準備や設定は通常表示で行います。準備後は「ミニ表示」またはアプリ内の `Ctrl+Shift+M` で、同じウィンドウを小さな表示へ切り替えられます。「通常表示」またはミニ表示中の `Esc` で戻ります。入力中の文章・翻訳履歴・選択した固定声は切り替えても保持します。

ミニ表示では確定した訳文を中心に表示し、音声認識・翻訳の途中状態は上部に分けて表示します。ライブ入力を一時停止・再開でき、最前面表示は初期状態ではオフで、必要なときにオンにできます。範囲翻訳をすると手動の原文と訳文を表示し、「ライブへ戻る」でライブ表示を再開します。

ウィンドウの閉じるボタンはアプリ全体の終了操作です。アプリが起動したモデル処理も終了します。ミニ表示の実GUI操作の確認はまだ残っており、詳しくは[検証範囲](docs/VALIDATION.md)をご覧ください。

## 画面・画像の文字認識

設定の「画面・画像の文字認識」で方式を選びます。初期状態はWindows OCRです。追加方式は「文字認識を準備」から利用条件を確認して導入します。

| 方式 | 追加するもの・使い方 |
| --- | --- |
| Windows OCR | Windowsに導入済みの文字認識を利用。追加モデル不要です。 |
| Indra OCR | 軽量なCPU用。日本語CJKを含む日英9モデル約54MB、専用Python、Indraを含む23パッケージを合わせた初回取得は約184MBです。行ごとに認識言語を自動判定します。 |
| PaddleOCR-VL-1.6 | 精度重視の選択肢。公式モデルと画像用モデルを合わせて約1.82GB取得し、翻訳用ランタイムを再利用します。CPUを指定するかGPUの利用可否を自動判定でき、Pythonは不要です。 |

Indra用の管理Python 3.13と環境はアプリ専用領域に置き、システムPythonやPATHを変更しません。モデルとパッケージを固定ハッシュで検証してから登録します。Indraの重みには出力のShare-a-Like、競合製品・サービス、売上・資金調達額に関する条件があります。追加前に[Modified OpenRAIL-Mの原文](https://huggingface.co/datalab-to/indra-ocr/blob/bd02167fdf2005a19962cc15188f8496a2c87d3b/LICENSE)を確認してください。

PaddleOCRのモデルはApache-2.0です。GPUの利用には対応する翻訳用ランタイムと空き容量が必要です。どの方式でも、小さい文字や複雑な配置の認識結果は原文と照合してください。画像はPC内で処理します。

## YouTubeの表示字幕

1. Chrome / Edgeの拡張機能画面で、同梱の `extension` フォルダーを手動で読み込みます。
2. アプリで接続キーをコピーし、拡張機能の設定に保存します。
3. YouTube側で英語CCなどの字幕を表示し、アプリの「YouTube字幕」をオンにします。

詳しくは [extension/README.md](extension/README.md) をご覧ください。拡張機能の読み込みやブラウザーのログイン操作を自動では行いません。

YouTube字幕と音声認識は独立にオン・オフできます。両方を使うときは表示中の字幕を優先し、字幕がないときは音声認識を使います。動画を一時停止すると、途中の文章を未確定のまま保持し、再開時に続きをつなぎます。字幕は動画の再生位置、音声は音声専用の経過時間を記録します。

## 文章をつなぐライブ字幕

音声認識やYouTube字幕の短い断片を段落へ追加し、段落全体を訳し直します。前の確定段落の原文と訳文も参照し、用語や文脈を引き継ぎます。字幕1行や短い間では区切らず、話がまとまるまで長めに原文をつなぎます。更新中の訳文は上の固定欄で置き換え、確定した文章は古い順に下へ追加します。上へスクロールしている間は読んでいる位置を保ちます。認識途中の原文が届いても、表示中の訳文は消しません。

英語では、初回のモデル準備で「英語の文章区切り補助」も選べます。追加取得は約86MBで、未準備の場合は設定から導入できます。CPUで使う小さな句読点補助モデルが区切りの候補を調べ、先の数語を受け取ってから段落をまとめます。補助モデルの出力が原文の単語や数字を変えた場合は採用せず、翻訳には原文をそのまま渡します。補助がなくても、原文を蓄積する翻訳は使えます。

自動読み上げは確定した文章だけを一度読みます。更新中の訳文は読みません。動画の変更やシーク、アプリ終了で中断した文章は、原文と途中の訳文を「未確定」として保持します。TXTとJSONには未確定の文章も含め、SRTには確定した訳文だけを書き出します。文章のつながりは改善しますが、認識・区切り・翻訳の誤りをなくすものではありません。

## 再生音声の翻訳は追加機能

必要なときに、設定の「音声認識を準備」から導入します。英語には**Nemotron Speech Streaming English 0.6B**を標準候補にし、多言語用のQwenとWhisperも選べます。既存の設定は、利用者が切り替えるまで保持します。

- **Nemotron Speech Streaming English 0.6B / CPU・Vulkan**：英語専用です。約700MBの公式Q8_0 GGUFとNVIDIA NeMo-Speech.cpp 0.3.0を取得し、Pythonを使わず動作します。160ms単位で音声を連続送信し、認識途中の英文を表示します。認識した原文を段落へ追加し、文章全体の訳文を更新して時刻付きで保存します。設定で560msまたは160msのモデル動作単位を選べます。実際の表示までには文の区切りや翻訳の待ち時間も含まれます。

- **多言語Whisper base / CPU**：対応GPUを確認できないPCの候補です。
- **Qwen3-ASR-0.6B-hf / CUDA**：対応GPUと空き容量を確認できるPCの候補です。音声用PyTorch CUDA12.8はドライバー570.65以上を別に確認します。翻訳用CUDAが動くことだけでは、この条件を満たした扱いにしません。

Nemotronの追加準備には約2GBの空き容量を見込みます。Qwen CUDAは14GiB、Qwen CPUは8GiB、Whisper CPUは4GiBが初回の目安です。翻訳モデル用とは別に必要です。QwenとWhisper用の固定版uv・管理Python 3.13・独立した環境はアプリ専用領域に用意し、システムPythonやPATHは変更しません。

モデルと依存関係の確認が完了してから音声認識環境を登録します。失敗時は既存のモデル登録を保持し、取得済みキャッシュを残します。NemotronとQwen・Whisperは別の保存領域を使います。

準備後に「PC音声」または `Ctrl+Alt+L` を押すと、Windowsの既定出力で再生中の音を取得します。マイク入力ではありません。Nemotronは無音も含む連続音声、QwenとWhisperは2〜4秒の区間を認識します。待ち行列に上限を設け、音声が欠落した場合はNemotronの認識状態をリセットします。認識区間をそのまま別々の字幕にはせず、原文を段落につないで訳文を更新します。音声翻訳中も範囲OCRを使えます。

## 日本語の読み上げ

設定の「読み上げ」で、**Irodori-TTS v4.1 Small-MF**と**speech.cpp 0.10.1**を追加できます。小型の日本語モデルを使い、Pythonは不要です。初回は約1.95GBを取得し、展開用を含む空き容量の目安は約4.4GiBです。Windows版の実行には、CPUを使う場合もVulkan対応ドライバーが必要です。

訳文やタイムラインの読み上げボタンで再生できます。「日本語の訳文を自動で読み上げる」は初期状態ではオフです。停止や停止後の再生ができ、読み上げ中はアプリ自身の音声を音声翻訳へ取り込まないようにします。GPUの空き容量を起動直前にも確認し、4GiB未満または確認できない場合は、その起動だけCPUを使います。

「標準の固定声」は初回の読み上げ時に準備します。「固定声を準備」から先に作ることもできます。自分の参照音声を使う場合は「参照音声から登録」で、声が入った1〜120秒・64MiB以内のWAVを選びます。元のWAVは変更せず、声の特徴をGGUFとしてアプリの保存先に登録します。読み上げる文章や区切りが変わっても、同じ固定声を各区間で使います。

読み上げの基準速度は0.8〜1.6倍で選べます。ライブ翻訳では、待ち時間に応じて生成速度を段階的に上げ、遅れが減ると基準速度へ戻します。自動調整の上限は2.0倍です。手動の読み上げは基準速度で全文を読みます。ライブで新しい訳文が届いた場合は、古い訳文の残りを区切り位置で切り上げます。

通常の読み上げ音声はメモリーで扱い、音声ファイルを残しません。標準声の初回準備では一時WAVを作り、声の特徴を保存した後に削除します。初回の声準備・モデル読み込み・長文の生成には待ち時間があり、ライブ字幕と同じ速度で読み上げ続けられることは保証していません。

## 保存・通信

既定の保存先は利用者のLocal AppData配下です。実際の場所はアプリ内に表示します。設定・英語や日本語の原文と訳文・履歴を再起動後も保持し、最新のタイムラインを復元します。「新しいタイムライン」でも以前のファイルは残ります。保存中断からの回復では原本を保全します。

タイムラインはTXT / JSON / SRTへ書き出せます。音声とYouTube、または複数動画が混在する履歴はSRTへの書き出しを拒否し、TXT / JSONを案内します。画面画像・生の音声を翻訳履歴へ保存する構成ではありません。

翻訳・文字認識・音声認識・読み上げはPC内で実行し、原文・画像・字幕・音声をクラウド推論へ送りません。初回のソフトウェアとモデルの取得にはネット接続を使います。ローカル接続は翻訳17860、Qwen・Whisper音声認識17861、字幕ブリッジ17862、読み上げ17863、Nemotron17864、PaddleOCR17865、Indra OCR17866です。アプリは自分で起動したモデルプロセスだけを管理します。

## CLI・開発・配布

配布版の通常利用ではGUIから準備できます。`setup.ps1` は方針・モデル・保存先をセットアップCLIへ渡す補助です。PC情報と推奨の確認だけなら、取得せず次を実行できます。

```powershell
.\setup.ps1 -Policy realtime -PlanOnly
# CLIを直接使う場合。保存先は自分の専用フォルダーへ変更します。
.\jimoto-setup.exe --root C:\LocalModels\Jimoto --policy balanced --probe
```

ソース版のビルドにはRust MSVC環境とWindows SDKが必要ですが、配布ZIPの利用者には不要です。`cargo build --release --locked --target x86_64-pc-windows-msvc`でビルドできます。パッケージの作成手順・収録範囲は [docs/DISTRIBUTION.md](docs/DISTRIBUTION.md) を参照してください。モデル重み・利用者の履歴・ログ・認証情報はZIPへ入れません。

検証範囲は [docs/VALIDATION.md](docs/VALIDATION.md) をご覧ください。公開する日本語はAntigravity CLIのGemini 4 Argonで校正しています。CPUでの速度、すべてのGPUでの動作、14種類すべての実測性能は保証していません。

開発・検証時は、絶対パスの環境変数`PLAMO_DATA_DIR`で設定・履歴・モデルの保存領域を分離できます。相対パスは採用しません。通常利用では指定不要です。

## モデルとライセンス

アプリのソースはMITです。モデル・外部ランタイム・フォントには、それぞれの条件が適用されます。モデルをアプリのMITライセンスで配布できるという意味ではありません。利用条件は公式本文を確認してください。

- [PLaMo Community License](https://plamo.preferredai.jp/info/plamo-community-license-ja)
- PFN公式GGUF：[0.8B](https://huggingface.co/pfnet/plamo-3-translate-0.8b-GGUF) / [2B](https://huggingface.co/pfnet/plamo-3-translate-2b-GGUF) / [8B](https://huggingface.co/pfnet/plamo-3-translate-8b-GGUF) / [31B](https://huggingface.co/pfnet/plamo-3-translate-31b-GGUF)
- [llama.cpp](https://github.com/ggml-org/llama.cpp) / [Qwen3-ASR](https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf) / [Whisper base](https://huggingface.co/Systran/faster-whisper-base)
- [speech.cpp](https://github.com/nyosegawa/speech.cpp) / [Irodori-TTS v4.1 Small-MF GGUF](https://huggingface.co/sakasegawa/Irodori-TTS-v4.1-Small-MF-GGUF)
- [Nemotron Speech Streaming English 0.6B](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b) / [NVIDIA NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) / [NVIDIA Open Model License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/)
- [Indra OCRコードのApache-2.0](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/LICENSE) / [重みのModified OpenRAIL-M](https://huggingface.co/datalab-to/indra-ocr/blob/bd02167fdf2005a19962cc15188f8496a2c87d3b/LICENSE)
- [PaddleOCR-VL-1.6公式GGUF](https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6-GGUF/tree/511b09642bb324401f15f97cc23bc67e8f0a291d) / [モデルのApache-2.0](https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6/blob/c5630abae1d940eafe0697512a0325494b02ab42/LICENSE)

読み上げの準備では固定したモデルカードとMIT・音声コーデックのApache-2.0通知も取得します。

NemotronのモデルにはNVIDIA Open Model License、NeMo-Speech.cppにはApache-2.0が適用されます。彩りTTS用speech.cppとは別の実行環境です。取得したランタイムのライセンス・第三者通知は保持します。
