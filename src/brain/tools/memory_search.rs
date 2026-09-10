//! Memory Search Tool
//!
//! Searches past conversation compaction logs using the memory store's FTS5 engine.
//! Always available — no external dependencies required.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Memory search tool backed by the memory store's FTS5 engine.
pub struct MemorySearchTool;

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search your memory. Returns ranked excerpts, cheap enough to run before \
         you write anything. \
         \
         `scope` picks the corpus, and picking wrong is the usual reason a search \
         comes back with nothing useful: \
         - \"memory\" (default) — daily logs. History: what happened, when, what was \
           decided in a past session. \
         - \"brain\" — your brain files (SOUL, USER, AGENTS, TOOLS, CODE, SECURITY, \
           MEMORY, BOOT, HEARTBEAT). Rules and policy: does a rule about this ALREADY \
           exist, and which file owns it. Use this before appending a rule. \
         - \"external\" — the user-configured external index paths. Structural \
           queries about indexed code (\"who calls X\", \"what does X call\", \
           \"where is X defined\", \"who implements Y\") route to the code symbol \
           graph and return call sites with file:line. \
         - \"all\" — both, for \"have I ever written about this anywhere\". \
         \
         Searching \"memory\" for a rule usually fails: there are far more daily notes \
         than brain files, and they reuse the same words for unrelated things, so \
         history outranks policy and you get three confident irrelevant hits. \
         \
         Once a hit tells you WHICH file holds a rule, use `load_brain_file` with a \
         `query` to read the whole section — this returns snippets, which are enough \
         to locate a rule but not always to judge it. \
         \
         Structural code queries (\"who calls X\", \"impact of X\") return complete, \
         sorted listings that support n/offset pagination and report their own \
         truncation; ranked FTS/vector results do NOT paginate (they stay \
         n-truncated by rank)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query. For code structure over indexed external paths (callers, definitions, references), phrase it naturally: \"who calls X\", \"where is X defined\" — routes to the code symbol graph."
                },
                "n": {
                    "type": "integer",
                    "description": "Number of results to return (default: 5)",
                    "default": 5
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip this many sorted results first. Structural listings only (who calls X / callees / definitions / impact — pagination window after ORDER BY). Ranked FTS/vector results ignore it.",
                    "default": 0
                },
                "scope": {
                    "type": "string",
                    "enum": ["memory", "brain", "external", "all"],
                    "description": "Which corpus to search: \"memory\" (daily logs, the default) for history, \"brain\" for rules and policy in your brain files, \"external\" for the user-configured external index paths, \"all\" for everything.",
                    "default": "memory"
                }
            },
            "required": ["query"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if query.is_empty() {
            return Ok(ToolResult::error("query parameter is required".to_string()));
        }

        let n = input.get("n").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        // Structural-only pagination (#89): applied after ORDER BY on the
        // graph-listing paths; ranked paths ignore it (enforced inside
        // search_external).
        let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        // Default stays "memory" so existing callers are unaffected (#1020).
        let scope = input
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("memory");

        // Get the memory store
        let store = match crate::memory::get_store() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Memory store init failed: {}", e);
                return Ok(ToolResult::error(format!(
                    "Memory search unavailable: {e}. \
                     Daily memory logs are still saved to your `memory/` dir as markdown files \
                     that you can read directly with the read_file tool."
                )));
            }
        };

        // External session gate (#1051, ADR-003): external content is
        // default-deny in shared/group sessions. The gate — not the exclude
        // patterns — is the security boundary.
        let external_blocked = crate::memory::is_session_shared(context.session_id)
            && !crate::memory::external_allowed_in_shared();

        let searched = match scope {
            "brain" => crate::memory::search_brain(store, &query, n).await,
            "external" => {
                if external_blocked {
                    return Ok(ToolResult::error(
                        "scope=\"external\" is not available in this shared/group session. \
                         External index content stays private to the owner's sessions by \
                         default (ADR-003). Set [memory] external_allowed_in_shared = true \
                         to allow it here."
                            .to_string(),
                    ));
                }
                crate::memory::search_external(store, &query, n, offset).await
            }
            "all" => match crate::memory::search_brain(store, &query, n).await {
                Ok(mut brain) => match crate::memory::search_memory(store, &query, n).await {
                    // Brain hits lead: a rule outranks a note mentioning it.
                    Ok(mem) => {
                        brain.extend(mem);
                        if external_blocked {
                            Ok(brain)
                        } else {
                            match crate::memory::search_external(store, &query, n, offset).await {
                                // External hits land last: brain > memory > external (Q10).
                                Ok(ext) => {
                                    brain.extend(ext);
                                    Ok(brain)
                                }
                                Err(e) => Err(e),
                            }
                        }
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            },
            // Default "memory" searches the memory corpus only (#1051):
            // external content never leaks into the default scope, and rules
            // live in scope="brain" (the empty-result hint below says so).
            _ => crate::memory::search_memory(store, &query, n).await,
        };

        match searched {
            Ok(results) if results.is_empty() => Ok(ToolResult::success(format!(
                "No matches in scope \"{scope}\".{}",
                if scope == "memory" {
                    " If you were checking whether a RULE already exists, search again \
                     with scope=\"brain\" — rules live in brain files, not daily logs."
                } else {
                    ""
                }
            ))),
            Ok(results) => {
                let mut output = String::new();
                // scope=all merges brain/memory/external blocks in order, so
                // the model can tell provenance: tag EVERY hit symmetrically
                // (#89) — the formatter stays corpus-generic.
                let tag_results = scope == "all";
                for (i, r) in results.iter().enumerate() {
                    let tag = corpus_tag(r.corpus, tag_results);
                    // Truncation markers ride empty-path results (#89): print
                    // them bare, without bolding a nonexistent path.
                    if r.path.is_empty() {
                        output.push_str(&format!("{}. {}{}\n\n", i + 1, tag, r.snippet));
                    } else {
                        output.push_str(&format!(
                            "{}. {}**{}**\n   {}\n\n",
                            i + 1,
                            tag,
                            r.path,
                            r.snippet
                        ));
                    }
                }
                Ok(ToolResult::success(output))
            }
            Err(e) => Ok(ToolResult::error(format!("Memory search failed: {e}"))),
        }
    }
}

/// The one-token corpus marker for a hit block (#89): `[brain] `, `[memory] `,
/// `[external] ` — symmetric across ALL corpora when scope=all merges them.
/// Empty when the scope names one corpus (no provenance ambiguity) or the
/// result carries no corpus (collection-wide search()).
pub(crate) fn corpus_tag(corpus: &str, scope_all: bool) -> String {
    if scope_all && !corpus.is_empty() {
        format!("[{corpus}] ")
    } else {
        String::new()
    }
}
