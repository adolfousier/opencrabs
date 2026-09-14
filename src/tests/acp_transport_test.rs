//! Capped inbound framing for the ACP transport (#1540 review).
//!
//! Lives under `src/tests/` per the contribution rule that tests never sit
//! inline in the crate.

use crate::acp::transport::{LineRead, read_capped_line};
use std::io::Cursor;
use tokio::io::BufReader;

fn reader(bytes: &[u8]) -> BufReader<Cursor<Vec<u8>>> {
    BufReader::new(Cursor::new(bytes.to_vec()))
}

#[tokio::test]
async fn complete_line_round_trip() {
    let mut r = reader(b"hello\nworld\n");
    match read_capped_line(&mut r, 64).await {
        LineRead::Line(bytes) => assert_eq!(bytes, b"hello"),
        other => panic!("expected a line, got {other:?}"),
    }
    match read_capped_line(&mut r, 64).await {
        LineRead::Line(bytes) => assert_eq!(bytes, b"world"),
        other => panic!("expected a line, got {other:?}"),
    }
    assert!(matches!(read_capped_line(&mut r, 64).await, LineRead::Eof));
}

#[tokio::test]
async fn oversized_frame_resyncs_instead_of_killing_framing() {
    // The runaway line is 12 bytes against a 8-byte cap; the second frame
    // after the newline must still be delivered whole.
    let mut r = reader(b"aaaaaaaaaaaaaa\nok\n");
    assert!(matches!(
        read_capped_line(&mut r, 8).await,
        LineRead::Oversized
    ));
    match read_capped_line(&mut r, 8).await {
        LineRead::Line(bytes) => assert_eq!(bytes, b"ok"),
        other => panic!("stream must stay framed after an oversized line, got {other:?}"),
    }
}

#[tokio::test]
async fn oversized_line_without_newline_hits_eof_still_overflowing() {
    // Runaway bytes, then the stream dies without ever writing a newline:
    // must report Eof, not hand out a truncated buffer as if it were a frame.
    let mut r = reader(b"aaaaaaaaaaaaaaaaa");
    match read_capped_line(&mut r, 8).await {
        LineRead::Oversized => {
            // Cap is breached as soon as growth exceeds it; remaining bytes
            // drain to EOF. Second read then sees the closed stream.
            assert!(matches!(read_capped_line(&mut r, 8).await, LineRead::Eof));
        }
        other => panic!("expected Oversized or Eof for a truncated runaway line, got {other:?}"),
    }
}

#[tokio::test]
async fn unterminated_tail_is_delivered_as_line() {
    // lines()/next_line() delivered a trailing fragment without newline;
    // the capped reader must match that behavior (prompt frame flushes on
    // client close).
    let mut r = reader(b"tail-without-newline-but-short");
    match read_capped_line(&mut r, 64).await {
        LineRead::Line(bytes) => assert_eq!(bytes, b"tail-without-newline-but-short"),
        other => panic!("expected the tail as a line, got {other:?}"),
    }
    assert!(matches!(read_capped_line(&mut r, 64).await, LineRead::Eof));
}

#[tokio::test]
async fn blank_lines_pass_through_as_empty() {
    // The reader loop treats empty frames as keep-alives; the primitive
    // just has to hand them over rather than swallow them.
    let mut r = reader(b"\nreal\n");
    match read_capped_line(&mut r, 64).await {
        LineRead::Line(bytes) => assert!(bytes.is_empty()),
        other => panic!("expected an empty line, got {other:?}"),
    }
    match read_capped_line(&mut r, 64).await {
        LineRead::Line(bytes) => assert_eq!(bytes, b"real"),
        other => panic!("expected the next line, got {other:?}"),
    }
}
