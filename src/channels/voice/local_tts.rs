//! Local TTS via Piper (piper-tts)
//!
//! Text-to-speech using the Piper project (Python venv + ONNX voice models).
//! No external Rust crate — spawns `piper` subprocess with text on stdin,
//! reads raw 16-bit PCM from stdout, encodes to OGG/Opus for channel delivery.
//! Gated behind the `local-tts` feature flag.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ─── Voice presets ────────────────────────────────────────────────────────────

/// A Piper voice preset.
pub struct PiperVoice {
    pub id: &'static str,
    pub label: &'static str,
    pub locale: &'static str,
    pub name: &'static str,
    pub quality: &'static str,
}

impl PiperVoice {
    /// HuggingFace URL for the ONNX model file.
    pub fn onnx_url(&self) -> String {
        format!(
            "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/{}/{}/{}/{}-{}-{}.onnx",
            self.locale, self.name, self.quality, self.locale, self.name, self.quality
        )
    }
    /// HuggingFace URL for the ONNX config file.
    pub fn config_url(&self) -> String {
        format!("{}.json", self.onnx_url())
    }
}

/// Available Piper voice presets.
pub const PIPER_VOICES: &[PiperVoice] = &[
    PiperVoice {
        id: "ryan",
        label: "Ryan (US Male)",
        locale: "en_US",
        name: "ryan",
        quality: "medium",
    },
    PiperVoice {
        id: "amy",
        label: "Amy (US Female)",
        locale: "en_US",
        name: "amy",
        quality: "medium",
    },
    PiperVoice {
        id: "lessac",
        label: "Lessac (US Female)",
        locale: "en_US",
        name: "lessac",
        quality: "medium",
    },
    PiperVoice {
        id: "kristin",
        label: "Kristin (US Female)",
        locale: "en_US",
        name: "kristin",
        quality: "medium",
    },
    PiperVoice {
        id: "joe",
        label: "Joe (US Male)",
        locale: "en_US",
        name: "joe",
        quality: "medium",
    },
    PiperVoice {
        id: "cori",
        label: "Cori (UK Female)",
        locale: "en_GB",
        name: "cori",
        quality: "medium",
    },
];

/// Look up a Piper voice preset by ID.
pub fn find_piper_voice(id: &str) -> Option<&'static PiperVoice> {
    PIPER_VOICES.iter().find(|v| v.id == id)
}

// ─── Paths & checks ──────────────────────────────────────────────────────────

/// Directory where Piper venv and voice models are stored.
pub fn piper_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("opencrabs").join("models").join("piper");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Check if the Piper venv is installed.
pub fn piper_venv_exists() -> bool {
    piper_bin_path().exists()
}

/// Path to the piper binary inside the venv.
fn piper_bin_path() -> PathBuf {
    let dir = piper_dir();
    if cfg!(target_os = "windows") {
        dir.join("venv").join("Scripts").join("piper.exe")
    } else {
        dir.join("venv").join("bin").join("piper")
    }
}

/// Check if a specific voice model is downloaded.
pub fn piper_voice_exists(voice_id: &str) -> bool {
    let dir = piper_dir();
    dir.join(format!("{voice_id}.onnx")).exists()
        && dir.join(format!("{voice_id}.onnx.json")).exists()
}

/// Delete a downloaded Piper voice model (ONNX + config).
pub fn delete_voice(voice_id: &str) {
    let dir = piper_dir();
    let onnx = dir.join(format!("{voice_id}.onnx"));
    let config = dir.join(format!("{voice_id}.onnx.json"));
    let _ = std::fs::remove_file(&onnx);
    let _ = std::fs::remove_file(&config);
    tracing::info!("Deleted Piper voice '{voice_id}'");
}

/// Delete all downloaded Piper voices except the given one.
pub fn delete_other_voices(keep_id: &str) {
    for voice in PIPER_VOICES {
        if voice.id != keep_id && piper_voice_exists(voice.id) {
            delete_voice(voice.id);
        }
    }
}

