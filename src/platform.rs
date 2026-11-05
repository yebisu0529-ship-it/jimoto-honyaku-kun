//! Local Windows screen capture, installed Windows OCR, and system-audio capture.
//! Audio is captured only while `capture_loopback` is running; no microphone is opened.

use anyhow::{Context, Result, bail};
use std::sync::{Arc, atomic::AtomicBool, mpsc::SyncSender};

#[derive(Clone, Debug)]
pub struct Screen {
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// Mono, normalized PCM with times relative to the start of the capture session.
#[derive(Clone, Debug)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Encode mono 16-bit PCM, accepted by local Whisper-compatible servers.
pub fn pcm_to_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
    anyhow::ensure!(
        (8_000..=192_000).contains(&sample_rate),
        "Invalid PCM sample rate: {sample_rate}"
    );
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut output,
            hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for &sample in samples {
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            writer.write_sample((sample * i16::MAX as f32).round() as i16)?;
        }
        writer.finalize()?;
    }
    Ok(output.into_inner())
}

#[cfg(windows)]
pub fn screens() -> Result<Vec<Screen>> {
    xcap::Monitor::all()
        .context("Could not enumerate displays")?
        .into_iter()
        .map(|monitor| {
            Ok(Screen {
                id: monitor.id()?,
                name: monitor.name()?,
                x: monitor.x()?,
                y: monitor.y()?,
                width: monitor.width()?,
                height: monitor.height()?,
                scale: monitor.scale_factor()?,
            })
        })
        .collect()
}

#[cfg(windows)]
pub fn capture_screen(id: u32) -> Result<image::RgbaImage> {
    for monitor in xcap::Monitor::all().context("Could not enumerate displays")? {
        if monitor.id()? == id {
            return monitor
                .capture_image()
                .context("Screen capture failed; check that the selected display is connected");
        }
    }
    bail!("Display {id} is no longer connected; refresh the display list")
}

#[cfg(windows)]
pub fn capture_for_selection(screen_id: u32, owner: isize) -> Result<image::RgbaImage> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetWindowThreadProcessId, IsWindow, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE,
            ShowWindowAsync,
        },
    };
    let window = HWND(owner as *mut _);
    anyhow::ensure!(
        owner != 0 && unsafe { IsWindow(Some(window)) }.as_bool(),
        "画面取得用のウィンドウを確認できません。"
    );
    let process = std::process::id();
    let mut owner_process = 0;
    let owner_thread = unsafe { GetWindowThreadProcessId(window, Some(&mut owner_process)) };
    anyhow::ensure!(
        owner_thread != 0 && owner_process == process,
        "別のアプリのウィンドウは画面取得用に非表示にできません。"
    );
    struct RestoreWindow {
        window: HWND,
        process: u32,
        thread: u32,
    }
    impl Drop for RestoreWindow {
        fn drop(&mut self) {
            let mut process = 0;
            let thread = unsafe { GetWindowThreadProcessId(self.window, Some(&mut process)) };
            // The HWND could have been destroyed while the capture ran. Never
            // restore a recycled handle belonging to an unrelated process.
            if thread == self.thread && process == self.process {
                let _ = unsafe { ShowWindowAsync(self.window, SW_SHOWNOACTIVATE) };
            }
        }
    }
    let _restore = if unsafe { IsWindowVisible(window) }.as_bool() {
        // Arm restoration before requesting hide, including a failed request.
        let restore = RestoreWindow {
            window,
            process,
            thread: owner_thread,
        };
        anyhow::ensure!(
            unsafe { ShowWindowAsync(window, SW_HIDE) }.as_bool(),
            "画面取得のためにウィンドウを非表示にできませんでした。"
        );
        std::thread::sleep(std::time::Duration::from_millis(250));
        Some(restore)
    } else {
        None
    };
    // Capture failures and Rust panics both restore via the guard. No egui
    // viewport command or subsequent UI frame is needed to show the window.
    std::panic::catch_unwind(|| capture_screen(screen_id))
        .map_err(|_| anyhow::anyhow!("画面の取得処理が停止しました。もう一度お試しください。"))?
}

#[cfg(not(windows))]
pub fn capture_for_selection(_screen_id: u32, _owner: isize) -> Result<image::RgbaImage> {
    bail!("Display capture requires Windows")
}

#[cfg(windows)]
pub fn ocr(image: &image::RgbaImage, language: &str) -> Result<String> {
    // A fresh worker ensures WinRT is initialized as MTA even if a caller is an STA UI.
    std::thread::scope(|scope| {
        scope
            .spawn(|| windows_ocr(image, language))
            .join()
            .map_err(|_| anyhow::anyhow!("The Windows OCR worker terminated unexpectedly"))?
    })
}

