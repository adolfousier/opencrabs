//! #1784: a queued follow-up that lands on a text-only round must fold into the
//! running turn and produce ONE final reply.
//!
//! The bug (`tool_loop.rs`, the `if tool_uses.is_empty()` branch that finds a
//! queued message): the round's finished draft was emitted as a deliverable
//! `ProgressEvent::IntermediateText` before the queued message was injected. On
//! a text-only round that text is not narration, it is the complete answer to
//! the PREVIOUS request, and channel progress handlers relay it as a regular
//! message. The next round then answered the queued request, so the user got
//! two replies for one burst.
//!
//! These are behavioural tests: they drive the real tool loop with a mock
//! provider and a real queue callback, and assert on the events the loop
//! actually emitted. The daemon log in the issue (Telegram group, claude-cli)
//! is the occurrence evidence; the test is the regression gate.

use crate::brain::agent::service::{
    AgentService, MessageQueueCallback, ProgressCallback, ProgressEvent, QueuedUserMessage,
};
use crate::brain::provider::{
    ContentBlock, LLMRequest, LLMResponse, Provider, ProviderStream, Role, StopReason, TokenUsage,
};
use crate::brain::tools::ToolRegistry;
use crate::db::Database;
use crate::services::{ServiceContext, SessionService};
use crate::tests::agent_service_mocks::{MockProviderWithTools, MockTool};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// What the model drafts on round 1, i.e. the answer to the FIRST request.
///
/// Every reply body here ends with a full stop on purpose. A close of 40+ chars
/// with no terminal punctuation reads as a mid-sentence cut to
/// `looks_truncated_mid_sentence` (`src/brain/agent/service/phantom.rs:1079`),
/// and `try_emit_truncation_continue` (`truncation.rs:125`) then emits that
/// partial as an `IntermediateText` plus a continuation nudge and re-enters the
/// loop for `MAX_CONTINUATION_ATTEMPTS = 2` extra rounds. An unpunctuated terse
/// fixture therefore fabricates rounds and deliverable bodies the product never
/// produces, and the counts in these tests stop meaning anything.
const DRAFT: &str = "SUPERSEDED-DRAFT-1784 is the recipe for the first request.";
/// What the model produces on round 2, i.e. the answer to BOTH requests.
const FINAL: &str = "SINGLE-FINAL-1784 covers both requests in one reply.";
const QUEUE_TEXT: &str = "and also the smoothie one.";
/// Returned by any provider round after the folded final. The loop should never
/// reach it on a text-only fold: if it does, the fold caused an extra round
/// instead of ending with one reply, and that round's body is visible.
const THIRD: &str = "THIRD-ROUND-1784 means the loop kept going after the fold.";

fn text_response(text: &str) -> LLMResponse {
    LLMResponse {
        id: "fold-mock-response".to_string(),
        model: "mock-model".to_string(),
        content: vec![ContentBlock::Text {
            text: text.to_string(),
        }],
        stop_reason: Some(StopReason::EndTurn),
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 20,
            ..Default::default()
        },
        streaming_active_secs: None,
        tool_text_leak: false,
    }
}

/// Text-only provider: first call drafts an answer, second call produces the
/// folded final. Records every request it was given so a test can read the
/// context the model actually saw on the round after the fold.
struct FoldProvider {
    requests: Mutex<Vec<Vec<(String, String)>>>,
}

impl FoldProvider {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> Vec<Vec<(String, String)>> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for FoldProvider {
    async fn complete(&self, request: LLMRequest) -> crate::brain::provider::Result<LLMResponse> {
        let seen: Vec<(String, String)> = request
            .messages
            .iter()
            .map(|m| {
                let text = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (format!("{:?}", m.role), text)
            })
            .collect();
        let round = {
            let mut guard = self.requests.lock().unwrap();
            guard.push(seen);
            guard.len()
        };
        // Round 3+ is a distinct body on purpose: if the fold ever makes the
        // loop keep iterating after the folded final, that extra round becomes
        // a third reply a test can actually see instead of a silent duplicate.
        let reply = match round {
            1 => DRAFT,
            2 => FINAL,
            _ => THIRD,
        };
        Ok(text_response(reply))
    }