// ─── Venv setup ───────────────────────────────────────────────────────────────

/// Set up the Piper Python venv and install piper-tts.
pub async fn setup_piper_venv(
    progress_tx: tokio::sync::mpsc::UnboundedSender<SetupProgress>,
) -> Result<()> {
    let dir = piper_dir();
    let venv_dir = dir.join("venv");

    if piper_venv_exists() {
        tracing::info!("Piper venv already exists, skipping setup");
        return Ok(());
    }

    let _ = progress_tx.send(SetupProgress {
        stage: "Creating Python venv...".to_string(),
        done: false,
        error: None,
    });

    // Create venv
    let python = if cfg!(target_os = "windows") {
        "python"
    } else {
        "python3"
    };

    let output = tokio::process::Command::new(python)
        .args(["-m", "venv"])
        .arg(&venv_dir)
        .output()
        .await
        .context("Failed to create Python venv — is python3 installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let hint = if stderr.contains("ensurepip") || stderr.contains("No module named") {
            if cfg!(target_os = "linux") {
                "\n\nFix: install the venv module with:\n  sudo apt install python3-venv   # Debian/Ubuntu\n  sudo dnf install python3        # Fedora"
            } else if cfg!(target_os = "macos") {
                "\n\nFix: reinstall Python with:\n  brew install python3"
            } else {
                "\n\nFix: install Python 3 with the venv module included."
            }
        } else {
            ""
        };
        anyhow::bail!("python3 -m venv failed: {stderr}{hint}");
    }

    let _ = progress_tx.send(SetupProgress {
        stage: "Installing piper-tts...".to_string(),
        done: false,
        error: None,
    });

    // Install piper-tts
    let pip = if cfg!(target_os = "windows") {
        venv_dir.join("Scripts").join("pip.exe")
    } else {
        venv_dir.join("bin").join("pip")
    };

    let output = tokio::process::Command::new(&pip)
        .args(["install", "piper-tts", "pathvalidate"])
        .output()
        .await
        .context("Failed to install piper-tts")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("pip install piper-tts failed: {stderr}");
    }

    let _ = progress_tx.send(SetupProgress {
        stage: "Piper TTS installed".to_string(),
        done: true,
        error: None,
    });

    tracing::info!("Piper venv setup complete at {}", venv_dir.display());
    Ok(())
}

/// Setup progress info.
#[derive(Debug, Clone)]
pub struct SetupProgress {
    pub stage: String,
    pub done: bool,
    pub error: Option<String>,
}

// ─── Voice model download ─────────────────────────────────────────────────────

/// Download progress info.
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
}

/// Download a Piper voice model (ONNX + config) with progress reporting.
pub async fn download_voice(
    voice_id: &str,
    progress_tx: tokio::sync::mpsc::UnboundedSender<DownloadProgress>,
) -> Result<()> {
    let preset = find_piper_voice(voice_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown Piper voice: {voice_id}"))?;

    let dir = piper_dir();
    let onnx_dest = dir.join(format!("{voice_id}.onnx"));
    let config_dest = dir.join(format!("{voice_id}.onnx.json"));

    // Download ONNX model
    download_file(&preset.onnx_url(), &onnx_dest, &progress_tx).await?;

    // Download config
    download_file(&preset.config_url(), &config_dest, &progress_tx).await?;

    let _ = progress_tx.send(DownloadProgress {
        downloaded: 0,
        total: None,
        done: true,
        error: None,
    });

    tracing::info!("Piper voice '{}' downloaded", voice_id);
    Ok(())
}

/// Download a single file with progress reporting. Writes to `.part` then renames.
async fn download_file(
    url: &str,
    dest: &Path,
    progress_tx: &tokio::sync::mpsc::UnboundedSender<DownloadProgress>,
) -> Result<()> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let part = dest.with_extension("part");

    tracing::info!("Downloading {} to {}", url, dest.display());

    let response = reqwest::get(url)
        .await
        .context("Failed to start download")?;

    if !response.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(&part)
        .await
        .context("Failed to create temp file")?;
    let mut downloaded: u64 = 0;

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
    tokio::fs::rename(&part, dest)
        .await
        .context("Failed to rename downloaded file")?;

    Ok(())
}

