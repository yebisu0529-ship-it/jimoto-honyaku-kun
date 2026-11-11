//! Resident CPU punctuation actor. All native pointers/DLLs stay on its worker.
use crate::punctuation_setup::{self, PunctuationInstallation};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PunctuationResult {
    pub original: String,
    /// Raw native output, including its full-width punctuation if present.
    pub restored: String,
    /// Byte positions in original, after words ending internal sentences.
    /// The native model's unconditional final period is never included.
    pub boundaries: Vec<usize>,
}

#[derive(Debug)]
struct Token {
    text: String,
    start: usize,
    end: usize,
}
fn ascii_punctuation(c: char) -> char {
    match c {
        '。' => '.',
        '，' => ',',
        '？' => '?',
        '！' => '!',
        '；' => ';',
        '：' => ':',
        other => other,
    }
}
fn allowed(c: char) -> bool {
    ".?!,;:".contains(c)
}
fn numeric_separator(chars: &[(usize, char)], i: usize) -> bool {
    i > 0
        && i + 1 < chars.len()
        && chars[i - 1].1.is_ascii_digit()
        && chars[i + 1].1.is_ascii_digit()
        && ".,:;".contains(chars[i].1)
}
fn tokens(text: &str, normalize_output: bool) -> Vec<Token> {
    let chars: Vec<_> = text
        .char_indices()
        .map(|(i, c)| {
            (
                i,
                if normalize_output {
                    ascii_punctuation(c)
                } else {
                    c
                },
            )
        })
        .collect();
    let mut result = Vec::new();
    let mut current = String::new();
    let mut start = 0;
    let mut end = 0;
    for (i, (byte, c)) in chars.iter().enumerate() {
        if c.is_whitespace() || allowed(*c) && !numeric_separator(&chars, i) {
            if !current.is_empty() {
                result.push(Token {
                    text: std::mem::take(&mut current),
                    start,
                    end,
                });
            }
        } else {
            if current.is_empty() {
                start = *byte;
            }
            current.push(c.to_ascii_lowercase());
            end = *byte + text[*byte..].chars().next().unwrap().len_utf8();
        }
    }
    if !current.is_empty() {
        result.push(Token {
            text: current,
            start,
            end,
        });
    }
    result
}

