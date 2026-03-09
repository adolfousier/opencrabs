//! Local STT via candle whisper (pure Rust)
//!
//! Model presets, download, and transcription engine.
//! Gated behind the `local-stt` feature flag.
//!
//! Uses candle-transformers' whisper implementation instead of whisper-rs/whisper.cpp
//! to avoid ggml symbol conflicts with llama-cpp-sys-2 (issue #38).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ─── Model presets ──────────────────────────────────────────────────────────

/// A local whisper model preset.
pub struct LocalModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub file_name: &'static str,
    pub size_label: &'static str,
    /// HuggingFace repo ID for candle model download.
    pub repo_id: &'static str,
}

/// Available local whisper model sizes.
/// Models are downloaded from HuggingFace in safetensors format.
pub const LOCAL_MODEL_PRESETS: &[LocalModelPreset] = &[
    LocalModelPreset {
        id: "local-tiny",
        label: "Tiny",
        file_name: "tiny.en",
        size_label: "~75 MB",
        repo_id: "openai/whisper-tiny.en",
    },
    LocalModelPreset {
        id: "local-base",
        label: "Base",
        file_name: "base.en",
        size_label: "~142 MB",
        repo_id: "openai/whisper-base.en",
    },
    LocalModelPreset {
        id: "local-small",
        label: "Small",
        file_name: "small.en",
        size_label: "~466 MB",
        repo_id: "openai/whisper-small.en",
    },
    LocalModelPreset {
        id: "local-medium",
        label: "Medium",
        file_name: "medium.en",
        size_label: "~1.5 GB",
        repo_id: "openai/whisper-medium.en",
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
/// For candle models we check if the config.json exists in the model dir.
pub fn is_model_downloaded(preset: &LocalModelPreset) -> bool {
    let dir = model_path(preset);
    dir.join("config.json").exists() && dir.join("model.safetensors").exists()
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

/// Download a model from HuggingFace Hub.
///
/// Downloads config.json, tokenizer.json, and model.safetensors.
pub async fn download_model(
    preset: &LocalModelPreset,
    progress_tx: tokio::sync::mpsc::UnboundedSender<DownloadProgress>,
) -> Result<PathBuf> {
    let dest = model_path(preset);
    std::fs::create_dir_all(&dest).ok();

    tracing::info!(
        "Downloading whisper model {} from HuggingFace ({})",
        preset.id,
        preset.repo_id
    );

    let files_to_download = ["config.json", "tokenizer.json", "model.safetensors"];

    let mut total_downloaded: u64 = 0;

    for file_name in &files_to_download {
        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            preset.repo_id, file_name
        );
        let file_path = dest.join(file_name);

        // Skip if already exists
        if file_path.exists() {
            tracing::debug!("Skipping {} (already exists)", file_name);
            continue;
        }

        let part_path = file_path.with_extension("part");

        let response = reqwest::get(&url)
            .await
            .with_context(|| format!("Failed to download {}", file_name))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Download failed for {}: HTTP {}",
                file_name,
                response.status()
            );
        }

        let file_total = response.content_length();
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(&part_path)
            .await
            .context("Failed to create temp file")?;
        let mut file_downloaded: u64 = 0;

        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Download stream error")?;
            file.write_all(&chunk)
                .await
                .context("Failed to write chunk")?;
            file_downloaded += chunk.len() as u64;
            total_downloaded += chunk.len() as u64;
            let _ = progress_tx.send(DownloadProgress {
                downloaded: total_downloaded,
                total: file_total.map(|t| t + total_downloaded - file_downloaded),
                done: false,
                error: None,
            });
        }
        file.flush().await?;
        drop(file);

        tokio::fs::rename(&part_path, &file_path)
            .await
            .with_context(|| format!("Failed to rename {}", file_name))?;

        tracing::info!("Downloaded {} ({} bytes)", file_name, file_downloaded);
    }

    let _ = progress_tx.send(DownloadProgress {
        downloaded: total_downloaded,
        total: Some(total_downloaded),
        done: true,
        error: None,
    });

    tracing::info!(
        "Whisper model {} downloaded ({} total bytes)",
        preset.id,
        total_downloaded
    );
    Ok(dest)
}

// ─── Transcription engine ───────────────────────────────────────────────────