    async fn stream(&self, request: LLMRequest) -> crate::brain::provider::Result<ProviderStream> {
        use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

        let response = self.complete(request).await?;
        let mut events = vec![Ok(StreamEvent::MessageStart {
            message: StreamMessage {
                id: response.id.clone(),
                model: response.model.clone(),
                role: Role::Assistant,
                usage: response.usage,
            },
        })];
        for (i, block) in response.content.iter().enumerate() {
            if let ContentBlock::Text { text } = block {
                events.push(Ok(StreamEvent::ContentBlockStart {
                    index: i,
                    content_block: ContentBlock::Text {
                        text: String::new(),
                    },
                }));
                events.push(Ok(StreamEvent::ContentBlockDelta {
                    index: i,
                    delta: ContentDelta::TextDelta { text: text.clone() },
                }));
                events.push(Ok(StreamEvent::ContentBlockStop { index: i }));
            }
        }
        events.push(Ok(StreamEvent::MessageDelta {
            delta: MessageDelta {
                stop_reason: response.stop_reason,
                stop_sequence: None,
            },
            usage: response.usage,
        }));
        events.push(Ok(StreamEvent::MessageStop));
        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &str {
        "fold-mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(4096)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.0
    }
}

async fn fresh_session() -> (ServiceContext, Uuid) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let session = SessionService::new(context.clone())
        .create_session(Some("1784 fold test".to_string()))
        .await
        .unwrap();
    (context, session.id)
}

fn collector() -> (ProgressCallback, Arc<Mutex<Vec<ProgressEvent>>>) {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let s = sink.clone();
    let cb: ProgressCallback = Arc::new(move |_sid, ev| {
        s.lock().unwrap().push(ev);
    });
    (cb, sink)
}

fn one_message_queue(session_id: Uuid) -> MessageQueueCallback {
    let queues = Arc::new(tokio::sync::Mutex::new(HashMap::from([(
        session_id,
        QUEUE_TEXT.to_string(),
    )])));
    Arc::new(move |sid: Uuid| {
        let q = queues.clone();
        Box::pin(async move { q.lock().await.remove(&sid).map(QueuedUserMessage::plain) })
    })
}

fn delivered_text(events: &[ProgressEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::IntermediateText { text, .. } if !text.is_empty() => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Compact event trace for failure messages: variant plus a short text hint, so
/// a red run shows what the loop did without dumping whole transcripts.
fn event_summary(events: &[ProgressEvent]) -> String {
    let parts: Vec<String> = events
        .iter()
        .map(|e| match e {
            ProgressEvent::IntermediateText { text, .. } => {
                let head: String = text.chars().take(24).collect();
                format!("IntermediateText({head:?})")
            }
            ProgressEvent::QueuedUserMessage { text } => {
                let head: String = text.chars().take(24).collect();
                format!("QueuedUserMessage({head:?})")
            }
            ProgressEvent::StreamingChunk { text } => format!("StreamingChunk(len={})", text.len()),
            other => {
                let mut s = format!("{other:?}");
                s.truncate(24);
                s
            }
        })
        .collect();
    parts.join(" ")
}

/// Issue test 1: text-only round + queued message -> the superseded draft is
/// never emitted as a deliverable intermediate, and the turn delivers one
/// final reply.
#[tokio::test]
async fn a_text_only_round_with_a_queued_message_does_not_deliver_the_draft() {
    let (context, session_id) = fresh_session().await;
    let provider = Arc::new(FoldProvider::new());
    let (progress, sink) = collector();

    let agent = AgentService::new_for_test(provider.clone(), context)
        .await
        .with_message_queue_callback(Some(one_message_queue(session_id)));

    let response = agent
        .send_message_with_tools_and_callback(
            session_id,
            "give me the pasta recipe".to_string(),
            None,
            None,
            None,
            Some(progress),
            "test",
            None,
            None,
        )
        .await
        .unwrap();

    let events = sink.lock().unwrap().clone();

    assert!(
        !delivered_text(&events)
            .iter()
            .any(|t| t.contains("SUPERSEDED-DRAFT")),
        "the superseded draft was relayed as its own message: {events:?}"
    );
    assert!(
        response.content.contains("SINGLE-FINAL-1784"),
        "the turn must end with the folded final, got {:?}",
        response.content
    );
    // One reply, not two. Nothing of the superseded round may reach a channel as
    // an intermediate body, and no extra round may show up either (the provider
    // hands a different body from round 3 on, so an extra iteration is visible
    // here instead of hiding behind identical text).
    let bodies = delivered_text(&events);
    assert!(
        !bodies.iter().any(|t| t.contains("SUPERSEDED-DRAFT")),
        "the superseded draft was relayed as its own message: [{}]",
        event_summary(&events)
    );
    assert!(
        bodies.iter().all(|t| !t.contains("THIRD-ROUND")),
        "the fold made the loop keep iterating: [{}]",
        event_summary(&events)
    );
    assert_eq!(
        provider.snapshot().len(),
        2,
        "a text-only fold must cost exactly two provider rounds (draft, then the folded final), got {}",
        provider.snapshot().len()
    );
    // The queued message is still announced to the UI exactly once, and nothing
    // was dropped: the fold happened, the model saw the request.
    let announced = events
        .iter()
        .filter(|e| matches!(e, ProgressEvent::QueuedUserMessage { .. }))
        .count();
    assert_eq!(announced, 1, "queued message announced {announced} times");
}

/// Issue test 3 (first half): the draft the user never saw must still be in the
/// model's context, and the injected turn must say out loud that it was not
/// delivered, so the final is complete on its own instead of a delta.
#[tokio::test]
async fn the_injected_turn_says_the_previous_draft_was_never_delivered() {
    let (context, session_id) = fresh_session().await;
    let provider = Arc::new(FoldProvider::new());

    let agent = AgentService::new_for_test(provider.clone(), context)
        .await
        .with_message_queue_callback(Some(one_message_queue(session_id)));

    agent
        .send_message_with_tools_and_callback(
            session_id,
            "give me the pasta recipe".to_string(),
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
        )
        .await
        .unwrap();

    let requests = provider.snapshot();
    assert!(
        requests.len() >= 2,
        "expected a second model call after the fold, got {}",
        requests.len()
    );
    let second = &requests[1];

    let draft_kept = second
        .iter()
        .any(|(role, text)| role == "Assistant" && text.contains("SUPERSEDED-DRAFT"));
    assert!(
        draft_kept,
        "the draft must stay in context so the model can fold it, got {second:?}"
    );

    let noted = second.iter().any(|(role, text)| {
        role == "User" && text.contains(QUEUE_TEXT) && text.contains("never delivered")
    });
    assert!(
        noted,
        "the injected user turn must carry the queued request AND the \
         not-delivered note, got {second:?}"
    );
}

/// Issue test 2: tool-using rounds keep today's behaviour (#475). Their
/// narration is intermediate by design and must still be emitted.
#[tokio::test]
async fn a_tool_round_with_a_queued_message_still_narrates() {
    let (context, session_id) = fresh_session().await;
    let provider = Arc::new(MockProviderWithTools::new());
    let registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));
    let (progress, sink) = collector();

    let agent = AgentService::new_for_test(provider, context)
        .await
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true)
        .with_message_queue_callback(Some(one_message_queue(session_id)));