/// Strict gate for native output. ASCII casing and the limited punctuation
/// alphabet are ignored; apostrophes, hyphens, other characters, word separation
/// and numeric punctuation (3.14 / 1,000 / 1:30) must remain exactly intact.
pub fn validate_word_preservation(original: &str, restored: &str) -> Result<PunctuationResult> {
    let input = tokens(original, false);
    let output = tokens(restored, true);
    ensure!(
        !input.is_empty() && input.len() == output.len(),
        "Punctuation output changed the original word sequence"
    );
    ensure!(
        input.iter().zip(&output).all(|(a, b)| a.text == b.text),
        "Punctuation output changed original words, names or numbers"
    );
    let chars: Vec<_> = restored
        .char_indices()
        .map(|(i, c)| (i, ascii_punctuation(c)))
        .collect();
    let mut boundaries = Vec::new();
    for (i, (byte, c)) in chars.iter().enumerate() {
        if !".!?".contains(*c) || numeric_separator(&chars, i) {
            continue;
        }
        let Some(index) = output.iter().rposition(|t| t.end <= *byte) else {
            continue;
        };
        if index + 1 >= input.len() || *byte >= output[index + 1].start {
            continue;
        }
        let mut boundary = input[index].end;
        for c in original[boundary..].chars() {
            if allowed(c) {
                boundary += c.len_utf8();
            } else {
                break;
            }
        }
        boundaries.push(boundary);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    Ok(PunctuationResult {
        original: original.into(),
        restored: restored.into(),
        boundaries,
    })
}

fn validate_input(text: &str) -> Result<()> {
    ensure!(
        !text.trim().is_empty() && text.chars().count() <= 12_000 && text.len() <= 64 * 1024,
        "Punctuation input must contain 1–12000 characters"
    );
    ensure!(
        !text.contains('\0'),
        "Punctuation input contains a NUL character"
    );
    ensure!(
        !text.chars().any(crate::live_text::cjk),
        "Automatic punctuation is enabled for English input only"
    );
    Ok(())
}

struct Job {
    text: String,
    cancel: Arc<AtomicBool>,
    reply: mpsc::SyncSender<std::result::Result<PunctuationResult, String>>,
}
pub struct PunctuationEngine {
    sender: Mutex<Option<mpsc::SyncSender<Job>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    stopped: Arc<AtomicBool>,
}
impl PunctuationEngine {
    /// Initializes once; an error is returned to the caller, with no restart loop.
    pub fn start(installation: PunctuationInstallation) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stopped);
        let worker = thread::Builder::new()
            .name("text-punctuation".into())
            .spawn(move || {
                let loaded = Native::load(&installation);
                let native = match loaded {
                    Ok(native) => {
                        let _ = ready_tx.send(Ok(()));
                        native
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("{error:#}")));
                        return;
                    }
                };
                while let Ok(job) = receiver.recv() {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let result = (|| -> Result<PunctuationResult> {
                        ensure!(
                            !job.cancel.load(Ordering::Acquire),
                            "Punctuation request cancelled"
                        );
                        let restored = native.punctuate(&job.text)?;
                        ensure!(
                            !job.cancel.load(Ordering::Acquire)
                                && !worker_stop.load(Ordering::Acquire),
                            "Punctuation request cancelled"
                        );
                        validate_word_preservation(&job.text, &restored)
                    })()
                    .map_err(|e| format!("{e:#}"));
                    let _ = job.reply.send(result);
                }
                // Native::drop destroys the model before unloading both DLLs here.
                drop(native);
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                sender: Mutex::new(Some(sender)),
                worker: Mutex::new(Some(worker)),
                stopped,
            }),
            result => {
                drop(sender);
                let _ = worker.join();
                match result {
                    Ok(Err(error)) => bail!(error),
                    _ => bail!("Punctuation worker stopped during initialization"),
                }
            }
        }
    }
    /// Caller must select this English-only stage explicitly. Cancellation
    /// discards work/results; an in-progress C ABI inference cannot be preempted.
    pub fn punctuate(&self, text: &str, cancel: &AtomicBool) -> Result<PunctuationResult> {
        validate_input(text)?;
        ensure!(
            !cancel.load(Ordering::Acquire) && !self.stopped.load(Ordering::Acquire),
            "Punctuation request cancelled"
        );
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let job_cancel = Arc::new(AtomicBool::new(false));
        let job = Job {
            text: text.into(),
            cancel: Arc::clone(&job_cancel),
            reply: reply_tx,
        };
        self.sender
            .lock()
            .map_err(|_| anyhow::anyhow!("Punctuation worker lock failed"))?
            .as_ref()
            .context("Punctuation worker is stopped")?
            .try_send(job)
            .map_err(|_| anyhow::anyhow!("Punctuation worker is busy or stopped"))?;
        loop {
            if cancel.load(Ordering::Acquire) || self.stopped.load(Ordering::Acquire) {
                job_cancel.store(true, Ordering::Release);
                bail!("Punctuation request cancelled");
            }
            match reply_rx.recv_timeout(Duration::from_millis(20)) {
                Ok(result) => return result.map_err(anyhow::Error::msg),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("Punctuation worker stopped before returning a result")
                }
            }
        }
    }
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.sender
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}
impl Drop for PunctuationEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

