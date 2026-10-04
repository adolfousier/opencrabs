//! Send-correlation telemetry for the Telegram surface (#1085).

use crate::channels::telegram::telemetry::*;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn hash8_is_stable_and_8_hex_chars() {
    let a = content_hash8("hello world");
    let b = content_hash8("hello world");
    assert_eq!(a, b);
    assert_eq!(a.len(), 8);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn hash8_separates_different_content() {
    assert_ne!(content_hash8("hello world"), content_hash8("hello worlD"));
}

#[test]
fn hash8_handles_empty_and_multibyte() {
    assert_eq!(content_hash8("").len(), 8);
    // PT-PT and emoji content must not panic (multibyte boundary safety).
    assert_eq!(content_hash8("ção 🦀 açúcar").len(), 8);
}

// ---------------------------------------------------------------------------
// #721 (E1): outbound-REQUEST telemetry — typing / delete / reaction.
//
// These lines answer a different question from the landing lines: not "what
// reached the chat" but "what did we put ON THE WIRE". A request that fails is
// still a request, so the line is emitted before the API call and its presence
// says nothing about the outcome. The field order is the contract — a reader
// parses by position — so it is pinned exactly.
// ---------------------------------------------------------------------------

#[test]
fn request_line_pins_the_field_order_for_a_targeted_request() {
    let line = request_line(
        "turn",
        "typing loop",
        "-",
        "delete",
        "deleteMessage",
        -1001234567890,
        Some(7),
        Some(42),
    );
    assert_eq!(
        line,
        "Telegram request: origin=turn detail=typing loop session=- kind=delete path=deleteMessage chat=-1001234567890 thread=Some(7) msg=42"
    );
}

/// A chat action targets no message. `msg` is then the create-shaped dash
/// (the convention #676 established for rich creates), NOT an absent field:
/// every line carries every field, so a reader never has to guess a shape.
#[test]
fn request_line_uses_the_create_shape_when_a_request_targets_no_message() {
    let line = request_line(
        "turn",
        "typing tick",
        "-",
        "typing",
        "sendChatAction",
        -1001234567890,
        None,
        None,
    );
    assert!(
        line.ends_with("chat=-1001234567890 thread=None msg=-"),
        "a chat action targets no message, so msg must be the create-shaped dash; got: {line}"
    );
}

/// `session` is the originating session id where the site knows it and `-`
/// where it genuinely cannot — the existing `log_send_success` convention,
/// unchanged.
#[test]
fn request_line_keeps_the_dash_session_fallback() {
    let line = request_line(
        "system",
        "probe_topic",
        "-",
        "typing",
        "sendChatAction",
        555,
        Some(3),
        None,
    );
    assert!(line.contains("session=- kind=typing"), "got: {line}");
}

/// A request has no body, so the two body-describing fields of the send
/// schema (`len`, `hash8`) are DROPPED rather than carried as dead dashes.
#[test]
fn request_line_carries_no_message_body_fields() {
    let line = request_line(
        "tool",
        "delete",
        "-",
        "delete",
        "deleteMessage",
        1,
        None,
        Some(2),
    );
    assert!(!line.contains("len="), "a request has no body; got: {line}");
    assert!(
        !line.contains("hash8="),
        "a request has no body; got: {line}"
    );
}

/// The prefix is what makes a request greppable WITHOUT matching a landing,
/// and vice versa: `oc-log-search 'Telegram request:'` must never return a
/// send that landed.
#[test]
fn request_line_prefix_cannot_be_confused_with_a_landing_line() {
    let line = request_line(
        "turn",
        "typing loop",
        "-",
        "typing",
        "sendChatAction",
        1,
        None,
        None,
    );
    assert!(line.starts_with("Telegram request: "), "got: {line}");
    assert!(!line.contains("Telegram send ok:"), "got: {line}");
    assert!(!line.contains("Telegram send failed:"), "got: {line}");
}


// ---------------------------------------------------------------------------
// Emitted half: the line must actually reach the log at INFO.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct EventCapture {
    events: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

impl EventCapture {
    fn events(&self) -> Vec<(String, String)> {
        self.events.lock().unwrap().clone()
    }
}

impl<S: tracing::Subscriber> Layer<S> for EventCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push((
            event.metadata().level().to_string(),
            visitor.message.unwrap_or_default(),
        ));
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

/// INFO, not DEBUG — and the reason is the READER, not the file: `oc-log-search`
/// stages `grep -E ' (INFO|WARN|ERROR) '` by default, so a `debug!` request line
/// would be invisible to the ordinary read and E1 would swap one blind spot for
/// another (owner challenge 2026-09-30, re-derived at source).
#[test]
fn log_request_emits_exactly_one_info_line_under_the_request_prefix() {
    let capture = EventCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    tracing::subscriber::with_default(subscriber, || {
        // Another test in this binary installs a process-wide subscriber capped
        // at WARN (`governor_gates_test.rs::ensure_tracing_capture`), which
        // would disable an INFO callsite if that dispatcher were the only one
        // consulted. It is not: building this thread's dispatch runs
        // `Dispatch::new` -> `callsite::register_dispatch`, which pushes THIS
        // subscriber into the dispatcher list and rebuilds the interest cache
        // over all of them. Two dispatchers that disagree combine to
        // `Interest::sometimes` (tracing-core `Interest::and`: differing
        // interests -> sometimes), never `never`, so the event is offered to
        // this thread's subscriber and captured. No manual interest rebuild is
        // needed -- and none is reachable from `tracing` 0.1 anyway.
        log_request(
            "turn",
            "typing loop",
            "-",
            "typing",
            "sendChatAction",
            -1001234567890,
            None,
            None,
        );
    });

    let events = capture.events();
    assert_eq!(
        events.len(),
        1,
        "exactly one request line must be emitted; got {events:?}"
    );
    assert_eq!(
        events[0].0, "INFO",
        "the request line must be emitted at INFO"
    );
    assert!(
        events[0]
            .1
            .contains("Telegram request: origin=turn detail=typing loop"),
        "the emitted line must carry the request prefix and its fields; got: {}",
        events[0].1
    );
}
