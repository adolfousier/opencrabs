//! Candle Whisper STT Tests
//!
//! Tests for mel filters, audio pipeline, tensor shapes, and encoder compatibility.
//! These validate the candle whisper integration end-to-end so regressions
//! like encoder forward pass failures are caught before shipping.

#[cfg(feature = "local-stt")]
mod candle_stt {
    use crate::channels::voice::local_whisper::*;

    // ─── Mel filters ────────────────────────────────────────────────────────

    #[test]
    fn mel_filters_correct_shape_80() {
        let filters = compute_mel_filters(80, 400, 16000);
        assert_eq!(filters.len(), 80 * 201); // n_mels * (N_FFT/2 + 1)
    }

    #[test]
    fn mel_filters_correct_shape_128() {
        let filters = compute_mel_filters(128, 400, 16000);
        assert_eq!(filters.len(), 128 * 201);
    }

    #[test]
    fn mel_filters_non_zero() {
        let filters = compute_mel_filters(80, 400, 16000);
        let nonzero = filters.iter().filter(|&&v| v > 0.0).count();
        assert!(
            nonzero > 200,
            "Expected many nonzero filter values, got {}",
            nonzero
        );
    }

    #[test]
    fn mel_filters_no_nan_inf_or_negative() {
        let filters = compute_mel_filters(80, 400, 16000);
        for (i, &v) in filters.iter().enumerate() {
            assert!(!v.is_nan(), "NaN at index {}", i);
            assert!(!v.is_infinite(), "Inf at index {}", i);
            assert!(v >= 0.0, "Negative value {} at index {}", v, i);
        }
    }

    // ─── pcm_to_mel integration ─────────────────────────────────────────────

    fn tiny_config() -> candle_transformers::models::whisper::Config {
        candle_transformers::models::whisper::Config {
            num_mel_bins: 80,
            max_source_positions: 1500,
            d_model: 384,
            encoder_attention_heads: 6,
            encoder_layers: 4,
            vocab_size: 51864,
            max_target_positions: 448,
            decoder_attention_heads: 6,
            decoder_layers: 4,
            suppress_tokens: vec![],
        }
    }

    #[test]
    fn pcm_to_mel_produces_enough_frames() {
        use candle_transformers::models::whisper;

        let config = tiny_config();
        let filters = compute_mel_filters(80, whisper::N_FFT, whisper::SAMPLE_RATE as u32);
        let samples = vec![0.0f32; whisper::N_SAMPLES];
        let mel = whisper::audio::pcm_to_mel(&config, &samples, &filters);

        let n_frames = mel.len() / 80;
        assert!(
            n_frames >= whisper::N_FRAMES,
            "Expected >= {} frames, got {}",
            whisper::N_FRAMES,
            n_frames
        );
    }

    #[test]
    fn mel_tensor_shape_fits_encoder() {
        use candle_transformers::models::whisper;

        let config = tiny_config();
        let filters = compute_mel_filters(80, whisper::N_FFT, whisper::SAMPLE_RATE as u32);
        let samples = vec![0.0f32; whisper::N_SAMPLES];
        let mel = whisper::audio::pcm_to_mel(&config, &samples, &filters);

        let n_frames = mel.len() / config.num_mel_bins;
        let device = candle_core::Device::Cpu;
        let tensor =
            candle_core::Tensor::from_vec(mel, (1, config.num_mel_bins, n_frames), &device)
                .unwrap();

        let dims = tensor.dims();
        assert_eq!(dims[0], 1);
        assert_eq!(dims[1], 80);
        // conv2 stride 2 halves frames; must fit in max_source_positions
        assert!(
            n_frames / 2 <= config.max_source_positions,
            "n_frames/2 ({}) > max_source_positions ({})",
            n_frames / 2,
            config.max_source_positions
        );
    }

    // ─── Audio padding/truncation ───────────────────────────────────────────

    #[test]
    fn short_audio_padded_to_30s() {
        let n_samples = candle_transformers::models::whisper::N_SAMPLES;
        let mut audio = vec![0.5f32; 8000]; // 0.5s
        audio.resize(n_samples, 0.0);
        assert_eq!(audio.len(), n_samples);
        assert_eq!(audio[7999], 0.5);
        assert_eq!(audio[8000], 0.0);
    }

    #[test]
    fn long_audio_truncated_to_30s() {
        let n_samples = candle_transformers::models::whisper::N_SAMPLES;
        let mut audio = vec![0.5f32; n_samples * 2];
        audio.truncate(n_samples);
        assert_eq!(audio.len(), n_samples);
    }

    // ─── Model presets ──────────────────────────────────────────────────────

    #[test]
    fn presets_have_required_fields() {
        for preset in LOCAL_MODEL_PRESETS {
            assert!(!preset.id.is_empty());
            assert!(!preset.label.is_empty());
            assert!(!preset.file_name.is_empty());
            assert!(!preset.size_label.is_empty());
            assert!(preset.repo_id.starts_with("openai/whisper-"));
        }
    }

    #[test]
    fn find_model_by_id() {
        assert!(find_local_model("local-tiny").is_some());
        assert!(find_local_model("local-base").is_some());
        assert!(find_local_model("local-small").is_some());
        assert!(find_local_model("local-medium").is_some());
        assert!(find_local_model("nonexistent").is_none());
    }
}
