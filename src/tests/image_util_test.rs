//! Tests for `utils::image` — `<<IMG:path>>` marker extraction.

use crate::utils::image::extract_img_markers;

#[test]
fn no_markers_returns_text_unchanged() {
    let (text, paths) = extract_img_markers("plain text");
    assert_eq!(text, "plain text");
    assert!(paths.is_empty());
}

#[test]
fn single_img_marker_extracts_path() {
    let (text, paths) = extract_img_markers("see <<IMG:/tmp/a.png>> here");
    assert_eq!(text, "see  here");
    assert_eq!(paths, vec!["/tmp/a.png"]);
}

#[test]
fn multiple_markers_in_order() {
    let input = "<<IMG:/x.jpg>> and <<IMG:/y.png>> done";
    let (text, paths) = extract_img_markers(input);
    assert_eq!(text, "and  done");
    assert_eq!(paths, vec!["/x.jpg", "/y.png"]);
}

#[test]
fn empty_path_skipped() {
    let (text, paths) = extract_img_markers("<<IMG:>> rest");
    assert_eq!(text, "rest");
    assert!(paths.is_empty());
}

#[test]
fn unclosed_marker_left_intact() {
    let input = "<<IMG:/no_close text";
    let (text, paths) = extract_img_markers(input);
    assert_eq!(text, input);
    assert!(paths.is_empty());
}

#[test]
fn marker_only_input_returns_empty() {
    let (text, paths) = extract_img_markers("<<IMG:/only.png>>");
    assert_eq!(text, "");
    assert_eq!(paths, vec!["/only.png"]);
}

#[test]
fn whitespace_around_path_trimmed() {
    let (_, paths) = extract_img_markers("<<IMG:  /tmp/photo.jpg  >>");
    assert_eq!(paths, vec!["/tmp/photo.jpg"]);
}

#[test]
fn adjacent_markers() {
    let (text, paths) = extract_img_markers("<<IMG:/a.png>><<IMG:/b.png>>");
    assert_eq!(text, "");
    assert_eq!(paths.len(), 2);
}

#[test]
fn empty_input() {
    let (text, paths) = extract_img_markers("");
    assert_eq!(text, "");
    assert!(paths.is_empty());
}
