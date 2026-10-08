//! Local-file link delivery (#1916): what the bot does with a markdown link
//! whose target is a real file on disk.
//!
//! A `[label](/path/report.pdf)` link is what the model writes when it means
//! "the file is here". The reader is a channel user with NO filesystem
//! access, so the link is not the deliverable: the FILE is. Before #1916 the
//! link rendered as an inert entity whose URL was the raw path, Telegram had
//! no scheme to resolve, and the reader got a dead link and no file.
//!
//! Two halves, one file:
//!
//! 1. **The scanner** (`mod scan`). Which references become a `LocalFile`
//!    and which stay as literal text. The removal policy is the one place
//!    the file family deliberately differs from the image family: a resolved
//!    file leaves the text, but a REJECTED candidate stays byte-identical
//!    and is reported, because a link carries its own label and a silent
//!    strip would delete the reader's only clue about what was referenced.
//! 2. **The delivery leg** (`mod floor`). A resolved link ships exactly ONE
//!    document carrying the link label as its caption, and a file the
//!    channel could not deliver comes back as a failure entry the caller can
//!    NAME, never as a silent drop. The two failure classes are distinct: a
//!    path that cannot be READ is a reference problem (`Unreadable`), while
//!    a file the API refuses is a delivery problem (`DeliveryFailed`).
//!
//! Deliberately NOT covered here: the notice wording battery, the regen
//! ladder, and the extraction rules of the image family. Each has its own
//! home.

/// Minimal byte string that passes the magic-byte sniff for PNG. Only the
/// image-reference test below needs it: the file family has no format gate,
/// so its fixtures use PDF bytes instead.
const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x01\x02\x03\x04\x05\x06\x07";

// ---------------------------------------------------------------------------
// the scanner: resolution, removal, and what must be LEFT ALONE
// ---------------------------------------------------------------------------

mod scan {
    use super::PNG_BYTES;
    use crate::utils::image::{
        LocalFileScan, LocalImageFailure, LocalImageFailureReason, TELEGRAM_DOCUMENT_MAX_BYTES,
        append_file_failure_notice, extract_local_files, file_failure_notice, validate_local_file,
    };
    use std::path::{Path, PathBuf};

    const PDF_BYTES: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";

    fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    fn paths(scan: &LocalFileScan) -> Vec<PathBuf> {
        scan.attachments.iter().map(|f| f.path.clone()).collect()
    }

    fn failure(raw: &str) -> LocalImageFailure {
        LocalImageFailure {
            raw: raw.to_string(),
            resolved: None,
            reason: LocalImageFailureReason::NotFound,
        }
    }

    // -----------------------------------------------------------------------
    // Resolution and removal
    // -----------------------------------------------------------------------

