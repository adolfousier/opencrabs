//! Tests for `utils::file_extract` — file classification, MIME detection, and content extraction.
//!
//! Covers `is_text_mime`, `mime_from_ext`, `classify_file`, and `FileContent`.

use crate::utils::file_extract::{FileContent, classify_file, is_text_mime, mime_from_ext};

// ── is_text_mime ────────────────────────────────────────────────

#[test]
fn text_plain_is_text() {
    assert!(is_text_mime("text/plain"));
}

#[test]
fn text_html_is_text() {
    assert!(is_text_mime("text/html"));
}

#[test]
fn application_json_is_text() {
    assert!(is_text_mime("application/json"));
}

#[test]
fn application_xml_is_text() {
    assert!(is_text_mime("application/xml"));
}

#[test]
fn application_yaml_is_text() {
    assert!(is_text_mime("application/yaml"));
    assert!(is_text_mime("application/x-yaml"));
}

#[test]
fn application_toml_is_text() {
    assert!(is_text_mime("application/toml"));
}

#[test]
fn application_javascript_is_text() {
    assert!(is_text_mime("application/javascript"));
    assert!(is_text_mime("application/x-javascript"));
}

#[test]
fn application_scripting_is_text() {
    assert!(is_text_mime("application/x-sh"));
    assert!(is_text_mime("application/x-python"));
    assert!(is_text_mime("application/x-ruby"));
}

#[test]
fn case_insensitive_mime() {
    assert!(is_text_mime("TEXT/PLAIN"));
    assert!(is_text_mime("Application/JSON"));
}

#[test]
fn image_mime_is_not_text() {
    assert!(!is_text_mime("image/png"));
    assert!(!is_text_mime("image/jpeg"));
}

#[test]
fn octet_stream_is_not_text() {
    assert!(!is_text_mime("application/octet-stream"));
}

#[test]
fn pdf_is_not_text() {
    assert!(!is_text_mime("application/pdf"));
}

// ── mime_from_ext ───────────────────────────────────────────────

#[test]
fn ext_txt_returns_text_plain() {
    assert_eq!(mime_from_ext("readme.txt"), "text/plain");
}

#[test]
fn ext_md_returns_text_plain() {
    assert_eq!(mime_from_ext("README.md"), "text/plain");
}

#[test]
fn ext_json_returns_application_json() {
    assert_eq!(mime_from_ext("config.json"), "application/json");
}

#[test]
fn ext_yaml_returns_application_yaml() {
    assert_eq!(mime_from_ext("config.yaml"), "application/yaml");
    assert_eq!(mime_from_ext("config.yml"), "application/yaml");
}

#[test]
fn ext_toml_returns_application_toml() {
    assert_eq!(mime_from_ext("config.toml"), "application/toml");
}

#[test]
fn ext_html_returns_text_html() {
    assert_eq!(mime_from_ext("index.html"), "text/html");
    assert_eq!(mime_from_ext("index.htm"), "text/html");
}

#[test]
fn ext_js_returns_application_javascript() {
    assert_eq!(mime_from_ext("app.js"), "application/javascript");
    assert_eq!(mime_from_ext("lib.mjs"), "application/javascript");
}

#[test]
fn ext_code_files_return_text_plain() {
    for ext in ["py", "rb", "sh", "rs", "go", "java", "c", "cpp", "h", "ts"] {
        assert_eq!(
            mime_from_ext(&format!("file.{ext}")),
            "text/plain",
            "failed for .{ext}"
        );
    }
}

#[test]
fn ext_images() {
    assert_eq!(mime_from_ext("photo.png"), "image/png");
    assert_eq!(mime_from_ext("photo.jpg"), "image/jpeg");
    assert_eq!(mime_from_ext("photo.jpeg"), "image/jpeg");
    assert_eq!(mime_from_ext("photo.gif"), "image/gif");
    assert_eq!(mime_from_ext("photo.webp"), "image/webp");
    assert_eq!(mime_from_ext("photo.bmp"), "image/bmp");
}

#[test]
fn ext_pdf() {
    assert_eq!(mime_from_ext("doc.pdf"), "application/pdf");
}

