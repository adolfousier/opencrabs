//! File classification + vision-aware file ingestion pipeline.
//!
//! Two entry points:
//! - `classify_file()` — legacy, no vision (text-extract only)
//! - `process_file_with_vision()` — new, checks vision availability and routes

use crate::config::Config;
use std::fs;
use std::path::PathBuf;

// ── Legacy classify_file (kept for backward compat) ──

/// Returns `true` if any provider has a `vision_model` configured,
/// or if `image.vision` is enabled with an API key.
pub fn is_vision_available(config: &Config) -> bool {
    if crate::brain::provider::factory::active_provider_vision(config).is_some() {
        return true;
    }
    if config.image.vision.enabled {
        if let Some(ref key) = config.image.vision.api_key {
            return !key.is_empty();
        }
    }
    false
}

// ── FileContent enum ──

/// Result of classifying and extracting content from a user-sent file.
pub enum FileContent {
    /// UTF-8 text extracted inline (capped at 8 000 chars)
    Text(String),
    /// Single image — caller should write bytes to the returned temp path
    Image(PathBuf),
    /// PDF rendered to page images (vision path)
    PdfPages {
        paths: Vec<PathBuf>,
        label: String,
    },
    /// Unsupported format or failed extraction
    Unsupported(String),
}

// ── Helpers ──

const TEXT_LIMIT: usize = 8_000;
const MAX_PDF_PAGES: usize = 100;

/// Determine whether a MIME type is a text file.
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

/// Guess MIME from filename extension.
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

/// Write file bytes to a temp path under `~/.opencrabs/tmp/files/` and return the path.
fn save_to_temp(bytes: &[u8], filename: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("No home directory found")?;
    let tmp_dir = home.join(".opencrabs").join("tmp").join("files");
    fs::create_dir_all(&tmp_dir).map_err(|e| format!("Failed to create temp dir: {e}"))?;

    let safe_name = filename
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .collect::<String>();
    let path = tmp_dir.join(format!("{}_{safe_name}", uuid::Uuid::new_v4()));
    fs::write(&path, bytes).map_err(|e| format!("Failed to write temp file: {e}"))?;
    Ok(path)
}

/// Extract text from PDF bytes, truncated to TEXT_LIMIT.
fn extract_pdf_text(bytes: &[u8], filename: &str) -> FileContent {
    match pdf_extract::extract_text_from_mem(bytes) {
        Ok(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                FileContent::Unsupported(format!(
                    "[File received: {filename} (PDF) — no extractable text found, may be image-based]"
                ))
            } else {
                let truncated = if trimmed.len() > TEXT_LIMIT {
                    format!("{}…[truncated]", trimmed.chars().take(TEXT_LIMIT).collect::<String>())
                } else {
                    trimmed
                };
                FileContent::Text(format!("[File: {filename}]\n```\n{truncated}\n```"))
            }
        }
        Err(_) => FileContent::Unsupported(format!(
            "[File received: {filename} (PDF) — failed to extract text]"
        )),
    }
}

// ── Vision-aware pipeline ──

/// Process file bytes with vision-first routing.
///
/// Priority:
/// 1. **Vision available** → render PDF pages to images, or save image for vision
/// 2. **No vision** → extract text (PDFs/text files) or return unsupported
///
/// Channels pass the result to `inject_file_content()` to format
/// for the agent prompt, or match on `FileContent` directly.
pub fn process_file_with_vision(
    bytes: &[u8],
    mime: &str,
    filename: &str,
    config: &Config,
) -> FileContent {
    let effective = if mime.is_empty() || mime == "application/octet-stream" {
        mime_from_ext(filename)
    } else {
        mime
    };

    let has_vision = is_vision_available(config);

    // ── Images ──
    if effective.starts_with("image/") {
        if has_vision {
            return match save_to_temp(bytes, filename) {
                Ok(path) => FileContent::Image(path),
                Err(e) => FileContent::Unsupported(format!("[Image attachment: {filename} — failed to save for vision: {e}]")),
            };
        }
        return FileContent::Unsupported(format!(
            "[Image attachment: {filename} — no vision model configured. \
             Set `image.vision.enabled = true` with an API key, or add `vision_model` \
             to your provider config in config.toml.]"
        ));
    }

    // ── PDFs ──
    if effective == "application/pdf" {
        if has_vision {
            return process_pdf_vision(bytes, filename);
        }
        // No vision → text extraction with user notice
        return extract_pdf_text(bytes, filename);
    }

    // ── Text files ──
    if is_text_mime(effective) {
        let raw = String::from_utf8_lossy(bytes);
        let truncated = if raw.len() > TEXT_LIMIT {
            format!("{}…[truncated]", raw.chars().take(TEXT_LIMIT).collect::<String>())
        } else {
            raw.into_owned()
        };
        return FileContent::Text(format!("[File: {filename}]\n```\n{truncated}\n```"));
    }

    FileContent::Unsupported(format!(
        "[File received: {filename} ({effective}) — unsupported format]"
    ))
}