// ─── TTS engine ───────────────────────────────────────────────────────────────

/// Local Piper TTS engine.
pub struct PiperTts {
    piper_bin: PathBuf,
    model_path: PathBuf,
    sample_rate: u32,
}

impl PiperTts {
    /// Load Piper with a specific voice model.
    pub fn new(voice_id: &str) -> Result<Self> {
        let dir = piper_dir();
        let piper_bin = piper_bin_path();
        let model_path = dir.join(format!("{voice_id}.onnx"));
        let config_path = dir.join(format!("{voice_id}.onnx.json"));

        if !piper_bin.exists() {
            anyhow::bail!("Piper not installed. Run /onboard:voice to set up local TTS.");
        }
        if !model_path.exists() {
            anyhow::bail!(
                "Piper voice '{}' not downloaded. Run /onboard:voice to download.",
                voice_id
            );
        }

        let sample_rate = if config_path.exists() {
            let config_str =
                std::fs::read_to_string(&config_path).context("Failed to read voice config")?;
            extract_sample_rate(&config_str).unwrap_or(22050)
        } else {
            22050
        };

        tracing::info!(
            "Piper TTS ready: voice={}, sample_rate={}",
            voice_id,
            sample_rate
        );

        Ok(Self {
            piper_bin,
            model_path,
            sample_rate,
        })
    }

    /// Synthesize text to raw i16 PCM samples.
    pub fn synthesize_pcm(&self, text: &str) -> Result<Vec<i16>> {
        use std::io::Write;

        let cleaned = clean_for_tts(text);
        if cleaned.is_empty() {
            anyhow::bail!("No speakable text after cleaning");
        }

        let mut child = std::process::Command::new(&self.piper_bin)
            .arg("--model")
            .arg(&self.model_path)
            .arg("--output_raw")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn piper")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(cleaned.as_bytes())
                .context("Failed to write text to piper")?;
        }

        let result = child
            .wait_with_output()
            .context("Failed to wait for piper")?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            anyhow::bail!("Piper failed: {stderr}");
        }

        let raw = &result.stdout;
        if raw.len() % 2 != 0 {
            anyhow::bail!("Piper output has odd byte count");
        }

        let samples: Vec<i16> = raw
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        tracing::info!(
            "Piper TTS: {} samples ({:.1}s audio)",
            samples.len(),
            samples.len() as f32 / self.sample_rate as f32,
        );

        Ok(samples)
    }

    /// Synthesize text to OGG/Opus bytes (for sending via Telegram/WhatsApp/etc).
    pub fn synthesize_opus(&self, text: &str) -> Result<Vec<u8>> {
        let samples = self.synthesize_pcm(text)?;
        pcm_to_opus(&samples, self.sample_rate)
    }

    /// Output sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Convert i16 PCM samples to OGG/Opus bytes (RFC 7845).
///
/// Piper outputs at the voice's native sample rate (typically 22050 Hz).
/// Opus requires 48000 Hz, so we resample first, then encode.
/// The result is a proper OGG/Opus stream that Telegram shows with waveform.
fn pcm_to_opus(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    // Resample to 48000 Hz (required by Opus)
    let samples_48k = if sample_rate == 48000 {
        samples.to_vec()
    } else {
        resample_i16(samples, sample_rate, 48000)?
    };

    encode_ogg_opus(&samples_48k, 48000)
}