    #[test]
    fn a_local_file_link_resolves_and_leaves_the_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let scan = extract_local_files(
            &format!("before [Q3 report]({}) after", pdf.display()),
            None,
        );
        assert_eq!(scan.text, "before  after");
        assert_eq!(paths(&scan), vec![pdf]);
        assert_eq!(scan.attachments[0].caption.as_deref(), Some("Q3 report"));
        assert!(scan.failures.is_empty());
    }

    #[test]
    fn an_empty_label_yields_no_caption() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let scan = extract_local_files(&format!("[]({})", pdf.display()), None);
        assert_eq!(paths(&scan), vec![pdf]);
        assert_eq!(
            scan.attachments[0].caption, None,
            "an empty label is not a caption"
        );
    }

    #[test]
    fn two_files_attach_in_order_of_appearance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = write_file(dir.path(), "a.pdf", PDF_BYTES);
        let b = write_file(dir.path(), "b.pdf", PDF_BYTES);
        let scan = extract_local_files(
            &format!("[A]({}) then [B]({})", a.display(), b.display()),
            None,
        );
        assert_eq!(paths(&scan), vec![a, b]);
        assert_eq!(scan.text, "then");
    }

    #[test]
    fn an_angle_bracket_target_holds_spaces() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "Q3 final.pdf", PDF_BYTES);
        let scan = extract_local_files(&format!("x [Q3](<{}>) y", pdf.display()), None);
        assert_eq!(scan.text, "x  y");
        assert_eq!(paths(&scan), vec![pdf]);
    }

    #[test]
    fn a_title_after_the_target_is_ignored_and_the_label_captions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let scan = extract_local_files(
            &format!("x [Q3 report]({} \"quarterly\") y", pdf.display()),
            None,
        );
        assert_eq!(scan.text, "x  y");
        assert_eq!(scan.attachments[0].caption.as_deref(), Some("Q3 report"));
    }

    #[test]
    fn a_relative_target_resolves_against_the_base_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("reports")).expect("mkdir");
        let pdf = write_file(&dir.path().join("reports"), "q3.pdf", PDF_BYTES);
        let scan = extract_local_files("see [report](reports/q3.pdf) here", Some(dir.path()));
        assert_eq!(scan.text, "see  here");
        assert_eq!(paths(&scan), vec![pdf]);
        assert_eq!(scan.attachments[0].caption.as_deref(), Some("report"));
    }

    // -----------------------------------------------------------------------
    // What the family must LEAVE ALONE
    // -----------------------------------------------------------------------

    #[test]
    fn a_relative_target_without_a_base_dir_stays_literal() {
        let text = "see [report](reports/q3.pdf) here";
        let scan = extract_local_files(text, None);
        assert_eq!(scan.text, text, "no base directory to resolve against");
        assert!(scan.attachments.is_empty());
        assert!(scan.failures.is_empty());
    }

    #[test]
    fn a_remote_link_is_left_alone() {
        let text = "see [the site](https://example.com/a) for details";
        let scan = extract_local_files(text, None);
        assert_eq!(scan.text, text, "Telegram resolves a real URL itself");
        assert!(scan.attachments.is_empty());
        assert!(scan.failures.is_empty(), "a URL is not a missing file");
    }

    #[test]
    fn an_image_reference_is_not_claimed_by_the_file_family() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png = write_file(dir.path(), "chart.png", PNG_BYTES);
        let text = format!("x ![chart]({}) y", png.display());
        let scan = extract_local_files(&text, None);
        assert_eq!(scan.text, text, "the image family owns `![...](...)`");
        assert!(scan.attachments.is_empty());
        assert!(scan.failures.is_empty());
    }

    #[test]
    fn a_link_inside_a_code_span_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let text = format!("`[Q3 report]({})`", pdf.display());
        let scan = extract_local_files(&text, None);
        assert_eq!(
            scan.text, text,
            "a code span is documentation, not a deliverable"
        );
        assert!(scan.attachments.is_empty());
        assert!(scan.failures.is_empty());
    }

    #[test]
    fn a_link_inside_a_code_fence_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let text = format!("```\n[Q3 report]({})\n```", pdf.display());
        let scan = extract_local_files(&text, None);
        assert_eq!(scan.text, text);
        assert!(scan.attachments.is_empty());
    }

    #[test]
    fn an_escaped_link_stays_literal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let text = format!("\\[Q3 report]({})", pdf.display());
        let scan = extract_local_files(&text, None);
        assert_eq!(scan.text, text);
        assert!(scan.attachments.is_empty());
    }

    #[test]
    fn a_non_file_scheme_or_anchor_is_left_alone() {
        for text in [
            "mail [me](mailto:a@b.c)",
            "call [me](tel:+1234)",
            "see [anchor](#section)",
            "see [nothing]()",
        ] {
            let scan = extract_local_files(text, None);
            assert_eq!(scan.text, text, "{text} must survive byte-identical");
            assert!(scan.attachments.is_empty(), "{text}");
            assert!(scan.failures.is_empty(), "{text}");
        }
    }

    #[test]
    fn prose_that_merely_looks_like_a_link_is_untouched() {
        for text in ["see [1] and [2] for details", "a [b] c", "[standalone]"] {
            let scan = extract_local_files(text, None);
            assert_eq!(scan.text, text);
            assert!(scan.attachments.is_empty());
            assert!(scan.failures.is_empty());
        }
    }

    // -----------------------------------------------------------------------
    // Rejections: reported AND left in the text
    // -----------------------------------------------------------------------

    #[test]
    fn a_missing_file_is_reported_and_the_link_survives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("gone.pdf");
        let raw = format!("[Q3 report]({})", missing.display());
        let text = format!("x {raw} y");
        let scan = extract_local_files(&text, Some(dir.path()));
        assert_eq!(
            scan.text, text,
            "a rejected link keeps its label: it is the reader's only clue"
        );
        assert!(scan.attachments.is_empty());
        assert_eq!(scan.failures.len(), 1);
        assert_eq!(scan.failures[0].reason, LocalImageFailureReason::NotFound);
        assert_eq!(scan.failures[0].raw, raw);
        assert_eq!(
            scan.failures[0].resolved.as_deref(),
            Some(missing.as_path())
        );
    }

    #[test]
    fn a_directory_is_not_a_regular_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("reports");
        std::fs::create_dir(&sub).expect("mkdir");
        let scan = extract_local_files(&format!("[x]({})", sub.display()), None);
        assert!(scan.attachments.is_empty());
        assert_eq!(scan.failures[0].reason, LocalImageFailureReason::NotAFile);
    }

    #[test]
    fn an_empty_file_is_reported_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty = write_file(dir.path(), "empty.pdf", b"");
        let scan = extract_local_files(&format!("[x]({})", empty.display()), None);
        assert!(scan.attachments.is_empty());
        assert_eq!(scan.failures[0].reason, LocalImageFailureReason::Empty);
    }

    #[test]
    fn a_file_past_the_document_ceiling_is_reported_as_too_large() {
        let dir = tempfile::tempdir().expect("tempdir");
        let big = dir.path().join("big.pdf");
        // A sparse file: `set_len` costs no disk and no time, where writing
        // 50 MiB of fixture bytes would.
        std::fs::File::create(&big)
            .expect("create")
            .set_len(TELEGRAM_DOCUMENT_MAX_BYTES + 1)
            .expect("set_len");
        let scan = extract_local_files(&format!("[x]({})", big.display()), None);
        assert!(scan.attachments.is_empty());
        assert_eq!(scan.failures[0].reason, LocalImageFailureReason::TooLarge);
    }

    #[test]
    fn validate_accepts_a_readable_non_empty_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        assert!(validate_local_file(&pdf).is_ok());
    }

    // -----------------------------------------------------------------------
    // The notice: the honest line when delivery did not happen
    // -----------------------------------------------------------------------

    #[test]
    fn no_failures_produces_no_file_notice() {
        assert!(file_failure_notice(&[]).is_none());
    }

    #[test]
    fn a_file_notice_names_the_reference_and_the_reason() {
        let notice =
            file_failure_notice(&[failure("[Q3 report](/root/reports/q3.pdf)")]).expect("notice");
        assert!(notice.contains("File not attached"), "{notice}");
        assert!(notice.contains("a file"), "{notice}");
        assert!(
            notice.contains("[Q3 report](/root/reports/q3.pdf)"),
            "{notice}"
        );
        assert!(notice.contains("file not found"), "{notice}");
    }

    #[test]
    fn appending_a_file_notice_keeps_the_body_and_adds_a_blank_line() {
        let body = append_file_failure_notice("hello", &[failure("[x](/nope)")]);
        assert!(body.starts_with("hello\n\n⚠️ File not attached"), "{body}");
    }

    #[test]
    fn an_empty_body_becomes_the_file_notice_alone() {
        let body = append_file_failure_notice("   ", &[failure("[x](/nope)")]);
        assert!(body.starts_with("⚠️ File not attached"), "{body}");
    }

    #[test]
    fn appending_without_failures_returns_the_body_untouched() {
        assert_eq!(append_file_failure_notice("hello", &[]), "hello");
    }
}

