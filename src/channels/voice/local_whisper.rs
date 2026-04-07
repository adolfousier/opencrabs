//! Local STT via rwhisper (candle-based, pure Rust)
//!
//! Model presets, download, and transcription engine.
//! Gated behind the `local-stt` feature flag.
//!
//! Uses rwhisper (built on candle-transformers) for quantized whisper inference.
//! No ggml C dependencies — resolves symbol conflicts with llama-cpp-sys-2 (issue #38).

use anyhow::{Context, Result};
use std::path::PathBuf;

// ─── Model presets ──────────────────────────────────────────────────────────

/// A local whisper model preset.
pub struct LocalModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub file_name: &'static str,
    pub size_label: &'static str,
    /// rwhisper model source variant.
    pub repo_id: &'static str,
}

/// Available local whisper model sizes.
/// Uses rwhisper's quantized GGUF models (fast, small, pure Rust).
pub const LOCAL_MODEL_PRESETS: &[LocalModelPreset] = &[
    LocalModelPreset {
        id: "local-tiny",
        label: "Tiny (Multilingual, Quantized)",
        file_name: "tiny",
        size_label: "~42 MB",
        repo_id: "QuantizedTiny",
    },
    LocalModelPreset {
        id: "local-base",
        label: "Base (English)",
        file_name: "base.en",
        size_label: "~142 MB",
        repo_id: "BaseEn",
    },
    LocalModelPreset {
        id: "local-small",
        label: "Small (English)",
        file_name: "small.en",
        size_label: "~466 MB",
        repo_id: "SmallEn",
    },
    LocalModelPreset {
        id: "local-medium",
        label: "Medium (English)",
        file_name: "medium.en",
        size_label: "~1.5 GB",
        repo_id: "MediumEn",
    },
];

/// Look up a model preset by ID.
pub fn find_local_model(id: &str) -> Option<&'static LocalModelPreset> {
    LOCAL_MODEL_PRESETS.iter().find(|m| m.id == id)
}

/// Directory where local models are stored.
pub fn models_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("opencrabs").join("models").join("whisper");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Full path for a model preset (directory containing model files).
pub fn model_path(preset: &LocalModelPreset) -> PathBuf {
    models_dir().join(preset.file_name)
}

/// Check if a model is already downloaded.
/// rwhisper handles its own caching, so we just check if the preset is valid.
pub fn is_model_downloaded(_preset: &LocalModelPreset) -> bool {
    // rwhisper auto-downloads and caches models — always return true
    // to skip our manual download logic. The model will be fetched on first use.
    true
}

// ─── Download ───────────────────────────────────────────────────────────────

/// Download progress info sent via channel.
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
}

