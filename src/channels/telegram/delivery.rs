//! Final-response delivery: the ONE path every Telegram turn ends on
//! (#471 phases 2-4). Live turns (handle_message) and crash-recovery
//! resumes both call deliver_final_response; the post-loop display drain
//! lives here with it.

use super::TelegramState;
use super::flow::{
    DisplayItem, StreamingState, append_intermediate_to_flow, append_system_to_flow,
    append_tool_group, folded_duplicates_final, last_folded_text, settle_options_reclaim,
    take_folded_final,
};
use super::handler::{fire_reaction, map_to_allowed_reaction};
use super::intermediates::send_html_or_plain;
use super::markdown::{markdown_to_telegram_html, split_message};
use super::send::{best_effort_delete, message_in_thread, photo_in_thread};
use crate::brain::agent::AgentService;
use crate::db::ChannelMessageRepository;
use crate::db::models::ChannelMessage as DbChannelMessage;
use crate::utils::sanitize::redact_secrets;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InputFile, MessageId, ParseMode};
use uuid::Uuid;

/// Whether a turn carrying a `<<react:…>>` directive is react-ONLY, given the
/// text that remains once the directive is stripped.
///
/// The prompt teaches the model two shapes (see `handler.rs`, "Reaction
/// directive"): output ONLY the directive to react-only, or put the directive
/// at the START and answer after it to react AND respond. Emptiness of the
/// remaining text is therefore the whole question, and the answer is on the
/// wire rather than inferred.
///
/// Deliberately takes no other input. #928 also consulted whether the turn ran
/// tools and suppressed the text when it had not, which destroyed completions
/// that simply needed no tools (#1009). Tool state does not tell you what the
/// content channel holds, so it is not a parameter here.
pub(crate) fn is_react_only(text_after_directive: &str) -> bool {
    text_after_directive.trim().is_empty()
}

/// Whether the turn ends on a suggest_options surface (#1226 K): the
/// progress handler stashes the options mid-turn (progress.rs), so by
/// delivery time a Some here means an option surface is armed and the
/// flow block ends on the suggest_options Tool entry — the reclaim calls
/// below must then look BEFORE that trailing tool for the answer.
fn options_pending(streaming: &Arc<std::sync::Mutex<StreamingState>>) -> bool {
    streaming
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pending_suggestions
        .is_some()
}

/// True when a rich-send failure means Telegram could not fetch the embedded
/// media (the mermaid.ink diagram) — a transient renderer or network window
/// that a single re-send can sail (#tg-mermaid-delivery-hardening). Our
/// resolve runs seconds before Telegram's own server-side refetch, so
/// `RICH_MESSAGE_PHOTO_NO_MEDIA_FOUND` is a race with a flaky renderer, not a
/// structural rejection. Structural 400s (schema, content) are never retried.
pub(crate) fn is_no_media_found(e: &anyhow::Error) -> bool {
    format!("{e:#}").contains("RICH_MESSAGE_PHOTO_NO_MEDIA_FOUND")
}

/// Drain all pending intermediate texts from the streaming state's display
/// queue and send them immediately. Called by the follow-up-question callback
/// BEFORE posting the question message, so the user sees contextual text
/// above the buttons instead of below (race reported in issue #142).
///
/// Applies the same sanitize/redact/dedup/split/send chain as the edit loop.
/// Deliver the final agent response for a live inbound turn: marker
/// extraction, sanitize, react directive, dedup, reaction-only handling,
/// folded-final reclaim, footer, rich-first send with HTML fallback,
/// chunked sends, history recording, TTS. Extracted VERBATIM from
/// handle_message (#471 phase 2) — the only edit is early
/// `return Ok(())` becoming `return Ok(false)` so the caller can
/// preserve handle_message's original control flow exactly.
/// Background-task indicator for the settled flow footer (#1054): the first
/// task's label when exactly one is running, a count when several, `None`
/// when nothing is detached or no manager is wired (#722). A settled turn
/// that ends with detached work looks identical to a complete one without
/// this, and the typing indicator staying alive is too easy to miss.
///
/// Returns the footer label **and** the numeric count (the settled header
/// needs the number to read "Waiting for N background task(s)" — #1144).
/// Both come from the single `running_tasks(session_id)` read so settle does
/// not hit the manager twice.
pub(crate) fn bg_indicator_for(
    agent: &AgentService,
    session_id: Uuid,
) -> (Option<String>, Option<usize>) {
    let Some(bm) = agent.background_manager() else {
        return (None, None);
    };
    let tasks = bm.running_tasks(session_id);
    match tasks.len() {
        0 => (None, Some(0)),
        1 => (Some(format!("{} running", tasks[0].label)), Some(1)),
        n => (Some(format!("{n} tasks running")), Some(n)),
    }
}

