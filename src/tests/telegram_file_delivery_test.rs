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
//!    and which stay as literal text. The marker policy is the one place
//!    the file family deliberately differs from the image family: a
//!    resolved file becomes an attachment and its link is replaced by a
//!    visible `📎 <label>` marker, while a REJECTED candidate stays
//!    byte-identical and is reported, because a link carries its own label
//!    and a silent strip would delete the reader's only clue about what
//!    was referenced.
//! 2. **The delivery leg** (`mod floor`). A resolved link ships exactly ONE
//!    document carrying the link label as its caption, and a file the
//!    channel could not deliver comes back as a failure entry the caller can
//!    NAME, never as a silent drop. The two failure classes are distinct: a
//!    path that cannot be READ is a reference problem (`Unreadable`), while
//!    a file the API refuses is a delivery problem (`DeliveryFailed`).
//! 3. **The bubble links** (`mod links`, #1918). A delivered file's `📎`
//!    marker becomes a `t.me` link to the document bubble, so a channel
//!    reader can tap the marker and land on the file. Links exist only for
//!    public supergroup and channel chats; a private chat keeps the plain
//!    marker. A link is spliced only where the recorded span still matches
//!    the buffer; a moved span degrades to the plain marker rather than
//!    link unrelated words.
//! 4. **The rich rewrite** (`mod rich`, #1918). On the rich plane the file
//!    goes one better than a marker: each resolvable reference is rewritten
//!    IN PLACE into a `tg://document?id=` reference and the document rides
//!    the bubble's media array, so the file renders at its position in the
//!    report. Already-delivered files consume their link and rewrite to
//!    nothing; remote links, code spans and unresolvable paths stay
//!    byte-identical.
//! 5. **The intermediate plane** (`mod intermediate`, #1918). A rich report
//!    emitted mid-turn runs the same scan: resolvable files inline into its
//!    media array, and when the rich send declines, each file drops to the
//!    floor as its own document so the marker never points at a file the
//!    chat never received. A repeated intermediate re-delivers nothing.
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
    // Resolution, the visible marker, and what must be LEFT ALONE
    // -----------------------------------------------------------------------

    #[test]
    fn a_resolved_link_becomes_a_visible_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let scan = extract_local_files(
            &format!("before [Q3 report]({}) after", pdf.display()),
            None,
        );
        assert_eq!(scan.text, "before 📎 Q3 report after");
        assert_eq!(paths(&scan), vec![pdf]);
        assert_eq!(scan.attachments[0].caption.as_deref(), Some("Q3 report"));
        let span = scan.attachments[0]
            .marker_span
            .clone()
            .expect("the scan records the marker's span");
        assert_eq!(&scan.text[span], "📎 Q3 report");
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
    fn an_empty_label_falls_back_to_the_basename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        let scan = extract_local_files(&format!("[]({})", pdf.display()), None);
        assert_eq!(scan.text, "📎 q3.pdf", "the marker is never blank");
        let span = scan.attachments[0]
            .marker_span
            .clone()
            .expect("the scan records the marker's span");
        assert_eq!(&scan.text[span], "📎 q3.pdf");
    }

    #[test]
    fn the_marker_span_indexes_the_final_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "q3.pdf", PDF_BYTES);
        // Leading AND trailing whitespace force the trim rebase: the span
        // was recorded while the buffer still carried the leading run, so
        // an unrebased span would index the WRONG bytes of the final text.
        let scan = extract_local_files(&format!("  \n [Q3 report]({})  ", pdf.display()), None);
        assert_eq!(scan.text, "📎 Q3 report");
        let span = scan.attachments[0]
            .marker_span
            .clone()
            .expect("the scan records the marker's span");
        assert_eq!(&scan.text[span], "📎 Q3 report");
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
        assert_eq!(scan.text, "📎 A then 📎 B");
        let span_a = scan.attachments[0]
            .marker_span
            .clone()
            .expect("the scan records the marker's span");
        let span_b = scan.attachments[1]
            .marker_span
            .clone()
            .expect("the scan records the marker's span");
        assert_eq!(&scan.text[span_a], "📎 A");
        assert_eq!(&scan.text[span_b], "📎 B");
    }

    #[test]
    fn an_angle_bracket_target_holds_spaces() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_file(dir.path(), "Q3 final.pdf", PDF_BYTES);
        let scan = extract_local_files(&format!("x [Q3](<{}>) y", pdf.display()), None);
        assert_eq!(scan.text, "x 📎 Q3 y");
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
        assert_eq!(scan.text, "x 📎 Q3 report y");
        assert_eq!(scan.attachments[0].caption.as_deref(), Some("Q3 report"));
    }

    #[test]
    fn a_relative_target_resolves_against_the_base_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("reports")).expect("mkdir");
        let pdf = write_file(&dir.path().join("reports"), "q3.pdf", PDF_BYTES);
        let scan = extract_local_files("see [report](reports/q3.pdf) here", Some(dir.path()));
        assert_eq!(scan.text, "see 📎 report here");
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
    use crate::channels::telegram::delivery::{DeliveredFile, send_local_files};
    use crate::utils::image::{LocalFile, LocalImageFailureReason};
    use std::path::{Path, PathBuf};

    /// The paths of what landed, in order. Every floor assertion reads this
    /// projection; the message ids are asserted separately where they matter.
    fn delivered_paths(delivered: &[DeliveredFile]) -> Vec<PathBuf> {
        delivered.iter().map(|f| f.path.clone()).collect()
    }

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
            marker_span: None,
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
        assert_eq!(
            delivered_paths(&delivered),
            vec![report],
            "the delivered path comes back"
        );
        // The bubble id is the half the #1918 link builder consumes: the
        // marker is spliced into a t.me link to THIS id.
        assert_eq!(delivered[0].message_id, 701, "the bubble id comes back");
        assert!(failures.is_empty(), "a delivered file reports no failure");
    }

    #[tokio::test]
    async fn a_delivered_document_arrives_named_after_its_file() {
        // #1937: `InputFile::memory` carries no filename and teloxide's
        // guess for raw bytes is empty, so Telegram labelled every delivered
        // document "file" with no MIME. The multipart part's filename is
        // where the name travels from, and the extension is what teloxide
        // derives the MIME type from, so the body must carry both.
        let dir = tempfile::tempdir().expect("tempdir");
        let report = write_fixture(dir.path(), "q3-report.pdf", PDF_BYTES);
        let mut server = mockito::Server::new_async().await;
        let document_mock = server
            .mock("POST", "/botTESTTOKEN/SendDocument")
            .match_body(mockito::Matcher::Regex(
                // The name travels in the part's filename attribute; teloxide
                // sets no per-part content type, so the MIME is not asserted.
                r#"filename="q3-report\.pdf""#.to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SEND_MESSAGE_OK)
            .expect(1)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let (_, failures) = send_local_files(
            uuid::Uuid::new_v4(),
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &[file_at(report, Some("Q3 report"))],
        )
        .await;

        document_mock.assert_async().await;
        assert!(failures.is_empty(), "a named document still delivers");
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
        assert_eq!(delivered_paths(&delivered), vec![report]);
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
            delivered_paths(&delivered),
            vec![first, second],
            "delivered in the reply's order"
        );
        assert!(failures.is_empty());
    }
}