// ---------------------------------------------------------------------------
// the delivery leg: one document bubble per link, captioned by the label
// ---------------------------------------------------------------------------

mod floor {
    use crate::channels::telegram::delivery::send_local_files;
    use crate::utils::image::{LocalFile, LocalImageFailureReason};
    use std::path::{Path, PathBuf};

    const CHAT: i64 = 133_526_395;

    /// A minimal PDF header: the bytes a real report starts with. The file
    /// family has no format gate (Telegram renders no document preview, so
    /// there is nothing to decode), which makes this a realistic fixture
    /// rather than a required signature; nothing in the send path inspects it.
    const PDF_BYTES: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";

    /// A response shaped like a successful text send. Only the ROUTING is
    /// under test, so the body mirrors the proven shape rather than inventing
    /// a media object.
    const SEND_MESSAGE_OK: &str = r#"{"ok":true,"result":{"message_id":701,"date":1757166400,"chat":{"id":133526395,"type":"private"},"text":"ok"}}"#;

    /// What Telegram answers for an upload it will not take.
    const REFUSED: &str =
        r#"{"ok":false,"error_code":413,"description":"Request Entity Too Large"}"#;

    /// A bot pointed at the mockito server.
    ///
    /// MOCK PATHS ARE PASCALCASE FOR EVERY TELOXIDE REQUEST: teloxide builds
    /// the method segment from the payload struct name, so
    /// `bot.send_document(..)` hits `/botTESTTOKEN/SendDocument`, never the
    /// lowercase form the Bot API docs use. A lowercase mock never matches:
    /// mockito serves its own unmatched 501, whose empty body teloxide
    /// reports as `InvalidJson` and the send reads as a network failure.
    fn test_bot(server: &mockito::ServerGuard) -> teloxide::Bot {
        teloxide::Bot::with_client(
            "TESTTOKEN",
            reqwest_teloxide::Client::builder().build().unwrap(),
        )
        .set_api_url(server.url().parse().unwrap())
    }

