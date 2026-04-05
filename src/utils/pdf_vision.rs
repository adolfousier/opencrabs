//! PDF page rendering to PNG images.
//!
//! Renders individual PDF pages as PNG files using (in order of preference):
//! 1. `pdfium-render` crate (bundled Pdfium — no external deps)
//! 2. Shell fallback to `pdftoppm` (poppler-utils)
//!
//! Pages are processed in configurable batches to cap memory usage.
//! On failure, any partial renders are cleaned up automatically.

use std::fs;
use std::path::{Path, PathBuf};

/// Default maximum number of pages to render before skipping.
const DEFAULT_MAX_PAGES: usize = 100;
/// Number of pages processed per batch to limit memory.
const BATCH_SIZE: usize = 10;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render up to `max_pages` pages of `pdf_path` as PNG files in `output_dir`.
///
/// - Creates `output_dir` if it does not exist.
/// - Pages beyond `max_pages` are skipped (a warning is logged).
/// - Returns the list of rendered PNG paths on success.
/// - On failure any partially-rendered files are removed.
pub fn render_pdf_pages(
    pdf_path: &str,
    max_pages: usize,
    output_dir: &str,
) -> Result<Vec<PathBuf>, String> {
    let pdf = PathBuf::from(pdf_path);
    let out = PathBuf::from(output_dir);

    if !pdf.exists() {
        return Err(format!("PDF file not found: {}", pdf_path));
    }

    // Ensure output directory exists.
    fs::create_dir_all(&out)
        .map_err(|e| format!("Failed to create output directory '{}': {}", output_dir, e))?;

    let result = render_pdf_pages_inner(&pdf, max_pages, &out);

    // On failure, clean up any partial renders.
    if let Err(ref err) = result {
        tracing::warn!("PDF render failed, cleaning up partial output: {}", err);
        cleanup_dir(&out);
    }

    result
}

// ---------------------------------------------------------------------------
// Dispatch: pdfium-render → pdftoppm → error
// ---------------------------------------------------------------------------

fn render_pdf_pages_inner(
    pdf_path: &Path,
    max_pages: usize,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    // Strategy 1: pdfium-render crate.
    match render_with_pdfium(pdf_path, max_pages, output_dir) {
        Ok(paths) => return Ok(paths),
        Err(e) => tracing::debug!("pdfium-render unavailable or failed: {}", e),
    }

    // Strategy 2: shell pdftoppm fallback.
    match render_with_pdftoppm(pdf_path, max_pages, output_dir) {
        Ok(paths) => return Ok(paths),
        Err(e) => tracing::debug!("pdftoppm unavailable or failed: {}", e),
    }

    Err(
        "No PDF renderer available. Install poppler-utils (pdftoppm) or enable the \
         'pdfium-render' feature."
            .into(),
    )
}

// ---------------------------------------------------------------------------
// pdfium-render implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "pdfium")]
fn render_with_pdfium(
    pdf_path: &Path,
    max_pages: usize,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    use pdfium_render::prelude::*;

    let pdfium = Pdfium::new(
        Pdfium::bind_to_system_library()
            .map_err(|e| format!("Cannot bind pdfium library: {}", e))?,
    );

    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|e| format!("Failed to open PDF '{}': {}", pdf_path.display(), e))?;

    let total_pages = document.pages().len() as usize;
    let pages_to_render = total_pages.min(max_pages);

    if total_pages > max_pages {
        tracing::warn!(
            "PDF has {} pages but max_pages={}; skipping pages {}–{}",
            total_pages,
            max_pages,
            max_pages + 1,
            total_pages,
        );
    }

    let mut rendered = Vec::with_capacity(pages_to_render);
    let mut page_idx: usize = 0;

    while page_idx < pages_to_render {
        let batch_end = (page_idx + BATCH_SIZE).min(pages_to_render);

        for i in page_idx..batch_end {
            let page = document
                .pages()
                .get(i as PdfPageIndex)
                .map_err(|e| format!("Failed to get page {}: {}", i + 1, e))?;

            let bitmap = page
                .render_with_config(&PdfRenderConfig::new().set_target_width(2000))
                .map_err(|e| format!("Failed to render page {}: {}", i + 1, e))?;

            let file_name = format!("page_{:04}.png", i + 1);
            let file_path = output_dir.join(&file_name);

            bitmap
                .as_image()
                .map_err(|e| format!("Failed to convert page {} to image: {}", i + 1, e))?
                .save(&file_path)
                .map_err(|e| format!("Failed to save page {}: {}", i + 1, e))?;

            rendered.push(file_path);
        }

        page_idx = batch_end;
        tracing::debug!("Rendered batch: pages {}/{}", page_idx, pages_to_render,);
    }

    Ok(rendered)
}

#[cfg(not(feature = "pdfium"))]
fn render_with_pdfium(
    _pdf_path: &Path,
    _max_pages: usize,
    _output_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    Err("pdfium feature not enabled".into())
}

// ---------------------------------------------------------------------------
// pdftoppm shell fallback
// ---------------------------------------------------------------------------

fn render_with_pdftoppm(
    pdf_path: &Path,
    max_pages: usize,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    // Check that pdftoppm is available.
    which::which("pdftoppm").map_err(|_| "pdftoppm not found in PATH".to_string())?;

    let pdf_path_str = pdf_path
        .to_str()
        .ok_or_else(|| "PDF path is not valid UTF-8".to_string())?;
    let out_dir_str = output_dir
        .to_str()
        .ok_or_else(|| "Output directory path is not valid UTF-8".to_string())?;

    // pdftoppm generates files named <prefix>-01.png, <prefix>-02.png, …
    let prefix = "page";
    let mut rendered: Vec<PathBuf> = Vec::with_capacity(max_pages.min(DEFAULT_MAX_PAGES));

    // Process in batches: pdftoppm renders ranges efficiently.
    let mut start = 1;
    while start <= max_pages {
        let batch_end = (start + BATCH_SIZE - 1).min(max_pages);

        let output = std::process::Command::new("pdftoppm")
            .args([
                "-png",
                "-r",
                "200",
                "-f",
                &start.to_string(),
                "-l",
                &batch_end.to_string(),
                pdf_path_str,
                &format!("{}/{}", out_dir_str, prefix),
            ])
            .output()
            .map_err(|e| format!("Failed to execute pdftoppm: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("pdftoppm failed: {}", stderr.trim()));
        }

        // Collect the generated files for this batch.
        for page_num in start..=batch_end {
            let file_name = format!("{}-{:02}.png", prefix, page_num);
            let file_path = output_dir.join(&file_name);
            if file_path.exists() {
                rendered.push(file_path);
            }
        }

        start = batch_end + 1;
    }

    if rendered.is_empty() {
        return Err("pdftoppm produced no output files".into());
    }

    Ok(rendered)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Remove all files inside `dir` (but not the directory itself).
fn cleanup_dir(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            let _ = fs::remove_file(entry.path());
        }
    }
}