// ---------------------------------------------------------------------------
// the bubble link: a delivered file's marker points at its t.me bubble
// ---------------------------------------------------------------------------

mod links {
    use crate::channels::telegram::delivery::{file_message_link, link_file_markers};
    use crate::utils::image::extract_local_files;
    use std::path::Path;

    const PDF_BYTES: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";
    const SUPERGROUP_ID: i64 = -100_123_456_789_012;

    fn write_fixture(dir: &Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, PDF_BYTES).expect("write fixture");
        path
    }

    /// A `Chat` from its wire form: the kind variants carry their own
    /// payloads, and building them from JSON keeps the test honest about
    /// what Telegram actually sends instead of hand-filling internals.
    fn chat_of_kind(v: serde_json::Value) -> teloxide::types::Chat {
        serde_json::from_value(v).expect("chat json")
    }

    #[test]
    fn a_supergroup_bubble_gets_a_message_link() {
        let chat = chat_of_kind(serde_json::json!({
            "id": SUPERGROUP_ID, "type": "supergroup", "title": "Ops",
        }));
        let link =
            file_message_link(&chat.kind, SUPERGROUP_ID, None, 701).expect("supergroup links");
        // The internal id drops the -100 marker; anything else would 404.
        assert_eq!(link, "https://t.me/c/123456789012/701");
    }

    #[test]
    fn a_topic_message_links_into_its_topic() {
        let chat = chat_of_kind(serde_json::json!({
            "id": SUPERGROUP_ID, "type": "supergroup", "title": "Ops", "is_forum": true,
        }));
        let thread = teloxide::types::ThreadId(teloxide::types::MessageId(42));
        let link = file_message_link(&chat.kind, SUPERGROUP_ID, Some(thread), 701)
            .expect("topic messages link");
        // The middle segment lands the reader INSIDE the topic, not at the
        // top of a thread they then have to search.
        assert_eq!(link, "https://t.me/c/123456789012/42/701");
    }

    #[test]
    fn a_channel_bubble_gets_a_message_link() {
        const CHANNEL_ID: i64 = -100_999_888_777_666;
        let chat = chat_of_kind(serde_json::json!({
            "id": CHANNEL_ID, "type": "channel", "title": "Announcements",
        }));
        let link =
            file_message_link(&chat.kind, CHANNEL_ID, None, 5).expect("channel bubbles link");
        assert_eq!(link, "https://t.me/c/999888777666/5");
    }

    #[test]
    fn a_private_chat_has_no_message_link() {
        let chat = chat_of_kind(serde_json::json!({
            "id": 133_526_395, "type": "private", "first_name": "A",
        }));
        assert!(file_message_link(&chat.kind, 133_526_395, None, 701).is_none());
    }

    #[test]
    fn a_basic_group_has_no_message_link() {
        let chat = chat_of_kind(serde_json::json!({
            "id": -456_789, "type": "group", "title": "Team",
        }));
        assert!(file_message_link(&chat.kind, -456_789, None, 701).is_none());
    }

    #[test]
    fn a_delivered_files_link_is_spliced_onto_its_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_fixture(dir.path(), "q3.pdf");
        let scan = extract_local_files("Here is [Q3 report](q3.pdf).", Some(dir.path()));
        assert_eq!(scan.text, "Here is 📎 Q3 report.");

        let linked = link_file_markers(
            &scan.text,
            &scan,
            &[(pdf, "https://t.me/c/123456789012/701".to_string())],
        );
        assert_eq!(
            linked, "Here is [📎 Q3 report](https://t.me/c/123456789012/701).",
            "the marker becomes a link to the bubble, everything else is untouched"
        );
    }

    #[test]
    fn prose_around_a_spliced_marker_is_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_fixture(dir.path(), "q3.pdf");
        let scan = extract_local_files("before [x](q3.pdf) after", Some(dir.path()));
        let linked = link_file_markers(
            &scan.text,
            &scan,
            &[(pdf, "https://t.me/c/1/1".to_string())],
        );
        assert!(linked.starts_with("before "));
        assert!(linked.ends_with(" after"));
        assert!(linked.contains("[📎 x](https://t.me/c/1/1)"));
    }

    #[test]
    fn a_moved_span_degrades_to_the_plain_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_fixture(dir.path(), "q3.pdf");
        let scan = extract_local_files("Here is [Q3 report](q3.pdf).", Some(dir.path()));
        // The buffer moved under the span (a prefix landed): cutting the
        // recorded range would splice a link over unrelated words, so the
        // rewrite must decline and the marker stays plain.
        let shifted = format!(">> {}", scan.text);
        let linked = link_file_markers(
            &shifted,
            &scan,
            &[(pdf, "https://t.me/c/1/701".to_string())],
        );
        assert_eq!(linked, shifted, "a moved span is never spliced");
    }

    #[test]
    fn an_absent_link_keeps_the_plain_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), "q3.pdf");
        let scan = extract_local_files("Here is [Q3 report](q3.pdf).", Some(dir.path()));
        // No links at all: a private chat (no link form) or a refusal. The
        // marker is the reader's anchor either way; it never disappears.
        assert_eq!(link_file_markers(&scan.text, &scan, &[]), scan.text);
        // A DIFFERENT file's link must not claim this marker either.
        let other = write_fixture(dir.path(), "other.pdf");
        let linked = link_file_markers(
            &scan.text,
            &scan,
            &[(other, "https://t.me/c/1/2".to_string())],
        );
        assert_eq!(linked, scan.text);
    }
}

