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
            Graphics::Dxgi::{
                CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
                DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIFactory1,
            },
            Storage::FileSystem::GetDiskFreeSpaceExW,
            System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        },
        core::{Interface, PCWSTR},
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
    let factory: windows::core::Result<IDXGIFactory1> = unsafe { CreateDXGIFactory1() };
    match factory {
        Ok(factory) => {
            let vulkan = vulkan_devices();
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
                let name = String::from_utf16_lossy(&desc.Description[..end]);
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
                let normalized = normalized_name(&name);
                let vulkan_eligible = !software
                    && vulkan.iter().any(|(vendor, candidate)| {
                        *vendor == desc.VendorId && (normalized_name(candidate) == normalized)
                    });
                hardware.gpus.push(Gpu {
                    name,
                    vendor: match desc.VendorId {
                        0x10de => "nvidia",
                        0x1002 | 0x1022 => "amd",
                        0x8086 => "intel",
                        _ => "unknown",
                    }
                    .into(),
                    dedicated_vram_bytes: Some(dedicated),
                    free_vram_bytes: free,
                    driver_version: None,
                    cuda_eligible: false,
                    vulkan_eligible,
                    is_software: software,
                    memory_telemetry: if free.is_some() {
                        "dxgi-local-budget"
                    } else {
                        "unknown"
                    }
                    .into(),
                });
            }
        }
        Err(_) => hardware
            .warnings
            .push("DXGIでGPUを確認できませんでした。CPUを候補にします。".into()),
    }
    if let Some(output) = hidden_output(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total,memory.free,driver_version",
            "--format=csv,noheader,nounits",
        ],
    ) {
        let mut used = std::collections::HashSet::new();
        for line in output.lines() {
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            if fields.len() != 4 {
                continue;
            }
            let total = fields[1]
                .parse::<u64>()
                .ok()
                .and_then(|n| n.checked_mul(1_048_576));
            let free = fields[2]
                .parse::<u64>()
                .ok()
                .and_then(|n| n.checked_mul(1_048_576));
            if let Some((index, gpu)) = hardware.gpus.iter_mut().enumerate().find(|(index, gpu)| {
                !used.contains(index)
                    && gpu.vendor == "nvidia"
                    && normalized_name(&gpu.name) == normalized_name(fields[0])
            }) {
                used.insert(index);
                gpu.driver_version = Some(fields[3].to_string());
                gpu.cuda_eligible = !gpu.is_software && cuda_124_driver_supported(fields[3]);
                if let (Some(free), Some(total)) = (free, total) {
                    gpu.free_vram_bytes = Some(
                        free.min(total)
                            .min(gpu.dedicated_vram_bytes.unwrap_or(total)),
                    );
                    gpu.memory_telemetry = "nvidia-smi".into();
                }
            }
        }
    }
    if hardware
        .gpus
        .iter()
        .any(|gpu| gpu.vendor == "nvidia" && !gpu.cuda_eligible)
    {
        hardware.warnings.push("CUDA 12.4用Windowsドライバー551.61以上を確認できないGPUはCUDA自動選択から除外しました。".into());
    }
}

#[cfg(windows)]
fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
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