    agent
        .send_message_with_tools_and_callback(
            session_id,
            "use the test tool".to_string(),
            None,
            None,
            None,
            Some(progress),
            "test",
            None,
            None,
        )
        .await
        .unwrap();

    let events = sink.lock().unwrap().clone();
    assert!(
        delivered_text(&events)
            .iter()
            .any(|t| t.contains("I'll use the test tool.")),
        "pre-tool narration must stay deliverable on a tool round: {events:?}"
    );
}

/// The note is what stops the final reply from being a delta. It is prepended,
/// the user's own words stay verbatim, and a round that wrote nothing gets no
/// claim about a withheld draft.
#[test]
fn the_fold_note_wraps_the_queued_text_without_touching_it() {
    let folded = crate::brain::agent::service::queued_fold::injected_context(
        true,
        "and also the smoothie one",
    );
    assert!(
        folded.contains("never delivered"),
        "the model must be told the draft was not sent: {folded}"
    );
    assert!(
        folded.ends_with("and also the smoothie one"),
        "the user's words must survive verbatim at the end: {folded}"
    );

    let plain =
        crate::brain::agent::service::queued_fold::injected_context(false, "and also the one");
    assert_eq!(
        plain, "and also the one",
        "a round that drafted nothing hid nothing; no note may be claimed"
    );
}

/// Behavioural pin on the site itself: the fold no longer hands a channel a
/// deliverable draft, and the #616 Kimi reasoning reroute (empty `text`, never
/// relayed as a message) still runs.
#[test]
fn the_fold_site_emits_no_deliverable_draft() {
    let src = include_str!("../brain/agent/service/tool_loop.rs");
    let marker = "Injecting queued user message (from_buf={})";
    let start = src.find(marker).expect("the text-only fold site exists");
    let rest = &src[start..];
    let end = rest
        .find("continue;")
        .expect("the site ends at its continue");
    let region = &rest[..end];

    assert!(
        !region.contains("text: iteration_text,"),
        "the deliverable emission of the superseded draft is back (#1784)"
    );
    assert!(
        region.contains("text: String::new()"),
        "the #616 reasoning reroute must stay, reasoning is not a chat message"
    );
    assert!(
        region.contains("queued_fold::injected_context("),
        "the injected turn must be built by queued_fold so the note is testable"
    );
}