/// Download a model. With rwhisper, models are auto-downloaded on first use.
/// This is kept for API compatibility with the onboarding flow.
pub async fn download_model(
    preset: &LocalModelPreset,
    progress_tx: tokio::sync::mpsc::UnboundedSender<DownloadProgress>,
) -> Result<PathBuf> {
    tracing::info!(
        "Whisper model '{}' will be downloaded on first use by rwhisper",
        preset.id
    );

    // Build the model now to trigger download.
    // Suppress stdout — kalosm-common prints "Running on CPU..." via println!
    let source = parse_whisper_source(preset)?;
    let progress_tx_clone = progress_tx.clone();
    let _stdout_guard = suppress_stdout();
    rwhisper::WhisperBuilder::default()
        .with_source(source)
        .build_with_loading_handler(move |progress| match progress {
            rwhisper::ModelLoadingProgress::Downloading {
                progress: file_progress,
                ..
            } => {
                let _ = progress_tx_clone.send(DownloadProgress {
                    downloaded: file_progress.progress,
                    total: Some(file_progress.size),
                    done: false,
                    error: None,
                });
            }
            rwhisper::ModelLoadingProgress::Loading { progress } => {
                let _ = progress_tx_clone.send(DownloadProgress {
                    downloaded: (progress * 100.0) as u64,
                    total: Some(100),
                    done: progress >= 1.0,
                    error: None,
                });
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to download/load whisper model: {}", e))?;

    let _ = progress_tx.send(DownloadProgress {
        downloaded: 100,
        total: Some(100),
        done: true,
        error: None,
    });

    Ok(model_path(preset))
}

/// Parse preset's repo_id into a WhisperSource.
fn parse_whisper_source(preset: &LocalModelPreset) -> Result<rwhisper::WhisperSource> {
    match preset.repo_id {
        "QuantizedTiny" => Ok(rwhisper::WhisperSource::QuantizedTiny),
        "QuantizedTinyEn" => Ok(rwhisper::WhisperSource::QuantizedTinyEn),
        "Tiny" => Ok(rwhisper::WhisperSource::Tiny),
        "TinyEn" => Ok(rwhisper::WhisperSource::TinyEn),
        "Base" => Ok(rwhisper::WhisperSource::Base),
        "BaseEn" => Ok(rwhisper::WhisperSource::BaseEn),
        "Small" => Ok(rwhisper::WhisperSource::Small),
        "SmallEn" => Ok(rwhisper::WhisperSource::SmallEn),
        "Medium" => Ok(rwhisper::WhisperSource::Medium),
        "MediumEn" => Ok(rwhisper::WhisperSource::MediumEn),
        "Large" => Ok(rwhisper::WhisperSource::Large),
        "LargeV2" => Ok(rwhisper::WhisperSource::LargeV2),
        other => anyhow::bail!("Unknown whisper source: {}", other),
    }
}

// ─── Stdout/stderr suppressor ───────────────────────────────────────────────
//
// kalosm-common prints "Running on CPU, to run on GPU..." to stdout via println!,
// and hf-hub renders an indicatif download progress bar directly to stderr.
// Both bleed into the TUI (raw mode, alt screen). We suppress fd 1 AND fd 2
// during model loading. Download progress is still forwarded to the TUI via
// the `progress_tx` channel so onboarding can render its own bar.
//
// SAFETY: This is only called while the TUI is in alternate screen.
// The background preload in ui.rs is delayed 2s to guarantee this.
// Brief fd suppression during a render tick just means one skipped frame.

/// Temporarily redirect stdout AND stderr to /dev/null. Returns a guard that
/// restores both on drop.
#[cfg(unix)]
pub(crate) fn suppress_stdout() -> Option<StdoutGuard> {
    use std::os::unix::io::AsRawFd;
    unsafe {
        let stdout_fd = std::io::stdout().as_raw_fd();
        let stderr_fd = std::io::stderr().as_raw_fd();
        let saved_stdout = libc::dup(stdout_fd);
        if saved_stdout < 0 {
            return None;
        }
        let saved_stderr = libc::dup(stderr_fd);
        if saved_stderr < 0 {
            libc::close(saved_stdout);
            return None;
        }
        let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if devnull < 0 {
            libc::close(saved_stdout);
            libc::close(saved_stderr);
            return None;
        }
        libc::dup2(devnull, stdout_fd);
        libc::dup2(devnull, stderr_fd);
        libc::close(devnull);
        Some(StdoutGuard {
            saved_stdout,
            saved_stderr,
        })
    }
}

#[cfg(unix)]
pub(crate) struct StdoutGuard {
    saved_stdout: i32,
    saved_stderr: i32,
}

#[cfg(unix)]
impl Drop for StdoutGuard {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            let stdout_fd = std::io::stdout().as_raw_fd();
            let stderr_fd = std::io::stderr().as_raw_fd();
            libc::dup2(self.saved_stdout, stdout_fd);
            libc::dup2(self.saved_stderr, stderr_fd);
            libc::close(self.saved_stdout);
            libc::close(self.saved_stderr);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn suppress_stdout() -> Option<()> {
    None
}

// ─── Transcription engine ───────────────────────────────────────────────────

/// Local whisper transcription engine using rwhisper.
pub struct LocalWhisper {
    model: rwhisper::Whisper,
}

impl LocalWhisper {
    /// Build a whisper model for the given preset. Downloads on first use.
    pub async fn new(preset: &LocalModelPreset) -> Result<Self> {
        let source = parse_whisper_source(preset)?;
        tracing::info!("Local STT: loading rwhisper model ({})...", preset.repo_id);

        // Suppress stdout — kalosm-common prints "Running on CPU..." via println!
        let _stdout_guard = suppress_stdout();
        let model = rwhisper::WhisperBuilder::default()
            .with_source(source)
            .build_with_loading_handler(|progress| {
                tracing::debug!("rwhisper loading: {:?}", progress);
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load whisper model: {}", e))?;
        drop(_stdout_guard);

        tracing::info!("Local STT: rwhisper model loaded");
        Ok(Self { model })
    }

    /// Transcribe OGG/Opus or WAV audio bytes to text.
    pub async fn transcribe(&self, audio_bytes: &[u8]) -> Result<String> {
        let (samples, sample_rate) = decode_audio(audio_bytes)?;

        if samples.is_empty() {
            anyhow::bail!("No audio samples decoded");
        }

        // Resample to 16kHz if needed
        let mut audio_16k = if sample_rate == 16000 {
            samples
        } else {
            resample(&samples, sample_rate, 16000)?
        };

        // Sanitize: replace NaN/Inf with 0.0 to prevent candle tensor panics
        for s in audio_16k.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
        }

        // Whisper needs at least N_FFT (400) samples for a single FFT window.
        // Pad very short audio to 1 second (16000 samples) to avoid edge cases
        // in candle's mel spectrogram that can panic with panic=abort.
        const MIN_SAMPLES: usize = 16000; // 1 second at 16kHz
        if audio_16k.len() < MIN_SAMPLES {
            tracing::debug!(
                "Audio too short ({} samples), padding to {} samples",
                audio_16k.len(),
                MIN_SAMPLES
            );
            audio_16k.resize(MIN_SAMPLES, 0.0);
        }

        tracing::info!(
            "Local STT: feeding {} samples ({:.1}s) to rwhisper",
            audio_16k.len(),
            audio_16k.len() as f64 / 16000.0
        );

        // Create a rodio-compatible source from PCM samples
        let source = PcmSource::new(audio_16k, 16000);

        // Transcribe using rwhisper
        use futures::StreamExt;
        let mut task = self.model.transcribe(source);
        let mut text = String::new();
        while let Some(segment) = task.next().await {
            text.push_str(segment.text());
        }

        let cleaned = clean_transcript(&text);
        tracing::info!("Local STT: transcribed {} chars", cleaned.len());
        Ok(cleaned)
    }
}

/// A rodio-compatible Source that wraps PCM f32 samples.
struct PcmSource {
    samples: Vec<f32>,
    pos: usize,
    sample_rate: u32,
}

impl PcmSource {
    fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            pos: 0,
            sample_rate,
        }
    }
}

impl Iterator for PcmSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pos < self.samples.len() {
            let s = self.samples[self.pos];
            self.pos += 1;
            Some(s)
        } else {
            None
        }
    }
}

impl rodio::Source for PcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.len() - self.pos)
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs_f64(
            self.samples.len() as f64 / self.sample_rate as f64,
        ))
    }
}