    fn file_at(path: PathBuf, caption: Option<&str>) -> LocalFile {
        LocalFile {
            path,
            caption: caption.map(str::to_string),
        }
    }

    fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    // -----------------------------------------------------------------------
    // the failure-entry contract
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn an_unreadable_file_returns_a_failure_entry() {
        // The reference resolves to a path that is not there by the time the
        // send runs. The caller must be able to NAME it: a document the model
        // announced that vanishes with no notice is the #502 failure mode
        // (silent in both directions).
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("gone.pdf");
        let server = mockito::Server::new_async().await;
        let bot = test_bot(&server);

        let (delivered, failures) = send_local_files(
            uuid::Uuid::new_v4(),
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &[file_at(missing.clone(), None)],
        )
        .await;

        assert!(delivered.is_empty(), "nothing was delivered");
        assert_eq!(failures.len(), 1, "the unreadable file must be reported");
        assert_eq!(failures[0].raw, missing.display().to_string());
        assert_eq!(
            failures[0].reason,
            LocalImageFailureReason::Unreadable,
            "a path that cannot be read is a REFERENCE problem, not a delivery one"
        );
    }

    #[tokio::test]
    async fn a_file_the_channel_refuses_returns_a_delivery_failure() {
        // The file is readable and inside the ceiling, so the reference is
        // fine: the API is what says no. That distinction is what routes the
        // correction, because a reference problem can be repaired by
        // rewriting the link and a delivery problem cannot.
        let dir = tempfile::tempdir().expect("tempdir");
        let report = write_fixture(dir.path(), "q3.pdf", PDF_BYTES);
        let mut server = mockito::Server::new_async().await;
        let document_mock = server
            .mock("POST", "/botTESTTOKEN/SendDocument")
            .with_status(413)
            .with_header("content-type", "application/json")
            .with_body(REFUSED)
            .expect(1)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let (delivered, failures) = send_local_files(
            uuid::Uuid::new_v4(),
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &[file_at(report.clone(), Some("Q3 report"))],
        )
        .await;

        document_mock.assert_async().await;
        assert!(
            delivered.is_empty(),
            "the channel refused it, so nothing was delivered"
        );
        assert_eq!(failures.len(), 1, "the refusal must be reported");
        assert_eq!(
            failures[0].reason,
            LocalImageFailureReason::DeliveryFailed,
            "a file the channel refused is a DELIVERY problem: the reference itself was fine"
        );
    }

