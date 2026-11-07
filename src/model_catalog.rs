//! Verified model metadata and conservative capacity heuristics, not benchmarks.
use crate::hardware::{Gpu, Hardware};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const GIB: u64 = 1_073_741_824;
const MIB: u64 = 1_048_576;
/// The CPU runs only the smallest size at a usable speed. llama.cpp does not use
/// an NPU, so a PC with only a CPU or an NPU is a CPU PC here.
const CPU_MAX_PARAMETERS_B: f32 = 1.0;

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

/// The first condition that keeps a model from running on this PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    /// No usable GPU, and this size is too large for the CPU.
    NeedsGpu,
    /// GPU memory is short by this many bytes.
    GpuMemory(u64),
    /// System memory is short by this many bytes.
    Memory(u64),
    /// Free disk space is short by this many bytes.
    Disk(u64),
    /// A measurement is missing, or this platform is not supported.
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alternative {
    pub model: Model,
    /// Always `blocker.is_none()`.
    pub fits: bool,
    pub reasons: Vec<String>,
    pub blocker: Option<Blocker>,
}

/// The GPU a plan is built around, and the memory the fit check keeps back on it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GpuPlan {
    pub name: String,
    /// Dedicated VRAM.
    pub total_bytes: u64,
    /// Free VRAM as measured, before counting anything a model switch releases.
    pub free_bytes: u64,
    /// Kept back for the display (and CUDA speech recognition).
    pub reserve_bytes: u64,
    /// "nvidia-smi" measures the whole card. "dxgi-local-budget" is an estimate.
    pub telemetry: String,
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
    /// A GPU large enough to hold the smallest model, even when it is busy now.
    /// `None` means this is a CPU PC.
    pub gpu: Option<GpuPlan>,
    pub warnings: Vec<String>,
    /// False also means an essential capacity measurement is unknown. The caller
    /// must obtain confirmation/measurements before downloading or launching it.
    pub feasible: bool,
}

/// The GPU `recommend` plans around.
struct Chosen<'a> {
    gpu: &'a Gpu,
    backend: Backend,
    total: u64,
    /// Measured free memory.
    free: u64,
    /// Free memory the plan counts on: the measurement plus what a switch releases.
    available: u64,
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
    recommend_replacing(hardware, policy, 0)
}