/// Tool-using rounds keep today's behaviour (issue fix plan item 3): the site
/// that folds a queue message between tool iterations must be untouched, and it
/// has no draft to withhold because the round ended with tool calls.
#[test]
fn the_tool_iteration_fold_site_is_unchanged() {
    let src = include_str!("../brain/agent/service/tool_loop.rs");
    let marker = "Injecting queued user message between tool iterations";
    let start = src
        .find(marker)
        .expect("the tool-iteration fold site exists");
    let rest = &src[start..];
    let end = rest
        .find("assistant_db_msg = message_service")
        .expect("the site ends when the new assistant placeholder is made");
    let region = &rest[..end];

    assert!(
        !region.contains("IntermediateText"),
        "tool-iteration folding must not start emitting or suppressing narration here"
    );
    assert!(
        region.contains("queued_msg.context_text.clone()"),
        "the tool path injects the queued text as-is: no draft was superseded (#1784)"
    );
}

/// The callback the loop sees when a session has nothing queued. It exists so
/// the fold can be measured against a plain turn instead of against a number I
/// picked: the claim in #1784 is "one reply per burst", and the only honest
/// yardstick for how many deliverable bodies one reply is, is a turn that never
/// had a second request at all.
fn empty_queue() -> MessageQueueCallback {
    Arc::new(|_sid: Uuid| Box::pin(async { Option::<QueuedUserMessage>::None }))
}

/// Control + fold in one test, so "one reply per burst" is a measurement instead
/// of a number I picked. Run the identical turn with nothing queued, then with
/// one message queued mid-turn, and require the folded turn to hand a channel
/// exactly as many deliverable bodies as the plain turn did, at the cost of
/// exactly one more provider round. A suppression-only "fix" shows up as fewer
/// bodies than the baseline; the #1784 double reply shows up as more, carrying
/// the draft's own text.
#[tokio::test]
async fn a_fold_delivers_no_more_reply_bodies_than_a_plain_turn() {
    let (context, baseline_session) = fresh_session().await;
    let baseline_provider = Arc::new(FoldProvider::new());
    let (baseline_progress, baseline_sink) = collector();
    let baseline_agent = AgentService::new_for_test(baseline_provider.clone(), context)
        .await
        .with_message_queue_callback(Some(empty_queue()));
    let baseline_response = baseline_agent
        .send_message_with_tools_and_callback(
            baseline_session,
            "give me the pasta recipe".to_string(),
            None,
            None,
            None,
            Some(baseline_progress),
            "test",
            None,
            None,
        )
        .await
        .unwrap();
    let baseline_events = baseline_sink.lock().unwrap().clone();
    let baseline_bodies = delivered_text(&baseline_events);

    let (folded_context, folded_session) = fresh_session().await;
    let folded_provider = Arc::new(FoldProvider::new());
    let (folded_progress, folded_sink) = collector();
    let folded_agent = AgentService::new_for_test(folded_provider.clone(), folded_context)
        .await
        .with_message_queue_callback(Some(one_message_queue(folded_session)));
    let folded_response = folded_agent
        .send_message_with_tools_and_callback(
            folded_session,
            "give me the pasta recipe".to_string(),
            None,
            None,
            None,
            Some(folded_progress),
            "test",
            None,
            None,
        )
        .await
        .unwrap();
    let folded_events = folded_sink.lock().unwrap().clone();
    let folded_bodies = delivered_text(&folded_events);

    assert_eq!(
        baseline_provider.snapshot().len(),
        1,
        "the control turn itself changed shape: {} provider round(s), trace [{}]",
        baseline_provider.snapshot().len(),
        event_summary(&baseline_events)
    );
    assert_eq!(
        folded_provider.snapshot().len(),
        2,
        "one queued message must cost exactly one extra provider round, got {}",
        folded_provider.snapshot().len()
    );
    assert_eq!(
        folded_bodies.len(),
        baseline_bodies.len(),
        "a queued follow-up changed how many bodies a channel can post (baseline {}, folded {}): [{}]",
        baseline_bodies.len(),
        folded_bodies.len(),
        event_summary(&folded_events)
    );
    assert!(
        !folded_bodies.iter().any(|t| t.contains("SUPERSEDED-DRAFT")),
        "the superseded draft reached a channel as its own reply: [{}]",
        event_summary(&folded_events)
    );
    assert!(
        baseline_response.content.contains(DRAFT) && folded_response.content.contains(FINAL),
        "baseline reply {:?}, folded reply {:?}",
        baseline_response.content,
        folded_response.content
    );
}
