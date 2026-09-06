use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallDisplay {
    pub description: String,
    pub success: bool,
    pub output: Option<String>,
    pub input: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChatDisplayItem {
    Text {
        content: String,
        reasoning: Option<String>,
    },
    Tools(Vec<ToolCallDisplay>),
    ProtocolFallback {
        label: &'static str,
        content: String,
    },
}

/// Reconstruct the TUI's compact message hierarchy for the desktop surface.
/// Raw provider markers never reach the default visible transcript: reasoning
/// and tool calls become progressive-disclosure items instead.
pub fn display_message_items(content: &str, thinking: Option<&str>) -> Vec<ChatDisplayItem> {
    let mut items = Vec::new();
    let mut remaining = content;
    while let Some((marker_at, v2)) = next_tool_marker(remaining) {
        push_text_item(&mut items, &remaining[..marker_at], None);
        let marker = if v2 { "<!-- tools-v2:" } else { "<!-- tools:" };
        let after_marker = &remaining[marker_at + marker.len()..];
        let Some(end) = after_marker.find("-->") else {
            items.push(ChatDisplayItem::ProtocolFallback {
                label: "Unparsed tool activity",
                content: remaining[marker_at..].to_string(),
            });
            remaining = "";
            break;
        };
        let raw_marker = &remaining[marker_at..marker_at + marker.len() + end + 3];
        let tool_data = after_marker[..end].trim();
        let tools: Vec<ToolCallDisplay> = if v2 {
            serde_json::from_str::<Vec<Value>>(tool_data)
                .unwrap_or_default()
                .into_iter()
                .map(|entry| ToolCallDisplay {
                    description: entry["d"].as_str().unwrap_or("Tool call").to_string(),
                    success: entry["s"].as_bool().unwrap_or(true),
                    output: entry["o"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                    input: entry.get("i").cloned().unwrap_or(Value::Null),
                })
                .collect()
        } else {
            tool_data
                .split(" | ")
                .filter(|entry| !entry.trim().is_empty())
                .map(|description| ToolCallDisplay {
                    description: description.to_string(),
                    success: true,
                    output: None,
                    input: Value::Null,
                })
                .collect()
        };
        if tools.is_empty() {
            items.push(ChatDisplayItem::ProtocolFallback {
                label: "Unparsed tool activity",
                content: raw_marker.to_string(),
            });
        } else {
            items.push(ChatDisplayItem::Tools(tools));
        }
        remaining = &after_marker[end + 3..];
    }
    push_text_item(&mut items, remaining, thinking);
    if items.is_empty() && thinking.is_some_and(|value| !value.trim().is_empty()) {
        items.push(ChatDisplayItem::Text {
            content: String::new(),
            reasoning: thinking.map(str::to_string),
        });
    }
    items
}

fn next_tool_marker(value: &str) -> Option<(usize, bool)> {
    match (value.find("<!-- tools-v2:"), value.find("<!-- tools:")) {
        (Some(v2), Some(v1)) => Some(if v2 <= v1 { (v2, true) } else { (v1, false) }),
        (Some(v2), None) => Some((v2, true)),
        (None, Some(v1)) => Some((v1, false)),
        (None, None) => None,
    }
}

fn push_text_item(items: &mut Vec<ChatDisplayItem>, value: &str, fallback_thinking: Option<&str>) {
    let (reasoning, clean) = extract_reasoning(value);
    let reasoning = reasoning.or_else(|| {
        fallback_thinking
            .filter(|item| !item.trim().is_empty())
            .map(str::to_string)
    });
    if !clean.is_empty() || reasoning.is_some() {
        items.push(ChatDisplayItem::Text {
            content: clean,
            reasoning,
        });
    }
}

fn extract_reasoning(value: &str) -> (Option<String>, String) {
    let mut rest = value.to_string();
    let mut traces = Vec::new();
    extract_delimited(
        &mut rest,
        "<!-- reasoning -->",
        "<!-- /reasoning -->",
        &mut traces,
        false,
    );
    for tag in ["think", "antthinking", "mm:think"] {
        extract_tag(&mut rest, tag, &mut traces);
        rest = replace_case_insensitive(&rest, &format!("<{} />", tag).replace(" ", ""), "");
        rest = replace_case_insensitive(&rest, &format!("</{tag}>"), "");
    }
    let clean = rest.trim().to_string();
    let trace = (!traces.is_empty()).then(|| traces.join("\n\n"));
    (trace, clean)
}

fn extract_delimited(
    rest: &mut String,
    open: &str,
    close: &str,
    parts: &mut Vec<String>,
    case_insensitive: bool,
) {
    loop {
        let haystack = if case_insensitive {
            rest.to_lowercase()
        } else {
            rest.clone()
        };
        let open_key = if case_insensitive {
            open.to_lowercase()
        } else {
            open.to_string()
        };
        let close_key = if case_insensitive {
            close.to_lowercase()
        } else {
            close.to_string()
        };
        let Some(start) = haystack.find(&open_key) else {
            break;
        };
        let after_start = start + open.len();
        let after = &rest[after_start..];
        let after_search = if case_insensitive {
            after.to_lowercase()
        } else {
            after.to_string()
        };
        let before = rest[..start].to_string();
        if let Some(end) = after_search.find(&close_key) {
            let trace = after[..end].trim();
            if !trace.is_empty() {
                parts.push(trace.to_string());
            }
            *rest = format!("{}{}", before, &after[end + close.len()..]);
        } else {
            let trace = after.trim();
            if !trace.is_empty() {
                parts.push(trace.to_string());
            }
            *rest = before;
            break;
        }
    }
}

fn extract_tag(rest: &mut String, tag: &str, parts: &mut Vec<String>) {
    extract_delimited(rest, &format!("<{tag}>"), &format!("</{tag}>"), parts, true);
}

fn replace_case_insensitive(value: &str, needle: &str, replacement: &str) -> String {
    let mut result = String::new();
    let mut remaining = value;
    let needle_lower = needle.to_lowercase();
    while let Some(index) = remaining.to_lowercase().find(&needle_lower) {
        result.push_str(&remaining[..index]);
        result.push_str(replacement);
        remaining = &remaining[index + needle.len()..];
    }
    result.push_str(remaining);
    result
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub provider_name: Option<String>,
    pub working_directory: Option<String>,
    pub token_count: i64,
    pub total_cost: f64,
    pub created_at: String,
    pub updated_at: String,
    pub is_archived: bool,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_items_hide_reasoning_and_group_tool_markers() {
        let content = "<!-- reasoning -->Check state<!-- /reasoning -->Ready.<!-- tools-v2: [{\"d\":\"Read config\",\"s\":true,\"o\":\"ok\",\"i\":{\"path\":\"config.toml\"}}] -->Done.";
        let items = display_message_items(content, None);
        assert!(
            matches!(&items[0], ChatDisplayItem::Text { content, reasoning: Some(reasoning) } if content == "Ready." && reasoning == "Check state")
        );
        assert!(
            matches!(&items[1], ChatDisplayItem::Tools(calls) if calls.len() == 1 && calls[0].description == "Read config")
        );
        assert!(matches!(&items[2], ChatDisplayItem::Text { content, .. } if content == "Done."));
    }

    #[test]
    fn reasoning_only_message_remains_inspectable() {
        let items = display_message_items("<mm:think>Check the database</mm:think>", None);
        assert!(
            matches!(&items[..], [ChatDisplayItem::Text { content, reasoning: Some(reasoning) }] if content.is_empty() && reasoning == "Check the database")
        );
    }

    #[test]
    fn malformed_tool_marker_is_preserved_in_a_collapsed_fallback() {
        let items = display_message_items("Before<!-- tools-v2: [{not JSON}] -->After", None);
        assert!(matches!(&items[0], ChatDisplayItem::Text { content, .. } if content == "Before"));
        assert!(
            matches!(&items[1], ChatDisplayItem::ProtocolFallback { label, content } if *label == "Unparsed tool activity" && content == "<!-- tools-v2: [{not JSON}] -->")
        );
        assert!(matches!(&items[2], ChatDisplayItem::Text { content, .. } if content == "After"));
    }

    #[test]
    fn unterminated_tool_marker_is_never_silently_dropped() {
        let items = display_message_items("Answer<!-- tools: Read config", None);
        assert!(matches!(&items[0], ChatDisplayItem::Text { content, .. } if content == "Answer"));
        assert!(
            matches!(&items[1], ChatDisplayItem::ProtocolFallback { content, .. } if content == "<!-- tools: Read config")
        );
    }

    #[test]
    fn orphan_reasoning_closer_does_not_leak_to_the_answer() {
        let items = display_message_items("Answer</mm:think>", None);
        assert!(
            matches!(&items[..], [ChatDisplayItem::Text { content, reasoning: None }] if content == "Answer")
        );
    }

    #[test]
    fn display_items_use_persisted_thinking_when_inline_marker_is_absent() {
        let items = display_message_items("Answer", Some("Persisted trace"));
        assert!(
            matches!(&items[0], ChatDisplayItem::Text { content, reasoning: Some(reasoning) } if content == "Answer" && reasoning == "Persisted trace")
        );
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    pub content: String,
    pub sequence: i32,
    pub token_count: Option<i64>,
    pub cost: Option<f64>,
    pub created_at: String,
    pub thinking: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub name: String,
    pub enabled: bool,
    pub default_model: Option<String>,
    pub models: Vec<String>,
    pub has_api_key: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigInfo {
    pub providers: Vec<ProviderEntry>,
    pub agent_auto_approve: bool,
    pub a2a_enabled: bool,
    pub a2a_port: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrainFile {
    pub name: String,
    pub content: String,
    pub category: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDetail {
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub source: String,
    pub review_gate: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: String,
    pub review_gate: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CronJobInfo {
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    pub timezone: String,
    pub prompt: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub thinking: String,
    pub auto_approve: bool,
    pub deliver_to: Option<String>,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub created_at: String,
    pub profile_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CronJobRunInfo {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    pub status: String,
    pub content: Option<String>,
    pub error: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelStatus {
    pub name: String,
    pub display_name: String,
    pub enabled: bool,
    pub alive: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DashboardDataInfo {
    pub summary: SummaryInfo,
    pub daily: Vec<DailyInfo>,
    pub projects: Vec<ProjectInfo>,
    pub models: Vec<ModelInfo>,
    pub tools: Vec<UsageToolInfo>,
    pub activities: Vec<ActivityInfo>,
    pub cache: Option<CacheInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummaryInfo {
    pub total_tokens: i64,
    pub total_cost: f64,
    pub session_count: i64,
    pub call_count: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DailyInfo {
    pub date: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub project: String,
    pub cost: f64,
    pub tokens: i64,
    pub sessions: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
    pub estimated: bool,
    pub variants: Vec<VariantInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantInfo {
    pub name: String,
    pub tokens: i64,
    pub cost: f64,
    pub calls: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageToolInfo {
    pub tool_name: String,
    pub call_count: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActivityInfo {
    pub category: String,
    pub cost: f64,
    pub turns: i64,
    pub one_shot_pct: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CacheInfo {
    pub cache_hit_pct: f64,
    pub cached_tokens: i64,
    pub total_input_tokens: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub extension: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub is_binary: bool,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoiceConfigInfo {
    pub stt_enabled: bool,
    pub stt_provider: String,
    pub tts_enabled: bool,
    pub tts_provider: String,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsSnapshot {
    pub app_version: String,
    pub config_present: bool,
    pub database_present: bool,
    pub log_path: String,
    pub log_tail: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub release_notes: String,
    pub date: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesktopState {
    pub route: String,
    pub selected_session_id: Option<String>,
}
