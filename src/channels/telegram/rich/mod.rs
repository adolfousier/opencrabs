//! Telegram rich-message support (Bot API "rich messages", 2026-06).
//!
//! Pipeline: markdown text → [`ast::Block`] AST ([`parse`]) → either Telegram's
//! `InputRichMessage` JSON (rich-first path, finalized against the Bot API
//! field schema) or Telegram HTML ([`render_html`], the fallback path).
//!
//! The AST and parser are deliberately independent of the wire schema, so the
//! markdown front-end and its tests don't churn when the serializer lands, and
//! the same AST drives both the rich and fallback renderers.
//!
//! Files are kept small and single-purpose: [`ast`] (types), [`inline`] /
//! [`table`] / [`list`] / [`parse`] (front-end), [`detect`] (structure gates),
//! [`render_html`] (fallback), [`render_json`] (rich-first serializer, #420
//! path B), [`api`] (raw send), [`mermaid`] (diagram resolution).
//!
//! This file is declarations only — module decls and the public-surface
//! re-exports below. Functions never live in `mod.rs` (CONTRIBUTING.md).

pub(crate) mod api;
pub(crate) mod ast;
pub(crate) mod detect;
pub(crate) mod edit_dedup;
mod inline;
mod list;
pub(crate) mod mermaid;
pub(crate) mod parse;
mod render_html;
pub(crate) mod render_json;
pub(crate) mod table;

// One import path per name for every caller (`rich::markdown_to_html`,
// `rich::should_send_native_rich`, ...). The re-exports are the module's
// lib-consumed surface — moving a fn into its submodule never touches a
// call site. Test-only consumers import from the source module directly
// (e.g. `rich::detect::has_rich_structure`) so a lib-wide re-export can
// never go unused in the non-test target.
pub(crate) use api::{
    send_rich_with_mermaid, send_rich_with_mermaid_id, send_rich_with_mermaid_target_id,
};
pub(crate) use detect::{
    contains_table, is_atx_heading, prefers_rich_render, should_send_native_rich,
    should_send_native_rich_for,
};
pub(crate) use render_html::{
    markdown_to_html, markdown_to_html_mermaid, markdown_to_html_mermaid_p, markdown_to_html_p,
};
#[cfg(test)]
pub(crate) use table::{balance_code_fences, shield_bare_leading_hashes};
pub(crate) use table::{normalize_tables, reflow_collapsed_tables};
