//! Verified model metadata and conservative capacity heuristics, not benchmarks.
use crate::hardware::Hardware;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const GIB: u64 = 1_073_741_824;
const MIB: u64 = 1_048_576;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Cpu,
    Vulkan,
    Cuda,
}
impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Vulkan => "vulkan",
            Self::Cuda => "cuda",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    #[default]
    Realtime,
    Balanced,
    Quality,
}
impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Realtime => "realtime",
            Self::Balanced => "balanced",
            Self::Quality => "quality",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub label: String,
    pub repo: String,
    pub revision: String,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    pub parameters_b: f32,
    pub quantization: String,
    pub backends: Vec<Backend>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub models: Vec<Model>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alternative {
    pub model: Model,
    pub fits: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recommendation {
    pub selected: Model,
    pub backend: Backend,
    pub asr_backend: String,
    pub asr_device: String,
    pub gpu_layers: u32,
    pub reasons: Vec<String>,
    pub alternatives: Vec<Alternative>,
    pub memory_budget_bytes: Option<u64>,
    pub warnings: Vec<String>,
    /// False also means an essential capacity measurement is unknown. The caller
    /// must obtain confirmation/measurements before downloading or launching it.
    pub feasible: bool,
}

pub fn bundled() -> Result<Catalog> {
    let catalog: Catalog = serde_json::from_str(include_str!("../scripts/model-catalog.json"))
        .context("同梱モデルカタログを読み込めません")?;
    anyhow::ensure!(!catalog.models.is_empty(), "モデルカタログが空です");
    for model in &catalog.models {
        anyhow::ensure!(
            model.bytes > 0
                && model.filename.ends_with(".gguf")
                && !model.filename.contains(['/', '\\'])
                && model.repo.starts_with("pfnet/plamo-3-translate-")
                && model.revision.len() == 40
                && model.revision.bytes().all(|b| b.is_ascii_hexdigit())
                && model.sha256.len() == 64
                && model.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "モデルの検証用メタデータが不正です: {}",
            model.id
        );
    }
    Ok(catalog)
}

pub fn recommend(hardware: &Hardware, policy: Policy) -> Recommendation {
    // Compile-time bundled data is validated by tests, never replaced with
    // invented hashes or an unpinned model when parsing fails.
    let catalog = bundled().expect("invalid bundled, build-time-verified model catalog");
    let mut warnings = hardware.warnings.clone();
    let gpu = hardware
        .gpus
        .iter()
        .filter(|gpu| !gpu.is_software)
        .filter_map(|gpu| {
            let dedicated = gpu.dedicated_vram_bytes?;
            let free = gpu.free_vram_bytes?.min(dedicated);
            let backend = if gpu.cuda_eligible {
                Backend::Cuda
            } else if gpu.vulkan_eligible {
                Backend::Vulkan
            } else {
                return None;
            };
            Some((gpu, backend, free))
        })
        .max_by_key(|(_, _, free)| *free);
    let minimum = catalog
        .models
        .iter()
        .find(|m| m.parameters_b == 0.8 && m.quantization == "Q4_K_M")
        .unwrap();
    let mut backend = gpu
        .as_ref()
        .map(|(_, backend, _)| *backend)
        .unwrap_or(Backend::Cpu);
    let mut asr_cuda = gpu.as_ref().is_some_and(|(gpu, backend, free)| {
        *backend == Backend::Cuda
            && *free >= 6 * GIB
            && gpu.vendor == "nvidia"
            && gpu
                .driver_version
                .as_deref()
                .is_some_and(qwen_cuda_driver_supported)
    });
    let mut gpu_budget = gpu
        .as_ref()
        .map(|(_, _, free)| free.saturating_sub(GIB + if asr_cuda { 2 * GIB } else { 0 }));
    if backend != Backend::Cpu && gpu_budget.is_none_or(|budget| budget < inference_bytes(minimum))
    {
        warnings.push("GPUの空き容量から表示・音声認識分を引くと小型モデルも収まらないため、CPUを候補にしました。".into());
        backend = Backend::Cpu;
        asr_cuda = false;
        gpu_budget = None;
    }
    let os_reserve = if hardware
        .ram_available_bytes
        .is_some_and(|free| free < 4 * GIB)
    {
        512 * MIB
    } else {
        GIB
    };
    let asr_reserve = if asr_cuda { 2 * GIB } else { 768 * MIB };
    let ram_reserve = os_reserve + asr_reserve;
    let ram_budget = hardware
        .ram_available_bytes
        .map(|n| n.saturating_sub(ram_reserve));
    let budget = if backend == Backend::Cpu {
        ram_budget
    } else {
        gpu_budget
    };
    let known = hardware.ram_available_bytes.is_some() && hardware.disk_free_bytes.is_some();
    let arch_ok = hardware.os_arch == "windows-x86_64";
    let target = if !known {
        0.8
    } else {
        match (policy, backend) {
            (Policy::Realtime, Backend::Cpu) => 0.8,
            (Policy::Realtime, _) => {
                if gpu.as_ref().is_some_and(|(_, _, free)| *free >= 16 * GIB) {
                    8.0
                } else {
                    2.0
                }
            }
            (Policy::Balanced, Backend::Cpu) => 2.0,
            (Policy::Balanced, _) => 8.0,
            (Policy::Quality, Backend::Cpu) => 8.0,
            (Policy::Quality, _) => 31.0,
        }
    };
    let mut alternatives: Vec<_> = catalog.models.iter().map(|model| {
        let needed = inference_bytes(model);
        let mut reasons = Vec::new();
        let memory_fit = match budget {
            Some(b) if needed <= b => { reasons.push(format!("推論用の見積もり{}が利用可能枠{}以内です。", size(needed), size(b))); true },
            Some(b) => { reasons.push(format!("推論に約{}必要ですが、利用可能枠は{}です。", size(needed), size(b))); false },
            None => { reasons.push("空きメモリーを確認できないため適合を保証できません。".into()); false },
        };
        let host_needed = if backend == Backend::Cpu { needed } else { GIB + model.bytes / 6 };
        let host_fit = ram_budget.is_some_and(|b| host_needed <= b);
        if !host_fit { reasons.push(format!("OS・音声認識の予約後にRAM約{}も必要です。", size(host_needed))); }
        let disk_needed = disk_peak_bytes(model);
        let disk_fit = hardware.disk_free_bytes.is_some_and(|free| free >= disk_needed);
        if !disk_fit { reasons.push(format!("翻訳モデルの準備には、保存先に最低約{}の空きが必要です。音声認識と読み上げは追加で容量を確認します。", size(disk_needed))); }
        if model.quantization == "Q2_K" { reasons.push("Q2_Kは容量優先の手動候補です。公式評価で一部の翻訳精度低下があり、自動推奨しません。".into()); }
        if model.parameters_b >= 31.0 { reasons.push("31Bは待ち時間が長くなる可能性があります。リアルタイム用途の既定にはしません。".into()); }
        if !arch_ok { reasons.push("この配布ランタイムはWindows x64向けです。OS・CPUアーキテクチャを確認してください。".into()); }
        let compatible = model.backends.contains(&backend);
        Alternative { model: model.clone(), fits: memory_fit && host_fit && disk_fit && arch_ok && compatible, reasons }
    }).collect();
    let selected = alternatives
        .iter()
        .filter(|a| a.fits && a.model.parameters_b <= target && a.model.quantization != "Q2_K")
        .max_by(|a, b| {
            a.model
                .parameters_b
                .total_cmp(&b.model.parameters_b)
                .then(quant_rank(&a.model.quantization).cmp(&quant_rank(&b.model.quantization)))
        })
        .map(|a| a.model.clone())
        .unwrap_or_else(|| minimum.clone());
    let chosen = alternatives
        .iter()
        .find(|a| a.model.id == selected.id)
        .unwrap();
    let feasible = chosen.fits;
    let mut reasons = vec![
        "llama.cpp CUDA12.4はドライバー551.61以上、同梱Qwen用PyTorch CUDA12.8は570.65以上を別々に確認します。音声側を確認できない場合はWhisper CPUです。".into(),
        "実測ベンチマークでの最適値ではなく、現在の空き容量と応答速度を考慮した初回導入の目安です。".into(),
        format!("空きRAMからOS予備{}と{}用{}を予約しています。", size(os_reserve), if asr_cuda { "Qwen3-ASR CUDA" } else { "Whisper base CPU" }, size(asr_reserve)),
        "推論見積もりは重み110%＋実行用512 MiB＋2スロット分のKV余裕です。長文や他アプリの使用量で増えます。".into(),
        "翻訳モデルの準備は重み2倍の一時領域＋実行環境4 GiB＋予備2 GiBを見込んでいます。音声認識と読み上げは別に空き容量を確認します。".into(),
    ];
    if backend != Backend::Cpu {
        if let Some((gpu, _, free)) = gpu {
            reasons.push(format!(
                "{}: 空き{}から表示用1 GiB{}を予約して{}を利用します（{}）。",
                gpu.name,
                size(free),
                if asr_cuda { "と音声認識2 GiB" } else { "" },
                backend.as_str(),
                gpu.memory_telemetry
            ));
        }
    } else {
        reasons.push("確認できた空き専用VRAMと対応ランタイムの条件を満たすGPUがないためCPUを選びました。共有GPUメモリーはVRAMとして加算しません。".into());
        warnings.push("CPU音声認識・翻訳は処理が追いつかない場合があります。字幕取得または短い文章から試してください。".into());
    }
    reasons.push(match policy {
        Policy::Realtime => "リアルタイム優先：確認済みGPUの空きが16 GiB以上なら最大8B、それ未満なら最大2B、CPUは0.8Bを基本にします。速度の実測保証ではありません。",
        Policy::Balanced => "バランス優先：GPUでは最大8B、CPUでは最大2Bを候補にし、Q4_K_Mを優先します。",
        Policy::Quality => "品質優先：容量内の大きなモデルを候補にし、同じ規模ならQ4_K_M、次にQ3_K_Mを選びます。",
    }.into());
    reasons.extend(chosen.reasons.clone());
    if !feasible {
        warnings.push("安全な容量条件を確認できていません。推奨名の表示だけではダウンロード・起動を許可しません。".into());
    }
    if !known {
        warnings.push(
            "RAMまたはディスクの空き容量が不明です。不明な容量は十分にあるものとして扱っていません。".into(),
        );
    }
    alternatives.sort_by(|a, b| {
        a.model
            .parameters_b
            .total_cmp(&b.model.parameters_b)
            .then(a.model.bytes.cmp(&b.model.bytes))
    });
    Recommendation {
        selected,
        backend,
        asr_backend: if asr_cuda { "qwen" } else { "whisper" }.into(),
        asr_device: if asr_cuda { "cuda" } else { "cpu" }.into(),
        gpu_layers: if backend == Backend::Cpu { 0 } else { 99 },
        reasons,
        alternatives,
        memory_budget_bytes: budget,
        warnings,
        feasible,
    }
}

fn qwen_cuda_driver_supported(driver: &str) -> bool {
    let mut parts = driver.split('.');
    let major = parts.next().and_then(|s| s.parse::<u32>().ok());
    let minor = parts.next().and_then(|s| s.parse::<u32>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if (major, minor) >= (570, 65))
}

fn quant_rank(quantization: &str) -> u8 {
    match quantization {
        "Q4_K_M" => 3,
        "Q3_K_M" => 2,
        "Q4_0" => 1,
        _ => 0,
    }
}
/// Two bounded inference slots; deliberately includes allocator/runtime headroom.
pub fn inference_bytes(model: &Model) -> u64 {
    let kv_mib = if model.parameters_b <= 1.0 {
        384
    } else if model.parameters_b <= 2.0 {
        640
    } else if model.parameters_b <= 8.0 {
        1536
    } else {
        3584
    };
    model
        .bytes
        .saturating_mul(11)
        .div_ceil(10)
        .saturating_add((512 + kv_mib) * MIB)
}
pub fn disk_peak_bytes(model: &Model) -> u64 {
    model.bytes.saturating_mul(2).saturating_add(6 * GIB)
}
fn size(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / GIB as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Gpu;
    fn fixture(ram: u64) -> Hardware {
        Hardware {
            os_arch: "windows-x86_64".into(),
            cpu_logical: 8,
            ram_total_bytes: Some(ram * GIB),
            ram_available_bytes: Some(ram * GIB * 3 / 4),
            disk_free_bytes: Some(200 * GIB),
            disk_target: "test".into(),
            gpus: vec![],
            warnings: vec![],
        }
    }
    fn gpu(vram: u64, free: u64, vendor: &str, cuda: bool, vulkan: bool) -> Gpu {
        Gpu {
            name: "fixture".into(),
            vendor: vendor.into(),
            dedicated_vram_bytes: Some(vram * GIB),
            free_vram_bytes: Some(free * GIB),
            driver_version: None,
            cuda_eligible: cuda,
            vulkan_eligible: vulkan,
            is_software: false,
            memory_telemetry: "test".into(),
        }
    }
    #[test]
    fn catalog_has_exact_verified_metadata_and_no_bf16() {
        let catalog = bundled().unwrap();
        assert_eq!(catalog.models.len(), 14);
        for family in [0.8, 2.0, 8.0, 31.0] {
            assert!(
                catalog
                    .models
                    .iter()
                    .any(|m| m.parameters_b == family && m.quantization == "Q4_K_M")
            );
        }
        assert!(
            !catalog
                .models
                .iter()
                .any(|m| m.filename.to_lowercase().contains("bf16"))
        );
    }
    #[test]
    fn cpu_ram_and_policy_fixtures_are_conservative() {
        for ram in [4, 8, 16, 32] {
            let rec = recommend(&fixture(ram), Policy::Realtime);
            assert_eq!(rec.backend, Backend::Cpu);
            assert_eq!(rec.selected.parameters_b, 0.8);
            assert!(rec.feasible);
            assert_eq!(rec.asr_backend, "whisper");
            assert_eq!(rec.gpu_layers, 0);
        }
        assert_eq!(
            recommend(&fixture(16), Policy::Balanced)
                .selected
                .parameters_b,
            2.0
        );
        assert_eq!(
            recommend(&fixture(32), Policy::Quality)
                .selected
                .parameters_b,
            8.0
        );
    }
    #[test]
    fn gpu_capacity_fixtures_do_not_select_31b_for_realtime() {
        for vram in [4, 8, 12, 24, 48] {
            let mut hw = fixture(64);
            hw.gpus.push(gpu(vram, vram - 1, "nvidia", true, false));
            let rec = recommend(&hw, Policy::Realtime);
            assert!(rec.selected.parameters_b <= if vram > 16 { 8.0 } else { 2.0 });
            assert_ne!(rec.selected.quantization, "Q2_K");
            assert_eq!(rec.gpu_layers, 99);
            assert!(rec.feasible);
            if vram >= 24 {
                assert_eq!(rec.selected.parameters_b, 8.0);
            }
            if vram == 48 {
                assert_eq!(recommend(&hw, Policy::Quality).selected.parameters_b, 31.0);
            }
        }
    }
    #[test]
    fn busy_unknown_shared_and_unverified_gpu_are_not_large_gpu_budgets() {
        let mut hw = fixture(16);
        hw.gpus.push(gpu(48, 1, "nvidia", true, false));
        assert_eq!(recommend(&hw, Policy::Realtime).backend, Backend::Cpu);
        hw.gpus[0].free_vram_bytes = None;
        assert_eq!(recommend(&hw, Policy::Realtime).backend, Backend::Cpu);
        hw.gpus[0] = gpu(0, 32, "intel", false, true);
        assert_eq!(recommend(&hw, Policy::Realtime).backend, Backend::Cpu);
        for vendor in ["amd", "intel"] {
            hw.gpus[0] = gpu(12, 11, vendor, false, false);
            assert_eq!(recommend(&hw, Policy::Realtime).backend, Backend::Cpu);
            hw.gpus[0].vulkan_eligible = true;
            let rec = recommend(&hw, Policy::Balanced);
            assert_eq!(rec.backend, Backend::Vulkan);
            assert_eq!(rec.asr_device, "cpu");
        }
        hw.ram_available_bytes = None;
        let rec = recommend(&hw, Policy::Quality);
        assert_eq!(rec.selected.parameters_b, 0.8);
        assert!(!rec.feasible);
        hw.ram_available_bytes = Some(12 * GIB);
        hw.disk_free_bytes = Some(GIB);
        assert!(!recommend(&hw, Policy::Realtime).feasible);
    }

    #[test]
    fn llama_and_qwen_cuda_driver_floors_are_separate() {
        let mut hw = fixture(32);
        let mut card = gpu(24, 23, "nvidia", true, false);
        card.driver_version = Some("551.61".into());
        hw.gpus.push(card);
        let rec = recommend(&hw, Policy::Realtime);
        assert_eq!(rec.backend, Backend::Cuda);
        assert_eq!(rec.asr_backend, "whisper");
        assert_eq!(rec.asr_device, "cpu");
        hw.gpus[0].driver_version = Some("570.65".into());
        let rec = recommend(&hw, Policy::Realtime);
        assert_eq!(rec.asr_backend, "qwen");
        assert_eq!(rec.asr_device, "cuda");
        hw.gpus[0].free_vram_bytes = Some(5 * GIB);
        assert_eq!(recommend(&hw, Policy::Realtime).asr_device, "cpu");
        hw.gpus[0].free_vram_bytes = Some(23 * GIB);
        hw.gpus[0].driver_version = None;
        assert_eq!(recommend(&hw, Policy::Realtime).asr_device, "cpu");
    }
}