fn process_pdf_vision(bytes: &[u8], filename: &str) -> FileContent {
    // Save PDF to temp so pdfium-render can read it
    let pdf_path = match save_to_temp(bytes, filename) {
        Ok(p) => p,
        Err(e) => {
            return FileContent::Unsupported(format!(
                "[PDF received: {filename} — failed to prepare: {e}]"
            ))
        }
    };

    let rendered = super::pdf_vision::render_pdf_pages(
        pdf_path.to_str().unwrap_or(""),
        MAX_PDF_PAGES,
        pdf_path.parent().map(|p| p.to_str().unwrap()).unwrap_or(""),
    );

    match rendered {
        Ok(paths) if !paths.is_empty() => {
            let page_count = paths.len();
            let label = if page_count == 1 {
                "PDF".to_string()
            } else {
                format!("{page_count}-page-PDF")
            };
            FileContent::PdfPages { paths, label }
        }
        Ok(_) | Err(_) => {
            // Pdfium/pdftoppm failed — fall back to text extraction
            extract_pdf_text(bytes, filename)
        }
    }
}

// ── Channel injection helper ──

/// Format `FileContent` into an injectable string for the agent prompt.
///
/// Returns `(text, needs_vision)` where `needs_vision` is true when the
/// result contains `<<IMG:...>>` markers that should trigger image attachments.
pub fn inject_file_content(content: &FileContent) -> (String, bool) {
    match content {
        FileContent::Image(path) => {
            let path_str = path.to_string_lossy();
            (
                format!("[User attached an image. Use analyze_image to view it.]\n<<IMG:{path_str}>>"),
                true,
            )
        }
        FileContent::PdfPages { paths, label } => {
            let markers: String = paths
                .iter()
                .map(|p| format!("<<IMG:{}>>", p.to_string_lossy()))
                .collect();
            (
                format!(
                    "[User attached a {label}. analyze_image each page and combine the results.]\n{markers}"
                ),
                true,
            )
        }
        FileContent::Text(text) => (text.clone(), false),
        FileContent::Unsupported(note) => (note.clone(), false),
    }
}

/// Legacy classify_file — text-extract only, no vision routing.
pub fn classify_file(bytes: &[u8], mime: &str, filename: &str) -> FileContent {
    // ... (backward compat — delegates to process_file_with_vision with a dummy config)
    // Actually, keep original behavior to not break callers that don't have Config
    let effective = if mime.is_empty() || mime == "application/octet-stream" {
        mime_from_ext(filename)
    } else {
        mime
    };

    if effective.starts_with("image/") {
        return FileContent::Image(PathBuf::new());
    }

    if effective == "application/pdf" {
        return extract_pdf_text(bytes, filename);
    }

    if is_text_mime(effective) {
        let raw = String::from_utf8_lossy(bytes);
        let truncated = if raw.len() > TEXT_LIMIT {
            format!("{}…[truncated]", raw.chars().take(TEXT_LIMIT).collect::<String>())
        } else {
            raw.into_owned()
        };
        return FileContent::Text(format!("[File: {filename}]\n```\n{truncated}\n```"));
    }

    FileContent::Unsupported(format!(
        "[File received: {filename} ({effective}) — unsupported format]"
    ))
}