/// Clean up whisper transcript output — collapse whitespace and trim.
fn clean_transcript(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Decode audio bytes (OGG or WAV) to f32 mono PCM samples + sample rate.
fn decode_audio(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    // Try WAV first (hound)
    if bytes.len() >= 4 && &bytes[..4] == b"RIFF" {
        return decode_wav(bytes);
    }

    // Try OGG via symphonia
    decode_ogg(bytes)
}

/// Decode WAV using hound.
fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = hound::WavReader::new(cursor).context("Failed to parse WAV")?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.unwrap_or(0) as f32 / i16::MAX as f32)
            .collect(),
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
    };
    // Mix to mono if stereo
    let mono = if spec.channels > 1 {
        samples
            .chunks(spec.channels as usize)
            .map(|ch| ch.iter().sum::<f32>() / ch.len() as f32)
            .collect()
    } else {
        samples
    };
    Ok((mono, spec.sample_rate))
}

/// Decode OGG (Vorbis or Opus) using symphonia.
fn decode_ogg(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CodecRegistry, DecoderOptions};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let mut codec_registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut codec_registry);
    codec_registry.register_all::<symphonia_adapter_libopus::OpusDecoder>();

    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("ogg");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("Failed to probe audio format")?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("No audio track found"))?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow::anyhow!("Unknown sample rate"))?;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);
    let track_id = track.id;

    let mut decoder = codec_registry
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Failed to create audio decoder")?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => {
                tracing::debug!("Audio decode packet error (continuing): {}", e);
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("Audio decode error (skipping packet): {}", e);
                continue;
            }
        };

        let mut sample_buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        sample_buf.copy_interleaved_ref(decoded);
        let interleaved = sample_buf.samples();

        if channels > 1 {
            for chunk in interleaved.chunks(channels) {
                all_samples.push(chunk.iter().sum::<f32>() / chunk.len() as f32);
            }
        } else {
            all_samples.extend_from_slice(interleaved);
        }
    }

    Ok((all_samples, sample_rate))
}

