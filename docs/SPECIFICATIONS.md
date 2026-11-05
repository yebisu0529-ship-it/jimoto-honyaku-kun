# 詳細仕様・動作要件・ライセンス

READMEから分離した、モデル構成・追加機能・通信ポート・ビルド手順・ライセンスの詳細です。

---

## 1. 翻訳モデルと実行環境

### 推奨方針の目安

初回セットアップでは、PCのRAM・専用VRAM・空き容量をもとに推奨モデルを提案します（機器情報はPC内でのみ処理します）。

| 方針 | GPU利用時の目安 | CPU利用時の目安 |
| --- | --- | --- |
| 速度を優先 | 空き容量に応じて最大 2B または 8B | 0.8B |
| バランス | 最大 8B | 最大 2B |
| 精度を優先 | 空き容量に収まる最大のモデル | 空き容量に収まるモデル |

- モデル候補は PFN 公式の PLaMo 0.8B / 2B / 8B / 31B（計14種類の GGUF）です。
- 取得先コミット・ファイルサイズ・SHA-256 を固定して検証します。中断時は未完了ファイルから再開できます。
- 既存の導入済みモデルを自動で別モデルへ置き換えることはありません。

### 翻訳ランタイム（llama.cpp b12119）

- **対応バックエンド**：CPU / Vulkan / CUDA 12.4
- **NVIDIA CUDA 条件**：Windows ドライバー 551.61 以上（共有GPUメモリーは専用VRAMに含めません）
- **必須ランタイム**：翻訳用 `llama.cpp` と英語区切り補助には [Microsoft Visual C++ v14 再頒布可能パッケージ（x64）](https://learn.microsoft.com/ja-jp/cpp/windows/latest-supported-vc-redist?view=msvc-170) が必要です（アプリ本体・セットアップCLIは静的リンク済み）。

---

## 2. 画面・画像の文字認識（OCR）

設定の「画面・画像の文字認識」から方式を切り替えられます。画像はすべてPC内で処理します。

| 方式 | 初回取得量 | 特徴・動作条件 |
| --- | --- | --- |
| **Windows OCR**（初期設定） | 追加なし | Windows標準の文字認識を使用。 |
| **Indra OCR** | 約184MB（日英9モデル約54MB＋専用Python・23パッケージ） | 軽量なCPU向け。行ごとに言語を自動判定。システムPythonやPATHは変更しません。 |
| **PaddleOCR-VL-1.6** | 約1.82GB（展開込み空き目安 約4GB） | 精度重視。翻訳用 `llama.cpp` を再利用するためPython不要。CPU固定またはGPU自動判定を選択可能。 |

---

## 3. ライブ字幕（段落結合・英語区切り補助）

- **段落単位の翻訳更新**：YouTube字幕や音声認識の短い断片を1行ずつ区切らず、話のまとまりごとに段落へつないで翻訳を更新します。直前の確定段落も文脈として参照します。
- **画面表示**：更新中の訳文は上部の固定欄に表示し、確定した対訳は履歴へ時系列で追加します。スクロール中は閲覧位置を保持します。
- **英語の文章区切り補助（任意）**：
  - 初回取得は約86MB（空き容量目安 512MiB以上）。`sherpa-onnx 2.0.1` と `CT-Transformer INT8` をCPUで実行します。
  - 補助モデルの出力が元の単語や数字を変えた場合は破棄し、翻訳には常に元の原文を渡します。
- **書き出し**：動画変更や終了時に途中だった文章は「未確定」として保持します。TXT / JSON には未確定分も含め、SRT には確定済みの訳文のみを書き出します（複数動画や音声＋YouTubeが混在する履歴はSRT対象外）。

---

## 4. PC再生音声の認識（STT）

Windowsの既定出力で再生中の音声を取得します（マイク入力ではありません）。必要な場合のみ設定の「音声認識を準備」から導入します。

| モデル | 対象言語・実行方式 | 初回空き容量の目安 | 特徴 |
| --- | --- | --- | --- |
| **Nemotron Speech Streaming 0.6B** | 英語専用（CPU / Vulkan） | 約2GB | 約700MBの公式Q8_0 GGUF＋`NeMo-Speech.cpp 0.3.0`。Python不要。160msまたは560ms単位で連続認識。 |
| **Qwen3-ASR-0.6B-hf** | 多言語（CUDA / CPU） | CUDA: 14GiB / CPU: 8GiB | 2〜4秒区間ごとに認識。CUDA 12.8版はNVIDIAドライバー 570.65 以上を別途確認します。 |
| **Whisper base** | 多言語（CPU） | 4GiB | 対応GPUを確認できない環境向けの多言語モデル。 |

- Qwen・Whisper 用の `uv`・管理Python 3.13・仮想環境はアプリ専用領域に作成し、システムのPython環境には影響しません。

---

## 5. 日本語の読み上げ（TTS）

設定の「読み上げ」から **Irodori-TTS v4.1 Small-MF** と **speech.cpp 0.10.1** を追加できます。

- **必要容量**：初回取得 約1.95GB（展開込み空き容量の目安 約4.4GiB）。Python不要。
- **動作条件**：Windows版の `speech.cpp` はCPU実行時でもシステムの Vulkan ローダーが必要です。起動時に空きVRAMが 4GiB 未満の場合は自動でCPU実行に切り替えます。
- **固定声**：初回再生時または設定から「標準の固定声」を作成します。手元のWAV（1〜120秒・64MiB以内）から声の特徴をGGUFとして登録することも可能です。
- **速度調整とループ防止**：基準速度は 0.8〜1.6倍。ライブ翻訳の自動読み上げ時は遅延に応じて最大 2.0倍まで自動加速します。読み上げ再生中はアプリ自身の声をPC音声認識へ取り込まないよう抑止します。

---

## 6. ミニ表示・字幕ウィンドウ

- **ミニ表示（`Ctrl+Shift+M` / `Esc`で戻る）**：同じウィンドウをコンパクトに切り替え、確定訳文と上部の進行状況を表示します。入力中の文章・履歴・固定声設定は維持されます。
- **独立した字幕ウィンドウ**：通常画面の「字幕ウィンドウ」またはミニ表示のメニューから別窓で開けます。字幕ウィンドウだけを閉じても翻訳・保存・読み上げは継続します。

---

## 7. 保存先とローカル通信

- **保存先**：既定は `Local AppData` 配下（アプリ内に実パスを表示）。設定・履歴・モデル・ランタイムを保存します。開発・検証時は絶対パスの環境変数 `PLAMO_DATA_DIR` で分離可能です。
- **外部送信なし**：モデルの初回取得時を除き、原文・画像・字幕・音声を外部サーバーやクラウドへ送信しません。
- **ローカル待受ポート（`127.0.0.1`）**：
  - `17860`：翻訳（llama.cpp）
  - `17861`：音声認識（Qwen / Whisper）
  - `17862`：YouTube字幕ブリッジ
  - `17863`：読み上げ（speech.cpp）
  - `17864`：英語音声認識（Nemotron）
  - `17865`：PaddleOCR-VL
  - `17866`：Indra OCR

---

## 8. CLI・ソースからのビルド

通常利用ではGUIからすべて操作できます。コマンドラインから構成確認を行う場合の例：

```powershell
# PC情報と推奨プランの確認のみ（ダウンロードなし）
.\setup.ps1 -Policy realtime -PlanOnly

# セットアップCLIを直接呼ぶ場合
.\jimoto-setup.exe --root C:\LocalModels\Jimoto --policy balanced --probe
```

ソースからビルドする場合は Rust（MSVC）と Windows SDK が必要です。

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc
```

配布ZIPの作成手順は [docs/DISTRIBUTION.md](DISTRIBUTION.md)、動作確認済みの範囲は [docs/VALIDATION.md](VALIDATION.md) を参照してください。

---

## 9. モデルとライセンス

アプリ本体のソースコードは [MIT License](../LICENSE) です。同梱フォント・各モデル・外部ランタイムにはそれぞれのライセンスが適用されます。

- **PLaMo 翻訳モデル**：[PLaMo Community License](https://plamo.preferredai.jp/info/plamo-community-license-ja)
  - PFN公式GGUF：[0.8B](https://huggingface.co/pfnet/plamo-3-translate-0.8b-GGUF) / [2B](https://huggingface.co/pfnet/plamo-3-translate-2b-GGUF) / [8B](https://huggingface.co/pfnet/plamo-3-translate-8b-GGUF) / [31B](https://huggingface.co/pfnet/plamo-3-translate-31b-GGUF)
  - PLaMo Community License 第5条に基づき、コピー・保存・本文ファイル翻訳の出力末尾に `translated by PLaMo` / `Built with PLaMo` を付記します（SRT / WebVTT 字幕はフォーマット維持のため除外）。
- **翻訳・多言語音声認識**：[llama.cpp](https://github.com/ggml-org/llama.cpp) / [Qwen3-ASR](https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf) / [Whisper base](https://huggingface.co/Systran/faster-whisper-base)
- **英語音声認識（Nemotron）**：[Nemotron Speech Streaming English 0.6B](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b)（[NVIDIA Open Model License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/)） / [NVIDIA NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp)（Apache-2.0）
- **英語区切り補助**：[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/blob/af2d8d81dbfa61c4cea4c568da78f3982edca005/LICENSE) / [CT-Transformer](https://k2-fsa.github.io/sherpa/onnx/punctuation/pretrained_models.html#sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8)（Apache-2.0）
- **読み上げ（Irodori-TTS）**：[speech.cpp](https://github.com/nyosegawa/speech.cpp)（MIT） / [Irodori-TTS v4.1 Small-MF GGUF](https://huggingface.co/sakasegawa/Irodori-TTS-v4.1-Small-MF-GGUF)（MIT・コーデック由来Apache-2.0）
- **文字認識（OCR）**：
  - Indra OCR：[コード（Apache-2.0）](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/LICENSE) / [モデル重み（Modified OpenRAIL-M）](https://huggingface.co/datalab-to/indra-ocr/blob/bd02167fdf2005a19962cc15188f8496a2c87d3b/LICENSE)（※出力のShare-a-Like、競合利用、組織規模に関する制限あり。導入前にリンク先原文をご確認ください）
  - PaddleOCR-VL-1.6：[公式GGUF](https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6-GGUF/tree/511b09642bb324401f15f97cc23bc67e8f0a291d) / [Apache-2.0](https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6/blob/c5630abae1d940eafe0697512a0325494b02ab42/LICENSE)
- **フォント**：Noto Sans JP（`assets/fonts/OFL.txt` の SIL Open Font License を適用）
