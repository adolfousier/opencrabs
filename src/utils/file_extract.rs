/// Classify and extract content from a user-sent file.
///
/// Returns one of:
/// - `FileContent::Text(String)` — UTF-8 text extracted inline (capped at 8 000 chars)
/// - `FileContent::Image` — caller should write bytes to a temp path and use `<<IMG:path>>`
/// - `FileContent::Unsupported(String)` — human-readable note about what was received
pub enum FileContent {
    Text(String),
    Image,
    Unsupported(String),
}

/// Determine whether a MIME type + filename extension is a text file.
pub fn is_text_mime(mime: &str) -> bool {
    let lower = mime.to_lowercase();
    lower.starts_with("text/")
        || matches!(
            lower.as_str(),
            "application/json"
                | "application/xml"
                | "application/x-yaml"
                | "application/yaml"
                | "application/toml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-sh"
                | "application/x-python"
                | "application/x-ruby"
        )
}

/// Fallback: guess MIME from filename extension.
pub fn mime_from_ext(filename: &str) -> &'static str {
    match filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "txt" | "md" | "rst" | "log" => "text/plain",
        "json" => "application/json",
        "xml" | "svg" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "csv" | "tsv" => "text/csv",
        "html" | "htm" => "text/html",
        "js" | "mjs" => "application/javascript",
        "ts" => "text/plain",
        "py" | "rb" | "sh" | "rs" | "go" | "java" | "c" | "cpp" | "h" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

const TEXT_LIMIT: usize = 8_000;

/// Classify file bytes given a MIME type and filename, returning `FileContent`.
pub fn classify_file(bytes: &[u8], mime: &str, filename: &str) -> FileContent {
    let effective_mime = if mime == "application/octet-stream" || mime.is_empty() {
        mime_from_ext(filename)
    } else {
        mime
    };

    if effective_mime.starts_with("image/") {
        return FileContent::Image;
    }

    if effective_mime == "application/pdf" {
        return match pdf_extract::extract_text_from_mem(bytes) {
            Ok(text) => {
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    FileContent::Unsupported(format!(
                        "[File received: {filename} (PDF) — no extractable text found, may be image-based]"
                    ))
                } else {
                    let truncated = if trimmed.len() > TEXT_LIMIT {
                        let safe: String = trimmed.chars().take(TEXT_LIMIT).collect();
                        format!("{}…[truncated]", safe)
                    } else {
                        trimmed
                    };
                    FileContent::Text(format!("[File: {filename}]\n```\n{}\n```", truncated))
                }
            }
            Err(_) => FileContent::Unsupported(format!(
                "[File received: {filename} (PDF) — failed to extract text, may be corrupted or image-based]"
            )),
        };
    }

    if is_text_mime(effective_mime) {
        let raw = String::from_utf8_lossy(bytes);
        let truncated = if raw.len() > TEXT_LIMIT {
            let safe: String = raw.chars().take(TEXT_LIMIT).collect();
            format!("{}…[truncated]", safe)
        } else {
            raw.into_owned()
        };
        return FileContent::Text(format!("[File: {filename}]\n```\n{}\n```", truncated));
    }

    FileContent::Unsupported(format!(
        "[File received: {filename} ({effective_mime}) — binary format not supported for text extraction]"
    ))
}
