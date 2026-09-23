//! Tests for `utils::pdf_vision::render_pdf_pages`.
//!
//! Covers the regression where `pdftoppm` would loop past the document's
//! actual page count, fail on a later batch, and cause the whole render
//! to be discarded (the "truncated after page 5" symptom). We can't
//! unit-test the pdftoppm shell behavior directly without a real PDF
//! and the binary on PATH, so these focus on what we can deterministically
//! check: the function entry-point contract.

use crate::utils::pdf_vision::render_pdf_pages;
use std::fs;

#[test]
fn missing_pdf_returns_error_with_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp.path().join("pages");
    let result = render_pdf_pages("/no/such/file.pdf", 10, out_dir.to_str().unwrap());
    let err = result.expect_err("missing pdf must error");
    assert!(err.contains("/no/such/file.pdf"));
}

#[test]
fn output_directory_is_created_if_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Write a 1-byte fake "pdf" so existence check passes — render
    // itself will fail (not a real PDF) but we only care that the
    // output directory was created during setup.
    let pdf = tmp.path().join("fake.pdf");
    fs::write(&pdf, b"%PDF-1.4\n").expect("write fake pdf");
    let out = tmp.path().join("nested").join("renders");
    assert!(!out.exists(), "precondition: output dir must not exist yet");
    let _ = render_pdf_pages(pdf.to_str().unwrap(), 5, out.to_str().unwrap());
    assert!(out.exists(), "output dir must be created on entry");
}

/// When no PDF renderer succeeds (pdfium feature off + no valid input
/// for pdftoppm), the helper must surface an actionable error rather
/// than panic or silently produce an empty Ok.
#[test]
fn unrenderable_pdf_returns_err_not_empty_ok() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pdf = tmp.path().join("not-really-a-pdf.pdf");
    fs::write(&pdf, b"this is not a pdf").expect("write fake");
    let out = tmp.path().join("out");
    let result = render_pdf_pages(pdf.to_str().unwrap(), 100, out.to_str().unwrap());
    // Either an Err (pdftoppm rejected the input) or an Ok with zero
    // pages — both are acceptable contracts; an Ok with no pages would
    // be the bug we explicitly want to avoid.
    if let Ok(paths) = &result {
        assert!(
            !paths.is_empty(),
            "Ok(vec![]) is forbidden — must be Err if no pages rendered"
        );
    }
}

/// #1715 defect 1: pdfium used to bind on every render call, so the
/// second and every later render in the same process failed with
/// `PdfiumLibraryBindingsAlreadyInitialized` (quietly degrading to
/// pdftoppm), and two concurrent renders could race the crate's
/// guard-less rebind into a panic. `shared_pdfium` must hand out the
/// same process-wide handle on every call. Gated on a successful first
/// bind so hosts without libpdfium installed stay green.
#[cfg(feature = "pdfium")]
#[test]
fn pdfium_bind_is_reused_across_calls() {
    use crate::utils::pdf_vision::shared_pdfium;

    let Ok(handle) = shared_pdfium() else {
        return; // no libpdfium on this host - nothing bound yet to reuse
    };
    let again = shared_pdfium().expect("second access must reuse the first bind, not rebind");
    assert!(
        std::ptr::eq(handle, again),
        "second access produced a different Pdfium - a rebind was attempted"
    );
}