/// Resample audio from one sample rate to another.
fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>> {
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let ratio = to_rate as f64 / from_rate as f64;
    let chunk_size = 1024;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, chunk_size, 1)
        .map_err(|e| anyhow::anyhow!("Resampler init error: {}", e))?;

    let mut output = Vec::with_capacity((input.len() as f64 * ratio) as usize + 1024);
    let mut pos = 0;

    while pos + chunk_size <= input.len() {
        let chunk = &input[pos..pos + chunk_size];
        let result = resampler
            .process(&[chunk], None)
            .map_err(|e| anyhow::anyhow!("Resample error: {}", e))?;
        output.extend_from_slice(&result[0]);
        pos += chunk_size;
    }

    if pos < input.len() {
        let remaining = &input[pos..];
        let result = resampler
            .process_partial(Some(&[remaining]), None)
            .map_err(|e| anyhow::anyhow!("Resample error: {}", e))?;
        output.extend_from_slice(&result[0]);
    }

    Ok(output)
}

/// Compute mel filterbank coefficients matching OpenAI whisper's implementation.
/// Returns a flat Vec of n_mels * n_freqs f32 values (row-major: filters[mel_idx * n_freqs + freq_idx]).
pub fn compute_mel_filters(n_mels: usize, n_fft: usize, sample_rate: u32) -> Vec<f32> {
    let n_freqs = n_fft / 2 + 1;
    let sr = sample_rate as f64;

    let hz_to_mel = |f: f64| -> f64 { 2595.0 * (1.0 + f / 700.0).log10() };
    let mel_to_hz = |m: f64| -> f64 { 700.0 * (10f64.powf(m / 2595.0) - 1.0) };

    let all_freqs: Vec<f64> = (0..n_freqs)
        .map(|i| sr / 2.0 * i as f64 / (n_freqs - 1) as f64)
        .collect();

    let m_min = hz_to_mel(0.0);
    let m_max = hz_to_mel(sr / 2.0);
    let m_pts: Vec<f64> = (0..n_mels + 2)
        .map(|i| m_min + (m_max - m_min) * i as f64 / (n_mels + 1) as f64)
        .collect();
    let f_pts: Vec<f64> = m_pts.iter().map(|&m| mel_to_hz(m)).collect();

    let mut filters = vec![0.0f32; n_mels * n_freqs];
    for i in 0..n_mels {
        let f_prev = f_pts[i];
        let f_curr = f_pts[i + 1];
        let f_next = f_pts[i + 2];
        // Slaney-style normalization
        let enorm = if f_next != f_prev {
            2.0 / (f_next - f_prev)
        } else {
            1.0
        };
        for j in 0..n_freqs {
            let freq = all_freqs[j];
            let v = if freq >= f_prev && freq <= f_curr && f_curr != f_prev {
                (freq - f_prev) / (f_curr - f_prev)
            } else if freq > f_curr && freq <= f_next && f_next != f_curr {
                (f_next - freq) / (f_next - f_curr)
            } else {
                0.0
            };
            filters[i * n_freqs + j] = (v * enorm) as f32;
        }
    }
    filters
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── clean_transcript ─────────────────────────────────────────────────

    #[test]
    fn clean_transcript_collapses_whitespace() {
        assert_eq!(clean_transcript("  hello   world  "), "hello world");
    }

    #[test]
    fn clean_transcript_handles_newlines_and_tabs() {
        assert_eq!(clean_transcript("hello\n\tworld\n"), "hello world");
    }

    #[test]
    fn clean_transcript_empty_input() {
        assert_eq!(clean_transcript(""), "");
        assert_eq!(clean_transcript("   "), "");
    }

    // ─── PcmSource (rodio::Source impl) ───────────────────────────────────

    #[test]
    fn pcm_source_iterates_all_samples() {
        let samples = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let mut source = PcmSource::new(samples.clone(), 16000);
        let collected: Vec<f32> = std::iter::from_fn(|| source.next()).collect();
        assert_eq!(collected, samples);
    }

    #[test]
    fn pcm_source_empty() {
        let mut source = PcmSource::new(vec![], 16000);
        assert!(source.next().is_none());
    }

    #[test]
    fn pcm_source_rodio_metadata() {
        use rodio::Source;
        let source = PcmSource::new(vec![0.0; 16000], 16000);
        assert_eq!(source.channels(), 1);
        assert_eq!(source.sample_rate(), 16000);
        assert_eq!(
            source.total_duration(),
            Some(std::time::Duration::from_secs(1))
        );
        assert_eq!(source.current_frame_len(), Some(16000));
    }

    #[test]
    fn pcm_source_frame_len_decreases() {
        use rodio::Source;
        let mut source = PcmSource::new(vec![0.0; 10], 16000);
        assert_eq!(source.current_frame_len(), Some(10));
        source.next();
        assert_eq!(source.current_frame_len(), Some(9));
    }

    // ─── parse_whisper_source ─────────────────────────────────────────────

    #[test]
    fn parse_all_preset_sources() {
        for preset in LOCAL_MODEL_PRESETS {
            let result = parse_whisper_source(preset);
            assert!(
                result.is_ok(),
                "Failed to parse source for preset '{}': {:?}",
                preset.id,
                result.err()
            );
        }
    }

    #[test]
    fn parse_unknown_source_fails() {
        let fake = LocalModelPreset {
            id: "fake",
            label: "Fake",
            file_name: "fake",
            size_label: "0",
            repo_id: "NonExistent",
        };
        let result = parse_whisper_source(&fake);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown whisper source")
        );
    }

    // ─── is_model_downloaded (rwhisper always returns true) ───────────────

    #[test]
    fn is_model_downloaded_always_true() {
        for preset in LOCAL_MODEL_PRESETS {
            assert!(
                is_model_downloaded(preset),
                "is_model_downloaded should always be true for rwhisper presets"
            );
        }
    }

    // ─── decode_audio format detection ────────────────────────────────────

    #[test]
    fn decode_audio_empty_fails() {
        let result = decode_audio(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_audio_garbage_fails() {
        let result = decode_audio(&[0xFF, 0xFE, 0xFD, 0xFC, 0xFB]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_wav_valid_sine() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut writer = hound::WavWriter::new(cursor, spec).unwrap();
            for i in 0..1600 {
                let t = i as f32 / 16000.0;
                let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin();
                writer
                    .write_sample((sample * i16::MAX as f32) as i16)
                    .unwrap();
            }
            writer.finalize().unwrap();
        }

        let (samples, rate) = decode_audio(&buf).unwrap();
        assert_eq!(rate, 16000);
        assert_eq!(samples.len(), 1600);
        // First sample near zero (sin(0) = 0)
        assert!(samples[0].abs() < 0.01);
    }

    #[test]
    fn decode_wav_stereo_mixes_to_mono() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut writer = hound::WavWriter::new(cursor, spec).unwrap();
            for _ in 0..100 {
                writer.write_sample(1000i16).unwrap(); // left
                writer.write_sample(3000i16).unwrap(); // right
            }
            writer.finalize().unwrap();
        }

        let (samples, rate) = decode_audio(&buf).unwrap();
        assert_eq!(rate, 16000);
        assert_eq!(samples.len(), 100); // stereo → mono
        // Each sample should be average of 1000 and 3000 = 2000
        let expected = 2000.0 / i16::MAX as f32;
        assert!((samples[0] - expected).abs() < 0.001);
    }

    // ─── resample ─────────────────────────────────────────────────────────

    #[test]
    fn resample_48k_to_16k() {
        // Generate 0.2s of 48kHz sine wave
        let input: Vec<f32> = (0..9600)
            .map(|i| (i as f32 / 48000.0 * 440.0 * 2.0 * std::f32::consts::PI).sin())
            .collect();
        let output = resample(&input, 48000, 16000).unwrap();
        // Should be roughly 1/3 the length (16k/48k)
        let expected_len = (input.len() as f64 * 16000.0 / 48000.0) as usize;
        assert!(
            (output.len() as i64 - expected_len as i64).unsigned_abs() < 256,
            "Expected ~{} samples, got {}",
            expected_len,
            output.len()
        );
    }

    #[test]
    fn resample_preserves_non_silence() {
        let input: Vec<f32> = (0..4800)
            .map(|i| (i as f32 / 48000.0 * 440.0 * 2.0 * std::f32::consts::PI).sin())
            .collect();
        let output = resample(&input, 48000, 16000).unwrap();
        let rms: f32 = (output.iter().map(|s| s * s).sum::<f32>() / output.len() as f32).sqrt();
        assert!(
            rms > 0.1,
            "Resampled audio should not be silence, RMS={}",
            rms
        );
    }

    // ─── DownloadProgress struct ──────────────────────────────────────────

    #[test]
    fn download_progress_done_state() {
        let p = DownloadProgress {
            downloaded: 42_000_000,
            total: Some(42_000_000),
            done: true,
            error: None,
        };
        assert!(p.done);
        assert_eq!(p.downloaded, p.total.unwrap());
        assert!(p.error.is_none());
    }

    #[test]
    fn download_progress_error_state() {
        let p = DownloadProgress {
            downloaded: 0,
            total: None,
            done: false,
            error: Some("network timeout".to_string()),
        };
        assert!(!p.done);
        assert!(p.error.is_some());
    }

    // ─── Audio sanitization ──────────────────────────────────────────────

    #[test]
    fn sanitize_nan_inf_in_audio() {
        // Simulate the NaN/Inf scrubbing done in transcribe()
        let mut samples = vec![0.5, f32::NAN, -0.3, f32::INFINITY, f32::NEG_INFINITY, 0.1];
        for s in samples.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
        }
        assert_eq!(samples, vec![0.5, 0.0, -0.3, 0.0, 0.0, 0.1]);
    }

    #[test]
    fn short_audio_padded_to_minimum() {
        // Simulate the min-padding logic from transcribe()
        const MIN_SAMPLES: usize = 16000;
        let mut audio = vec![0.5f32; 100]; // very short
        if audio.len() < MIN_SAMPLES {
            audio.resize(MIN_SAMPLES, 0.0);
        }
        assert_eq!(audio.len(), MIN_SAMPLES);
        assert_eq!(audio[0], 0.5); // original data preserved
        assert_eq!(audio[100], 0.0); // padded with silence
        assert_eq!(audio[15999], 0.0);
    }

    #[test]
    fn audio_at_minimum_not_padded() {
        const MIN_SAMPLES: usize = 16000;
        let mut audio = vec![0.1f32; MIN_SAMPLES];
        let original_len = audio.len();
        if audio.len() < MIN_SAMPLES {
            audio.resize(MIN_SAMPLES, 0.0);
        }
        assert_eq!(audio.len(), original_len);
    }

    #[test]
    fn audio_above_minimum_not_padded() {
        const MIN_SAMPLES: usize = 16000;
        let mut audio = vec![0.1f32; 48000]; // 3 seconds
        let original_len = audio.len();
        if audio.len() < MIN_SAMPLES {
            audio.resize(MIN_SAMPLES, 0.0);
        }
        assert_eq!(audio.len(), original_len);
    }

    // ─── Default preset is QuantizedTiny (multilingual) ───────────────────

    #[test]
    fn default_preset_is_quantized_tiny() {
        let preset = find_local_model("local-tiny").unwrap();
        assert_eq!(preset.repo_id, "QuantizedTiny");
        assert!(preset.label.contains("Multilingual"));
        assert!(preset.label.contains("Quantized"));
    }

    #[test]
    fn all_presets_have_unique_ids() {
        let mut ids: Vec<&str> = LOCAL_MODEL_PRESETS.iter().map(|p| p.id).collect();
        let len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), len, "Preset IDs must be unique");
    }

    #[test]
    fn model_path_under_opencrabs_dir() {
        let preset = &LOCAL_MODEL_PRESETS[0];
        let path = model_path(preset);
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("opencrabs"),
            "Path should be under opencrabs dir"
        );
        assert!(
            path_str.contains("whisper"),
            "Path should be under whisper subdir"
        );
        assert!(path_str.ends_with(preset.file_name));
    }
}