// ---------------------------------------------------------------------------
// the rich rewrite: a resolvable link becomes an in-place tg://document ref
// ---------------------------------------------------------------------------

mod rich {
    use crate::utils::image::{DOC_ID_PREFIX, extract_local_files, rewrite_local_files};
    use std::path::{Path, PathBuf};

    const PDF_BYTES: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";

    fn write_fixture(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, PDF_BYTES).expect("write fixture");
        path
    }

    #[test]
    fn a_resolved_link_is_rewritten_in_place_as_a_document_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_fixture(dir.path(), "q3.pdf");

        let rw = rewrite_local_files(
            "Read [report](q3.pdf) first.",
            Some(dir.path()),
            DOC_ID_PREFIX,
            &[],
        );
        assert_eq!(
            rw.rich, "Read ![📎 report](tg://document?id=doc0) first.",
            "the reference takes the link's exact position"
        );
        assert_eq!(rw.entries.len(), 1, "one entry per rewritten reference");
        assert_eq!(rw.entries[0].id, "doc0");
        assert_eq!(rw.entries[0].file.path, pdf);
        assert_eq!(
            rw.entries[0].file.caption.as_deref(),
            Some("report"),
            "the label captions the document, as the floor does"
        );
    }

    #[test]
    fn two_files_get_doc0_and_doc1_in_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = write_fixture(dir.path(), "a.pdf");
        let second = write_fixture(dir.path(), "b.pdf");

        let rw = rewrite_local_files(
            "[A](a.pdf) then [B](b.pdf)",
            Some(dir.path()),
            DOC_ID_PREFIX,
            &[],
        );
        assert_eq!(
            rw.rich,
            "![📎 A](tg://document?id=doc0) then ![📎 B](tg://document?id=doc1)"
        );
        assert_eq!(rw.entries[0].file.path, first);
        assert_eq!(rw.entries[1].file.path, second);
    }

    #[test]
    fn a_remote_link_is_left_byte_identical() {
        let rw = rewrite_local_files(
            "See [docs](https://example.com/x.pdf) online.",
            None,
            DOC_ID_PREFIX,
            &[],
        );
        assert_eq!(rw.rich, "See [docs](https://example.com/x.pdf) online.");
        assert!(rw.entries.is_empty());
    }

    #[test]
    fn a_link_inside_a_code_span_is_left_byte_identical() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), "q3.pdf");

        let rw = rewrite_local_files(
            "run `cat [x](q3.pdf)` now",
            Some(dir.path()),
            DOC_ID_PREFIX,
            &[],
        );
        assert_eq!(rw.rich, "run `cat [x](q3.pdf)` now");
        assert!(rw.entries.is_empty());
    }

    #[test]
    fn a_missing_file_stays_a_verbatim_link() {
        // A rejected candidate is the SCAN's to report; the rewrite stays
        // silent and the reader keeps the link that carries its label.
        let dir = tempfile::tempdir().expect("tempdir");
        let rw = rewrite_local_files(
            "See [ghost](nope.pdf).",
            Some(dir.path()),
            DOC_ID_PREFIX,
            &[],
        );
        assert_eq!(rw.rich, "See [ghost](nope.pdf).");
        assert!(rw.entries.is_empty());
    }

    #[test]
    fn an_already_delivered_file_is_consumed_not_referenced() {
        // The file is already in the chat (an intermediate shipped it).
        // Inlining it AGAIN would put a second copy in the chat, and leaving
        // the link would point at a file the reader already has: consume the
        // reference and record nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_fixture(dir.path(), "q3.pdf");

        let rw = rewrite_local_files(
            "Read [report](q3.pdf) first.",
            Some(dir.path()),
            DOC_ID_PREFIX,
            &[pdf],
        );
        assert_eq!(rw.rich, "Read  first.", "the consumed link leaves quietly");
        assert!(rw.entries.is_empty());
    }

    #[test]
    fn a_custom_prefix_namespaces_the_ids() {
        // Entries are matched to references BY ID inside one message's media
        // array, so two families cannot share a prefix.
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), "q3.pdf");

        let rw = rewrite_local_files("[r](q3.pdf)", Some(dir.path()), "attachment", &[]);
        assert_eq!(rw.rich, "![📎 r](tg://document?id=attachment0)");
    }

    #[test]
    fn an_empty_label_falls_back_to_the_basename_in_the_alt() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), "q3.pdf");

        let rw = rewrite_local_files("[](q3.pdf)", Some(dir.path()), DOC_ID_PREFIX, &[]);
        assert_eq!(rw.rich, "![📎 q3.pdf](tg://document?id=doc0)");
        assert_eq!(rw.entries[0].file.caption, None, "no label, no caption");
    }

    #[test]
    fn the_scan_and_the_rewrite_agree_on_what_is_a_file() {
        // One input, both walks: the marker form (floor) and the reference
        // form (rich) must claim the SAME references, or a link could be
        // reported twice or not at all.
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), "q3.pdf");
        let input = "[ok](q3.pdf) and [remote](https://x/y) and `[/code/q.pdf](q.pdf)`";

        let scan = extract_local_files(input, Some(dir.path()));
        let rw = rewrite_local_files(input, Some(dir.path()), DOC_ID_PREFIX, &[]);
        assert_eq!(scan.attachments.len(), rw.entries.len(), "same claims");
        assert_eq!(scan.attachments[0].path, rw.entries[0].file.path);
        // The scanner keeps everything it did not claim byte-identical, and
        // so must the rewrite, minus the one claim.
        assert!(scan.text.contains("📎"));
        assert!(rw.rich.contains("[remote](https://x/y)"));
        assert!(rw.rich.contains("`[/code/q.pdf](q.pdf)`"));
    }

    #[test]
    fn a_hand_written_document_reference_passes_through_byte_identical() {
        // The rewriter owns references it EMITS; a tg:// link the model
        // wrote itself is a telegram media ref, not a local path, and
        // rewriting it would double-encode a reference that already points
        // at the wire.
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), "q3.pdf");

        let rw = rewrite_local_files(
            "See ![sheet](tg://document?id=q3) and [report](q3.pdf).",
            Some(dir.path()),
            DOC_ID_PREFIX,
            &[],
        );

        assert!(
            rw.rich.contains("![sheet](tg://document?id=q3)"),
            "a hand-written tg:// reference survives untouched: {:?}",
            rw.rich
        );
        assert_eq!(rw.entries.len(), 1, "only the local link produces an entry");
        assert_eq!(rw.entries[0].id, "doc0");
    }
}