/// [`recommend`] for a PC where this app has a model loaded on the GPU.
/// Choosing another model replaces it, so `released` (the loaded model's GPU
/// memory) counts as free when judging what fits.
pub fn recommend_replacing(hardware: &Hardware, policy: Policy, released: u64) -> Recommendation {
    // Compile-time bundled data is validated by tests, never replaced with
    // invented hashes or an unpinned model when parsing fails.
    let catalog = bundled().expect("invalid bundled, build-time-verified model catalog");
    let mut warnings = hardware.warnings.clone();
    let chosen = hardware
        .gpus
        .iter()
        .filter(|gpu| !gpu.is_software)
        .filter_map(|gpu| {
            let total = gpu.dedicated_vram_bytes?;
            let free = gpu.free_vram_bytes?.min(total);
            let backend = if gpu.cuda_eligible {
                Backend::Cuda
            } else if gpu.vulkan_eligible {
                Backend::Vulkan
            } else {
                return None;
            };
            Some(Chosen {
                gpu,
                backend,
                total,
                free,
                available: free.saturating_add(released).min(total),
            })
        })
        .max_by_key(|c| c.free);
    let minimum = catalog
        .models
        .iter()
        .find(|m| m.parameters_b == 0.8 && m.quantization == "Q4_K_M")
        .unwrap();
    let mut backend = chosen.as_ref().map_or(Backend::Cpu, |c| c.backend);
    let mut asr_cuda = chosen.as_ref().is_some_and(|c| {
        c.backend == Backend::Cuda
            && c.available >= 6 * GIB
            && c.gpu.vendor == "nvidia"
            && c.gpu
                .driver_version
                .as_deref()
                .is_some_and(qwen_cuda_driver_supported)
    });
    let mut gpu_budget = chosen.as_ref().map(|c| {
        c.available
            .saturating_sub(GIB + if asr_cuda { 2 * GIB } else { 0 })
    });
    if backend != Backend::Cpu && gpu_budget.is_none_or(|budget| budget < inference_bytes(minimum))
    {
        warnings.push("GPUの空き容量から表示・音声認識分を引くと小型モデルも収まらないため、CPUを候補にしました。".into());
        backend = Backend::Cpu;
        asr_cuda = false;
        gpu_budget = None;
    }
    // A GPU that could hold the smallest model when idle is a GPU, even if it is
    // busy now. A small integrated one is not, so that PC reads as a CPU PC.
    let reserve = GIB + if asr_cuda { 2 * GIB } else { 0 };
    let capable = chosen
        .as_ref()
        .filter(|c| c.total.saturating_sub(GIB) >= inference_bytes(minimum));
    let gpu_room = capable.map(|c| c.available.saturating_sub(reserve));
    let gpu = capable.map(|c| GpuPlan {
        name: c.gpu.name.clone(),
        total_bytes: c.total,
        free_bytes: c.free,
        reserve_bytes: reserve,
        telemetry: c.gpu.memory_telemetry.clone(),
    });
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
            (_, Backend::Cpu) => CPU_MAX_PARAMETERS_B,
            (Policy::Realtime, _) => {
                if chosen.as_ref().is_some_and(|c| c.available >= 16 * GIB) {
                    8.0
                } else {
                    2.0
                }
            }
            (Policy::Balanced, _) => 8.0,
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
        let too_large_for_cpu = backend == Backend::Cpu && model.parameters_b > CPU_MAX_PARAMETERS_B;
        if too_large_for_cpu { reasons.push("CPUで動かせるのは0.8Bまでです。2B以上にはGPUが必要です。".into()); }
        let compatible = model.backends.contains(&backend);
        let blocker = if too_large_for_cpu {
            Some(gpu_room.map_or(Blocker::NeedsGpu, |room| Blocker::GpuMemory(needed.saturating_sub(room))))
        } else if !memory_fit {
            Some(budget.map_or(Blocker::Unavailable, |b| {
                let missing = needed.saturating_sub(b);
                if backend == Backend::Cpu { Blocker::Memory(missing) } else { Blocker::GpuMemory(missing) }
            }))
        } else if !host_fit {
            Some(ram_budget.map_or(Blocker::Unavailable, |b| Blocker::Memory(host_needed.saturating_sub(b))))
        } else if !disk_fit {
            Some(hardware.disk_free_bytes.map_or(Blocker::Unavailable, |free| Blocker::Disk(disk_needed.saturating_sub(free))))
        } else if !arch_ok || !compatible {
            Some(Blocker::Unavailable)
        } else {
            None
        };
        Alternative { model: model.clone(), fits: blocker.is_none(), reasons, blocker }
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
    let picked = alternatives
        .iter()
        .find(|a| a.model.id == selected.id)
        .unwrap();
    let feasible = picked.fits;
    let mut reasons = vec![
        "llama.cpp CUDA12.4はドライバー551.61以上、同梱Qwen用PyTorch CUDA12.8は570.65以上を別々に確認します。音声側を確認できない場合はWhisper CPUです。".into(),
        "実測ベンチマークでの最適値ではなく、現在の空き容量と応答速度を考慮した初回導入の目安です。".into(),
        format!("空きRAMからOS予備{}と{}用{}を予約しています。", size(os_reserve), if asr_cuda { "Qwen3-ASR CUDA" } else { "Whisper base CPU" }, size(asr_reserve)),
        "推論見積もりは重み110%＋実行用512 MiB＋2スロット分のKV余裕です。長文や他アプリの使用量で増えます。".into(),
        "翻訳モデルの準備は重み2倍の一時領域＋実行環境4 GiB＋予備2 GiBを見込んでいます。音声認識と読み上げは別に空き容量を確認します。".into(),
    ];
    if backend != Backend::Cpu {
        if let Some(c) = &chosen {
            reasons.push(format!(
                "{}: 空き{}から表示用1 GiB{}を予約して{}を利用します（{}）。",
                c.gpu.name,
                size(c.available),
                if asr_cuda { "と音声認識2 GiB" } else { "" },
                backend.as_str(),
                c.gpu.memory_telemetry
            ));
        }
    } else {
        reasons.push("確認できた空き専用VRAMと対応ランタイムの条件を満たすGPUがないためCPUを選びました。共有GPUメモリーはVRAMとして加算しません。".into());
        warnings.push("CPU音声認識・翻訳は処理が追いつかない場合があります。字幕取得または短い文章から試してください。".into());
    }
    reasons.push(match policy {
        Policy::Realtime => "リアルタイム優先：確認済みGPUの空きが16 GiB以上なら最大8B、それ未満なら最大2Bを基本にします。CPUでは0.8Bだけです。速度の実測保証ではありません。",
        Policy::Balanced => "バランス優先：GPUでは最大8Bを候補にし、Q4_K_Mを優先します。CPUでは0.8Bだけです。",
        Policy::Quality => "品質優先：GPUで容量内の大きなモデルを候補にし、同じ規模ならQ4_K_M、次にQ3_K_Mを選びます。CPUでは0.8Bだけです。",
    }.into());
    reasons.extend(picked.reasons.clone());
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
        gpu,
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

/// What the selected model does to the GPU's memory in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryChange {
    /// It is the model already loaded, so the measurement includes it.
    Loaded,
    /// More memory in use than now.
    Adds(u64),
    /// It replaces a larger loaded model, so less memory is in use.
    Frees(u64),
}

