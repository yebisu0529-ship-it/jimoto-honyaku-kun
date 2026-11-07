//! Local, read-only capacity detection. No identifiers, network calls, or uploads.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hardware {
    pub os_arch: String,
    pub cpu_logical: usize,
    pub ram_total_bytes: Option<u64>,
    pub ram_available_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
    pub disk_target: String,
    pub gpus: Vec<Gpu>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    pub dedicated_vram_bytes: Option<u64>,
    pub free_vram_bytes: Option<u64>,
    pub driver_version: Option<String>,
    pub cuda_eligible: bool,
    pub vulkan_eligible: bool,
    pub is_software: bool,
    /// "nvidia-smi", "dxgi-local-budget", or "unknown"; DXGI is an OS budget,
    /// not a measurement of the entire physical card's unused memory.
    pub memory_telemetry: String,
}

pub fn detect(target: &Path) -> Result<Hardware> {
    let ancestor = existing_ancestor(target)?;
    let mut hardware = Hardware {
        os_arch: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        cpu_logical: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        ram_total_bytes: None,
        ram_available_bytes: None,
        disk_free_bytes: None,
        disk_target: ancestor.display().to_string(),
        gpus: Vec::new(),
        warnings: Vec::new(),
    };
    #[cfg(windows)]
    windows_detect(&ancestor, &mut hardware);
    #[cfg(not(windows))]
    hardware.warnings.push(
        "この配布版の詳細な容量検出はWindows専用です。未取得の容量は推定値で補っていません。"
            .into(),
    );
    Ok(hardware)
}

fn existing_ancestor(target: &Path) -> Result<PathBuf> {
    let mut path = if target.is_absolute() {
        target.to_path_buf()
    } else {
        std::env::current_dir()?.join(target)
    };
    while !path.exists() {
        anyhow::ensure!(path.pop(), "保存先の既存の親フォルダーが見つかりません");
    }
    if path.is_file() {
        path.pop();
    }
    path.canonicalize()
        .context("保存先フォルダーを確認できません")
}

#[cfg(any(windows, test))]
fn cuda_124_driver_supported(driver: &str) -> bool {
    let mut parts = driver.trim().split('.');
    let Some(major) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
        return false;
    };
    (major, minor) >= (551, 61)
}

#[cfg(windows)]
fn windows_detect(target: &Path, hardware: &mut Hardware) {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Storage::FileSystem::GetDiskFreeSpaceExW,
            System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        },
        core::PCWSTR,
    };
    let mut memory = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut memory) }.is_ok() {
        hardware.ram_total_bytes = Some(memory.ullTotalPhys);
        hardware.ram_available_bytes = Some(memory.ullAvailPhys);
    } else {
        hardware
            .warnings
            .push("空きRAMを取得できませんでした。自動選択を保守的にしています。".into());
    }
    let wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0;
    if unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.is_ok() {
        hardware.disk_free_bytes = Some(free);
    } else {
        hardware
            .warnings
            .push("保存先ディスクの空き容量を確認できませんでした。".into());
    }
    match dxgi_adapters() {
        Some(adapters) => {
            let vulkan = vulkan_devices();
            for adapter in adapters {
                let normalized = normalized_name(&adapter.name);
                let vulkan_eligible = !adapter.software
                    && vulkan.iter().any(|(vendor, candidate)| {
                        *vendor == adapter.vendor_id && (normalized_name(candidate) == normalized)
                    });
                hardware.gpus.push(Gpu {
                    name: adapter.name,
                    vendor: match adapter.vendor_id {
                        0x10de => "nvidia",
                        0x1002 | 0x1022 => "amd",
                        0x8086 => "intel",
                        _ => "unknown",
                    }
                    .into(),
                    dedicated_vram_bytes: Some(adapter.dedicated),
                    free_vram_bytes: adapter.free,
                    driver_version: None,
                    cuda_eligible: false,
                    vulkan_eligible,
                    is_software: adapter.software,
                    memory_telemetry: if adapter.free.is_some() {
                        "dxgi-local-budget"
                    } else {
                        "unknown"
                    }
                    .into(),
                });
            }
        }
        None => hardware
            .warnings
            .push("DXGIでGPUを確認できませんでした。CPUを候補にします。".into()),
    }
    let rows = nvidia_smi();
    for (index, row) in match_smi(&hardware.gpus, &rows) {
        let gpu = &mut hardware.gpus[index];
        gpu.driver_version = Some(row.driver.clone());
        gpu.cuda_eligible = !gpu.is_software && cuda_124_driver_supported(&row.driver);
        apply_smi_memory(gpu, row);
    }
    if hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == "nvidia" && !gpu.cuda_eligible)
    {
        hardware.warnings.push("CUDA 12.4用Windowsドライバー551.61以上を確認できないGPUはCUDA自動選択から除外しました。".into());
    }
}