// ---------------------------------------------------------------------------
// the intermediate plane: a rich report's files ship (inline) or mark (floor)
// ---------------------------------------------------------------------------

mod intermediate {
    use crate::channels::telegram::TelegramState;
    use crate::channels::telegram::flow::StreamingState;
    use crate::channels::telegram::intermediates::deliver_intermediate_message;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const CHAT: i64 = 133_526_395;
    const PDF_BYTES: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n";
    const RICH_OK: &str = r#"{"ok":true,"result":{"message_id":812}}"#;
    const SEND_MESSAGE_OK: &str = r#"{"ok":true,"result":{"message_id":701,"date":1757166400,"chat":{"id":133526395,"type":"private"},"text":"ok"}}"#;

    fn write_fixture(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, PDF_BYTES).expect("write fixture");
        path
    }

    fn test_bot(server: &mockito::ServerGuard) -> teloxide::Bot {
        teloxide::Bot::with_client(
            "TESTTOKEN",
            reqwest_teloxide::Client::builder().build().unwrap(),
        )
        .set_api_url(server.url().parse().unwrap())
    }

    /// Minimal StreamingState for the intermediate plane (field set mirrors
    /// the house pattern in src/tests/telegram_state_test.rs, plus the
    /// #1918 delivered ledger this module reads back).
    fn streaming() -> Arc<std::sync::Mutex<StreamingState>> {
        Arc::new(std::sync::Mutex::new(StreamingState {
            is_dm: true,
            pending_suggestions: None,
            pending_trailer: None,
            delivered_file_paths: Vec::new(),
            msg_id: None,
            thinking: String::new(),
            tool_msgs: Vec::new(),
            display_queue: Vec::new(),
            open_group_msg_id: None,
            rich_transport_failures: 0,
            flow_entries: Vec::new(),
            flow_status: None,
            flow_rich: false,
            response: String::new(),
            final_bubble: None,
            dirty: false,
            recreate: false,
            header_preview: None,
            compacting: false,
            sections: Default::default(),
            retained_goal: None,
            applied_plan_kb: Default::default(),
            tool_round_count: 0,
            tools_started_at: None,
            turn_started_at: std::time::Instant::now(),
            flow_outcome: None,
            bg_indicator: None,
            bg_count: None,
            subagent_counts: Default::default(),
            sent_intermediates: Vec::new(),
            intermediate_msg_ids: Vec::new(),
            voice_msg_ids: Vec::new(),
            processing: true,
            is_cli: false,
        }))
    }

    #[tokio::test]
    async fn a_report_with_a_file_inlines_it_in_the_rich_media_array() {
        // The rich plane owns the file: the document rides the bubble's media
        // array at its reference, so there is NO detached document bubble,
        // and the file is recorded as delivered so the final leg cannot
        // ship a second copy.
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_fixture(dir.path(), "q3.pdf");
        let mut server = mockito::Server::new_async().await;
        let rich_mock = server
            .mock("POST", "/botTESTTOKEN/sendRichMessage")
            .match_body(mockito::Matcher::Regex(
                "tg://document\\?id=doc0".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(RICH_OK)
            .expect(1)
            .create_async()
            .await;
        let document_mock = server
            .mock("POST", "/botTESTTOKEN/SendDocument")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SEND_MESSAGE_OK)
            // expect(0) is the assertion: an inlined document never also
            // ships as its own detached bubble.
            .expect(0)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let st = streaming();
        let tg = TelegramState::new();
        let ok = deliver_intermediate_message(
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &st,
            &tg,
            uuid::Uuid::new_v4(),
            dir.path(),
            "Report ready: [q3.pdf](q3.pdf)\n\nSome prose so the report has a body.",
        )
        .await;

        assert!(ok, "the rich bubble is the delivery");
        rich_mock.assert_async().await;
        document_mock.assert_async().await;
        let s = st.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            s.delivered_file_paths,
            vec![pdf],
            "an inlined file counts as delivered for the final leg"
        );
        assert_eq!(s.sent_intermediates.len(), 1);
    }

    #[tokio::test]
    async fn a_declined_rich_send_ships_the_file_as_its_own_bubble() {
        // When the rich send fails, the floor is the file's only remaining
        // leg: it ships as its own document bubble and the marker text
        // carries the reader's anchor. A marker with no delivery behind it
        // would point at a file that never reaches the chat.
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = write_fixture(dir.path(), "q3.pdf");
        let mut server = mockito::Server::new_async().await;
        // The rich ladder fires twice on a dead endpoint: markdown+media
        // first, then the HTML dialect fallback. Both decline, and only then
        // does the file drop to the floor.
        let rich_mock = server
            .mock("POST", "/botTESTTOKEN/sendRichMessage")
            .with_status(500)
            .with_body("boom")
            .expect(2)
            .create_async()
            .await;
        let document_mock = server
            .mock("POST", "/botTESTTOKEN/SendDocument")
            .match_body(mockito::Matcher::Regex("q3".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SEND_MESSAGE_OK)
            .expect(1)
            .create_async()
            .await;
        let text_mock = server
            .mock("POST", "/botTESTTOKEN/SendMessage")
            .match_body(mockito::Matcher::Regex("q3.pdf".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SEND_MESSAGE_OK)
            .expect(1)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let st = streaming();
        let tg = TelegramState::new();
        let ok = deliver_intermediate_message(
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &st,
            &tg,
            uuid::Uuid::new_v4(),
            dir.path(),
            "Report ready: [q3.pdf](q3.pdf)\n\nProse body for the bubble.",
        )
        .await;

        rich_mock.assert_async().await;
        document_mock.assert_async().await;
        text_mock.assert_async().await;
        assert!(ok);
        let s = st.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(s.delivered_file_paths, vec![pdf]);
    }

    #[tokio::test]
    async fn a_repeated_intermediate_does_not_ship_its_file_twice() {
        // The dedup ledger keeps the PRE-scan text, so an identical
        // intermediate misses no check and returns before the scan runs:
        // the file shipped the first time, and the second attempt must add
        // no request at all.
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture(dir.path(), "q3.pdf");
        let mut server = mockito::Server::new_async().await;
        let rich_mock = server
            .mock("POST", "/botTESTTOKEN/sendRichMessage")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(RICH_OK)
            .expect(1)
            .create_async()
            .await;

        let bot = test_bot(&server);
        let st = streaming();
        let tg = TelegramState::new();
        let text = "Report ready: [q3.pdf](q3.pdf)\n\nProse body for the bubble.";
        let first = deliver_intermediate_message(
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &st,
            &tg,
            uuid::Uuid::new_v4(),
            dir.path(),
            text,
        )
        .await;
        let second = deliver_intermediate_message(
            &bot,
            teloxide::types::ChatId(CHAT),
            None,
            &st,
            &tg,
            uuid::Uuid::new_v4(),
            dir.path(),
            text,
        )
        .await;

        assert!(first);
        assert!(second, "the repeat reports success without re-delivering");
        rich_mock.assert_async().await; // exactly ONE rich send happened
    }

    // The shield invariant (#1921): a tg://document reference reaches the
    // wire only with its media entry. The probe is where that is enforced.
    #[cfg(unix)]
    #[test]
    fn the_probe_reports_an_unreadable_file_instead_of_its_bytes() {
        use crate::utils::image::{LocalFile, probe_document_bytes};
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let good = write_fixture(dir.path(), "good.pdf");
        let sealed = dir.path().join("sealed.pdf");
        std::fs::write(&sealed, b"%PDF-1.7\n").expect("write sealed");
        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000))
            .expect("seal the file");

        let files = vec![
            LocalFile {
                path: good.clone(),
                caption: Some("good".to_string()),
                marker_span: None,
            },
            LocalFile {
                path: sealed.clone(),
                caption: Some("sealed".to_string()),
                marker_span: None,
            },
        ];

        let (readable, failures) = probe_document_bytes(&files);

        assert_eq!(
            readable.keys().collect::<Vec<_>>(),
            vec![&good],
            "exactly the readable file comes back with bytes"
        );
        assert_eq!(failures.len(), 1, "one failure, naming the sealed file");
        assert_eq!(failures[0].resolved, Some(sealed.clone()));
        assert_eq!(
            failures[0].reason,
            crate::utils::image::LocalImageFailureReason::Unreadable
        );
    }
}
