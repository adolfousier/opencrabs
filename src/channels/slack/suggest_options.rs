//! Slack-side rendering for the OPTIONAL `suggest_options` tool (#599).
//!
//! Non-blocking option surface: the agent surfaces
//! `ProgressEvent::SuggestedOptions`, and we post a Block Kit ActionsBlock
//! with one button per suggestion under the finished response. Tapping a button
//! injects that suggestion as the user's next message (a fresh turn). Typing
//! your own message always works; there is no oneshot or timeout.

use std::sync::Arc;

use super::governor::GatedWrites;
use slack_morphism::prelude::*;
use uuid::Uuid;

/// Action-id prefix for a tapped follow-up suggestion: `followup:<session>:<idx>`.
pub(crate) const FOLLOWUP_PREFIX: &str = "followup:";

/// Post the follow-up suggestion buttons under the response and stash the option
/// list on state so the interaction handler resolves `idx -> text`. No-op on
/// empty options or a disconnected bot.
pub(crate) async fn render_suggestions(
    state: &Arc<super::SlackState>,
    session_id: Uuid,
    options: Vec<String>,
) {
    if options.is_empty() {
        return;
    }
    let client = match state.client().await {
        Some(c) => c,
        None => return,
    };
    let bot_token = match state.bot_token().await {
        Some(t) => t,
        None => return,
    };
    // Session→owner fallback via shared helper (#764 R5, silent twin).
    let Some(channel_id) = crate::channels::question_common::resolve_channel_or_silent(
        state.session_channel(session_id),
        state.owner_channel_id(),
    )
    .await
    else {
        return;
    };

    // Slack ActionsBlock allows up to 25 elements — well above the 4-suggestion
    // cap. Button text tops out at 75 chars. The absolute index rides in the
    // action_id; the text is stashed on state, not in the id.
    let buttons: Vec<SlackActionBlockElement> = options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            SlackActionBlockElement::Button(SlackBlockButtonElement::new(
                SlackActionId::new(format!("{FOLLOWUP_PREFIX}{session_id}:{idx}")),
                SlackBlockPlainTextOnly::from(SlackBlockPlainText::new(
                    crate::channels::question_common::truncate_label(
                        opt,
                        crate::channels::question_common::SLACK_LABEL_BUDGET,
                    )
                    .to_string(),
                )),
            ))
        })
        .collect();

    let header = SlackBlock::Section(SlackSectionBlock::new().with_text(SlackBlockText::MarkDown(
        SlackBlockMarkDownText::new("\u{1f4a1} *Suggested next:*".to_string()),
    )));
    let actions = SlackBlock::Actions(SlackActionsBlock::new(buttons));
    let content = SlackMessageContent::new()
        .with_text("Suggested next".to_string())
        .with_blocks(vec![header, actions]);

    state.set_pending_followups(session_id, options).await;

    let request = SlackApiChatPostMessageRequest::new(SlackChannelId::new(channel_id), content);
    let token = SlackApiToken::new(SlackApiTokenValue::from(bot_token));
    let session = client.open_session(&token);
    if let Err(e) = session.post(&request).await {
        tracing::warn!("Slack suggest_options: send failed: {e}");
        state.clear_pending_followups(session_id).await;
    }
}