/// Alive sub-agent counts for the settled header (#1183): how many of THIS
/// session's children are still working vs parked awaiting collection. The
/// sub-agent registry is separate from `BackgroundTaskManager`, so the #1144
/// header gate never saw it — a turn ending with agents mid-work still read
/// "✅ Finished". Empty when no manager is wired or every child already
/// terminated; the header then falls back to the background-task-only (or
/// plain Finished) form.
pub(crate) fn subagent_counts_for(
    agent: &AgentService,
    session_id: Uuid,
) -> super::flow::SubagentCounts {
    let Some(mgr) = agent.subagent_manager() else {
        return super::flow::SubagentCounts::default();
    };
    let (working, awaiting) = mgr.alive_counts_for(session_id);
    super::flow::SubagentCounts { working, awaiting }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn deliver_final_response(
    bot: &Bot,
    chat_id: ChatId,
    // The inbound message this turn answers: reaction target and reply
    // anchor. None on the crash-recovery resume path, where the original
    // message id is lost across restarts — reactions strip without firing.
    inbound: Option<&Message>,
    thread_id: Option<teloxide::types::ThreadId>,
    streaming: &Arc<std::sync::Mutex<StreamingState>>,
    session_id: Uuid,
    agent: &Arc<AgentService>,
    telegram_state: &Arc<TelegramState>,
    channel_msg_repo: &ChannelMessageRepository,
    voice_config: &crate::config::VoiceConfig,
    is_voice: bool,
    is_dm: bool,
    chat_title: &str,
    mut streaming_msg_id: Option<MessageId>,
    result: Result<crate::brain::agent::AgentResponse, crate::brain::agent::AgentError>,
) -> ResponseResult<bool> {
    match result {
        Ok(response) => {
            // Merge candidate captured below: whichever bubble carried the final
            // response (classic HTML edit/send, or table-free rich message).
            // render_suggestions attaches its keyboard to THIS bubble when Some.
            let mut final_bubble: Option<super::state::MergeBubble> = None;
            // Extract <<IMG:path>> markers — send each as a Telegram photo.
            let (text_only, img_paths) = crate::utils::extract_img_markers(&response.content);
            // Strip LLM-hallucinated artifacts (<!-- tools-v2 -->, XML tool blocks)
            let text_only = crate::utils::sanitize::strip_llm_artifacts(&text_only);
            let text_only = redact_secrets(&text_only);

            // Drop an echoed plan title (#837). The reminder shows the model
            // the title every turn and it opens by repeating it, directly
            // under the card that already renders it.
            //
            // Covers Editing as well as Active: #621 folded title and prose
            // into the card in BOTH states, so limiting this to Active left
            // the duplicate visible for every plan still being drafted.
            let text_only = match crate::utils::plan_files::load_plan(session_id).await {
                Some(plan) if !plan.title.trim().is_empty() => {
                    strip_echoed_plan_title(&text_only, &plan.title)
                }
                _ => text_only,
            };

            // Extract <<react:emoji>> directive — the LLM outputs this to
            // signal a reaction-only response (no text bubble). If the
            // response is ONLY a reaction, the emoji is sent as a Telegram
            // reaction on the user's message and text delivery is skipped.
            let (text_only, react_emoji) = crate::utils::extract_react_marker(&text_only);

            // Dedup: strip text that was already sent as intermediate messages
            // to avoid duplicating content on Telegram. An intermediate chunk
            // that already carries the final answer (e.g. "Done. Uploaded to
            // Drive: https://…") will otherwise be repeated when the
            // streaming placeholder is edited with the final response.
            // Intermediates stay visible as-is; only the streaming
            // placeholder's final text is pruned.
            let sent = {
                let s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                s.sent_intermediates.clone()
            };
            tracing::info!(
                "Telegram dedup: response.content len={}, sent_intermediates count={}",
                text_only.len(),
                sent.len(),
            );
            let pre_dedup_text = text_only.clone();
            // Normalize whitespace for comparison — collapse runs of
            // whitespace (including newlines) to single spaces so that
            // minor formatting differences between the streamed
            // intermediate and the final response don't bypass dedup.
            let norm = |s: &str| -> String { s.split_whitespace().collect::<Vec<_>>().join(" ") };
            let mut suppressed_final = false;
            let text_only = if !sent.is_empty() {
                let norm_final = norm(&text_only);
                if sent.iter().any(|i| norm(i) == norm_final) {
                    tracing::info!(
                        "Telegram dedup: match found among {} intermediates (normalized) — suppressing final response",
                        sent.len()
                    );
                    suppressed_final = true;
                    String::new()
                } else {
                    text_only
                }
            } else {
                text_only
            };

            // Reclaim a folded answer BEFORE the reaction decision (#478):
            // the completion can stream mid-turn as IntermediateText and
            // fold into the flow block. When the final text is empty, the
            // trailing folded run IS the answer — pulling it out here means
            // a closing react directive becomes a reaction ON TOP of the
            // delivered completion, instead of the reaction-only skip
            // imprisoning the answer inside the Processing log block.
            //
            // UNLESS the final was just suppressed by dedup (#1152): the
            // trailing folded run then belongs to the SAME answer, which
            // already went out as intermediate bubbles. Reclaiming it here
            // re-ships an orphan duplicate fragment (62-char tail under the
            // full answer). `folded_duplicates_final` cannot arbitrate this:
            // streaming may leave a SUFFIX chunk folded in the block, and
            // that predicate deliberately only matches prefix overlap. So
            // when suppression happened, strip the folded copy from the
            // block silently — it is already delivered.
            //
            // #31: tracks whether THIS block already reclaimed the answer, so
            // the main reclaim below can tell "nothing left to reclaim" from
            // "the reclaim found nothing that was ever there" in its K warn.
            let mut pre_reclaimed = false;
            let text_only = if text_only.trim().is_empty() {
                if suppressed_final {
                    let (discarded, discarded_trailer) =
                        take_folded_final(bot, chat_id, streaming, options_pending(streaming))
                            .await;
                    if discarded.is_some() || discarded_trailer.is_some() {
                        tracing::info!(
                            "Telegram: final suppressed by dedup — dropping {}(+{}) folded \
                             chars already delivered as intermediates (#1152)",
                            discarded.as_ref().map(|t| t.len()).unwrap_or(0),
                            discarded_trailer.as_ref().map(|t| t.len()).unwrap_or(0),
                        );
                    }
                    text_only
                } else {
                    let (reclaimed, trailer) =
                        take_folded_final(bot, chat_id, streaming, options_pending(streaming))
                            .await;
                    if let Some(t) = trailer {
                        let trailer_len = t.len();
                        // #31: the post-halt sign-off rides AFTER the buttons —
                        // stash it for render_suggestions (keep-never-discard).
                        streaming
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .pending_trailer = Some(t);
                        tracing::info!(
                            "Telegram: stashed {} trailer chars reclaimed before the react \
                             decision (#31)",
                            trailer_len
                        );
                    }
                    match reclaimed {
                        Some(reclaimed) => {
                            pre_reclaimed = true;
                            tracing::info!(
                                "Telegram: reclaimed folded final ({} chars) before react \
                                 decision (#478)",
                                reclaimed.len()
                            );
                            reclaimed
                        }
                        None => text_only,
                    }
                }
            } else {
                text_only
            };

            // Reaction directive: if the LLM included <<react:emoji>>, send
            // a reaction on the user's message. For reaction-only responses
            // (empty text after stripping the directive), skip all text/TTS
            // delivery and just react — but ONLY when the turn did no tool
            // work (#439): a turn that executed tools and ended with a bare
            // reaction dropped its whole completion (issues were closed and
            // commented, the user saw only 🔥). For a work turn, empty final
            // text is a failure mode, never a deliberate ack.
            let turn_ran_tools = {
                let s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                !s.tool_msgs.is_empty()
            };
            if let Some(ref emoji) = react_emoji {
                let mapped = map_to_allowed_reaction(emoji);
                let reaction = teloxide::types::ReactionType::Emoji {
                    emoji: mapped.clone(),
                };
                let react_result = match inbound {
                    Some(m) => bot
                        .set_message_reaction(chat_id, m.id)
                        .reaction(vec![reaction])
                        .is_big(false)
                        .await
                        .map(|_| ()),
                    // Resume path: the original message is gone, nothing to
                    // react to — treat as delivered so no fallback fires.
                    None => Ok(()),
                };
                if let Err(ref e) = react_result {
                    tracing::warn!("Telegram: failed to set reaction ({mapped}): {}", e);
                }
                // `!suppressed_final` (#1152): a final suppressed by dedup was
                // not dropped — it shipped early as intermediate bubbles. That
                // is delivery, not the failure mode #439 guards against, so no
                // synthetic "Done — X/Y tool calls" summary on top of it.
                if text_only.trim().is_empty() && turn_ran_tools && !suppressed_final {
                    // Work turn with no completion text (#439): the model
                    // replaced its summary with a reaction. Deliver a
                    // fallback completion so the work is reported — the
                    // reaction already landed above.
                    tracing::warn!(
                        "Telegram: turn executed tools but produced no completion text — \
                         delivering fallback summary instead of reaction-only skip (#439)"
                    );
                    let fallback = {
                        let s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                        let done = s
                            .tool_msgs
                            .iter()
                            .filter(|t| t.completed == Some(true))
                            .count();
                        format!(
                            "Done — {done}/{} tool calls completed. (The model ended the turn \
                             without a summary; see the log above for what ran.)",
                            s.tool_msgs.len()
                        )
                    };
                    if let Err(e) = message_in_thread(bot, chat_id, thread_id, &fallback).await {
                        tracing::error!("Telegram: fallback completion send failed: {}", e);
                    }
                    if let Some(mid) = streaming_msg_id {
                        best_effort_delete(bot, chat_id, mid, "fallback send cleanup").await;
                    }
                    return Ok(false);
                }
                // React-only means exactly what the prompt says it means: ONLY
                // the directive, nothing after it. The contract taught to the
                // model (handler.rs, "Reaction directive") states both shapes:
                //
                //   react-only        -> output ONLY the directive
                //   react AND respond -> directive at the START, then the text
                //
                // so text after the directive IS the documented second shape,
                // and delivering it is the contract being honoured rather than
                // violated.
                //
                // #928 suppressed that text whenever the turn ran no tools, on
                // the theory that a react turn has no answer in it and anything
                // present must be reasoning spilled into the content channel.
                // The tool state was standing in for "is this reasoning", and
                // it does not answer that question: a turn that answers from
                // analysis alone runs no tools, which is the ordinary shape of
                // an explanation or a follow-up. #1009 is four consecutive
                // completions destroyed that way, none of which contained any
                // reasoning at all.
                //
                // Reasoning that reaches the content channel is a provider-side
                // defect (#760) and belongs where it originates. Delivery must
                // not delete a completion on suspicion, because that failure is
                // silent and total while a leak is merely ugly.
                if is_react_only(&text_only) {
                    // Never-silent guard (#353): a reaction-only turn whose
                    // reaction FAILED must degrade to text, not to nothing.
                    if react_result.is_err() {
                        tracing::warn!(
                            "Telegram: reaction-only turn with failed reaction — \
                             delivering the emoji as text instead"
                        );
                        if let Err(e) =
                            message_in_thread(bot, chat_id, thread_id, emoji.as_str()).await
                        {
                            tracing::error!("Telegram: emoji text fallback also failed: {}", e);
                        }
                    } else {
                        // A genuine ack (praise, "got it") completes in seconds.
                        // A react-only turn that instead took MINUTES engaged
                        // with real work, ran no tools, and then bailed with
                        // only a reaction — a dropped request, not an ack (#546:
                        // "executing a task for 5m, it reacts and just stops").
                        // #439 only guards the tools-ran case; this catches the
                        // reasoned-but-no-tools case. Delivery can't re-run the
                        // model, so surface it as an incomplete turn instead of
                        // a silent drop, so the user knows to re-send.
                        let elapsed = {
                            let s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                            s.turn_started_at.elapsed()
                        };
                        // This branch is now reached only with an empty content
                        // channel, so the turn produced a reaction and nothing
                        // else. #928's carve-out for prose landing here is gone
                        // with the suppression it guarded (#1009): prose no
                        // longer routes into this branch at all, it is
                        // delivered.
                        if elapsed >= std::time::Duration::from_secs(60) {
                            tracing::warn!(
                                "Telegram: react-only after {}s with no tools and no text — \
                                 surfacing as an incomplete turn, not a silent drop (#546)",
                                elapsed.as_secs()
                            );
                            let notice = "⚠️ I ended that turn with only a reaction and did not \
                                          complete the request. Please re-send it if you needed \
                                          me to act.";
                            if let Err(e) = message_in_thread(bot, chat_id, thread_id, notice).await
                            {
                                tracing::error!(
                                    "Telegram: #546 incomplete-turn notice failed: {}",
                                    e
                                );
                            }
                        } else {
                            tracing::info!(
                                "Telegram: reaction-only response ({}), skipping text delivery",
                                mapped
                            );
                        }
                    }
                    // A react-only turn ran no tools (the tools-with-no-text
                    // case returned above, #439), so any open processing-log
                    // block is header-only — the model's thinking preview plus
                    // persistent plan chrome. Left up, it reads as a "Processing
                    // log" bubble with no answer, i.e. a dropped request (#544).
                    // Remove it and clear its state so ONLY the reaction remains;
                    // the plan state persists independently (/show-plan still
                    // works).
                    let flow_mid = {
                        let mut s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                        s.flow_entries.clear();
                        s.open_group_msg_id.take()
                    };
                    if let Some(mid) = flow_mid {
                        best_effort_delete(bot, chat_id, mid, "flow teardown").await;
                        // #1377: the card is gone — drop the fold handle so a
                        // later completion falls back to the bubble lane
                        // instead of editing a deleted message.
                        telegram_state.clear_flow_state(session_id).await;
                    }
                    if let Some(mid) = streaming_msg_id {
                        best_effort_delete(bot, chat_id, mid, "streaming teardown").await;
                    }
                    return Ok(false);
                }
            }

            // Context budget footer is display-only chrome: it rides the
            // settled flow message as a section, never the final answer, and
            // is NOT stored in the session/messages table or used for TTS.
            let ctx_max = agent.context_limit_for_session(session_id);
            let footer = crate::utils::format_ctx_footer(
                response.context_tokens,
                ctx_max,
                response.tokens_per_second,
            );
            // Quiet ❕ while the #909 pressure hint is active (#29): the
            // settled footer mirrors the nudge that's in the prompt. Cleared
            // on compaction success; re-arms below the 55% floor.
            let footer = if !footer.is_empty() && agent.pressure_warning_active(session_id) {
                format!("{footer} ❕")
            } else {
                footer
            };
            {
                let mut s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                s.sections.ctx = (!footer.is_empty()).then(|| footer.clone());
            }

            for img_path in img_paths {
                match tokio::fs::read(&img_path).await {
                    Ok(bytes) => {
                        if let Err(e) =
                            photo_in_thread(bot, chat_id, thread_id, InputFile::memory(bytes)).await
                        {
                            tracing::error!("Telegram: failed to send generated image: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Telegram: failed to read image {}: {}", img_path, e);
                    }
                }
            }

            // Rich fallback: when all content was sent as HTML intermediates
            // during streaming, the dedup step strips text_only to empty. If
            // the original response had rich structure (tables, headings,
            // lists), replace the HTML intermediates with a single native rich
            // message so Telegram renders proper tables and blocks.
            // #46: this arm decides on `pre_dedup_text` BEFORE the options
            // reclaim below can restore the host, so it must consult
            // options_pending itself — same gate as the final-answer site
            // (#45) — or a fully-deduped buttons turn stays plain.
            let text_only = if text_only.is_empty()
                && !sent.is_empty()
                && super::rich::should_send_native_rich_for(
                    &pre_dedup_text,
                    options_pending(streaming),
                ) {
                let rich_md = pre_dedup_text.clone();
                match super::rich::send_rich_with_mermaid_id(
                    bot.api_url().as_str(),
                    bot.token(),
                    chat_id.0,
                    thread_id,
                    &rich_md,
                    None,
                    "turn",
                    "-",
                )
                .await
                {
                    Ok(rich_msg_id) => {
                        // Delete the HTML intermediates now that rich message succeeded
                        let intermediate_ids = {
                            let s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                            s.intermediate_msg_ids.clone()
                        };
                        for mid in &intermediate_ids {
                            best_effort_delete(bot, chat_id, *mid, "intermediate cleanup").await;
                        }
                        tracing::info!(
                            "Telegram: rich fallback delivered ({} chars), deleted {} HTML intermediates",
                            rich_md.len(),
                            intermediate_ids.len()
                        );
                        // Merge candidate (#tg-suggest-merge): every rich
                        // bubble can carry the suggestion controls now —
                        // table-bearing answers ride the markdown plane,
                        // whose server-side render keeps tables intact
                        // (#79 piece 4; the html plane flattened them,
                        // #679, which is why they used to be excluded).
                        final_bubble = Some(super::state::MergeBubble {
                            message_id: teloxide::types::MessageId(rich_msg_id),
                            body: super::state::BubbleBody::Markdown(pre_dedup_text.clone()),
                        });
                        // Store bot reply in channel_messages even though
                        // text_only is empty (dedup stripped it). The rich
                        // fallback already sent pre_dedup_text, so the next
                        // turn's recent() query sees the bot's side of the
                        // conversation. Without this, the agent "talks to
                        // itself in the dark" after every rich fallback.
                        if !is_dm {
                            let bot_display_name = telegram_state
                                .bot_username()
                                .await
                                .map(|u| format!("@{}", u))
                                .unwrap_or_else(|| "OpenCrabs".to_string());
                            let thread_id_str = thread_id.map(|t| t.0.to_string());
                            let cm = DbChannelMessage::new(
                                "telegram".to_string(),
                                chat_id.0.to_string(),
                                Some(chat_title.to_string()),
                                "bot:opencrabs".to_string(),
                                bot_display_name,
                                pre_dedup_text.clone(),
                                "text".to_string(),
                                Some(rich_msg_id.to_string()),
                            )
                            .with_thread(thread_id_str, None);
                            if let Err(e) = channel_msg_repo.insert(&cm).await {
                                tracing::warn!(
                                    "Telegram: rich fallback: failed to record bot reply: {}",
                                    e
                                );
                            }
                        }
                        text_only
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Telegram: rich fallback failed, keeping HTML intermediates: {e}"
                        );
                        text_only
                    }
                }
            } else {
                text_only
            };

            // #300 follow-up: ALWAYS check if the trailing folded text matches the
            // final answer and remove it to prevent duplication. For CLI providers,
            // the final answer arrives as a trailing IntermediateText folded into
            // the collapsed block while response.content comes back empty, so we
            // reclaim it. For other providers (or CLI turns where the answer stayed
            // in content), if the same text ended up both folded and in the final
            // response, remove the folded copy to avoid showing it twice.
            //
            // #31: with options pending the reclaim returns BOTH runs — the
            // answer as host, the post-halt sign-off run as trailer — and
            // `settle_options_reclaim` arbitrates. The options arm runs FIRST:
            // the stock trailing-duplicate strip used to pop the trailer run
            // and drop the host with it (the smoke-v4 abandonment).
            let (text_only, reclaimed_trailer) = if text_only.trim().is_empty() {
                // CLI provider case: no separate answer, reclaim the folded final
                let (host, trailer) =
                    take_folded_final(bot, chat_id, streaming, options_pending(streaming)).await;
                settle_options_reclaim(text_only, host, trailer)
            } else if options_pending(streaming) {
                // #1226 flow-fold (K) + #31 trailer: the turn halted on the
                // suggest_options surface — the substantive pre-options answer
                // is the Text run BEFORE the Tool entry, the sign-off ack the
                // run AFTER it. Reclaim both; the answer becomes the final
                // bubble, the ack rides after the buttons.
                let (host, trailer) = take_folded_final(bot, chat_id, streaming, true).await;
                if host.is_none() && trailer.is_none() && !pre_reclaimed {
                    tracing::warn!(
                        "Telegram: options pending but flow reclaim returned nothing — \
                         answer stuck in flow block (#1226 K)"
                    );
                }
                let (text, trailer) = settle_options_reclaim(text_only, host, trailer);
                if let Some(t) = &trailer {
                    tracing::info!(
                        "Telegram: options reclaim settled — {} trailer chars ride after \
                         the buttons (#31)",
                        t.len()
                    );
                }
                (text, trailer)
            } else {
                // Non-CLI case: check if the trailing folded text matches the final
                // answer and remove it to prevent duplication
                let trailing_matches = {
                    let s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                    // Past any chrome appended after the answer (#1253): a
                    // provider switch landing late must not hide the folded
                    // copy and let it render twice.
                    match last_folded_text(&s.flow_entries) {
                        Some(folded) => folded_duplicates_final(folded, &text_only),
                        None => false,
                    }
                };
                if trailing_matches {
                    // Remove the duplicate from the block
                    take_folded_final(bot, chat_id, streaming, options_pending(streaming)).await;
                    (text_only, None)
                } else {
                    (text_only, None)
                }
            };

            // #31: hand the trailer to render_suggestions — rich merges it as
            // a paragraph after the in-body button rows, classic delivers it
            // as its own bubble. Stashed here so both render sites (handler
            // turn end + resume) pick it up identically.
            if let Some(trailer) = reclaimed_trailer {
                streaming
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .pending_trailer = Some(trailer);
            }

            // #690: re-expand any table the model collapsed onto one line so it
            // renders (native rich or <pre> grid) instead of raw pipes. Applied
            // once here before the rich-vs-HTML branching, so both the
            // should_send_native_rich detection below and the HTML render see the
            // reflowed table. Idempotent on well-formed tables.
            let text_only = super::rich::reflow_collapsed_tables(&text_only);

            // Deliver final response — prefer editing the streaming message in-place
            // to avoid the delete+send race that causes duplicates.
            let html = markdown_to_telegram_html(&text_only);
            // Final answers stay clean prose: the ctx footer lives on the
            // settled flow message, not here.
            let display_html = html.clone();
            tracing::info!(
                "Telegram deliver: html.len={}, ctx footer on flow='{}'",
                html.len(),
                footer
            );
            // Telegram message_id of the FINAL reply bubble. Captured across the
            // delivery paths (rich send, in-place edit, chunked send) so we can
            // persist it and later recover the EXACT message a user replies to,
            // instead of guessing "the most recent bot message" (#234 follow-up).
            let mut sent_reply_id: Option<i32> = None;
            // Merge candidate (#tg-suggest-merge): id + exact HTML of the bubble
            // the classic HTML path delivered the final response in. Captured in
            // every success arm below; handed to StreamingState after delivery so
            // suggest_options can attach its keyboard to THIS bubble instead of
            // posting a separate "Suggested next" message. Rich and voice paths
            // deliberately leave it None — their bubbles are not re-editable as
            // plain HTML without breaking their rendering.
            if !display_html.is_empty() {
                // Rich-first delivery: a structured reply (tables / headings /
                // lists / math) is delivered as a native Telegram rich message
                // regardless of length — Telegram renders the raw markdown into
                // real tables and blocks. Edit the streamed placeholder in place
                // if we have one, otherwise send a fresh message. The ctx footer
                // is plain text, appended as-is. On ANY failure we fall through
                // to the HTML chunking path below, so the streaming path is
                // never regressed. Plain prose skips rich entirely so Telegram's
                // parser never reinterprets incidental characters — EXCEPT prose
                // that ends on a suggest_options surface (#45): the tap rewrite
                // preserves the host plane, so button-bearing prose rides rich
                // too and the pick record edits back in rendered form.
                // #679: for TABLE messages, skip only the doomed native-BLOCKS
                // attempt — Telegram's InputRichBlock rejects our header/rows/align
                // shape (its schema wants cells/size), so a table always 400s the
                // block send and wastes a round-trip. But the rich-MARKDOWN send
                // renders tables correctly, so route tables straight to it instead
                // of skipping the whole rich branch (which #651 did, sending tables
                // to the HTML path where they showed as bare markup). Non-table
                // rich content still tries blocks first (clean fences) then falls
                // back to markdown.
                let mut delivered_rich = super::rich::should_send_native_rich_for(
                    &text_only,
                    // #45: `options_pending` is true when the turn stashed a
                    // suggest_options set mid-turn (#1226 K helper) — force the
                    // rich plane for prose so buttons never live on a plain host.
                    options_pending(streaming),
                ) && {
                    let rich_md = text_only.clone();
                    // Send a FRESH rich message rather than editing the streamed
                    // placeholder into rich. Editing a normal message into a rich
                    // one glitches the client render — overlap during the
                    // transition, and a stale pre-edit (HTML) version after a
                    // refresh / chat switch. A fresh sendRichMessage renders clean.
                    //
                    // Delete the placeholder FIRST so the fresh rich message is
                    // the LAST thing added to the chat — deleting it AFTER the
                    // send pulls the content up and leaves the view mid-chat
                    // instead of scrolling to the bottom on completion. `.take()`
                    // clears the id so the HTML fallback below sends a fresh
                    // message (not an edit of a deleted one) if the rich send fails.
                    if let Some(mid) = streaming_msg_id.take() {
                        best_effort_delete(bot, chat_id, mid, "pre-rich-fallback cleanup").await;
                    }
                    // Native BLOCKS first (#476 path B) for NON-table content: the
                    // block value is sent as-is, so code fences render natively with
                    // no server-side parser to mangle them into <code> artifacts. A
                    // table would only 400 here (schema mismatch), so skip blocks
                    // entirely when one is present and let the markdown send below
                    // render it. On any block rejection we also fall through to
                    // markdown, so worst case is exactly the rich-markdown render.
                    // Straight to rich-markdown (#871). The native-blocks
                    // attempt ran 68 times across two days and returned 400
                    // (RICH_MESSAGE_CONTENT_REQUIRED) all 68 times, never once
                    // succeeding, so every rich message already arrived via the
                    // markdown fallback below. Keeping it cost a guaranteed
                    // round-trip and a guaranteed error before every delivery.
                    //
                    // Markdown is also the only mode that renders tables: the
                    // rich HTML input mode returns 200 and then flattens a table
                    // into a run-on paragraph, which is why telegram_send now
                    // uses this same call rather than its own.
                    {
                        // Rich MARKDOWN renders tables correctly; mermaid fences
                        // are routed to the rich-HTML image path inside the sender
                        // (#1044), everything else stays on markdown.
                        // #tg-mermaid-delivery-hardening: retry once when
                        // Telegram's server-side refetch of the embedded media
                        // (mermaid.ink) died — `RICH_MESSAGE_PHOTO_NO_MEDIA_FOUND`
                        // is a race against a flaky renderer (our resolve ran
                        // seconds earlier), not a structural rejection. The sender
                        // re-resolves media on every call, so the retry naturally
                        // re-fetches from the renderer. Structural 400s are never
                        // retried.
                        let mut rich_send = super::rich::send_rich_with_mermaid_id(
                            bot.api_url().as_str(),
                            bot.token(),
                            chat_id.0,
                            thread_id,
                            &rich_md,
                            None,
                            "turn",
                            "-",
                        )
                        .await;
                        if rich_send.is_err() && is_no_media_found(rich_send.as_ref().unwrap_err())
                        {
                            tracing::warn!(
                                "Telegram: rich send hit NO_MEDIA_FOUND (renderer flake?) — retrying once"
                            );
                            rich_send = super::rich::send_rich_with_mermaid_id(
                                bot.api_url().as_str(),
                                bot.token(),
                                chat_id.0,
                                thread_id,
                                &rich_md,
                                None,
                                "turn",
                                "-",
                            )
                            .await;
                        }
                        match rich_send {
                            Ok(id) => {
                                // Success was silent, which is why an
                                // unformatted table could not be traced (#860).
                                tracing::info!(
                                    "Telegram: rich markdown delivered as msg {id} ({} chars)",
                                    rich_md.len()
                                );
                                sent_reply_id = Some(id);
                                // Merge candidate (#tg-suggest-merge): the
                                // controls can ride this bubble — table-bearing
                                // answers included: the merge edit goes back out
                                // on the markdown plane, whose server-side
                                // render keeps tables intact (#79 piece 4; the
                                // old html-plane merge flattened them, #679,
                                // which is why tables used to be excluded).
                                final_bubble = Some(super::state::MergeBubble {
                                    message_id: teloxide::types::MessageId(id),
                                    body: super::state::BubbleBody::Markdown(rich_md.clone()),
                                });
                                true
                            }
                            Err(e) => {
                                tracing::warn!("Telegram: rich delivery failed, using HTML: {e}");
                                false
                            }
                        }
                    }
                };

                if !delivered_rich {
                    // #tg-mermaid-delivery-hardening: last-chance mermaid render
                    // before degrading to chunks — the classic chunked HTML path
                    // cannot embed `<img>`, so when the source carried a diagram
                    // try the rich HTML dialect once more (it supports images).
                    // If the renderer recovered in the meantime the diagram still
                    // lands inline instead of raw fence text; if it is still down
                    // the resolve yields a legible failure block (renderer note +
                    // source) rather than a bare code dump.
                    if super::rich::mermaid::should_render_mermaid(&text_only) {
                        let fallback_html =
                            super::rich::markdown_to_html_mermaid_p(&text_only).await;
                        match super::rich::api::send_rich_html_id(
                            bot.api_url().as_str(),
                            bot.token(),
                            chat_id.0,
                            thread_id,
                            &fallback_html,
                            None,
                            "turn",
                            "-",
                        )
                        .await
                        {
                            Ok(id) => {
                                tracing::info!(
                                    "Telegram: rich-html mermaid fallback delivered as msg {id}"
                                );
                                sent_reply_id = Some(id);
                                delivered_rich = true;
                            }
                            Err(e2) => {
                                tracing::warn!(
                                    "Telegram: rich-html mermaid fallback failed ({e2}); degrading to chunks"
                                );
                            }
                        }
                    }
                }
                if !delivered_rich {
                    let chunks: Vec<String> = split_message(&display_html, 4096)
                        .into_iter()
                        .map(|s| s.to_string())
                        .collect();

                    // If single chunk and we have a streaming message, edit it in-place
                    if chunks.len() == 1
                        && let Some(mid) = streaming_msg_id
                    {
                        match bot
                            .edit_message_text(chat_id, mid, &chunks[0])
                            .parse_mode(ParseMode::Html)
                            .await
                        {
                            Ok(_) => {
                                // Edited in place — the reply bubble keeps `mid`.
                                // The final visible text is what a duplicate or
                                // chatty-agent investigation needs: one line
                                // closes "what actually landed in the mutated
                                // message" (#1085 post-review). Failure arms
                                // already log via the fallback sends' telemetry.
                                super::telemetry::log_send_success(
                                    "turn",
                                    "-",
                                    &session_id.to_string(),
                                    "stream_edit_final",
                                    "in_place_edit",
                                    chat_id.0,
                                    thread_id.map(|t| t.0.0),
                                    mid.0,
                                    chunks[0].len(),
                                    &super::telemetry::content_hash8(&chunks[0]),
                                );
                                sent_reply_id = Some(mid.0);
                                // Merge candidate (#tg-suggest-merge): the
                                // answer bubble suggest_options can ride on.
                                final_bubble = Some(super::state::MergeBubble {
                                    message_id: mid,
                                    body: super::state::BubbleBody::Html(chunks[0].clone()),
                                });
                            }
                            Err(teloxide::RequestError::RetryAfter(secs)) => {
                                super::rate_limit::wait_out("edit", secs.duration(), "").await;
                                match bot
                                    .edit_message_text(chat_id, mid, &chunks[0])
                                    .parse_mode(ParseMode::Html)
                                    .await
                                {
                                    Ok(_) => {
                                        sent_reply_id = Some(mid.0);
                                        final_bubble = Some(super::state::MergeBubble {
                                            message_id: mid,
                                            body: super::state::BubbleBody::Html(
                                                chunks[0].clone(),
                                            ),
                                        });
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Telegram: edit retry failed ({e}), falling back to delete+send"
                                        );
                                        best_effort_delete(
                                            bot,
                                            chat_id,
                                            mid,
                                            "edit-retry fallback",
                                        )
                                        .await;
                                        // Never silent (#1019): this is the LAST fallback.
                                        // The edit already failed and was logged; if the
                                        // resend fails too the message is gone entirely,
                                        // so the recovery path must not be the quiet one.
                                        if let Ok(sent) = send_html_or_plain(
                                            bot, chat_id, thread_id, &chunks[0], "turn", None,
                                        )
                                        .await
                                        {
                                            sent_reply_id = Some(sent.0);
                                            final_bubble = Some(super::state::MergeBubble {
                                                message_id: sent,
                                                body: super::state::BubbleBody::Html(
                                                    chunks[0].clone(),
                                                ),
                                            });
                                        } else {
                                            tracing::error!(
                                                "Telegram: delete+send fallback failed in chat {chat_id}, \
                                                 the reply was lost"
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Telegram: edit final failed ({e}), falling back to delete+send"
                                );
                                best_effort_delete(bot, chat_id, mid, "edit-final fallback").await;
                                if let Ok(sent) = send_html_or_plain(
                                    bot, chat_id, thread_id, &chunks[0], "turn", None,
                                )
                                .await
                                {
                                    sent_reply_id = Some(sent.0);
                                    final_bubble = Some(super::state::MergeBubble {
                                        message_id: sent,
                                        body: super::state::BubbleBody::Html(
                                            chunks[0].clone(),
                                        ),
                                    });
                                }
                            }
                        }
                    } else {
                        // Multi-chunk or no streaming message — delete old, send new
                        if let Some(mid) = streaming_msg_id {
                            best_effort_delete(bot, chat_id, mid, "multi-chunk swap").await;
                        }
                        for chunk in &chunks {
                            // Last chunk wins — that's the bubble a user replies to.
                            if let Ok(sent) =
                                send_html_or_plain(bot, chat_id, thread_id, chunk, "turn", None)
                                    .await
                            {
                                sent_reply_id = Some(sent.0);
                                final_bubble = Some(super::state::MergeBubble {
                                    message_id: sent,
                                    body: super::state::BubbleBody::Html(chunk.clone()),
                                });
                            }
                        }
                    }
                }
            } else if let Some(mid) = streaming_msg_id {
                // Empty final text: all content was already delivered as
                // intermediate messages. The ctx budget rides the settled
                // flow message now, so just remove the now-empty streaming
                // placeholder.
                best_effort_delete(bot, chat_id, mid, "empty-final placeholder").await;
            }

            // Hand the merge candidate to the turn state (#tg-suggest-merge):
            // handler.rs reads it immediately after this returns and passes it
            // into render_suggestions. None (rich / voice / suppressed paths)
            // means suggestions fall back to their standalone block as before.
            if final_bubble.is_some() {
                streaming
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .final_bubble = final_bubble;
            }

            // Record the bot's text reply into channel_messages.
            //
            // Groups: needed so the recent() query that builds conversation
            // context on the NEXT turn sees both sides — without it the bot
            // loads a one-sided transcript and talks to itself in the dark.
            //
            // Both group AND DM persist the Telegram message_id (when captured)
            // so a later reply to THIS bubble can be recovered EXACTLY by id —
            // Telegram delivers rich bot messages with empty text, so the reply
            // handler can't read the quoted content from the update and must
            // look it up by id (#234 follow-up). DMs are stored only when we
            // have an id (lookup-only; DM conversation context still comes from
            // the session messages table, not channel_messages).
            let pmid = sent_reply_id.map(|i| i.to_string());
            if !text_only.trim().is_empty() && (!is_dm || pmid.is_some()) {
                let bot_display_name = telegram_state
                    .bot_username()
                    .await
                    .map(|u| format!("@{}", u))
                    .unwrap_or_else(|| "OpenCrabs".to_string());
                let thread_id = thread_id.map(|t| t.0.to_string());
                let cm = DbChannelMessage::new(
                    "telegram".to_string(),
                    chat_id.0.to_string(),
                    Some(chat_title.to_string()),
                    "bot:opencrabs".to_string(),
                    bot_display_name,
                    text_only.clone(),
                    "text".to_string(),
                    pmid.clone(),
                )
                .with_thread(thread_id, None);
                if let Err(e) = channel_msg_repo.insert(&cm).await {
                    tracing::warn!(
                        "Telegram: failed to record bot reply in channel_messages: {}",
                        e
                    );
                }
            }

            // If input was voice AND TTS is enabled, also send voice note after text
            if is_voice && voice_config.tts_enabled {
                tracing::info!(
                    "Telegram: TTS requested — synthesizing response text (len={})",
                    response.content.len()
                );
                match crate::channels::voice::synthesize(&response.content, voice_config).await {
                    Ok(audio_bytes) => {
                        tracing::info!(
                            "Telegram: TTS succeeded — {} bytes of audio, sending to chat {}",
                            audio_bytes.len(),
                            chat_id
                        );
                        match bot
                            .send_voice(chat_id, InputFile::memory(audio_bytes))
                            .await
                        {
                            Ok(m) => {
                                tracing::info!(
                                    "Telegram: voice message delivered (msg_id={})",
                                    m.id
                                );
                                // Record the delivered voice message ID in
                                // the isolated voice_msg_ids list. Cleanup
                                // paths do not touch this list. See the
                                // field doc on StreamingState.
                                let mut s = streaming.lock().unwrap_or_else(|e| e.into_inner());
                                s.voice_msg_ids.push(m.id);
                            }
                            Err(e) => {
                                tracing::error!("Telegram: send_voice failed — {}: {:?}", e, e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Telegram: TTS synthesis failed: {:#}", e);
                    }
                }
            }
        }
        Err(ref e) if matches!(e, crate::brain::agent::AgentError::Cancelled) => {
            tracing::info!("Telegram: agent call cancelled for session {}", session_id);
            // Silently clean up — user already received "Operation cancelled." from /stop
            if let Some(mid) = streaming_msg_id {
                best_effort_delete(bot, chat_id, mid, "cancel cleanup").await;
            }
        }
        Err(e) => {
            tracing::error!("Telegram: agent error: {}", e);
            // Translate via the shared helper so the message tells the
            // user WHAT self-heal already tried + what to do next,
            // instead of leaking the raw `API error (502)` shape that
            // confused users into thinking the agent silently dropped
            // their request. See `brain::agent::format_user_error` for
            // the pattern matchers (5xx exhausted / 429 / context too
            // large / stream broken / repetition loop / etc.).
            let user_msg = format!("❌ Error\n\n{}", crate::brain::agent::format_user_error(&e));
            if let Some(mid) = streaming_msg_id {
                if let Err(e) = bot.edit_message_text(chat_id, mid, user_msg).await {
                    tracing::warn!(
                        target: "telegram::send",
                        chat_id = chat_id.0,
                        message_id = mid.0,
                        error = %e,
                        "final-error edit failed"
                    );
                }
            } else {
                message_in_thread(bot, chat_id, thread_id, user_msg).await?;
            }
        }
    }
    Ok(true)
}

/// Drain the display items left queued after the edit loop stopped,
/// folding them into the processing-log flow. ONE shared implementation for
/// handle_message and resume_session (#470 / #462 item 1: the drains were
/// copy-pasted and could drift apart — parity is now structural).
/// `react_target` is the inbound message a folded `<<react:>>` directive
/// acknowledges; resume has none (the original message id is lost across
/// restarts), so the directive strips without firing (#261).
pub(crate) async fn drain_remaining_display(
    bot: &Bot,
    chat: ChatId,
    thread_id: Option<teloxide::types::ThreadId>,
    streaming: &Arc<std::sync::Mutex<StreamingState>>,
    remaining: Vec<DisplayItem>,
    react_target: Option<MessageId>,
) {
    let mut tool_buffer: Vec<usize> = Vec::new();
    for item in remaining {
        match item {
            DisplayItem::NewTool(idx) => {
                tool_buffer.push(idx);
            }
            DisplayItem::Intermediate(text) => {
                // Fold into the open processing-log flow instead of sending
                // a standalone message (#300); sanitize exactly like the
                // live edit-loop path.
                append_tool_group(bot, chat, thread_id, streaming, &tool_buffer).await;
                tool_buffer.clear();
                let text = crate::utils::sanitize::strip_llm_artifacts(&text);
                let text = redact_secrets(&text);
                let (text, _img_paths) = crate::utils::extract_img_markers(&text);
                let (text, react_emoji) = crate::utils::extract_react_marker(&text);
                if let (Some(target), Some(emoji)) = (react_target, react_emoji.as_deref()) {
                    fire_reaction(bot, chat, target, emoji).await;
                }
                append_intermediate_to_flow(bot, chat, thread_id, streaming, &text).await;
            }
            DisplayItem::System(text) => {
                // Chrome folds into the same block, in order, but skips the
                // model pipeline: it is our own text, so there is no artifact
                // to strip, no image marker, and no react directive to fire.
                // Secrets are still scrubbed — a self-healing alert embeds a
                // raw upstream error string (#1253).
                append_tool_group(bot, chat, thread_id, streaming, &tool_buffer).await;
                tool_buffer.clear();
                let text = redact_secrets(&text);
                append_system_to_flow(bot, chat, thread_id, streaming, &text).await;
            }
        }
    }
    // Flush any remaining tools into the open group (merges the final batch
    // into the running collapsible block instead of opening a new message).
    append_tool_group(bot, chat, thread_id, streaming, &tool_buffer).await;
}

/// Strip an echoed plan title from the start of the agent's response (#837).
///
/// The `[ACTIVE PLAN REMINDER]` shows the model `📋 Plan: "{title}"` every
/// turn, and the model opens its reply by repeating it. The plan card already
/// carries the title, so the text repeats what is rendered directly above it.
///
/// Pure, taking the title rather than loading it, so the matching is testable
/// without a session on disk — the first version compared an exact string and
/// could only be exercised end to end.
pub(crate) fn strip_echoed_plan_title(text: &str, plan_title: &str) -> String {
    let title = normalize_title_line(plan_title);
    if title.is_empty() {
        return text.to_string();
    }
    let trimmed = text.trim_start();
    let Some(first_line) = trimmed.lines().next() else {
        return text.to_string();
    };
    if normalize_title_line(first_line) != title {
        return text.to_string();
    }
    // Drop the line and the blank line that usually follows a heading.
    let rest = trimmed[first_line.len()..].trim_start_matches('\n');
    rest.to_string()
}

/// Reduce a line to the bare title for comparison.
///
/// Handles the shapes the model actually produces, which are the shapes the
/// reminder itself shows it: markdown headings and emphasis, the `📋` the
/// reminder and the card both use, a `Plan:` label, and surrounding quotes.
/// The earlier version trimmed only `#`, `*` and `~`, so an echo of the
/// reminder's own formatting — the most likely echo of all — never matched.
fn normalize_title_line(line: &str) -> String {
    let mut s = line.trim();
    // Leading blockquote / list / heading markers.
    s = s.trim_start_matches(['#', '>', '-', '*', '_', '~', ' ']);
    // Any leading non-alphanumeric run: emoji such as 📋, bullets, symbols.
    s = s.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '"');
    // The reminder labels it `Plan: "…"`; the model copies the label too.
    for label in ["Plan:", "plan:", "PLAN:"] {
        if let Some(rest) = s.strip_prefix(label) {
            s = rest.trim_start();
            break;
        }
    }
    s = s.trim_matches(['"', '\'', '*', '~', '_', ' ']);
    s.trim().to_string()
}
