//! Tests for the TUI byte-capture ring and writer (#1719).
//!
//! The ring/writer is the black box around the redraw path: when a render
//! panic gets caught, the captured tail distinguishes escape-junk corruption
//! from genuine render panics. These tests pin the ring ordering, the
//! pass-through behavior, and the log-safe byte escaping.

use crate::tui::capture::{CaptureRing, CaptureWriter, escape_bytes, snapshot};
use std::io::Write;

#[test]
fn ring_wraps_and_orders_oldest_first() {
    let mut ring = CaptureRing::with_capacity(8);
    ring.push(b"abcdefgh");
    ring.push(b"XY");
    assert_eq!(ring.snapshot(), b"cdefghXY");
}

#[test]
fn ring_below_capacity_is_plain_append() {
    let mut ring = CaptureRing::with_capacity(16);
    ring.push(b"hello");
    ring.push(b" world");
    assert_eq!(ring.snapshot(), b"hello world");
}

#[test]
fn ring_exact_fill_then_single_byte_overwrites_oldest() {
    let mut ring = CaptureRing::with_capacity(4);
    ring.push(b"abcd");
    assert_eq!(ring.snapshot(), b"abcd");
    ring.push(b"!");
    assert_eq!(ring.snapshot(), b"bcd!");
}

#[test]
fn writer_passes_bytes_through_and_records_them() {
    let mut inner: Vec<u8> = Vec::new();
    {
        let mut w = CaptureWriter::new(&mut inner);
        w.write_all(b"render bytes").unwrap();
        w.flush().unwrap();
    }
    assert_eq!(inner, b"render bytes");
    // The global ring saw the same bytes (test binaries start empty, so the
    // tail of the snapshot is exactly what this writer just pushed).
    let snap = snapshot();
    assert!(snap.ends_with(b"render bytes"));
}

#[test]
fn escape_bytes_keeps_printables_and_marks_controls() {
    assert_eq!(escape_bytes(b"plain"), "plain");
    assert_eq!(escape_bytes(b"a\x1b[31m"), "a\\x1b[31m");
    assert_eq!(escape_bytes(b"line\n"), "line\\n");
    assert_eq!(escape_bytes(&[0x00, 0x07]), "\\x00\\x07");
}
