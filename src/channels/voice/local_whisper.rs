//! Local STT via whisper.cpp (whisper-rs)
//!
//! Model presets, download, and transcription engine.
//! Gated behind the `local-stt` feature flag.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ─── Model presets ──────────────────────────────────────────────────────────

/// A local whisper model preset.
pub struct LocalModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub file_name: &'static str,
    pub size_label: &'static str,
}

/// Available local whisper model sizes.
pub const LOCAL_MODEL_PRESETS: &[LocalModelPreset] = &[
    LocalModelPreset {
        id: "local-tiny",
        label: "Tiny",
        file_name: "ggml-tiny.en.bin",
        size_label: "~75 MB",
    },
    LocalModelPreset {
        id: "local-base",
        label: "Base",
        file_name: "ggml-base.en.bin",
        size_label: "~142 MB",
    },
    LocalModelPreset {
        id: "local-small",
        label: "Small",
        file_name: "ggml-small.en.bin",
        size_label: "~466 MB",
    },
    LocalModelPreset {
        id: "local-medium",
        label: "Medium",
        file_name: "ggml-medium.en.bin",
        size_label: "~1.5 GB",
    },
];

/// Look up a model preset by ID.
pub fn find_local_model(id: &str) -> Option<&'static LocalModelPreset> {
    LOCAL_MODEL_PRESETS.iter().find(|m| m.id == id)
}

/// HuggingFace download URL for a whisper model file.
pub fn model_url(file_name: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        file_name
    )
}

/// Directory where local models are stored.
pub fn models_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("opencrabs").join("models").join("whisper");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Full path for a model preset.
pub fn model_path(preset: &LocalModelPreset) -> PathBuf {
    models_dir().join(preset.file_name)
}

/// Check if a model is already downloaded.
pub fn is_model_downloaded(preset: &LocalModelPreset) -> bool {
    model_path(preset).exists()
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

/// Download a model file with progress reporting.
///
/// Writes to a `.part` temp file, then renames atomically on success.
pub async fn download_model(
    preset: &LocalModelPreset,
    progress_tx: tokio::sync::mpsc::UnboundedSender<DownloadProgress>,
) -> Result<PathBuf> {
    use futures::StreamExt;

    let url = model_url(preset.file_name);
    let dest = model_path(preset);
    let part = dest.with_extension("part");

    tracing::info!("Downloading whisper model {} from {}", preset.id, url);

    let response = reqwest::get(&url)
        .await
        .context("Failed to start model download")?;

    if !response.status().is_success() {
        let status = response.status();
        anyhow::bail!("Download failed: HTTP {}", status);
    }

    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(&part)
        .await
        .context("Failed to create temp file")?;
    let mut downloaded: u64 = 0;

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Download stream error")?;
        file.write_all(&chunk)
            .await
            .context("Failed to write chunk")?;
        downloaded += chunk.len() as u64;
        let _ = progress_tx.send(DownloadProgress {
            downloaded,
            total,
            done: false,
            error: None,
        });
    }
    file.flush().await?;
    drop(file);

    // Atomic rename
    tokio::fs::rename(&part, &dest)
        .await
        .context("Failed to rename model file")?;

    let _ = progress_tx.send(DownloadProgress {
        downloaded,
        total,
        done: true,
        error: None,
    });

    tracing::info!(
        "Whisper model {} downloaded ({} bytes)",
        preset.id,
        downloaded
    );
    Ok(dest)
}

// ─── Transcription engine ───────────────────────────────────────────────────

/// Local whisper transcription engine.
pub struct LocalWhisper {
    ctx: whisper_rs::WhisperContext,
}

impl LocalWhisper {
    /// Load a whisper model from disk.
    pub fn new(model_path: &Path) -> Result<Self> {
        // Suppress whisper.cpp / ggml stderr output that bleeds into the TUI.
        // Must be called before any whisper context is created.
        suppress_whisper_logs();

        let path_str = model_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Model path is not valid UTF-8"))?;
        let ctx = whisper_rs::WhisperContext::new_with_params(
            path_str,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| anyhow::anyhow!("Failed to load whisper model: {}", e))?;
        Ok(Self { ctx })
    }

    /// Transcribe OGG/Opus or WAV audio bytes to text.
    pub fn transcribe(&self, audio_bytes: &[u8]) -> Result<String> {
        let (samples, sample_rate) = decode_audio(audio_bytes)?;

        if samples.is_empty() {
            anyhow::bail!("No audio samples decoded");
        }

        // Resample to 16kHz if needed
        let audio_16k = if sample_rate == 16000 {
            samples
        } else {
            resample(&samples, sample_rate, 16000)?
        };

        // Run whisper inference
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| anyhow::anyhow!("Failed to create whisper state: {}", e))?;
        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, &audio_16k)
            .map_err(|e| anyhow::anyhow!("Whisper inference failed: {}", e))?;

        let mut text = String::new();
        for segment in state.as_iter() {
            if let Ok(s) = segment.to_str() {
                text.push_str(s);
            }
        }

        Ok(clean_transcript(&text))
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

    // Build a codec registry that includes Opus (via libopus adapter)
    // on top of symphonia's default codecs.
    let mut codec_registry = CodecRegistry::new();
    // Register all default codecs (Vorbis, PCM, etc.)
    symphonia::default::register_enabled_codecs(&mut codec_registry);
    // Register Opus decoder
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

        // Mix to mono if multi-channel
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

/// Redirect all whisper.cpp + ggml log output to a no-op callback.
///
/// Without this, whisper.cpp dumps verbose model-loading and inference debug
/// lines (token probabilities, timestamps, decoder state) to stderr, which
/// bleeds into the TUI and makes the display unreadable.
fn suppress_whisper_logs() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Safety: the callback is a plain C function that does nothing —
        // no panics, no allocations, no unwinding.
        unsafe {
            // ggml_log_level is c_uint on Unix but c_int on Windows.
            // Transmute the noop callback to match WhisperLogCallback.
            // Sound because the callback ignores all arguments.
            let cb: whisper_rs::WhisperLogCallback =
                std::mem::transmute(noop_log_callback as *const ());
            whisper_rs::set_log_callback(cb, std::ptr::null_mut());
        }
    });
}

/// C-compatible no-op log callback. Uses `c_int` as a placeholder —
/// transmuted to match the platform-specific `ggml_log_level` at the call site.
unsafe extern "C" fn noop_log_callback(
    _level: std::os::raw::c_int,
    _text: *const std::ffi::c_char,
    _user_data: *mut std::ffi::c_void,
) {
    // Intentionally empty — swallow all whisper.cpp / ggml log output.
}