/// Local whisper transcription engine using candle.
pub struct LocalWhisper {
    model: candle_transformers::models::whisper::model::Whisper,
    tokenizer: tokenizers::Tokenizer,
    config: candle_transformers::models::whisper::Config,
    device: candle_core::Device,
    mel_filters: Vec<f32>,
}

impl LocalWhisper {
    /// Load a whisper model from a directory containing config.json,
    /// tokenizer.json, and model.safetensors.
    pub fn new(model_dir: &Path) -> Result<Self> {
        use candle_core::Device;
        use candle_nn::VarBuilder;

        let device = Device::Cpu;

        // Load config
        let config_path = model_dir.join("config.json");
        let config_str =
            std::fs::read_to_string(&config_path).context("Failed to read config.json")?;
        let config: candle_transformers::models::whisper::Config =
            serde_json::from_str(&config_str).context("Failed to parse config.json")?;

        // Load tokenizer
        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        // Load model weights
        let model_path = model_dir.join("model.safetensors");
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_path], candle_core::DType::F32, &device)
                .context("Failed to load model weights")?
        };

        let model = candle_transformers::models::whisper::model::Whisper::load(&vb, config.clone())
            .context("Failed to build whisper model")?;

        // Load mel filters from embedded binary data
        let mel_bytes = match config.num_mel_bins {
            80 => include_bytes!("mel_filters_80.bin").as_slice(),
            128 => include_bytes!("mel_filters_128.bin").as_slice(),
            n => anyhow::bail!("Unsupported mel filter count: {}", n),
        };
        let mut mel_filters = vec![0f32; mel_bytes.len() / 4];
        <byteorder::LittleEndian as byteorder::ByteOrder>::read_f32_into(
            mel_bytes,
            &mut mel_filters,
        );

        Ok(Self {
            model,
            tokenizer,
            config,
            device,
            mel_filters,
        })
    }

    /// Transcribe OGG/Opus or WAV audio bytes to text.
    pub fn transcribe(&mut self, audio_bytes: &[u8]) -> Result<String> {
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

        // Compute log-mel spectrogram
        let mel = candle_transformers::models::whisper::audio::pcm_to_mel(
            &self.config,
            &audio_16k,
            &self.mel_filters,
        );
        let mel_len = mel.len();
        let num_mel_bins = self.config.num_mel_bins;
        let n_frames = mel_len / num_mel_bins;
        let mel = candle_core::Tensor::from_vec(mel, (1, num_mel_bins, n_frames), &self.device)
            .context("Failed to create mel tensor")?;

        // Reset KV cache before new transcription
        self.model.reset_kv_cache();

        // Encode audio
        let audio_features = self
            .model
            .encoder
            .forward(&mel, true)
            .context("Encoder forward pass failed")?;

        // Build initial token sequence for decoding
        let vocab = self.tokenizer.get_vocab(true);
        let sot = *vocab
            .get(candle_transformers::models::whisper::SOT_TOKEN)
            .unwrap_or(&50258u32);
        let transcribe = *vocab
            .get(candle_transformers::models::whisper::TRANSCRIBE_TOKEN)
            .unwrap_or(&50359);
        let no_timestamps = *vocab
            .get(candle_transformers::models::whisper::NO_TIMESTAMPS_TOKEN)
            .unwrap_or(&50363);
        let eot = *vocab
            .get(candle_transformers::models::whisper::EOT_TOKEN)
            .unwrap_or(&50257);
        let en = *vocab.get("<|en|>").unwrap_or(&50259);

        let mut tokens: Vec<u32> = vec![sot, en, transcribe, no_timestamps];
        let max_tokens = 224;

        for _ in 0..max_tokens {
            let token_tensor =
                candle_core::Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;

            let logits = self
                .model
                .decoder
                .forward(&token_tensor, &audio_features, tokens.len() == 4)
                .context("Decoder forward pass failed")?;

            // Get logits for the last token position
            let logits = self.model.decoder.final_linear(&logits)?;
            let logits = logits.squeeze(0)?;
            let last_logits = logits.get(logits.dim(0)? - 1)?;

            let next_token = last_logits.argmax(0)?.to_scalar::<u32>()?;

            if next_token == eot {
                break;
            }

            tokens.push(next_token);
        }

        // Decode tokens to text (skip the initial prompt tokens)
        let output_tokens: Vec<u32> = tokens[4..].to_vec();
        let text = self
            .tokenizer
            .decode(&output_tokens, true)
            .map_err(|e| anyhow::anyhow!("Tokenizer decode error: {}", e))?;

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
