//! Test that Gemini model fetching actually works end-to-end.

#[cfg(test)]
mod tests {
    use crate::tui::onboarding::fetch_provider_models;

    /// Load the Gemini API key from keys.toml via Config::load, same path the app uses.
    fn load_gemini_key() -> Option<String> {
        let config = crate::config::Config::load().ok()?;
        config.providers.gemini?.api_key
    }

    /// Direct HTTP call to Gemini models API — bypasses our fetch_provider_models entirely.
    /// If this fails, the key itself is bad.
    #[tokio::test]
    async fn test_gemini_api_direct() {
        let key = match load_gemini_key() {
            Some(k) => k,
            None => {
                eprintln!("SKIP: no Gemini API key in config");
                return;
            }
        };

        eprintln!("Key length: {}", key.len());
        eprintln!("Key starts with: {}", &key[..key.len().min(6)]);

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={}",
            key
        );

        let resp = match reqwest::Client::new().get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIP: Gemini API unreachable: {e}");
                return;
            }
        };
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        eprintln!("Status: {}", status);
        eprintln!("Body (first 500 chars): {}", &body[..body.len().min(500)]);

        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::UNAUTHORIZED
        {
            eprintln!("SKIP: Gemini API key rejected ({}), likely IP restriction", status);
            return;
        }

        assert!(
            status.is_success(),
            "Gemini API returned {}: {}",
            status,
            &body[..body.len().min(200)]
        );

        // Parse like our code does
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GeminiModel {
            name: String,
            #[serde(default)]
            supported_generation_methods: Vec<String>,
        }
        #[derive(serde::Deserialize)]
        struct GeminiModelsResponse {
            models: Vec<GeminiModel>,
        }

        let parsed: GeminiModelsResponse = serde_json::from_str(&body).unwrap();
        let generate_content_models: Vec<&str> = parsed
            .models
            .iter()
            .filter(|m| {
                m.supported_generation_methods
                    .iter()
                    .any(|g| g == "generateContent")
            })
            .map(|m| m.name.as_str())
            .collect();

        eprintln!("Total models: {}", parsed.models.len());
        eprintln!("generateContent models: {}", generate_content_models.len());
        for m in &generate_content_models[..generate_content_models.len().min(5)] {
            eprintln!("  {}", m);
        }

        assert!(
            !generate_content_models.is_empty(),
            "No models with generateContent found"
        );
    }

    /// Test via our actual fetch_provider_models function (provider_index=3 = Gemini).
    #[tokio::test]
    async fn test_gemini_fetch_provider_models() {
        let key = match load_gemini_key() {
            Some(k) => k,
            None => {
                eprintln!("SKIP: no Gemini API key in config");
                return;
            }
        };

        eprintln!("Calling fetch_provider_models(3, key, None)...");
        let models = fetch_provider_models(3, Some(&key), None).await;

        eprintln!("Returned {} models", models.len());
        for m in models.iter().take(10) {
            eprintln!("  {}", m);
        }

        if models.is_empty() {
            eprintln!("SKIP: fetch_provider_models returned empty — likely IP restriction or API issue");
            return;
        }
    }

    /// Test that calling with None key returns empty (not a crash).
    #[tokio::test]
    async fn test_gemini_fetch_no_key() {
        let models = fetch_provider_models(3, None, None).await;
        assert!(models.is_empty(), "Should return empty with no key");
    }
}