#[repr(C)]
struct ModelConfig {
    ct_transformer: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
}
#[repr(C)]
struct Config {
    model: ModelConfig,
}
type Create = unsafe extern "C" fn(*const Config) -> *const c_void;
type Destroy = unsafe extern "C" fn(*const c_void);
type Add = unsafe extern "C" fn(*const c_void, *const c_char) -> *const c_char;
type Free = unsafe extern "C" fn(*const c_char);
struct Native {
    handle: *const c_void,
    destroy: Destroy,
    add: Add,
    free: Free,
    _library: libloading::Library,
}
impl Native {
    #[cfg(windows)]
    fn load(installation: &PunctuationInstallation) -> Result<Self> {
        let verified =
            punctuation_setup::load(&installation.root)?.context("Punctuation is not installed")?;
        ensure!(
            &verified == installation,
            "Punctuation installation changed"
        );
        check_onnxruntime_collision(&installation.library)?;
        // Absolute DLL path, dependencies from its own directory/System32 only;
        // no PATH edits or process-wide AddDllDirectory changes.
        let library = unsafe {
            libloading::os::windows::Library::load_with_flags(&installation.library, 0x100 | 0x800)
        }
        .context("Cannot load punctuation DLL; Microsoft Visual C++ v14 x64 runtime is required")?;
        let library: libloading::Library = library.into();
        let create: Create = unsafe { *library.get(b"SherpaOnnxCreateOfflinePunctuation\0")? };
        let destroy: Destroy = unsafe { *library.get(b"SherpaOnnxDestroyOfflinePunctuation\0")? };
        let add: Add = unsafe { *library.get(b"SherpaOfflinePunctuationAddPunct\0")? };
        let free: Free = unsafe { *library.get(b"SherpaOfflinePunctuationFreeText\0")? };
        let model = CString::new(
            installation
                .model
                .to_str()
                .context("Punctuation model path must be UTF-8")?,
        )?;
        let provider = c"cpu";
        let config = Config {
            model: ModelConfig {
                ct_transformer: model.as_ptr(),
                num_threads: 1,
                debug: 0,
                provider: provider.as_ptr(),
            },
        };
        let handle = unsafe { create(&config) };
        ensure!(!handle.is_null(), "Cannot initialize punctuation model");
        Ok(Self {
            handle,
            destroy,
            add,
            free,
            _library: library,
        })
    }
    #[cfg(not(windows))]
    fn load(_installation: &PunctuationInstallation) -> Result<Self> {
        bail!("Punctuation runtime supports Windows x64 only")
    }
    fn punctuate(&self, text: &str) -> Result<String> {
        let text = CString::new(text)?;
        let output = unsafe { (self.add)(self.handle, text.as_ptr()) };
        ensure!(!output.is_null(), "Punctuation model returned no output");
        // Copy before releasing C-allocated storage, including the UTF-8 error path.
        let result = unsafe { CStr::from_ptr(output) }
            .to_str()
            .map(str::to_owned)
            .context("Punctuation model returned invalid UTF-8");
        unsafe { (self.free)(output) };
        let result = result?;
        ensure!(
            result.len() <= 256 * 1024,
            "Punctuation output exceeds bound"
        );
        Ok(result)
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.handle) }
    }
}