#[cfg(windows)]
fn windows_ocr(image: &image::RgbaImage, requested: &str) -> Result<String> {
    use windows::{
        Globalization::Language,
        Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        core::HSTRING,
    };
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.context("Could not initialize Windows OCR")?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    let _apartment = Apartment;
    anyhow::ensure!(
        image.width() > 0 && image.height() > 0,
        "OCR image is empty"
    );

    let languages =
        OcrEngine::AvailableRecognizerLanguages().context("Windows OCR is not available")?;
    let mut installed = Vec::new();
    for language in languages {
        installed.push(language.LanguageTag()?.to_string());
    }
    anyhow::ensure!(
        !installed.is_empty(),
        "No Windows OCR languages are installed. Open Windows Settings → Time & language → Language & region, add the source language and install its Basic typing/OCR feature."
    );
    let requested = requested.trim().replace('_', "-");
    let auto = requested.is_empty() || requested.eq_ignore_ascii_case("auto");
    let engine = if auto {
        OcrEngine::TryCreateFromUserProfileLanguages().or_else(|_| {
            let language = Language::CreateLanguage(&HSTRING::from(&installed[0]))?;
            OcrEngine::TryCreateFromLanguage(&language)
        })?
    } else {
        let tag = installed_language(&installed, &requested)
            .ok_or_else(|| anyhow::anyhow!(
                "Windows OCR language '{requested}' is not installed. Installed: {}. Add its Basic typing/OCR feature in Windows Settings → Time & language → Language & region, or select auto.", installed.join(", ")))?;
        let language = Language::CreateLanguage(&HSTRING::from(tag))?;
        OcrEngine::TryCreateFromLanguage(&language)?
    };
    let max = OcrEngine::MaxImageDimension()?;
    let resized;
    let image = if image.width() > max || image.height() > max {
        let ratio = max as f64 / image.width().max(image.height()) as f64;
        resized = image::imageops::resize(
            image,
            ((image.width() as f64 * ratio).floor() as u32).max(1),
            ((image.height() as f64 * ratio).floor() as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
        &resized
    } else {
        image
    };
    let mut bgra = image.as_raw().clone();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &writer.DetachBuffer()?,
        BitmapPixelFormat::Bgra8,
        image.width() as i32,
        image.height() as i32,
        BitmapAlphaMode::Ignore,
    )?;
    let result = engine
        .RecognizeAsync(&bitmap)?
        .join()
        .context("Windows OCR recognition failed")?;
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let text = line.Text()?.to_string();
        if !text.trim().is_empty() {
            lines.push(text);
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(any(windows, test))]
fn installed_language<'a>(installed: &'a [String], requested: &str) -> Option<&'a str> {
    let requested = requested.trim().replace('_', "-").to_ascii_lowercase();
    let exact = |candidate: &str| {
        installed
            .iter()
            .find(|tag| tag.eq_ignore_ascii_case(candidate))
            .map(String::as_str)
    };
    if let Some(tag) = exact(&requested) {
        return Some(tag);
    }
    let prefix = format!("{requested}-");
    if let Some(tag) = installed
        .iter()
        .find(|tag| tag.to_ascii_lowercase().starts_with(&prefix))
    {
        return Some(tag.as_str());
    }
    // Windows can advertise Chinese OCR using region-only tags. Never fall
    // back from an explicitly requested Chinese script to its other variant.
    let region_aliases: &[&str] = match requested.as_str() {
        "zh-hans" => &["zh-CN", "zh-SG"],
        "zh-hant" => &["zh-TW", "zh-HK", "zh-MO"],
        _ => &[],
    };
    for alias in region_aliases {
        if let Some(tag) = exact(alias) {
            return Some(tag);
        }
    }
    if (2..=3).contains(&requested.len()) && requested.bytes().all(|c| c.is_ascii_alphabetic()) {
        return installed
            .iter()
            .find(|tag| {
                tag.split('-')
                    .next()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(&requested)
            })
            .map(String::as_str);
    }
    None
}

#[cfg(windows)]
pub fn capture_loopback(
    stop: Arc<AtomicBool>,
    tx: SyncSender<AudioChunk>,
    chunk_seconds: f32,
) -> Result<()> {
    capture_loopback_suppressing(stop, tx, chunk_seconds, Arc::new(AtomicBool::new(false)))
}