/// A GPU memory bar: `used` of `total`, and whether the selected model fits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuMeter {
    pub total: u64,
    pub used: u64,
    /// `total` minus the reserve the fit check keeps back.
    pub usable: u64,
    pub change: MemoryChange,
    /// Bytes beyond `usable` once the model is loaded. Zero when it fits.
    pub missing: u64,
}

impl GpuMeter {
    pub fn fits(&self) -> bool {
        self.missing == 0
    }
}

/// The arithmetic of the fit check, drawn as a bar: a model fits when the memory
/// in use, less the `held` bytes of a loaded model it replaces, plus its own
/// `estimate`, stays within `total - reserve`. With nothing loaded this is
/// `estimate <= free - reserve`, which is what [`recommend`] tests.
pub fn gpu_meter(
    total: u64,
    free: u64,
    reserve: u64,
    held: u64,
    estimate: u64,
    loaded: bool,
) -> GpuMeter {
    let used = total.saturating_sub(free.min(total));
    let usable = total.saturating_sub(reserve);
    if loaded {
        return GpuMeter {
            total,
            used,
            usable,
            change: MemoryChange::Loaded,
            missing: 0,
        };
    }
    let after = used.saturating_sub(held).saturating_add(estimate);
    GpuMeter {
        total,
        used,
        usable,
        change: if estimate >= held {
            MemoryChange::Adds(estimate - held)
        } else {
            MemoryChange::Frees(held - estimate)
        },
        missing: after.saturating_sub(usable),
    }
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
        for policy in [Policy::Realtime, Policy::Balanced, Policy::Quality] {
            assert_eq!(recommend(&fixture(32), policy).selected.parameters_b, 0.8);
        }
    }
    fn card(total: u64, used: u64) -> Gpu {
        Gpu {
            name: "fixture".into(),
            vendor: "nvidia".into(),
            dedicated_vram_bytes: Some(total),
            free_vram_bytes: Some(total - used),
            driver_version: None,
            cuda_eligible: true,
            vulkan_eligible: false,
            is_software: false,
            memory_telemetry: "test".into(),
        }
    }
    fn alt<'a>(rec: &'a Recommendation, id: &str) -> &'a Alternative {
        rec.alternatives.iter().find(|a| a.model.id == id).unwrap()
    }
    fn missing_on_gpu(rec: &Recommendation, id: &str) -> Option<u64> {
        match alt(rec, id).blocker {
            Some(Blocker::GpuMemory(missing)) => Some(missing),
            _ => None,
        }
    }
    #[test]
    fn a_cpu_pc_can_choose_only_the_smallest_size_under_every_policy() {
        let hw = fixture(128);
        for policy in [Policy::Realtime, Policy::Balanced, Policy::Quality] {
            let rec = recommend(&hw, policy);
            assert!(rec.gpu.is_none());
            assert_eq!(rec.selected.id, "plamo3-0.8b-q4_k_m");
            for a in &rec.alternatives {
                if a.model.parameters_b <= 1.0 {
                    assert!(a.fits && a.blocker.is_none(), "{}", a.model.id);
                } else {
                    assert!(!a.fits, "{}", a.model.id);
                    assert_eq!(a.blocker, Some(Blocker::NeedsGpu), "{}", a.model.id);
                }
            }
            serde_json::to_value(&rec).expect("jimoto-setup prints this as JSON");
        }
    }
    #[test]
    fn an_npu_or_software_adapter_or_tiny_integrated_gpu_is_a_cpu_pc() {
        let mut software = card(0, 0);
        software.is_software = true;
        let mut npu = card(0, 0);
        npu.cuda_eligible = false;
        npu.free_vram_bytes = None;
        let integrated = card(GIB / 2, 0);
        for adapter in [software, npu, integrated] {
            let mut hw = fixture(64);
            hw.gpus.push(adapter);
            let rec = recommend(&hw, Policy::Realtime);
            assert!(rec.gpu.is_none());
            assert_eq!(rec.backend, Backend::Cpu);
            assert_eq!(
                alt(&rec, "plamo3-2b-q4_k_m").blocker,
                Some(Blocker::NeedsGpu)
            );
            assert!(alt(&rec, "plamo3-0.8b-q4_k_m").fits);
        }
    }
    #[test]
    fn a_busy_gpu_lacks_memory_it_is_not_a_missing_gpu() {
        let mut hw = fixture(64);
        hw.gpus.push(card(8 * GIB, 13 * GIB / 2));
        let rec = recommend(&hw, Policy::Realtime);
        let plan = rec
            .gpu
            .as_ref()
            .expect("an 8 GiB card is a GPU even when full");
        assert_eq!(
            (plan.total_bytes, plan.free_bytes, plan.reserve_bytes),
            (8 * GIB, 3 * GIB / 2, GIB)
        );
        assert_eq!(rec.backend, Backend::Cpu);
        assert_eq!(rec.selected.id, "plamo3-0.8b-q4_k_m");
        let room = 3 * GIB / 2 - GIB;
        for id in ["plamo3-2b-q4_k_m", "plamo3-8b-q4_k_m", "plamo3-31b-q4_k_m"] {
            let a = alt(&rec, id);
            assert_eq!(
                a.blocker,
                Some(Blocker::GpuMemory(inference_bytes(&a.model) - room)),
                "{id}"
            );
        }
    }
    #[test]
    fn sizes_that_do_not_fit_the_gpu_say_how_much_memory_is_missing() {
        let mut hw = fixture(64);
        hw.gpus.push(card(8 * GIB, 3 * GIB));
        let rec = recommend(&hw, Policy::Realtime);
        assert_eq!(rec.backend, Backend::Cuda);
        assert_eq!(rec.selected.id, "plamo3-2b-q4_k_m");
        let room = 8 * GIB - 3 * GIB - GIB;
        assert!(alt(&rec, "plamo3-0.8b-q4_k_m").fits && alt(&rec, "plamo3-2b-q4_k_m").fits);
        for id in ["plamo3-8b-q4_k_m", "plamo3-31b-q4_k_m"] {
            let a = alt(&rec, id);
            assert!(!a.fits, "{id}");
            assert_eq!(
                missing_on_gpu(&rec, id),
                Some(inference_bytes(&a.model) - room),
                "{id}"
            );
        }
        serde_json::to_value(&rec).expect("jimoto-setup prints this as JSON");
    }
    #[test]
    fn host_memory_and_disk_shortfalls_are_named_too() {
        let mut hw = fixture(64);
        hw.gpus.push(card(24 * GIB, 0));
        hw.disk_free_bytes = Some(8 * GIB);
        let rec = recommend(&hw, Policy::Realtime);
        let model = &alt(&rec, "plamo3-8b-q4_k_m").model;
        assert_eq!(
            alt(&rec, "plamo3-8b-q4_k_m").blocker,
            Some(Blocker::Disk(disk_peak_bytes(model) - 8 * GIB))
        );
        hw.disk_free_bytes = Some(200 * GIB);
        hw.ram_available_bytes = Some(2 * GIB);
        assert!(matches!(
            alt(&recommend(&hw, Policy::Realtime), "plamo3-8b-q4_k_m").blocker,
            Some(Blocker::Memory(_))
        ));
    }
    #[test]
    fn switching_models_counts_the_loaded_models_memory_as_free() {
        let loaded = inference_bytes(
            &alt(
                &recommend(&fixture(64), Policy::Realtime),
                "plamo3-2b-q4_k_m",
            )
            .model,
        );
        let mut hw = fixture(64);
        hw.gpus.push(card(12 * GIB, 2 * GIB + loaded));
        let kept = recommend(&hw, Policy::Realtime);
        assert!(missing_on_gpu(&kept, "plamo3-8b-q4_k_m").is_some());
        let switching = recommend_replacing(&hw, Policy::Realtime, loaded);
        assert!(alt(&switching, "plamo3-8b-q4_k_m").fits);
        assert_eq!(
            switching.gpu.as_ref().unwrap().free_bytes,
            12 * GIB - 2 * GIB - loaded,
            "the plan still reports the measurement"
        );
    }
    #[test]
    fn the_recommendation_does_not_shrink_because_the_chosen_model_is_loaded() {
        let eight = inference_bytes(
            &alt(
                &recommend(&fixture(64), Policy::Realtime),
                "plamo3-8b-q4_k_m",
            )
            .model,
        );
        let mut hw = fixture(64);
        hw.gpus.push(card(24 * GIB, 5 * GIB + eight));
        assert_eq!(recommend(&hw, Policy::Realtime).selected.parameters_b, 2.0);
        assert_eq!(
            recommend_replacing(&hw, Policy::Realtime, eight)
                .selected
                .parameters_b,
            8.0
        );
    }
    #[test]
    fn meter_adds_the_estimate_to_what_is_in_use() {
        let estimate = 7 * GIB + GIB * 2 / 5;
        let meter = gpu_meter(24 * GIB, 19 * GIB, GIB, 0, estimate, false);
        assert_eq!(meter.total, 24 * GIB);
        assert_eq!(meter.used, 5 * GIB);
        assert_eq!(meter.usable, 23 * GIB);
        assert_eq!(meter.change, MemoryChange::Adds(estimate));
        assert!(meter.fits() && meter.missing == 0);
    }
    #[test]
    fn meter_flags_a_model_that_exceeds_the_usable_memory_by_how_much() {
        let meter = gpu_meter(8 * GIB, 3 * GIB / 2, GIB, 0, 3 * GIB / 2, false);
        assert_eq!(meter.used, 13 * GIB / 2);
        assert!(!meter.fits());
        assert_eq!(meter.missing, GIB);
        let exact = gpu_meter(8 * GIB, 5 * GIB / 2, GIB, 0, 3 * GIB / 2, false);
        assert!(exact.fits(), "using exactly the usable amount still fits");
    }
    #[test]
    fn meter_for_the_loaded_model_adds_nothing() {
        let meter = gpu_meter(8 * GIB, GIB, GIB, 5 * GIB, 30 * GIB, true);
        assert_eq!(meter.change, MemoryChange::Loaded);
        assert_eq!(meter.used, 7 * GIB);
        assert!(meter.fits());
    }
    #[test]
    fn meter_counts_a_replaced_model_once() {
        let used_by_others = 2 * GIB;
        let held = 3 * GIB;
        let estimate = 7 * GIB;
        let free = 12 * GIB - used_by_others - held;
        let replacing = gpu_meter(12 * GIB, free, GIB, held, estimate, false);
        assert_eq!(replacing.change, MemoryChange::Adds(estimate - held));
        assert!(replacing.fits());
        let stacking = gpu_meter(12 * GIB, free, GIB, 0, estimate, false);
        assert_eq!(
            stacking.missing,
            used_by_others + held + estimate - 11 * GIB
        );
        let smaller = gpu_meter(12 * GIB, free, GIB, held, GIB, false);
        assert_eq!(smaller.change, MemoryChange::Frees(held - GIB));
    }
    #[test]
    fn meter_and_recommendation_agree_on_what_fits() {
        let sizes = [
            "plamo3-0.8b-q4_k_m",
            "plamo3-2b-q4_k_m",
            "plamo3-8b-q4_k_m",
            "plamo3-31b-q4_k_m",
        ];
        let loaded = inference_bytes(
            &alt(
                &recommend(&fixture(64), Policy::Realtime),
                "plamo3-2b-q4_k_m",
            )
            .model,
        );
        for total in [8, 12, 16, 24, 48] {
            for used in [0, 2 * GIB, 9 * GIB / 2, 13 * GIB / 2, 20 * GIB] {
                for held in [0, loaded] {
                    if used >= total * GIB || used < held {
                        continue;
                    }
                    let mut hw = fixture(64);
                    hw.gpus.push(card(total * GIB, used));
                    let rec = recommend_replacing(&hw, Policy::Realtime, held);
                    let plan = rec.gpu.as_ref().unwrap();
                    for id in sizes {
                        let a = alt(&rec, id);
                        let meter = gpu_meter(
                            plan.total_bytes,
                            plan.free_bytes,
                            plan.reserve_bytes,
                            held,
                            inference_bytes(&a.model),
                            false,
                        );
                        let label = format!("{id} on {total} GiB with {used} in use, {held} held");
                        match a.blocker {
                            Some(Blocker::GpuMemory(missing)) => {
                                assert_eq!(missing, meter.missing, "{label}")
                            }
                            None if rec.backend != Backend::Cpu => {
                                assert!(meter.fits(), "{label}")
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
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
