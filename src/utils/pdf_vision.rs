//! PDF page rendering to PNG images.
//!
//! Renders individual PDF pages as PNG files using (in order of preference):
//! 1. `pdfium-render` crate, bound to the system Pdfium library (feature
//!    `pdfium`, on by default). Nothing is bundled: `bind_to_system_library`
//!    needs libpdfium present, and returns an error when it is missing, which
//!    is what falls through to the next strategy.
//! 2. Shell fallback to `pdftoppm` (poppler-utils)
//!
//! Pages are processed in configurable batches to cap memory usage.
//! On failure, any partial renders are cleaned up automatically.

use std::fs;
use std::path::{Path, PathBuf};

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

    // On total failure (no pages rendered at all), clean up any stray
    // partial renders from earlier strategies. A partial success
    // (Ok with some pages) is preserved by `render_pdf_pages_inner`
    // — losing the good pages because the last batch failed was the
    // bug that surfaced as "PDFs truncated after page 5".
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
        "No PDF renderer available. Install poppler — macOS: `brew install poppler`, \
         Debian/Ubuntu: `apt install poppler-utils`, Fedora: `dnf install poppler-utils` \
         (or build with the optional `pdfium` feature)."
            .into(),
    )
}

/// Return total page count for `pdf_path`, or `None` if it can't be
/// determined (no `pdfinfo` available, malformed PDF, etc.). Used to
/// bound the pdftoppm render loop so we don't ask for pages past EOF.
fn pdf_page_count(pdf_path: &Path) -> Option<usize> {
    let path_str = pdf_path.to_str()?;
    let out = std::process::Command::new("pdfinfo")
        .arg(path_str)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            return rest.trim().parse::<usize>().ok();
        }
    }
    None
}

// ---------------------------------------------------------------------------
// pdfium-render implementation
// ---------------------------------------------------------------------------

/// Process-wide pdfium handle. `pdfium-render` 0.9.1 keeps its loaded
/// library in a process-global `OnceCell` that is never reset: a second
/// bind attempt in the same process returns
/// `PdfiumLibraryBindingsAlreadyInitialized`, and two concurrent attempts
/// race through the crate's guard-less `is_none()` check into
/// `Pdfium::new`'s `assert!(BINDINGS.get().is_none())`, panicking the
/// loser (#1715). Binding exactly once behind
/// `once_cell::sync::OnceCell::get_or_try_init` serialises the bind and
/// lets every later render reuse the handle; a *failed* bind is not
/// cached, so hosts without libpdfium still fall through to the pdftoppm
/// strategy on every call instead of latching the first failure. The `std`
/// `OnceLock::get_or_try_init` is still unstable on this toolchain, hence
/// the `once_cell` crate, matching the `memory/embedding.rs` precedent.
///
/// `Pdfium::default()` is not a substitute: it re-runs the same guard-less
/// racy bind, and per its own docs it "will panic if no suitable Pdfium
/// library can be loaded", which inside `spawn_blocking` surfaces as a
/// `JoinError` with no fallback at all (#1716).
#[cfg(feature = "pdfium")]
static PDFIUM: once_cell::sync::OnceCell<pdfium_render::prelude::Pdfium> =
    once_cell::sync::OnceCell::new();

/// Bind pdfium once per process; see [`PDFIUM`] for why re-binding is a
/// guaranteed failure and concurrent re-binding a panic landmine. The
/// error is already the caller-facing string so no remapping is needed.
#[cfg(feature = "pdfium")]
pub(crate) fn shared_pdfium() -> Result<&'static pdfium_render::prelude::Pdfium, String> {
    PDFIUM
        .get_or_try_init(|| {
            pdfium_render::prelude::Pdfium::bind_to_system_library()
                .map(pdfium_render::prelude::Pdfium::new)
        })
        .map_err(|e| format!("Cannot bind pdfium library: {}", e))
}

#[cfg(feature = "pdfium")]
fn render_with_pdfium(
    pdf_path: &Path,
    max_pages: usize,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    use pdfium_render::prelude::*;

    let pdfium = shared_pdfium()?;

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

    // Bound the render loop to the document's actual page count when
    // pdfinfo is available. Without this, batches past EOF make
    // pdftoppm error with "Wrong page range given: the first page (N)
    // can not be after the last page (M)" — the whole render then
    // returns Err and the caller wipes the already-rendered batches.
    // When pdfinfo isn't installed we fall back to max_pages and rely
    // on per-batch error tolerance below.
    let effective_max = match pdf_page_count(pdf_path) {
        Some(total) => total.min(max_pages),
        None => max_pages,
    };
    if effective_max == 0 {
        return Err("pdftoppm: PDF reports zero pages".into());
    }

    // pdftoppm generates files named <prefix>-01.png, <prefix>-02.png, …
    let prefix = "page";
    let mut rendered: Vec<PathBuf> = Vec::with_capacity(effective_max);

    // Process in batches. We tolerate per-batch errors: a partial render
    // (early pages succeed, a late batch fails) is more useful than no
    // pages at all. Only return Err if every batch fails.
    let mut start = 1;
    let mut last_err: Option<String> = None;
    while start <= effective_max {
        let batch_end = (start + BATCH_SIZE - 1).min(effective_max);

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
            tracing::warn!(
                "pdftoppm batch {}-{} failed: {} — keeping earlier pages",
                start,
                batch_end,
                stderr.trim()
            );
            last_err = Some(stderr.trim().to_string());
            // Don't return — try the next batch. PDFs with corrupt
            // pages in the middle still surface their good pages.
            start = batch_end + 1;
            continue;
        }

        // Collect the generated files for this batch. pdftoppm pads page
        // numbers to the digit-width of the *last page number rendered*,
        // not of the document: `-f 1 -l 3` writes `page-1.png`, while
        // `-f 1 -l 99` writes `page-01.png`. Every batch ending at a
        // single-digit page therefore produced files none of the old
        // 2-to-4 widths could find, and the pages were silently dropped
        // (#1715). Try every candidate width instead of guessing the one.
        for page_num in start..=batch_end {
            if let Some(p) = pdftoppm_output_names(prefix, page_num)
                .into_iter()
                .find_map(|name| output_dir.join(name).canonicalize().ok())
            {
                rendered.push(p);
            }
        }

        start = batch_end + 1;
    }

    if rendered.is_empty() {
        return Err(format!(
            "pdftoppm produced no output files{}",
            last_err
                .map(|e| format!(" (last error: {e})"))
                .unwrap_or_default()
        ));
    }

    Ok(rendered)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Filenames `pdftoppm` may have written for `page_num`: the digits
/// left-padded to every width from 1 to 4. Width 1 is the unpadded form
/// `pdftoppm` uses whenever the rendered range ends at page 9 or below
/// (`-f 1 -l 3` writes `page-1.png`), which the previous fixed 2-to-4
/// lookup never tried (#1715).
pub(crate) fn pdftoppm_output_names(prefix: &str, page_num: usize) -> Vec<String> {
    (1..=4)
        .map(|w| format!("{}-{:0w$}.png", prefix, page_num, w = w))
        .collect()
}

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