/// Resample i16 PCM from one rate to another using rubato.
fn resample_i16(samples: &[i16], from_rate: u32, to_rate: u32) -> Result<Vec<i16>> {
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
        .map_err(|e| anyhow::anyhow!("Resampler init: {e}"))?;

    // Convert i16 → f32
    let float_samples: Vec<f32> = samples
        .iter()
        .map(|&s| s as f32 / i16::MAX as f32)
        .collect();

    let mut output = Vec::with_capacity((float_samples.len() as f64 * ratio) as usize + 1024);
    let mut pos = 0;

    while pos + chunk_size <= float_samples.len() {
        let chunk = &float_samples[pos..pos + chunk_size];
        let result = resampler
            .process(&[chunk], None)
            .map_err(|e| anyhow::anyhow!("Resample error: {e}"))?;
        output.extend_from_slice(&result[0]);
        pos += chunk_size;
    }

    if pos < float_samples.len() {
        let remaining = &float_samples[pos..];
        let result = resampler
            .process_partial(Some(&[remaining]), None)
            .map_err(|e| anyhow::anyhow!("Resample error: {e}"))?;
        output.extend_from_slice(&result[0]);
    }

    // Convert f32 → i16
    Ok(output
        .iter()
        .map(|&s| (s * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16)
        .collect())
}

/// Encode i16 PCM samples at 48 kHz into an OGG/Opus byte stream (RFC 7845).
fn encode_ogg_opus(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    use opusic_sys::*;

    const FRAME_MS: usize = 20; // 20 ms frames
    let frame_size = (sample_rate as usize * FRAME_MS) / 1000; // 960 samples per frame

    // Create Opus encoder
    let mut error: libc::c_int = 0;
    let encoder = unsafe {
        opus_encoder_create(
            sample_rate as i32,
            1, // mono
            OPUS_APPLICATION_VOIP,
            &mut error,
        )
    };
    if error != OPUS_OK || encoder.is_null() {
        anyhow::bail!("Failed to create Opus encoder: error {error}");
    }

    // Encode all frames
    let mut opus_packets: Vec<Vec<u8>> = Vec::new();
    let mut encode_buf = vec![0u8; 4000]; // max opus frame is ~4000 bytes
    let mut pos = 0;

    while pos + frame_size <= samples.len() {
        let frame = &samples[pos..pos + frame_size];
        let encoded_len = unsafe {
            opus_encode(
                encoder,
                frame.as_ptr(),
                frame_size as libc::c_int,
                encode_buf.as_mut_ptr(),
                encode_buf.len() as i32,
            )
        };
        if encoded_len < 0 {
            unsafe { opus_encoder_destroy(encoder) };
            anyhow::bail!("Opus encode failed: error {encoded_len}");
        }
        opus_packets.push(encode_buf[..encoded_len as usize].to_vec());
        pos += frame_size;
    }

    // Encode remaining samples (pad with silence)
    if pos < samples.len() {
        let mut padded = vec![0i16; frame_size];
        let remaining = &samples[pos..];
        padded[..remaining.len()].copy_from_slice(remaining);
        let encoded_len = unsafe {
            opus_encode(
                encoder,
                padded.as_ptr(),
                frame_size as libc::c_int,
                encode_buf.as_mut_ptr(),
                encode_buf.len() as i32,
            )
        };
        if encoded_len > 0 {
            opus_packets.push(encode_buf[..encoded_len as usize].to_vec());
        }
    }

    unsafe { opus_encoder_destroy(encoder) };

    // Build OGG container (RFC 7845)
    let serial: u32 = rand::random();
    let mut ogg = OggWriter::new(serial);

    // Page 1: OpusHead header
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead"); // magic
    head.push(1); // version
    head.push(1); // channel count (mono)
    head.extend_from_slice(&0u16.to_le_bytes()); // pre-skip
    head.extend_from_slice(&sample_rate.to_le_bytes()); // input sample rate
    head.extend_from_slice(&0i16.to_le_bytes()); // output gain
    head.push(0); // channel mapping family
    ogg.write_page(&[&head], 0, OggPageFlag::BOS);

    // Page 2: OpusTags header
    let vendor = b"opencrabs";
    let mut tags = Vec::new();
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    tags.extend_from_slice(vendor);
    tags.extend_from_slice(&0u32.to_le_bytes()); // no user comments
    ogg.write_page(&[&tags], 0, OggPageFlag::None);

    // Audio pages: one Opus packet per OGG page
    let mut granule: u64 = 0;
    for (i, packet) in opus_packets.iter().enumerate() {
        granule += frame_size as u64;
        let flag = if i == opus_packets.len() - 1 {
            OggPageFlag::EOS
        } else {
            OggPageFlag::None
        };
        ogg.write_page(&[packet], granule, flag);
    }

    Ok(ogg.into_bytes())
}

/// Build a WAV file from i16 PCM (for local preview playback only, not Telegram).
fn pcm_to_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    let file_len = 36 + data_len;
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_len.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    Ok(buf)
}