/// Update the free memory of GPUs that [`detect`] found, cheaply enough to repeat
/// every few seconds. Names, eligibility and capacity stay as detected, and a
/// reading that fails leaves the last good value in place.
pub fn refresh_gpu_memory(gpus: &mut [Gpu]) {
    #[cfg(windows)]
    {
        if let Some(adapters) = dxgi_adapters() {
            for (gpu, adapter) in gpus.iter_mut().zip(adapters) {
                // nvidia-smi measures the whole card. Do not trade it for the OS budget.
                if gpu.name == adapter.name
                    && gpu.memory_telemetry != "nvidia-smi"
                    && let Some(free) = adapter.free
                {
                    gpu.free_vram_bytes = Some(free);
                    gpu.memory_telemetry = "dxgi-local-budget".into();
                }
            }
        }
        if gpus.iter().any(|gpu| gpu.vendor == "nvidia") {
            let rows = nvidia_smi();
            for (index, row) in match_smi(gpus, &rows) {
                apply_smi_memory(&mut gpus[index], row);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = gpus;
}

#[cfg(any(windows, test))]
fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

/// One graphics adapter as DXGI lists it.
#[cfg(windows)]
struct Adapter {
    name: String,
    vendor_id: u32,
    dedicated: u64,
    software: bool,
    /// What the OS budget lets this process still allocate. Not the card's whole free memory.
    free: Option<u64>,
}

/// Every adapter DXGI lists, in its enumeration order, or `None` without DXGI.
#[cfg(windows)]
fn dxgi_adapters() -> Option<Vec<Adapter>> {
    use windows::{
        Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
            DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIFactory1,
        },
        core::Interface,
    };
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    let mut adapters = Vec::new();
    for index in 0..32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let dedicated = desc.DedicatedVideoMemory as u64;
        let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
        let mut free = None;
        if !software
            && dedicated > 0
            && let Ok(adapter3) = adapter.cast::<IDXGIAdapter3>()
        {
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if unsafe {
                adapter3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
            }
            .is_ok()
            {
                free = Some(info.Budget.saturating_sub(info.CurrentUsage).min(dedicated));
            }
        }
        adapters.push(Adapter {
            name: String::from_utf16_lossy(&desc.Description[..end]),
            vendor_id: desc.VendorId,
            dedicated,
            software,
            free,
        });
    }
    Some(adapters)
}

/// One row of `nvidia-smi --query-gpu=name,memory.total,memory.free,driver_version`.
#[cfg(any(windows, test))]
struct SmiRow {
    name: String,
    total: Option<u64>,
    free: Option<u64>,
    driver: String,
}

#[cfg(any(windows, test))]
fn parse_nvidia_smi(output: &str) -> Vec<SmiRow> {
    let mib = |text: &str| {
        text.parse::<u64>()
            .ok()
            .and_then(|n| n.checked_mul(1_048_576))
    };
    output
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            let [name, total, free, driver] = fields[..] else {
                return None;
            };
            Some(SmiRow {
                name: name.into(),
                total: mib(total),
                free: mib(free),
                driver: driver.into(),
            })
        })
        .collect()
}

#[cfg(windows)]
fn nvidia_smi() -> Vec<SmiRow> {
    hidden_output(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total,memory.free,driver_version",
            "--format=csv,noheader,nounits",
        ],
    )
    .map(|output| parse_nvidia_smi(&output))
    .unwrap_or_default()
}

/// Pair each NVIDIA adapter with the row that names it. Identical cards pair in
/// listing order, each used once.
#[cfg(any(windows, test))]
fn match_smi<'a>(gpus: &[Gpu], rows: &'a [SmiRow]) -> Vec<(usize, &'a SmiRow)> {
    let mut used = std::collections::HashSet::new();
    rows.iter()
        .filter_map(|row| {
            let (index, _) = gpus.iter().enumerate().find(|(index, gpu)| {
                !used.contains(index)
                    && gpu.vendor == "nvidia"
                    && normalized_name(&gpu.name) == normalized_name(&row.name)
            })?;
            used.insert(index);
            Some((index, row))
        })
        .collect()
}