#[cfg(windows)]
fn check_onnxruntime_collision(library: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetModuleFileNameW(module: *mut c_void, path: *mut u16, size: u32) -> u32;
    }
    let name: Vec<u16> = "onnxruntime.dll\0".encode_utf16().collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    if module.is_null() {
        return Ok(());
    }
    let mut path = vec![0u16; 32768];
    let n = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
    ensure!(
        n > 0 && (n as usize) < path.len(),
        "Cannot verify an already loaded ONNX runtime"
    );
    let actual = std::fs::canonicalize(std::path::PathBuf::from(std::ffi::OsString::from_wide(
        &path[..n as usize],
    )))?;
    let expected = std::fs::canonicalize(
        library
            .parent()
            .context("Punctuation DLL parent missing")?
            .join("onnxruntime.dll"),
    )?;
    ensure!(
        actual == expected,
        "A different ONNX runtime is already loaded; punctuation was not initialized"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn punctuation_maps_to_original_bytes_without_terminal_period() {
        let original = "Hello café visitors we have 150 books";
        let result =
            validate_word_preservation(original, "Hello café visitors。 We have 150 books。")
                .unwrap();
        assert_eq!(result.original, original);
        assert_eq!(result.boundaries, vec!["Hello café visitors".len()]);
        assert!(original.is_char_boundary(result.boundaries[0]));
    }
    #[test]
    fn edits_numbers_apostrophes_hyphens_and_word_joins_are_rejected() {
        for (a, b) in [
            ("do not change", "do not ever change"),
            ("150 words", "15 words"),
            ("3.14 litres", "314 litres"),
            ("1,000 books", "1000 books"),
            ("don't leave", "don t leave"),
            ("co-op library", "co op library"),
            ("New York", "NewYork"),
            ("café visitors", "cafe visitors"),
        ] {
            assert!(validate_word_preservation(a, b).is_err(), "{a} -> {b}");
        }
    }
    #[test]
    fn case_and_limited_punctuation_only() {
        let result = validate_word_preservation(
            "we have 3.14 litres and 1,000 books",
            "We have 3.14 litres， and 1,000 books。",
        )
        .unwrap();
        assert!(result.boundaries.is_empty());
        assert!(validate_input("日本語を自動で処理しない").is_err());
    }
    #[test]
    fn text_with_a_compatibility_ideograph_is_not_taken_for_english() {
        assert!(
            validate_input("The \u{FA11} family opened a library").is_err(),
            "U+FA11 is a CJK ideograph to the live text as well"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "opt-in pinned model installation and local native inference; requires explicit paths"]
    fn native_punctuation_opt_in() {
        use std::{fs, path::PathBuf, time::Instant};
        let root = PathBuf::from(
            std::env::var_os("JIMOTO_PUNCTUATION_ROOT").expect("JIMOTO_PUNCTUATION_ROOT"),
        );
        let source_path = PathBuf::from(
            std::env::var_os("JIMOTO_PUNCTUATION_SOURCE").expect("JIMOTO_PUNCTUATION_SOURCE"),
        );
        let report = PathBuf::from(
            std::env::var_os("JIMOTO_PUNCTUATION_REPORT").expect("JIMOTO_PUNCTUATION_REPORT"),
        );
        assert!(root.is_absolute() && source_path.is_absolute() && report.is_absolute());
        let cancelled = Arc::new(AtomicBool::new(false));
        let installation = if std::env::var("JIMOTO_PUNCTUATION_INSTALL").as_deref() == Ok("1") {
            punctuation_setup::install(&root, Arc::clone(&cancelled), |p| {
                eprintln!("punctuation stage={} bytes={}", p.stage, p.downloaded_bytes)
            })
            .unwrap()
        } else {
            punctuation_setup::load(&root)
                .unwrap()
                .expect("existing installation")
        };
        let source = fs::read_to_string(source_path).unwrap();
        let prefix_end = source
            .char_indices()
            .nth(428)
            .map_or(source.len(), |(i, _)| i);
        let prefix_end = source[..prefix_end]
            .rfind(char::is_whitespace)
            .unwrap_or(prefix_end);
        let cases = [
            ("full_232_word_source", source.as_str()),
            (
                "300_char_prefix_plus_up_to_128_char_lookahead",
                &source[..prefix_end],
            ),
        ];
        let before = working_set();
        let started = Instant::now();
        let mut engine = PunctuationEngine::start(installation.clone()).unwrap();
        let load_ms = started.elapsed().as_millis();
        let loaded = working_set();
        let mut results = Vec::new();
        for (name, text) in cases {
            let started = Instant::now();
            let result = engine.punctuate(text, &cancelled).unwrap();
            let elapsed = started.elapsed().as_millis();
            assert_eq!(result.original, text);
            assert!(
                !result.boundaries.is_empty(),
                "no internal sentence boundaries"
            );
            assert!(
                result
                    .boundaries
                    .iter()
                    .all(|b| text.is_char_boundary(*b) && *b < text.len())
            );
            results.push(serde_json::json!({"case":name,"original":text,"restored":result.restored,"source_chars":text.chars().count(),"source_words":text.split_whitespace().count(),"boundaries":result.boundaries,"segments":result.boundaries.iter().scan(0,|start,end|{let piece=&text[*start..*end];*start = *end;Some(piece)}).collect::<Vec<_>>(),"elapsed_ms":elapsed,"word_preservation_gate":"passed"}));
        }
        let after_calls = working_set();
        let stopped = Instant::now();
        engine.stop();
        let stop_ms = stopped.elapsed().as_millis();
        let after_stop = working_set();
        assert!(
            !module_loaded("sherpa-onnx-c-api.dll"),
            "native library still loaded after stop/join"
        );
        assert!(
            !module_loaded("onnxruntime.dll"),
            "ONNX runtime still loaded after stop/join"
        );
        fs::write(report,serde_json::to_vec_pretty(&serde_json::json!({"mode":"simulated text input; local native DLL inference, no audio/GUI/server","installation":installation,"load_ms":load_ms,"stop_ms":stop_ms,"working_set_before":before,"working_set_after_load":loaded,"working_set_after_calls":after_calls,"working_set_after_stop":after_stop,"worker_joined":true,"sherpa_dll_unloaded":true,"onnxruntime_dll_unloaded":true,"calls":results})).unwrap()).unwrap();
    }

    #[cfg(windows)]
    fn module_loaded(name: &str) -> bool {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        }
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        !unsafe { GetModuleHandleW(name.as_ptr()) }.is_null()
    }
    #[cfg(windows)]
    fn working_set() -> usize {
        #[repr(C)]
        struct Counters {
            cb: u32,
            page_faults: u32,
            peak_working_set: usize,
            working_set: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut c_void;
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(
                process: *mut c_void,
                counters: *mut Counters,
                size: u32,
            ) -> i32;
        }
        let mut counters = std::mem::MaybeUninit::<Counters>::zeroed();
        let ok = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                counters.as_mut_ptr(),
                std::mem::size_of::<Counters>() as u32,
            )
        };
        assert_ne!(ok, 0);
        unsafe { counters.assume_init() }.working_set
    }
}