// ─── Minimal OGG page writer (RFC 3533 / RFC 7845) ───────────────────────────

#[derive(Clone, Copy)]
#[allow(clippy::upper_case_acronyms)]
enum OggPageFlag {
    None,
    BOS, // beginning of stream
    EOS, // end of stream
}

struct OggWriter {
    serial: u32,
    page_seq: u32,
    buf: Vec<u8>,
}

impl OggWriter {
    fn new(serial: u32) -> Self {
        Self {
            serial,
            page_seq: 0,
            buf: Vec::with_capacity(64 * 1024),
        }
    }

    /// Write one OGG page containing the given segments.
    fn write_page(&mut self, segments: &[&[u8]], granule_pos: u64, flag: OggPageFlag) {
        // Build segment table: each segment max 255 bytes; 255 = continuation, <255 = end
        let mut seg_table = Vec::new();
        for seg in segments {
            let mut remaining = seg.len();
            while remaining >= 255 {
                seg_table.push(255u8);
                remaining -= 255;
            }
            seg_table.push(remaining as u8);
        }

        let header_type = match flag {
            OggPageFlag::BOS => 0x02,
            OggPageFlag::EOS => 0x04,
            OggPageFlag::None => 0x00,
        };

        // Page header (27 bytes + segment table)
        let header_start = self.buf.len();
        self.buf.extend_from_slice(b"OggS"); // capture pattern
        self.buf.push(0); // stream structure version
        self.buf.push(header_type); // header type flag
        self.buf.extend_from_slice(&granule_pos.to_le_bytes()); // granule position
        self.buf.extend_from_slice(&self.serial.to_le_bytes()); // serial number
        self.buf.extend_from_slice(&self.page_seq.to_le_bytes()); // page sequence number
        self.buf.extend_from_slice(&0u32.to_le_bytes()); // CRC placeholder
        self.buf.push(seg_table.len() as u8); // number of segments
        self.buf.extend_from_slice(&seg_table); // segment table

        // Page data
        for seg in segments {
            self.buf.extend_from_slice(seg);
        }

        // Calculate and patch CRC-32
        let page_end = self.buf.len();
        let crc = ogg_crc32(&self.buf[header_start..page_end]);
        let crc_offset = header_start + 22;
        self.buf[crc_offset..crc_offset + 4].copy_from_slice(&crc.to_le_bytes());

        self.page_seq += 1;
    }

    fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// OGG CRC-32 (polynomial 0x04C11DB7, no pre/post inversion).
fn ogg_crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for i in 0..256u32 {
            let mut r = i << 24;
            for _ in 0..8 {
                r = if r & 0x80000000 != 0 {
                    (r << 1) ^ 0x04C11DB7
                } else {
                    r << 1
                };
            }
            t[i as usize] = r;
        }
        t
    });

    let mut crc: u32 = 0;
    for &byte in data {
        let idx = ((crc >> 24) ^ byte as u32) as usize;
        crc = (crc << 8) ^ table[idx];
    }
    crc
}