/// Drop playback packets while our own TTS plays, including the partial window.
#[cfg(windows)]
pub fn capture_loopback_suppressing(
    stop: Arc<AtomicBool>,
    tx: SyncSender<AudioChunk>,
    chunk_seconds: f32,
    suppress: Arc<AtomicBool>,
) -> Result<()> {
    use std::{
        sync::{atomic::Ordering, mpsc::TrySendError},
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Media::Audio::{
                AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
                AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
            },
            System::Com::{CLSCTX_ALL, CoCreateInstance},
        },
        core::{HSTRING, PCWSTR},
    };
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    anyhow::ensure!(
        chunk_seconds.is_finite(),
        "Audio chunk duration must be finite"
    );
    let chunk_duration = Duration::from_secs_f32(chunk_seconds.clamp(2.0, 4.0));
    wasapi::initialize_mta()
        .ok()
        .context("Could not initialize system audio; run capture on a worker thread")?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            wasapi::deinitialize();
        }
    }
    let _apartment = Apartment;

    let enumerator = wasapi::DeviceEnumerator::new()?;
    let endpoint_id = enumerator
        .get_default_device(&wasapi::Direction::Render)
        .context("No default playback device is available")?
        .get_id()?;
    let native: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
    let endpoint_id = HSTRING::from(endpoint_id);
    let endpoint = unsafe { native.GetDevice(PCWSTR(endpoint_id.as_ptr())) }?;
    let client: IAudioClient = unsafe { endpoint.Activate(CLSCTX_ALL, None) }?;
    const RATE: u32 = 16_000;
    // Shared-mode engine conversion handles the device's channel count and sample rate.
    let format =
        wasapi::WaveFormat::new(32, 32, &wasapi::SampleType::Float, RATE as usize, 1, None);
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            1_000_000,
            0,
            format.as_waveformatex_ref(),
            None,
        )
    }
    .context("Could not start loopback capture on the default playback device")?;
    let capture: IAudioCaptureClient = unsafe { client.GetService() }?;
    unsafe { client.Start() }?;
    struct Running<'a>(&'a IAudioClient);
    impl Drop for Running<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.0.Stop() };
        }
    }
    let _running = Running(&client);
    let started = Instant::now();
    let mut window_start = started;
    let mut samples = Vec::with_capacity((RATE as f32 * chunk_duration.as_secs_f32()) as usize);
    let mut first_sample_ms = None;
    let mut last_sample_ms = 0;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if suppress.load(Ordering::Acquire) {
            samples.clear();
            first_sample_ms = None;
            window_start = Instant::now();
        }
        // Read packets directly: wasapi 0.22's copying helpers do not guard a null
        // packet pointer on AUDCLNT_BUFFERFLAGS_SILENT. Windows permits that pointer.
        while !stop.load(Ordering::Acquire) && unsafe { capture.GetNextPacketSize() }? > 0 {
            let mut pointer = std::ptr::null_mut();
            let (mut frames, mut flags) = (0, 0);
            unsafe { capture.GetBuffer(&mut pointer, &mut frames, &mut flags, None, None) }?;
            // Always release an acquired packet, including when processing fails.
            let packet = (|| -> Result<()> {
                if suppress.load(Ordering::Acquire) {
                    samples.clear();
                    first_sample_ms = None;
                    window_start = Instant::now();
                    return Ok(());
                }
                if frames == 0 {
                    return Ok(());
                }
                if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                    samples.clear();
                    first_sample_ms = None;
                }
                let now_ms = started.elapsed().as_millis() as u64;
                let duration_ms = frames as u64 * 1_000 / RATE as u64;
                first_sample_ms.get_or_insert(now_ms.saturating_sub(duration_ms));
                last_sample_ms = now_ms;
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    samples.resize(samples.len() + frames as usize, 0.0);
                } else {
                    anyhow::ensure!(
                        !pointer.is_null(),
                        "System audio returned an invalid packet"
                    );
                    let bytes = unsafe { std::slice::from_raw_parts(pointer, frames as usize * 4) };
                    for sample in bytes.as_chunks::<4>().0 {
                        let value = f32::from_le_bytes(*sample);
                        samples.push(if value.is_finite() {
                            value.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        });
                    }
                }
                Ok(())
            })();
            let release = unsafe { capture.ReleaseBuffer(frames) };
            packet?;
            release?;
            // Bound memory even if the capture thread was descheduled for a long time.
            if samples.len() >= (RATE as f32 * chunk_duration.as_secs_f32()) as usize {
                break;
            }
        }
        if window_start.elapsed() >= chunk_duration
            || samples.len() >= (RATE as f32 * chunk_duration.as_secs_f32()) as usize
        {
            if has_audio(&samples) {
                let chunk = AudioChunk {
                    samples: std::mem::take(&mut samples),
                    sample_rate: RATE,
                    start_ms: first_sample_ms.unwrap_or(last_sample_ms),
                    end_ms: last_sample_ms,
                };
                match tx.try_send(chunk) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => return Ok(()),
                }
            }
            samples.clear();
            first_sample_ms = None;
            window_start = Instant::now();
        }
        // Polling remains responsive when Windows delivers no packets during silence.
        std::thread::sleep(Duration::from_millis(10));
    }
    // Preserve a useful partial utterance when the user stops capture.
    if samples.len() >= RATE as usize / 3 && has_audio(&samples) {
        let _ = tx.try_send(AudioChunk {
            samples,
            sample_rate: RATE,
            start_ms: first_sample_ms.unwrap_or(last_sample_ms),
            end_ms: last_sample_ms,
        });
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn has_audio(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let energy: f64 = samples.iter().map(|&x| (x as f64).powi(2)).sum();
    (energy / samples.len() as f64).sqrt() >= 0.0007
}

#[cfg(not(windows))]
pub fn screens() -> Result<Vec<Screen>> {
    bail!("Display capture requires Windows")
}
#[cfg(not(windows))]
pub fn capture_screen(_id: u32) -> Result<image::RgbaImage> {
    bail!("Display capture requires Windows")
}
#[cfg(not(windows))]
pub fn ocr(_image: &image::RgbaImage, _language: &str) -> Result<String> {
    bail!("Native OCR requires Windows")
}
#[cfg(not(windows))]
pub fn capture_loopback(
    _stop: Arc<AtomicBool>,
    _tx: SyncSender<AudioChunk>,
    _chunk_seconds: f32,
) -> Result<()> {
    bail!("System audio capture requires Windows")
}

#[cfg(not(windows))]
pub fn capture_loopback_suppressing(
    stop: Arc<AtomicBool>,
    tx: SyncSender<AudioChunk>,
    chunk_seconds: f32,
    _suppress: Arc<AtomicBool>,
) -> Result<()> {
    capture_loopback(stop, tx, chunk_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn selection_capture_rejects_null_owner_before_monitor_lookup() {
        let error = capture_for_selection(u32::MAX, 0).unwrap_err();
        assert!(error.to_string().contains("ウィンドウを確認できません"));
    }

    #[test]
    fn ocr_language_selection_preserves_chinese_scripts() {
        let installed = ["zh-Hans-CN", "zh-Hant-TW", "en-US", "ja-JP"].map(String::from);
        assert_eq!(
            installed_language(&installed, "ZH-hANS"),
            Some("zh-Hans-CN")
        );
        assert_eq!(
            installed_language(&installed, "zh_hant"),
            Some("zh-Hant-TW")
        );
        assert_eq!(installed_language(&installed, "EN"), Some("en-US"));
        assert_eq!(installed_language(&installed, "ja"), Some("ja-JP"));
        assert_eq!(installed_language(&installed, "en-GB"), None);
        let regions = ["zh-HK", "zh-CN", "zh-TW"].map(String::from);
        assert_eq!(installed_language(&regions, "zh-Hans"), Some("zh-CN"));
        assert_eq!(installed_language(&regions, "zh-Hant"), Some("zh-TW"));
        assert_eq!(
            installed_language(&["zh-HK".into()], "zh-Hant"),
            Some("zh-HK")
        );
        assert_eq!(
            installed_language(&["zh-Hans-CN".into(), "zh-CN".into()], "zh-Hant"),
            None
        );
        assert_eq!(
            installed_language(&["zh-Hant-TW".into(), "zh-TW".into()], "zh-Hans"),
            None
        );
        let exact = ["zh-Hant-TW", "zh-Hant"].map(String::from);
        assert_eq!(installed_language(&exact, "zh-Hant"), Some("zh-Hant"));
    }

    #[test]
    fn wav_is_mono_pcm_and_sanitizes_nonfinite_samples() {
        let wav = pcm_to_wav(&[-2.0, 0.0, 2.0, f32::NAN, f32::INFINITY], 16_000).unwrap();
        let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(
            reader
                .samples::<i16>()
                .map(|sample| sample.unwrap())
                .collect::<Vec<_>>(),
            vec![-32767, 0, 32767, 0, 0]
        );
        assert!(pcm_to_wav(&[], 0).is_err());
    }

    #[test]
    fn silence_is_not_queued() {
        assert!(!has_audio(&[]));
        assert!(!has_audio(&vec![0.0; 16_000]));
        assert!(!has_audio(&vec![0.00001; 16_000]));
        assert!(has_audio(&vec![0.01; 16_000]));
    }
}