#[cfg(any(windows, test))]
fn apply_smi_memory(gpu: &mut Gpu, row: &SmiRow) {
    if let (Some(free), Some(total)) = (row.free, row.total) {
        gpu.free_vram_bytes = Some(
            free.min(total)
                .min(gpu.dedicated_vram_bytes.unwrap_or(total)),
        );
        gpu.memory_telemetry = "nvidia-smi".into();
    }
}

#[cfg(windows)]
fn hidden_output(program: &str, args: &[&str]) -> Option<String> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = Command::new(program)
        .args(args)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return child
                    .wait_with_output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok());
            }
            Ok(Some(_)) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => {}
        }
        if start.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Probe the installed Vulkan loader and physical devices, without creating a
/// graphics surface or uploading data. Properties layout follows Vulkan-Headers.
#[cfg(windows)]
fn vulkan_devices() -> Vec<(u32, String)> {
    use std::{
        ffi::{c_char, c_void},
        ptr,
    };
    use windows::{
        Win32::{
            Foundation::FreeLibrary,
            System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
        },
        core::{PCSTR, w},
    };
    #[repr(C)]
    struct App {
        ty: u32,
        next: *const c_void,
        name: *const c_char,
        version: u32,
        engine: *const c_char,
        engine_version: u32,
        api_version: u32,
    }
    #[repr(C)]
    struct Create {
        ty: u32,
        next: *const c_void,
        flags: u32,
        app: *const App,
        layer_count: u32,
        layers: *const *const c_char,
        extension_count: u32,
        extensions: *const *const c_char,
    }
    type Instance = *mut c_void;
    type Device = *mut c_void;
    type CreateFn = unsafe extern "system" fn(*const Create, *const c_void, *mut Instance) -> i32;
    type EnumFn = unsafe extern "system" fn(Instance, *mut u32, *mut Device) -> i32;
    type PropsFn = unsafe extern "system" fn(Device, *mut c_void);
    type DestroyFn = unsafe extern "system" fn(Instance, *const c_void);
    unsafe {
        let Ok(module) = LoadLibraryExW(w!("vulkan-1.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
        else {
            return Vec::new();
        };
        let result = (|| {
            let create: CreateFn = std::mem::transmute(GetProcAddress(
                module,
                PCSTR(c"vkCreateInstance".as_ptr().cast()),
            )?);
            let enumerate: EnumFn = std::mem::transmute(GetProcAddress(
                module,
                PCSTR(c"vkEnumeratePhysicalDevices".as_ptr().cast()),
            )?);
            let properties: PropsFn = std::mem::transmute(GetProcAddress(
                module,
                PCSTR(c"vkGetPhysicalDeviceProperties".as_ptr().cast()),
            )?);
            let destroy: DestroyFn = std::mem::transmute(GetProcAddress(
                module,
                PCSTR(c"vkDestroyInstance".as_ptr().cast()),
            )?);
            let app = App {
                ty: 0,
                next: ptr::null(),
                name: c"JimotoHardwareProbe".as_ptr(),
                version: 1,
                engine: ptr::null(),
                engine_version: 0,
                api_version: (1 << 22) | (2 << 12),
            };
            let info = Create {
                ty: 1,
                next: ptr::null(),
                flags: 0,
                app: &app,
                layer_count: 0,
                layers: ptr::null(),
                extension_count: 0,
                extensions: ptr::null(),
            };
            let mut instance = ptr::null_mut();
            if create(&info, ptr::null(), &mut instance) != 0 {
                return None;
            }
            let mut count = 0;
            let mut result = Vec::new();
            if enumerate(instance, &mut count, ptr::null_mut()) == 0 && count <= 32 {
                let mut devices = vec![ptr::null_mut(); count as usize];
                if enumerate(instance, &mut count, devices.as_mut_ptr()) == 0 {
                    for device in devices.into_iter().take(count as usize) {
                        // VkPhysicalDeviceProperties is <= 1024 bytes, aligned to 8.
                        let mut storage = [0u64; 256];
                        properties(device, storage.as_mut_ptr().cast());
                        let bytes = std::slice::from_raw_parts(
                            storage.as_ptr().cast::<u8>(),
                            storage.len() * 8,
                        );
                        let api = u32::from_ne_bytes(bytes[0..4].try_into().unwrap());
                        let vendor = u32::from_ne_bytes(bytes[8..12].try_into().unwrap());
                        let device_type = u32::from_ne_bytes(bytes[16..20].try_into().unwrap());
                        let name_bytes = &bytes[20..276];
                        let end = name_bytes
                            .iter()
                            .position(|&b| b == 0)
                            .unwrap_or(name_bytes.len());
                        if device_type != 4 && api >= ((1 << 22) | (2 << 12)) {
                            result.push((
                                vendor,
                                String::from_utf8_lossy(&name_bytes[..end]).into_owned(),
                            ));
                        }
                    }
                }
            }
            destroy(instance, ptr::null());
            Some(result)
        })()
        .unwrap_or_default();
        let _ = FreeLibrary(module);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cuda_driver_floor_is_explicit_and_unknown_is_not_supported() {
        assert!(!cuda_124_driver_supported("unknown"));
        assert!(!cuda_124_driver_supported("551.60"));
        assert!(cuda_124_driver_supported("551.61"));
        assert!(cuda_124_driver_supported("580.97"));
    }
    fn adapter(name: &str, vendor: &str, dedicated: u64) -> Gpu {
        Gpu {
            name: name.into(),
            vendor: vendor.into(),
            dedicated_vram_bytes: Some(dedicated),
            free_vram_bytes: None,
            driver_version: None,
            cuda_eligible: false,
            vulkan_eligible: false,
            is_software: false,
            memory_telemetry: "unknown".into(),
        }
    }
    #[test]
    fn nvidia_smi_rows_are_read_in_bytes_and_malformed_lines_are_skipped() {
        let rows = parse_nvidia_smi(
            "NVIDIA GeForce RTX 4090, 24564, 12932, 617.14\nnot a row\nNVIDIA GeForce RTX 4090, N/A, 100, 617.14\n",
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(rows[0].total, Some(24564 * 1_048_576));
        assert_eq!(rows[0].free, Some(12932 * 1_048_576));
        assert_eq!(rows[0].driver, "617.14");
        assert_eq!(rows[1].total, None);
    }
    #[test]
    fn identical_cards_pair_with_smi_rows_in_order_and_each_row_once() {
        let gpus = vec![
            adapter("Microsoft Basic Render Driver", "unknown", 0),
            adapter("NVIDIA GeForce RTX 4090", "nvidia", 1),
            adapter("NVIDIA GeForce RTX 4090", "nvidia", 2),
            adapter("AMD Radeon RX 7900", "amd", 3),
        ];
        let rows = parse_nvidia_smi(
            "NVIDIA GeForce RTX 4090, 100, 10, 600.00\nNVIDIA GeForce RTX 4090, 100, 20, 600.00\nNVIDIA GeForce RTX 4090, 100, 30, 600.00\n",
        );
        let pairs: Vec<_> = match_smi(&gpus, &rows)
            .into_iter()
            .map(|(gpu, row)| (gpu, row.free.unwrap() / 1_048_576))
            .collect();
        assert_eq!(pairs, [(1, 10), (2, 20)]);
    }
    #[test]
    fn smi_memory_replaces_the_estimate_but_never_exceeds_the_dedicated_size() {
        let mut gpu = adapter("NVIDIA GeForce RTX 4090", "nvidia", 24_000 * 1_048_576);
        gpu.free_vram_bytes = Some(1);
        gpu.memory_telemetry = "dxgi-local-budget".into();
        let rows = parse_nvidia_smi("NVIDIA GeForce RTX 4090, 24564, 24500, 617.14\n");
        apply_smi_memory(&mut gpu, &rows[0]);
        assert_eq!(gpu.free_vram_bytes, Some(24_000 * 1_048_576));
        assert_eq!(gpu.memory_telemetry, "nvidia-smi");
        let unreadable = parse_nvidia_smi("NVIDIA GeForce RTX 4090, N/A, N/A, 617.14\n");
        gpu.free_vram_bytes = Some(5);
        apply_smi_memory(&mut gpu, &unreadable[0]);
        assert_eq!(
            gpu.free_vram_bytes,
            Some(5),
            "a failed reading keeps the last value"
        );
    }
    #[test]
    fn disk_target_uses_an_existing_ancestor_without_creating_directories() {
        let root = std::env::temp_dir();
        let missing = root
            .join(format!("jimotokun-not-created-{}", uuid::Uuid::new_v4()))
            .join("models");
        assert_eq!(
            existing_ancestor(&missing).unwrap(),
            root.canonicalize().unwrap()
        );
        assert!(!missing.exists());
    }
}
