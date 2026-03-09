//! Candle Whisper STT Tests
//!
//! Tests for mel filters and model presets.
//! These validate the rwhisper integration so regressions are caught before shipping.

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

    // ─── Model presets ──────────────────────────────────────────────────────

    #[test]
    fn presets_have_required_fields() {
        let valid_sources = [
            "QuantizedTiny",
            "QuantizedTinyEn",
            "Tiny",
            "TinyEn",
            "Base",
            "BaseEn",
            "Small",
            "SmallEn",
            "Medium",
            "MediumEn",
            "Large",
            "LargeV2",
        ];
        for preset in LOCAL_MODEL_PRESETS {
            assert!(!preset.id.is_empty());
            assert!(!preset.label.is_empty());
            assert!(!preset.file_name.is_empty());
            assert!(!preset.size_label.is_empty());
            assert!(
                valid_sources.contains(&preset.repo_id),
                "Repo ID should be a valid rwhisper source: {}",
                preset.repo_id
            );
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