/// Play a short voice preview to confirm setup works.
/// Synthesizes "Hey, I am {label}" and plays via system audio.
pub async fn preview_voice(voice_id: &str) -> Result<()> {
    let preset = find_piper_voice(voice_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown Piper voice: {voice_id}"))?;
    let tts = PiperTts::new(voice_id)?;
    let text = format!("Hey! I am {}. Nice to meet you!", preset.label);
    let wav_bytes = {
        let samples = tts.synthesize_pcm(&text)?;
        pcm_to_wav(&samples, tts.sample_rate())?
    };

    // Write WAV to temp file and play
    let tmp = std::env::temp_dir().join(format!("opencrabs_preview_{voice_id}.wav"));
    std::fs::write(&tmp, &wav_bytes)?;

    let player = if cfg!(target_os = "macos") {
        "afplay"
    } else if cfg!(target_os = "windows") {
        "powershell"
    } else {
        "aplay"
    };

    let result = if cfg!(target_os = "windows") {
        tokio::process::Command::new(player)
            .args([
                "-c",
                &format!(
                    "(New-Object Media.SoundPlayer '{}').PlaySync()",
                    tmp.display()
                ),
            ])
            .output()
            .await
    } else {
        tokio::process::Command::new(player)
            .arg(&tmp)
            .output()
            .await
    };

    let _ = std::fs::remove_file(&tmp);

    match result {
        Ok(output) if output.status.success() => {
            tracing::info!("Piper preview played for voice '{}'", voice_id);
            Ok(())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Voice preview playback failed: {}", stderr);
            Ok(()) // non-fatal
        }
        Err(e) => {
            tracing::warn!("Could not play voice preview: {}", e);
            Ok(()) // non-fatal
        }
    }
}

// ─── Text sanitization for TTS ───────────────────────────────────────────────

/// Clean text for TTS synthesis — strip markdown formatting markers only,
/// keep all actual content so Piper reads the full response naturally.
fn clean_for_tts(text: &str) -> String {
    let mut s = text.to_string();

    // Strip code fence markers (```lang and ```) but keep the code content
    s = s
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("```")
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Remove inline backticks but keep content inside
    s = s.replace('`', "");

    // Remove markdown bold/italic markers (**, *, __)
    s = s.replace("**", "");
    s = s.replace("__", "");
    s = s.replace('*', "");

    // Remove markdown headers (# ## ### etc.) but keep text
    s = s
        .lines()
        .map(|line| line.trim_start_matches('#').trim_start())
        .collect::<Vec<_>>()
        .join("\n");

    // Remove markdown links [text](url) → keep text only
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            let mut link_text = String::new();
            let mut found_close = false;
            for c in chars.by_ref() {
                if c == ']' {
                    found_close = true;
                    break;
                }
                link_text.push(c);
            }
            if found_close && chars.peek() == Some(&'(') {
                chars.next(); // skip '('
                for c in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
                result.push_str(&link_text);
            } else {
                result.push_str(&link_text);
            }
        } else {
            result.push(ch);
        }
    }
    s = result;

    // Remove bullet markers (- or •) at start of lines
    s = s
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("- ") {
                rest.trim()
            } else if let Some(rest) = trimmed.strip_prefix("• ") {
                rest.trim()
            } else {
                trimmed
            }
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(". ");

    // Collapse repeated punctuation (!!! → !, ??? → ?)
    let mut prev_punct = false;
    let mut cleaned = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '!' || ch == '?' {
            if !prev_punct {
                cleaned.push(ch);
            }
            prev_punct = true;
        } else {
            prev_punct = false;
            cleaned.push(ch);
        }
    }
    s = cleaned;

    // Collapse ellipsis (... → .)
    while s.contains("...") {
        s = s.replace("...", ".");
    }
    while s.contains("..") {
        s = s.replace("..", ".");
    }

    // Collapse multiple whitespace/newlines into single space
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Extract sample_rate from piper voice config JSON.
fn extract_sample_rate(config: &str) -> Option<u32> {
    let needle = "\"sample_rate\"";
    let pos = config.find(needle)?;
    let rest = &config[pos + needle.len()..];
    let colon = rest.find(':')?;
    let after_colon = rest[colon + 1..].trim_start();
    let num_end = after_colon.find(|c: char| !c.is_ascii_digit())?;
    after_colon[..num_end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_sample_rate() {
        let config = r#"{"sample_rate": 22050, "other": "stuff"}"#;
        assert_eq!(extract_sample_rate(config), Some(22050));
    }

    #[test]
    fn test_extract_sample_rate_missing() {
        let config = r#"{"other": "stuff"}"#;
        assert_eq!(extract_sample_rate(config), None);
    }

    #[test]
    fn test_find_piper_voice() {
        assert!(find_piper_voice("ryan").is_some());
        assert!(find_piper_voice("amy").is_some());
        assert!(find_piper_voice("nonexistent").is_none());
    }

    #[test]
    fn test_piper_voice_urls() {
        let ryan = find_piper_voice("ryan").unwrap();
        assert!(ryan.onnx_url().contains("en_US"));
        assert!(ryan.onnx_url().contains("ryan"));
        assert!(ryan.onnx_url().ends_with(".onnx"));
        assert!(ryan.config_url().ends_with(".onnx.json"));
    }

    #[test]
    fn test_clean_for_tts_strips_markdown() {
        let input = "**Hello** *world*! Check `this_code` out.";
        let cleaned = clean_for_tts(input);
        assert_eq!(cleaned, "Hello world! Check this_code out.");
    }

    #[test]
    fn test_clean_for_tts_keeps_code_block_content() {
        let input = "Here is code:\n\n```rust\nfn main() {}\n```\n\nDone.";
        let cleaned = clean_for_tts(input);
        assert!(cleaned.contains("fn main()"));
        assert!(cleaned.contains("Done."));
        assert!(!cleaned.contains("```"));
    }

    #[test]
    fn test_clean_for_tts_collapses_whitespace() {
        let input = "Hello    world   how   are  you";
        let cleaned = clean_for_tts(input);
        assert_eq!(cleaned, "Hello world how are you");
    }

    #[test]
    fn test_clean_for_tts_collapses_punctuation() {
        let input = "Wow!!! Really??? Yes...";
        let cleaned = clean_for_tts(input);
        assert_eq!(cleaned, "Wow! Really? Yes.");
    }

    #[test]
    fn test_clean_for_tts_strips_headers() {
        let input = "## My Header\nSome text";
        let cleaned = clean_for_tts(input);
        assert_eq!(cleaned, "My Header. Some text");
    }

    #[test]
    fn test_clean_for_tts_strips_bullets() {
        let input = "- First item\n- Second item";
        let cleaned = clean_for_tts(input);
        assert_eq!(cleaned, "First item. Second item");
    }

    #[test]
    fn test_default_voice_is_ryan() {
        assert_eq!(PIPER_VOICES[0].id, "ryan");
    }

    #[test]
    fn test_pcm_to_wav() {
        let samples = vec![0i16, 100, -100, 32767, -32768];
        let wav = pcm_to_wav(&samples, 22050).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn test_pcm_to_opus_produces_ogg() {
        // 960 samples = one 20ms frame at 48kHz
        let samples = vec![0i16; 960];
        let ogg = encode_ogg_opus(&samples, 48000).unwrap();
        assert_eq!(&ogg[..4], b"OggS", "Should produce OGG container");
    }

    #[test]
    fn test_ogg_crc32() {
        // Known test vector: CRC of empty data should be 0
        assert_eq!(ogg_crc32(&[]), 0);
    }
}
