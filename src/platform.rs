//! Local Windows screen capture, installed Windows OCR, and system-audio capture.
//! Audio is captured only while `capture_loopback`, `capture_loopback_suppressing` or
//! `capture_loopback_streaming` is running; no microphone is opened.

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

pub fn hide_console(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
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

/// Only the chosen area, in the pixels of a full capture of the display.
#[cfg(windows)]
pub fn capture_area(screen_id: u32, area: (u32, u32, u32, u32)) -> Result<image::RgbaImage> {
    let (x, y, width, height) = area;
    for monitor in xcap::Monitor::all().context("Could not enumerate displays")? {
        if monitor.id()? == screen_id {
            return monitor
                .capture_region(x, y, width, height)
                .context("Screen capture failed; check that the selected display is connected");
        }
    }
    bail!("Display {screen_id} is no longer connected; choose the area again")
}

/// The window is minimized. Windows then reports it at (-32000, -32000) with
/// no size.
#[cfg(windows)]
pub fn is_minimized(window: isize) -> bool {
    use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsIconic};
    window != 0 && unsafe { IsIconic(HWND(window as *mut _)) }.as_bool()
}
#[cfg(not(windows))]
pub fn is_minimized(_window: isize) -> bool {
    false
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
                AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
                AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient,
                IMMDeviceEnumerator, MMDeviceEnumerator,
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
                // AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY does not clear the window:
                // Windows can set it on every packet of a contiguous loopback stream,
                // which would discard all audio. One window is one ASR request.
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
pub fn capture_area(_screen_id: u32, _area: (u32, u32, u32, u32)) -> Result<image::RgbaImage> {
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

/// Continuous 16 kHz, mono PCM16. Times are relative to this capture session.
/// A discontinuity requires the streaming recognizer to reset before this frame.
#[derive(Clone, Debug)]
pub struct StreamingAudioFrame {
    pub samples: Vec<i16>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub discontinuity: bool,
}

#[cfg(any(windows, test))]
struct StreamingFrames {
    frame_samples: usize,
    cursor: u64,
    pending: std::collections::VecDeque<i16>,
    discontinuity: bool,
    suppressed: bool,
}

/// Preserve exact sample positions across packet boundaries. QPC is used only
/// to anchor/re-anchor the stream, not to round each packet's sample position.
#[cfg(any(windows, test))]
#[derive(Default)]
struct StreamingPacketClock {
    next_sample: Option<i64>,
}

#[cfg(any(windows, test))]
impl StreamingPacketClock {
    /// Places one WASAPI loopback packet. The read time stands in only when
    /// Windows reports no valid timestamp.
    ///
    /// AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY is not a reset: Windows can set it
    /// on every packet of a contiguous loopback stream, and resetting the
    /// recognizer on each packet stops all recognition. A real gap moves the
    /// timestamp, which `position` detects.
    #[cfg(windows)]
    fn place(
        &mut self,
        flags: u32,
        timestamp_sample: i64,
        read_sample: i64,
        count: u32,
    ) -> (i64, bool) {
        use windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR;
        let invalid_time = flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0;
        let sample = if invalid_time {
            read_sample - count as i64
        } else {
            timestamp_sample
        };
        self.position(sample, count, invalid_time)
    }

    fn position(&mut self, clock_sample: i64, count: u32, reset: bool) -> (i64, bool) {
        // The shared engine can convert device-rate frames to the requested
        // 16kHz format. Advance by the returned packet's frame count, never by
        // the device-position delta (which need not have the same rate).
        let (start, discontinuity) = match self.next_sample {
            Some(next) if !reset && (next as i128 - clock_sample as i128).abs() <= 320 => {
                (next, false)
            }
            previous => (clock_sample, reset || previous.is_some()),
        };
        self.next_sample = Some(start.saturating_add(count as i64));
        (start, discontinuity)
    }
}

#[cfg(any(windows, test))]
impl StreamingFrames {
    fn new(frame_ms: u32) -> Result<Self> {
        anyhow::ensure!(
            matches!(frame_ms, 80 | 160),
            "Streaming audio frames must be 80 or 160 ms"
        );
        Ok(Self {
            frame_samples: frame_ms as usize * 16,
            cursor: 0,
            pending: std::collections::VecDeque::new(),
            discontinuity: true,
            suppressed: false,
        })
    }

    fn suppress(&mut self, suppressed: bool) {
        if suppressed != self.suppressed {
            self.suppressed = suppressed;
            // Do not leak a buffered partial TTS packet across either boundary.
            self.pending.make_contiguous().fill(0);
            self.discontinuity = true;
        }
    }

    fn packet(&mut self, start: i64, samples: &[i16], discontinuity: bool) {
        self.discontinuity |= discontinuity;
        let skipped = if start < 0 {
            start.unsigned_abs().min(samples.len() as u64) as usize
        } else {
            0
        };
        let samples = &samples[skipped..];
        let start = start.max(0) as u64;
        if start.saturating_add(samples.len() as u64) <= self.cursor {
            // Wall-clock silence was already emitted. Never append late samples
            // after it: that would accumulate latency indefinitely.
            if samples.iter().any(|&value| value != 0) {
                self.discontinuity = true;
            }
            return;
        }
        // A stalled worker must not allocate minutes of missing PCM. Advance to
        // a frame boundary and explicitly reset instead of joining unrelated audio.
        if start > self.cursor.saturating_add(32_000) {
            self.cursor = start / self.frame_samples as u64 * self.frame_samples as u64;
            self.pending.clear();
            self.discontinuity = true;
        }
        let skip = self.cursor.saturating_sub(start).min(samples.len() as u64) as usize;
        let start = start.max(self.cursor);
        let offset = (start - self.cursor) as usize;
        let samples = &samples[skip..];
        let keep = samples.len().min(32_000_usize.saturating_sub(offset));
        if keep < samples.len() {
            self.discontinuity = true;
        }
        self.pending
            .resize(self.pending.len().max(offset + keep), 0);
        for (index, &sample) in samples[..keep].iter().enumerate() {
            self.pending[offset + index] = if self.suppressed { 0 } else { sample };
        }
    }

    fn emit_until(&mut self, due_samples: u64, tx: &SyncSender<StreamingAudioFrame>) -> bool {
        use std::sync::mpsc::TrySendError;
        // Bound recovery work after a long OS scheduling pause as well as memory.
        if due_samples > self.cursor.saturating_add(32_000) {
            let next = due_samples.saturating_sub(self.frame_samples as u64);
            self.cursor = next / self.frame_samples as u64 * self.frame_samples as u64;
            self.pending.clear();
            self.discontinuity = true;
        }
        while self.cursor + self.frame_samples as u64 <= due_samples {
            let start = self.cursor;
            let samples = (0..self.frame_samples)
                .map(|_| self.pending.pop_front().unwrap_or(0))
                .collect();
            self.cursor += self.frame_samples as u64;
            let frame = StreamingAudioFrame {
                samples,
                start_ms: start / 16,
                end_ms: self.cursor / 16,
                discontinuity: self.discontinuity,
            };
            match tx.try_send(frame) {
                Ok(()) => self.discontinuity = false,
                Err(TrySendError::Full(_)) => self.discontinuity = true,
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
        true
    }
}

/// Default-render WASAPI loopback only; no microphone or network is opened.
/// Call on a worker with a bounded sync_channel. Silence is emitted too, and TTS
/// suppression becomes silence plus reset markers. Full queues drop frames and
/// mark the next accepted frame for reset. Stop discards an incomplete frame.
#[cfg(windows)]
pub fn capture_loopback_streaming(
    stop: Arc<AtomicBool>,
    tx: SyncSender<StreamingAudioFrame>,
    frame_ms: u32,
    suppress: Arc<AtomicBool>,
) -> Result<()> {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Media::Audio::{
                AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
                AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient,
                IMMDeviceEnumerator, MMDeviceEnumerator,
            },
            System::Com::{CLSCTX_ALL, CoCreateInstance},
        },
        core::{HSTRING, PCWSTR},
    };
    let mut frames = StreamingFrames::new(frame_ms)?;
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    wasapi::initialize_mta()
        .ok()
        .context("Could not initialize streaming system audio on this worker")?;
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
    // One continuously running Windows SRC instance converts the device mix to
    // 16 kHz mono. Framing never resets SRC or rounds individual packet lengths.
    let format = wasapi::WaveFormat::new(32, 32, &wasapi::SampleType::Float, 16_000, 1, None);
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
    .context("Could not initialize continuous playback capture")?;
    let capture: IAudioCaptureClient = unsafe { client.GetService() }?;
    // WASAPI QPC timestamps use 100 ns units. Obtain the same monotonic clock
    // rather than mapping packets by read time, which jitters with scheduling.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceCounter(value: *mut i64) -> i32;
        fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    }
    let (mut frequency, mut counter) = (0_i64, 0_i64);
    anyhow::ensure!(
        unsafe { QueryPerformanceFrequency(&mut frequency) } != 0 && frequency > 0,
        "Could not read the streaming audio clock"
    );
    anyhow::ensure!(
        unsafe { QueryPerformanceCounter(&mut counter) } != 0,
        "Could not read the streaming audio clock"
    );
    let origin_100ns = counter as i128 * 10_000_000 / frequency as i128;
    let started = Instant::now();
    unsafe { client.Start() }?;
    struct Running<'a>(&'a IAudioClient);
    impl Drop for Running<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.0.Stop() };
        }
    }
    let _running = Running(&client);
    let mut packet_clock = StreamingPacketClock::default();
    struct Packet<'a> {
        capture: &'a IAudioCaptureClient,
        count: u32,
    }
    impl Drop for Packet<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.capture.ReleaseBuffer(self.count) };
        }
    }
    while !stop.load(Ordering::Acquire) {
        frames.suppress(suppress.load(Ordering::Acquire));
        while !stop.load(Ordering::Acquire) && unsafe { capture.GetNextPacketSize() }? > 0 {
            let mut pointer = std::ptr::null_mut();
            let (mut count, mut flags, mut qpc) = (0, 0, 0);
            unsafe {
                capture.GetBuffer(&mut pointer, &mut count, &mut flags, None, Some(&mut qpc))
            }?;
            let packet = Packet {
                capture: &capture,
                count,
            };
            frames.suppress(suppress.load(Ordering::Acquire));
            let (start, reset) = packet_clock.place(
                flags,
                ((qpc as i128 - origin_100ns) * 16_000 / 10_000_000) as i64,
                (started.elapsed().as_nanos() * 16_000 / 1_000_000_000) as i64,
                count,
            );
            anyhow::ensure!(
                count <= 32_000,
                "System audio returned an oversized streaming packet"
            );
            let mut pcm = vec![0; count as usize];
            if !frames.suppressed && flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 == 0 && count > 0 {
                anyhow::ensure!(
                    !pointer.is_null(),
                    "System audio returned an invalid streaming packet"
                );
                let bytes = unsafe { std::slice::from_raw_parts(pointer, count as usize * 4) };
                for (sample, bytes) in pcm.iter_mut().zip(bytes.as_chunks::<4>().0) {
                    let value = f32::from_le_bytes(*bytes);
                    *sample = if value.is_finite() {
                        (value.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
                    } else {
                        0
                    };
                }
            }
            frames.packet(start, &pcm, reset);
            // Explicit normal release reports errors; Drop covers every error/panic above.
            let release = unsafe { capture.ReleaseBuffer(count) };
            std::mem::forget(packet);
            release?;
        }
        frames.suppress(suppress.load(Ordering::Acquire));
        // Windows may stop delivering loopback packets in complete silence.
        // A short guard tolerates engine packet delivery jitter without adding
        // accumulating delay; stale packets can never be appended afterwards.
        let due = started.elapsed().saturating_sub(Duration::from_millis(20));
        if !frames.emit_until((due.as_nanos() * 16_000 / 1_000_000_000) as u64, &tx) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn capture_loopback_streaming(
    _stop: Arc<AtomicBool>,
    _tx: SyncSender<StreamingAudioFrame>,
    _frame_ms: u32,
    _suppress: Arc<AtomicBool>,
) -> Result<()> {
    bail!("Continuous system audio capture requires Windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_frames_preserve_packet_samples_and_silence_timing() {
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let mut stream = StreamingFrames::new(80).unwrap();
        let input: Vec<i16> = (0..2560).map(|value| value as i16).collect();
        stream.packet(0, &input[..731], false);
        stream.packet(731, &input[731..1703], false);
        stream.packet(1703, &input[1703..], false);
        assert!(stream.emit_until(3840, &tx));
        let first = rx.try_recv().unwrap();
        let second = rx.try_recv().unwrap();
        let third = rx.try_recv().unwrap();
        assert!(first.discontinuity);
        assert!(!second.discontinuity);
        assert_eq!([first.samples, second.samples].concat(), input);
        assert_eq!((second.start_ms, second.end_ms), (80, 160));
        assert_eq!((third.start_ms, third.end_ms), (160, 240));
        assert_eq!(third.samples, vec![0; 1280]);
        assert!(StreamingFrames::new(100).is_err());
        assert_eq!(StreamingFrames::new(160).unwrap().frame_samples, 2560);
    }

    #[test]
    fn streaming_queue_drop_and_suppression_reset_without_blocking() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let mut stream = StreamingFrames::new(80).unwrap();
        stream.packet(0, &vec![123; 5120], false);
        assert!(stream.emit_until(2560, &tx)); // second frame is dropped
        assert_eq!(rx.try_recv().unwrap().start_ms, 0);
        assert!(stream.emit_until(3840, &tx));
        let after_gap = rx.try_recv().unwrap();
        assert_eq!(after_gap.start_ms, 160);
        assert!(after_gap.discontinuity);
        stream.suppress(true);
        assert!(stream.emit_until(5120, &tx));
        let muted = rx.try_recv().unwrap();
        assert!(muted.discontinuity);
        assert!(muted.samples.iter().all(|&sample| sample == 0));
        stream.suppress(false);
        stream.packet(5120, &vec![456; 1280], false);
        assert!(stream.emit_until(6400, &tx));
        let resumed = rx.try_recv().unwrap();
        assert!(resumed.discontinuity);
        assert!(resumed.samples.iter().all(|&sample| sample == 456));
        drop(rx);
        assert!(!stream.emit_until(7680, &tx));
    }

    #[test]
    fn streaming_clock_has_no_packet_rounding_loss_and_recovers_a_pause() {
        let mut clock = StreamingPacketClock::default();
        assert_eq!(clock.position(17, 441, false), (17, false));
        // Timestamp rounding/jitter must not change consecutive sample positions.
        assert_eq!(clock.position(457, 441, false), (458, false));
        assert_eq!(clock.position(900, 441, false), (899, false));
        assert_eq!(clock.position(4800, 441, false), (4800, true));
        assert_eq!(clock.position(5240, 441, false), (5241, false));
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let mut stream = StreamingFrames::new(80).unwrap();
        stream.packet(160_000, &[42; 1280], true);
        assert!(stream.pending.len() <= 32_000);
        assert!(stream.emit_until(161_280, &tx));
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.start_ms, 10_000);
        assert!(frame.discontinuity);
        assert_eq!(frame.samples, vec![42; 1280]);
    }

    #[cfg(windows)]
    #[test]
    fn loopback_packets_flagged_discontinuous_on_every_packet_stay_one_stream() {
        use windows::Win32::Media::Audio::{
            AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR,
        };
        let flagged = AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32;
        let mut clock = StreamingPacketClock::default();
        // 10 ms packets whose timestamps wander by up to 31 samples (2 ms), as
        // measured on a Windows loopback stream that flagged every packet.
        let placed: Vec<_> = [0, 191, 303, 488]
            .into_iter()
            .map(|timestamp| clock.place(flagged, timestamp, 0, 160))
            .collect();
        assert_eq!(
            placed,
            [(0, false), (160, false), (320, false), (480, false)],
            "flagged packets with continuing timestamps stay one stream"
        );
        assert_eq!(
            clock.place(flagged, 1_120, 0, 160),
            (1_120, true),
            "a timestamp jump is a real gap"
        );
        let missing = AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32;
        assert_eq!(
            clock.place(missing, 0, 5_000, 160),
            (4_840, true),
            "without a timestamp the read time places the packet and the stream resets"
        );
    }

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