#[test]
fn ext_csv() {
    assert_eq!(mime_from_ext("data.csv"), "text/csv");
    assert_eq!(mime_from_ext("data.tsv"), "text/csv");
}

#[test]
fn ext_xml_svg() {
    assert_eq!(mime_from_ext("data.xml"), "application/xml");
    assert_eq!(mime_from_ext("icon.svg"), "application/xml");
}

#[test]
fn unknown_ext_returns_octet_stream() {
    assert_eq!(mime_from_ext("archive.zip"), "application/octet-stream");
    assert_eq!(mime_from_ext("binary.bin"), "application/octet-stream");
    assert_eq!(mime_from_ext("noext"), "application/octet-stream");
}

#[test]
fn case_insensitive_ext() {
    assert_eq!(mime_from_ext("file.JSON"), "application/json");
    assert_eq!(mime_from_ext("image.PNG"), "image/png");
}

// ── classify_file ───────────────────────────────────────────────

#[test]
fn classify_text_file() {
    let content = b"Hello, world!";
    match classify_file(content, "text/plain", "hello.txt") {
        FileContent::Text(t) => {
            assert!(t.contains("Hello, world!"));
            assert!(t.contains("[File: hello.txt]"));
        }
        other => panic!("expected Text, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn classify_json_file() {
    let content = b"{\"key\": \"value\"}";
    match classify_file(content, "application/json", "data.json") {
        FileContent::Text(t) => {
            assert!(t.contains("key"));
            assert!(t.contains("[File: data.json]"));
        }
        other => panic!("expected Text, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn classify_image_file() {
    match classify_file(b"fake png bytes", "image/png", "photo.png") {
        FileContent::Image(_) => {}
        other => panic!("expected Image, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn classify_pdf_file_invalid() {
    // Invalid/minimal PDF bytes — extraction fails, returns Unsupported
    match classify_file(b"%PDF-1.4", "application/pdf", "doc.pdf") {
        FileContent::Unsupported(msg) => {
            assert!(msg.contains("doc.pdf"));
        }
        FileContent::Text(t) => {
            assert!(t.contains("doc.pdf"));
        }
        FileContent::Image(_) => panic!("PDF should not be classified as Image"),
        FileContent::PdfPages { .. } => {
            panic!("PDF with invalid bytes should not produce PdfPages")
        }
    }
}

#[test]
fn classify_binary_file() {
    match classify_file(b"\x00\x01\x02", "application/octet-stream", "data.bin") {
        FileContent::Unsupported(msg) => {
            assert!(msg.contains("data.bin"));
            assert!(msg.contains("unsupported format"));
        }
        other => panic!(
            "expected Unsupported, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

#[test]
fn classify_uses_ext_when_mime_is_octet_stream() {
    // .rs should be detected as text/plain via ext fallback
    match classify_file(b"fn main() {}", "application/octet-stream", "main.rs") {
        FileContent::Text(t) => {
            assert!(t.contains("fn main()"));
        }
        other => panic!("expected Text, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn classify_uses_ext_when_mime_is_empty() {
    match classify_file(b"# Title", "", "readme.md") {
        FileContent::Text(t) => {
            assert!(t.contains("# Title"));
        }
        other => panic!("expected Text, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn classify_image_via_ext_fallback() {
    match classify_file(b"fake", "application/octet-stream", "photo.jpg") {
        FileContent::Image(_) => {}
        other => panic!("expected Image, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn classify_text_truncates_large_content() {
    let content = vec![b'a'; 10_000];
    match classify_file(&content, "text/plain", "big.txt") {
        FileContent::Text(t) => {
            assert!(t.contains("…[truncated]"));
            // Should contain less than original 10_000 a's
            assert!(t.len() < 10_000);
        }
        other => panic!("expected Text, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn classify_text_wraps_in_code_fence() {
    let content = b"line1\nline2";
    match classify_file(content, "text/plain", "test.txt") {
        FileContent::Text(t) => {
            assert!(t.contains("```"));
            assert!(t.contains("line1\nline2"));
        }
        other => panic!("expected Text, got {:?}", std::mem::discriminant(&other)),
    }
}