    // -----------------------------------------------------------------------
    // the promise: one document bubble, captioned by the link label
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn a_resolved_link_ships_exactly_one_document_captioned_by_its_label() {
        // The positive control for the two tests above, and the feature's
        // core assertion at once: the instrument must be able to return an
        // empty failure list (or "it reported a failure" proves nothing about
        // which inputs fail), and the caption must ride the request as the
        // link LABEL, not the path, which the user cannot open anyway.
        let dir = tempfile::tempdir().expect("tempdir");
        let report = write_fixture(dir.path(), "q3.pdf", PDF_BYTES);
        let mut server = mockito::Server::new_async().await;
        // `expect(1)` is the whole point: a second bubble for one link is the
        // #502/#360 duplicate, one media family over. The body match pins the
        // caption, since the label is the only part of the link the user gets
        // to read in the chat.
        let document_mock = server
            .mock("POST", "/botTESTTOKEN/SendDocument")
            .match_body(mockito::Matcher::Regex("Q3 report".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SEND_MESSAGE_OK)
            .expect(1)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let (delivered, failures) = send_local_files(
            uuid::Uuid::new_v4(),
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &[file_at(report.clone(), Some("Q3 report"))],
        )
        .await;

        document_mock.assert_async().await;
        assert_eq!(delivered, vec![report], "the delivered path comes back");
        assert!(failures.is_empty(), "a delivered file reports no failure");
    }

    #[tokio::test]
    async fn a_link_without_a_label_ships_the_document_with_no_caption() {
        // `[](path)`: an empty label is not a caption. Sending `Some("")`
        // would make Telegram render an empty caption bubble; the scanner
        // already drops the empty label, and this pins that the send path
        // does not resurrect it.
        let dir = tempfile::tempdir().expect("tempdir");
        let report = write_fixture(dir.path(), "q3.pdf", PDF_BYTES);
        let mut server = mockito::Server::new_async().await;
        let document_mock = server
            .mock("POST", "/botTESTTOKEN/SendDocument")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SEND_MESSAGE_OK)
            .expect(1)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let (delivered, failures) = send_local_files(
            uuid::Uuid::new_v4(),
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &[file_at(report.clone(), None)],
        )
        .await;

        document_mock.assert_async().await;
        assert_eq!(delivered, vec![report]);
        assert!(failures.is_empty());
    }

    #[tokio::test]
    async fn each_link_ships_its_own_bubble_in_order() {
        // Two links are two documents, not one, and the order the reply wrote
        // them in is the order they arrive, so the reply's prose still reads
        // top-to-bottom against the bubbles beside it.
        let dir = tempfile::tempdir().expect("tempdir");
        let first = write_fixture(dir.path(), "q3.pdf", PDF_BYTES);
        let second = write_fixture(dir.path(), "q4.pdf", PDF_BYTES);
        let mut server = mockito::Server::new_async().await;
        let document_mock = server
            .mock("POST", "/botTESTTOKEN/SendDocument")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SEND_MESSAGE_OK)
            .expect(2)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let (delivered, failures) = send_local_files(
            uuid::Uuid::new_v4(),
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &[
                file_at(first.clone(), Some("Q3")),
                file_at(second.clone(), Some("Q4")),
            ],
        )
        .await;

        document_mock.assert_async().await;
        assert_eq!(
            delivered,
            vec![first, second],
            "delivered in the reply's order"
        );
        assert!(failures.is_empty());
    }
}
